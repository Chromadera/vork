//! Compute's immutable geometric-query provider. Logical buffer writes and AS
//! readers are admitted separately; no query can read its own changing geometry.
use crate::*;

pub(crate) struct Scene<'a> {
    pub nodes:&'a [install::EvalNode],
    pub objects:&'a [Object],
    pub materials:&'a [Material],
    pub integrator:&'a vork_scene::Integrator,
    pub words:&'a [u32],
    pub word_base:usize,
    pub identity:Option<blake3::Hash>,
}
pub(crate) struct Queries {
    pub identity:blake3::Hash,
    _snapshot:rt::ComputeSnapshot,
    diagnostics:wgpu::Buffer,
    global:wgpu::BindGroup,
    tables:wgpu::BindGroup,
    hits:wgpu::BindGroup,
    layouts:[wgpu::BindGroupLayout;3],
    publish:wgpu::ComputePipeline,
}
const TABLES:&str=r#"
import vork_constants;
[[vk::binding(0,2)]] public StructuredBuffer<uint> material_words;
[[vk::binding(1,2)]] public RWStructuredBuffer<uint> shader_errors;
[[vk::binding(2,2)]] public RaytracingAccelerationStructure compute_scene;
[[vk::binding(3,2)]] public StructuredBuffer<uint> compute_words;
[[vk::binding(0,3)]] public StructuredBuffer<uint> compute_hit_buffers[REG_RAY_HIT_BUFFERS];
"#;
const PROVIDER:&str=r#"
import compute_trace_tables;
import geometric_trace;
import hit_geometry;
struct ComputeGeometry : IHitGeometry {
    static uint word(uint i) {return compute_words[i];}
    static uint load(uint buffer,uint i) {return compute_hit_buffers[NonUniformResourceIndex(buffer)][i];}
}
export TraceHit native_geometric_trace(float3 o,float3 d,float lo,float hi,uint mask,uint object) {
    return geometric_trace<ComputeGeometry>(compute_scene,o,d,lo,hi,mask,object);
}
"#;
impl Queries {
    pub(crate) fn new(r:&Renderer,filters:&[vork_slang::QueryFilter],source:&vork_scene::v2::Scene,scene:Scene<'_>,roots:&[install::MeshRoot],draws:&[install::Draw])->Result<Self> {
        let mut identity=blake3::Hasher::new();identity.update(scene.identity.expect("query snapshot identity").as_bytes());identity.update(bytemuck::cast_slice(scene.objects));
        let snapshot=rt::ComputeSnapshot::new(r,filters,source,scene.nodes,roots,draws,scene.objects,scene.materials,scene.integrator)?;
        let diagnostics=init(&r.device,"compute query diagnostics",bytemuck::cast_slice(&materials::SHADER_DIAGNOSTICS_INIT),wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST);
        let mut words=vec![0u32;scene.word_base];words.extend_from_slice(scene.words);if words.is_empty() {words.push(0);}
        let words=init(&r.device,"compute query identities",bytemuck::cast_slice(&words),wgpu::BufferUsages::STORAGE);
        let buffer=|binding,read_only|pipelines::buffer_entry(binding,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only});
        let layout=|label,entries:&[_]|r.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some(label),entries});
        let global_layout=layout("compute query objects",&[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Uniform),buffer(1,true)]);
        let table_layout=layout("compute query snapshot",&[buffer(0,true),buffer(1,false),wgpu::BindGroupLayoutEntry {binding:2,visibility:wgpu::ShaderStages::COMPUTE,ty:wgpu::BindingType::AccelerationStructure {vertex_return:false},count:None},buffer(3,true)]);
        let hit_layout=layout("compute query geometry",&[rt::hit_buffers_entry(0,wgpu::ShaderStages::COMPUTE)]);
        let bind=|label,layout,entries:&[_]|r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some(label),layout,entries});
        // Native Vulkan bindings use the backend's dense slot order. Preserve
        // frame's unused slot zero so its object reader stays at binding one.
        let unused_frame=init(&r.device,"compute query frame slot",&[0;16],wgpu::BufferUsages::UNIFORM);
        let global=bind("compute query objects",&global_layout,&[entry(0,unused_frame.as_entire_binding()),entry(1,snapshot.objects.as_entire_binding())]);
        let tables=bind("compute query snapshot",&table_layout,&[entry(0,words.as_entire_binding()),entry(1,diagnostics.as_entire_binding()),entry(2,wgpu::BindingResource::AccelerationStructure(&snapshot.tlas)),entry(3,snapshot.rays.alpha.as_entire_binding())]);
        let buffers:Vec<_>=snapshot.rays.hit_buffers.iter().map(|b|match b {rt::HitBuffer::Static(b)=>b.as_entire_buffer_binding(),_=>unreachable!("static query snapshot")}).collect();
        let hits=bind("compute query geometry",&hit_layout,&[entry(0,wgpu::BindingResource::BufferArray(&buffers))]);
        let source=format!("@group(0) @binding(0) var<storage,read_write> source:array<atomic<u32>>;@group(0) @binding(1) var<storage,read_write> destination:array<atomic<u32>>;@compute @workgroup_size(1) fn main() {{atomicMax(&destination[0],atomicLoad(&source[0]));atomicMin(&destination[1],atomicLoad(&source[1]));let rays=atomicExchange(&source[{}],0u);let before=atomicAdd(&destination[{}],rays);if(before>{}u || rays>{}u-before) {{atomicMax(&destination[0],{}u);atomicMin(&destination[1],1u);}}}}",vork_slang::registry::TRACE_RAYS_WORD,vork_slang::registry::TRACE_RAYS_WORD,vork_slang::registry::TRACE_RAY_LIMIT,vork_slang::registry::TRACE_RAY_LIMIT,vork_slang::registry::TRACE_EXHAUSTED);
        let publish=compute::pipeline_layout(r,&source,"compute query diagnostics",&[Some(false),Some(false)],"main");
        Ok(Self {identity:identity.finalize(),_snapshot:snapshot,diagnostics,global,tables,hits,layouts:[global_layout,table_layout,hit_layout],publish})
    }
    pub(crate) fn bind<'a>(&'a self,pass:&mut wgpu::ComputePass<'a>) {
        pass.set_bind_group(0,&self.global,&[]);pass.set_bind_group(2,&self.tables,&[]);pass.set_bind_group(3,&self.hits,&[]);
    }
    pub(crate) fn publish(&self,r:&Renderer,encoder:&mut wgpu::CommandEncoder,destination:&wgpu::Buffer) {
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("compute query diagnostics"),layout:&self.publish.get_bind_group_layout(0),entries:&[entry(0,self.diagnostics.as_entire_binding()),entry(1,destination.as_entire_binding())]});
        let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&self.publish);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);
    }
    pub(crate) fn pipeline(&self,r:&Renderer,prepared:&compute::PreparedProgram,types:&[(u32,Option<bool>)])->Result<wgpu::ComputePipeline> {
        let metal=r.adapter_info.backend==wgpu::Backend::Metal;
        ensure!(!metal,"compute trace native Metal execution is not qualified yet");
        let dense:std::collections::BTreeMap<_,_>=types.iter().map(|(binding,_)|*binding).collect::<std::collections::BTreeSet<_>>().into_iter().enumerate().map(|(slot,binding)|(binding,slot)).collect();
        let binding=dense.get(&prepared.input_binding).copied().unwrap_or(0);
        let mut header=prepared.header.clone();
        for binding in 0..prepared.input_binding {
            // Native modules bypass Naga's resource remap. Match Vulkan's dense
            // slots while leaving eliminated declarations at unused unique slots.
            let slot=dense.get(&binding).copied().unwrap_or(types.len()+binding as usize);
            header=header.replace(&format!("[[vk::binding({binding},0)]]"),&format!("[[vk::binding({slot},1)]]"));
        }
        let inputs=if prepared.artifact.inputs.fields.is_empty() {"public uint compute_input(uint i){return 0u;}".to_owned()} else {format!("[[vk::binding({binding},1)]] StructuredBuffer<uint> compute_inputs;public uint compute_input(uint i){{return compute_inputs[i];}}")};
        let mut modules:Vec<(String,String)>=native::SOURCES.iter().map(|(n,s)|((*n).into(),(*s).into())).collect();
        for (name,mut text) in programs::abi().native_modules() {
            if name=="vork" {text=format!("import compute_inputs;\n{}",text.replace("return material_words[index];","return compute_input(index);"));}
            if let Some(module)=modules.iter_mut().find(|m|m.0==name) {*module=(name,text);} else {modules.push((name,text));}
        }
        let geometric=modules.iter_mut().find(|m|m.0=="geometric_trace").unwrap();geometric.1=geometric.1.replace("import material;","import compute_trace_tables;");
        modules.push(("compute_trace_tables".into(),TABLES.into()));modules.push(("compute_inputs".into(),inputs));
        modules.push(("compute_query_provider".into(),PROVIDER.into()));
        modules.push(("compute_query_main".into(),format!("import compute_query_provider;\n{}",prepared.artifact.slang.replacen(&prepared.header,&header,1))));
        let modules:Vec<_>=modules.iter().map(|(n,s)|(n.as_str(),s.as_str())).collect();
        let mut args=vec!["-target","spirv","-O2","-matrix-layout-row-major","-emit-spirv-directly","-fvk-use-entrypoint-name"];
        let include=prepared.origin.parent().unwrap_or(std::path::Path::new("."));let include=include.to_string_lossy();args.extend(["-I",include.as_ref()]);
        let compiler=vork_slang::Compiler::discover();
        let cache=|hash|r.disk_cache.path("slang-variant",1,hash);
        let identities=std::collections::BTreeMap::new();
        let inputs=std::slice::from_ref(&prepared.identity);
        let (identity,prepared_native)=programs::native_input_identity(&cache,&compiler,&modules,"compute_query_main",&[("vork_entry","compute")],&args,&identities,inputs)?;
        let receipt_path=programs::native_input_receipt_path(&cache,&compiler,&modules,"compute_query_main",&[("vork_entry","compute")],&args,&identities)?;
        let code=programs::native_code(&identity,prepared_native,inputs,&receipt_path,
            |identity|cache(blake3::hash(identity)),
            ||compiler.prepare_identified_modules(&modules,"compute_query_main",&[("vork_entry","compute")],&args,&identities),
            |prepared|prepared.compile())?;
        let module=native::module(&r.device,"compute geometric query",Some(&code),None,(0,0,0));
        let entries:Vec<_>=types.iter().map(|(binding,ty)|pipelines::buffer_entry(*binding,wgpu::ShaderStages::COMPUTE,ty.map_or(wgpu::BufferBindingType::Uniform,|read_only|wgpu::BufferBindingType::Storage {read_only}))).collect();
        let compute_layout=r.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("compute query buffers"),entries:&entries});
        let layout=r.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("compute query layout"),bind_group_layouts:&[Some(&self.layouts[0]),Some(&compute_layout),Some(&self.layouts[1]),Some(&self.layouts[2])],immediate_size:0});
        Ok(r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("compute geometric query"),layout:Some(&layout),module:&module,entry_point:Some("vork_entry"),compilation_options:Default::default(),cache:r.cache.as_ref()}))
    }
}
