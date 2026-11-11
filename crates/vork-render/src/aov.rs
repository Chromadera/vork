//! AOV resolve uses the same sample weights and retained planar output ring as colour.
use crate::*;

pub(crate) struct Resolve {
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    size: [u32;2],
}
impl Resolve {
    pub(crate) fn new(s:&InstalledScene,r:&Renderer,uniform:&wgpu::Buffer,values:&wgpu::Buffer,pixels:&wgpu::Buffer,size:[u32;2],internal:[u32;2],base:u32)->Result<Option<Self>> {
        if s.render.aovs.is_empty() {return Ok(None);}
        use std::fmt::Write;
        let metal=r.adapter_info.backend==wgpu::Backend::Metal;
        // Slang renames `main` for Metal; retain an explicit native entry name.
        let entry_point="aov_accumulate";
        let mut source=String::from("import vork_prelude;\n[[vk::binding(0,0)]] ConstantBuffer<Frame> frame : register(b0);\n[[vk::binding(1,0)]] RWStructuredBuffer<uint> pixels : register(u1);\n");
        source.push_str("[[vk::binding(2,0)]] StructuredBuffer<float4> aovs : register(t2);\n");
        let count=s.render.aovs.len();
        let [iw,ih]=internal;
        let [w,h]=size;
        writeln!(source,"[shader(\"compute\")][numthreads(8,8,1)] void {entry_point}(uint3 id:SV_DispatchThreadID) {{if(id.x>={w}u || id.y>={h}u) return;uint pixel=id.y*{w}u+id.x;float weight=1./frame.temporal.y;")?;
        writeln!(source,"uint2 p=min(uint2((float2(id.xy)+.5)*float2({iw},{ih})/float2({w},{h})),uint2({iw}u,{ih}u)-1u);for(uint i=0u;i<{count}u;i++){{float4 v=aovs[(p.y*{iw}u+p.x)*{count}u+i];for(uint c=0u;c<4u;c++){{uint at=({base}u+i*4u+c)*{}u+pixel;float old=frame.temporal.x==0. ? 0. : asfloat(pixels[at]);pixels[at]=asuint(old+v[c]*weight);}}}}",w*h)?;
        source.push_str("}\n");
        let (_,target,flags)=if metal {crate::forward_variant::METAL_TARGET} else {crate::forward_variant::SPIRV_TARGET};
        let path=r.disk_cache.path("aov-resolve",1,blake3::hash(format!("{target}{source}").as_bytes()));
        let code=if let Some(code)=crate::disk_cache::read(&path) {code} else {
            let mut modules=crate::native::SOURCES.to_vec();modules.push(("aov_resolve",&source));
            let args:Vec<_>=["-target",target].into_iter().chain(crate::forward_variant::SLANG_FLAGS.iter().copied()).chain(flags.iter().copied()).collect();
            let code=vork_slang::Compiler::discover().compile_modules(&modules,"aov_resolve",&[(entry_point,"compute")],&args)?;
            let _=crate::disk_cache::write(&path,&code);code
        };
        let shader=if metal {crate::native::module(&r.device,"AOV resolve",None,Some(&String::from_utf8(code)?),(8,8,1))} else {crate::native::module(&r.device,"AOV resolve",Some(&code),None,(8,8,1))};
        let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Uniform),pipelines::buffer_entry(1,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage{read_only:false}),pipelines::buffer_entry(2,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage{read_only:true})];
        let group=r.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("AOV resolve"),entries:&entries});
        let layout=r.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("AOV resolve"),bind_group_layouts:&[Some(&group)],immediate_size:0});
        let pipeline=r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some("AOV resolve"),layout:Some(&layout),module:&shader,entry_point:Some(entry_point),compilation_options:Default::default(),cache:r.cache.as_ref()});
        let entries=[entry(0,uniform.as_entire_binding()),entry(1,pixels.as_entire_binding()),entry(2,values.as_entire_binding())];
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("AOV resolve"),layout:&group,entries:&entries});
        Ok(Some(Self{pipeline,bind,size}))
    }
    pub(crate) fn encode(&self,encoder:&mut wgpu::CommandEncoder) {
        let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("AOV sample accumulation"),timestamp_writes:None});
        pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&self.bind,&[]);pass.dispatch_workgroups(self.size[0].div_ceil(8),self.size[1].div_ceil(8),1);
    }
}

/// Admit the scene's interleaved float4 AOV storage before compiling its shaders.
/// Resolved values and packed output planes both use the authored resolution.
pub(crate) fn check_storage(limits: &wgpu::Limits, render: &vork_scene::v2::Render) -> Result<()> {
    let count = render.aovs.len() as u64;
    if count == 0 { return Ok(()); }
    for (name, [w, h]) in [
        ("AOV resolved", render.resolution),
        ("AOV output planes", render.resolution),
    ] {
        // The packed frame also retains its built-in planes. Float readback
        // checks its larger prefix when the caller selects that view precision.
        let prefix = if name == "AOV output planes" {u64::from(crate::constants::PACKED_OUTPUT_PLANES)*4} else {0};
        let bytes = count.checked_mul(16).and_then(|n| n.checked_add(prefix)).and_then(|n| n.checked_mul(u64::from(w))).and_then(|n| n.checked_mul(u64::from(h)));
        anyhow::ensure!(bytes.is_some_and(|bytes| bytes <= limits.max_storage_buffer_binding_size && bytes <= limits.max_buffer_size),
            "{}", crate::gpu_memory::limit_message(format_args!("render.aovs: {count} declared; {name}"), format_args!("{} bytes at {w}x{h}",bytes.map_or_else(|| "more than u64::MAX".to_owned(), |n| n.to_string())), format_args!("max_storage_buffer_binding_size={} bytes, max_buffer_size is {} bytes",limits.max_storage_buffer_binding_size,limits.max_buffer_size), "reduce resolution or the number of render.aovs"));
    }
    Ok(())
}

#[cfg(all(test, vork_private_tests))]
include!("aov_tests_private.rs");
