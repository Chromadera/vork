//! World-medium install; one reflected word buffer and the existing pipeline/cache path.
use super::*;
use crate::constants::FROXEL_ELEMENT_BYTES;
use ::vork_scene::v2;
use vork_slang::{Compiler, MediumProgram};
const HOOK: &str = "// WORLD MEDIUM: medium.rs installs the volume integration entries here.";

#[derive(Clone, Default)]
pub(crate) struct Native {
    /// Resource-free point dispatch over the same reflected programs/words.
    pub point: String,
    /// Existing resource-free segment service, linked by secondary camera consumers.
    pub transport: String,
    /// Boundary interval slots in a scene-specific lighting variant.
    pub boundaries: usize,
    /// Installed mesh domains; one occupancy bit per domain in ray intervals.
    pub domains: usize,
    pub world_only: bool,
    /// Relative material-word offset of a proven constant/exponential World profile.
    pub surface_connection: Option<u32>,
    pub tracking: bool,
    pub sampled_error: Option<String>,
    pub modules: Vec<(String, String)>,
    pub includes: Vec<PathBuf>,
    pub identity: Vec<u8>,
    pub steps: u32,
    pub homogeneous: bool,
    pub time_dependent: bool,
    pub dense_grids_only: bool,
    pub tile: u32,
    pub sample_pages: u32,
    /// Maximum volume-capable light records, including animated visibility.
    pub visibility_lights: u32,
    /// The installed light plan proves no analytic source can become active.
    /// False keeps generic/test consumers conservative until installation.
    pub analytic_lights_absent: bool,
    /// All installed medium instances prove zero scattering for every input pose.
    pub scattering_free: bool,
    pub slices: u32,
    pub froxel_page_elements: u32,
    pub froxel_pages: u32,
    pub distribution: f32,
    pub ray_shadows: bool,
    pub mesh_sources: bool,
    pub program_coverage: bool,
    pub shadow_capacity: u32,
    pub implicit: bool,
    pub self_shadows: bool,
    pub shadow_samples: u32,
    pub light_clamp: f32,
}
impl Native {
    /// Match installed shadow closures for native volume entries, including table-only surfaces.
    pub(crate) fn link_shadow_programs(&mut self, programs: Option<&programs::Native>, transmission: bool) {
        if !self.ray_shadows { return; }
        let programs = programs.filter(|p| p.coverage || transmission);
        if programs.is_none() && !transmission { return; }
        self.program_coverage = true;
        if let Some(programs) = programs {
            self.modules.extend(programs.modules.clone());
            self.modules.push(("scene_programs".into(), programs.code.replace("struct Programs :", "struct ScenePrograms :")));
            self.includes.extend(programs.include.clone());
        } else {
            self.modules.push(("scene_programs".into(), "import surface; typedef NoPrograms ScenePrograms;".into()));
        }
    }

    pub(crate) fn froxel_bytes(&self, [w, h]: [u32; 2]) -> u64 {
        u64::from(w.div_ceil(self.tile.max(1)))
            * u64::from(h.div_ceil(self.tile.max(1)))
            * u64::from(self.slices.max(1) + 2)
            * (2 * FROXEL_ELEMENT_BYTES)
    }
    pub(crate) fn configure_storage(
        &mut self,
        scene: &v2::Scene,
        limits: &wgpu::Limits,
    ) -> Result<()> {
        // Camera selection (including imported cameras) happens after media
        // installation. Reserve the maximum visibility budget here; each view
        // allocates its actual camera-specific count below.
        let animated_shutter = scene.track_targets()?.iter().flatten().any(|target|
            matches!(target, v2::TrackTarget::Render(v2::RenderProperty::MotionBlurShutter)));
        let (bytes,sample_bytes)=std::iter::once((false,scene.render.clone()))
            .chain(scene.images.values().filter_map(|i|i.render.as_ref()).map(|r|(true,r.resolve(&scene.render))))
            .filter(|(_,render)|!render.settings.integrator.is_path())
            .map(|(capture,render)| {
                let extent=render.resolution;
                let (_,samples)=crate::plan::sampling_counts(&render,!capture && !animated_shutter);
                (self.froxel_bytes(extent),self.sample_bytes(extent,samples))
            }).fold((0,0),|(grid,samples),(a,b)|(grid.max(a),samples.max(b)));
        // Each address is a float4. Page boundaries may cut a prefix column:
        // accessors preserve the logical address, so scans need no boundary halo.
        let capacity = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size)
            / FROXEL_ELEMENT_BYTES;
        ensure!(
            capacity > 0,
            "device cannot bind a volume transport element"
        );
        self.froxel_page_elements = capacity.min(u64::from(u32::MAX)) as u32;
        let page_bytes=u64::from(self.froxel_page_elements)*FROXEL_ELEMENT_BYTES;
        let pages=|bytes:u64,label:&str| -> Result<u32> {
            ensure!(bytes/FROXEL_ELEMENT_BYTES<=u64::from(u32::MAX),"{label} exceeds 32-bit element addressing");
            Ok(u32::try_from(bytes.div_ceil(page_bytes))?)
        };
        self.froxel_pages=pages(bytes,"volume transport")?;
        self.sample_pages=pages(sample_bytes,"volume samples")?;
        self.identity.extend_from_slice(&self.froxel_page_elements.to_le_bytes());
        self.identity.extend_from_slice(&self.froxel_pages.to_le_bytes());
        self.identity.extend_from_slice(&self.sample_pages.to_le_bytes());
        self.identity.extend_from_slice(&self.visibility_lights.to_le_bytes());
        Ok(())
    }
    pub(crate) fn sample_bytes(&self, [w,h]: [u32;2], samples:u32) -> u64 {
        let field=u64::from(vork_slang::registry::MEDIUM_VISIBILITY_SIDE).pow(3)*u64::from(self.visibility_lights);
        (u64::from(w)*u64::from(h)*(u64::from(samples.max(1))+1)+field)*FROXEL_ELEMENT_BYTES
    }
    pub(crate) fn paged_source(&self, source: &str) -> String {
        let source=source.replace("// FROXEL_VIEW",FROXEL_VIEW);
        let source=page_source(&source,FROXEL_STORAGE,"medium_froxel",self.froxel_pages,self.froxel_page_elements,false,crate::resolve_native::FROXEL_PAGES,"VORK_FROXEL_PAGES_REGISTER");
        page_source(&source,"// SAMPLE_STORAGE","volume_sample",self.sample_pages,self.froxel_page_elements,true,crate::resolve_native::SAMPLE_PAGES,"VORK_SAMPLE_PAGES_REGISTER")
    }
    pub(crate) fn defines(&self) -> Vec<String> {
        // Precision is a consumer contract, not a volume capability: sampled
        // path kernels install MEDIUM_FP_FLAGS in SlangForward. Raster lighting
        // and receiver quadrature do not consume certified tracking bounds.
        vec![format!("-DVORK_SHADOW_CAPACITY={}",self.shadow_capacity),
            format!("-DVORK_MEDIUM_NO_ANALYTIC_LIGHTS={}",u8::from(self.analytic_lights_absent)),
            format!("-DVORK_HOMOGENEOUS_MEDIA={}",u8::from(self.homogeneous)),
            format!("-DVORK_MEDIUM_SCATTERING_FREE={}",u8::from(self.scattering_free)),
            format!("-DVORK_DENSE_GRIDS_ONLY={}",u8::from(self.dense_grids_only)),
            format!("-DVORK_MEDIUM_BOUNDARIES={}",self.boundaries),
            format!("-DVORK_MEDIUM_DOMAINS={}",self.domains),
            format!("-DVORK_MEDIUM_WORLD_ONLY={}",u8::from(self.world_only)),
            format!("-DVORK_VOLUME_SHADOWS={}",u8::from(self.self_shadows)),
            format!("-DVORK_VOLUME_SHADOW_SAMPLES={}",self.shadow_samples.max(1)),
            format!("-DVORK_VOLUME_LIGHT_CLAMP={:e}",self.light_clamp),
            format!("-DVORK_VOLUME_TILE={}",self.tile.max(1)),
            format!("-DVORK_VOLUME_SLICES={}",self.slices.max(1)),
            format!("-DVORK_VOLUME_DISTRIBUTION={:e}",self.distribution)]
    }
}
pub(crate) struct Installed {
    pub native: Native,
    pub shader: String,
    pub words: Vec<u32>,
    pub instances: bounded::Installed,
    pub inputs: Vec<(String, usize, vork_slang::InputLayout, f32)>,
}
pub(crate) fn install(
    r: &Renderer,
    scene: &v2::Scene,
    name: Option<&str>,
    base: &Path,
    stages: &std::collections::BTreeMap<String, programs::Stages>,
) -> Result<Option<Installed>> {
    if stages.values().any(|s| s.medium) {
        ensure!(
            scene.render.root.is_none()
                && scene.images.values().all(|image| image
                    .render
                    .as_ref()
                    .is_none_or(|render| render.root.is_none())),
            "bounded media in subtree renders are not supported yet"
        );
        fn validate_instances(stages: &std::collections::BTreeMap<String, programs::Stages>, nodes: &[v2::Node]) -> Result<()> {
            for node in nodes {
                if let v2::NodeKind::Mesh {
                    materials,
                    instances: Some(instances),
                    ..
                } = &node.kind
                {
                    ensure!(
                        instances.count.is_none()
                            || !materials.iter().any(|name| stages[name].medium),
                        "computed instance counts on volume boundaries are not supported yet"
                    );
                }
                validate_instances(stages, &node.children)?;
            }
            Ok(())
        }
        validate_instances(stages, &scene.nodes)?;
        return bounded::install(r, scene, name, base, stages).map(Some);
    }
    let Some(name) = name else {return Ok(None);};
    let installed = bounded::install(r, scene, Some(name), base, stages)?;
    if installed.instances.world_is_vacuum(scene, name)? {return Ok(None);}
    Ok(Some(installed))
}

pub(super) fn configure(native: &mut Native) {
    native.tile=constants::VOLUME_TILE_SIZE;
    native.slices=constants::VOLUME_SAMPLES;
    native.distribution=constants::VOLUME_DISTRIBUTION;
    native.self_shadows=true;
    native.shadow_samples=constants::VOLUME_SHADOW_SAMPLES;
    native.light_clamp=0.;
}

/// A medium program's compiled module (cached by source).
pub(crate) fn compiled(r: &Renderer, source: &str, origin: &Path) -> Result<MediumProgram> {
    let abi = programs::abi();
    let mut hash = blake3::Hasher::new();
    hash.update(abi.cache_identity()?.as_bytes());
    hash.update(b"native-medium-5");
    hash.update(source.as_bytes());
    hash.update(origin.to_string_lossy().as_bytes());
    let key = hash.finalize();
    let tag = key.to_hex();
    programs::cached(
        r,
        key,
        &Compiler::discover(),
        &abi, &source, &origin,
        |compiler| compiler.compile_medium(&abi, source, origin, &tag[..12]),
    )
}

pub(crate) fn compile(
    r: &Renderer,
    scene: &v2::Scene,
    name: &str,
    base: &Path,
) -> Result<(MediumProgram, Vec<u32>)> {
    let material = scene
        .materials
        .get(name)
        .with_context(|| format!("unknown world medium {name}"))?;
    let (source, origin) = if let Some(source) = vork_slang::module_source(&material.program) {
        (
            source.to_owned(),
            PathBuf::from(format!("{}.slang", material.program)),
        )
    } else {
        let program = scene
            .programs
            .get(&material.program)
            .with_context(|| format!("unknown medium program {}", material.program))?;
        programs::program_source(base, &material.program, program)?
    };
    let abi = programs::abi();
    let mut program = compiled(r, &source, &origin)?;
    if !program.warnings.trim().is_empty() {
        eprintln!("{}", program.warnings.trim());
    }
    let words = programs::pack_inputs(
        &program.inputs,
        &material.inputs.iter().map(|(name, value)| Ok((name.clone(), serde_json::to_value(value)?))).collect::<Result<_>>()?,
        scene,
        base,
        |name, _| bail!("medium image input {name} is unsupported"),
    )?;
    let dynamic = crate::materials::animated_inputs(scene)?.remove(name).unwrap_or_default();
    let proof = program.inputs.varying_words(&words, &dynamic);
    program.time_dependent=program.time_dependent_for_inputs(&abi,&proof)?;
    program.scattering_free=program.scattering_free_for_inputs(&abi,&proof)?;
    if !program.homogeneous && program.homogeneous_for_inputs(&abi,&proof)? {
        program.homogeneous=true;
        // Two kernel variants per module share the reflected material words.
        // Distinct bridge names let homogeneous and varying instances coexist.
        let entry=format!("{}_homogeneous",program.native_point);
        program.native=program.native.replace(&program.native_point,&entry);
        program.native_point=entry;
    }
    if !program.homogeneous {
        program.exponential=program.exponential_for_inputs(&abi,&proof)?;
        program.tail_slab=program.tail_slab_for_inputs(&abi,&proof)?;
        program.emission_free=program.emission_free_for_inputs(&abi,&proof)?;
        if let Some((a,b,g))=program.exponential {
            // Immutable coefficient specializations must remain distinct even
            // when they originate in the same authored module.
            let suffix=a.into_iter().chain(b).chain(g).map(|x|format!("{:08x}",x.to_bits())).collect::<String>();
            let entry=format!("{}_exponential_{suffix}",program.native_point);
            program.native=program.native.replace(&program.native_point,&entry);program.native_point=entry;
        }
    }
    Ok((program, words))
}

// Nonnegative distances begin a proven constant exterior. -1 retains the
// nonemissive approximation; -2 diagnoses an unsupported emissive tail.
fn tail_closure(program:&MediumProgram)->String {
    let mut sample="return medium_world_coefficients(o);".to_owned();
    let body=if program.homogeneous {"return 0.;".to_owned()}
    else if program.exponential.is_some() {"return -3.;".to_owned()}
    else if let Some((axis,bound))=program.tail_slab {
        // Probe the proven exterior directly. Reconstructing o+d*distance can
        // round back across a discontinuous coefficient boundary.
        sample=format!("var p=o;if(d[{axis}]!=0.) p[{axis}]=select(d[{axis}]>0.,{bound:.9e},-{bound:.9e});return medium_world_coefficients(p);");
        format!("if(d[{axis}]==0.) return 0.; return max(0.,(select(d[{axis}]>0.,{bound:.9e},-{bound:.9e})-o[{axis}])/d[{axis}]);")
    } else if program.emission_free {"return -1.;".to_owned()}
    else {"return -2.;".to_owned()};
    let exponential=if let Some((a,b,g))=program.exponential {
        let v=|v:[f32;3]|format!("float3({:.9e},{:.9e},{:.9e})",v[0],v[1],v[2]);
        format!("a={};b={};g={};",v(a),v(b),v(g))
    } else {"a=0.;b=0.;g=0.;".to_owned()};
    format!("export float medium_world_tail_start(float3 o,float3 d) {{{body}}}\nexport MediumSample medium_world_tail_coefficients(float3 o,float3 d) {{{sample}}}\nexport void medium_world_exponential(out float3 a,out float3 b,out float3 g) {{{exponential}}}\n")
}

#[cfg(test)]
fn shader(program: &MediumProgram, samples: u32) -> Result<(String, Native)> {
    let mut native = Native {steps:samples,..Native::default()};
    let transport = native_transport(program, 0, &mut native);
    let hook=format!("{NATIVE_ACCESSORS}\n{transport}");
    let source = pipelines::replace_hook(
        RESOLVE,
        HOOK,
        &hook,
    )?;
    Ok((source, native))
}

#[cfg(all(test, vork_private_tests))]
include!("medium_tests_private.rs");

#[cfg(test)]
#[path = "medium_numeric_tests.rs"]
mod numeric_tests;

#[path = "medium_bounded.rs"]
mod bounded;
#[path = "medium_bound.rs"]
pub(crate) mod bound;
pub(crate) use bound::certification_times;

const NATIVE_ACCESSORS: &str = r#"
import vork_material_table;
import vork;
import vork_medium;
[[vk::binding(43,0)]] StructuredBuffer<uint> medium_words;
export uint medium_word(uint index) {return medium_words[index];}
export Light medium_light(uint index) {return lights[index];}
export float3 medium_environment_radiance(float3 direction) {return reflection_fallback(direction,0.,frame,env_specular,env_sampler);}
[[vk::binding(64,0)]] ConstantBuffer<ClusterConfig> medium_clusters;
[[vk::binding(65,0)]] StructuredBuffer<uint> medium_cluster_lights;
export uint medium_light_list(float3 position,out uint base) {
    if (camera_cluster_base(position,frame,medium_clusters,base)) {return medium_cluster_lights[base];}
    base=REG_CLUSTER_UNCULLED;return uint(frame_light_count(frame));
}
export uint medium_light_index(uint base,uint next) {
    if (base==REG_CLUSTER_UNCULLED) {return next;}
    return medium_cluster_lights[base+REG_CLUSTER_LIST_HEADER+next];
}

import vork_constants;
#ifdef VORK_VOLUME_RAY
import vork_shadow_ray;
[[vk::binding(59,0)]] RaytracingAccelerationStructure medium_ray_scene;
[[vk::binding(60,0)]] StructuredBuffer<uint> medium_ray_alpha;
[[vk::binding(71,0)]] StructuredBuffer<float> medium_emitter_weights;
export uint shadow_ray_groups(uint index) {return objects[index].metadata.x;}
export uint shadow_ray_material(uint index) {return objects[index].material_index;}
export float3 ray_eye() {return frame.eye.xyz;}
#ifdef VORK_PROGRAM_COVERAGE
import scene_programs;
import ray_coverage;
import shadow_coverage;
import light_connection;
import hit;
import vork_ray_alpha;
#ifdef VORK_METAL
[[vk::binding(0,3)]] ParameterBlock<RayHitBuffersTable> medium_hit_buffers : register(VORK_HIT_REGISTER);
#else
[[vk::binding(0,3)]] StructuredBuffer<uint> medium_hit_buffers[REG_RAY_HIT_BUFFERS];
#endif
struct MediumGeometry : IHitGeometry {
    static uint word(uint index) {return medium_ray_alpha[index];}
    static uint load(uint buffer,uint index) {
#ifdef VORK_METAL
        return medium_hit_buffers.buffers[buffer][index];
#else
        return medium_hit_buffers[NonUniformResourceIndex(buffer)][index];
#endif
    }
};
export float3 shadow_ray_fields(RayDesc ray,uint groups,bool all_casters,uint sample_index) {
    return field_shadow_transmittance<ScenePrograms,MediumGeometry>(medium_ray_alpha,ray,groups,all_casters,sample_index);
}
export float3 shadow_ray_contained(RayDesc ray,bool all_casters,uint groups,uint sample_index) {
    return shadow_contained_transmittance<ScenePrograms,MediumGeometry>(medium_ray_scene,medium_ray_alpha,ray,all_casters,groups,float2(2.)/float2(medium_size()),sample_index);
}
export float3 shadow_ray_candidate(RayIntersection hit,float3x4 world_to_object,RayDesc ray,float distance,uint sample_index,uint sample_count) {
    return shadow_candidate_transmittance<ScenePrograms,MediumGeometry>(medium_ray_scene,medium_ray_alpha,hit,world_to_object,ray,distance,float2(2.)/float2(medium_size()),sample_index);
}
#endif
#endif
import vork_medium_shadow;
#ifdef VORK_MEDIUM_MESH_SOURCES
import vork_medium_mesh;
export float3 medium_mesh_illumination(float3 position,float3 ray,float g) {
    let f=medium_frame();
    return medium_mesh_incident<ScenePrograms,MediumGeometry>(medium_ray_scene,medium_ray_alpha,medium_emitter_weights,f.medium.y,position,ray,g,medium_shadow_pixel(position),uint(f.temporal.x),uint(f.temporal.y));
}
#endif
import vork_shadow_visibility;
// medium_shadow_bindings
import variant;
export float3 medium_visibility(float3 position,uint light_index) {
    // Each lattice uses its own stratum of the exposure's installed sequence.
    let temporal=medium_frame().temporal;
    let sample_index=uint(temporal.x);
#ifdef VORK_VOLUME_RAY
    let flags=lights[light_index].shadow_offsets[REG_LIGHT_RAY_WORD];
    if((flags&REG_LIGHT_RAY_SHADOWED)!=0u) {
        let light=light_values<Unspecialized>(light_index);
        let u=ray_emitter_sample(shadow_sequence,frame.integrator.w>>REG_INTEGRATOR_RAY_SEQUENCE_SHIFT,medium_shadow_pixel(position),light_index,sample_index,uint(temporal.y));
        Frame sample_frame=medium_frame();
        return shared_ray_transmission(medium_ray_scene,medium_ray_alpha,sample_frame,light,light_index,flags,lights[light_index].shadow_offsets[REG_LIGHT_RAY_LINK_WORD],position,float3(0.),sample_index,float3(0.),u);
    }
#endif
    return 1.;
}
// FROXEL_VIEW
import vork_froxel;
// FROXEL_STORAGE
// SAMPLE_STORAGE
import vork_medium_visibility_grid;
import vork_medium_visibility_direct;
import vork_medium_visibility_light;
export float4 medium_visibility_read(uint index) {return volume_sample_read(index);}
export void medium_visibility_write(uint index,float4 value) {volume_sample_write(index,value);}
export uint medium_visibility_sample() {return volume_sample_index.index.x;}
export Frame medium_visibility_frame() {return frame;}
[shader("compute")][numthreads(8,8,1)] void volume_visibility_world(uint3 id:SV_DispatchThreadID) {medium_visibility_light_world(id);}
[shader("compute")][numthreads(8,8,1)] void volume_apply(uint3 id:SV_DispatchThreadID) {
    let size=dimensions(color);if(any(id.xy>=size)) {return;}
    let pixel=id.y*size.x+id.x;let pixels=size.x*size.y;
    let index=volume_sample_index.index.x*pixels+pixel;
    let fraction=volume_sample_depth(index);
    let background=asuint(volume_sample_read(index).w)==0u;
    var m=froxel_transport(int2(id.xy),fraction,background);
    if(background && (!all(isfinite(m.source)) || !all(isfinite(m.transmission)))) {
        InterlockedMax(shader_errors[0],REG_PATH_MEDIUM_DIVERGENT);
        m=MediumTransport(float3(0.),float3(1.),float3(0.));
    }
    // In-scatter contributes once per sample, regardless of transparent layers.
    // Sum it in one pixel plane instead of retaining a source for every sample.
    let source_index=frame.sampling.w*pixels+pixel;
    var source=m.source;
    if(volume_sample_index.index.x>0u) {source+=volume_sample_read(source_index).rgb;}
    volume_sample_write(source_index,float4(source,0.));
    // The next visibility pass starts with an empty atomic nearest-depth word.
    volume_sample_write(index,float4(m.transmission,0.));
}
[shader("compute")][numthreads(8,8,1)] void volume_lighting(uint3 id:SV_DispatchThreadID) {froxel_light(id);}
export uint native_material_word(uint index) {return medium_words[index];}
"#;

/// Register one reflected point program and adapt world to medium coordinates.
fn native_coefficients(program: &MediumProgram,id:u32,bounded:bool,native:&mut Native) -> String {
    native.identity.extend_from_slice(program.native.as_bytes());
    let module = format!("medium_program_{id}");
    native
        .modules
        .push((module.clone(), program.native.clone()));
    for path in &program.native_includes {
        if !native.includes.contains(path) {
            native.includes.push(path.clone());
        }
    }
    let base="context.input_base";
    let point = &program.native_point;
    let position = if bounded {
        "mul(float4(position,1.),context.inverse).xyz"
    } else {
        "position"
    };
    let v=|v:[f32;3]|format!("float3({:.9e},{:.9e},{:.9e})",v[0],v[1],v[2]);
    // Sampled tracking is certified against the authored ordered operations.
    // Raster's log-space exponential rewrite is a different rounding graph.
    let exponential=program.exponential.filter(|_|!native.tracking);
    let evaluate=if let Some((a,b,g))=exponential {
        let (a,b,g)=(v(a),v(b),v(g));
        format!("let local={position};let m={point}_coefficients(float3(0.),frame.scene.x,{base},position,vork_path_wavelength);let log_density=dot({g},local);return MediumSample(medium_exponential_coefficient({a},log_density),medium_exponential_coefficient({b},log_density),m.emission,m.anisotropy);")
    } else {format!("let m={point}_coefficients({position},frame.scene.x,{base},position,vork_path_wavelength);return MediumSample(m.absorption,m.scattering,m.emission,m.anisotropy);")};
    let integrated=if let Some((a,b,g))=exponential {
        let (a,b,g)=(v(a),v(b),v(g));
        format!("let local=mul(float4(o,1.),context.inverse).xyz;let direction=mul(float4(d,0.),context.inverse).xyz;let log_density=dot({g},local);let slope=dot({g},direction);let m=volume_{id}_medium_point(o,context);return MediumSample(medium_exponential_tau({a},log_density,slope,distance,false)/distance,medium_exponential_tau({b},log_density,slope,distance,false)/distance,m.emission,m.anisotropy);")
    } else {format!("return volume_{id}_medium_point(o+d*(distance*fraction),context);")};
    format!("import vork_volume_settings;\nimport {module};\nMediumSample volume_{id}_medium_point(float3 position,MediumTransportContext context) {{{evaluate}}}\nMediumSample volume_{id}_interval_point(float3 o,float3 d,float distance,float fraction,MediumTransportContext context) {{{integrated}}}\n")
}

/// A fixture domain service: one known interval, with point bindings only.
/// The aggregate, lighting and attenuation code remains the shipping kernel.
#[cfg(test)]
fn numerical_domains(count:u32)->String {
    r#"import vork_medium_point;
import frame;
internal extern Frame medium_frame();
static MediumTransportContext numerical_context;
public void volume_numerical_context(MediumTransportContext context) {numerical_context=context;}
public uint volume_domain_count() {return DOMAIN_COUNTu;}
public float volume_interval_successor(float start) {return start;}
public MediumInstance volume_instance(uint index,float3 o,float3 d,float after,float limit,uint inside_object,bool camera_ray=true) {
    MediumInstance instance={};instance.program=index;instance.object=0xffffffffu;instance.context=numerical_context;
    instance.context.input_base=medium_frame().medium.x;
    instance.context.inverse=float4x4(float4(1.,0.,0.,0.),float4(0.,1.,0.,0.),float4(0.,0.,1.,0.),float4(0.,0.,0.,1.));
    return instance;
}
public float volume_interval_end(float3 o,float3 d,float after,float limit,uint inside_object,bool camera_ray=true) {return limit;}
public struct MediumBoundaryInterval {public float end;};
public MediumBoundaryInterval volume_boundary_interval(float3 o,float3 d,float after,float limit,uint inside_object,bool camera_ray) {
    MediumBoundaryInterval interval={};interval.end=limit;return interval;
}
public MediumInstance volume_interval_instance(uint index,MediumBoundaryInterval interval,float3 o,float3 d,float midpoint,float limit,uint inside_object,bool camera_ray) {
    return volume_instance(index,o,d,midpoint,limit,inside_object,camera_ray);
}
"#.replace("DOMAIN_COUNT",&count.to_string())
}

/// Numerical fixtures keep their point bindings but execute the production interval kernel.
#[cfg(test)]
fn native_transport(
    program: &MediumProgram,
    id: u32,
    native: &mut Native,
) -> String {
    let mut code = native_coefficients(program,id,false,native);
    native.point= format!("import vork_medium_point;\n{}\npublic MediumSample path_medium_coefficients(uint id,float3 p,MediumTransportContext context) {{return volume_{id}_medium_point(p,context);}}\n",code);
    native.homogeneous=program.homogeneous;
    native.world_only=true;
    native.modules.push(("vork_medium_domain".into(),numerical_domains(0)));
    native.point.push_str(&format!("public bool path_medium_homogeneous(uint id) {{return {};}}\n",program.homogeneous));
    code.push_str(&format!(r#"import vork_medium_bounds;
export MediumSample volume_coefficients(uint id,float3 p,MediumTransportContext context) {{return volume_{id}_medium_point(p,context);}}
export MediumSample volume_interval_coefficients(uint id,float3 o,float3 d,float distance,float fraction,MediumTransportContext context) {{return volume_{id}_interval_point(o,d,distance,fraction,context);}}
export bool volume_homogeneous(uint id) {{return {homogeneous};}}
float3 volume_{id}_optical_depth(float3 o,float3 d,float distance,MediumTransportContext context) {{volume_numerical_context(context);return volume_finite<VOLUME_SHADOW_SAMPLES,false>(o,d,medium_receiver(distance),context.sample_fraction,0xffffffffu,true).optical_depth;}}
MediumTransport volume_{id}_medium_transport(float3 o,float3 d,float distance,MediumTransportContext context) {{volume_numerical_context(context);return volume_finite<VORK_MEDIUM_STEPS,true>(o,d,medium_receiver(distance),context.sample_fraction,0xffffffffu,true);}}
"#,homogeneous=program.homogeneous));
    code.push_str(&tail_closure(program));
    code.push_str("export MediumSample medium_world_coefficients(float3 p) {MediumTransportContext context={};context.input_base=medium_frame().medium.x;return volume_coefficients(0u,p,context); }\nexport MediumTransport medium_transport_unbounded(float3 o,float3 d,float scale,float fraction) {return medium_world_tail<true>(o,d,scale,fraction); }\n");
    code
}

/// Numerical tests run the same native modules as production with tiny buffers.
/// Their entry is `numerical_test`: MSL reserves and renames `main`.
#[cfg(test)]
pub(crate) fn test_pipeline(r: &Renderer, source: &str, native: &Native, entries: &[wgpu::BindGroupLayoutEntry], threads: (u32, u32, u32)) -> Result<wgpu::ComputePipeline> {
    test_pipeline_defines(r,source,native,entries,threads,&[])
}
#[cfg(test)]
pub(crate) fn test_pipeline_defines(r: &Renderer, source: &str, native: &Native, entries: &[wgpu::BindGroupLayoutEntry], threads: (u32,u32,u32), extra: &[&str]) -> Result<wgpu::ComputePipeline> {
    let mut owned = programs::abi().native_modules();
    owned[0].1=programs::abi().module();
    owned.extend(native.modules.clone());
    let mut modules = crate::native::SOURCES.to_vec();
    modules.extend(owned.iter().map(|(n,s)| (n.as_str(),s.as_str())));
    // Match wgpu's dense per-resource-class Metal slots even when imported
    // declarations precede the output or an unused resource is eliminated.
    let mut source = source.to_owned();
    // Transport oracles supply analytic visibility; field interpolation has its
    // own GPU test with real stored records and multiple retained samples.
    if !source.contains("medium_cached_visibility") && !source.contains("import vork_medium_visibility_grid;") {
        let visibility=if source.contains("export float3 medium_visibility(") {"medium_visibility(p,i)"} else {"float3(1.)"};
        source.push_str(&format!("\nexport float3 medium_cached_visibility(float3 p,uint i) {{return {visibility};}}\nexport float3 medium_direct_visibility(float3 p,uint i) {{return {visibility};}}\n"));
    }
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by_key(|e| e.binding);
    let mut slots = [0u32; 3];
    for entry in sorted {
        let (class, counter) = match entry.ty {
            wgpu::BindingType::Buffer {ty, ..} => (match ty {
                wgpu::BufferBindingType::Uniform => 'b',
                wgpu::BufferBindingType::Storage {read_only:true} => 't',
                wgpu::BufferBindingType::Storage {read_only:false} => 'u',
            }, 0),
            wgpu::BindingType::AccelerationStructure {..} => ('t', 0),
            wgpu::BindingType::Texture {..} => ('t', 1),
            wgpu::BindingType::StorageTexture {..} => ('u', 1),
            wgpu::BindingType::Sampler(_) => ('s', 2),
            _ => anyhow::bail!("unsupported numerical test resource"),
        };
        let annotation = format!("[[vk::binding({},0)]]", entry.binding);
        // Page tables declare a Metal argument buffer and a Vulkan array.
        let mut from = 0;
        while let Some(start) = source[from..].find(&annotation).map(|at| from + at) {
            let end = start + source[start..].find(';').context("numerical resource declaration")?;
            let mut declaration = source[start..end].to_owned();
            if let Some(register) = declaration.find(" : register(") {declaration.truncate(register);}
            let class = if declaration.contains("ParameterBlock<") {'b'} else {class};
            declaration.push_str(&format!(" : register({class}{})", slots[counter]));
            source.replace_range(start..end, &declaration);
            from = start + declaration.len();
        }
        slots[counter] += 1;
    }
    modules.push(("medium_test", &source));
    let steps = format!("-DVORK_MEDIUM_STEPS={}",native.steps.max(1));
    // Passthrough modules require the native target's representation, not a
    // backend restriction on the numerical kernel being tested.
    let metal = r.adapter_info.backend == wgpu::Backend::Metal;
    let (_, target, flags) = if metal {crate::forward_variant::METAL_TARGET} else {crate::forward_variant::SPIRV_TARGET};
    let mut args = vec![steps.as_str(), "-DVORK_NATIVE_MEDIUM", "-target", target];
    args.extend_from_slice(crate::forward_variant::SLANG_FLAGS);
    args.extend_from_slice(flags);
    let defines=native.defines();
    args.extend(defines.iter().map(String::as_str));
    args.extend_from_slice(extra);
    // Numerical transport oracles exercise the certified path arithmetic contract.
    args.extend(vork_slang::MEDIUM_FP_FLAGS);
    let include: Vec<_> = native.includes.iter().map(|p| p.display().to_string()).collect();
    for path in &include {args.extend(["-I",path]);}
    let code = Compiler::discover().compile_modules(&modules, "medium_test", &[("numerical_test","compute")], &args)?;
    let msl = metal.then(|| std::str::from_utf8(&code)).transpose()?;
    let shader = crate::native::module(&r.device, "native medium numerical test", (!metal).then_some(code.as_slice()), msl, threads);
    let bind = r.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label: None, entries});
    let layout = r.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label: None, bind_group_layouts: &[Some(&bind)], immediate_size: 0});
    let pipeline = r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label: None, layout: Some(&layout), module: &shader, entry_point: Some("numerical_test"), compilation_options: Default::default(), cache: None});
    r.errors.check()?;
    Ok(pipeline)
}

const FROXEL_STORAGE: &str = "// FROXEL_STORAGE";

/// One receiver/transmittance vector per visibility sample, followed by the
/// pixel source sum; visibility, streamed transport and shading share the pages.
pub(crate) fn sample_buffers(s: &InstalledScene, d: &wgpu::Device, extent: [u32;2], samples:u32) -> Vec<wgpu::Buffer> {
    let Some(n)=&s.pipelines.resolve_native else {return Vec::new();};
    paged_buffers(s,d,"volume receiver samples",n.sample_bytes(extent,samples),n.sample_pages,n.froxel_page_elements)
}
/// The volume passes' page group: transport then receiver-sample page arrays.
pub(crate) fn page_layout(d:&wgpu::Device,n:&Native) -> wgpu::BindGroupLayout {
    let entries=[(crate::resolve_native::FROXEL_PAGES,n.froxel_pages),(crate::resolve_native::SAMPLE_PAGES,n.sample_pages)].map(|(binding,pages)| {
        let mut table=crate::pipelines::buffer_entry(binding,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false});
        table.count=std::num::NonZeroU32::new(pages.max(1));
        table
    });
    d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("volume pages"),entries:&entries})
}
pub(crate) fn paged_buffers(s:&InstalledScene,d:&wgpu::Device,label:&str,bytes:u64,pages:u32,elements:u32) -> Vec<wgpu::Buffer> {
    let mut remaining=bytes;let capacity=u64::from(elements)*FROXEL_ELEMENT_BYTES;
    (0..pages).map(|page| {
        let size=remaining.min(capacity).max(16);remaining=remaining.saturating_sub(size);
        s.transients.buffer(d,&format!("{label} page {page}"),size,wgpu::BufferUsages::STORAGE)
    }).collect()
}

/// Pages form one binding array (a Metal argument buffer, as camera pages do),
/// so their count never consumes per-stage buffer slots.
fn page_source(source: &str, hook: &str, name: &str, pages: u32, capacity: u32, depth: bool, binding: u32, register: &str) -> String {
    let mut code=String::new();let pages=pages.max(1);let capacity=capacity.max(1);
    let element=if depth {"uint4"} else {"float4"};
    let group=crate::resolve_native::PAGE_GROUP;
    code.push_str(&format!("#ifdef VORK_METAL\nstruct {name}_pages_t {{RWStructuredBuffer<{element}> buffers[];}};\n[[vk::binding({binding},{group})]] ParameterBlock<{name}_pages_t> {name} : register({register});\n#define {name}_page(page) {name}.buffers[page]\n#else\n[[vk::binding({binding},{group})]] RWStructuredBuffer<{element}> {name}[];\n#define {name}_page(page) {name}[page]\n#endif\n"));
    let mut operations=vec![("read","float4",""),("write","void",",float4 value")];
    if depth {operations.push(("depth","float",""));}
    for (operation,result,argument) in operations {
        code.push_str(&format!("export {result} {name}_{operation}(uint index{argument}) {{\n"));
        if pages>1 {code.push_str(&format!("uint page=index/{capacity}u;index=index%{capacity}u;switch(page) {{\n"));}
        for page in 0..pages {
            if pages>1 {code.push_str(&format!("case {page}u: "));}
            let at=format!("{name}_page({page}u)[index]");
            let body=match operation {
                "read"=>format!("return {};",if depth {format!("asfloat({at})")} else {at.clone()}),
                "write"=>format!("{at}={};return;",if depth {"asuint(value)"} else {"value"}),
                _=>format!("let packed={at}.w;return select(packed==0u,1.,asfloat(~packed));"),
            };
            code.push_str(&format!("{{{body}}}\n"));
        }
        if pages>1 {code.push_str("}\n");match result {"float4"=>code.push_str("return float4(0.);\n"),"float"=>code.push_str("return 1.;\n"),_=>{}}}
        code.push_str("}\n");
    }
    source.replace(hook,&code)
}

const FROXEL_VIEW: &str = r#"
struct VolumeSampleParameters {uint4 index;float4 disk;};
[[vk::binding(3,0)]] ConstantBuffer<VolumeSampleParameters> volume_sample_index;
export Frame medium_frame() {
    return froxel_sample_frame(frame,volume_sample_index.index,volume_sample_index.disk.xy);
}
export uint2 medium_size() {return dimensions(color);}
"#;

impl Installed {
    pub(crate) fn bind_inputs(&self, plan: &mut animation::Plan, words: &mut [u32], base: usize) -> Result<()> {
        for (name, offset, layout, time_offset) in &self.inputs {
            programs::bind_inputs(plan, name, layout, &mut words[*offset..*offset + layout.words], base + offset, *time_offset)?;
        }
        Ok(())
    }
}

// Receiver support does not link illumination, point programs or their exported
// transport adapters. Keep this reduction small on both native backends.
pub(crate) fn receiver_support_source(native:&Native)->String {
    native.paged_source(RECEIVER_SUPPORT_SOURCE)
}
const RECEIVER_SUPPORT_SOURCE:&str=r#"
import vork_froxel;
import vork_prelude;
[[vk::binding(0,0)]] Texture2D<float4> color;
[[vk::binding(5,0)]] ConstantBuffer<Frame> frame;
uint2 dimensions(Texture2D<float4> t) {uint w,h;t.GetDimensions(w,h);return uint2(w,h);}
// FROXEL_VIEW
// FROXEL_STORAGE
// SAMPLE_STORAGE
export float2 medium_receiver_support(uint2 pixel) {
    let size=medium_size();
    let index=volume_sample_index.index.x*size.x*size.y+pixel.y*size.x+pixel.x;
    let background=asuint(volume_sample_read(index).w)==0u;
    return background ? float2(0.,1.) : float2(clamp(volume_sample_depth(index),0.,1.),0.);
}
[shader("compute")][numthreads(8,8,1)] void volume_receiver_support(uint3 id:SV_DispatchThreadID) {froxel_receiver_support(id);}
"#;
