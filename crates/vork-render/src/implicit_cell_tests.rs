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
    add(std::array::from_fn(|i| (i&1) as f32-1.),
        [0.; 3], [1.; 3], Some(1.), [1.,0.,0.]);
    add(std::array::from_fn(|i| (i&1) as f32*(((i>>1)&1) as f32-1.)),
        [0.; 3], [1.; 3], Some(0.), [-1.,0.,0.]);
    // Complete ordered sequences are independent analytic expectations, not
    // just agreement with an eager implementation. Include both endpoints.
    let mut sequences=vec![vec![0.25,0.75],vec![0.5],vec![],vec![],vec![],vec![0.],vec![0.5],vec![1.],vec![0.,1.]];
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
        sequences.push(if reverse {roots.into_iter().rev().map(|t|1.-t).collect()} else {roots.to_vec()});
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
    var roots=implicit_cell_roots(cell,input.start.xyz,input.end.xyz);
    float4 sequence=float4(-1.);uint emitted=0u;
    for(uint slot=0u;slot<4u;slot++) {{
        float fraction;
        if(!implicit_cell_next(cell,input.start.xyz,input.end.xyz,roots,fraction)) break;
        sequence[slot]=fraction;emitted++;
    }}
    float extra;
    let exhausted=!implicit_cell_next(cell,input.start.xyz,input.end.xyz,roots,extra);
    // End-of-sequence must remain exhausted, including an endpoint at one.
    let stable=!implicit_cell_next(cell,input.start.xyz,input.end.xyz,roots,extra);
    results[7u*{count}u+id.x]=sequence;
    results[8u*{count}u+id.x]=float4(float(emitted),exhausted ? 1. : 0.,stable ? 1. : 0.,0.);
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
    let bytes = (cases.len()*144) as u64;
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
    for (i,sequence) in sequences.iter().enumerate() {
        let state=results[8*cases.len()+i];
        assert_eq!(state,[sequence.len() as f32,1.,1.,0.],"ordered root cursor {i}: {state:?}");
        let actual=results[7*cases.len()+i];
        for slot in 0..4 {
            let expected=sequence.get(slot).copied().unwrap_or(-1.);
            assert!((actual[slot]-expected).abs()<2e-5,"ordered root {i}/{slot}: {actual:?}, expected {sequence:?}");
        }
    }
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
        // Case 5 is an algebraic endpoint root of an entirely nonnegative
        // field. It is not a surface bounding any negative region.
        let grid_expected=if i==5 {None} else {expected};
        match grid_expected {
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

#[test]
#[ignore = "requires a real GPU and pinned Slang"]
fn implicit_grid_admits_only_local_negative_boundaries() -> Result<()> {
    struct Case { name:String, dims:[usize;3], values:Vec<f32>, origin:[f32;3], direction:[f32;3], maximum:f32, expected:Option<f32>, valid:bool }
    let mut fixtures=Vec::new();
    let mut add=|name:&str,dims:[usize;3],f:&dyn Fn(usize,usize,usize)->f32,origin,direction,maximum,expected| {
        let mut values=Vec::new();
        for z in 0..dims[2] {for y in 0..dims[1] {for x in 0..dims[0] {values.push(f(x,y,z));}}}
        fixtures.push(Case {name:name.into(),dims,values,origin,direction,maximum,expected,valid:true});
    };
    add("dry zero plane",[3,2,2],&|x,_,_|[1.,0.,1.][x],[-2.,0.,0.],[1.,0.,0.],4.,None);
    add("signed plane",[3,2,2],&|x,_,_|[-1.,0.,1.][x],[0.5,0.,0.],[-1.,0.,0.],1.,Some(0.5));
    add("signed endpoint",[3,2,2],&|x,_,_|[-1.,0.,1.][x],[0.,0.,0.],[1.,0.,0.],0.5,Some(0.));
    add("dry zero edge",[3,3,2],&|x,y,_|(x as f32-1.).abs()+(y as f32-1.).abs(),[-1.,0.,0.],[1.,0.,0.],2.,None);
    add("dry zero vertex",[3,3,3],&|x,y,z|(x as f32-1.).abs()+(y as f32-1.).abs()+(z as f32-1.).abs(),[-1.;3],[1.;3],2.,None);
    add("incident V tangent",[3,3,2],&|x,y,_|(x as f32-1.).abs()+y as f32-1.,[-1.,0.,0.],[1.,0.,0.],2.,Some(1.));
    add("reject first root retain same-cell crossing",[2,2,2],&|x,y,_|x as f32+y as f32-3.*(x*y) as f32,[-0.5,-0.5,0.],[1.,1.,0.],1.,Some(2./3.));
    add("negative quadratic singular",[2,2,2],&|x,y,_|-((x*y) as f32),[-0.5,-0.5,0.],[1.,1.,0.],1.,Some(0.));
    add("negative cubic singular",[2,2,2],&|x,y,z|-((x*y*z) as f32),[-0.5;3],[1.;3],1.,Some(0.));
    add("lower support dominates cubic",[2,2,2],&|x,y,z|(x*y) as f32*(1.-3.*z as f32),[-0.5;3],[1.;3],1.,Some(1./3.));
    add("thin negative sample",[3,2,2],&|x,_,_|[1.,-1e-20,1.][x],[-2.,0.,0.],[1.,0.,0.],4.,Some(2.));
    add("nextafter positive quadratic",[2,2,2],&|x,y,z|z as f32+(x*y) as f32*(f32::from_bits(1f32.to_bits()+1)-1.),[-0.5;3],[1.,1.,0.],1.,None);
    add("nextafter negative quadratic",[2,2,2],&|x,y,z|z as f32+(x*y) as f32*(f32::from_bits(1f32.to_bits()-1)-1.),[-0.5;3],[1.,1.,0.],1.,Some(0.));
    // The DDA-owned cell is finite and already proves negative support. A
    // different incident cell contains NaN: admission must still validate it.
    // Move that late neighbor from a face through an edge to the last vertex
    // orthant; the permutation/direction/scale sweep below covers each axis.
    for boundary_axes in 1..=3 {
        let mut values:Vec<f32>=(0..27).map(|i|(i%3) as f32-1.).collect();
        let late=[2,if boundary_axes>=2 {2} else {0},if boundary_axes>=3 {2} else {0}];
        values[late[0]+3*(late[1]+3*late[2])]=f32::NAN;
        let position=[1.,if boundary_axes>=2 {1.} else {0.5},if boundary_axes>=3 {1.} else {0.5}];
        fixtures.push(Case {
            name:format!("late nonfinite incident neighbor, {boundary_axes} boundary axes"),
            dims:[3;3],values,origin:position.map(|v|(v+0.5)*2./3.-1.),
            direction:[-1.,if boundary_axes>=2 {-1.} else {0.},if boundary_axes>=3 {-1.} else {0.}],
            maximum:0.5,expected:None,valid:false,
        });
    }
    let mut cases=Vec::new();
    for fixture in fixtures {
        for axes in [[0,1,2],[1,2,0],[2,0,1]] {
            for reverse in [false,true] {
                for scale in [1e-20f32,1.,1e20] {
                    // Preserve the nextafter-sized coefficient relative to unit terms.
                    if (fixture.name.starts_with("nextafter") || fixture.name.starts_with("thin")) && scale!=1. {continue;}
                    let mut dims=[0;3];let mut origin=[0.;3];let mut direction=[0.;3];
                    for k in 0..3 {dims[axes[k]]=fixture.dims[k];origin[axes[k]]=fixture.origin[k];direction[axes[k]]=fixture.direction[k];}
                    let mut values=vec![0.;fixture.values.len()];
                    for z in 0..fixture.dims[2] {for y in 0..fixture.dims[1] {for x in 0..fixture.dims[0] {
                        let old=[x,y,z];let mut new=[0;3];for k in 0..3 {new[axes[k]]=old[k];}
                        values[new[0]+dims[0]*(new[1]+dims[1]*new[2])]=fixture.values[x+fixture.dims[0]*(y+fixture.dims[1]*z)]*scale;
                    }}}
                    if reverse {for k in 0..3 {origin[k]+=fixture.maximum*direction[k];direction[k]=-direction[k];}}
                    cases.push(Case {name:format!("{} {axes:?} reverse={reverse} scale={scale}",fixture.name),dims,values,origin,direction,maximum:fixture.maximum,expected:fixture.expected.map(|t|if reverse {fixture.maximum-t} else {t}),valid:fixture.valid});
                }
            }
        }
    }
    let renderer=Renderer::new()?;
    // Seven float4s of samples, uint4 dims, and three float4 ray records.
    let mut words=Vec::<u32>::new();
    for case in &cases {
        let mut row=[0u32;44];
        for (i,v) in case.values.iter().enumerate() {row[i]=v.to_bits();}
        for k in 0..3 {row[28+k]=case.dims[k] as u32;row[32+k]=case.origin[k].to_bits();row[36+k]=case.direction[k].to_bits();}
        row[40]=case.maximum.to_bits();words.extend(row);
    }
    let shader=format!(r#"
import implicit_grid;
struct Case {{float4 samples[7];uint4 dims;float4 origin;float4 direction;float4 interval;}};
struct Grid:IImplicitGrid {{
    Case input;bool accelerated;
    uint3 dimensions() {{return input.dims.xyz;}}
    float value(uint3 c) {{uint i=c.x+input.dims.x*(c.y+input.dims.y*c.z);return input.samples[i/4u][i%4u];}}
    uint cell_step(uint3 cell) {{
        if(!accelerated) return 1u;
        float low=1e30,high=-1e30;
        for(uint z=0;z<input.dims.z;z++) for(uint y=0;y<input.dims.y;y++) for(uint x=0;x<input.dims.x;x++) {{float v=value(uint3(x,y,z));low=min(low,v);high=max(high,v);}}
        return low>0. || high<0. ? 8u : 1u;
    }}
}};
[[vk::binding(0,0)]] StructuredBuffer<Case> cases;
[[vk::binding(1,0)]] RWStructuredBuffer<float4> results;
[shader("compute")][numthreads(64,1,1)] void numerical_test(uint3 id:SV_DispatchThreadID) {{
    if(id.x>={count}u) return;
    Grid grid;grid.input=cases[id.x];grid.accelerated=false;
    let a=implicit_grid_hit(grid,0.,grid.input.origin.xyz,grid.input.direction.xyz,0.,grid.input.interval.x);
    grid.accelerated=true;
    let b=implicit_grid_hit(grid,0.,grid.input.origin.xyz,grid.input.direction.xyz,0.,grid.input.interval.x);
    results[id.x]=float4(a.found ? a.distance : -1.,b.found ? b.distance : -1.,(a.valid ? 1. : 0.)+(b.valid ? 1. : 0.),all(isfinite(a.gradient)) && all(isfinite(b.gradient)) ? 1. : 0.);
}}
"#,count=cases.len());
    let entries:Vec<_>=(0..2).map(|binding|pipelines::buffer_entry(binding,wgpu::ShaderStages::COMPUTE,wgpu::BufferBindingType::Storage {read_only:binding==0})).collect();
    let pipeline=medium::test_pipeline(&renderer,&shader,&medium::Native::default(),&entries,(64,1,1))?;
    use wgpu::util::DeviceExt;
    let input=renderer.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("negative boundary cases"),contents:bytemuck::cast_slice(&words),usage:wgpu::BufferUsages::STORAGE});
    let bytes=(cases.len()*16) as u64;
    let output=renderer.device.create_buffer(&wgpu::BufferDescriptor {label:Some("negative boundary results"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
    let read=renderer.device.create_buffer(&wgpu::BufferDescriptor {label:Some("negative boundary readback"),size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let bind=renderer.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[wgpu::BindGroupEntry {binding:0,resource:input.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:output.as_entire_binding()}]});
    let mut encoder=renderer.device.create_command_encoder(&Default::default());
    {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups((cases.len() as u32).div_ceil(64),1,1);}
    encoder.copy_buffer_to_buffer(&output,0,&read,0,bytes);
    let submission=renderer.queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();read.slice(..).map_async(wgpu::MapMode::Read,move |v|{let _=tx.send(v);});
    renderer.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=read.slice(..).get_mapped_range();let actual:&[[f32;4]]=bytemuck::cast_slice(&mapped);
    for (case,value) in cases.iter().zip(actual) {
        assert_eq!(value[2..],[if case.valid {2.} else {0.},1.],"{}: validity/finite gradient {value:?}",case.name);
        for found in &value[..2] {match case.expected {None=>assert_eq!(*found,-1.,"{}: phantom root {value:?}",case.name),Some(t)=>assert!((*found-t).abs()<2e-5,"{}: {value:?}, expected {t}",case.name)}}
    }
    drop(mapped);read.unmap();renderer.errors.check()?;Ok(())
}
