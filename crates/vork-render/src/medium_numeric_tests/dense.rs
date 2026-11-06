use super::*;

#[test]
fn dense_overlap_preserves_finite_equilibrium_energy() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    // A bounded component and World are simultaneously active. The current
    // solid's known containment avoids a geometry approximation in this oracle.
    let source=r#"
import vork_medium_bounds;
import vork_froxel;
import vork_medium;
import vork_medium_types;
import vork_prelude;
[[vk::binding(0,0)]] StructuredBuffer<uint> words;
[[vk::binding(2,0)]] RWStructuredBuffer<float4> values;
static uint variant;
export Frame medium_frame() {Frame f={};f.medium.w=1u;f.radiance=float4(1.);return f;}
export uint medium_word(uint at) {return words[at];}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 0u;}
export uint medium_light_index(uint base,uint i) {return i;}
export Light medium_light(uint i) {return (Light)0;}
export float3 medium_visibility(float3 p,uint i) {return float3(1.);}
MediumSample coefficients(uint id) {
    MediumSample m={};
    let density=variant==2u ? 2. : 2e38;
    // Pure scattering; emission alone; and mixed RGB absorption/scattering.
    if(variant==6u) {m.emission=float3(1e-37);}
    else if(variant==5u) {m.absorption=float3(.1);m.emission=float3(1e38);}
    else if(variant==0u) {m.scattering=float3(density);}
    else if(variant==1u) {m.absorption=float3(density);m.emission=density*float3(.5,1.,.25);}
    else if(variant==4u) {m.absorption=float3(density);m.scattering=float3(density);m.emission=float3(density);}
    else {m.absorption=density*float3(.75,.5,.25);m.scattering=density*float3(.25,.5,.75);m.emission=density*float3(.5,.25,.125);}
    return m;
}
export MediumSample volume_coefficients(uint id,float3 p,MediumTransportContext c) {return coefficients(id);}
export bool volume_homogeneous(uint id) {return HOMOGENEOUS;}
export MediumSample volume_interval_coefficients(uint id,float3 o,float3 d,float distance,float fraction,MediumTransportContext c) {return coefficients(id);}
export MediumSample medium_world_coefficients(float3 p) {return coefficients(0u);}
export float medium_world_tail_start(float3 o,float3 d) {return 0.;}
export MediumSample medium_world_tail_coefficients(float3 o,float3 d) {return coefficients(0u);}
export void medium_world_exponential(out float3 a,out float3 s,out float3 g) {let m=coefficients(0u);a=m.absorption;s=m.scattering;g=0.;}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 tid:SV_DispatchThreadID) {
    variant=tid.x;
    let distance=variant==6u ? 1e38 : 1.;
    let m=volume_finite<7,true>(float3(0.),float3(0.,0.,1.),medium_receiver(distance),.37,0u,true);
    values[tid.x*5u]=float4(m.source,0.);values[tid.x*5u+1u]=float4(m.transmission,0.);
    let cell=froxel_estimate(m,distance);FroxelBlend blend={};
    froxel_blend(blend,cell,.25);froxel_blend(blend,cell,.75);
    values[tid.x*5u+2u]=float4(froxel_integrate(froxel_blended(blend),distance).source,0.);
    values[tid.x*5u+3u]=float4(medium_world_constant_tail<true>(float3(0.),float3(0.,0.,1.),.37,coefficients(0u)).source,0.);
    values[tid.x*5u+4u]=float4(medium_world_exponential_tail<true>(float3(0.),float3(0.,0.,1.),.37).source,0.);
}"#;
    // Header then a real bounded-domain descriptor (program 1), World program 0.
    let words=[4u32,1,0,0,0,0,0,1,0];
    let input=r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:None,contents:bytemuck::cast_slice(&words),usage:wgpu::BufferUsages::STORAGE});
    let objects=r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:None,contents:bytemuck::bytes_of(&Object::new(Mat4::IDENTITY).gpu()),usage:wgpu::BufferUsages::STORAGE});
    let bytes=7*5*16;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:true}),pipelines::buffer_entry(1,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:true}),pipelines::buffer_entry(2,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    for homogeneous in [false,true] {for world_only in [false,true] {
        let source=source.replace("HOMOGENEOUS",if homogeneous {"true"}else{"false"});
        let pipeline=test_pipeline(&r,&source,&Native {steps:7,homogeneous,world_only,..Default::default()},&entries,(1,1,1))?;
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,input.as_entire_binding()),entry(1,objects.as_entire_binding()),entry(2,output.as_entire_binding())]});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(7,1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
        let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.wait(Some(submission))?;rx.recv()??;
        let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
        for variant in 0..7 {for channel in 0..3 {
            let t=if variant==6 {1.}else if variant==2 {(if world_only {-2f64}else{-4f64}).exp()}else if variant==5 {(if world_only {-0.1f64}else{-0.2f64}).exp()}else{0.};
            let equilibrium=match variant {5=>1e39,0|4=>1.,1=>[0.5,1.,0.25][channel],_=>[0.75,0.75,0.875][channel]};
            let expected=if variant==6 {if world_only {10.} else {20.}} else {equilibrium*(1.-t)};
            if variant<5 {for closure in [3,4] {
                let tail=f64::from(values[variant*5+closure][channel]);
                assert!(tail.is_finite() && (tail-equilibrium).abs()<2e-5,"tail variant={variant} channel={channel}: {tail} != {equilibrium}");
            }}
            let actual=f64::from(values[variant*5][channel]);
            assert!(actual.is_finite() && (actual-expected).abs()<2e-5*expected.max(1.),"homogeneous={homogeneous} world_only={world_only} variant={variant} channel={channel}: {actual} != {expected}");
            assert!((f64::from(values[variant*5+1][channel])-t).abs()<2e-5);
            let cached=f64::from(values[variant*5+2][channel]);
            assert!(cached.is_finite() && (cached-expected).abs()<2e-5*expected.max(1.),"froxel source {cached} != {expected}");
        }}
        drop(mapped);readback.unmap();
    }}
    r.errors.check()?;Ok(())
}
