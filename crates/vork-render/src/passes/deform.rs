//! One dispatch evaluates current and previous skinning into slot-private streams.
use crate::*;
pub(crate) const LAYOUT: [wgpu::BindGroupLayoutEntry; 7] = {
    let mut entries = [wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }; 7];
    let mut i = 0;
    while i < entries.len() {
        entries[i].binding = i as u32;
        i += 1;
    }
    entries[4].ty = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: false },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    entries[6].ty = entries[4].ty;
    entries
};

pub(crate) fn pipeline(
    r: &Renderer,
    module: &std::sync::OnceLock<wgpu::ShaderModule>,
) -> wgpu::ComputePipeline {
    pipeline_for(r,module,&native::DEFORM_EVALUATE)
}

fn pipeline_for(r: &Renderer, module: &std::sync::OnceLock<wgpu::ShaderModule>, shader: &native::Compiled) -> wgpu::ComputePipeline {
    let bind = r
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("deformation"),
            entries: &LAYOUT,
        });
    let layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("deformation"),
            bind_group_layouts: &[Some(&bind)],
            immediate_size: 0,
        });
    r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("deformation"),
        layout: Some(&layout),
        module: module.get_or_init(|| native::shader_module(&r.device, shader)),
        entry_point: Some("evaluate"),
        compilation_options: Default::default(),
        cache: r.cache.as_ref(),
    })
}

#[cfg(test)]
#[test]
fn slang_layout() {
    native::tests::check(&native::DEFORM_EVALUATE, &[&LAYOUT]);
    native::tests::check(&native::DEFORM_CHAIN_EVALUATE, &[&LAYOUT]);
}

// Reserved header lanes carry exact integer bits; Metal cannot query buffer lengths.
pub(crate) fn header(deform: &Deform) -> [f32; 4] {
    let count = deform.source.size() / std::mem::size_of::<Vertex>() as u64;
    let sets = deform.influences.size() / std::mem::size_of::<Influence>() as u64 / count;
    let targets = deform.morphs.size() / (count * constants::MORPH_FLOATS as u64 * 4);
    [
        f32::from_bits(count as u32),
        f32::from_bits(sets as u32),
        deform.joints as f32,
        f32::from_bits(targets as u32),
    ]
}

/// Each deformation leader's byte offset in the frame slot's deform pool, and
/// the pool size. Offsets satisfy both storage-binding alignment (ray hits bind
/// each range as a hit buffer) and whole DeformedVertex units (BLAS builds).
pub(crate) fn pool_layout(draws: &[install::Draw], limits: &wgpu::Limits) -> (Vec<Option<u64>>, u64) {
    pool_layout_for(draws,limits,None)
}

fn pool_layout_for(draws: &[install::Draw], limits: &wgpu::Limits, leaders: Option<&[usize]>) -> (Vec<Option<u64>>, u64) {
    let stride = std::mem::size_of::<DeformedVertex>() as u64;
    let storage = u64::from(limits.min_storage_buffer_offset_alignment);
    let gcd = |mut a: u64, mut b: u64| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let alignment = storage / gcd(storage, stride) * stride;
    let mut offsets: Vec<Option<u64>> = Vec::with_capacity(draws.len());
    let mut size = 0u64;
    for (index,draw) in draws.iter().enumerate() {
        let Some(a) = draw.deform.as_ref() else {
            offsets.push(None);
            continue;
        };
        let leader=leaders.map_or(a.leader,|leaders|leaders[index]);
        if leader < offsets.len() {
            offsets.push(offsets[leader]);
            continue;
        }
        let offset = size.next_multiple_of(alignment);
        size = offset + u64::from(a.count) * stride;
        offsets.push(Some(offset));
    }
    (offsets, size)
}

#[derive(Clone)]
pub(crate) struct Chained {
    source: crate::geometry_stage_source::Source,
    pipeline: wgpu::ComputePipeline,
    pub bounds: crate::geometry_stage_source::Bounds,
}

pub(crate) fn resources(s: &InstalledScene, r: &Renderer, inputs: &[Option<EvaluatedGeometry>]) -> Result<Vec<Option<FrameDeform>>> {
    let d = &r.device;
    let has_inputs=s.draws.iter().zip(inputs).any(|(draw,input)|draw.deform.is_some() && input.is_some());
    let leaders=has_inputs.then(|| {
        let mut found=std::collections::HashMap::new();
        s.draws.iter().enumerate().map(|(index,draw)| {
            let Some(deform)=&draw.deform else {return index;};
            let input=inputs.get(index).and_then(Option::as_ref).map(|input| (
                input.output.buffer.clone(),input.output.offset,input.output.size,
                input.previous_output.buffer.clone(),input.previous_output.offset,input.previous_output.size,
            ));
            *found.entry((deform.job,input)).or_insert(index)
        }).collect::<Vec<_>>()
    });
    let (offsets, pool_size) = if let Some(leaders)=&leaders {pool_layout_for(&s.draws,&d.limits(),Some(leaders))} else {pool_layout(&s.draws,&d.limits())};
    let chained_pipeline=has_inputs.then(||pipeline_for(r,&std::sync::OnceLock::new(),&native::DEFORM_CHAIN_EVALUATE));
    // Raster vertex streams, BLAS builds and ray hits all read the one pool.
    let pool = (pool_size > 0).then(|| {
        crate::gpu_memory::buffer(d, &wgpu::BufferDescriptor {
            label: Some("frame-private deformation"),
            size: pool_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | crate::rt::geometry_usage(s.rays.is_some()),
            mapped_at_creation: false,
        })
    });
    let mut palettes = std::collections::HashMap::new();
    let mut frames: Vec<Option<FrameDeform>> = Vec::with_capacity(s.draws.len());
    for (index, draw) in s.draws.iter().enumerate() {
        let Some(a) = draw.deform.as_ref() else {
            frames.push(None);
            continue;
        };
        let leader=leaders.as_ref().map_or(a.leader,|leaders|leaders[index]);
        if leader < index {
            frames.push(frames[leader].clone());
            continue;
        }
        ensure!(
            leader == index,
            "deformation leader must precede its aliases"
        );
        let pipeline = s
            .pipelines
            .deform_pipeline
            .get()
            .context("incomplete installed deformation plan")?;
        let buffer = |label, size, usage| {
            crate::gpu_memory::buffer(&d, &wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let output = || {
            buffer(
                "frame-private previous deformation",
                a.count as u64 * 2 * std::mem::size_of::<DeformedVertex>() as u64,
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::VERTEX
                    | crate::rt::geometry_usage(s.rays.is_some()),
            )
        };
        let current = crate::geometry::VertexStream {
            buffer: pool.clone().context("deform pool")?,
            offset: offsets[index].context("deform pool range")?,
            size: a.count as u64 * std::mem::size_of::<DeformedVertex>() as u64,
        };
        let previous = output();
        let (params, previous_params) = palettes
            .entry((
                a.native.as_ref().map(|binding| binding.mesh),
                a.asset,
                a.node,
                a.skin,
                a.targets,
                a.joints,
                // Header counts are geometry-specific even when the joint palette is shared.
                a.source.size(),
                a.influences.size(),
                a.morphs.size(),
                s.roots[draw.object as usize].time_offset.to_bits(),
            ))
            .or_insert_with(|| {
                let palette = || {
                    buffer(
                        "frame skin palette",
                        ((4 + a.joints * 16 + a.targets).div_ceil(4) * 16 * 2) as u64,
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    )
                };
                (palette(), palette())
            })
            .clone();
        let chained=inputs.get(index).and_then(Option::as_ref).map(|input| {
            ensure!(a.cpu.is_none(),"a deformation stage after another stage cannot yet include subdivision; separate subdivision from deformation");
            Ok(Chained {
                source:crate::geometry_stage_source::Source::new(r,&a.source,input,a.count)?,
                pipeline:chained_pipeline.as_ref().context("chained deformation pipeline")?.clone(),
                bounds:crate::geometry_stage_source::Bounds::new(r,&current,a.count),
            })
        }).transpose()?;
        let pipeline=chained.as_ref().map_or(pipeline,|chained|&chained.pipeline);
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("current and previous deformation"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, chained.as_ref().map_or(&a.source,|chained|&chained.source.buffer).as_entire_binding()),
                entry(1, a.influences.as_entire_binding()),
                entry(2, a.morphs.as_entire_binding()),
                entry(3, params.as_entire_binding()),
                entry(4, current.binding()),
                entry(5, previous_params.as_entire_binding()),
                entry(6, previous.as_entire_binding()),
            ],
        });
        frames.push(Some(FrameDeform {
            leader,
            chained,
            output: current,
            previous_output: previous,
            params,
            previous_params,
            bind,
        }));
    }
    Ok(frames)
}

/// An explicitly ordered deformation consumes its predecessor before dispatch.
/// The legacy fused prefix keeps the original dispatch path below.
pub(crate) fn dispatch_chained(encoder:&mut wgpu::CommandEncoder,s:&InstalledScene,skin:&[Option<FrameDeform>]) {
    for (index,frame) in skin.iter().enumerate().filter_map(|(i,f)|f.as_ref().map(|f|(i,f))) {
        if frame.leader==index {if let Some(chained)=&frame.chained {chained.source.encode(encoder);}}
    }
    let Some(default)=s.pipelines.deform_pipeline.get() else {return;};
    let mut pass=encoder.begin_compute_pass(&Default::default());
    for (index,(draw,frame)) in s.draws.iter().zip(skin).enumerate() {
        if let (Some(deform),Some(frame))=(&draw.deform,frame) {
            if frame.leader!=index {continue;}
            pass.set_pipeline(frame.chained.as_ref().map_or(default,|chained|&chained.pipeline));
            pass.set_bind_group(0,&frame.bind,&[]);
            pass.dispatch_workgroups(deform.count.div_ceil(constants::DEFORM_GROUP),3,1);
        }
    }
}

pub(crate) fn dispatch(encoder:&mut wgpu::CommandEncoder,s:&InstalledScene,skin:&[Option<FrameDeform>],queries:Option<&wgpu::QuerySet>) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("current and previous skinning + morphs"),
        timestamp_writes: queries.map(|queries|wgpu::ComputePassTimestampWrites {
            query_set: queries,
            beginning_of_pass_write_index: Some(timing::query("deform")),
            end_of_pass_write_index: Some(timing::query("deform") + 1),
        }),
    });
    let Some(pipeline) = s.pipelines.deform_pipeline.get() else {
        return;
    };
    pass.set_pipeline(pipeline);
    for (index, (draw, frame)) in s.draws.iter().zip(skin).enumerate() {
        if let (Some(a), Some(f)) = (&draw.deform, frame) {
            if a.leader != index {
                continue;
            }
            pass.set_bind_group(0, &f.bind, &[]);
            pass.dispatch_workgroups(a.count.div_ceil(constants::DEFORM_GROUP), 3, 1);
        }
    }
}

#[cfg(test)]
#[test]
#[ignore = "requires real GPU and pinned Slang"]
fn generated_normals_follow_evaluated_faces_at_all_poses() -> Result<()> {
    let r=Renderer::new()?;
    let pipeline=pipeline(&r,&std::sync::OnceLock::new());
    // Two split triangles: one position-only morph, one independently skinned
    // corner. A third triangle has deliberately authored smooth normals.
    let mut vertices=Vec::new();
    let mut influences=Vec::new();
    let mut morphs=vec![0.;9*constants::MORPH_FLOATS];
    for triangle in 0..3 {
        for (corner,p) in [[0.,0.,0.],[1.,0.,0.],[0.,1.,0.]].into_iter().enumerate() {
            vertices.push(Vertex {POSITION:p,NORMAL:[0.,0.,1.],TEXCOORD_0:[0.;2],TEXCOORD_1:[0.;2],
                pad:[if triangle<2 {1.} else {0.},0.],COLOR_0:[1.;4],TANGENT:[0.;4]});
            influences.push(Influence {JOINTS_0:[if triangle==1 && corner==2 {1} else {0},0,0,0],WEIGHTS_0:[1.,0.,0.,0.]});
            if triangle!=1 && corner==2 {morphs[(triangle*3+corner)*constants::MORPH_FLOATS+2]=1.;}
        }
    }
    let deform=crate::geometry::deformation(&r,None,0,None,&vertices,influences,morphs,2,1,1)?;
    let params=|weight:f32,shift:f32| {
        let mut p=header(&deform).to_vec();
        p.extend(Mat4::IDENTITY.to_cols_array());
        p.extend(Mat4::from_translation(Vec3::Z*shift).to_cols_array());
        p.extend([weight,0.,0.,0.]);p
    };
    let current=init(&r.device,"current test pose",bytemuck::cast_slice(&params(1.,1.)),wgpu::BufferUsages::STORAGE);
    let mut history=params(0.,0.);history.extend(params(0.5,-0.5));
    let previous=init(&r.device,"historical test poses",bytemuck::cast_slice(&history),wgpu::BufferUsages::STORAGE);
    let size=vertices.len() as u64*std::mem::size_of::<DeformedVertex>() as u64;
    let buffer=|label,size,usage|r.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,usage,mapped_at_creation:false});
    let usage=wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC;
    let output=buffer("test output",size,usage);
    let previous_output=buffer("test previous output",size*2,usage);
    let read=buffer("test readback",size*3,wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ);
    let bindings=[&deform.source,&deform.influences,&deform.morphs,&current,&output,&previous,&previous_output];
    let entries:Vec<_>=bindings.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry{binding:i as u32,resource:b.as_entire_binding()}).collect();
    let group=r.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("test deform"),layout:&pipeline.get_bind_group_layout(0),entries:&entries});
    let mut encoder=r.device.create_command_encoder(&Default::default());
    {
        let mut pass=encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);pass.set_bind_group(0,&group,&[]);pass.dispatch_workgroups(1,3,1);
    }
    encoder.copy_buffer_to_buffer(&output,0,&read,0,size);
    encoder.copy_buffer_to_buffer(&previous_output,0,&read,size,size*2);
    let submission=r.queue.submit([encoder.finish()]);
    let(tx,rx)=std::sync::mpsc::channel();read.map_async(wgpu::MapMode::Read,..,move|v|{let _=tx.send(v);});
    r.device.poll(wgpu::PollType::Wait{submission_index:Some(submission),timeout:None})?;rx.recv()??;
    let mapped=read.get_mapped_range(..);let floats:&[f32]=bytemuck::cast_slice(&mapped);
    let stride=std::mem::size_of::<DeformedVertex>()/4;
    for pose in 0..3 {for triangle in 0..3 {
        let at=|corner:usize|if pose==0 {(triangle*3+corner)*stride} else {vertices.len()*stride+(triangle*3+corner)*stride*2+(pose-1)*stride};
        let position=|corner|Vec3::from_slice(&floats[at(corner)..at(corner)+3]);
        let expected=if triangle==2 {Vec3::Z} else {(position(1)-position(0)).cross(position(2)-position(0)).normalize()};
        for corner in 0..3 {
            let actual=Vec3::from_slice(&floats[at(corner)+3..at(corner)+6]);
            assert!(actual.abs_diff_eq(expected,1e-6),"pose={pose} triangle={triangle} corner={corner}: {actual:?} != {expected:?}");
        }
    }}
    Ok(())
}
