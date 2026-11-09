//! Native backend code paths (DESIGN §5: native calls live in one module).
//! Slang compiles every renderer entry point at build time to SPIR-V for
//! Vulkan and MSL for Metal; wgpu receives native backend code
//! through its passthrough path, so naga neither translates nor validates
//! them. `tests` checks Slang's reflection against the pipeline layout tables
//! instead, including the Metal resource indices wgpu-hal assigns.
include!(concat!(env!("OUT_DIR"), "/slang_shaders.rs"));

/// One entry point compiled by `build_slang.rs`, its own shader module.
pub(crate) struct Compiled {
    pub name: &'static str,
    pub entry: &'static str,
    spirv: &'static [u8],
    msl: &'static str,
    /// Compute threadgroup size (Metal reads it from the module descriptor); zero otherwise.
    pub(crate) threads: (u32, u32, u32),
    #[cfg(test)]
    stage: &'static str,
    #[cfg(test)]
    spirv_reflection: &'static str,
    #[cfg(test)]
    msl_reflection: &'static str,
}

/// The shader module for `shader` on this device's backend.
pub(crate) fn shader_module(device: &wgpu::Device, shader: &Compiled) -> wgpu::ShaderModule {
    module(device, shader.name, Some(shader.spirv), (!shader.msl.is_empty()).then_some(shader.msl), shader.threads)
}

/// Built-in surface capabilities are late constants, independent of the
/// module's resource interface. Change their defaults before driver compilation;
/// dependent OpSpecConstantOp expressions remain for the driver to fold.
pub(crate) fn surface_module(device: &wgpu::Device, shader: &Compiled, capabilities: &[(&str, crate::forward_variant::Capability)]) -> anyhow::Result<wgpu::ShaderModule> {
    let mut code=shader.spirv.to_vec();
    specialize_surface(&mut code,capabilities)?;
    Ok(module(device,shader.name,Some(&code),None,shader.threads))
}

fn specialize_surface(code: &mut [u8], capabilities: &[(&str, crate::forward_variant::Capability)]) -> anyhow::Result<()> {
    use crate::forward_variant::Capability;
    // This also validates the complete instruction stream. Keep Slang's shared
    // functions without constraining the device compiler's register allocation.
    crate::slang_forward::allow_driver_inlining(code)?;
    let mut words:Vec<u32>=code.chunks_exact(4).map(|w|u32::from_le_bytes(w.try_into().unwrap())).collect();
    let mut constants=std::collections::HashMap::new();
    let mut at=5;
    while at<words.len() {
        let count=(words[at]>>16) as usize;
        if words[at]&0xffff==71 && count>=3 && words[at+2]==1 { // OpDecorate SpecId
            anyhow::ensure!(count==4,"malformed surface specialization decoration");
            let id=words[at+3] as usize;
            anyhow::ensure!(id<capabilities.len(),"unknown surface capability SpecId {id}");
            constants.insert(words[at+1],capabilities[id].1);
        }
        at+=count;
    }
    at=5;
    while at<words.len() {
        let count=(words[at]>>16) as usize;
        let op=words[at]&0xffff;
        if matches!(op,48..=50) { // OpSpecConstantTrue, False, scalar
            anyhow::ensure!(count>=3,"malformed surface specialization constant");
            if let Some(value)=constants.get(&words[at+2]) {
                match (op,value) {
                    (48|49,Capability::Bool(value))=> {
                        anyhow::ensure!(count==3,"malformed boolean surface capability");
                        words[at]=(3<<16)|if *value {48} else {49};
                    }
                    (50,Capability::Uint(value))=> {
                        anyhow::ensure!(count==4,"surface capability must be a 32-bit scalar");
                        words[at+3]=*value;
                    }
                    _=>anyhow::bail!("surface capability type differs from its shipped declaration"),
                }
            }
        }
        at+=count;
    }
    for (out,word) in code.chunks_exact_mut(4).zip(words) {out.copy_from_slice(&word.to_le_bytes());}
    Ok(())
}

/// A passthrough module from Slang's SPIR-V and/or MSL; wgpu takes the one for
/// this device's backend.
pub(crate) fn module(
    device: &wgpu::Device,
    label: &str,
    spirv: Option<&[u8]>,
    msl: Option<&str>,
    threads: (u32, u32, u32),
) -> wgpu::ShaderModule {
    // Slang does not annotate vertex positions. Depth-equal prepass/forward
    // pairs need invariance even when their vertex entry points differ.
    #[cfg(target_vendor = "apple")]
    let msl = msl.map(invariant_vertex_positions);

    // SAFETY: the code is the pinned Slang compiler's output for this entry
    // point; the layout tests check its bindings against the pipeline layouts.
    let start=std::time::Instant::now();
    let module=unsafe {
        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
            label: Some(label),
            spirv: spirv.map(wgpu::util::make_spirv_raw),
            msl: msl.map(Into::into),
            num_workgroups: threads,
            ..Default::default()
        })
    };
    crate::timing::creation(device,label,"shader module",0.,start.elapsed().as_secs_f64()*1000.);
    module
}

/// Each Slang module contains one entry point. Never annotate fragment inputs.
#[cfg(any(target_vendor = "apple", test))]
fn invariant_vertex_positions(source: &str) -> std::borrow::Cow<'_, str> {
    if source.contains("[[vertex]]") {
        source.replace("[[position]];", "[[position, invariant]];").into()
    } else {
        source.into()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::Value;
    use wgpu::{BindGroupLayoutEntry, BindingType, ShaderStages};

    #[test]
    fn late_surface_artifacts_keep_dispatch_sizes_and_accept_capabilities() -> anyhow::Result<()> {
        use crate::{forward_variant::*,surface_native::SurfaceKernel};
        for &(family,_,_,_,_,shader) in SURFACES {
            let threads=match family {
                SurfaceKernel::Gi=>GI_RAY_COMPUTE.threads,
                SurfaceKernel::Reflections=>REFLECTIONS_HITS_COMPUTE.threads,
                SurfaceKernel::Probes=>PROBE_UPDATE_TRACE.threads,
                SurfaceKernel::Directional=>PROBE_UPDATE_DIRECTIONAL_TRACE.threads,
                SurfaceKernel::Evaluate=>CAMERAS.iter().find(|(_,_,stage,_)|*stage=="evaluate").unwrap().3.threads,
                SurfaceKernel::Indirect=>(vork_slang::registry::CAMERA_INDIRECT_GROUP,1,1),
                SurfaceKernel::Occupancy=>(8,8,1),
            };
            assert_eq!(shader.threads,threads,"{}",shader.name);
            let mut minimal=shader.spirv.to_vec();
            specialize_surface(&mut minimal,&variant_capabilities(FEATURE_SPECIALIZED,false,false,false))?;
            let mut complete=shader.spirv.to_vec();
            specialize_surface(&mut complete,&variant_capabilities(u128::MAX,true,true,true))?;
            assert_ne!(minimal,complete,"{} has no late capabilities",shader.name);
        }
        Ok(())
    }

    #[test]
    fn late_surface_defaults_preserve_dependent_expressions() -> anyhow::Result<()> {
        use crate::forward_variant::Capability;
        let words=[0x07230203u32,0x00010500,0,32,0,
            (4<<16)|71,2,1,0, (4<<16)|71,3,1,1,
            (3<<16)|49,1,2, (4<<16)|50,4,3,0,
            (6<<16)|52,4,5,128,3,3,
            (5<<16)|54,1,6,2,7,(1<<16)|56];
        let mut code:Vec<u8>=words.into_iter().flat_map(u32::to_le_bytes).collect();
        specialize_surface(&mut code,&[("bool",Capability::Bool(true)),("uint",Capability::Uint(0x87654321))])?;
        let mut expected=words;
        expected[13]=(3<<16)|48;expected[19]=0x87654321;expected[29]=0;
        assert_eq!(code,expected.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>());
        assert!(specialize_surface(&mut code,&[("wrong",Capability::Uint(1))]).is_err());
        Ok(())
    }

    #[test]
    fn metal_callbacks_link_without_unreachable_mutable_library_exports() -> anyhow::Result<()> {
        let modules = [
            ("callbacks", "internal extern uint callback(out uint value); public uint read_callback() {uint v;return callback(v)+v;}"),
            ("main", r#"import callbacks; export uint callback(out uint value) {value=7u;return 3u;} [[vk::binding(0,0)]] RWStructuredBuffer<uint> output; [shader("compute")][numthreads(1,1,1)] void entry() {output[0]=1u;}"#),
        ];
        let (_, target, flags) = crate::forward_variant::METAL_TARGET;
        let args: Vec<_> = ["-target",target].into_iter().chain(flags.iter().copied()).collect();
        let code = vork_slang::Compiler::discover().compile_modules(&modules,"main",&[("entry","compute")],&args)?;
        assert!(!String::from_utf8(code)?.contains("uint callback("));
        Ok(())
    }

    #[test]
    fn emitted_vertex_locations_match_stream_bindings() {
        fn inputs(v: &Value, out: &mut std::collections::BTreeMap<String,u64>) {
            if v["binding"]["kind"]=="varyingInput" {
                if let (Some(name),Some(index))=(v["name"].as_str(),v["binding"]["index"].as_u64()) {
                    out.insert(name.to_owned(),index);
                }
            }
            match v {
                Value::Object(o)=>for child in o.values() {inputs(child,out);},
                Value::Array(a)=>for child in a {inputs(child,out);},
                _=>(),
            }
        }
        for &(_,_,_,shader) in STREAMS.iter().filter(|s|s.3.stage=="vertex") {
            let mut locations=std::collections::BTreeMap::new();
            inputs(&serde_json::from_str::<Value>(shader.spirv_reflection).unwrap()["entryPoints"][0],&mut locations);
            if !locations.contains_key("next_p") {continue;}
            for (name,index) in [("POSITION",0),("NORMAL",1),("TEXCOORD_0",2),
                ("previous_p",3),("COLOR_0",4),("TANGENT",5),("TEXCOORD_1",6),("rest",15),("next_p",16)] {
                assert_eq!(locations.get(name),Some(&index),"{}: {name}",shader.name);
            }
        }
    }

    /// The shipped depth and control stages declare SV_PrimitiveID exactly when
    /// they evaluate hashed coverage (forward_variant::FEATURE_DITHERED).
    #[test]
    fn only_hashed_coverage_stages_declare_the_primitive() {
        fn declares(shader: &Compiled) -> bool {
            let found = spirv_decorations(shader.spirv).iter().any(|&(_, d, v)| d == 11 && v == 7); // BuiltIn PrimitiveId
            if !shader.msl.is_empty() {
                assert_eq!(shader.msl.contains("[[primitive_id]]"), found, "{}: SPIR-V and MSL disagree", shader.name);
            }
            found
        }
        let mut depth = vec![(false, &DEPTH_DEPTH_FS), (true, &DEPTH_HASHED_DEPTH_FS)];
        depth.extend(STREAMS.iter().filter(|s| s.1 == "depth" && s.2.ends_with("depth_fs")).map(|s| (s.2 == "hashed_depth_fs", s.3)));
        assert_eq!(depth.len(), 2 * (crate::constants::PRECOMPILED_ATTRIBUTE_SLOTS + 1));
        for (hashed, shader) in depth {
            assert_eq!(declares(shader), hashed, "{}", shader.name);
        }
        assert_eq!(CONTROL_GENERICS.iter().filter(|g| g.3).count(), CONTROL_GENERICS.len() / 2);
        for &(_, _, _, hashed, shader) in CONTROL_GENERICS {
            assert_eq!(declares(shader), hashed, "{}", shader.name);
        }
    }

    #[test]
    fn shipped_attribute_stages_preserve_their_varying_interface() {
        fn attributes(reflection: &str, kind: &str) -> std::collections::BTreeMap<String,u64> {
            fn walk(v:&Value, kind:&str, out:&mut std::collections::BTreeMap<String,u64>) {
                if let Some(name) = v.get("name").and_then(Value::as_str) {
                    if name.starts_with("attr") && v["binding"]["kind"]==kind {
                        out.insert(name.into(),v["binding"]["index"].as_u64().unwrap());
                    }
                }
                match v {
                    Value::Object(o)=>for child in o.values() {walk(child,kind,out);},
                    Value::Array(a)=>for child in a {walk(child,kind,out);},
                    _=>(),
                }
            }
            let mut out=std::collections::BTreeMap::new();
            let json:Value=serde_json::from_str(reflection).unwrap();
            walk(&json["entryPoints"][0],kind,&mut out);out
        }
        assert_eq!(CONTROL_GENERICS.iter().map(|p| p.0 as usize).max(), Some(crate::constants::PRECOMPILED_ATTRIBUTE_SLOTS));
        assert_eq!(STREAMS.iter().map(|s|s.0 as usize).max(), Some(crate::constants::PRECOMPILED_ATTRIBUTE_SLOTS));
        assert_eq!(VOLUME_VISIBILITY.iter().map(|s|s.1 as usize).max(), Some(crate::constants::PRECOMPILED_ATTRIBUTE_SLOTS));
        for &(count,module,entry,vertex) in STREAMS.iter().filter(|s|s.3.stage=="vertex") {
            // Fibre captures compile their material fragment at install time.
            if entry=="shadow_vs" {
                for reflection in [vertex.spirv_reflection,vertex.msl_reflection].into_iter().filter(|r|!r.is_empty()) {
                    assert_eq!(attributes(reflection,"varyingOutput").len(),count as usize,"{}",vertex.name);
                }
                continue;
            }
            let fragment_entry=match entry {"main_vs"=>"depth_fs",_=>"masked_fs"};
            let fragment=STREAMS.iter().find(|&&(a,m,e,_)|a==count && m==module && e==fragment_entry).unwrap().3;
            for (v,f) in [(vertex.spirv_reflection,fragment.spirv_reflection),(vertex.msl_reflection,fragment.msl_reflection)].into_iter().filter(|(v,f)|!v.is_empty() && !f.is_empty()) {
                let output=attributes(v,"varyingOutput");let input=attributes(f,"varyingInput");
                assert_eq!(output.len(),count as usize,"{}",vertex.name);
                assert_eq!(output,input,"{} -> {}",vertex.name,fragment.name);
            }
        }
        for &(count,_,_,_,fragment) in CONTROL_GENERICS {
            let vertex = if count==0 {&DEPTH_MAIN_VS} else {
                STREAMS.iter().find(|&&(a,m,e,_)|a==count && m=="depth" && e=="main_vs").unwrap().3
            };
            for (v,f) in [(vertex.spirv_reflection,fragment.spirv_reflection),(vertex.msl_reflection,fragment.msl_reflection)].into_iter().filter(|(v,f)|!v.is_empty() && !f.is_empty()) {
                assert_eq!(attributes(v,"varyingOutput"),attributes(f,"varyingInput"),
                    "{} -> {}",vertex.name,fragment.name);
            }
        }
    }

    #[test]
    fn metal_invariance_only_changes_vertex_position_outputs() {
        let destination = std::env::var_os("VORK_CHECK_METAL_DIR").map(std::path::PathBuf::from);
        for shader in ALL.iter().filter(|shader| !shader.msl.is_empty()) {
            let invariant = invariant_vertex_positions(shader.msl);
            if let Some(destination) = &destination {
                std::fs::write(destination.join(format!("{}.metal", shader.name)), invariant.as_bytes()).unwrap();
            }
            if shader.stage == "vertex" {
                assert!(shader.msl.contains("[[position]];"), "{}", shader.name);
                assert!(!invariant.contains("[[position]];"), "{}", shader.name);
                assert!(invariant.contains("[[position, invariant]];"), "{}", shader.name);
                assert_eq!(
                    invariant.replace("[[position, invariant]];", "[[position]];"),
                    shader.msl,
                    "{}: only vertex output annotations may change", shader.name,
                );
            } else {
                assert_eq!(invariant, shader.msl, "{}: fragment/compute unchanged", shader.name);
            }
        }
    }

    /// Every fragment SV_Depth must reach MSL as a depth attribute.
    #[test]
    fn metal_fragment_depth_outputs_write_depth() {
        let mut depth_writers = 0;
        for shader in ALL.iter().filter(|shader| shader.stage == "fragment" && !shader.msl.is_empty()) {
            if shader.msl_reflection.contains("\"SV_DEPTH\"") {
                depth_writers += 1;
                assert!(shader.msl.contains("[[depth(any)]]"), "{}: SV_Depth is not a Metal depth output", shader.name);
            }
        }
        assert!(depth_writers > 0, "no fragment shader writes SV_Depth; the check is vacuous");
    }

    fn stage(name: &str, stage: &str) -> ShaderStages {
        match stage {
            "vertex" => ShaderStages::VERTEX,
            "fragment" => ShaderStages::FRAGMENT,
            "compute" => ShaderStages::COMPUTE,
            other => panic!("{name}: stage {other}"),
        }
    }

    /// Parameters entry point `entry` uses, from Slang's reflection JSON.
    fn used(json: &str, entry: &str) -> Vec<(String, Value)> {
        let reflection: Value = serde_json::from_str(json).unwrap();
        let globals = reflection["parameters"].as_array().unwrap();
        let point = reflection["entryPoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == entry)
            .unwrap_or_else(|| panic!("no entry point {entry}"));
        let bindings = point["bindings"].as_array().map_or(&[][..], |b| &b[..]);
        assert!(
            bindings.iter().all(|b| b.to_string().contains("\"used\"")),
            "{entry}: reflection without usage (compile the entry alone)"
        );
        bindings
            .iter()
            .filter(|b| {
                b.get("binding")
                    .into_iter()
                    .chain(b["bindings"].as_array().into_iter().flatten())
                    .any(|b| b["used"] == 1 && b["count"].as_u64() != Some(0))
            })
            .map(|b| {
                let name = b["name"].as_str().unwrap().to_owned();
                let global = globals.iter().find(|g| g["name"] == name.as_str()).unwrap();
                (name, global.clone())
            })
            .collect()
    }

    /// (path, offset, size) of every field in a buffer's element type.
    fn fields(ty: &Value, path: &str, out: &mut Vec<(String, u64, u64)>) {
        let ty = match ty["kind"].as_str() {
            Some("constantBuffer" | "parameterBlock") => &ty["elementType"],
            Some("resource") => &ty["resultType"],
            _ => ty,
        };
        let ty = if ty["kind"] == "array" {
            &ty["elementType"]
        } else {
            ty
        };
        for f in ty["fields"].as_array().into_iter().flatten() {
            let name = format!("{path}.{}", f["name"].as_str().unwrap());
            let b = &f["binding"];
            if let (Some(offset), Some(size)) = (b["offset"].as_u64(), b["size"].as_u64()) {
                out.push((name.clone(), offset, size));
            }
            fields(&f["type"], &name, out);
        }
    }

    #[derive(Debug, PartialEq)]
    enum Kind {
        Uniform,
        Storage,
        Texture,
        StorageTexture,
        Sampler,
        AccelerationStructure,
    }

    fn layout_kind(ty: &BindingType) -> Kind {
        match ty {
            BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                ..
            } => Kind::Uniform,
            BindingType::Buffer { .. } => Kind::Storage,
            BindingType::Texture { .. } => Kind::Texture,
            BindingType::StorageTexture { .. } => Kind::StorageTexture,
            BindingType::Sampler(_) => Kind::Sampler,
            BindingType::AccelerationStructure { .. } => Kind::AccelerationStructure,
            other => panic!("unchecked binding type {other:?}"),
        }
    }

    /// The Slang type's binding kind and array count (None: not an array).
    fn slang_kind(ty: &Value) -> (Kind, Option<u64>) {
        if ty["kind"] == "array" {
            let (kind, _) = slang_kind(&ty["elementType"]);
            return (kind, ty["elementCount"].as_u64());
        }
        if ty["kind"] == "parameterBlock" {
            // A Metal argument buffer wraps exactly one array.
            let fields = ty["elementType"]["fields"].as_array().unwrap();
            assert_eq!(fields.len(), 1, "argument buffer {ty}");
            return slang_kind(&fields[0]["type"]);
        }
        let kind = match ty["kind"].as_str().unwrap() {
            "constantBuffer" => Kind::Uniform,
            "samplerState" => Kind::Sampler,
            "resource" => match ty["baseShape"].as_str().unwrap() {
                "structuredBuffer" | "byteAddressBuffer" => Kind::Storage,
                "accelerationStructure" => Kind::AccelerationStructure,
                _ if ty["access"] == "readWrite" || ty["access"] == "write" => Kind::StorageTexture,
                _ => Kind::Texture,
            },
            other => panic!("unchecked Slang parameter kind {other}: {ty}"),
        };
        (kind, None)
    }

    /// wgpu-hal Metal's per-stage resource index for each (group, binding):
    /// buffers, textures and samplers each count up across groups in order,
    /// entries in binding order, and a binding array takes one buffer slot
    /// (its argument buffer). Slot consumption is owned by the binding type,
    /// shared with the HAL allocator; native pipeline reflection verifies it independently.
    fn metal_slots(
        groups: &[&[BindGroupLayoutEntry]],
        stage: ShaderStages,
    ) -> Vec<(u32, u32, &'static str, u32)> {
        let (mut buffers, mut textures, mut samplers) = (0, 0, 0);
        let mut slots = Vec::new();
        for (group, entries) in groups.iter().enumerate() {
            let mut entries = entries.to_vec();
            entries.sort_by_key(|e| e.binding);
            for e in entries.iter().filter(|e| e.visibility.contains(stage)) {
                let counter = match e.ty.metal_resource_slots(e.count) {
                    [1, 0, 0] => (&mut buffers, "buffer"),
                    [0, 1, 0] => (&mut textures, "texture"),
                    [0, 0, 1] => (&mut samplers, "sampler"),
                    other => panic!("unsupported passthrough resource slots {other:?}"),
                };
                slots.push((group as u32, e.binding, counter.1, *counter.0));
                *counter.0 += 1;
            }
        }
        assert!(buffers <= 31, "{buffers} Metal buffer slots");
        assert!(textures <= 128, "{textures} Metal texture slots");
        assert!(samplers <= 16, "{samplers} Metal sampler slots");
        slots
    }

    #[test]
    fn reflection_medium_pages_preserve_native_binding_slots() {
        let globals=crate::rt::reflections_hits_entries();
        // Vulkan passthrough bindings must match wgpu's dense native numbering.
        let mut bindings:Vec<_>=globals.iter().map(|entry|entry.binding).collect();
        bindings.sort_unstable();
        assert_eq!(bindings,(0..globals.len() as u32).collect::<Vec<_>>());

        let materials=crate::pipelines::material_entries(true);
        for probes in [false,true] {
            let read=if probes {crate::probes::read_entries(0,ShaderStages::COMPUTE).to_vec()} else {vec![]};
            let registers=crate::forward_variant::hit_kernel_registers("reflections_hits",false,probes);
            for pages in [0,1,2,9] {
                let hits=crate::rt::reflection_read_entries(pages);
                let slots=metal_slots(&[&globals,&materials,&read,&hits],ShaderStages::COMPUTE);
                let hit=slots.iter().find(|s|s.0==3 && s.1==0).unwrap().3;
                assert!(registers.contains(&format!("-DVORK_HIT_REGISTER=b{hit}")));
                assert_eq!(hits.len(),if pages==0 {2} else {3});
                let fields=slots.iter().find(|s|s.0==3 && s.1==1).unwrap().3;
                assert!(registers.contains(&format!("-DVORK_REFLECTION_FIELDS_REGISTER=b{fields}")));
                if pages>0 {
                    let grid=slots.iter().find(|s|s.0==3 && s.1==2).unwrap().3;
                    assert!(registers.contains(&format!("-DVORK_REFLECTION_MEDIUM_REGISTER=b{grid}")));
                    assert_eq!(hits[2].count.unwrap().get(),pages);
                }
            }
        }
    }

    #[test]
    fn gi_accumulation_preserves_native_binding_slots() {
        let globals=crate::rt::gi_entries();let materials=crate::pipelines::material_entries(true);
        for probes in [false,true] {
            let read=if probes {crate::probes::read_entries(0,ShaderStages::COMPUTE).to_vec()} else {vec![]};
            let hits=crate::rt::gi_read_entries();
            let slots=metal_slots(&[&globals,&materials,&read,&hits],ShaderStages::COMPUTE);
            let registers=crate::forward_variant::hit_kernel_registers("gi_ray",false,probes);
            assert_eq!(hits.len(),2);
            let slot=slots.iter().find(|s|s.0==3 && s.1==1).unwrap().3;
            assert!(registers.contains(&format!("-DVORK_GI_QUERY_REGISTER=u{slot}")));
        }
    }

    // Exported resource accessors can leave false-positive "used" flags in
    // Slang reflection after linking. The emitted SPIR-V is authoritative.
    /// Single-operand OpDecorate instructions: (target, decoration, operand).
    fn spirv_decorations(bytes: &[u8]) -> Vec<(u32, u32, u32)> {
        let words: Vec<u32> = bytes.chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut decorations = Vec::new();
        let mut at = 5;
        while at < words.len() {
            let count = (words[at] >> 16) as usize;
            assert!(count > 0 && at + count <= words.len());
            if words[at] & 0xffff == 71 && count == 4 { // OpDecorate
                decorations.push((words[at + 1], words[at + 2], words[at + 3]));
            }
            at += count;
        }
        decorations
    }
    fn spirv_descriptor_slots(bytes: &[u8]) -> Vec<(u32, u32)> {
        let mut sets = std::collections::HashMap::new();
        let mut bindings = std::collections::HashMap::new();
        for (id, decoration, operand) in spirv_decorations(bytes) {
            match decoration {
                33 => { bindings.insert(id, operand); } // Binding
                34 => { sets.insert(id, operand); } // DescriptorSet
                _ => {}
            }
        }
        bindings.into_iter().map(|(id, binding)| (sets[&id], binding)).collect()
    }

    /// Check `shader` against the pipeline layout it runs with.
    pub(crate) fn check(shader: &Compiled, groups: &[&[BindGroupLayoutEntry]]) {
        {
            let (entry, spirv_reflection, msl_reflection) =
                (shader.entry, shader.spirv_reflection, shader.msl_reflection);
            let name = shader.name;
            let stage = stage(name, shader.stage);
            // wgpu-hal Vulkan renumbers each group's entries 0..n in binding order
            // and remaps only naga shaders; passthrough SPIR-V keeps our numbers.
            for (group, entries) in groups.iter().enumerate() {
                let mut bindings: Vec<u32> = entries.iter().map(|e| e.binding).collect();
                bindings.sort_unstable();
                assert!(
                    bindings.iter().copied().eq(0..bindings.len() as u32),
                    "{name}: group {group} bindings {bindings:?} are not 0..n"
                );
            }
            let declared = spirv_descriptor_slots(shader.spirv);
            let spirv: Vec<_> = used(spirv_reflection, entry).into_iter().filter(|(_, r)| {
                r["binding"]["kind"] == "descriptorTableSlot" && declared.contains(&(r["binding"]["space"].as_u64().unwrap_or(0) as u32,
                    r["binding"]["index"].as_u64().unwrap() as u32))
            }).collect();
            for &(group, binding) in &declared {
                assert!(spirv.iter().any(|(_, r)|
                    r["binding"]["space"].as_u64().unwrap_or(0) == u64::from(group)
                    && r["binding"]["index"].as_u64() == Some(u64::from(binding))),
                    "{name}: emitted descriptor ({group}, {binding}) missing from reflection");
            }
            for (param, reflection) in &spirv {
                let binding = &reflection["binding"];
                let group = binding["space"].as_u64().unwrap_or(0) as usize;
                let index = binding["index"].as_u64().unwrap() as u32;
                assert_eq!(binding["kind"], "descriptorTableSlot", "{name}: {param}");
                let entry = groups
                    .get(group)
                    .and_then(|g| g.iter().find(|e| e.binding == index))
                    .unwrap_or_else(|| {
                        panic!("{name}: {param} at ({group}, {index}) is not in the layout")
                    });
                assert!(
                    entry.visibility.contains(stage),
                    "{name}: {param} is not visible to {stage:?}"
                );
                let (kind, count) = slang_kind(&reflection["type"]);
                assert_eq!(kind, layout_kind(&entry.ty), "{name}: {param} kind");
                if reflection["type"]["kind"]=="array" && count.is_none_or(|n|n==0 || n==u64::MAX) {
                    assert!(entry.count.is_some(),"{name}: {param} runtime descriptor array");
                } else {
                    assert_eq!(count,entry.count.map(|c|c.get() as u64),"{name}: {param} array count");
                }
            }
            if msl_reflection.is_empty() { return; }
            let slots = metal_slots(groups, stage);
            let declarations: Value = serde_json::from_str(spirv_reflection).unwrap();
            for (param, reflection) in used(msl_reflection, entry) {
                // Metal can retain a declared buffer that SPIR-V eliminates.
                let vulkan = spirv.iter().find(|(p, _)| *p == param).map(|(_, r)| r)
                    .or_else(|| declarations["parameters"].as_array().unwrap().iter()
                        .find(|p| p["name"] == param))
                    .unwrap_or_else(|| panic!("{name}: missing declaration for {param}"));
                let group = vulkan["binding"]["space"].as_u64().unwrap_or(0) as u32;
                let binding = vulkan["binding"]["index"].as_u64().unwrap() as u32;
                let &(_, _, kind, slot) = slots
                    .iter()
                    .find(|s| (s.0, s.1) == (group, binding))
                    .unwrap_or_else(|| panic!("{name}: {param} at ({group}, {binding}) is not in the layout"));
                let metal = reflection
                    .get("binding")
                    .into_iter()
                    .chain(reflection["bindings"].as_array().into_iter().flatten())
                    .next()
                    .unwrap();
                let metal_kind = match metal["kind"].as_str().unwrap() {
                    "constantBuffer" | "metalArgumentBufferElement" => "buffer",
                    "shaderResource" | "unorderedAccess" => "texture",
                    "samplerState" => "sampler",
                    other => panic!("{name}: {param} Metal binding kind {other}"),
                };
                assert_eq!(
                    (metal_kind, metal["index"].as_u64().unwrap() as u32),
                    (kind, slot),
                    "{name}: {param} Metal index must match wgpu-hal's {stage:?} counter"
                );
                // The buffer data is laid out once (the WGSL/Vulkan rules); Metal must agree.
                let (mut vk, mut mtl) = (Vec::new(), Vec::new());
                fields(&vulkan["type"], &param, &mut vk);
                fields(&reflection["type"], &param, &mut mtl);
                assert_ne!(reflection["type"]["kind"], "array",
                    "{name}: {param} Metal resource arrays require an argument buffer");
                if reflection["type"]["kind"] == "parameterBlock" {
                    // wgpu-hal fills a binding array's argument buffer with 8-byte
                    // MTLResourceIDs, one per element, from offset 0.
                    let (_, count) = slang_kind(&vulkan["type"]);
                    if count == Some(0) {
                        let fields=reflection["type"]["elementType"]["fields"].as_array().unwrap();
                        assert_eq!(fields.len(),1,"{name}: {param} runtime argument-buffer tail");
                        assert_eq!(fields[0]["binding"]["index"],0,"{name}: {param} resource IDs begin at zero");
                        assert_eq!(fields[0]["type"]["uniformStride"],8,"{name}: {param} resource ID stride");
                    } else {
                        assert!(matches!(&mtl[..], [(_,0,size)] if Some(*size)==count.map(|c|c*8)),
                            "{name}: {param} argument buffer {mtl:?} is not {count:?} resource IDs");
                    }
                } else {
                    assert_eq!(
                        mtl, vk,
                        "{name}: {param} Metal struct layout differs from Vulkan"
                    );
                }
            }
        }
    }

    /// The Slang ABI structs read the Rust `#[repr(C)]` uploads field by field.
    #[test]
    fn slang_abi_structs_match_the_uploads() {
        use std::mem::{offset_of, size_of};
        macro_rules! fields {
            ($t:ty: $($f:ident),*) => {
                (size_of::<$t>() as u64, vec![$((stringify!($f), offset_of!($t, $f) as u64)),*])
            };
        }
        // Slang's Frame folds WGSL `background: vec3, light_count: f32` into one float4.
        let frame = fields!(crate::Uniform: vp, light_vp, eye, background, radiance, params, flags,
            previous_vp, next_vp, inverse_vp, view, ao, projection, lens, aperture, temporal, environment,
            environment_rotation, environment_color, environment_mean, scene, shadow_links,
            integrator, sampling, environment_links, medium, display, post, composite_projection, composite_optics, composite_sensor, composite_shutter, composite_step, camera_sample, camera_program, field_geometry);
        let object = fields!(crate::GpuObject: model, previous_model, next_model, opacity, material_index, reserved, metadata, padding);
        let light = fields!(crate::lights::Light: shadow_offsets, position,
            direction, radiance, params, emitter_u, emitter_v, emission, projection);
        let reflection: Value = serde_json::from_str(PATH_PATH_FIXED.spirv_reflection).unwrap();
        macro_rules! camera_fields {($($field:ident:$rust:ty=>$slang:ident),*) => {fields!(crate::passes::decoupled::CameraConfig: $($field),*)};}
        let camera=crate::camera_native::camera_config_fields!(camera_fields);
        let camera_reflection:Value=serde_json::from_str(CAMERAS[0].3.spirv_reflection).unwrap();
        for (param, (size, rust), reflection) in [("frame", frame, &reflection), ("objects", object, &reflection), ("lights", light, &reflection), ("camera_config", camera, &camera_reflection)] {
            let p = reflection["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == param)
                .unwrap();
            let ty = &p["type"];
            let element = match ty["kind"].as_str() {
                Some("constantBuffer") => &ty["elementType"],
                _ => &ty["resultType"],
            };
            let slang: Vec<_> = element["fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    (
                        f["name"].as_str().unwrap(),
                        f["binding"]["offset"].as_u64().unwrap(),
                    )
                })
                .collect();
            let last = element["fields"].as_array().unwrap().last().unwrap();
            let end = last["binding"]["offset"].as_u64().unwrap()
                + last["binding"]["size"].as_u64().unwrap();
            assert_eq!(
                slang, rust,
                "{param}: Slang fields differ from the Rust upload"
            );
            assert_eq!(
                end, size,
                "{param}: Slang size differs from the Rust upload"
            );
        }
    }

    #[test]
    fn every_shader_matches_its_layout() {
        use crate::pipelines::{global_entries, material_entries};
        for samples in [1, crate::constants::RASTER_MSAA_SAMPLES] {
            for denoise in [false, true] {
                let entries = crate::pipelines::control_resolve_entries(samples, denoise);
                let shader = crate::pipelines::control_resolve_shader(samples, denoise);
                check(shader, &[&entries]);
            }
        }
        let denoise = crate::denoise::gpu::entries();
        let denoise_groups: &[&[BindGroupLayoutEntry]] = &[&denoise];
        let reconstruction = crate::denoise::spatial::entries();
        let reconstruction_groups: &[&[BindGroupLayoutEntry]] = &[&reconstruction];
        let mip: &[&[BindGroupLayoutEntry]] = &[&crate::materials::MIP_LAYOUT];
        let global = global_entries();
        // Metal indices depend on visibility, so check both material table layouts.
        let tables = [material_entries(false), material_entries(true)];
        let depth: Vec<Vec<&[BindGroupLayoutEntry]>> =
            tables.iter().map(|m| vec![&global[..], &m[..]]).collect();
        let images: &[&[BindGroupLayoutEntry]] = &[&crate::render_images::PUBLICATION_LAYOUT];
        let cluster = crate::pipelines::cluster_entries();
        let cluster_groups: &[&[BindGroupLayoutEntry]] = &[&cluster];
        let deform: &[&[BindGroupLayoutEntry]] = &[&crate::passes::deform::LAYOUT];
        let path = crate::passes::path::layout_entries(true);
        let hits = [crate::rt::hit_buffers_entry(0, ShaderStages::COMPUTE)];
        let path_groups: &[&[BindGroupLayoutEntry]] = &[&path, &tables[1], &[], &hits];
        let mut checked = vec![(&CLUSTERS_CULL, cluster_groups), (&MIP_VS, mip), (&MIP_FS, mip), (&DEFORM_EVALUATE, deform)];
        for shader in [&DENOISE_PACK, &DENOISE_UNPACK] {checked.push((shader,denoise_groups));}
        let indirect_args = crate::passes::decoupled::indirect_arguments_entries();
        let indirect_args_groups: &[&[BindGroupLayoutEntry]] = &[&indirect_args];
        checked.push((&CAMERA_INDIRECT_ARGS_PREPARE, indirect_args_groups));
        let emitter_weights = crate::emitter_weights::entries();
        let emitter_groups: &[&[BindGroupLayoutEntry]] = &[&emitter_weights, &[], &[], &hits];
        for shader in [&EMITTER_WEIGHTS_AREAS, &EMITTER_WEIGHTS_REDUCE] {checked.push((shader, emitter_groups));}
        checked.push((&PATH_PATH_MAIN, path_groups));
        let fixed_path = crate::passes::path::layout_entries(false);
        let fixed_path_groups: &[&[BindGroupLayoutEntry]] = &[&fixed_path, &tables[1], &[], &hits];
        checked.push((&PATH_PATH_FIXED, fixed_path_groups));
        let camera_cull = crate::passes::camera_cull::entries();
        for shader in [&RECONSTRUCT_PREPARE, &RECONSTRUCT_FILTER, &RECONSTRUCT_FINISH] {checked.push((shader,reconstruction_groups));}
        let camera_cull_groups: &[&[BindGroupLayoutEntry]] = &[&camera_cull];
        for shader in [&CAMERA_CULL_CLASSIFY,&CAMERA_CULL_OFFSETS,&CAMERA_CULL_COMPACT] {checked.push((shader,camera_cull_groups));}
        checked.push((&RENDER_IMAGES_PUBLISH, images));
        let fragments = crate::passes::decoupled::fragments_entries(0);
        let fragments_geometry = crate::passes::decoupled::fragments_geometry_entries();
        let fragments_groups: Vec<Vec<&[BindGroupLayoutEntry]>> = tables.iter().map(|m| vec![&global[..], &m[..], &fragments[..], &fragments_geometry[..]]).collect();
        let decoupled_merge = crate::passes::decoupled::merge_entries();
        let decoupled_merge_groups: &[&[BindGroupLayoutEntry]] = &[&decoupled_merge];
        checked.push((&DECOUPLED_MERGE_MERGE, decoupled_merge_groups));

        let environment: &[&[BindGroupLayoutEntry]] = &[&crate::environment::REDUCE_LAYOUT];
        checked.push((&ENVIRONMENT_REDUCE_REDUCE_ENVIRONMENT, environment));
        let display_texture: &[&[BindGroupLayoutEntry]] = &[&crate::display_stats::TEXTURE_LAYOUT];
        let display_packed: &[&[BindGroupLayoutEntry]] = &[&crate::display_stats::PACKED_LAYOUT];
        checked.push((&DISPLAY_STATS_REDUCE, display_texture));
        checked.push((&DISPLAY_STATS_PACKED_REDUCE, display_packed));
        let light_temperature: &[&[BindGroupLayoutEntry]] = &[&crate::light_temperature::LAYOUT];
        checked.push((&LIGHT_TEMPERATURE_TABULATE, light_temperature));
        for i in 0..2 {
            for shader in [
                &DEPTH_MAIN_VS,
                &DEPTH_OPAQUE_DEPTH_VS,
                &DEPTH_OPAQUE_SINGLE_DEPTH_VS,
                &DEPTH_OPAQUE_SINGLE_DEPTH_FS,
                &DEPTH_DEPTH_FS,
                &DEPTH_HASHED_DEPTH_FS,
                &DEPTH_SHADOW_VS,
            ] {
                checked.push((shader, &depth[i][..]));
            }
            for shader in [&DECOUPLED_VISIBILITY_VS,&DECOUPLED_VISIBILITY_FS,&DECOUPLED_VISIBILITY_SINGLE_FS,&DECOUPLED_VISIBILITY_MASKED_VS,&DECOUPLED_VISIBILITY_MASKED_FS] {
                checked.push((shader,&fragments_groups[i][..]));
            }
            checked.push((&DECOUPLED_VISIBILITY_TRANSPARENT_FRAGMENTS_FS, &fragments_groups[i][..]));
            checked.push((&DECOUPLED_VISIBILITY_TRANSPARENT_TRIANGLES_FS, &fragments_groups[i][..]));
        }
        for &(_, module, _, shader) in STREAMS {
            let layouts = if module == "decoupled_visibility" { &fragments_groups } else { &depth };
            for groups in layouts { checked.push((shader, &groups[..])); }
        }
        // Controls share the depth layout; emitted descriptors stay in groups 0/1.
        for shader in CONTROL_GENERICS.iter().map(|s| s.4) {
            for groups in &depth { checked.push((shader, &groups[..])); }
        }
        let blur = crate::passes::ao::slang_entries();
        let blur_groups: &[&[BindGroupLayoutEntry]] = &[&blur];
        checked.extend([
            (&AO_BLUR_HORIZONTAL, blur_groups),
            (&AO_BLUR_VERTICAL, blur_groups),
        ]);
        let ray_material = material_entries(true);
        let gi = crate::rt::gi_entries();
        let gi_hits = crate::rt::gi_read_entries();
        let gi_groups: &[&[BindGroupLayoutEntry]] = &[&gi, &ray_material, &[], &gi_hits];
        checked.push((&GI_RAY_COMPUTE, gi_groups));
        let probe_update = crate::probes::update_entries();
        let probe_read = crate::probes::read_entries(0, ShaderStages::COMPUTE);
        let probe_groups: &[&[BindGroupLayoutEntry]] = &[&probe_update, &ray_material, &probe_read, &hits];
        checked.extend([(&PROBE_UPDATE_TRACE, probe_groups), (&PROBE_UPDATE_GATHER, probe_groups), (&PROBE_UPDATE_CHOOSE,probe_groups), (&PROBE_UPDATE_DIRECTIONAL_TRACE,probe_groups), (&PROBE_UPDATE_DIRECTIONAL_GATHER,probe_groups), (&PROBE_UPDATE_FILTER,probe_groups)]);
        let reflection_hits = crate::rt::reflections_hits_entries();
        let reflection_fields=crate::rt::reflection_read_entries(0);
        let reflection_hit_groups: &[&[BindGroupLayoutEntry]] = &[&reflection_hits, &ray_material, &[], &reflection_fields];
        checked.push((&REFLECTIONS_HITS_COMPUTE, reflection_hit_groups));
        let bc7: &[&[BindGroupLayoutEntry]] = &[&crate::gpu_bc7::LAYOUT];
        for shader in [&BC7_TRYMODE456CS, &BC7_ENCODEBLOCKCS, &BC7_REFINEMODE56CS] {
            let reflection: Value = serde_json::from_str(shader.spirv_reflection).unwrap();
            let entry = reflection["entryPoints"].as_array().unwrap().iter()
                .find(|e| e["name"] == shader.entry).unwrap();
            assert_eq!(entry["threadGroupSize"], serde_json::json!([
                crate::constants::TEXTURE_BC7_GROUP_SIZE, 1, 1
            ]));
            checked.push((shader, bc7));
        }
        let filter = crate::passes::subsurface::entries(false);
        let composite = crate::passes::subsurface::entries(true);
        let filter_groups = [&filter[..]];
        let composite_groups = [&composite[..]];
        checked.extend([
            (&SUBSURFACE_SINGLE_SUBSURFACE_DIFFUSE, &filter_groups[..]),
            (&SUBSURFACE_COMPOSITE_SINGLE_SUBSURFACE_VS, &composite_groups[..]),
            (&SUBSURFACE_COMPOSITE_SINGLE_SUBSURFACE_COMPOSE, &composite_groups[..]),
        ]);
        let camera_layouts:Vec<_>=CAMERAS.iter().map(|(msaa,ray,_,_)| {
            let shadow=crate::pipelines::camera_shadow_entries(*ray!=0);
            vec![global.clone(),crate::pipelines::material_entries(*ray!=0),shadow,crate::passes::decoupled::camera_geometry_entries(),crate::passes::decoupled::camera_entries(if *msaa!=0 {crate::constants::RASTER_MSAA_SAMPLES} else {1})]
        }).collect();
        let camera_groups:Vec<Vec<&[BindGroupLayoutEntry]>>=camera_layouts.iter().map(|groups|groups.iter().map(Vec::as_slice).collect()).collect();
        for ((_,_,_,shader),groups) in CAMERAS.iter().zip(&camera_groups) {checked.push((*shader,&groups[..]));}
        let surface_layouts:Vec<Vec<Vec<BindGroupLayoutEntry>>>=SURFACES.iter().map(|(family,rays,probes,_,_,_)| {
            use crate::surface_native::SurfaceKernel;
            if family.camera() {
                vec![global.clone(),ray_material.clone(),crate::pipelines::camera_shadow_entries(*rays),crate::passes::decoupled::camera_geometry_entries(),crate::passes::decoupled::camera_entries(1)]
            } else {
                let entries=match family {
                    SurfaceKernel::Gi=>gi.clone(),
                    SurfaceKernel::Reflections=>reflection_hits.clone(),
                    SurfaceKernel::Probes|SurfaceKernel::Directional=>probe_update.clone(),
                    _=>unreachable!(),
                };
                let reads_probes=*probes || matches!(family,SurfaceKernel::Probes|SurfaceKernel::Directional);
                let geometry=match family {
                    SurfaceKernel::Reflections=>reflection_fields.clone(),
                    SurfaceKernel::Gi=>gi_hits.clone(),
                    _=>hits.to_vec(),
                };
                vec![entries,ray_material.clone(),if reads_probes {probe_read.to_vec()} else {vec![]},geometry]
            }
        }).collect();
        let surface_groups:Vec<Vec<&[BindGroupLayoutEntry]>>=surface_layouts.iter().map(|groups|groups.iter().map(Vec::as_slice).collect()).collect();
        for ((_,_,_,_,_,shader),groups) in SURFACES.iter().zip(&surface_groups) {checked.push((*shader,&groups[..]));}
        let volume_layouts:Vec<_> = VOLUME_VISIBILITY.iter().map(|&(pages,_,_,_)|
            vec![global.clone(),crate::pipelines::material_entries(false),crate::passes::decoupled::fragments_entries(pages),crate::passes::decoupled::fragments_geometry_entries()]).collect();
        let volume_groups:Vec<Vec<&[BindGroupLayoutEntry]>> = volume_layouts.iter().map(|groups|groups.iter().map(Vec::as_slice).collect()).collect();
        for ((_,_,_,shader),groups) in VOLUME_VISIBILITY.iter().zip(&volume_groups) {checked.push((*shader,&groups[..]));}
        for &(shader, groups) in &checked {
            check(shader, groups);
        }
        let environment = crate::environment::layout_entries();
        let environment_groups: &[&[BindGroupLayoutEntry]] = &[&environment];
        check(&ENVIRONMENT_FILTER_MAIN, environment_groups);
        checked.push((&ENVIRONMENT_FILTER_MAIN, environment_groups));
        let resolve_shaders = crate::passes::resolve::shaders();
        let resolve_layouts: Vec<_> = resolve_shaders.iter().map(|shader| {
            let mut entries = crate::passes::resolve::layout_entries(shader.entry);
            for (i,entry) in entries.iter_mut().enumerate() {entry.binding=i as u32;}
            entries
        }).collect();
        let output = [BindGroupLayoutEntry {binding:0,visibility:ShaderStages::COMPUTE,ty:BindingType::StorageTexture {access:wgpu::StorageTextureAccess::WriteOnly,format:wgpu::TextureFormat::Rgba8Unorm,view_dimension:wgpu::TextureViewDimension::D2},count:None}];
        let resolve_groups:Vec<Vec<&[BindGroupLayoutEntry]>> = resolve_layouts.iter().map(|entries|vec![&entries[..],&output[..]]).collect();
        for (shader,groups) in resolve_shaders.iter().zip(&resolve_groups) {check(shader,groups);checked.push((shader,&groups[..]));}
        for shader in ALL {
            assert!(
                checked.iter().any(|(s, _)| s.name == shader.name)
                    || [1, crate::constants::RASTER_MSAA_SAMPLES].into_iter().any(|samples| [false, true].into_iter().any(|denoise| crate::pipelines::control_resolve_shader(samples, denoise).name == shader.name)),
                "{} has no layout check",
                shader.name
            );
        }
    }
}
