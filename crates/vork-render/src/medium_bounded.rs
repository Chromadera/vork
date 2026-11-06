//! Mesh-bounded media use the same compiled point evaluator and transport.
use super::*;

use std::collections::BTreeMap;

pub(crate) struct Installed {
    names: BTreeMap<String, (u32, u32)>,
    programs: BTreeMap<String, (vork_slang::MediumProgram, Vec<u32>)>,
}
impl Installed {
    /// A spatially and temporally constant, resource-free world program whose
    /// complete coefficient expressions prove vacuum needs no transport pass.
    pub(super) fn world_is_vacuum(&self, scene: &v2::Scene, name: &str) -> Result<bool> {
        let (program, words) = &self.programs[name];
        if !program.homogeneous || program.time_dependent || program.inputs.fields.iter().any(|f|
            !matches!(f.ty, vork_slang::Type::Float | vork_slang::Type::Int | vork_slang::Type::Uint | vork_slang::Type::Bool | vork_slang::Type::Vector(_))) {
            return Ok(false);
        }
        let dynamic = crate::materials::animated_inputs(scene)?.remove(name).unwrap_or_default();
        let proof = program.inputs.varying_words(words, &dynamic);
        let abi = programs::abi();
        // This is an optional identity proof; unsupported interval operations
        // retain the existing transport rather than rejecting a valid medium.
        Ok(program.emission_free_for_inputs(&abi, &proof).unwrap_or(false)
            && program.extinction_majorant(&abi, &proof, [[0.,0.];3], [0.,0.]).is_ok_and(|bound| bound == 0.))
    }
}
pub(super) fn install(
    r: &Renderer,
    scene: &v2::Scene,
    world: Option<&str>,
    base: &Path,
    stages: &BTreeMap<String, programs::Stages>,
) -> Result<super::Installed> {
    let mut names = BTreeMap::new();
    for name in stages
        .iter()
        .filter(|(_, stages)| stages.medium)
        .map(|(name, _)| name.as_str())
        .chain(world)
    {
        names.insert(name.to_owned(), (0, 0));
    }
    // Header: descriptor offset/count, world program/base. All offsets are relative
    // to frame.medium.x so material records and program words stay in one table.
    let mut words = vec![0, 0, u32::MAX, 0];
    let mut input_bindings = Vec::new();
    let animated = crate::materials::animated_inputs(scene)?;
    let mut native = Native {steps:constants::VOLUME_SAMPLES,..Native::default()};
    native.dense_grids_only=scene.buffers.values().any(|b|b.dims.is_some()) && scene.buffers.values().all(|b|b.grid.is_none());
    native.identity.push(u8::from(native.dense_grids_only));
    native.ray_shadows=crate::rt::ray_shadows(scene);
    native.identity.push(u8::from(native.ray_shadows));
    configure(&mut native);
    #[cfg(test)]
    if let Some((tile,slices,distribution))=r.reference_medium_grid {
        native.tile=tile; native.slices=slices; native.steps=slices; native.distribution=distribution;
    }
    let mut native_code = String::from("import vork_medium_bounds;\n");
    let mut native_dispatch = String::from(
        "export MediumSample volume_coefficients(uint id,float3 p,MediumTransportContext context) {switch(id) {\n",
    );
    let mut interval_dispatch=String::from("export MediumSample volume_interval_coefficients(uint id,float3 o,float3 d,float distance,float fraction,MediumTransportContext context) {switch(id) {\n");
    let mut emitting_dispatch=String::from("export bool volume_emitting(uint input) {switch(input-medium_frame().medium.x) {\n");
    let mut homogeneous_dispatch=String::from("export bool volume_homogeneous(uint id) {switch(id) {\n");
    let mut programs = BTreeMap::new();
    // A majorant is stored beside its existing input words only when a sampled
    // consumer needs heterogeneous tracking. Homogeneous/raster installs retain
    // their original layout and allocate no tracking metadata.
    let compiled=names.keys().map(|name|Ok((name.clone(),compile(r,scene,name,base)?))).collect::<Result<BTreeMap<_,_>>>()?;
    native.world_only=stages.values().all(|s|!s.medium);
    if native.world_only {
        if let Some(name)=world {
            let (program,inputs)=&compiled[name];
            let dynamic=animated.get(name).cloned().unwrap_or_default();
            let proof=program.inputs.varying_words(inputs,&dynamic);
            if let Some((a,b,g))=program.connection_profile_for_inputs(&programs::abi(),&proof)? {
                native.surface_connection=Some(words.len() as u32);
                words.extend(a.into_iter().chain(b).chain(g).map(f32::to_bits));
            }
        }
    }
    native.identity.push(u8::from(native.surface_connection.is_some()));
    native.homogeneous=compiled.values().all(|(p,_)|p.homogeneous);
    native.scattering_free=compiled.values().all(|(p,_)|p.scattering_free);
    native.time_dependent=compiled.values().any(|(p,_)|p.time_dependent);
    let tracking=!native.homogeneous && scene.render.settings.integrator.is_path();
    native.tracking=tracking;
    native.point.push_str("import vork_medium_point;\n");
    let exponential=world.and_then(|name| {let program=&compiled[name].0;(!program.homogeneous).then_some(program.exponential).flatten()});
    let world_exponential=if let Some((a,b,g))=exponential {
        let v=|v:[f32;3]|format!("float3({:.9e},{:.9e},{:.9e})",v[0],v[1],v[2]);
        format!("a={};b={};g={};return true;",v(a),v(b),v(g))
    } else {"a=0.;b=0.;g=0.;return false;".to_owned()};
    native.point.push_str(&format!("public bool path_world_exponential(out float3 a,out float3 b,out float3 g) {{{world_exponential}}}\n"));

    let mut path_homogeneous=String::from("public bool path_medium_homogeneous(uint id) {switch(id) {\n");
    for (name, offsets) in &mut names {
        let (program,inputs)=&compiled[name];

        if !program.homogeneous {
            if Some(name.as_str())==world && program.exponential.is_none() {native.sampled_error=Some(format!("sampled heterogeneous world medium {name} needs a finite domain; bind it to a closed mesh boundary for sampled transport; unbounded varying world media require raster transport (vork help profiles)"));}
        }
        if tracking {words.push(0);}
        let base = words.len() as u32;
        words.extend_from_slice(inputs);
        // Input bases also identify instances in emitting dispatch. Empty layouts
        // must own a word so the next instance cannot reuse this identity.
        if inputs.is_empty() { words.push(0); }
        emitting_dispatch.push_str(&format!("case {base}u: return {};\n",!program.emission_free));
        if animated.contains_key(name) {
            input_bindings.push((name.clone(), base as usize, program.inputs.clone(), 0.));
        }
        if let Some(&id) = programs.get(&program.native_point) {
            *offsets = (id, base);
            continue;
        }
        let id = programs.len() as u32;
        programs.insert(program.native_point.clone(), id);
        *offsets = (id, base);
        let point=native_coefficients(&program,id,true,&mut native);
        native.point.push_str(&point);native_code.push_str(&point);
        path_homogeneous.push_str(&format!("case {id}u: return {};\n",program.homogeneous));
        interval_dispatch.push_str(&format!("case {id}u: return volume_{id}_interval_point(o,d,distance,fraction,context);\n"));
        homogeneous_dispatch.push_str(&format!("case {id}u: return {};\n",program.homogeneous));
        native_dispatch.push_str(&format!(
            "case {id}u: return volume_{id}_medium_point(p,context);\n"
        ));
    }
    path_homogeneous.push_str("default: return true;}}\n");native.point.push_str(&path_homogeneous);
    native.identity.push(u8::from(native.homogeneous));
    if let Some(world) = world {
        (words[2], words[3]) = names[world];
    }
    native_code.push_str(&world.map(|name|tail_closure(&compiled[name].0)).unwrap_or_else(||"export float medium_world_tail_start(float3 o,float3 d) {return 0.;}\nexport MediumSample medium_world_tail_coefficients(float3 o,float3 d) {return MediumSample();}\nexport void medium_world_exponential(out float3 a,out float3 b,out float3 g) {a=0.;b=0.;g=0.;}\n".to_owned()));
    native_dispatch.push_str("default: return MediumSample();}}\n");
    native.point.push_str(&native_dispatch.replace("export MediumSample volume_coefficients","public MediumSample path_medium_coefficients"));
    native_code.push_str(&native_dispatch);
    interval_dispatch.push_str("default: return MediumSample();}}\n");native_code.push_str(&interval_dispatch);
    native_code.push_str("export MediumSample medium_world_coefficients(float3 p) {let base=medium_frame().medium.x;MediumTransportContext context={};context.input_base=base+medium_word(base+3u);context.inverse=float4x4(float4(1.,0.,0.,0.),float4(0.,1.,0.,0.),float4(0.,0.,1.,0.),float4(0.,0.,0.,1.));return volume_coefficients(medium_word(base+2u),p,context); }\nexport MediumTransport medium_transport_unbounded(float3 o,float3 d,float scale,float fraction) {return medium_transport_infinity(o,d,scale,fraction); }\n");
    homogeneous_dispatch.push_str("default: return false;}}\n");native_code.push_str(&homogeneous_dispatch);
    emitting_dispatch.push_str("default: return true;}}\n");native_code.push_str(&emitting_dispatch);
    native.transport=native_code.clone();
    Ok(super::Installed {
        shader: pipelines::replace_hook(RESOLVE, HOOK, &format!("{NATIVE_ACCESSORS}\n{native_code}"))?,
        native,
        words,
        instances: Installed { names, programs: compiled },
        inputs: input_bindings,
    })
}

impl super::Installed {
    pub(crate) fn domain_count(&self, draws: &[install::Draw], materials: &BTreeMap<String, usize>) -> usize {
        let bindings: std::collections::BTreeSet<_> = materials.iter()
            .filter_map(|(name,index)|self.instances.names.contains_key(name).then_some(*index)).collect();
        draws.iter().filter(|draw|bindings.contains(&draw.material)).map(|draw|draw.instances.len()).sum()
    }
    pub(crate) fn mesh_words(
        &mut self,
        _r: &Renderer,
        scene: &v2::Scene,
        draws: &[install::Draw],
        roots: &[install::MeshRoot],
        materials: &BTreeMap<String, usize>,
    ) -> Result<()> {
        let bounded=&self.instances;
        let bindings: BTreeMap<_, _> = materials
            .iter()
            .filter_map(|(name, index)| {
                bounded.names.get(name).map(|binding| (*index, (name.as_str(), *binding)))
            })
            .collect();
        let count=draws.iter().filter(|draw|bindings.contains_key(&draw.material))
            .map(|draw|draw.instances.len()).sum();
        admit_domains(count)?;
        let animated = crate::materials::animated_inputs(scene)?;
        if self.native.tracking {
            for (name, (_, base)) in &bounded.names {
                let (program, inputs)=&bounded.programs[name];
                if program.homogeneous {continue;}
                let mut bounds=(Vec3::splat(f32::INFINITY),Vec3::splat(f32::NEG_INFINITY));
                for draw in draws.iter().filter(|d|materials.get(name)==Some(&d.material)) {
                    bounds.0=bounds.0.min(draw.bounds.0);bounds.1=bounds.1.max(draw.bounds.1);
                }
                if bounds.0.is_finite() {
                    let dynamic=animated.get(name).cloned().unwrap_or_default();
                    let proof=grid_proof_words(program,inputs,&dynamic).0;
                    self.words[*base as usize-1]=bound::certify(program,&proof,bounds,bound::certification_times(scene)).with_context(||format!("medium shader {:?} (material {name:?}): cannot certify extinction majorant; use supported shader operations or the raster integrator",scene.materials[name].program))?.to_bits();
                }
            }
        }
        let mut descriptors = Vec::new();
        let mut phased = BTreeMap::new();
        for draw in draws {
            let Some(&(material_name, (program, inputs))) = bindings.get(&draw.material) else {
                continue;
            };
            ensure!(
                !draw.evaluated(),
                "deformed volume boundaries are not supported yet"
            );
            let geometry = draw
                .exit_geometry
                .as_ref()
                .context("volume requires retained mesh geometry")?;
            validate_boundary(geometry)?;
            ensure!(
                draw.count as usize == geometry.indices.len(),
                "volume boundaries require one closed material group"
            );
            let start = draw.primitive_start as usize * 3;
            let root = self.words.len() as u32;
            {
                let mut triangles: Vec<_> = geometry.indices[start..start + draw.count as usize]
                    .chunks_exact(3)
                    .map(|indices| [indices[0], indices[1], indices[2]].map(|i| Vec3::from_array(geometry.vertices[i as usize].POSITION)))
                    .collect();
                boundary_bvh(&mut self.words, &mut triangles);
            }
            let end = self.words.len() as u32;
            for object in draw.instances.clone() {
                let time_offset=roots[object as usize].time_offset;
                let input=if time_offset == 0. || !animated.contains_key(material_name) {inputs} else if let Some(&base)=phased.get(&(program,inputs,time_offset.to_bits())) {base} else {
                    let (name,_,layout,_)=self.inputs.iter().find(|(name,base,_,offset)|name == material_name && *base==inputs as usize && *offset==0.).context("medium input binding")?;
                    {
                        let (name,layout)=(name.clone(),layout.clone());
                        let compiled=&bounded.programs[&name];
                        let words=self.words[inputs as usize..inputs as usize+compiled.1.len()].to_vec();
                        if self.native.tracking && !compiled.0.homogeneous {self.words.push(self.words[inputs as usize-1]);}
                        let base=self.words.len() as u32;
                        self.words.extend(words);
                        self.inputs.push((name,base as usize,layout,time_offset));
                        phased.insert((program,inputs,time_offset.to_bits()),base);
                        base
                    }
                };
                descriptors.extend([
                    object,
                    root,
                    end,
                    program,
                    input,
                ]);
            }
        }
        self.words[0] = self.words.len() as u32;
        self.words[1] = (descriptors.len() / constants::MEDIUM_BOUND_WORDS) as u32;
        self.native.domains = self.words[1] as usize;
        self.words.extend(descriptors);
        Ok(())
    }
}

// Preorder, threaded BVH: min.xyz/escape, max.xyz/leaf count, followed by
// either two children or leaf triangles. Escape offsets make traversal stackless
// on every backend. A single small leaf avoids extra nodes for small boundaries.
fn boundary_bvh(words: &mut Vec<u32>, triangles: &mut [[Vec3; 3]]) {
    let at = words.len();
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    let mut centers_lo = lo;
    let mut centers_hi = hi;
    for triangle in triangles.iter() {
        let center = (triangle[0] + triangle[1] + triangle[2]) / 3.;
        centers_lo = centers_lo.min(center);
        centers_hi = centers_hi.max(center);
        for &p in triangle {lo = lo.min(p); hi = hi.max(p);}
    }
    words.extend(lo.to_array().map(|v| v.next_down().to_bits()));
    words.push(0);
    words.extend(hi.to_array().map(|v| v.next_up().to_bits()));
    words.push(0);
    if triangles.len() <= constants::MEDIUM_BVH_LEAF_TRIANGLES {
        words[at + 7] = triangles.len() as u32;
        for triangle in triangles.iter() {
            for p in triangle {words.extend(p.to_array().map(f32::to_bits));}
        }
    } else {
        let extent = centers_hi - centers_lo;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {0} else if extent.y >= extent.z {1} else {2};
        let mid = triangles.len() / 2;
        triangles.select_nth_unstable_by(mid, |a, b| {
            let center = |t: &[Vec3; 3]| (t[0][axis] + t[1][axis] + t[2][axis]) / 3.;
            center(a).total_cmp(&center(b))
        });
        let (left, right) = triangles.split_at_mut(mid);
        boundary_bvh(words, left);
        boundary_bvh(words, right);
    }
    words[at + 3] = words.len() as u32;
}

/// Weld attribute-split vertices by position before checking oriented edge incidence.
fn validate_boundary(geometry: &geometry::Geometry) -> Result<()> {
    let (edges, degenerate) = geometry::edge_uses(&geometry.indices, |index| geometry::weld_key(geometry.vertices[index as usize].POSITION));
    ensure!(!degenerate, "volume boundary contains a degenerate edge");
    ensure!(
        !edges.is_empty()
            && edges
                .values()
                .all(|&(count, winding)| count == 2 && winding == 0),
        "volume boundary must be a closed, consistently oriented manifold mesh"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cube() -> geometry::Geometry {
        let vertices = [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ]
        .into_iter()
        .map(|p| Vertex {
            POSITION: p,
            ..Vertex::zeroed()
        })
        .collect();
        geometry::Geometry::new(
            vertices,
            vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0,
                5, 4, 3, 7, 6, 3, 6, 2,
            ],
            0,
        )
    }
    #[test]
    fn threaded_boundary_bvh_preserves_triangles_and_prunes_large_mesh() {
        // An 81,920-triangle closed boundary, spread across disjoint cubes.
        // The ray only visits one cube; a triangle scan visits all of them.
        let mesh = cube();
        let original: Vec<_> = mesh.indices.chunks_exact(3).map(|i|
            [i[0], i[1], i[2]].map(|v| Vec3::from_array(mesh.vertices[v as usize].POSITION))).collect();
        let mut triangles = Vec::new();
        for i in 0..6826 {
            for t in &original {triangles.push(t.map(|p| p + Vec3::new(i as f32 * 3., 0., 0.)));}
        }
        for i in 0..2 {
            let offset = Vec3::new(21000. + i as f32 * 3., 0., 0.);
            let p = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z].map(|p| p + offset);
            for t in [[0,2,1], [0,1,3], [0,3,2], [1,2,3]] {triangles.push(t.map(|i| p[i]));}
        }
        assert_eq!(triangles.len(), 81_920);
        let mut words = vec![0; 4];
        boundary_bvh(&mut words, &mut triangles);
        let vec3 = |at| Vec3::from_array([0, 1, 2].map(|i| f32::from_bits(words[at+i])));
        let mut node = 4;
        let mut visited = 0;
        let mut leaves = 0;
        while node < words.len() {
            let escape = words[node+3] as usize;
            assert!(escape > node && escape <= words.len());
            let lo = vec3(node); let hi = vec3(node+4);
            if lo.x <= 0. && hi.x >= 0. && lo.y <= 0. && hi.y >= 0. && hi.z >= -2. {
                let count = words[node+7] as usize;
                if count == 0 {node += constants::MEDIUM_BVH_WORDS; continue;}
                visited += count;
                leaves += 1;
                for triangle in (node+constants::MEDIUM_BVH_WORDS..escape).step_by(9) {
                    for p in [vec3(triangle), vec3(triangle+3), vec3(triangle+6)] {
                        assert!(p.cmpge(lo).all() && p.cmple(hi).all());
                    }
                }
            }
            node = escape;
        }
        assert!(leaves > 0);
        assert!(visited < 100, "{visited} triangle candidates for {} triangles", triangles.len());
        // Every payload is retained exactly once, including winding.
        let mut rebuilt = Vec::new();
        let mut node = 4;
        while node < words.len() {
            let count = words[node+7] as usize;
            if count == 0 {node += constants::MEDIUM_BVH_WORDS; continue;}
            for i in 0..count {let t=node+constants::MEDIUM_BVH_WORDS+i*9;rebuilt.push([vec3(t),vec3(t+3),vec3(t+6)]);}
            node = words[node+3] as usize;
        }
        assert_eq!(rebuilt, triangles);
    }
    #[test]
    fn open_and_inconsistently_oriented_volume_boundaries_are_rejected() -> Result<()> {
        let mut mesh = cube();
        validate_boundary(&mesh)?;
        mesh.indices.swap(0, 1);
        assert!(validate_boundary(&mesh).is_err());
        mesh.indices.swap(0, 1);
        mesh.indices.truncate(mesh.indices.len() - 3);
        assert!(validate_boundary(&mesh).is_err());
        Ok(())
    }
    fn std_boundary(builder: &str, mut options: serde_json::Value) -> geometry::Geometry {
        options["name"] = "test".into();
        let fragment: serde_json::Value =
            serde_json::from_str(&vork_std::evaluate(builder, &options.to_string()).unwrap())
                .unwrap();
        let positions: Vec<[f32; 3]> =
            serde_json::from_value(fragment["buffers"]["test_position"]["data"].clone()).unwrap();
        let indices =
            serde_json::from_value(fragment["buffers"]["test_indices"]["data"].clone()).unwrap();
        geometry::Geometry::new(
            positions
                .into_iter()
                .map(|position| Vertex {
                    POSITION: position,
                    ..Vertex::zeroed()
                })
                .collect(),
            indices,
            0,
        )
    }

    #[test]
    fn closed_std_primitives_are_exact_volume_boundaries() {
        use serde_json::json;
        for r in [1., 6e6] {
            for segments in [3, 4, 7, 16, 31, 32] {
                for rings in [2, 3, 8] {
                    for (builder, options) in [
                        ("box", json!({"size":[r,2.*r,3.*r]})),
                        (
                            "sphere",
                            json!({"radius":r,"segments":segments,"rings":rings}),
                        ),
                        (
                            "cylinder",
                            json!({"radius":r,"height":3.*r,"segments":segments}),
                        ),
                        (
                            "cone",
                            json!({"radius":r,"height":3.*r,"segments":segments}),
                        ),
                        (
                            "torus",
                            json!({"radius":r,"tube":r/4.,"segments":segments,"rings":rings+1}),
                        ),
                        (
                            "capsule",
                            json!({"radius":r,"height":3.*r,"segments":segments,"rings":rings}),
                        ),
                        (
                            "capsule",
                            json!({"radius":r,"height":2.*r,"segments":segments,"rings":rings}),
                        ),
                        (
                            "extrude",
                            json!({"outline":[[0.,0.],[r,0.],[r,r],[0.,r]],"depth":r}),
                        ),
                    ] {
                        validate_boundary(&std_boundary(builder, options.clone()))
                            .unwrap_or_else(|e| panic!("{builder} {options}: {e}"));
                    }
                    for mut profile in [
                        vec![[0., r], [r, 0.], [0., -r]],
                        vec![[r, 0.], [2. * r, 0.], [2. * r, r], [r, r], [r, 0.]],
                    ] {
                        for _ in 0..2 {
                            let options = json!({"profile":profile,"segments":segments});
                            validate_boundary(&std_boundary("revolve", options.clone()))
                                .unwrap_or_else(|e| panic!("revolve {options}: {e}"));
                            profile.reverse();
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn std_open_rims_and_real_cracks_still_fail() {
        use serde_json::json;
        for (builder, options) in [
            ("grid", json!({})),
            (
                "revolve",
                json!({"profile":[[1.,1.],[1.,-1.]],"segments":7}),
            ),
        ] {
            assert!(validate_boundary(&std_boundary(builder, options)).is_err());
        }
        let mut mesh = std_boundary("sphere", json!({"segments":7,"rings":3}));
        // Move just one UV seam vertex by a representable amount, leaving an authored crack.
        mesh.vertices[8].POSITION[2] += 1e-7;
        assert!(validate_boundary(&mesh).is_err());
    }
    #[test]
    #[ignore = "requires pinned Slang and GPU; lavapipe works"]
    fn bounded_transport_matches_beer_lambert_for_inside_clipped_scaled_and_overlapping_meshes()
    -> Result<()> {
        let renderer = Renderer::new()?;
        let device = &renderer.device;
        let mut scene: v2::Scene =
            vork_scene::v2::from_test_value(vork_scene::v2::with_test_settings(include_str!("../tests/fixtures/volume-glass-box.json"), include_str!("../tests/presets/volume-glass-box-c5ca7d6d.json"))?)?;
        scene.materials.get_mut("smoke").unwrap().program = "std/fog".into();
        let stages = programs::mesh_stages(&renderer, &scene, Path::new("."))?;
        scene.materials.get_mut("smoke").unwrap().inputs = serde_json::from_value(
            serde_json::json!({"density":0.5,"scattering":[0.2,0.4,0.6],"absorption":[0.8,0.6,0.39999998],"emission":[2,0,0]}),
        )?;
        scene
            .materials
            .insert("world_test".into(), scene.materials["smoke"].clone());
        let multiple = install(&renderer, &scene, Some("world_test"), Path::new("."), &stages)?;
        assert_eq!(
            multiple.instances.names["smoke"].0,
            multiple.instances.names["world_test"].0,
            "instances share one compiled medium program"
        );
        for metal in [false, true] {
            passes::resolve::compile_native(
                &multiple.shader,
                &multiple.native,
                "volume_lighting",
                metal,
            )?;
        }
        scene.materials.get_mut("world_test").unwrap().inputs.insert("height_falloff".into(),serde_json::from_value(serde_json::json!(0.1))?);
        let mixed=install(&renderer,&scene,Some("world_test"),Path::new("."),&stages)?;
        assert!(multiple.native.homogeneous);
        assert!(!mixed.native.homogeneous);
        assert_ne!(mixed.instances.names["smoke"].0,mixed.instances.names["world_test"].0,
            "homogeneous and varying instances share source but need distinct kernel variants");
        for metal in [false,true] {passes::resolve::compile_native(&mixed.shader,&mixed.native,"volume_lighting",metal)?;}
        // Exercise both specializations of the shared kernel, including the
        // camera cursor module and an independently traversed light connection.
        for installed in [&multiple,&mixed] {
            let mut native=installed.native.clone();native.self_shadows=true;native.boundaries=2;
            for metal in [false,true] {
                passes::resolve::compile_native(&installed.shader,&native,"volume_lighting",metal)?;
            }
        }
        scene.programs.insert("world_points".into(), serde_json::from_value(serde_json::json!({"source":"import blender.volume; struct Inputs {}; MediumCoefficients medium(MediumContext ctx, Inputs inputs) { MediumPoint m;m.density=0.01;return principled_volume(m);}"}))?);
        scene.materials.get_mut("world_test").unwrap().program = "world_points".into();
        scene
            .materials
            .get_mut("world_test")
            .unwrap()
            .inputs
            .clear();
        let distinct = install(&renderer, &scene, Some("world_test"), Path::new("."), &stages)?;
        for metal in [false, true] {
            passes::resolve::compile_native(
                &distinct.shader,
                &distinct.native,
                "volume_lighting",
                metal,
            )?;
        }
        // The varying variant is constant along these y=0 rays, giving the same
        // independent oracle while exercising its per-cell integration path.
        for (density, height_falloff) in [(0.5_f64, 0.), (1e-7, 0.), (1e-7, 0.1)] {
        for (name,value) in [("density",density),("height_falloff",height_falloff)] {
            scene.materials.get_mut("smoke").unwrap().inputs.insert(name.into(),serde_json::from_value(serde_json::json!(value))?);
        }
        let mut installed = install(&renderer, &scene, None, Path::new("."), &stages)?;
        installed.native.boundaries = 2;
        // Lighting takes one point step per froxel cell. Keep the original
        // 64-step segment oracle too, without multiplying its discretization
        // into 4096 substeps when checking column composition.
        let segment_steps = installed.native.steps;
        for steps in [1, segment_steps] {
        installed.native.steps = steps;
        let (program, inputs) = installed.instances.names["smoke"];
        let mesh = cube();
        let mut words = Vec::new();
        let mut stride = 0;
        for case in 0..8 {
            let mut table = installed.words.clone();
            let triangle_offset = table.len() as u32;
            let mut triangles: Vec<_> = mesh.indices.chunks_exact(3).map(|i|
                [i[0], i[1], i[2]].map(|v| Vec3::from_array(mesh.vertices[v as usize].POSITION))).collect();
            boundary_bvh(&mut table, &mut triangles);
            let boundary_end = table.len() as u32;
            table[0] = table.len() as u32;
            table[1] = if case == 5 || case == 7 { 2 } else { 1 };
            table.extend([
                u32::from(case == 4),
                triangle_offset,
                boundary_end,
                program,
                inputs,
            ]);
            table.extend([u32::from(case == 7), triangle_offset, boundary_end, program, inputs]);
            stride = table.len();
            words.extend(table);
        }
        let code=installed.shader.split_once(NATIVE_ACCESSORS).unwrap().1
            .strip_suffix(RESOLVE.split_once(HOOK).unwrap().1).unwrap();
        let source = format!(r#"
import vork;
import vork_constants;
import vork_prelude;
import vork_medium;
static Frame frame={{}};
[[vk::binding(0,0)]] StructuredBuffer<uint> words;
[[vk::binding(1,0)]] StructuredBuffer<Object> objects;
[[vk::binding(2,0)]] RWStructuredBuffer<float4> values;
export float3 medium_visibility(float3 position,uint light_index) {{return 1.;}}
export Frame medium_frame() {{return frame;}}
export uint medium_word(uint index) {{return words[index];}}
export Light medium_light(uint index) {{Light light={{}};return light;}}
export uint medium_light_list(float3 position,out uint base) {{base=0u;return 0u;}}
export uint medium_light_index(uint base,uint next) {{return next;}}
export uint native_material_word(uint index) {{return words[index];}}
{code}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {{
frame.medium.x=id.x*{stride}u;frame.medium.w=1u;frame.radiance=float4(2.,3.,4.,0.);
float3 origin=float3(0.,0.,2.);float distance=4.;
if(id.x==1u) {{origin=float3(0.);}}if(id.x==2u) {{origin.x=2.;}}if(id.x==3u) {{distance=1.75;}}if(id.x>=6u) {{origin=float3(0.);distance=.25;}}
var m=MediumTransport();
if(id.x>=6u) {{m=medium_transport_segment(origin,float3(0.,0.,-1.),medium_receiver(distance),.5,0u);}}
else {{m=medium_transport_segment(origin,float3(0.,0.,-1.),medium_receiver(distance),.5,0xffffffffu);}}
values[id.x*3u]=float4(m.transmission,0.);values[id.x*3u+1u]=float4(m.source,0.);
#if VORK_MEDIUM_STEPS == 1
if(id.x<6u) {{
    medium_column_begin(origin);
    var prefix=MediumTransport(float3(0.),float3(1.),float3(0.));
    for(uint cell=0u;cell<64u;cell++) {{
        let part=medium_transport_range(origin,float3(0.,0.,-1.),distance*float(cell)/64.,distance*float(cell+1u)/64.,.5);
        prefix.source+=prefix.transmission*part.source;prefix.transmission*=part.transmission;
    }}
    values[id.x*3u+2u]=float4(max(abs(prefix.transmission-m.transmission),abs(prefix.source-m.source)),0.);
}}
#endif
}}"#);
        let entries: Vec<_> = (0..3).map(|binding| pipelines::buffer_entry(binding,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only: binding!=2})).collect();
        let pipeline = test_pipeline(&renderer,&source,&installed.native,&entries, (1, 1, 1))?;
        let objects = [
            Object::new(Mat4::IDENTITY).gpu(),
            Object::new(Mat4::from_scale(Vec3::new(2., 1., 3.))).gpu(),
        ];
        let input = init(
            device,
            "volume cases",
            bytemuck::cast_slice(&words),
            wgpu::BufferUsages::STORAGE,
        );
        let objects = init(
            device,
            "volume transforms",
            bytemuck::cast_slice(&objects),
            wgpu::BufferUsages::STORAGE,
        );
        let bytes = 8 * 3 * 16;
        let output = crate::gpu_memory::buffer(
            &device,
            &wgpu::BufferDescriptor {
                label: None,
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        );
        let readback = crate::gpu_memory::buffer(
            &device,
            &wgpu::BufferDescriptor {
                label: None,
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, input.as_entire_binding()),
                entry(1, objects.as_entire_binding()),
                entry(2, output.as_entire_binding()),
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(8, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        let submission = renderer.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read, .., move |result| {
            let _ = tx.send(result);
        });
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        rx.recv()??;
        let mapped = readback.get_mapped_range(..);
        let results: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        for (case, length) in [1.0f32, 0.5, 0., 0.25, 3., 2., 0.25, 0.5].into_iter().enumerate() {
            if case < 6 && steps == 1 {
                assert!(results[case*3+2][..3].iter().all(|v| *v < 0.0002), "streamed intervals case {case}: {:?}", results[case*3+2]);
            }
            let t = (-density * f64::from(length)).exp() as f32;
            for channel in 0..3 {
                assert!(
                    (results[case * 3][channel] - t).abs() < 0.0001,
                    "case {case}: {:?} != {t}",
                    results[case * 3]
                );
            }
            // Combined extinction weights both emission and coloured scattering,
            // including two coincident media and nonuniform world transforms.
            let integral = -(-density * f64::from(length)).exp_m1() / density;
            for (channel, scattering) in [0.2*2., 0.4*3., 0.6*4.].into_iter().enumerate() {
                let source = (if channel == 0 {2.} else {0.}) + density * scattering;
                assert!((results[case * 3 + 1][channel] - (source * integral) as f32).abs() < 0.0002,
                    "case {case}, channel {channel}: {:?}", results[case * 3 + 1]);
            }
        }
        }
        }
        Ok(())
    }
}

// Column cursors and coefficient dispatch scale with the total installed domain
// count, even when the boundaries are disjoint. Admit the worst case explicitly.
fn admit_domains(count: usize) -> Result<()> {
    ensure!(count <= 256, "media require {count} bounded domains; the supported work budget is 256 (including possible overlaps); reduce or combine medium boundaries");
    Ok(())
}
#[cfg(test)]
mod admission_tests {
    #[test]
    fn excessive_domains_refuse_instead_of_truncating_transport() {
        assert!(super::admit_domains(256).is_ok());
        let error=super::admit_domains(257).unwrap_err().to_string();
        assert!(error.contains("257") && error.contains("reduce or combine"));
    }
}

impl super::Installed {
    pub(crate) fn dense_bindings(&self,scene:&v2::Scene,medium_base:u32,draws:&[install::Draw],materials:&BTreeMap<String,usize>)->Result<(Vec<(String,u32)>,Vec<crate::dense::Certificate>)> {
        let mut destinations=Vec::new();let mut certificates=Vec::new();
        let animated=crate::materials::animated_inputs(scene)?;
        let copies=self.instances.names.iter().map(|(name,(_,base))|(name,*base))
            .chain(self.inputs.iter().filter(|(_,_,_,time)|*time!=0.).map(|(name,base,_,_)|(name,*base as u32)));
        for (name,base) in copies {
            let (program,inputs)=&self.instances.programs[name];
            let (proof,proof_slots)=grid_proof_words(program,inputs,&animated.get(name).cloned().unwrap_or_default());
            let mut grids=Vec::new();
            for field in &program.inputs.fields {
                if !matches!(field.ty,vork_slang::Type::FloatGrid|vork_slang::Type::VectorGrid) {continue;}
                let Some(v2::Input::Grid(binding))=scene.materials[name].inputs.get(&field.name) else {continue;};
                if scene.buffers[&binding.grid].dims.is_none() {continue;}
                let slot=inputs[field.offset] as usize;
                destinations.push((binding.grid.clone(),medium_base+base+slot as u32+4));
                grids.push((binding.grid.clone(),proof_slots[&field.name]));
            }
            if self.native.tracking && !grids.is_empty() && !program.homogeneous {
                let mut bounds=(Vec3::splat(f32::INFINITY),Vec3::splat(f32::NEG_INFINITY));
                for draw in draws.iter().filter(|d|materials.get(name)==Some(&d.material)) {bounds.0=bounds.0.min(draw.bounds.0);bounds.1=bounds.1.max(draw.bounds.1);}
                if bounds.0.is_finite() {
                    certificates.push(crate::dense::Certificate {program:program.clone(),words:proof,grids,destination:medium_base+base-1,bounds,times:super::bound::certification_times(scene)});
                }
            }
        }
        Ok((destinations,certificates))
    }
}

/// The certificate consumes grid ranges, never voxel payloads. Remap only its
/// private resource handles to compact metadata; GPU input addresses stay intact.
fn grid_proof_words(program:&vork_slang::MediumProgram,inputs:&[u32],dynamic:&std::collections::BTreeSet<String>)->(Vec<Option<u32>>,BTreeMap<String,usize>) {
    let mut words=program.inputs.varying_words(&inputs[..program.inputs.words],dynamic);
    let mut slots=BTreeMap::new();
    for field in &program.inputs.fields {
        if !matches!(field.ty,vork_slang::Type::FloatGrid|vork_slang::Type::VectorGrid) {continue;}
        let source=inputs[field.offset] as usize;
        let slot=words.len();
        words[field.offset]=Some(slot as u32);
        if inputs.get(source)==Some(&u32::MAX) {
            words.extend(inputs[source..source+6].iter().copied().map(Some));
        } else {
            // Sparse interpolation still has no enclosure. Keep the resource
            // unknown rather than substituting a synthetic empty grid.
            words.push(None);
        }
        slots.insert(field.name.clone(),slot);
    }
    (words,slots)
}

#[cfg(test)]
mod empty_input_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_medium_input_bases_remain_distinct() -> Result<()> {
        let Some(r) = crate::test_renderer()? else { return Ok(()); };
        for path in [false, true] {
            for pair in [["empty_a", "empty_b"], ["empty_a", "animated"], ["animated", "empty_b"]] {
                let mut value = json!({"version":2,"render":{"resolution":[4,4],"duration":1},
                    "nodes":[{"type":"camera"}],
                    "programs":{
                        "empty_a":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i){MediumCoefficients m={};m.absorption=float3(.1);return m;}"},
                        "empty_b":{"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i){MediumCoefficients m={};m.emission=float3(.2);return m;}"},
                        "animated":{"source":"struct Inputs {float density=.3;}; MediumCoefficients medium(MediumContext c,Inputs i){MediumCoefficients m={};m.absorption=float3(i.density);return m;}"}},
                    "materials":{"a":{"program":pair[0]},"b":{"program":pair[1]}}});
                value["render"]["settings"] = json!({"integrator":if path {"path"} else {"raster"},"samples":1,"denoiser":null});
                if let Some(index) = pair.iter().position(|name| *name == "animated") {
                    value["animation"] = json!([{"tracks":[{"path":["materials",if index==0 {"a"} else {"b"},"inputs","density"],"keys":[[0,0.3],[1,0.6]]}]}]);
                }
                let scene: v2::Scene = v2::from_test_value(value)?;
                let stages = ["a", "b"].into_iter().map(|name|(name.to_owned(),programs::Stages {medium:true,..Default::default()})).collect();
                let installed = install(&r,&scene,None,Path::new("."),&stages)?;
                let a = installed.instances.names["a"].1;
                let b = installed.instances.names["b"].1;
                assert_ne!(a,b,"{pair:?}, path={path}");
                assert!((b as usize) < installed.words.len());
                for (name, base, _, _) in &installed.inputs {
                    assert_eq!(*base,installed.instances.names[name].1 as usize);
                }
            }
        }
        Ok(())
    }
}
