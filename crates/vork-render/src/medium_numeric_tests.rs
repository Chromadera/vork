use super::*;
use wgpu::util::DeviceExt;
#[test]
#[ignore = "requires pinned Slang and Vulkan (lavapipe works)"]
fn path_medium_free_flight_and_phase() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import path_medium;
import vork_medium_point;
import vork_constants;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    if(id.x==0u) {
        var sigma=16777216.;var bound=sigma;var reverse=0.;
        for(var j=0u;j<6u;j++) {sigma+=1.25;bound=medium_majorant_add(bound,1.25);reverse+=1.25;}
        reverse+=16777216.;
        values[27]=float4(sigma,bound,reverse,medium_majorant_add(0.,0.));
        values[28]=float4(medium_sample_majorant(float3(asfloat(1u)),float3(0.)),medium_phase(.999,1.),medium_phase(-.999,-1.),medium_phase(1.,1.));
    }
    float gs[9]={-1.,-.999,-.6,-.000001,0.,.000001,.6,.999,1.};
    let g=gs[id.x];let sigma=id.x%3u==0u ? float3(0.) : id.x%3u==1u ? float3(.2,.8,1.3) : float3(0.,.7,1.2);
    var survival=float3(0.);var collisions=float3(0.);var mean=0.;var second=0.;var norm=0.;
    for (var i=0u;i<8192u;i++) {
        let u=(float(i)+.5)/8192.;
        for (var c=0u;c<3u;c++) {
            let flight=medium_free_flight(sigma,2.,float2((float(c)+.5)/3.,u));
            if (flight.event) {collisions+=flight.weight*(sigma*.7);}else{survival+=flight.weight;}
        }
        let phase=medium_phase_sample(g,u);mean+=phase.cosine;second+=phase.cosine*phase.cosine;
        if(abs(g)<=.6) {norm+=medium_phase(g,2.*u-1.)*4.*REG_PI;}
    }
    values[id.x*3u]=float4(survival/24576.,0.);
    values[id.x*3u+1u]=float4(collisions/24576.,0.);
    values[id.x*3u+2u]=float4(mean/8192.,second/8192.,norm/8192.,medium_phase_sample(g,.5).delta ? 1. : 0.);
}"#;
    let bytes=29*16;
    let output=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:Some("path medium measures"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,source,&Native::default(),&entries,(1,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(9,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
    r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    assert!(values[27][2]<values[27][0],"regression must expose reversed-order underbound");
    assert!(values[27][1]>=values[27][0],"outward bound must cover coefficient accumulation");
    assert_eq!(values[27][3],0.);
    assert_eq!(values[28][0],f32::MIN_POSITIVE);
    let g=f64::from(0.999_f32);let peak=(1.+g)/(4.*std::f64::consts::PI*(1.-g).powi(2));
    for c in [1,2] {assert!((f64::from(values[28][c])/peak-1.).abs()<1e-5,"HG peak {:?} != {peak}",values[28]);}
    assert_eq!(values[28][3],0.); // Delta has no finite solid-angle density.
    for (i,g) in [-1_f32,-0.999,-0.6,-0.000001,0.,0.000001,0.6,0.999,1.].into_iter().enumerate() {
        let sigma=match i%3 {0=>[0_f32;3],1=>[0.2,0.8,1.3],_=>[0.,0.7,1.2]};
        for c in 0..3 {
            let survival=(-2.*sigma[c]).exp();
            assert!((values[i*3][c]-survival).abs()<0.001,"survival {i}/{c}: {:?}",values[i*3]);
            assert!((values[i*3+1][c]-0.7*(1.-survival)).abs()<0.001,"collision {i}/{c}: {:?}",values[i*3+1]);
        }
        let moments=values[i*3+2];
        assert!((moments[0]-g).abs()<0.001,"phase mean {g}: {moments:?}");
        assert!((moments[1]-(1.+2.*g*g)/3.).abs()<0.001,"phase second {g}: {moments:?}");
        if g.abs()<=0.6 {assert!((moments[2]-1.).abs()<0.001,"phase integral {g}: {moments:?}");}
        assert_eq!(moments[3],if g.abs()==1. {1.}else{0.});
    }
    r.errors.check()?;Ok(())
}

/// Compare retained exponential optical-depth numerics to an f64 reference.
#[test]
fn exponential_optical_depth_matches_f64_in_all_ray_limits() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import vork_constants;
import vork_medium_point;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    if(id.x==8u) {
        let tau=medium_optical_depth(MediumExtinction(float3(1.),float3(REG_FP32_MAX)),2.);
        values[id.x]=float4(tau,0.);return;
    }
    if(id.x==7u) {
        let coefficient=medium_exponential_coefficient(float3(0.,1.,2.),1000.);
        values[id.x]=float4(coefficient,medium_exponential_tau(1.,1000.,-1.,0.,true));return;
    }
    float slopes[7]={0.,1.0e-7,-1.0e-7,.7,-.7,10.,-10.};let slope=slopes[id.x];
    let log_density=id.x==0u ? -81. : -.3;
    let a=id.x==0u ? 1.0e35 : .4;let length=2.;
    let full=medium_exponential_tau(a,log_density,slope,length,false);
    let split=medium_exponential_tau(a,log_density,slope,.3,false)+medium_exponential_tau(a,log_density+slope*.3,slope,1.7,false);
    values[id.x]=float4(full,split,medium_exponential_tau(a,log_density,slope,0.,true),medium_exponential_tau(0.,log_density,slope,0.,true));
}"#;
    let bytes=9*16;
    let output=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:Some("exponential optical depth"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,source,&Native::default(),&entries,(1,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(9,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
    r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for (i,slope) in [0_f64,1.0e-7,-1.0e-7,0.7,-0.7,10.,-10.].into_iter().enumerate() {
        let sigma=if i==0 {1.0e35*(-81_f64).exp()} else {0.4*(-0.3_f64).exp()};
        let expected=sigma*if slope==0. {2.} else {(2.*slope).exp_m1()/slope};
        for &actual in &values[i][..2] {assert!((f64::from(actual)/expected-1.).abs()<1.0e-5,"case {i}: {:?}, expected {expected}",values[i]);}
        if slope>=0. {assert!(values[i][2].is_infinite());}
        else {assert!((f64::from(values[i][2])/(sigma/-slope)-1.).abs()<1.0e-5);}
        assert_eq!(values[i][3],0.);
    }
    assert!(values[8][..3].iter().all(|v|v.is_infinite() && *v>0.),"opaque optical depth must reconstruct without a nonfinite source");
    assert_eq!(values[8][3],0.);
    assert_eq!(values[7],[0.,f32::MAX,f32::MAX,f32::MAX],"exponential saturation must stay finite after rounded exp");
    Ok(())
}

#[test]
#[ignore = "requires pinned Slang and a GPU"]
fn certified_exponential_point_stays_below_majorant() -> Result<()> {
    let abi=programs::abi();
    let mut program=Compiler::discover().compile_medium(&abi,
        "struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i) {MediumCoefficients m;m.absorption=float3(.75*exp(-5.*c.position.x));m.scattering=float3(.25*exp(-5.*c.position.x));return m;}",
        Path::new("certified-exponential.slang"),"certifiedexponential")?;
    program.exponential=program.exponential_for_inputs(&abi,&[])?;
    assert!(program.exponential.is_some());
    let bound=program.extinction_majorant(&abi,&[],[[-1.,1.];3],[0.,1.])?;
    let mut native=Native {tracking:true,..Native::default()};
    let point=native_coefficients(&program,0,false,&mut native);
    let source=format!(r#"
import vork_medium_point;
import vork_prelude;
static Frame frame={{}};
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
export uint native_material_word(uint index) {{return 0u;}}
{point}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {{
    MediumTransportContext context={{}};
    let p=float3(-1.+float(id.x)/128.,0.,0.);
    let m=volume_0_medium_point(p,context);
    let authored={raw}_coefficients(p,0.,0u,p);
    values[id.x]=float4(m.absorption+m.scattering,(authored.absorption+authored.scattering).x);
}}
"#,raw=program.native_point);
    let r=Renderer::new()?;
    let bytes=257*16;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let pipeline=test_pipeline(&r,&source,&native,&[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})],(1,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(257,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |result| {let _=tx.send(result);});
    r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;
    rx.recv()??;
    let mapped=readback.get_mapped_range(..);
    let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for (i,value) in values.iter().enumerate() {
        let exact=(-5.*(-1.+i as f64/128.)).exp();
        for &sigma in &value[..3] {
            assert_eq!(sigma.to_bits(),value[3].to_bits(),"sampled point changed authored arithmetic");
            assert!(sigma.is_finite() && sigma<=bound,"{sigma} exceeds certified {bound}");
            assert!((f64::from(sigma)/exact-1.).abs()<1e-5,"{sigma} vs {exact}");
        }
    }
    r.errors.check()?;
    Ok(())
}

/// Sampling lighting cells must reproduce the complete source quadrature while
/// keeping extinction identical, including coloured and height-varying tails.
#[test]
fn exponential_free_flights_invert_integrated_majorants()->Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let mut cases=Vec::<[f32;4]>::new();
    for slope in [0.,1e-7,-1e-7,0.7,-0.7,10.,-10.] {
        for constant in [0.,0.3] {for log_density in [-20.,-0.3,20.] {for optical in [1e-5,0.4,20.] {for limit in [2.,f32::MAX] {
            cases.extend([[constant,0.4,log_density,slope],[optical,limit,0.,0.]]);
        }}}}
    }
    let source=r#"
import path_medium;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
[[vk::binding(1,0)]] StructuredBuffer<float4> cases;
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    let p=cases[id.x*2u];let q=cases[id.x*2u+1u];
    let distance=medium_exponential_candidate(p.x,p.y,p.z,p.w,q.x,q.y);
    values[id.x]=float4(distance,0.,0.,0.);
}"#;
    let bytes=(cases.len()/2*16) as u64;
    let output=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:Some("exponential free flights"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let input=r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:None,contents:bytemuck::cast_slice(&cases),usage:wgpu::BufferUsages::STORAGE});
    let readback=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false}),pipelines::buffer_entry(1,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:true})];
    let pipeline=test_pipeline(&r,source,&Native::default(),&entries,(1,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding()),entry(1,input.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups((cases.len()/2) as u32,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for (i,c) in cases.chunks_exact(2).enumerate() {
        let [constant,amplitude,log_density,slope]=c[0].map(f64::from);let optical=f64::from(c[1][0]);let limit=f64::from(c[1][1]);
        let tau=|distance:f64|constant*distance+amplitude*log_density.exp()*if slope==0. {distance}else{(slope*distance).exp_m1()/slope};
        let mut low=0.;let mut high=limit;
        if tau(limit)>optical {
            loop {let middle=low+(high-low)*0.5;if middle==low || middle==high {break;}if tau(middle)<optical {low=middle;}else{high=middle;}}
        }
        let actual=f64::from(values[i][0]);
        assert!((actual/high-1.).abs()<3e-4,"case {c:?}: {actual}, expected {high}");
    }
    r.errors.check()?;Ok(())
}

#[test]
#[ignore = "requires pinned Slang and a real GPU; Decision D program preparation"]
fn decision_d_candidate_point_coefficients_match_physical_oracle() -> Result<()> {
    let Some(r) = crate::test_renderer()? else { anyhow::bail!("Decision D requires a real GPU"); };
    for (name, source, height) in [
        ("constantcandidate", include_str!("../tests/fixtures/fog-constant.slang"), false),
        ("heightcandidate", include_str!("../tests/fixtures/fog-height.slang"), true),
    ] {
        let program = Compiler::discover().compile_medium(&programs::abi(), source, Path::new("candidate.slang"), name)?;
        let mut inputs = serde_json::json!({"absorption":[0.1,0.2,0.3],"scattering":[0.3,0.1,0.2],"emission":[0.2,0.4,0.8],"anisotropy":0.6});
        if height { inputs["base_height"]=serde_json::json!(2.); inputs["falloff"]=serde_json::json!(0.7); }
        let words = program.inputs.pack(&serde_json::from_value(inputs)?, |_,_| unreachable!())?;
        let literals = words.iter().map(|w| format!("{w}u")).collect::<Vec<_>>().join(",");
        let native = Native { modules: vec![("candidate_point".into(), program.native)], includes: program.native_includes, ..Native::default() };
        let shader = format!(r#"
import vork;
import candidate_point;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
export uint native_material_word(uint index) {{ uint words[{}] = {{{literals}}}; return words[index]; }}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {{
    // Deliberately different local and world positions: height is world-relative.
    let p={}_coefficients(float3(0.,-100.,0.),0.,0u,float3(0.,float(id.x),0.));
    values[3u*id.x]=float4(p.absorption,p.anisotropy);
    values[3u*id.x+1u]=float4(p.scattering,0.);
    values[3u*id.x+2u]=float4(p.emission,0.);
}}
"#, words.len(), program.native_point);
        let bytes=5*3*16;
        let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("Decision D point coefficients"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
        let pipeline=test_pipeline_defines(&r,&shader,&native,&entries,(1,1,1),&["-DVORK_NATIVE_MEDIUM"])?;
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(5,1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
        let submission=r.queue.submit([encoder.finish()]);let(tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move|v|{let _=tx.send(v);});
        r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
        let mapped=readback.get_mapped_range(..);let actual:&[[f32;4]]=bytemuck::cast_slice(&mapped);
        for y in 0..5 {
            let scale=if height {(-0.7_f64*(y as f64-2.)).exp()} else {1.};
            for (field,coefficients) in [[0.1,0.2,0.3],[0.3,0.1,0.2],[0.2,0.4,0.8]].iter().enumerate() {
                for c in 0..3 {
                    let expected=coefficients[c]*if field == 2 {1.} else {scale};
                    assert!((f64::from(actual[3*y+field][c])-expected).abs() <= 1e-6+1e-4*expected.abs(),"{name} y={y} field={field} channel={c}: {:?}, expected {expected}",actual[3*y+field]);
                }
            }
            assert!((actual[3*y][3]-0.6).abs()<1e-6);
        }
    }
    Ok(())
}

#[test]
#[ignore = "Decision D convex spike requires a real GPU and pinned Slang"]
fn decision_d_convex_slabs_match_beer_and_emission() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {anyhow::bail!("Decision D requires a real GPU");};
    for (density,emission,inside,copies) in [(0.5_f32,0.,false,1),(0.5,0.,true,1),(0.5,0.,false,2),(0.,2.,false,1),(0.000001,2.,false,1)] {
        let mut s:serde_json::Value=vork_scene::v2::with_test_settings(include_str!("../tests/fixtures/analytic.json"),include_str!("../tests/presets/analytic-7fefd52a.json"))?;
        s["animation"]=serde_json::json!([]);
        s["buffers"].as_object_mut().unwrap().retain(|name,_|name.starts_with("shape_box_"));
        s["geometry"].as_object_mut().unwrap().retain(|name,_|name=="shape_box");
        s["render"]=serde_json::json!({"resolution":[3,3],"background":"environment","settings":{"integrator":"path","denoiser":null,"samples":1}});
        s["materials"]=serde_json::json!({"fog":{"program":"std/fog","inputs":{"density":density,"scattering":[0,0,0],"absorption":[1,1,1],"emission":[emission,emission*0.5,emission*0.25]}}});
        let mut nodes=vec![serde_json::json!({"name":"camera","type":"camera","translation":[0,0,if inside {0.}else{2.}],"material":{"program":"std/camera","inputs":{"projection":"ortho","ortho_scale":0.1,"clip_start":0.0001}}}),serde_json::json!({"name":"env","type":"light","shape":"environment","color":[1,1,1],"energy":1})];
        for i in 0..copies {nodes.push(serde_json::json!({"name":format!("fog{i}"),"type":"mesh","geometry":"shape_box","materials":["fog"],"scale":[2,2,1]}));}
        s["nodes"]=serde_json::json!(nodes);
        let scene:vork_scene::v2::Scene=vork_scene::v2::from_test_value(s)?;
        let mut control=None;
        for work_limit in [None,Some(131072),Some(0)] {
            let installed=r.install_with_options(scene.clone(),Path::new("."),InstallOptions {
                test_convex_media:true,test_convex_work_limit:work_limit,
                test_volume_bounces:64,..Default::default()
            })?;
            let mut frame=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
            frame.submit_outputs(&installed,&r,0.,ReadbackOutputs {float:FloatOutputs {color:true,..Default::default()},..Default::default()})?;
            let collected=frame.collect(&r);
            if work_limit==Some(0) {
                let error=collected.err().context("exhausted medium frame published an image")?;
                ensure!(format!("{error:#}").contains("medium work budget exhausted"),"{error:#}");
                continue;
            }
            let output=collected?.linear_rgba.context("missing float output")?;
            if let Some(control)=&control {ensure!(control==&output,"guarded/control raw images differ");}
            else {control=Some(output);}
        }
        let output=control.context("missing control image")?;
        if density==0.5 && emission==0. && !inside && copies==1 {
            let mut invalid=scene.clone();
            invalid.programs.insert("invalidcoefficients".into(),serde_json::from_value(serde_json::json!({"source":"struct Inputs {}; MediumCoefficients medium(MediumContext c,Inputs i){MediumCoefficients m={};m.absorption=float3(-1.);return m;}"}))?);
            let material=invalid.materials.get_mut("fog").unwrap();
            material.program="invalidcoefficients".into();material.inputs.clear();
            let installed=r.install_convex_media_for_test(invalid,Path::new("."))?;
            let mut frame=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
            frame.submit_outputs(&installed,&r,0.,ReadbackOutputs {float:FloatOutputs {color:true,..Default::default()},..Default::default()})?;
            let error=frame.collect(&r).err().context("invalid medium coefficients published an image")?;
            ensure!(format!("{error:#}").contains("invalid physical coefficients"),"{error:#}");
        }
        let length=if inside {0.5_f64-0.0001}else{1.};
        let sigma=f64::from(density)*copies as f64;let t=(-sigma*length).exp();
        let integral=if sigma==0. {length}else{-(-sigma*length).exp_m1()/sigma};
        for pixel in output {for (c,channel) in [1_f64,0.5,0.25].into_iter().enumerate() {
            let expected=t+f64::from(emission)*channel*copies as f64*integral;
            assert!((f64::from(pixel[c])-expected).abs() <= 1e-6+1e-4*expected.abs(),"convex density {density} inside {inside} copies {copies}: {pixel:?}, expected {expected}");
        }}
    }
    Ok(())
}

/// Packet refusal gate: three installs, rather than recompiling the complete
/// coefficient matrix already covered by the five external physical fixtures.
#[test]
#[ignore = "Decision D packet refusal oracle requires a real GPU and pinned Slang"]
fn decision_d_packet_refusals() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {anyhow::bail!("Decision D requires a real GPU");};
    for (absorption,limit,expected_error) in [(0.5_f32,0,Some("medium work budget exhausted")),(-1.,131072,Some("invalid physical coefficients")),(0.5,131072,None)] {
        let mut s:serde_json::Value=vork_scene::v2::with_test_settings(include_str!("../tests/fixtures/analytic.json"),include_str!("../tests/presets/analytic-7fefd52a.json"))?;
        s["animation"]=serde_json::json!([]);
        s["buffers"].as_object_mut().unwrap().retain(|name,_|name.starts_with("shape_box_"));
        s["geometry"].as_object_mut().unwrap().retain(|name,_|name=="shape_box");
        s["render"]=serde_json::json!({"resolution":[3,3],"background":"environment","settings":{"integrator":"path","denoiser":null,"samples":1}});
        s["programs"]=serde_json::json!({"packetcoeff":{"source":format!("struct Inputs {{}}; MediumCoefficients medium(MediumContext c,Inputs i) {{MediumCoefficients m={{}};m.absorption=float3({absorption});return m;}}")}});
        s["materials"]=serde_json::json!({"fog":{"program":"packetcoeff"}});
        s["nodes"]=serde_json::json!([
            {"type":"camera","translation":[0,0,2],"material":{"program":"std/camera","inputs":{"projection":"ortho","ortho_scale":0.1,"clip_start":0.0001}}},
            {"type":"light","shape":"environment","color":[1,1,1],"energy":1},
            {"type":"mesh","geometry":"shape_box","materials":["fog"],"scale":[2,2,1]}
        ]);
        let installed=r.install_with_options(vork_scene::v2::from_test_value(s)?,Path::new("."),InstallOptions {
            test_convex_media:true,test_convex_work_limit:Some(limit),test_volume_bounces:64,..Default::default()
        })?;
        let mut frame=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
        frame.submit_outputs(&installed,&r,0.,ReadbackOutputs {float:FloatOutputs {color:true,..Default::default()},..Default::default()})?;
        let collected=frame.collect(&r);
        if let Some(expected)=expected_error {
            let error=collected.err().context("refused medium frame published an image")?;
            ensure!(format!("{error:#}").contains(expected),"{error:#}");
            let retry=frame.submit_outputs(&installed,&r,0.,ReadbackOutputs::default()).unwrap_err();
            ensure!(format!("{retry:#}").contains("reinstall"),"{retry:#}");
            let mut successor=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
            let retry=successor.submit_outputs(&installed,&r,0.,ReadbackOutputs::default()).unwrap_err();
            ensure!(format!("{retry:#}").contains("reinstall"),"{retry:#}");
        } else {
            // A valid render after both refusals also verifies device reuse.
            let pixels=collected?.linear_rgba.context("missing recovery image")?;
            for pixel in pixels {for channel in &pixel[..3] {ensure!((f64::from(*channel)-(-0.5_f64).exp()).abs()<1e-5,"{pixel:?}");}}
        }
    }
    Ok(())
}

/// External fixed-packet driver. Fixtures and evidence stay outside the tree;
/// this uses the ordinary frame slot, denoiser, readback and publication boundary.
#[test]
#[ignore = "Decision D packet: real GPU, pinned runtimes and VORK_FOG_PACKET_INPUT required"]
fn decision_d_convex_packet() -> Result<()> {
    let process_start=std::time::Instant::now();
    let input=PathBuf::from(std::env::var("VORK_FOG_PACKET_INPUT").context("set VORK_FOG_PACKET_INPUT")?);
    let output=PathBuf::from(std::env::var("VORK_FOG_PACKET_OUT").context("set VORK_FOG_PACKET_OUT")?);
    let packet:serde_json::Value=serde_json::from_slice(&std::fs::read(&input)?)?;
    let scene:vork_scene::v2::Scene=vork_scene::v2::from_test_value(packet["scene"].clone())?;
    let times:Vec<f32>=serde_json::from_value(packet["times"].clone())?;
    let repetitions=packet["repetitions"].as_u64().context("packet repetitions missing")?;
    ensure!(!times.is_empty() && repetitions>0,"empty packet");
    let guarded=std::env::var("VORK_FOG_GUARDED").as_deref()==Ok("1");
    let renderer_start=std::time::Instant::now();
    let Some(mut r)=crate::test_renderer()? else {anyhow::bail!("Decision D requires a real GPU");};
    let renderer_ms=renderer_start.elapsed().as_secs_f64()*1000.;
    let install_start=std::time::Instant::now();
    let roi:Option<[u32;4]>=packet.get("float_roi").map(|v|serde_json::from_value(v.clone())).transpose()?;
    if let Some([x,y,w,h])=roi {ensure!(w>0 && h>0 && u64::from(x)+u64::from(w)<=u64::from(scene.render.resolution[0]) && u64::from(y)+u64::from(h)<=u64::from(scene.render.resolution[1]),"invalid float ROI");}
    if packet["reference"].as_bool()==Some(true) {r.reference_transport_bounces=Some(64);}
    if packet["measure_segments"].as_bool()==Some(true) {r.test_gpu_segment_times=Some(Default::default());}
    let outputs=ReadbackOutputs {color:true,float:FloatOutputs {color:roi.is_some(),..Default::default()},..Default::default()};
    let installed=r.install_with_options(scene.clone(),input.parent().unwrap(),InstallOptions {
        test_convex_media:true,test_convex_work_limit:guarded.then_some(131072),
        test_medium_batch_rows:std::env::var("VORK_FOG_BATCH_ROWS").ok().map(|v|v.parse()).transpose()?,
        test_volume_bounces:64,..Default::default()
    })?;
    let install_ms=install_start.elapsed().as_secs_f64()*1000.;
    let slot_start=std::time::Instant::now();
    let mut frame=installed.create_frame_slot(&r,if roi.is_some() {ReadbackPrecision::Float}else{ReadbackPrecision::Packed})?;
    frame.profile=true;
    println!("FOG_SETUP {}",serde_json::json!({"renderer_ms":renderer_ms,"install_ms":install_ms,"slot_ms":slot_start.elapsed().as_secs_f64()*1000.,"before_warm_ms":process_start.elapsed().as_secs_f64()*1000.,"shader_creation":crate::timing::creations(&r.device)}));
    // Each newly installed variant gets an untimed first run.
    let warm_start=std::time::Instant::now();
    frame.submit_outputs(&installed,&r,times[0],outputs)?;
    if let Err(error)=frame.collect(&r) {
        println!("FOG_REFUSED_WALL_MS {}",warm_start.elapsed().as_secs_f64()*1000.);
        if let Some(segments)=&r.test_gpu_segment_times {
            std::fs::create_dir_all(&output)?;
            std::fs::write(output.join("segments.json"),serde_json::to_vec_pretty(&*segments.lock().unwrap())?)?;
        }
        return Err(error);
    }
    println!("FOG_WARM_WALL_MS {}",warm_start.elapsed().as_secs_f64()*1000.);
    std::fs::create_dir_all(&output)?;
    if let Some(segments)=&r.test_gpu_segment_times {
        std::fs::write(output.join("warm-segments.json"),serde_json::to_vec_pretty(&*segments.lock().unwrap())?)?;
        segments.lock().unwrap().clear();
    }
    let mut rows=Vec::new();
    for repetition in 0..repetitions {for (index,&time) in times.iter().enumerate() {
        let start=std::time::Instant::now();
        frame.submit_outputs(&installed,&r,time,outputs)?;
        let pixels=frame.collect(&r)?;
        let collected_ms=start.elapsed().as_secs_f64()*1000.;
        let name=format!("{repetition:02}-{index:03}.png");
        if let Some([x,y,w,h])=roi {
            let linear=pixels.linear_rgba.as_ref().context("missing float ROI output")?;
            let stride=scene.render.resolution[0] as usize;let mut crop=Vec::with_capacity((w*h) as usize);
            for row in y..y+h {crop.extend_from_slice(&linear[row as usize*stride+x as usize..row as usize*stride+(x+w) as usize]);}
            std::fs::write(output.join(format!("{repetition:02}-{index:03}.f32")),bytemuck::cast_slice(&crop))?;
        }
        let segments=r.test_gpu_segment_times.as_ref().map(|v|std::mem::take(&mut *v.lock().unwrap()));
        image::save_buffer(output.join(&name),&pixels.rgba,scene.render.resolution[0],scene.render.resolution[1],image::ColorType::Rgba8)?;
        let row=serde_json::json!({"repetition":repetition,"index":index,"time":time,"collected_ms":collected_ms,
            "timing_source":pixels.timing.timing_source,"gpu_ms":pixels.timing.gpu_ms,"gpu_elapsed_ms":pixels.timing.gpu_elapsed_ms,
            "gpu_spans_ms":pixels.timing.gpu_spans_ms,"sample_count":pixels.timing.sample_count,
            "gpu_segments":segments,"rgba_blake3":blake3::hash(&pixels.rgba).to_hex().as_str(),"image":name});
        println!("FOG_PACKET {}",serde_json::to_string(&row)?);rows.push(row);
    }}
    std::fs::write(output.join("rows.json"),serde_json::to_vec_pretty(&rows)?)?;
    Ok(())
}

/// Independent single-scatter integral: both camera and opposing sunlight travel
/// the same distance x through a unit slab, so integral_0^1 sigma_s*exp(-2*sigma_t*x) dx.
#[test]
#[ignore = "Decision D single-scatter oracle requires a real GPU and pinned Slang"]
fn decision_d_single_scatter_sun_slab() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {anyhow::bail!("Decision D requires a real GPU");};
    let mut s:serde_json::Value=vork_scene::v2::with_test_settings(include_str!("../tests/fixtures/analytic.json"),include_str!("../tests/presets/analytic-7fefd52a.json"))?;
    s["animation"]=serde_json::json!([]);
    s["buffers"].as_object_mut().unwrap().retain(|name,_|name.starts_with("shape_box_"));
    s["geometry"].as_object_mut().unwrap().retain(|name,_|name=="shape_box");
    s["render"]=serde_json::json!({"resolution":[64,64],"background":"environment","settings":{"integrator":"path","denoiser":null,"samples":16}});
    s["programs"]=serde_json::json!({"coefficients":{"source":include_str!("../tests/fixtures/fog-constant.slang")}});
    s["materials"]=serde_json::json!({"fog":{"program":"coefficients","inputs":{"absorption":[0,0,0],"scattering":[0.5,0.5,0.5]}},"black":{"inputs":{"base_weight":0,"specular_weight":0}}});
    s["nodes"]=serde_json::json!([
        {"name":"camera","type":"camera","translation":[0,0,2],"material":{"program":"std/camera","inputs":{"projection":"ortho","ortho_scale":0.1,"clip_start":0.0001}}},
        {"name":"world","type":"light","shape":"environment","energy":0},
        {"name":"sun","type":"light","shape":"sun","energy":4.*std::f64::consts::PI,"angle":0},
        {"name":"slab","type":"mesh","geometry":"shape_box","materials":["fog"],"scale":[2,2,1]},
        {"name":"back","type":"mesh","geometry":"shape_box","materials":["black"],"translation":[0,0,-2],"scale":[5,5,0.1]}
    ]);
    let scene:vork_scene::v2::Scene=vork_scene::v2::from_test_value(s)?;
    let mut control:Option<Vec<[f32;4]>>=None;
    for work_limit in [None,Some(131072)] {
        let installed=r.install_with_options(scene.clone(),Path::new("."),InstallOptions {
            test_convex_media:true,test_convex_work_limit:work_limit,
            test_volume_bounces:0,..Default::default()
        })?;
        let mut frame=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
        frame.submit_outputs(&installed,&r,0.,ReadbackOutputs {float:FloatOutputs {color:true,..Default::default()},..Default::default()})?;
        let pixels=frame.collect(&r)?.linear_rgba.context("missing oracle float output")?;
        if let Some(control)=&control {ensure!(&pixels==control,"guard changed raw single-scatter samples");}
        let n=pixels.len() as f64;
        let mean=pixels.iter().map(|p|f64::from(p[0])).sum::<f64>()/n;
        let variance=pixels.iter().map(|p|(f64::from(p[0])-mean).powi(2)).sum::<f64>()/(n-1.);
        let stderr=(variance/n).sqrt();let expected=0.5*(1.-(-1_f64).exp());
        println!("FOG_SINGLE_SCATTER guarded={} mean={mean:.9} expected={expected:.9} stderr={stderr:.9}",work_limit.is_some());
        ensure!((mean-expected).abs()<=0.01*expected,"single scatter exceeds 1%: {mean} vs {expected}");
        ensure!((mean-expected).abs()<=3.*stderr,"single scatter exceeds three standard errors: {mean} vs {expected}, stderr {stderr}");
        control=Some(pixels);
    }
    Ok(())
}

#[test]
#[ignore = "Decision D world closure oracle requires a real GPU and pinned Slang"]
fn decision_d_world_infinity_closure() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {anyhow::bail!("Decision D requires a real GPU");};
    for (absorption,emission,divergent) in [
        ([0_f32;3],[0_f32;3],false),
        ([0.5,1.,2.],[1.,0.5,0.25],false),
        ([0.,1.2e-38,0.5],[0.,0.,1.5],false),
        ([1e-30;3],[0.;3],false),
        ([1.,0.,1.],[0.,1.,0.],true),
    ] {
        let mut s:serde_json::Value=vork_scene::v2::with_test_settings(include_str!("../tests/fixtures/analytic.json"),include_str!("../tests/presets/analytic-7fefd52a.json"))?;
        s["animation"]=serde_json::json!([]);s["buffers"]=serde_json::json!({});s["geometry"]=serde_json::json!({});
        s["render"]=serde_json::json!({"resolution":[3,3],"background":"environment","settings":{"integrator":"path","denoiser":null,"samples":1}});
        s["programs"]=serde_json::json!({"coefficients":{"source":include_str!("../tests/fixtures/fog-constant.slang")}});
        s["materials"]=serde_json::json!({"fog":{"program":"coefficients","inputs":{"absorption":absorption,"scattering":[0,0,0],"emission":emission}}});
        s["nodes"]=serde_json::json!([
            {"name":"camera","type":"camera","translation":[0,0,2],"material":{"program":"std/camera","inputs":{"projection":"ortho","ortho_scale":0.1,"clip_start":0.0001}}},
            {"name":"world","type":"light","shape":"environment","energy":1,"color":[1,1,1],"medium":"fog"}
        ]);
        let scene:vork_scene::v2::Scene=vork_scene::v2::from_test_value(s)?;
        let mut control=None;
        for work_limit in [None,Some(131072)] {
            let installed=r.install_with_options(scene.clone(),Path::new("."),InstallOptions {
                test_convex_media:true,test_convex_work_limit:work_limit,
                test_volume_bounces:0,..Default::default()
            })?;
            let mut frame=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
            frame.submit_outputs(&installed,&r,0.,ReadbackOutputs {float:FloatOutputs {color:true,..Default::default()},..Default::default()})?;
            let collected=frame.collect(&r);
            if divergent {
                let error=collected.err().context("divergent world source published an image")?;
                ensure!(format!("{error:#}").contains("zero-extinction"),"{error:#}");continue;
            }
            let pixels=collected?.linear_rgba.context("missing world float output")?;
            if let Some(control)=&control {ensure!(&pixels==control,"guard changed raw world output");}
            for pixel in &pixels {for c in 0..3 {
                let expected=if absorption[c]>0. {f64::from(emission[c])/f64::from(absorption[c])}else{1.};
                ensure!((f64::from(pixel[c])-expected).abs()<=1e-6+1e-4*expected.abs(),"world absorption={absorption:?} emission={emission:?}, {pixel:?} vs channel {c}: {expected}");
            }}
            control=Some(pixels);
        }
    }
    Ok(())
}

#[test]
#[ignore = "Decision D height Beer oracle requires a real GPU and pinned Slang"]
fn decision_d_height_world_beer_closure() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {anyhow::bail!("Decision D requires a real GPU");};
    // Upward has finite optical depth; horizontal and downward tails are opaque
    // in each positive-extinction channel. Rotation keeps upward film points at
    // one world height, so no pixel-center or film-footprint approximation enters.
    let half=std::f64::consts::FRAC_1_SQRT_2;
    for (rotation,upward) in [([half,0.,0.,half],true),([-half,0.,0.,half],false),([0.,0.,0.,1.],false)] {
        let mut s:serde_json::Value=vork_scene::v2::with_test_settings(include_str!("../tests/fixtures/analytic.json"),include_str!("../tests/presets/analytic-7fefd52a.json"))?;
        s["animation"]=serde_json::json!([]);s["buffers"]=serde_json::json!({});s["geometry"]=serde_json::json!({});
        s["render"]=serde_json::json!({"resolution":[3,3],"background":"environment","settings":{"integrator":"path","denoiser":null,"samples":1}});
        s["programs"]=serde_json::json!({"coefficients":{"source":include_str!("../tests/fixtures/fog-height.slang")}});
        s["materials"]=serde_json::json!({"fog":{"program":"coefficients","inputs":{"absorption":[0,0.5,1],"scattering":[0,0,0],"emission":[0,0,0],"falloff":0.7,"base_height":0}}});
        s["nodes"]=serde_json::json!([
            {"name":"camera","type":"camera","rotation":rotation,"material":{"program":"std/camera","inputs":{"projection":"ortho","ortho_scale":0.1,"clip_start":0.0001}}},
            {"name":"world","type":"light","shape":"environment","energy":1,"color":[1,1,1],"medium":"fog"}
        ]);
        let scene:vork_scene::v2::Scene=vork_scene::v2::from_test_value(s)?;let mut control=None;
        for work_limit in [None,Some(131072),Some(0)] {
            let installed=r.install_with_options(scene.clone(),Path::new("."),InstallOptions {
                test_convex_media:true,test_convex_work_limit:work_limit,
                test_volume_bounces:0,..Default::default()
            })?;
            let mut frame=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
            frame.submit_outputs(&installed,&r,0.,ReadbackOutputs {float:FloatOutputs {color:true,..Default::default()},..Default::default()})?;
            let collected=frame.collect(&r);
            if work_limit==Some(0) {
                let error=collected.err().context("exhausted height frame published an image")?;
                ensure!(format!("{error:#}").contains("medium work budget exhausted"),"{error:#}");continue;
            }
            let pixels=collected?.linear_rgba.context("missing height float output")?;
            if let Some(control)=&control {ensure!(&pixels==control,"guard changed raw height output");}
            for pixel in &pixels {for (c,sigma) in [0_f64,0.5,1.].into_iter().enumerate() {
                let expected=if sigma==0. {1.}else if upward {(-sigma*(-0.7_f64*0.0001).exp()/0.7).exp()}else{0.};
                ensure!((f64::from(pixel[c])-expected).abs()<=1e-6+1e-4*expected.abs(),"height rotation={rotation:?}, {pixel:?} vs channel {c}: {expected}");
            }}
            control=Some(pixels);
        }
    }
    Ok(())
}

/// Rejected alpha candidates must spend the same invocation budget as accepted
/// geometry; transparent cutouts cannot bypass the safety bound.
#[test]
#[ignore = "requires real GPU and pinned Slang"]
fn fog_alpha_candidates_exhaust_shared_budget() -> Result<()> {
    let Some(mut r)=crate::test_renderer()? else {anyhow::bail!("real GPU required");};
    r.test_gpu_segment_times=Some(Default::default());
    let mut positions=Vec::new();
    for i in 0..4096 {
        let z=-(i as f32)*0.001;
        positions.extend([[-10.,-10.,z],[10.,-10.,z],[0.,10.,z]]);
    }
    let value=serde_json::json!({
        "version":2,
        "render":{"resolution":[1,1],"settings":{"integrator":"path","samples":1,"denoiser":null}},
        "buffers":{"positions":{"type":"vec3","data":positions}},
        "geometry":{"cutouts":{"attributes":{"POSITION":"positions"}}},
        "programs":{"constant-fog":{"source":include_str!("../tests/fixtures/fog-constant.slang")}},
        "materials":{
            "fog":{"program":"constant-fog","inputs":{"absorption":[0.01,0.01,0.01],"scattering":[0,0,0]}},
            "cutout":{"inputs":{"geometry_opacity":0.1,"alpha_cutoff":0.5}}
        },
        "nodes":[{"type":"camera","translation":[0,0,2]},
            {"type":"light","shape":"environment","medium":"fog"},
            {"type":"mesh","geometry":"cutouts","materials":["cutout"]}]
    });
    let scene:vork_scene::v2::Scene=vork_scene::v2::from_test_value(value)?;
    for (limit,operation) in [(96,8),(97,7),(131072,8)] {
        let installed=r.install_with_options(scene.clone(),Path::new("."),InstallOptions {
            test_convex_media:true,test_convex_work_limit:Some(limit),..Default::default()
        })?;
        let mut frame=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
        frame.submit_outputs(&installed,&r,0.,ReadbackOutputs::default())?;
        let error=frame.collect(&r).err().context("alpha candidates bypassed the medium work bound")?;
        let message=format!("{error:#}");
        println!("FOG_ALPHA_REFUSAL limit={limit} expected_operation={operation}: {message}");
        ensure!(message.contains("medium work budget exhausted") && message.contains(&format!("operation {operation}")),"{message}");
    }
    fog_admission_segment_witness(&r)?;
    Ok(())
}

#[test]
#[ignore = "requires real GPU and pinned Slang"]
fn fog_render_image_failure_invalidates_queued_publications() -> Result<()> {
    let Some(mut r)=crate::test_renderer()? else {anyhow::bail!("real GPU required");};
    r.test_gpu_segment_times=Some(Default::default());
    let mut value:serde_json::Value=vork_scene::v2::with_test_settings(include_str!("../tests/fixtures/analytic.json"),include_str!("../tests/presets/analytic-7fefd52a.json"))?;
    value["animation"]=serde_json::json!([]);
    value["buffers"].as_object_mut().unwrap().retain(|n,_|n.starts_with("shape_box_"));
    value["geometry"].as_object_mut().unwrap().retain(|n,_|n=="shape_box");
    let mut positions=Vec::new();
    for i in 0..4096 {
        let z=-(i as f32)*0.001;
        positions.extend([[-10.,-10.,z],[10.,-10.,z],[0.,10.,z]]);
    }
    value["buffers"]["cutout_positions"]=serde_json::json!({"type":"vec3","data":positions});
    value["geometry"]["cutouts"]=serde_json::json!({"attributes":{"POSITION":"cutout_positions"}});
    value["render"]=serde_json::json!({"camera":"main","root":"screen","resolution":[3,3],"settings":{"integrator":"path","samples":1,"denoiser":null}});
    value["images"]=serde_json::json!({"capture":{"render":{"camera":"capture_camera","root":"cutouts","resolution":[3,3]}}});
    value["materials"]=serde_json::json!({
        "fog":{"program":"constant-fog","inputs":{"scattering":[0,0,0],"absorption":[0.01,0.01,0.01]}},
        "cutout":{"inputs":{"geometry_opacity":0.1,"alpha_cutoff":0.5}},
        "monitor":{"inputs":{"base_color":{"image":"capture"}}}
    });
    value["programs"]=serde_json::json!({"constant-fog":{"source":include_str!("../tests/fixtures/fog-constant.slang")}});
    value["nodes"]=serde_json::json!([
        {"type":"camera","name":"main","translation":[0,0,3]},
        {"type":"camera","name":"capture_camera","translation":[0,0,2]},
        {"type":"light","shape":"environment","energy":1,"medium":"fog"},
        {"type":"mesh","name":"screen","geometry":"shape_box","materials":["monitor"],"scale":[2,2,0.1]},
        {"type":"mesh","name":"cutouts","geometry":"cutouts","materials":["cutout"]}
    ]);
    let installed=r.install_with_options(vork_scene::v2::from_test_value(value)?,Path::new("."),InstallOptions {
        test_convex_media:true,test_convex_work_limit:Some(4096),..Default::default()
    })?;
    let mut first=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
    let mut successor=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;
    first.submit_outputs(&installed,&r,0.,ReadbackOutputs::ALL)?;
    successor.submit_outputs(&installed,&r,1.,ReadbackOutputs::ALL)?;
    let error=successor.collect(&r).err().context("queued successor reused a failed capture")?;
    ensure!(format!("{error:#}").contains("medium work budget exhausted") || format!("{error:#}").contains("invalidated"),"{error:#}");
    let error=first.collect(&r).err().context("invalid capture published an image")?;
    ensure!(format!("{error:#}").contains("medium work budget exhausted"),"{error:#}");
    let error=successor.submit_outputs(&installed,&r,2.,ReadbackOutputs::ALL).unwrap_err();
    ensure!(format!("{error:#}").contains("reinstall"),"{error:#}");
    fog_admission_segment_witness(&r)?;
    Ok(())
}

fn fog_admission_segment_witness(r:&Renderer)->Result<()> {
    let segments=r.test_gpu_segment_times.as_ref().context("missing GPU witnesses")?.lock().unwrap();
    ensure!(!segments.is_empty(),"no measured GPU segments");
    println!("FOG_ADMISSION_SEGMENTS {}",serde_json::to_string(&*segments)?);
    for (_,_,ms,source) in segments.iter() {
        ensure!(*source=="gpu_timestamp" && *ms>0. && *ms<=100.,"unqualified GPU segment {ms} ms ({source})");
    }
    Ok(())
}
