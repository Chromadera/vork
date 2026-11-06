//! GPU numerical coverage for the implicit-cell kernel, not scene integration.
use super::*;

#[test]
#[ignore = "requires Metal/Vulkan and pinned Slang; scripts/ci-tests.py --suite gpu"]
fn implicit_cell_isolates_multiple_crossings_and_gradients() -> Result<()> {
    // This qualification test must fail, rather than silently pass, without a GPU.
    let renderer = Renderer::new()?;
    let mut cases = Vec::<[f32; 16]>::new();
    let mut expected = Vec::<Option<(f32, [f32; 3])>>::new();
    let mut add = |values: [f32; 8], start: [f32; 3], end: [f32; 3], root: Option<f32>, gradient: [f32; 3]| {
        let mut row = [0.; 16];
        row[..8].copy_from_slice(&values);
        row[8..11].copy_from_slice(&start);
        row[12..15].copy_from_slice(&end);
        cases.push(row);
        expected.push(root.map(|t| (t, gradient)));
    };
    // Two roots, equal endpoint signs. Endpoint-only marching misses both.
    add(std::array::from_fn(|i| ((i & 1) as f32 - 0.25) * (((i >> 1) & 1) as f32 - 0.75)),
        [0.; 3], [1.; 3], Some(0.25), [-0.5, 0., 0.]);
    // Planar, parallel, miss, constant zero, and an exact endpoint root.
    let plane = std::array::from_fn(|i| (i & 1) as f32 - 0.5);
    add(plane, [0.; 3], [1.; 3], Some(0.5), [1., 0., 0.]);
    add(plane, [0.; 3], [0., 1., 1.], None, [0.; 3]);
    add([1.; 8], [0.; 3], [1.; 3], None, [0.; 3]);
    add([0.; 8], [0.; 3], [1.; 3], None, [0.; 3]);
    add(std::array::from_fn(|i| (i & 1) as f32), [0.; 3], [1.; 3], Some(0.), [1., 0., 0.]);
    // Exact tangent to a regular (nonzero-gradient) saddle surface.
    add(std::array::from_fn(|i| (i&1) as f32*((i>>1)&1) as f32-((i>>2)&1) as f32+0.25),
        [0.; 3], [1.; 3], Some(0.5), [0.5,0.5,-1.]);
    // Independent analytic oracle: a product of three planes is trilinear.
    // Vary root order, segment direction, entry points and coefficient scale.
    for index in 0..512usize {
        let roots = [0.10 + (index % 17) as f32 * 0.006,
                     0.42 + (index % 13) as f32 * 0.006,
                     0.76 + (index % 19) as f32 * 0.006];
        let start = [0.02, 0.12, 0.07];
        let end = [0.93, 0.88, 0.99];
        let perm = index % 3;
        let levels: [f32; 3] = std::array::from_fn(|a| start[a] + (end[a]-start[a])*roots[(a+perm)%3]);
        let scale = if index % 2 == 0 { 1. } else { -0.03125 };
        let values = std::array::from_fn(|i| scale * (0..3).map(|a| ((i>>a)&1) as f32-levels[a]).product::<f32>());
        let reverse = index % 4 >= 2;
        let t = if reverse { roots[2] } else { roots[0] };
        let p: [f32; 3] = std::array::from_fn(|a| start[a]+t*(end[a]-start[a]));
        let gradient = std::array::from_fn(|a| scale*(p[(a+1)%3]-levels[(a+1)%3])*(p[(a+2)%3]-levels[(a+2)%3]));
        add(values, if reverse {end} else {start}, if reverse {start} else {end}, Some(if reverse {1.-t} else {t}), gradient);
    }
    let source = format!(r#"
import implicit_cell;
import implicit_grid;
import implicit_normal;
struct Case {{float4 low;float4 high;float4 start;float4 end;}};
struct Grid : IImplicitGrid {{
    bool accelerated;
    bool wide;
    uint cell_step(uint3 cell_index) {{
        if(!accelerated) return 1u;
        let lo=(cell_index/8u)*8u;let hi=min(lo+8u,dimensions()-1u);
        var low=1e30;var high=-1e30;
        for(uint i=0u;i<8u;i++) {{
            let c=uint3((i&1u)!=0u ? hi.x : lo.x,(i&2u)!=0u ? hi.y : lo.y,(i&4u)!=0u ? hi.z : lo.z);
            let v=value(c);low=min(low,v);high=max(high,v);
        }}
        return low>0. || high<0. ? 8u : 1u;
    }}
    ImplicitCell cell;
    uint3 dimensions() {{return wide ? uint3(768u,256u,768u) : uint3(7u,9u,11u);}}
    float value(uint3 vertex) {{return implicit_value(cell,float3(vertex)/float3(dimensions()-1u));}}
}};
struct EmptyGrid : IImplicitGrid {{
    uint cell_step(uint3 cell) {{return 8u;}}
    uint3 dimensions() {{return uint3(768u,256u,768u);}}
    float value(uint3 vertex) {{return 1.;}}
}};
struct FullGrid : IImplicitGrid {{
    uint cell_step(uint3 cell) {{return 8u;}}
    uint3 dimensions() {{return uint3(7u,9u,11u);}}
    float value(uint3 vertex) {{return -1.;}}
}};
struct SphereGrid : IImplicitGrid {{
    uint cell_step(uint3 cell) {{return 1u;}}
    uint3 dimensions() {{return uint3(32u,40u,48u);}}
    float value(uint3 vertex) {{float3 p=(float3(vertex)+.5)*2./float3(dimensions())-1.;return dot(p,p)-.42;}}
}};
[[vk::binding(0,0)]] StructuredBuffer<Case> cases;
[[vk::binding(1,0)]] RWStructuredBuffer<float4> results;
[shader("compute")][numthreads(64,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {{
    if(id.x>={count}u) return;
    let input=cases[id.x];
    ImplicitCell cell;
    for(uint i=0u;i<4u;i++) {{cell.values[i]=input.low[i];cell.values[4u+i]=input.high[i];}}
    let hit=implicit_cell_hit(cell,input.start.xyz,input.end.xyz);
    results[id.x]=float4(hit.gradient,hit.found ? hit.fraction : -1.);
    Grid grid;grid.cell=cell;grid.accelerated=false;grid.wide=false;
    let extent=1.-1./float3(grid.dimensions());
    let surface=implicit_grid_hit(grid,0.,(2.*input.start.xyz-1.)*extent,2.*(input.end.xyz-input.start.xyz)*extent,0.,1.);
    let normal=any(surface.gradient!=float3(0.)) ? normalize(surface.gradient) : float3(0.);
    results[{count}u+id.x]=float4(normal,surface.valid ? (surface.found ? surface.distance : -1.) : -2.);
    if(surface.found) {{
        let local=(2.*lerp(input.start.xyz,input.end.xyz,surface.distance)-1.)*extent;
        results[2u*{count}u+id.x]=float4(normalize(implicit_shading_gradient(grid,local)),0.);
    }}
    // Large grids round an analytic tangent's vertex samples enough to
    // remove the tangent. Compare traversal of the same discrete field here;
    // keep the analytic oracle above on its original small grid.
    grid.wide=true;grid.accelerated=false;
    let large_extent=1.-1./float3(grid.dimensions());
    let dense=implicit_grid_hit(grid,0.,(2.*input.start.xyz-1.)*large_extent,2.*(input.end.xyz-input.start.xyz)*large_extent,0.,1.);
    grid.accelerated=true;
    let skipped=implicit_grid_hit(grid,0.,(2.*input.start.xyz-1.)*large_extent,2.*(input.end.xyz-input.start.xyz)*large_extent,0.,1.);
    results[4u*{count}u+id.x]=float4(skipped.gradient,skipped.valid ? (skipped.found ? skipped.distance : -1.) : -2.);
    results[5u*{count}u+id.x]=float4(dense.gradient,dense.valid ? (dense.found ? dense.distance : -1.) : -2.);
    if(id.x==0u) {{
        FullGrid full;
        let entry=implicit_grid_hit(full,0.,float3(-2.,0.,0.),float3(2.,0.,0.),0.,4.);
        let exit=implicit_grid_hit(full,0.,float3(0.),float3(0.,0.,4.),0.,4.);
        let short_ray=implicit_grid_hit(full,0.,float3(0.),float3(0.,0.,4.),0.,.2);
        results[6u*{count}u+2u]=float4(entry.gradient,entry.found && entry.boundary ? entry.distance : -1.);
        results[6u*{count}u+3u]=float4(exit.gradient,exit.found && exit.boundary ? exit.distance : -1.);
        results[6u*{count}u+4u]=float4(short_ray.valid ? 1. : 0.,short_ray.found ? 1. : 0.,0.,0.);
        EmptyGrid empty;
        // This ray cycled after reprojection rounded behind another cell face.
        let missed=implicit_grid_hit(empty,0.,float3(-.560008347,.341729373,-.903421700),float3(.175251395,.401508659,.822773635),0.,1e30);
        results[6u*{count}u]=float4(missed.valid ? 1. : 0.,missed.found ? 1. : 0.,0.,0.);
        // A ray offset can exceed the remaining finite shadow chord.
        let exhausted=implicit_grid_hit(empty,0.,float3(0.),float3(1.,0.,0.),1e-6,1e-8);
        let nonfinite=implicit_grid_hit(empty,0.,float3(0.),float3(1.,0.,0.),asfloat(0x7fc00000u),1.);
        let stationary=implicit_grid_hit(empty,0.,float3(0.),float3(0.),1.,0.);
        results[6u*{count}u+1u]=float4(exhausted.valid ? 1. : 0.,exhausted.found ? 1. : 0.,nonfinite.valid ? 1. : 0.,stationary.valid ? 1. : 0.);
    }}
    SphereGrid sphere;
    // A quadratic sphere has exact centred differences. Sweep continuously
    // across grid faces in an anisotropic grid, independently of ray roots.
    float3 position=float3(-.6+1.2*float(id.x)/float({count}u-1u),.23,.37);
    results[3u*{count}u+id.x]=float4(normalize(implicit_shading_gradient(sphere,position)),0.);
}}
"#, count=cases.len());
    let d = &renderer.device;
    let entries: Vec<_> = (0..2).map(|binding| pipelines::buffer_entry(binding,
        wgpu::ShaderStages::COMPUTE, wgpu::BufferBindingType::Storage {read_only: binding==0})).collect();
    let pipeline = medium::test_pipeline(&renderer, &source, &medium::Native::default(), &entries, (64,1,1))?;
    use wgpu::util::DeviceExt;
    let input = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("implicit cell cases"), contents: bytemuck::cast_slice(&cases), usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (cases.len()*112) as u64;
    let output = d.create_buffer(&wgpu::BufferDescriptor {label: None, size: bytes,
        usage: wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false});
    let readback = d.create_buffer(&wgpu::BufferDescriptor {label: None, size: bytes,
        usage: wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST, mapped_at_creation: false});
    let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {label: None,
        layout: &pipeline.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry {binding: 0, resource: input.as_entire_binding()},
            wgpu::BindGroupEntry {binding: 1, resource: output.as_entire_binding()},
        ]});
    let mut encoder = d.create_command_encoder(&Default::default());
    {let mut pass = encoder.begin_compute_pass(&Default::default());
     pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);
     pass.dispatch_workgroups((cases.len() as u32).div_ceil(64),1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    let submitted = renderer.queue.submit([encoder.finish()]);
    let (tx,rx) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read,..,move |r| {let _=tx.send(r);});
    d.poll(wgpu::PollType::Wait {submission_index:Some(submitted),timeout:None})?;
    rx.recv()??;
    renderer.errors.check()?;
    let mapped = readback.get_mapped_range(..);
    let results: &[[f32;4]] = bytemuck::cast_slice(&mapped);
    assert_eq!(results[6*cases.len()],[1.,0.,0.,0.],"empty-block traversal must terminate with a valid miss");
    assert_eq!(results[6*cases.len()+1],[1.,0.,0.,0.],"finite exhausted intervals miss; nonfinite or zero-direction rays remain invalid");
    assert_eq!(results[6*cases.len()+2],[-1.,0.,0.,0.5],"negative boundary must close on the physical entry face");
    assert_eq!(results[6*cases.len()+3],[0.,0.,1.,0.25],"inside rays must exit the physical domain");
    assert_eq!(results[6*cases.len()+4],[1.,0.,0.,0.],"clipped inside rays must not invent a domain exit");
    for (i,(actual,expected)) in results.iter().zip(expected).enumerate() {
        match expected {
            None => assert_eq!(actual[3],-1.,"case {i}: unexpected hit {actual:?}"),
            Some((t,gradient)) => {
                assert!((actual[3]-t).abs()<2e-5,"case {i}: {actual:?}, expected fraction {t}");
                for axis in 0..3 {assert!((actual[axis]-gradient[axis]).abs()<2e-5,
                    "case {i}: gradient {actual:?}, expected {gradient:?}");}
            }
        }
        let position=Vec3::new(-0.6+1.2*i as f32/(cases.len()-1) as f32,0.23,0.37);
        let smooth=results[3*cases.len()+i];
        assert!((Vec3::new(smooth[0],smooth[1],smooth[2])-position.normalize()).length()<2e-5,
            "quadratic sphere normal {i}: {smooth:?}");
        let grid = results[cases.len()+i];
        let accelerated=results[4*cases.len()+i];
        let dense=results[5*cases.len()+i];
        assert!((accelerated[3]-dense[3]).abs()<2e-5,"occupied-block traversal changed root {i}: {accelerated:?} versus {dense:?}");
        match expected {
            None => assert_eq!(grid[3],-1.,"grid case {i}: unexpected hit {grid:?}"),
            Some((t,gradient)) => {
                assert!((grid[3]-t).abs()<2e-5,"grid case {i}: {grid:?}, expected fraction {t}");
                let normal = (Vec3::from(gradient)/Vec3::new(6./7.,8./9.,10./11.)).normalize_or_zero();
                assert!((Vec3::new(grid[0],grid[1],grid[2])-normal).length()<2e-4,
                    "grid case {i}: normal {grid:?}, expected {normal:?}");
                let smooth=results[2*cases.len()+i];
                assert!((Vec3::new(smooth[0],smooth[1],smooth[2])-normal).length()<2e-4,
                    "reconstructed trilinear normal {i}: {smooth:?}, expected {normal:?}");
            }
        }
    }
    Ok(())
}
