//! Finish a complete image shader through the pinned native optimizer.
use super::*;
impl Compiler {
    /// `slots` maps authored group/binding pairs to backend binding numbers.
    /// Sampler-bearing shaders retain the renderer's WGSL compilation path.
    pub fn compile_image_spirv(&self,source:&str,slots:&[(u32,u32,u32)])->Result<Vec<u8>> {
        let module=naga::front::wgsl::parse_str(source).map_err(|e|anyhow::anyhow!("{}",e.emit_to_string(source)))?;
        let info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32)
            .validate(&module).context("validate complete image shader")?;
        ensure!(!module.types.iter().any(|(_,ty)|matches!(ty.inner,naga::TypeInner::Sampler {..})),"native image lowering does not support samplers");
        let entry=module.entry_points.iter().position(|e|e.name=="main" && e.stage==naga::ShaderStage::Compute).context("image shader requires compute main")?;
        for (handle,global) in module.global_variables.iter() {if let Some(binding)=global.binding {
            ensure!(info.get_entry_point(entry)[handle].is_empty() || slots.iter().any(|&(g,b,_)|g==binding.group && b==binding.binding),
                "image shader uses unbound resource {}:{}",binding.group,binding.binding);
        }}
        let mut options=naga::back::hlsl::Options {fake_missing_bindings:false,force_loop_bounding:false,..Default::default()};
        let mut extra=BTreeMap::<u32,u32>::new();
        for &(group,_,physical) in slots {let next=extra.entry(group).or_default();*next=(*next).max(physical+1);}
        for (_,global) in module.global_variables.iter() {if let Some(binding)=global.binding {
            let physical=slots.iter().find(|&&(g,b,_)|g==binding.group && b==binding.binding).map(|&(_,_,p)|p).unwrap_or_else(|| {
                let next=extra.entry(binding.group).or_default();let physical=*next;*next+=1;physical
            });
            options.binding_map.insert(binding,naga::back::hlsl::BindTarget {space:binding.group.try_into()?,register:physical,
                binding_array_size:None,dynamic_storage_buffer_offsets_index:None,restrict_indexing:true});
        }}
        let pipeline=naga::back::hlsl::PipelineOptions {entry_point:Some((naga::ShaderStage::Compute,"main".into()))};
        let mut hlsl=String::new();
        naga::back::hlsl::Writer::new(&mut hlsl,&options,&pipeline).write(&module,&info,None).context("lower image shader to HLSL")?;
        // HLSL typed UAVs do not distinguish 16- from 32-bit float formats.
        // Preserve the WGSL storage-image format in Slang's Vulkan annotation.
        for (_,global) in module.global_variables.iter() {
            if let (Some(binding),naga::TypeInner::Image {class:naga::ImageClass::Storage {format,..},..})=(global.binding,&module.types[global.ty].inner) {
                let format=match format {naga::StorageFormat::Rgba16Float=>"rgba16f",naga::StorageFormat::Rgba32Float=>"rgba32f",naga::StorageFormat::R32Float=>"r32f",_=>bail!("unsupported native image storage format {format:?}")};
                let physical=options.binding_map[&binding];
                let register=if physical.space==0 {format!("register(u{})",physical.register)} else {format!("register(u{}, space{})",physical.register,physical.space)};
                let mut scalar_image=None;
                hlsl=hlsl.lines().map(|line|if line.contains(&register) {
                    if format=="r32f" {scalar_image=line.split(" : register").next().and_then(|s|s.split_whitespace().last()).map(str::to_owned);}
                    format!("[[vk::image_format(\"{format}\")]] {line}\n")
                } else {format!("{line}\n")}).collect();
                // DXC permits vector truncation on scalar UAV stores; Slang
                // requires the component explicitly. WGSL imageStore supplies 4.
                if let Some(name)=scalar_image {
                    let prefix=format!("{name}[");
                    hlsl=hlsl.lines().map(|line| {
                        if line.trim_start().starts_with(&prefix) {if let Some((left,right))=line.split_once(" = ") {
                            if let Some(value)=right.strip_suffix(';') {return format!("{left} = ({value}).x;\n");}
                        }}
                        format!("{line}\n")
                    }).collect();
                }
            }
        }
        let mut args=vec!["-target".to_owned(),"spirv".into(),"-O2".into(),"-fvk-use-entrypoint-name".into()];
        let groups:BTreeSet<_>=module.global_variables.iter().filter_map(|(_,g)|g.binding.map(|b|b.group)).collect();
        for group in groups {for shift in ["-fvk-b-shift","-fvk-t-shift","-fvk-u-shift","-fvk-s-shift"] {
            args.extend([shift.into(),"0".into(),group.to_string()]);
        }}
        self.compile_modules(&[("image",&hlsl)],"image",&[("main","compute")],&args.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore="requires pinned Slang toolchain"]
    fn image_native_retains_bindings_and_storage_formats()->Result<()> {
        let source=r#"
struct Frame {value:vec4<f32>}
@group(0) @binding(0) var<storage,read> words:array<u32,1>;
@group(0) @binding(1) var<storage,read> source:array<vec4<f32>,1>;
@group(0) @binding(2) var<storage,read_write> destination:array<vec4<f32>,1>;
@group(0) @binding(3) var<uniform> frame:Frame;
@group(0) @binding(7) var half_image:texture_storage_2d<rgba16float,write>;
@group(0) @binding(8) var scalar_image:texture_storage_2d<r32float,write>;
@compute @workgroup_size(1) fn main() {
 let value=source[0]+vec4(f32(words[0]))+frame.value;
 destination[0]=value;textureStore(half_image,vec2(0),value);textureStore(scalar_image,vec2(0),value);
}
"#;
        let slots=[(0,0,0),(0,1,1),(0,2,2),(0,3,3),(0,7,4),(0,8,5)];
        let compiler=Compiler::discover();
        let missing:Vec<_>=slots.iter().copied().filter(|&(_,binding,_)|binding!=1).collect();
        assert!(compiler.compile_image_spirv(source,&missing).unwrap_err().to_string().contains("unbound resource 0:1"));
        let code=compiler.compile_image_spirv(source,&slots)?;
        let words:Vec<_>=code.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(words[0],0x07230203);
        let mut bindings=BTreeMap::new();let mut groups=BTreeMap::new();let mut formats=BTreeSet::new();let mut offset=5;
        while offset<words.len() {
            let count=(words[offset]>>16) as usize;assert!(count>0 && offset+count<=words.len());
            match words[offset]&0xffff {
                71 if count==4=>match words[offset+2] {
                    33=>{bindings.insert(words[offset+1],words[offset+3]);},
                    34=>{groups.insert(words[offset+1],words[offset+3]);},_=>{}
                },
                25 if count>=9=>{formats.insert(words[offset+8]);},_=>{}
            }
            offset+=count;
        }
        assert_eq!(bindings.values().copied().collect::<BTreeSet<_>>(),BTreeSet::from([0,1,2,3,4,5]));
        for id in bindings.keys() {assert_eq!(groups.get(id),Some(&0));}
        assert!(formats.contains(&2) && formats.contains(&3),"rgba16f and r32f image formats: {formats:?}");
        Ok(())
    }
}
