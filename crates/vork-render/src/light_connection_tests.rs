use crate::*;

/// The production sampler must land on the authored source and report its
/// conditional measure. The old facing disk fails the sphere-surface invariant
/// and does not produce a uniform solar-cap cosine distribution.
#[test]
#[ignore = "requires pinned Slang and Vulkan (lavapipe works)"]
fn connection_shape_endpoints_and_conditional_pdfs() -> Result<()> {
    let r = Renderer::new()?;
    let source = r#"
import emitter;
import variant;
import frame;
import vork_constants;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> result;
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    let kind=id.x;
    LightValues l={};l.position=float4(0.,0.,4.,1.);l.direction=float4(0.,0.,-1.,-1.);
    l.radiance=float4(1.);l.emitter_u=float4(4.,0.,0.,0.);l.emitter_v=float4(0.,2.,0.,0.);
    if(kind==1u || kind==4u) {l.position.w=0.;}
    if(kind==2u || kind==3u) {l.params.y=2.;}
    if(kind==3u) {l.position.w=2.;}
    if(kind==4u) {l.params.y=.4;}
    if(kind>=5u) {l.position.w=3.;l.emitter_u.w=kind==6u ? 1. : 0.;}
    var endpoint_error=0.;var pdf_error=0.;var cosine=0.;var old_error=0.;
    for(var y=0u;y<32u;y++) {for(var x=0u;x<32u;x++) {
        let u=(float2(x,y)+.5)/32.;
        let c=emitter_sample_li<Unspecialized>(l,kind,float3(0.),u);
        cosine+=c.direction.z/1024.;
        if(kind==2u || kind==3u) {
            endpoint_error=max(endpoint_error,abs(length(c.endpoint-l.position.xyz)-2.));
            let expected=1./(REG_TAU*(1.-sqrt(.75)));
            pdf_error=max(pdf_error,abs(c.pdf/expected-1.));
            // Old copy placed a uniform disk in the centre plane, inside the
            // physical sphere except on its boundary.
            old_error=max(old_error,abs(2.*sqrt(u.x)-2.));
        } else if(kind==4u) {
            pdf_error=max(pdf_error,abs(c.pdf*(REG_TAU*(1.-cos(.4)))-1.));
            let disk_radius=sqrt(u.x)*tan(.4);
            old_error+=rsqrt(1.+disk_radius*disk_radius)/1024.;
        } else if(kind>=5u) {
            endpoint_error=max(endpoint_error,abs(c.endpoint.z-4.));
            let q=c.endpoint.xy/float2(2.,1.);
            endpoint_error=max(endpoint_error,kind==6u ? max(dot(q,q)-1.,0.) : max(max(abs(q.x),abs(q.y))-1.,0.));
            let area=kind==6u ? 2.*REG_PI : 8.;
            pdf_error=max(pdf_error,abs(c.pdf*area*c.direction.z/(c.distance*c.distance)-1.));
        } else {pdf_error=max(pdf_error,abs(c.pdf));}
    }}
    result[kind]=float4(endpoint_error,pdf_error,cosine,old_error);
}
"#;
    let entries = [pipelines::buffer_entry(0, wgpu::ShaderStages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: false })];
    let pipeline = medium::test_pipeline(&r, source, &medium::Native::default(), &entries, (1,1,1))?;
    let size = 7*16;
    let output = r.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
    let readback = r.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[entry(0,output.as_entire_binding())] });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    { let mut pass = encoder.begin_compute_pass(&Default::default()); pass.set_pipeline(&pipeline); pass.set_bind_group(0,&bind,&[]); pass.dispatch_workgroups(7,1,1); }
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,size);
    let submission = r.queue.submit([encoder.finish()]);
    let (tx,rx) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
    r.wait(Some(submission))?;rx.recv()??;
    let mapped = readback.get_mapped_range(..);
    let rows: &[[f32;4]] = bytemuck::cast_slice(&mapped);
    for (kind,row) in rows.iter().enumerate() {
        assert!(row[0]<1e-4 && row[1]<1e-4,"shape {kind} endpoint/PDF: {row:?}");
    }
    let sphere_mean = (1.+0.75_f32.sqrt())*0.5;
    assert!((rows[2][2]-sphere_mean).abs()<1e-5);
    assert!((rows[3][2]-sphere_mean).abs()<1e-5);
    let sun_mean = (1.+0.4_f32.cos())*0.5;
    assert!((rows[4][2]-sun_mean).abs()<1e-5);
    assert!(rows[2][3]>1.,"old disk must fail endpoint invariant");
    assert!((rows[4][3]-sun_mean).abs()>0.0005,"old tangent disk must fail cap measure");
    eprintln!("connection before/after: old sphere endpoint max error {} -> {}; solar cosine mean {} -> {} (reference {sun_mean})",rows[2][3],rows[2][0],rows[4][3],rows[4][2]);
    Ok(())
}

/// Lobe exclusion must preserve the cosine response independently of Fresnel,
/// anisotropy and film, while the enabled lobes remain additive.
#[test]
#[ignore = "requires pinned Slang and Vulkan (lavapipe works)"]
fn direct_brdf_masks_preserve_cosine_and_lobe_additivity() -> Result<()> {
    let r = Renderer::new()?;
    let source = r#"
import bsdf_response;
import closure;
import vork_constants;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> result;
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    let mu=(float(id.x%64u)+.5)/64.;
    let phi=float(id.x/64u)*2.39996323;
    let view=normalize(float3(.7,.2,1.));
    let light=float3(sqrt(1.-mu*mu)*cos(phi),sqrt(1.-mu*mu)*sin(phi),mu);
    Closure c={};c.geometry_normal=float3(0.,0.,1.);c.geometry_tangent=float3(1.,0.,0.);
    c.base_weight=1.;c.base_color=float3(.7);c.specular_weight=1.;c.specular_color=float3(1.);c.specular_ior=1.5;
    c.specular_roughness=.02+.97*float((id.x/64u)%8u)/7.;
    c.base_metalness=float((id.x/512u)%3u)*.5;
    c.specular_roughness_anisotropy=(id.x&1u)!=0u ? .7 : 0.;
    c.thin_film_weight=(id.x&2u)!=0u ? 1. : 0.;c.thin_film_thickness=400.;c.thin_film_ior=1.3;
    let diffuse=float3(.2,.4,.8);let compensation=float3(.1,.2,.3);
    let d=direct_brdf_prepared(c,view,light,REG_RAY_DIFFUSE,diffuse,compensation);
    let s=direct_brdf_prepared(c,view,light,REG_RAY_SPECULAR,diffuse,compensation);
    let both=direct_brdf_prepared(c,view,light,REG_RAY_DIFFUSE|REG_RAY_SPECULAR,diffuse,compensation);
    let none=direct_brdf_prepared(c,view,light,0u,diffuse,compensation);
    let error=abs(d-diffuse*mu);let sum_error=abs(both-d-s)/max(abs(both),float3(1.));
    result[id.x]=float4(max(error.x,max(error.y,error.z)),max(sum_error.x,max(sum_error.y,sum_error.z)),
        length(none),all(isfinite(d)) && all(isfinite(s)) && all(isfinite(both)) ? 1. : 0.);
}
"#;
    let entries = [pipelines::buffer_entry(0, wgpu::ShaderStages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: false })];
    let pipeline = medium::test_pipeline(&r, source, &medium::Native::default(), &entries, (1,1,1))?;
    let size = 4096*16;
    let buffer = |usage| r.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage, mapped_at_creation: false });
    let output = buffer(wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC);
    let readback = buffer(wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST);
    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[entry(0,output.as_entire_binding())] });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    { let mut pass = encoder.begin_compute_pass(&Default::default()); pass.set_pipeline(&pipeline); pass.set_bind_group(0,&bind,&[]); pass.dispatch_workgroups(4096,1,1); }
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,size);
    let submission = r.queue.submit([encoder.finish()]);
    let (tx,rx) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
    r.wait(Some(submission))?;rx.recv()??;
    let mapped = readback.get_mapped_range(..);
    let rows: &[[f32;4]] = bytemuck::cast_slice(&mapped);
    for (i,row) in rows.iter().enumerate() {
        assert!(row[0]<1e-6 && row[1]<1e-6 && row[2]==0. && row[3]==1., "BRDF mask case {i}: {row:?}");
    }
    r.errors.check()?;
    Ok(())
}
