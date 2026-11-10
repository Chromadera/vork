//! GPU coverage for the path contribution clamp shared by every accumulation site.
use super::*;

#[test]
#[ignore = "requires Metal/Vulkan and pinned Slang; scripts/ci-tests.py --suite gpu"]
fn clamp_sample_drops_nonfinite_and_keeps_finite_clamp() -> Result<()> {
    let renderer = Renderer::new()?;
    let inf = f32::INFINITY;
    let nan = f32::NAN;
    // [r, g, b, limit] -> expected rgb.
    let cases: [([f32; 4], [f32; 3]); 12] = [
        // Non-finite input is dropped whatever the limit, including the unclamped direct limit 0.
        ([inf, 0., 0., 10.], [0.; 3]),
        ([-inf, 1., 1., 10.], [0.; 3]),
        ([nan, 1., 1., 10.], [0.; 3]),
        ([inf, 0., 0., 0.], [0.; 3]),
        ([1., nan, 1., 0.], [0.; 3]),
        ([inf, -inf, 0., 0.], [0.; 3]),
        // Finite behaviour is unchanged: below the limit, limit off, and clamped by mean |RGB|.
        ([3., 6., 9., 10.], [3., 6., 9.]),
        ([300., 600., 900., 0.], [300., 600., 900.]),
        ([30., 60., 90., 10.], [5., 10., 15.]),
        ([-30., 0., 0., 5.], [-15., 0., 0.]),
        ([3e30, 0., 0., 10.], [30., 0., 0.]),
        ([0., 0., 0., 10.], [0.; 3]),
    ];
    let inputs: Vec<[f32; 4]> = cases.iter().map(|c| c.0).collect();
    let source = format!(r#"
import sample_clamp;
[[vk::binding(0,0)]] StructuredBuffer<float4> cases;
[[vk::binding(1,0)]] RWStructuredBuffer<float4> results;
[[vk::binding(2,0)]] RWStructuredBuffer<uint> diagnostics;
[shader("compute")][numthreads(64,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {{
    if(id.x>={count}u) return;
    let c=cases[id.x];
    uint dropped=0u;
    results[id.x]=float4(clamp_sample(c.xyz,c.w,dropped),0.);
    clamp_sample(c.xyz,c.w,dropped);
    record_dropped_light_samples(diagnostics,dropped);
}}
"#, count = inputs.len());
    let d = &renderer.device;
    let entries: Vec<_> = (0..3).map(|binding| pipelines::buffer_entry(binding,
        wgpu::ShaderStages::COMPUTE, wgpu::BufferBindingType::Storage {read_only: binding==0})).collect();
    let pipeline = medium::test_pipeline(&renderer, &source, &medium::Native::default(), &entries, (64,1,1))?;
    use wgpu::util::DeviceExt;
    let input = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("clamp sample cases"), contents: bytemuck::cast_slice(&inputs), usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (inputs.len()*16) as u64;
    let output = d.create_buffer(&wgpu::BufferDescriptor {label: None, size: bytes,
        usage: wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false});
    let readback = d.create_buffer(&wgpu::BufferDescriptor {label: None, size: bytes,
        usage: wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST, mapped_at_creation: false});
    let mut initial = materials::SHADER_DIAGNOSTICS_INIT;
    initial[vork_slang::registry::DROPPED_LIGHT_SAMPLES_WORD as usize] = u32::MAX - 2;
    let counters = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("drop counters"), contents: bytemuck::cast_slice(&initial),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let counter_readback = d.create_buffer(&wgpu::BufferDescriptor {label: None,
        size: materials::SHADER_DIAGNOSTICS_BYTES,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false});
    let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {label: None,
        layout: &pipeline.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry {binding: 0, resource: input.as_entire_binding()},
            wgpu::BindGroupEntry {binding: 1, resource: output.as_entire_binding()},
            wgpu::BindGroupEntry {binding: 2, resource: counters.as_entire_binding()},
        ]});
    let mut encoder = d.create_command_encoder(&Default::default());
    {let mut pass = encoder.begin_compute_pass(&Default::default());
     pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);
     pass.dispatch_workgroups(1,1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    encoder.copy_buffer_to_buffer(&counters,0,&counter_readback,0,materials::SHADER_DIAGNOSTICS_BYTES);
    let submitted = renderer.queue.submit([encoder.finish()]);
    let (tx,rx) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |r| {let _=tx.send(r);});
    let (counter_tx,counter_rx) = std::sync::mpsc::channel();
    counter_readback.map_async(wgpu::MapMode::Read,..,move |r| {let _=counter_tx.send(r);});
    d.poll(wgpu::PollType::Wait {submission_index:Some(submitted),timeout:None})?;
    rx.recv()??;
    counter_rx.recv()??;
    let counter_map = counter_readback.get_mapped_range(..);
    let counts: &[u32] = bytemuck::cast_slice(&counter_map);
    let offset = vork_slang::registry::DROPPED_LIGHT_SAMPLES_WORD as usize;
    assert_eq!(counts[offset], 9, "twelve drops, aggregated in pairs, carry past u32::MAX; finite samples do not count");
    assert_eq!(counts[offset+1], 1);
    renderer.errors.check()?;
    let mapped = readback.get_mapped_range(..);
    let results: &[[f32;4]] = bytemuck::cast_slice(&mapped);
    for (i, ((input, expected), actual)) in cases.iter().zip(results).enumerate() {
        for k in 0..3 {
            assert!(actual[k].is_finite() && (actual[k]-expected[k]).abs() <= 1e-5*expected[k].abs().max(1.),
                "case {i} {input:?}: got {actual:?}, expected {expected:?}");
        }
    }
    Ok(())
}
