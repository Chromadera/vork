//! Executes the compiled point program through the real kernel integrator.
use super::*;
use wgpu::util::DeviceExt;

/// Blender Principled Volume sockets plus an exponential height falloff, through
/// blender.volume: the coefficient conversion these transport oracles use.
const PRINCIPLED_FOG: &str = "import blender.volume;
struct Inputs {
    float3 color = float3(0.5); float density = 1.0; float anisotropy = 0.0; float3 absorption_color = float3(0.0);
    float emission_strength = 0.0; float3 emission_color = float3(1.0); float blackbody_intensity = 0.0;
    float3 blackbody_tint = float3(1.0); float temperature = 1000.0; float height_falloff = 0;
};
MediumCoefficients medium(MediumContext ctx, Inputs inputs) {
    MediumPoint point;
    point.color=inputs.color; point.density=inputs.density*exp(clamp(-inputs.height_falloff*ctx.position.y,-80.0,80.0));
    point.anisotropy=inputs.anisotropy; point.absorption_color=inputs.absorption_color;
    point.emission_strength=inputs.emission_strength; point.emission_color=inputs.emission_color;
    point.blackbody_intensity=inputs.blackbody_intensity; point.blackbody_tint=inputs.blackbody_tint;
    point.temperature=inputs.temperature;
    return principled_volume(point);
}";

#[test]
#[ignore = "requires pinned Slang and a GPU (lavapipe works); run with --ignored"]
fn principled_volume_coefficients_and_emission_transport() -> Result<()> {
    let program = Compiler::discover().compile_medium(
        &programs::abi(),
        PRINCIPLED_FOG,
        Path::new("principled-fog.slang"),
        "numeric",
    )?;
    let cases = [
        serde_json::json!({}),
        serde_json::json!({"density":0.3,"color":[0.2,0.4,0.6],"absorption_color":[0.25,0,1]}),
        serde_json::json!({"density":0,"emission_strength":2,"emission_color":[0.2,0.4,0.8]}),
        serde_json::json!({"density":0,"blackbody_intensity":1,"temperature":1000}),
        serde_json::json!({"density":0,"emission_strength":2,"emission_color":[0.2,0.4,0.8],"blackbody_intensity":1,"temperature":1000}),
        serde_json::json!({"density":0.4,"height_falloff":0.7,"emission_strength":0.2,"emission_color":[0.3,0.2,0.1]}),
        serde_json::json!({"anisotropy":1}),
        serde_json::json!({"anisotropy":-1}),
        serde_json::json!({}),
        serde_json::json!({}),
        serde_json::json!({"density":0.0001,"emission_strength":1000,"emission_color":[1,1,1]}),
    ];
    let mut words = Vec::new();
    for case in &cases {
        words.extend(program.inputs.pack(
            &serde_json::from_value(case.clone())?,
            |_, _| unreachable!(),
        )?);
    }
    let stride = words.len() / cases.len();
    let mut convergence = Vec::new();
    for samples in [8,32,128] {
    let mut native = Native {steps:samples,..Native::default()};
    let transport = native_transport(&program,0,&mut native);
    let point = &program.native_point;
    let source = format!(r#"
import vork;
import vork;
import vork_constants;
import vork_prelude;
import vork_medium;
static Frame frame={{}};
[[vk::binding(0,0)]] StructuredBuffer<uint> words;
[[vk::binding(1,0)]] RWStructuredBuffer<float4> values;
export float3 medium_visibility(float3 position,uint light_index) {{return select(position.z<1.,1.,0.);}}
export Frame medium_frame() {{return frame;}}
export uint medium_word(uint index) {{return words[index];}}
export uint native_material_word(uint index) {{return words[index];}}
// Light zero is deliberately absent from the receiver list.
export uint medium_light_list(float3 position,out uint base) {{base=0u;return min(uint(frame_light_count(frame)),1u);}}
export uint medium_light_index(uint base,uint next) {{return next+1u;}}
export Light medium_light(uint index) {{Light light={{}};light.direction=float4(0.,0.,-1.,0.);light.radiance=float4(select(index==0u,80.,4.)*REG_PI);light.shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]=REG_RAY_VOLUME;return light;}}
{transport}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {{
frame.scene.x=1.25;frame.medium.x=id.x*{stride}u;
frame.medium.w=1u;frame.radiance=float4(2.,3.,4.,0.);
if(id.x>=8u) {{frame.medium.w=0u;frame.background.w=2.;}}
let d=select(id.x==5u,float3(0.,.6,.8),float3(0.,0.,1.));
float4 p[3];float4 s[3];{point}(float3(0.),1.25,frame.medium.x,float3(0.),p);
MediumTransportContext fixed={{}};fixed.input_base=frame.medium.x;fixed.sample_fraction=.5;
let distance=id.x==10u ? .001 : 2.;
let saved=frame;frame.medium.w=1u;frame.background.w=0.;frame.radiance=float4(2.,3.,4.,0.);
let transport=volume_0_medium_transport(float3(0.),d,distance,fixed);
frame.radiance=0.;let emission=volume_0_medium_transport(float3(0.),d,distance,fixed);frame=saved;
s[0]=float4(transport.source-emission.source,0.);s[1]=float4(transport.transmission,0.);s[2]=float4(emission.source,0.);
for(uint i=0u;i<3u;i++) {{values[id.x*8u+i]=p[i];values[id.x*8u+3u+i]=s[i];}}
MediumTransportContext context={{}};context.input_base=frame.medium.x;context.sample_fraction=.5;
let m=volume_0_medium_transport(float3(0.),d,distance,context);
values[id.x*8u+6u]=float4(m.source,0.);values[id.x*8u+7u]=float4(m.transmission,medium_opacity(m.transmission,.25));
}}"#);
    let Some(renderer) = crate::test_renderer()? else {return Ok(());};
    let device = &renderer.device;
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("volume cases"),
        contents: bytemuck::cast_slice(&words),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (cases.len() * 8 * 16) as u64;
    let output = crate::gpu_memory::buffer(&device, &wgpu::BufferDescriptor {
        label: Some("volume results"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = crate::gpu_memory::buffer(&device, &wgpu::BufferDescriptor {
        label: Some("volume readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let entries: Vec<_> = (0..2).map(|binding| pipelines::buffer_entry(binding,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only: binding==0})).collect();
    let pipeline = test_pipeline(&renderer,&source,&native,&entries, (1, 1, 1))?;
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            entry(0, input.as_entire_binding()),
            entry(1, output.as_entire_binding()),
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(cases.len() as u32, 1, 1);
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
    let result: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
    let close = |actual: f32, expected: f32| {
        assert!(
            (actual - expected).abs() < 2e-5 + expected.abs() * 2e-5,
            "{actual} != {expected}"
        )
    };
    // Independent homogeneous Beer-Lambert oracles; includes RGB extinction.
    for (case, (scattering, absorption)) in [
        ([0.5f32; 3], [0.5f32; 3]),
        ([0.06, 0.12, 0.18], [0.12, 0.18, 0.]),
    ]
    .iter()
    .enumerate()
    {
        for c in 0..3 {
            close(result[case * 8][c], absorption[c]);
            close(result[case * 8 + 1][c], scattering[c]);
            let sigma = scattering[c] + absorption[c];
            let t = (-sigma * 2.).exp();
            close(result[case * 8 + 4][c], t);
            close(
                result[case * 8 + 3][c],
                (c as f32 + 2.) * scattering[c] / sigma * (1. - t),
            );
            close(result[case * 8 + 5][c], 0.);
        }
    }
    // Planck/CIE 1931 integration at 1000 K, evaluated independently in f64.
    let temperature_rgb = [4.312186391198153f32, 0.12609215480938682, 0.];
    let blackbody_scale = 5.670373e-8f32 * 1e-6 / std::f32::consts::PI * 1000f32.powi(4);
    for c in 0..3 {
        let emission = [0.4, 0.8, 1.6][c];
        let thermal = temperature_rgb[c] * blackbody_scale;
        for (case, expected) in [(2, emission), (3, thermal), (4, emission + thermal)] {
            close(result[case * 8 + 2][c], expected);
            close(result[case * 8 + 3][c], 0.);
            close(result[case * 8 + 4][c], 1.);
            close(result[case * 8 + 5][c], 2. * expected);
        }
    }
    // An almost transparent millimetre still emits one unit of radiance.
    // Use f64 exp_m1 to avoid reproducing the shader's cancellation.
    let expected=-(-0.0001_f64*0.001).exp_m1()/0.0001*1000.;
    for c in 0..3 {close(result[10*8+5][c],expected as f32);}
    close(result[6 * 8][3], 0.999);
    close(result[7 * 8][3], -0.999);
    // Independent fine midpoint integration for rising, height-varying haze.
    let steps = 10000;
    let step = 2f64 / steps as f64;
    let mut tau = 0f64;
    let mut scatter = [0f64; 3];
    let mut emission = [0f64; 3];
    for i in 0..steps {
        let t = (i as f64 + 0.5) * step;
        let sigma = 0.4f64 * (-0.7 * 0.6 * t).exp();
        let transmission = (-tau - sigma * step * 0.5).exp();
        for c in 0..3 {
            scatter[c] += transmission * sigma * 0.5 * (c as f64 + 2.) * step;
            emission[c] += transmission * 0.2 * [0.3, 0.2, 0.1][c] * step;
        }
        tau += sigma * step;
    }
    let mut error=0f32;
    for c in 0..3 {
        error=error.max((result[5*8+3][c]-scatter[c] as f32).abs());
        error=error.max((result[5*8+4][c]-(-tau).exp() as f32).abs());
        error=error.max((result[5*8+5][c]-emission[c] as f32).abs());
    }
    convergence.push(error);
    // Exercise the renderer's real median/compose helper and mean-T alpha.
    for case in 0..8 {
        for c in 0..3 {
            close(
                result[case * 8 + 6][c],
                result[case * 8 + 3][c] + result[case * 8 + 5][c],
            );
            close(result[case * 8 + 7][c], result[case * 8 + 4][c]);
        }
        let mean = result[case * 8 + 4][..3].iter().sum::<f32>() / 3.;
        close(result[case * 8 + 7][3], 1. - mean * 0.75);
    }
    // A shadow divides the world segment at one metre. A single
    // median probe would make this completely lit or completely dark.
    for case in 8..10 {
        for c in 0..3 {
            close(result[case*8+6][c], 0.5*(1.-(-1f32).exp()));
            close(result[case*8+7][c], (-2f32).exp());
        }
    }
    }
    assert!(convergence[1] < convergence[0]*0.2, "{convergence:?}");
    assert!(convergence[2] < convergence[1]*0.25, "{convergence:?}");
    assert!(convergence[2] < 2e-5, "{convergence:?}");
    Ok(())
}

#[test]
#[ignore = "requires pinned Slang and a GPU (lavapipe works); run with --ignored"]
fn implicit_medium_locals_receive_declared_defaults() -> Result<()> {
    let program = Compiler::discover().compile_medium(
        &programs::abi(),
        "import blender.volume; struct Inputs {}; MediumCoefficients medium(MediumContext ctx, Inputs inputs) { MediumPoint point; return principled_volume(point); }",
        Path::new("implicit-medium.slang"),
        "implicitdefaults",
    )?;
    let native = Native {
        modules: vec![("default_medium".into(), program.native)],
        includes: program.native_includes,
        ..Native::default()
    };
    let source = format!(r#"
import vork;
import default_medium;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
export uint native_material_word(uint index) {{ return 0u; }}
[shader("compute")][numthreads(1,1,1)] void numerical_test() {{
    let point={}_coefficients(float3(0.),0.,0u,float3(0.));
    values[0]=float4(point.absorption,point.anisotropy);
    values[1]=float4(point.scattering,0.);
    values[2]=float4(point.emission,0.);
}}
"#, program.native_point);
    let Some(renderer) = crate::test_renderer()? else {return Ok(());};
    let device=&renderer.device;
    let bytes=48;
    let output=device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("implicit medium defaults"), size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback=device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipeline=test_pipeline(&renderer,&source,&native,&[
        pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false}),
    ], (1, 1, 1))?;
    let bind=device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout:&pipeline.get_bind_group_layout(0),
        entries:&[entry(0,output.as_entire_binding())],
    });
    let mut encoder=device.create_command_encoder(&Default::default());
    {
        let mut pass=encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind, &[]);
        pass.dispatch_workgroups(1,1,1);
    }
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=renderer.queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |result| {let _=tx.send(result);});
    device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;
    rx.recv()??;
    let mapped=readback.get_mapped_range(..);
    let actual:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    // Principled Volume's declared density=1, colour=.5, no emission or anisotropy.
    assert_eq!(actual,&[[0.5,0.5,0.5,0.],[0.5,0.5,0.5,0.],[0.,0.,0.,0.]]);
    Ok(())
}

#[test]
fn froxel_storage_preserves_addresses_across_unaligned_pages() -> Result<()> {
    let Some(renderer) = crate::test_renderer()? else {
        return Ok(());
    };
    let native = Native {
        froxel_pages: 3,
        froxel_page_elements: 17,
        ..Default::default()
    };
    let mut source = native.paged_source(
        r#"
// FROXEL_STORAGE
[[vk::binding(0,0)]] RWStructuredBuffer<float4> result;
[shader("compute")][numthreads(1,1,1)] void numerical_test() {
    for(uint i=0u;i<49u;i++) {medium_froxel_write(i,float4(float(i),float(i+1u),float(i*3u),1.));}
    for(uint i=0u;i<49u;i++) {result[i]=medium_froxel_read(i);}
}
"#,
    );
    // Native passthrough uses wgpu-hal's dense Vulkan descriptor bindings.
    source = source.replace(
        &format!("binding({},{})", crate::resolve_native::FROXEL_PAGES, crate::resolve_native::PAGE_GROUP),
        "binding(1,0)",
    );
    let device = &renderer.device;
    let output = crate::gpu_memory::buffer(
        device,
        &wgpu::BufferDescriptor {
            label: Some("paged froxel values"),
            size: 49 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        },
    );
    let pages: Vec<_> = [17, 17, 15]
        .map(|elements| {
            crate::gpu_memory::buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some("froxel page"),
                    size: elements * 16,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                },
            )
        })
        .into();
    let entries: Vec<_> = (0..2)
        .map(|binding| {
            let mut entry = pipelines::buffer_entry(
                binding,
                wgpu::ShaderStages::COMPUTE,
                wgpu::BufferBindingType::Storage { read_only: false },
            );
            // The pages form one binding array.
            if binding == 1 { entry.count = std::num::NonZeroU32::new(3); }
            entry
        })
        .collect();
    let pipeline = test_pipeline(&renderer, &source, &native, &entries, (1, 1, 1))?;
    let page_bindings: Vec<_> = pages
        .iter()
        .map(|page| wgpu::BufferBinding { buffer: page, offset: 0, size: None })
        .collect();
    let entries = vec![
        entry(0, output.as_entire_binding()),
        entry(1, wgpu::BindingResource::BufferArray(&page_bindings)),
    ];
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    });
    let readback = crate::gpu_memory::buffer(
        device,
        &wgpu::BufferDescriptor {
            label: Some("paged froxel readback"),
            size: 49 * 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        },
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 49 * 16);
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
    let values: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
    for (i, &actual) in values.iter().enumerate() {
        assert_eq!(actual, [i as f32, (i + 1) as f32, (i * 3) as f32, 1.]);
    }
    renderer.errors.check()?;
    Ok(())
}

/// Exercise production distance/phase sampling against independent slab energy
/// and HG angular moments, including transparent spectral channels and deltas.
#[test]
#[ignore = "requires pinned Slang and Vulkan (lavapipe works)"]
fn path_medium_free_flight_and_phase() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import path_medium;
import vork_medium_visibility;
import vork_medium_point;
import vork_constants;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    if(id.x==0u) {
        var sigma=16777216.;var bound=sigma;var reverse=0.;
        for(var j=0u;j<6u;j++) {sigma+=1.25;bound=medium_majorant_add(bound,1.25);reverse+=1.25;}
        reverse+=16777216.;
        values[27]=float4(sigma,bound,reverse,medium_majorant_add(0.,0.));
        values[28]=float4(medium_sample_majorant(float3(asfloat(1u)),float3(0.)),medium_phase_weight(float3(0.,0.,1.),float3(0.,0.,1.),.999),medium_phase_weight(float3(0.,0.,-1.),float3(0.,0.,1.),-.999),medium_phase_weight(float3(0.,0.,1.),float3(0.,0.,1.),1.));
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

/// Compare local-cell reconstruction to a double-precision serial reference.
/// Nonuniform coloured cells exercise ordered depth composition, metric changes
/// and optically opaque coefficient storage.
#[test]
#[ignore = "requires pinned Slang and Vulkan (lavapipe works)"]
fn froxel_local_cells_reintegrate_receiver_metric_in_order() -> Result<()> {
    let Some(r) = crate::test_renderer()? else { return Ok(()); };
    for slices in [1u32, 7, 9, 64, 96] {
        let columns = 35u32;
        let rows = 2u32;
        let cells = (slices+2)*columns*rows;
        let mut values = vec![[0f32; 4]; (cells * 2) as usize];
        for z in 1..=slices { for xy in 0..columns*rows {
            let at = (z * columns * rows + xy) as usize;
            values[at] = [0.01 * (z % 7 + 1) as f32, 0.02 * (xy % 5 + 1) as f32, 0.03, 0.];
            values[at+cells as usize] = [0.99, 0.8 + 0.01 * (z % 9) as f32, 1., 0.];
        }}
        let source = format!(r#"
import vork_froxel;
import vork_medium_point;
import vork_constants;
import vork_medium_types;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
[shader("compute")][numthreads(8,1,1)] void numerical_test(uint3 gid:SV_DispatchThreadID) {{
    let xy=gid.x;if(xy>={columns}u*{rows}u) return;
    var prefix=MediumTransport(float3(0.),float3(1.),float3(0.));
    values[xy]=float4(0.);values[xy+{cells}u]=float4(1.,1.,1.,0.);
    for(uint z=1u;z<={slices}u;z++) {{
        let at=z*{columns}u*{rows}u+xy;
        let local=froxel_estimate(MediumTransport(values[at].rgb,values[at+{cells}u].rgb,-log(values[at+{cells}u].rgb)),1.);
        let segment=froxel_integrate(local,1.7);
        prefix.source+=prefix.transmission*segment.source;
        prefix.transmission*=segment.transmission;
        values[at]=float4(prefix.source,0.);values[at+{cells}u]=float4(prefix.transmission,0.);
    }}
    if(xy==0u) {{
        let sigma=MediumExtinction(float3(1.),float3(1000.));
        let tau=medium_optical_depth(sigma,1.);let transmission=exp(-tau);
        let dense=froxel_estimate(MediumTransport(medium_integrated_coefficient(float3(2000.),sigma,1.,transmission),transmission,tau),1.);
        values[({slices}u+1u)*{columns}u*{rows}u]=float4(dense.extinction.x,froxel_integrate(dense,.7).source.x,0.,0.);
    }}
    if(xy>=1u && xy<=3u) {{
        let sigma=xy==3u ? asfloat(0x7f800000u) : 1.e37;
        let light=xy==2u ? 0. : 42.;
        let local=froxel_estimate(MediumTransport(float3(light),float3(0.),float3(sigma)),1.);
        let received=froxel_integrate(local,.7);
        values[({slices}u+1u)*{columns}u*{rows}u+xy]=float4(received.source.x,received.transmission.x,0.,0.);
    }}
    if(xy==4u || xy==5u) {{
        var mix=(FroxelBlend)0;
        let high=xy==4u;
        let sigma=high ? 1.e37 : .2;
        let t=exp(-sigma);
        let a=froxel_estimate(MediumTransport(float3(2.),float3(1.),float3(0.)),1.);
        let b=froxel_estimate(MediumTransport(float3((1.-t)*(high ? 7. : 4.)),float3(t),float3(sigma)),1.);
        froxel_blend(mix,a,.5);froxel_blend(mix,b,.5);
        let received=froxel_integrate(froxel_blended(mix),2.);
        values[({slices}u+1u)*{columns}u*{rows}u+xy]=float4(received.source.x,received.transmission.x,0.,0.);
    }}
    if(xy==7u) {{
        let received=medium_received_source(float3(0.,0.,.5),float3(asfloat(0x7fc00000u),asfloat(0x7f800000u),8.));
        values[({slices}u+1u)*{columns}u*{rows}u+xy]=float4(received,0.);
    }}
    if(xy==6u) {{
        let weight=medium_integrated_coefficient(float3(1.e37),float3(1.e37),1.,float3(0.));
        values[({slices}u+1u)*{columns}u*{rows}u+xy]=float4(weight*42.,0.);
    }}
}}
"#);
        let bytes = (values.len() * 16) as u64;
        let output = r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("nonuniform cell transports"), contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
        });
        let entries = [pipelines::buffer_entry(0, wgpu::ShaderStages::COMPUTE, wgpu::BufferBindingType::Storage {read_only:false})];
        let pipeline = test_pipeline(&r, &source, &Native {slices,tile:1,..Default::default()}, &entries, (8,1,1))?;
        let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[entry(0,output.as_entire_binding())],
        });
        let mut encoder = r.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline); pass.set_bind_group(0,&bind,&[]);
            pass.dispatch_workgroups((columns*rows).div_ceil(8),1,1);
        }
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
        let submission = r.queue.submit([encoder.finish()]);
        let (tx,rx) = std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |result| {let _=tx.send(result);});
        r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;
        rx.recv()??;
        let mapped = readback.get_mapped_range(..);
        let actual: &[[f32;4]] = bytemuck::cast_slice(&mapped);
        for xy in 0..columns*rows {
            let mut source = [0f64;3]; let mut transmission = [1f64;3];
            for z in 0..=slices {
                let at = (z * columns * rows + xy) as usize;
                for c in 0..3 {
                    if z>0 {
                        let t=f64::from(values[at+cells as usize][c]);
                        let new_t=t.powf(1.7);
                        let factor=if t==1. {1.7}else{(1.-new_t)/(1.-t)};
                        source[c]+=transmission[c]*f64::from(values[at][c])*factor;
                        transmission[c]*=new_t;
                    }
                    assert!((f64::from(actual[at][c])-source[c]).abs()<2e-5,"source slices={slices} xy={xy} z={z}");
                    assert!((f64::from(actual[at+cells as usize][c])-transmission[c]).abs()<2e-5,"transmission slices={slices} xy={xy} z={z}");
                }
            }
        }
        let dense=actual[((slices+1)*columns*rows) as usize];
        assert!((dense[0]-1000.).abs()<0.001 && (dense[1]-2.).abs()<0.001,"opaque local cell: {dense:?}");
        let extra=((slices+1)*columns*rows) as usize;
        for (offset,expected) in [(1,42.),(2,0.),(3,42.),(4,7.),(6,42.)] {
            let value=actual[extra+offset][0];assert!(value.is_finite() && (value-expected).abs()<0.001,"dense case {offset}: {value}");
        }
        assert_eq!(&actual[extra+7][..3],&[0.,0.,4.],"opaque source composition");
        let expected=1.4_f64*(-(-0.2_f64).exp_m1())/0.1;
        assert!((f64::from(actual[extra+5][0])-expected).abs()<0.0001,"mixed vacuum/source density: {:?}",actual[extra+5]);
        r.errors.check()?;
    }
    Ok(())
}

#[test]
fn exponential_optical_depth_matches_f64_in_all_ray_limits() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import vork_constants;
import vork_medium_point;
import vork_froxel;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    if(id.x==8u) {
        let tau=medium_optical_depth(MediumExtinction(float3(1.),float3(REG_FP32_MAX)),2.);
        let m=MediumTransport(float3(0.),exp(-tau),tau);
        let reconstructed=froxel_integrate(froxel_estimate(m,2.),2.);
        values[id.x]=float4(m.optical_depth,reconstructed.source.x);return;
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
fn exponential_interval_adapter_preserves_local_metric() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let scene:v2::Scene=serde_json::from_value(serde_json::json!({"version":2,
        "render":{"resolution":[8,8]},"nodes":[],"materials":{"fog":{"program":"std/fog",
        "inputs":{"absorption":[0,1,2],"scattering":[0,0,0],"height_falloff":4}}}}))?;
    let (program,inputs)=compile(&r,&scene,"fog",Path::new("."))?;
    assert_eq!(program.exponential,Some(([0.,1.,2.],[0.;3],[0.,-4.,0.])));
    let mut words=vec![0,0,0,4];words.extend(inputs);
    let mut native=Native::default();let code=native_coefficients(&program,0,true,&mut native);
    let source=format!(r#"
import vork_prelude;
import vork_medium_point;
import vork_medium_domain;
import vork_medium_bounds;
static Frame frame={{}};
[[vk::binding(0,0)]] StructuredBuffer<uint> words;
[[vk::binding(2,0)]] RWStructuredBuffer<float4> values;
export uint native_material_word(uint index) {{return words[index];}}
export uint medium_word(uint index) {{return words[index];}}
export Frame medium_frame() {{return frame;}}
{code}
export MediumSample volume_coefficients(uint id,float3 p,MediumTransportContext c) {{return volume_0_medium_point(p,c);}}
export MediumSample volume_interval_coefficients(uint id,float3 o,float3 d,float distance,float fraction,MediumTransportContext c) {{return volume_0_interval_point(o,d,distance,fraction,c);}}
export bool volume_homogeneous(uint id) {{return false;}}
export Light medium_light(uint i) {{return Light();}}
export uint medium_light_list(float3 p,out uint base) {{base=0u;return 0u;}}
export uint medium_light_index(uint base,uint next) {{return next;}}
export float3 medium_visibility(float3 p,uint i) {{return float3(1.);}}
export uint2 medium_size() {{return uint2(1);}}
export MediumSample medium_world_coefficients(float3 p) {{return MediumSample(float3(0.),float3(0.),float3(0.),0.);}}
export float medium_world_tail_start(float3 o,float3 d) {{return 0.;}}
export MediumSample medium_world_tail_coefficients(float3 o,float3 d) {{return MediumSample(float3(0.),float3(0.),float3(0.),0.);}}
export void medium_world_exponential(out float3 a,out float3 b,out float3 g) {{a=0.;b=0.;g=0.;}}
[shader("compute")][numthreads(1,1,1)] void numerical_test() {{
    frame.medium.w=1u;
    MediumTransportContext context={{}};context.input_base=4u;
    context.inverse=float4x4(float4(1.,0.,0.,0.),float4(0.,1.,0.,0.),float4(0.,0.,1.,0.),float4(0.,0.,0.,1.));
    let m=volume_0_interval_point(float3(0.),float3(0.,-.6,-.8),.3125,.5,context);
    values[0]=float4(m.absorption,0.);
    values[1]=float4(exp(-medium_optical_depth(MediumExtinction(float3(1.),m.absorption+m.scattering),.3125)),0.);
    let endpoint=medium_receiver(.3125);values[2]=float4(float(endpoint.kind),endpoint.distance,0.,0.);
    let instance=volume_instance(0u,float3(0.),float3(0.,-.6,-.8),.1,.3125,0xffffffffu);
    for(uint row=0u;row<4u;row++) {{values[3u+row]=instance.context.inverse[row];}}
    values[7]=float4(volume_0_interval_point(float3(0.),float3(0.,-.6,-.8),.3125,.5,instance.context).absorption,0.);
    values[8]=float4(medium_transport_segment(float3(0.),float3(0.,-.6,-.8),medium_receiver(.3125),.5,0xffffffffu).transmission,0.);
}}"#);
    let input=r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:None,contents:bytemuck::cast_slice(&words),usage:wgpu::BufferUsages::STORAGE});
    let objects=r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:None,contents:bytemuck::cast_slice(&[0u32;128]),usage:wgpu::BufferUsages::STORAGE});
    let bytes=9*16;
    let output=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=crate::gpu_memory::buffer(&r.device,&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:true}),pipelines::buffer_entry(1,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:true}),pipelines::buffer_entry(2,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,&source,&native,&entries,(1,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,input.as_entire_binding()),entry(1,objects.as_entire_binding()),entry(2,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
    r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    let depth=(0.75_f64).exp_m1()/2.4;
    for c in 0..3 {assert!((f64::from(values[0][c])-c as f64*depth/0.3125).abs()<1e-5,"{values:?}");assert!((f64::from(values[1][c])-(-(c as f64)*depth).exp()).abs()<1e-5,"{values:?}");}
    assert_eq!(values[2],[0.,0.3125,0.,0.]);
    for row in 0..4 {assert_eq!(values[3+row],std::array::from_fn(|col|if row==col {1.} else {0.}),"{values:?}");}
    for c in 0..3 {assert!((f64::from(values[7][c])-c as f64*depth/0.3125).abs()<1e-5,"{values:?}");assert!((f64::from(values[8][c])-(-(c as f64)*depth).exp()).abs()<1e-5,"{values:?}");}
    Ok(())
}

/// A hard shaft edge has an analytic visible fraction of .37. Test the real
/// visibility footprint, including blocked taps and coloured transmission, with
/// no camera reconstruction or display transform hiding normalization errors.
#[test]
#[ignore = "requires pinned Slang and Vulkan (lavapipe works)"]
fn shaft_visibility_footprint_normalizes_and_reduces_variance() -> Result<()> {
    let Some(r) = crate::test_renderer()? else { return Ok(()); };
    let source = r#"
import vork_medium;
import vork_prelude;
import vork_constants;
import vork_light_falloff;
export Light medium_light(uint index) {
    Light light={};light.direction=float4(0.,0.,-1.,0.);light.radiance=float4(1.);
    if(index>=4u) {light.position.w=2.;light.direction=float4(0.,0.,1.,.95);light.radiance.w=.98;}
    return light;
}
// Isotropic directional source has weight 1/(4*pi); report visibility units.
float3 medium_visibility_footprint(float3 p,float3 footprint,float fraction,uint light,
    float3 x=float3(0.),float3 y=float3(0.),uint2 pixel=uint2(0u),uint index=0u,uint count=1u) {
    return medium_source_footprint(p,footprint,fraction,light,x,y,pixel,index,count)*(4.*REG_PI);
}
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
export float3 medium_visibility(float3 p,uint light) {
    if(light==1u || light==5u) return float3(.2,.4,.8);
    if(light==4u) return abs(p.x)<.4 ? float3(0.) : float3(1.);
    if(light==6u) return float3(1.);
    if(light==2u) return frac(p.x)<.37 ? float3(1.) : float3(0.);
    if(light==3u) return p.z>.37 ? float3(1.) : float3(0.);
    return p.z<.37 ? float3(1.) : float3(0.);
}
[shader("compute")][numthreads(8,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    if(id.x>=128u) return;
    let rotation=(float(id.x)+.5)/128.;
    const uint budgets[10]={1u,2u,3u,4u,7u,8u,16u,31u,64u,86u};
    for(uint budget=0u;budget<10u;budget++) {
        let samples=budgets[budget];
        var filtered=float3(0.);var point=0.;var lateral=0.;
        for(uint sample=0u;sample<samples;sample++) {
            let fraction=frac((float(sample)+.5)/float(samples)+rotation);
            let p=float3(0.,0.,fraction);
            filtered+=medium_visibility_footprint(p,float3(0.,0.,1.),fraction,0u,float3(0.),float3(0.),uint2(0u),sample,samples)/float(samples);
            point+=medium_visibility(p,0u).x/float(samples);
            lateral+=medium_visibility_footprint(float3(.5+rotation,0.,.5),float3(0.,0.,1.),.5,2u,
                float3(1.,0.,0.),float3(0.,1.,0.),uint2(0u),sample,samples).x/float(samples);
        }
        values[budget*128u+id.x]=float4(filtered,point);
        let tint=medium_visibility_footprint(float3(0.,0.,.23),float3(0.,0.,1.),.23,1u,
            float3(0.),float3(0.),uint2(0u),0u,samples);
        values[1408u+budget*128u+id.x]=float4(lateral,tint);
    }
    let tint=medium_visibility_footprint(float3(0.,0.,.23),float3(0.,0.,1.),.23,1u);
    let clipped=medium_visibility_footprint(float3(0.,0.,.075),float3(0.,0.,.3),.25,3u);
    values[1280u+id.x]=float4(tint,clipped.x);
    let p=float3(rotation,0.,rotation);
    let high=medium_visibility_footprint(p,float3(0.,0.,1.),.5,0u,float3(1.,0.,0.),float3(0.,1.,0.),uint2(0u),0u,256u);
    values[2688u+id.x]=float4(high-medium_visibility(p,0u),float(medium_visibility_taps(256u)));
    let cone=medium_visibility_footprint(float3(0.,0.,1.),float3(2.,0.,0.),.5,4u);
    let missed=medium_visibility_footprint(float3(0.,0.,1.),float3(4.,0.,0.),.5,4u);
    var expected=float3(0.);
    let light=medium_light(5u);
    for(uint tap=0u;tap<4u;tap++) {
        let p=float3((float(tap)+.4)/4.-.4,0.,1.);
        expected+=light_falloff(p,light.position,light.direction,light.radiance.w,0.,false).punctual*float3(.2,.4,.8)/4.;
    }
    let tint_delta=medium_visibility_footprint(float3(0.,0.,1.),float3(1.,0.,0.),.4,5u)-expected;
    // Centre x=.5 is outside this cone; the first footprint stratum is inside.
    let narrow=medium_visibility_footprint(float3(.5,0.,1.),float3(1.,0.,0.),.5,6u).x;
    let expected_narrow=light_falloff(float3(.125,0.,1.),light.position,light.direction,light.radiance.w,0.,false).punctual/4.;
    values[2816u+id.x]=float4(cone.x,missed.x,max(abs(tint_delta.x),max(abs(tint_delta.y),abs(tint_delta.z))),narrow-expected_narrow);
}
"#;
    let bytes = 2944*16;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("shaft estimates"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,source,&Native::default(),&entries,(8,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(16,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |result| {let _=tx.send(result);});
    r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;
    rx.recv()??;
    let mapped=readback.get_mapped_range(..);
    let actual:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for (budget,samples) in [1u32,2,3,4,7,8,16,31,64,86].into_iter().enumerate() {
        let rows=&actual[budget*128..(budget+1)*128];
        let mse=|c:usize| rows.iter().map(|p|(f64::from(p[c])-0.37).powi(2)).sum::<f64>()/128.;
        let mean=rows.iter().map(|p|f64::from(p[0])).sum::<f64>()/128.;
        assert!((mean-0.37).abs()<0.003,"normalization at {} samples: {mean}",samples);
        if samples<=16 {assert!(mse(0)<mse(3)*0.25,"shaft variance at {} samples: {} vs {}",samples,mse(0),mse(3));}
        println!("shaft visibility samples={} filtered_rmse={} point_rmse={}",samples,mse(0).sqrt(),mse(3).sqrt());
    }
    for row in &actual[1280..1408] {
        for (value,expected) in row.iter().zip([0.2,0.4,0.8,0.]) {assert!((value-expected).abs()<1e-6,"tint/receiver normalization: {row:?}");}
    }
    for (budget,samples) in [1u32,2,3,4,7,8,16,31,64,86].into_iter().enumerate() {
        let rows=&actual[1408+budget*128..1408+(budget+1)*128];
        let mean=rows.iter().map(|p|f64::from(p[0])).sum::<f64>()/128.;
        assert!((mean-0.37).abs()<0.01,"lateral visibility mean: {mean}");
        let taps=16u32.min(samples.max(4)).min(1+255/samples);
        for row in rows {
            for (value,expected) in row[1..].iter().zip([0.2,0.4,0.8]) {assert!((value-expected).abs()<1e-6,"coloured normalization at {samples}: {row:?}");}
            assert!((row[0]-0.37).abs()<=1./(taps*samples) as f32+1e-5,
            "lateral strata at {} samples: {}",samples,row[0]);}
    }
    for row in &actual[2688..2816] {assert_eq!(*row,[0.,0.,0.,1.],"high-budget point identity");}
    for row in &actual[2816..] {for (value,expected) in row.iter().zip([0.,0.,0.,0.]) {assert!((value-expected).abs()<1e-6,"source support/footprint normalization: {row:?}");}}
    r.errors.check()?;
    Ok(())
}

/// The grid must not illuminate cells or infinite tails that no receiver can
/// consume. The support reduction includes the image edge and background mass.
#[test]
#[ignore = "requires pinned Slang and Vulkan (lavapipe works)"]
fn froxel_receiver_support_skips_unused_lighting_and_tails() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import vork_froxel;
import vork_medium_types;
import vork_prelude;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
[[vk::binding(1,0)]] RWStructuredBuffer<uint> counters;
export uint2 medium_size() {return uint2(8u,8u);}
export Frame medium_frame() {
    Frame f={};f.view=float4x4(1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.);
    f.inverse_vp=f.view;f.projection=float4(1.,1.,0.,0.);
    f.params.xy=float2(.01,1.);f.flags.x=1.;f.temporal.xy=float2(0.,4.);return f;
}
export float4 medium_froxel_read(uint index) {return values[index];}
export void medium_froxel_write(uint index,float4 value) {values[index]=value;}
export float2 medium_receiver_support(uint2 pixel) {
    let background=counters[2]==2u || (counters[2]==1u && all(pixel==uint2(7u)));
    return background ? float2(0.,1.) : float2(.6,0.);
}
export MediumTransport medium_transport_segment(float3 o,float3 d,MediumEndpoint endpoint,float fraction,uint inside) {
    InterlockedAdd(counters[0],1u);
    return MediumTransport(float3(endpoint.distance),float3(1.),float3(0.));
}
export MediumTransport medium_transport_unbounded(float3 o,float3 d,float scale,float fraction) {
    InterlockedAdd(counters[1],1u);
    return MediumTransport(float3(7.),float3(.25),float3(0.));
}
[shader("compute")][numthreads(8,8,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {BODY}
"#;
    let entries=[
        pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false}),
        pipelines::buffer_entry(1,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false}),
    ];
    let native=Native {tile:8,slices:4,distribution:0.,steps:1,..Default::default()};
    let support=test_pipeline(&r,&source.replace("BODY","froxel_receiver_support(id);"),&native,&entries,(8,8,1))?;
    let lighting=test_pipeline(&r,&source.replace("BODY","froxel_light(id);"),&native,&entries,(8,8,1))?;
    for mode in 0u32..3 {
        let background=mode!=0;
        let values=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("receiver support cells"),size:256,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let counters=r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("lighting queries"),contents:bytemuck::cast_slice(&[0u32,0,mode,0]),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC});
        let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:272,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        // Match production: ordered support, lighting and reconstruction dispatches.
        {let mut pass=encoder.begin_compute_pass(&Default::default());
        for (pipeline,groups) in [(&support,1),(&lighting,5)] {
            let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,values.as_entire_binding()),entry(1,counters.as_entire_binding())]});
            pass.set_pipeline(pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,groups);
        }}
        encoder.copy_buffer_to_buffer(&values,0,&readback,0,256);
        encoder.copy_buffer_to_buffer(&counters,0,&readback,256,16);
        let submission=r.queue.submit([encoder.finish()]);
        let (tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |result| {let _=tx.send(result);});
        r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;
        rx.recv()??;
        let mapped=readback.get_mapped_range(..);
        let actual:&[[f32;4]]=bytemuck::cast_slice(&mapped[..256]);
        let counts:&[u32]=bytemuck::cast_slice(&mapped[256..]);
        assert_eq!(&counts[..2],if background {&[4,1]}else{&[2,0]},"unused lighting, background={background}");
        assert_eq!(actual[0][..2],[if mode==2 {0.}else{0.6},if background {1.}else{0.}]);
        for z in 1..=4 {
            let expected=if background || z<=2 {1.}else{0.};
            assert!((actual[z][0]-expected).abs()<1e-6,"cell {z}, background={background}: {:?}",actual[z]);
        }
        assert_eq!(actual[5][0],if background {7.}else{0.});
        assert_eq!(actual[11][0],if background {0.25}else{1.});
        r.errors.check()?;
    }
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
#[ignore = "requires pinned Slang and a real GPU"]
fn sampled_tail_lighting_preserves_mean_and_extinction() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    for jittered in [true] {
    let source=r#"
import vork_medium;
import vork_prelude;
import vork_constants;
static uint calls=0u;
static uint variant=0u;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
export Frame medium_frame() {Frame f={};f.temporal.y=64.;return f;}
export uint2 medium_size() {return uint2(1u);}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 1u;}
export uint medium_light_index(uint base,uint next) {return next;}
export Light medium_light(uint i) {Light l={};l.direction=float4(0.,0.,-1.,0.);l.radiance=float4(4.*REG_PI);l.shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]=REG_RAY_VOLUME;return l;}
export float3 medium_visibility(float3 p,uint i) {calls++;return float3(.2,.5,.8)*(0.5+0.5*cos(p.z));}
export MediumSample medium_world_coefficients(float3 p) {return MediumSample(float3(.1,.2,.3),float3(.3,.4,.5),float3(0.),0.);}
export float medium_world_tail_start(float3 o,float3 d) {return -3.;}
export MediumSample medium_world_tail_coefficients(float3 o,float3 d) {return medium_world_coefficients(o);}
export void medium_world_exponential(out float3 a,out float3 b,out float3 g) {a=float3(.1,.2,.3);b=float3(.3,.4,.5);g=float3(0.,0.,variant==1u || variant==3u ? -.2 : .2);}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    variant=id.y;calls=0u;
    let fraction=(float(id.x)+.5)/6144.;
    let m=variant==0u ? medium_world_constant_tail<true>(float3(0.),float3(0.,0.,1.),fraction,medium_world_coefficients(float3(0.)))
        : medium_world_exponential_tail<true>(float3(0.,0.,variant==3u ? 80. : 0.),float3(0.,0.,1.),fraction);
    values[(id.y*6144u+id.x)*2u]=float4(m.source,float(calls));
    values[(id.y*6144u+id.x)*2u+1u]=float4(m.transmission,0.);
}"#;
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let native=Native {slices:8,..Default::default()};
    let mut results=Vec::new();
    for sampled in [false,true] {
        // The full reference uses each cell's midpoint, just as production
        // continuation did before lighting was distributed over camera samples.
        let full=source.replace("(float(id.x)+.5)/6144.",if jittered {"(float(id.x%768u)+.5)/768."} else {".5"});
        let pipeline=test_pipeline_defines(&r,if sampled {source} else {&full},&native,&entries,(1,1,1),if sampled && jittered {&["-DVORK_MEDIUM_SAMPLED_LIGHTING","-DVORK_MEDIUM_JITTERED_LIGHTING"]} else if sampled {&["-DVORK_MEDIUM_SAMPLED_LIGHTING"]} else {&[]})?;
        let bytes=4*6144*2*16;
        let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(6144,4,1);}
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
        let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
        r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
        results.push(bytemuck::cast_slice::<u8,[f32;4]>(&readback.get_mapped_range(..)).to_vec());
    }
    for variant in 0..4 {
        let reference: [f32;4]=std::array::from_fn(|c| (0..6144).map(|i|results[0][variant*12288+i*2][c]).sum::<f32>()/6144.);
        assert!(reference.iter().all(|v|v.is_finite()),"nonfinite tail {variant}: {reference:?}");
        for channel in 0..3 {
            let mean=(0..6144).map(|i|results[1][variant*12288+i*2][channel]).sum::<f32>()/6144.;
            assert!(mean.is_finite() && (mean-reference[channel]).abs()<2e-4,"variant {variant}, channel {channel}: {mean} != {}",reference[channel]);
        }
        for i in 0..6144 {
            assert_eq!(results[1][variant*12288+i*2+1],results[0][variant*12288+1]);
            assert_eq!(results[1][variant*12288+i*2][3],1.);
            assert_eq!(reference[3],24.);
            assert!(reference[3]>0.);
        }
    }
    }
    r.errors.check()?;
    Ok(())
}

#[test]
fn medium_source_direction_mixture_preserves_diffuse_measure()->Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import medium_source_proposal;
import vork_constants;
import vork_sampling;
import vork_ray_sampling;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
static uint variant;
export uint medium_source_domains() {return variant+1u;}
export bool medium_source_domain(uint index) {return true;}
export float medium_source_cone(uint index,float3 from,out float3 axis) {
    axis=index==0u ? float3(0.,0.,1.) : index==1u ? float3(0.,0.,-1.) : normalize(float3(1.,0.,1.));
    return index==0u ? .1 : index==1u ? .4 : .7;
}
[shader("compute")][numthreads(64,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    variant=id.y;let n=float3(0.,0.,1.);let count=16384u;
    let disk=ray_concentric_disk(sample_2d(uint2(variant,0u),id.x,count,REG_SAMPLING_STREAM_RAY_GI));
    var direction=float3(disk,sqrt(max(1.-dot(disk,disk),0.)));
    let weight=medium_source_direction(float3(0.),n,uint2(variant,0u),id.x,count,direction);
    values[variant*count+id.x]=weight*float4(1.,direction.z,direction.x,direction.z>=.9 ? 1. : 0.);
}"#;
    let count=16384usize;let bytes=(count*3*16) as u64;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("medium source mixture"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,source,&Native::default(),&entries,(64,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(count as u32/64,3,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    // Constant furnace, cosine moment, odd moment and a narrow emitting cap.
    // Cases include a wholly backward proposal and overlapping forward cones.
    for (variant,samples) in values.chunks_exact(count).enumerate() {
        for (c,expected) in [1.,2./3.,0.,1.-0.9*0.9].into_iter().enumerate() {
            let actual=samples.iter().map(|x|f64::from(x[c])).sum::<f64>()/count as f64;
            assert!((actual-expected).abs()<0.012,"case {variant}, moment {c}: {actual} != {expected}");
        }
    }
    r.errors.check()?;Ok(())
}

#[test]
fn medium_source_direction_mixture_preserves_composited_furnace()->Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import medium_source_proposal;
import radiance_terms;
import vork_constants;
import vork_sampling;
import vork_ray_sampling;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
static uint variant;
export uint medium_source_domains() {return variant==0u ? 1u : 3u;}
export bool medium_source_domain(uint index) {return true;}
export float medium_source_cone(uint index,float3 from,out float3 axis) {
    axis=variant==0u ? float3(0.,0.,-1.) : index==0u ? float3(0.,0.,1.) : index==1u ? float3(0.,0.,-1.) : normalize(float3(1.,0.,1.));
    return index==0u ? .1 : index==1u ? .4 : .7;
}
[shader("compute")][numthreads(64,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    variant=id.y;let n=float3(0.,0.,1.);let count=16384u;
    let disk=ray_concentric_disk(sample_2d(uint2(variant,0u),id.x,count,REG_SAMPLING_STREAM_RAY_GI));
    var direction=float3(disk,sqrt(max(1.-dot(disk,disk),0.)));
    let weight=medium_source_direction(float3(0.),n,uint2(variant,0u),id.x,count,direction);
    // Variant 0: wholly backward source. Variant 1: overlapping forward
    // cones. Variant 2: the same mixture clipped by a tilted geometric normal.
    let ng=variant==2u ? normalize(float3(1.,0.,1.)) : n;
    let replaced=dot(direction,ng)>=0. ? weight : 0.;
    let world=float3(.25,.5,1.);let response=float3(.8,.6,.4);
    let baseline=response*world;
    let incident=world*replaced;
    let composed=compose_indirect(baseline,baseline,1.-replaced,float3(0.),response,incident);
    values[variant*count+id.x]=float4(composed,incident.z);
}"#;
    let count=16384usize;let bytes=(count*3*16) as u64;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("medium source mixture"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,source,&Native::default(),&entries,(64,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(count as u32/64,3,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for (variant,samples) in values.chunks_exact(count).enumerate() {
        let clipped_mass=if variant==2 {(1.+1./2_f64.sqrt())/2.} else {1.};
        for (c,expected) in [0.2,0.3,0.4,clipped_mass].into_iter().enumerate() {
            let actual=samples.iter().map(|x|f64::from(x[c])).sum::<f64>()/count as f64;
            assert!((actual-expected).abs()<0.012,"case {variant}, component {c}: {actual} != {expected}");
        }
    }
    r.errors.check()?;Ok(())
}

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

/// Stored per-light visibility must preserve the full HG peak, coloured
/// attenuation, light identity and shared exposure storage, including exterior rays.
#[test]
#[ignore = "requires pinned Slang and a GPU"]
fn medium_visibility_field_preserves_shared_lights_and_exact_phase() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let native=Native {tile:8,slices:4,visibility_lights:2,..Native::default()};
    let source=r#"
import vork_medium_visibility;
import vork_medium_visibility_grid;
import vork_prelude;
import vork_constants;
static Frame frame={};
static uint sample_index=0u;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> field;
[[vk::binding(1,0)]] RWStructuredBuffer<float4> result;
export Frame medium_frame() {var sampled=frame;sampled.eye.x+=float(sample_index);return sampled;}
export Frame medium_visibility_frame() {return frame;}
export uint2 medium_size() {return uint2(32u);}
export uint medium_visibility_sample() {return sample_index;}
export float4 medium_visibility_read(uint at) {return field[at];}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 3u;}
export uint medium_light_index(uint base,uint next) {return next;}
export Light medium_light(uint i) {
    Light light={};light.direction=float4(0.,0.,i==0u ? -1. : 1.,0.);
    light.radiance=float4(4.*REG_PI);light.shadow_offsets[REG_LIGHT_VOLUME_SLOT_WORD]=i==2u ? 1u : 0u;light.shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]=i==1u ? 0u : REG_RAY_VOLUME;return light;
}
// This algebra test isolates interpolation/phase; residual tests supply direct geometry.
export float3 medium_direct_visibility(float3 p,uint i) {return medium_cached_visibility(p,i); }
[shader("compute")][numthreads(8,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    frame.sampling.w=2u;frame.temporal.y=2.;frame.params.xy=float2(1.,10.);frame.background.w=2.;
    BODY
}
"#;
    let fill=r#"
    sample_index=id.y;let side=REG_MEDIUM_VISIBILITY_SIDE;
    if(id.x>=side*side*side) return;
    let cell=uint3(id.x%side,(id.x/side)%side,id.x/(side*side));
    let scale=.75;
    field[medium_visibility_address(cell,0u)]=float4(float3(.25,.5,.75)*(float(cell.x)/float(side-1u))*scale,0.);
    field[medium_visibility_address(cell,1u)]=float4(float3(1.,.5,.25)*scale,0.);
"#;
    let query=r#"
    if(id.x>=2u) return;sample_index=id.x;
    result[8u+id.x]=float4(medium_visibility_position(float3(id.x==0u ? -.5 : float(REG_MEDIUM_VISIBILITY_SIDE)-.5)),0.);
    result[id.x*4u]=float4(medium_shared_illumination(float3(0.),float3(0.,0.,1.),0.),0.);
    result[id.x*4u+1u]=float4(medium_shared_illumination(float3(0.),float3(0.,0.,1.),.8),0.);
    result[id.x*4u+2u]=float4(medium_shared_illumination(float3(10000.,0.,0.),float3(0.,0.,1.),.8),0.);
    result[id.x*4u+3u]=float4(medium_shared_illumination(float3(-10000.,0.,0.),float3(0.,0.,-1.),.8),0.);
"#;
    let entries=[0,1].map(|i|pipelines::buffer_entry(i,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false}));
    let fill=test_pipeline(&r,&source.replace("BODY",fill),&native,&entries,(8,1,1))?;
    let query=test_pipeline(&r,&source.replace("BODY",query),&native,&entries,(8,1,1))?;
    let field=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("visibility test field"),size:native.sample_bytes([32,32],2),usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false});
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("visibility test results"),size:160,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:160,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    let cells=vork_slang::registry::MEDIUM_VISIBILITY_SIDE.pow(3);
    for (pipeline,groups) in [(&fill,[cells.div_ceil(8),1,1]),(&query,[1,1,1])] {
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,field.as_entire_binding()),entry(1,output.as_entire_binding())]});
        let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(groups[0],groups[1],groups[2]);
    }
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,160);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
    r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let actual:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    let back=0.2/1.8f32.powi(2);
    for sample in 0..2 {for (i,(a,b)) in [(0.5,1.),(22.5,back),(45.,back),(0.,45.)].into_iter().enumerate() {
        for channel in 0..3 {
            let expected=([0.25,0.5,0.75][channel]*a+[1.,0.5,0.25][channel]*b)*0.75;
            assert!((actual[sample*4+i][channel]-expected).abs()<2e-4,"sample {sample}, query {i}: {actual:?}, expected {expected}");
        }
    }}
    for (index,sign) in [(8,-1.),(9,1.)] {
        assert!(actual[index][..3].iter().all(|v|v.is_finite() && v*sign>1e6),"endpoint stratum must map to a finite exterior point: {actual:?}");
    }
    r.errors.check()?;Ok(())
}

#[test]
fn environment_phase_connections_include_infinite_height_extinction()->Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import medium_environment;
import vork_medium_types;
import vork_prelude;
import vork_constants;
import vork_medium_point;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
static uint sample_index;
export Frame medium_frame() {Frame f={};f.medium.w=1u;f.temporal.xy=float2(sample_index,65536u);return f;}
export float3 medium_environment_radiance(float3 direction) {return float3(1.);}
export float3 medium_transmittance(float3 o,float3 d,MediumEndpoint endpoint) {
    return exp(-medium_exponential_tau(float3(.2,.5,0.),0.,-.7*d.y,REG_FP32_MAX,true));
}
[shader("compute")][numthreads(64,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    sample_index=id.x;
    let g=id.y==0u ? 0. : id.y==1u ? .7 : id.y==2u ? -.7 : id.y==3u ? 1. : -1.;
    values[id.y*65536u+id.x]=float4(medium_environment_illumination(float3(0.),float3(0.,1.,0.),g),0.);
}"#;
    let count=65536usize;let bytes=(count*5*16) as u64;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("World phase transport"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline_defines(&r,source,&Native {self_shadows:true,..Default::default()},&entries,(64,1,1),&["-DVORK_MEDIUM_ENVIRONMENT"])?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(count as u32/64,5,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for (variant,g) in [0f64,0.7,-0.7,1.,-1.].into_iter().enumerate() {
        for (channel,amplitude) in [0.2f64,0.5,0.].into_iter().enumerate() {
            let expected=if amplitude==0. {1.} else if g==1. {(-amplitude/0.7).exp()} else if g== -1. {0.} else {
                (0..65536).map(|i| {let cosine=(i as f64+0.5)/65536.;
                    0.5*(1.-g*g)/(1.+g*g-2.*g*cosine).powf(1.5)*(-amplitude/(0.7*cosine)).exp()
                }).sum::<f64>()/65536.
            };
            let actual=values[variant*count..(variant+1)*count].iter().map(|x|f64::from(x[channel])).sum::<f64>()/count as f64;
            assert!((actual-expected).abs()<0.003,"g={g}, channel {channel}: {actual} != {expected}");
        }
    }
    r.errors.check()?;Ok(())
}

#[test]
fn exponential_tail_preserves_grouped_and_distinct_spectral_extinction()->Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import vork_medium;
import vork_medium_types;
import vork_prelude;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
static uint variant;
export Frame medium_frame() {Frame f={};f.medium.w=1u;f.radiance=float4(2.,3.,4.,0.);return f;}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 0u;}
export uint medium_light_index(uint base,uint i) {return i;}
export Light medium_light(uint i) {return (Light)0;}
export float3 medium_visibility(float3 p,uint i) {return float3(1.);}
export MediumSample medium_world_coefficients(float3 p) {MediumSample m={};if(variant>=8u) {m.emission=float3(1.,0.,2.);}return m;}
export void medium_world_exponential(out float3 absorption,out float3 scattering,out float3 gradient) {
    let index=variant%4u;
    let sigma=index==0u ? float3(.2) : index==1u ? float3(.2,.2,.6) : index==2u ? float3(.2,.4,.6) : float3(0.,.2,.2);
    scattering=sigma*float3(.25,.5,.75);absorption=sigma-scattering;gradient=float3(0.,-.4,0.);
}
[shader("compute")][numthreads(8,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    if(id.x>=12u) {return;}
    variant=id.x;
    let direction=variant<4u ? float3(0.,1.,0.) : float3(1.,0.,0.);
    let m=medium_world_exponential_tail<true>(float3(0.,2.,0.),direction,.37);
    values[id.x*2u]=float4(m.source,0.);values[id.x*2u+1u]=float4(m.transmission,0.);
}"#;
    let bytes=384;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,source,&Native::default(),&entries,(8,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(2,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for variant in 0..12 {for channel in 0..3 {
        let sigma=[[0.2_f64;3],[0.2,0.2,0.6],[0.2,0.4,0.6],[0.,0.2,0.2]][variant%4][channel];
        let transmission=if sigma==0. {1.} else if variant<4 {(-sigma*(-0.8_f64).exp()/0.4).exp()} else {0.};
        let emission=if variant>=8 {[1.,0.,2.][channel]}else{0.};
        if sigma==0. && emission>0. {assert!(values[variant*2][channel].is_infinite());continue;}
        let emitted=if emission==0. {0.} else {emission*0.8_f64.exp()/sigma};
        let expected=emitted+(1.-transmission)*[0.25,0.5,0.75][channel]*[2.,3.,4.][channel];
        assert!((f64::from(values[variant*2][channel])-expected).abs()<2e-5,"source variant {variant} channel {channel}: {:?} != {expected}",values[variant*2]);
        assert!((f64::from(values[variant*2+1][channel])-transmission).abs()<2e-5);
    }}
    r.errors.check()?;Ok(())
}

#[test]
fn world_streaming_matches_overlapping_transport_and_linear_optical_depth()->Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    for steps in [1u32,7,64] {for world_only in [false,true] {for variant in 0..4 {
        eprintln!("world streaming: steps={steps} world_only={world_only} variant={variant}");
        let source=r#"
import vork_medium_bounds;
import vork_medium;
import vork_medium_types;
import vork_prelude;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
static const uint variant=VARIANT;
export Frame medium_frame() {Frame f={};f.medium.w=variant==3u ? 0u : 1u;f.radiance=float4(2.,3.,4.,0.);return f;}
export uint medium_word(uint at) {return 0u;}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 0u;}
export uint medium_light_index(uint base,uint i) {return i;}
export Light medium_light(uint i) {return (Light)0;}
export float3 medium_visibility(float3 p,uint i) {return float3(1.);}
MediumSample coefficients(float3 p) {
    let sigma=.1+.2*p.y;MediumSample m={};m.scattering=sigma*float3(.25,.5,.75);
    m.absorption=float3(sigma)-m.scattering;m.emission=sigma*float3(1.,0.,2.);return m;
}
export MediumSample volume_coefficients(uint id,float3 p,MediumTransportContext context) {return coefficients(p);}
export bool volume_homogeneous(uint id) {return false;}
export MediumSample volume_interval_coefficients(uint id,float3 o,float3 d,float distance,float fraction,MediumTransportContext context) {return coefficients(o+d*(distance*.5));}
export MediumSample medium_world_coefficients(float3 p) {return coefficients(p);}
export float medium_world_tail_start(float3 o,float3 d) {return 0.;}
export MediumSample medium_world_tail_coefficients(float3 o,float3 d) {return coefficients(o);}
export void medium_world_exponential(out float3 a,out float3 s,out float3 g) {a=float3(.1);s=float3(.2);g=float3(0.);}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {

    var m=MediumTransport();
    if(variant==1u) {m=volume_finite<STEPS,false>(float3(0.),float3(0.,1.,0.),medium_receiver(3.),.37,0xffffffffu,true);}
    else if(variant==2u) {m=volume_finite<STEPS,true,true>(float3(0.),float3(0.,1.,0.),medium_receiver(3.),.37,0xffffffffu,true);}
    else {m=volume_finite<STEPS,true>(float3(0.),float3(0.,1.,0.),medium_receiver(3.),.37,0xffffffffu,true);}
    values[0]=float4(m.source,0.);values[1]=float4(m.transmission,0.);values[2]=float4(m.optical_depth,0.);
}"#.replace("STEPS",&steps.to_string()).replace("VARIANT",&variant.to_string());
        let bytes=48;
        let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
        let pipeline=test_pipeline(&r,&source,&Native {steps,world_only,..Default::default()},&entries,(1,1,1))?;
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
        let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
        let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
        for channel in 0..3 {
            let tau=if variant==3 {0.} else {1.2_f64};let t=(-tau).exp();
            let source=if variant==1 || variant==3 {0.} else {(1.-t)*[1.5,1.5,5.][channel]};
            for (offset,expected) in [(0,source),(1,t),(2,tau)] {
                assert!((f64::from(values[offset][channel])-expected).abs()<3e-5,"stream={world_only} steps={steps} variant={variant} field={offset}: {:?} != {expected}",values[offset]);
            }
        }
    }}}
    r.errors.check()?;Ok(())
}

/// Exercise the production residual with deliberately wrong cached visibility.
/// The direct oracle intersects a thin slab and carries RGB extinction; queries
/// include points beyond the old lattice's outer centres and changed far clips.
#[test]
#[ignore = "requires pinned Slang and a GPU"]
fn medium_visibility_residual_corrects_thin_distant_and_coloured_queries() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import vork_medium_visibility;
import vork_medium_visibility_direct;
import vork_medium_types;
import vork_prelude;
import vork_constants;
static Frame frame={};
static uint calls=0u;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> result;
export Frame medium_frame() {return frame;}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 1u;}
export uint medium_light_index(uint base,uint next) {return next;}
export Light medium_light(uint i) {
    Light light={};light.direction=float4(-1.,0.,0.,0.);light.radiance=float4(4.*REG_PI);
    light.shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]=REG_RAY_VOLUME;return light;
}
export float3 medium_cached_visibility(float3 p,uint i) {
    return float3(.8,.2,.6)*(frame.params.y==10. ? .2 : 1.);
}
export float3 medium_visibility(float3 p,uint i) {
    calls++;
    // A +X connection crosses a thin slab at x=1, |y|<.005, |z|<1e7.
    return p.x<1. && abs(p.y)<.005 && abs(p.z)<1e7 ? float3(0.) : float3(1.);
}
export float3 medium_transmittance(float3 o,float3 d,MediumEndpoint endpoint) {return float3(.25,.5,.75);}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    let count=id.x==0u ? 2u : (id.x==1u ? 8u : (id.x==2u ? 64u : 256u));
    frame.background.w=4.;frame.temporal.y=float(count);frame.params.y=id.z==0u ? 10. : 1000.;
    let p=float3(0.,(id.y&1u)==0u ? 0. : .02,id.y<2u ? -2. : -1e6);
    var sum=float3(0.);
    for(uint sample=0u;sample<count;sample++) {
        frame.temporal.x=float(sample);
        sum+=medium_shared_illumination(p,float3(0.,0.,-1.),0.);
    }
    result[(id.z*4u+id.y)*4u+id.x]=float4(sum/float(count),float(calls)/float(count));
}
"#;
    for shared in [true,false] {for sampled in [false,true] {for clamp in [0.,0.3] {
        let native=Native {self_shadows:true,light_clamp:clamp,..Native::default()};
        let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
        // Four identical sources make the sampled sum exact. The clamp must
        // precede the inverse PDF, for both shared residual and direct services.
        let direct=source.replace("import vork_medium_visibility;","import vork_medium;").replace("medium_shared_illumination(","medium_illumination(");
        let pipeline=test_pipeline_defines(&r,if shared {source} else {&direct},&native,&entries,(1,1,1),if sampled {&["-DVORK_MEDIUM_SAMPLED_ANALYTIC_LIGHT"]} else {&[]})?;
        let bytes=32*16;
        let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("visibility residual results"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(4,4,2);}
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
        let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
        r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
        let mapped=readback.get_mapped_range(..);let actual:&[[f32;4]]=bytemuck::cast_slice(&mapped);
        for (i,value) in actual.iter().enumerate() {
            let blocked=(i/4)%2==0;
            for c in 0..3 {
                let expected=if blocked {0.}else{[0.25,0.5,0.75][c]*if clamp==0. {1.} else {0.4}};
                let expected=expected*if sampled {4.} else {1.};
                assert!((value[c]-expected).abs()<2e-5,"clamp={clamp} query={i}: {value:?}, channel={c} expected={expected}");
            }
            assert_eq!(value[3],if shared {0.5} else {1.},"direct support/cost at query {i}");
        }
    }
    }}
    r.errors.check()?;Ok(())
}
/// Varying incident light must integrate against extinction, not a uniform cell
/// average. One cell deliberately spans several mean free paths in blue.
#[test]
#[ignore="requires pinned Slang and a GPU"]
fn homogeneous_cell_lighting_uses_rgb_extinction_measure()->Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import vork_medium_bounds;
import vork_medium;
import vork_medium_types;
import vork_prelude;
import vork_constants;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> result;
export Frame medium_frame() {Frame f={};f.medium.w=1u;f.temporal.y=4096.;return f;}
export uint medium_word(uint at) {return 0u;}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 1u;}
export uint medium_light_index(uint base,uint i) {return i;}
export Light medium_light(uint i) {Light l={};l.radiance=float4(4.*REG_PI);l.direction=float4(0.,0.,-1.,0.);l.shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]=REG_RAY_VOLUME;return l;}
export float3 medium_visibility(float3 p,uint i) {return float3(p.y);}
MediumSample coefficients() {MediumSample m={};let sigma=float3(.1,.5,2.);m.scattering=sigma*float3(.25,.5,.75);m.absorption=sigma-m.scattering;return m;}
export MediumSample volume_coefficients(uint id,float3 p,MediumTransportContext context) {return coefficients();}
export bool volume_homogeneous(uint id) {return true;}
export MediumSample volume_interval_coefficients(uint id,float3 o,float3 d,float distance,float fraction,MediumTransportContext context) {return coefficients();}
export MediumSample medium_world_coefficients(float3 p) {return coefficients();}
export float medium_world_tail_start(float3 o,float3 d) {return 0.;}
export MediumSample medium_world_tail_coefficients(float3 o,float3 d) {return coefficients();}
export void medium_world_exponential(out float3 a,out float3 s,out float3 g) {a=float3(0.);s=float3(0.);g=float3(0.);}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    let u=(float(id.x)+.5)/4096.;
    let value=volume_finite<1,true>(float3(0.),float3(0.,1.,0.),medium_receiver(3.),u,0xffffffffu,true);
    let extinction=id.x==0u ? 0. : id.x==1u ? 1e-38 : id.x==2u ? 1e-8 : id.x==3u ? .1 : id.x==4u ? 1. : 100.;
    result[id.x]=float4(value.source,medium_extinction_distance(extinction,3.,.37));
}
"#;
    let bytes=4096*16;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,source,&Native {homogeneous:true,world_only:true,steps:1,..Default::default()},&entries,(1,1,1))?;
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(4096,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for (i,sigma) in [0_f64,1e-38,1e-8,0.1,1.,100.].into_iter().enumerate() {
        let expected=if sigma==0. {0.37*3.} else {-(-0.37*(-(-sigma*3.).exp_m1())).ln_1p()/sigma};
        assert!((f64::from(values[i][3])-expected).abs()<1e-6*(1.+expected),"quantile sigma={sigma}: {} vs {expected}",values[i][3]);
    }
    for c in 0..3 {
        let sigma=[0.1_f64,0.5,2.][c];let albedo=[0.25,0.5,0.75][c];
        let expected=albedo*(1.-(1.+3.*sigma)*(-3.*sigma).exp())/sigma;
        let actual=values.iter().map(|v|f64::from(v[c])).sum::<f64>()/4096.;
        assert!((actual/expected-1.).abs()<1e-4,"channel={c} actual={actual} expected={expected}");
    }
    r.errors.check()?;Ok(())
}

/// Two coincident domains share extinction while retaining independent coloured
/// scattering and emission. Huge finite illumination must not overflow the
/// source-density product that the former numerical adapter constructed.
#[test]
fn production_overlapping_dense_sources_match_independent_integrals()->Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    for homogeneous in [false,true] {for steps in [1u32,8] {
        let source=r#"
import vork_medium_bounds;
import vork_medium;
import vork_medium_types;
import vork_prelude;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
export Frame medium_frame() {Frame f={};f.medium.w=1u;f.radiance=float4(1e30,2.,3.,0.);return f;}
export uint medium_word(uint at) {return 0u;}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 0u;}
export uint medium_light_index(uint base,uint i) {return i;}
export Light medium_light(uint i) {return (Light)0;}
export float3 medium_visibility(float3 p,uint i) {return float3(1.);}
MediumSample coefficients(uint id) {
    MediumSample m={};
    m.scattering=id==0u ? float3(7e29,.1,0.) : float3(3e29,.4,0.);
    m.absorption=id==0u ? float3(3e29,.3,0.) : float3(7e29,.2,0.);
    m.emission=id==0u ? float3(1e30,.2,1.) : float3(1e30,.3,2.);
    return m;
}
export MediumSample volume_coefficients(uint id,float3 p,MediumTransportContext c) {return coefficients(id);}
export MediumSample volume_interval_coefficients(uint id,float3 o,float3 d,float distance,float fraction,MediumTransportContext c) {return coefficients(id);}
export bool volume_homogeneous(uint id) {return HOMOGENEOUS;}
export MediumSample medium_world_coefficients(float3 p) {return MediumSample();}
export float medium_world_tail_start(float3 o,float3 d) {return 0.;}
export MediumSample medium_world_tail_coefficients(float3 o,float3 d) {return MediumSample();}
export void medium_world_exponential(out float3 a,out float3 b,out float3 g) {a=0.;b=0.;g=0.;}
[shader("compute")][numthreads(1,1,1)] void numerical_test() {
    let m=volume_finite<STEPS,true>(float3(0.),float3(0.,0.,1.),medium_receiver(2.),.5,0xffffffffu,true);
    values[0]=float4(m.source,0.);values[1]=float4(m.transmission,0.);values[2]=float4(m.optical_depth,0.);
}"#.replace("STEPS",&steps.to_string()).replace("HOMOGENEOUS",if homogeneous {"true"} else {"false"});
        let bytes=48;
        let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
        let mut native=Native {steps,homogeneous,world_only:false,..Default::default()};
        native.modules.push(("vork_medium_domain".into(),numerical_domains(1)));
        let pipeline=test_pipeline(&r,&source,&native,&entries,(1,1,1))?;
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
        let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
        let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
        assert!(values.iter().flatten().all(|v|v.is_finite()),"homogeneous={homogeneous} steps={steps}: {values:?}");
        // Independent closed-form sum: sigma=(2e30,1,0), source density
        // (2e30+1e60,1.5,3), integrated over two metres. Form ratios in f64.
        let expected=[5e29_f64,1.5*(-(-2_f64).exp_m1()),6.];
        for channel in 0..3 {assert!((f64::from(values[0][channel])/expected[channel]-1.).abs()<2e-5,"source {homogeneous} {steps}: {:?} vs {expected:?}",values[0]);}
        assert!(values[1][0]<=2e-35 && (f64::from(values[1][1])-(-2_f64).exp()).abs()<2e-6 && values[1][2]==1.);
        assert!((f64::from(values[2][0])/4e30-1.).abs()<2e-6 && (values[2][1]-2.).abs()<2e-6 && values[2][2]==0.);
        drop(mapped);readback.unmap();
    }}
    r.errors.check()?;Ok(())
}
#[path = "medium_numeric_tests/dense.rs"]
mod dense_overlap;

/// One RGB draw must retain each extinction-weighted integral, including a
/// varying incident field and transparent channels. The oracle is analytic.
#[test]
#[ignore = "requires pinned Slang and a real GPU"]
fn sampled_finite_spectral_mixture_preserves_energy() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import vork_medium_point;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
[shader("compute")][numthreads(64,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
    let sigma=id.y==0u ? float3(0.,.4,3.) : float3(.001,10.,1000.);
    MediumSample m={};m.absorption=sigma;
    MediumExtinction sum={};medium_extinction_add(sum,m);
    float x;let weight=medium_extinction_sample(sum,2.,(float(id.x)+.5)/24576.,x);
    let integral=medium_integrated_coefficient(float3(1.),sum,2.,exp(-sigma*2.));
    values[id.y*24576u+id.x]=float4(integral*weight*(1.+x),x);
}"#;
    let bytes=2*24576*16;
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let pipeline=test_pipeline(&r,source,&Native::default(),&entries,(64,1,1))?;
    let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(384,2,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
    r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=readback.get_mapped_range(..);let values:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    assert!(values.iter().flatten().all(|v|v.is_finite()));
    for (variant,sigmas) in [[0.0_f64,0.4,3.],[0.001,10.,1000.]].iter().enumerate() {
        for (channel,&sigma) in sigmas.iter().enumerate() {
            let expected=if sigma==0. {4.} else {(1.-(-2.*sigma).exp())/sigma+(1.-(1.+2.*sigma)*(-2.*sigma).exp())/(sigma*sigma)};
            let mean=(0..24576).map(|i|f64::from(values[variant*24576+i][channel])).sum::<f64>()/24576.;
            assert!((mean/expected-1.).abs()<2e-4,"variant {variant}, channel {channel}: {mean} != {expected}");
        }
    }
    r.errors.check()?;Ok(())
}

#[test]
#[ignore = "requires pinned Slang and a real GPU"]
fn sampled_exponential_emission_retains_complete_quadrature() -> Result<()> {
    let Some(r)=crate::test_renderer()? else {return Ok(());};
    let source=r#"
import vork_medium;
import vork_prelude;
[[vk::binding(0,0)]] RWStructuredBuffer<float4> values;
export Frame medium_frame() {return (Frame)0;}
export uint medium_light_list(float3 p,out uint base) {base=0u;return 0u;}
export uint medium_light_index(uint base,uint i) {return i;}
export Light medium_light(uint i) {return (Light)0;}
export float3 medium_visibility(float3 p,uint i) {return float3(1.);}
export MediumSample medium_world_coefficients(float3 p) {return MediumSample(float3(.2,.4,.6),float3(0.),float3(1.,2.,3.),0.);}
export void medium_world_exponential(out float3 absorption,out float3 scattering,out float3 gradient) {
    absorption=float3(.2,.4,.6);scattering=float3(0.);gradient=float3(0.,0.,.4);
}
[shader("compute")][numthreads(1,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {
#ifdef VORK_MEDIUM_SAMPLED_LIGHTING
    let fraction=(float(id.x)+.5)/32.;
#else
    let fraction=.5;
#endif
    let m=medium_world_exponential_tail<true>(float3(0.),float3(0.,0.,1.),fraction);
    values[id.x]=float4(m.source,0.);
}"#;
    let entries=[pipelines::buffer_entry(0,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:false})];
    let mut results=Vec::new();
    for sampled in [false,true] {
        let pipeline=test_pipeline_defines(&r,source,&Native {slices:64,..Default::default()},&entries,(1,1,1),if sampled {&["-DVORK_MEDIUM_SAMPLED_LIGHTING"]} else {&[]})?;
        let bytes=32*16;
        let output=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let bind=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,output.as_entire_binding())]});
        let mut encoder=r.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(32,1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
        let submission=r.queue.submit([encoder.finish()]);let (tx,rx)=std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read,..,move |v| {let _=tx.send(v);});
        r.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
        results.push(bytemuck::cast_slice::<u8,[f32;4]>(&readback.get_mapped_range(..)).to_vec());
    }
    for channel in 0..3 {
        // Independent world-distance integral of emission times Beer-Lambert
        // transmission, rather than the production optical-flight quadrature.
        let sigma=[0.2_f64,0.4,0.6][channel];let steps=65536;let dt=32./f64::from(steps);
        let reference=(0..steps).map(|i| {
            let distance=(f64::from(i)+0.5)*dt;
            (-sigma*((0.4*distance).exp()-1.)/0.4).exp()*dt*(channel+1) as f64
        }).sum::<f64>();
        assert!((f64::from(results[0][0][channel])/reference-1.).abs()<0.002,"emission channel {channel}: {:?} vs {reference}",results[0][0]);
        for value in &results[1] {
            assert!((value[channel]-results[0][0][channel]).abs()<2e-6,"lighting selection changed emission: {value:?} vs {:?}",results[0][0]);
        }
    }
    r.errors.check()?;Ok(())
}
