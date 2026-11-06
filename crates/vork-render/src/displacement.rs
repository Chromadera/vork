//! The material geometry stage. Topology is installed once; GPU evaluation owns
//! both motion poses and produces the bounds consumed by visibility planning.
use crate::*;
use std::sync::Arc;

pub(crate) struct Mesh {
    source: wgpu::Buffer,
    attributes: wgpu::Buffer,
    topology: wgpu::Buffer,
    count: u32,
    timed: bool,
    point_vertices: u32,
}
impl Mesh {
    pub(crate) fn new(
        r: &Renderer,
        geometry: &geometry::Geometry,
        attributes: &[String],
        timed: bool,
    ) -> Result<Arc<Self>> {
        ensure!(
            geometry.indices.len().is_multiple_of(3),
            "material displacement requires triangle geometry"
        );
        let count = u32::try_from(geometry.vertices.len())?;
        let source: Vec<_> = geometry
            .vertices
            .iter()
            .map(|v| DeformedVertex {
                POSITION: v.POSITION,
                NORMAL: v.NORMAL,
                TANGENT: v.TANGENT,
            })
            .collect();
        let point_vertices = if geometry.point_centers.is_empty() { 0 } else { (geometry.vertices.len() / geometry.point_centers.len()) as u32 };
        let mut values = Vec::new();
        for (i, v) in geometry.vertices.iter().enumerate().filter(|(i, _)| point_vertices == 0 || i % point_vertices as usize == 0) {
            values.push([v.TEXCOORD_0[0], v.TEXCOORD_0[1], 0., 0.]);
            for name in attributes {
                values.push(
                    geometry
                        .attributes
                        .get(name)
                        .map_or([0.; 4], |values| values[i]),
                );
            }
        }
        let topology = if point_vertices != 0 {
            geometry.point_centers.iter().flatten().map(|v| v.to_bits()).collect()
        } else {
            let mut incidents = vec![Vec::new(); count as usize];
            for (face, triangle) in geometry.indices.chunks_exact(3).enumerate() {
                for &vertex in triangle {
                    incidents
                        .get_mut(vertex as usize)
                        .context("displacement triangle index outside geometry")?
                        .push(face as u32);
                }
            }
            let offset_base = u32::try_from(geometry.indices.len() + 1)?;
            let mut topology = vec![offset_base];
            topology.extend_from_slice(&geometry.indices);
            let mut offset = 0u32;
            for faces in &incidents {
                topology.push(offset);
                offset += faces.len() as u32;
            }
            topology.push(offset);
            for faces in incidents {
                topology.extend(faces);
            }
            topology
        };
        let storage =
            |label, bytes: &[u8]| init(&r.device, label, bytes, wgpu::BufferUsages::STORAGE);
        Ok(Arc::new(Self {
            source: storage("undisplaced geometry", bytemuck::cast_slice(&source)),
            attributes: storage("displacement attributes", bytemuck::cast_slice(&values)),
            topology: storage("displacement topology", bytemuck::cast_slice(&topology)),
            count,
            timed,
            point_vertices,
        }))
    }
}

pub(crate) struct Pipeline {
    initialize: wgpu::ComputePipeline,
    evaluate: wgpu::ComputePipeline,
    finish: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}
impl Pipeline {
    pub(crate) fn new(
        r: &Renderer,
        native_programs: &programs::Native,
        materials: &wgpu::BindGroupLayout,
        attributes: usize,
    ) -> Result<Self> {
        ensure!(
            r.device.limits().max_storage_buffers_per_shader_stage >= 11,
            "material displacement requires eleven storage buffer bindings"
        );
        let entries: Vec<_> = (0..9)
            .map(|binding| {
                pipelines::buffer_entry(
                    binding,
                    wgpu::ShaderStages::COMPUTE,
                    if binding == 0 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding < 5,
                        }
                    },
                )
            })
            .collect();
        let layout = r
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("material displacement"),
                entries: &entries,
            });
        let pipeline_layout = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("material displacement"),
                bind_group_layouts: &[None, Some(materials), Some(&layout)],
                immediate_size: 0,
            });
        let source = include_str!("../shaders/displacement.slangh");
        let metal = r.adapter_info.backend == wgpu::Backend::Metal;
        let (_, target, flags) = if metal {
            forward_variant::METAL_TARGET
        } else {
            forward_variant::SPIRV_TARGET
        };
        let define = format!("-DVORK_ATTRIBUTES={attributes}");
        let includes: Vec<_> = native_programs
            .include
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        let args: Vec<_> = ["-target", target]
            .into_iter()
            .chain(["-fp-mode", "precise"])
            .chain(forward_variant::SLANG_FLAGS.iter().copied())
            .chain(flags.iter().copied())
            .chain([
                define.as_str(),
                "-DVORK_MATERIAL_RECORDS_REGISTER=t0",
                "-DVORK_MATERIAL_WORDS_REGISTER=t3",
                "-DVORK_MATERIAL_TEXTURES_REGISTER=b1",
                "-DVORK_MATERIAL_SAMPLERS_REGISTER=b2",
                "-DVORK_SHADER_ERRORS_REGISTER=u4",
            ])
            .chain(includes.iter().flat_map(|p| ["-I", p.as_str()]))
            .collect();
        let mut modules = native::SOURCES.to_vec();
        modules.extend(
            native_programs
                .modules
                .iter()
                .map(|(n, s)| (n.as_str(), s.as_str())),
        );
        modules.push(("displacement", source));
        let compile = |entry: &str| -> Result<wgpu::ComputePipeline> {
            let mut key = blake3::Hasher::new();
            for part in [target, entry, define.as_str()] {
                key.update(part.as_bytes());
            }
            for argument in &args {
                key.update(argument.as_bytes());
            }
            for (name, text) in &modules {
                key.update(name.as_bytes());
                key.update(text.as_bytes());
            }
            let path = r
                .disk_cache
                .path("material-displacement", 1, key.finalize());
            let code = if let Some(code) = disk_cache::read(&path) {
                code
            } else {
                let start=std::time::Instant::now();
                let code = vork_slang::Compiler::discover().compile_modules(
                    &modules,
                    "displacement",
                    &[(entry, "compute")],
                    &args,
                )?;
                crate::timing::creation(&r.device,entry,"Slang",start.elapsed().as_secs_f64()*1000.,0.);
                let _ = disk_cache::write(&path, &code);
                code
            };
            let threads = if entry == "initialize" {
                (1, 1, 1)
            } else {
                (constants::DEFORM_GROUP, 1, 1)
            };
            let shader = if metal {
                native::module(
                    &r.device,
                    "material displacement",
                    None,
                    Some(&String::from_utf8(code)?),
                    threads,
                )
            } else {
                native::module(
                    &r.device,
                    "material displacement",
                    Some(&code),
                    None,
                    threads,
                )
            };
            Ok(r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("material displacement"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: r.cache.as_ref(),
            }))
        };
        Ok(Self {
            initialize: compile("initialize")?,
            evaluate: compile("evaluate")?,
            finish: compile("finish")?,
            layout,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Parameters {
    count: u32,
    material: u32,
    time: f32,
    previous_time: f32,
    point_vertices: u32,
    pose: u32,
    dispatch_width: u32,
    reserved: u32,
    object_header: [u32; 4],
    to_stream: Mat4,
    from_stream: Mat4,
}

/// Deterministic frame-private page addresses, also used by installed hit records.
/// Endpoint pages contain two interleaved vertices per current vertex.
pub(crate) fn page_layout(
    draws: &[install::Draw],
    limits: &wgpu::Limits,
) -> Result<(Vec<Option<(usize, u64)>>, Vec<(usize, u64)>)> {
    let alignment = u64::from(limits.min_storage_buffer_offset_alignment) * 5;
    let capacity =
        (u64::from(limits.max_storage_buffer_binding_size) / 2).min(limits.max_buffer_size / 2);
    let mut ranges = Vec::with_capacity(draws.len());
    let mut pages: Vec<(usize, u64)> = Vec::new();
    for (i, draw) in draws.iter().enumerate() {
        let Some(mesh) = &draw.displacement else {
            ranges.push(None);
            continue;
        };
        if let Some(leader) = draw.displacement_leader.filter(|&leader| leader < i) {
            ranges.push(ranges[leader]);
            continue;
        }
        let bytes = u64::from(mesh.count) * 40;
        ensure!(
            bytes <= capacity,
            "displaced geometry needs {} endpoint bytes; adapter page limit is {}",
            bytes * 2,
            capacity * 2
        );
        if pages
            .last()
            .is_none_or(|(_, size)| size.next_multiple_of(alignment) + bytes > capacity)
        {
            pages.push((i, 0));
        }
        let (leader, size) = pages.last_mut().unwrap();
        let offset = size.next_multiple_of(alignment);
        ranges.push(Some((*leader, offset)));
        *size = offset + bytes;
    }
    Ok((ranges, pages))
}

pub(crate) fn resources(
    r: &Renderer,
    s: &InstalledScene,
    skin: &[Option<FrameDeform>],
) -> Result<Vec<Option<Frame>>> {
    let Some(pipeline) = &s.displacement else {
        return Ok(Vec::new());
    };
    let (ranges, pages) = page_layout(&s.draws, &r.device.limits())?;
    let mut streams = std::collections::HashMap::new();
    for (leader, size) in pages {
        let make = |label, size, rays| {
            gpu_memory::buffer(
                &r.device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::COPY_SRC
                        | rt::geometry_usage(rays),
                    mapped_at_creation: false,
                },
            )
        };
        streams.insert(
            leader,
            (
                make("evaluated geometry page", size, s.rays.is_some()),
                make("evaluated endpoint page", size * 2, false),
            ),
        );
    }
    Ok(s.draws
        .iter()
        .enumerate()
        .map(|(i, draw)| {
            let mesh = draw.displacement.as_ref()?;
            if draw.displacement_leader.is_some_and(|leader| leader != i) {
                return None;
            }
            let (leader, offset) = ranges[i]?;
            let (current, previous) = &streams[&leader];
            let size = u64::from(mesh.count) * 40;
            Some(Frame::new(
                r,
                pipeline,
                mesh,
                skin[i].as_ref(),
                EvaluatedGeometry {
                    attributes: None,
                    output: geometry::VertexStream {
                        buffer: current.clone(),
                        offset,
                        size,
                    },
                    previous_output: geometry::VertexStream {
                        buffer: previous.clone(),
                        offset: offset * 2,
                        size: size * 2,
                    },
                },
            ))
        })
        .collect())
}

pub(crate) struct Frame {
    pub(crate) geometry: EvaluatedGeometry,
    bind: [wgpu::BindGroup; 3],
    params: [wgpu::Buffer; 3],
    bounds: wgpu::Buffer,
    readback: wgpu::Buffer,
    count: u32,
    groups: [u32; 2],
    timed: bool,
    point_vertices: u32,
}
impl Frame {
    pub(crate) fn append_inputs(&self, changes: &values::ChangeTracker, inputs: &mut Vec<u8>) {
        for params in &self.params {
            changes.append_buffer_inputs(params, inputs);
        }
    }
    pub(crate) fn dirty(&self, changes: &values::ChangeTracker) -> bool {
        self.params.iter().any(|buffer| {
            changes
                .uploads
                .iter()
                .any(|(destination, ..)| destination == buffer)
        })
    }
    pub(crate) fn new(
        r: &Renderer,
        pipeline: &Pipeline,
        mesh: &Mesh,
        skin: Option<&FrameDeform>,
        geometry: EvaluatedGeometry,
    ) -> Self {
        let buffer = |label, size, usage| {
            gpu_memory::buffer(
                &r.device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage,
                    mapped_at_creation: false,
                },
            )
        };
        let params = std::array::from_fn(|_| {
            buffer(
                "geometry evaluation parameters",
                std::mem::size_of::<Parameters>() as u64,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            )
        });
        let positions = buffer(
            "displaced positions",
            u64::from(mesh.count / mesh.point_vertices.max(1)) * 3 * 4 * 3,
            wgpu::BufferUsages::STORAGE,
        );
        let bounds = buffer(
            "evaluated bounds",
            32,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = buffer(
            "evaluated bounds readback",
            32,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let bind = std::array::from_fn(|pose| {
            r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("material geometry evaluation"),
                layout: &pipeline.layout,
                entries: &[
                    entry(0, params[pose].as_entire_binding()),
                    entry(
                        1,
                        skin.map_or_else(
                            || mesh.source.as_entire_binding(),
                            |s| s.output.binding(),
                        ),
                    ),
                    entry(
                        2,
                        skin.map_or(&mesh.source, |s| &s.previous_output)
                            .as_entire_binding(),
                    ),
                    entry(3, mesh.attributes.as_entire_binding()),
                    entry(4, mesh.topology.as_entire_binding()),
                    entry(5, positions.as_entire_binding()),
                    entry(6, geometry.output.binding()),
                    entry(7, geometry.previous_output.binding()),
                    entry(8, bounds.as_entire_binding()),
                ],
            })
        });
        let count = mesh.count.div_ceil(constants::DEFORM_GROUP);
        let width = count.min(r.device.limits().max_compute_workgroups_per_dimension);
        let groups = [width, count.div_ceil(width)];
        Self {
            groups,
            geometry,
            bind,
            params,
            bounds,
            readback,
            count: mesh.count,
            timed: mesh.timed,
            point_vertices: mesh.point_vertices,
        }
    }
}

/// GPU geometry precedes CPU visibility planning. Only the reduced bounds cross
/// this boundary; every triangle consumer retains the GPU position/frame stream.
pub(crate) fn evaluate(
    r: &Renderer,
    s: &InstalledScene,
    skin: &[Option<FrameDeform>],
    frames: &[Option<Frame>],
    materials: Option<&materials::FrameMaterials>,
    objects: &[Object],
    spaces: &[[Mat4; 3]],
    cached_bounds: &[Option<(Vec3, Vec3)>],
    queries: &wgpu::QuerySet,
    changes: &mut values::ChangeTracker,
    time: f32,
    previous_time: f32,
    next_time: f32,
) -> Result<Vec<Option<(Vec3, Vec3)>>> {
    let Some(pipeline) = &s.displacement else {
        return Ok(Vec::new());
    };
    let mut bounds = vec![None; s.draws.len()];
    for (i, frame) in frames.iter().enumerate() {
        let Some(frame) = frame else { continue };
        let offset = s.roots[s.draws[i].object as usize].time_offset;
        for pose in 0..3 {
            let to_stream = s.draws[i]
                .deform
                .as_ref()
                .map_or(Mat4::IDENTITY, |d| spaces[d.job][pose]);
            let from_stream = crate::inverse_or_identity(to_stream);
            ensure!(
                from_stream.is_finite(),
                "material displacement requires an invertible mesh-local frame for draw {i}"
            );
            let params = Parameters {
                count: frame.count,
                material: objects[s.draws[i].object as usize].material_index[0],
                time: if frame.timed { time + offset } else { 0. },
                previous_time: if frame.timed {
                    if pose==2 {next_time+offset} else {previous_time + offset}
                } else {
                    0.
                },
                point_vertices: frame.point_vertices,
                pose: pose as u32,
                dispatch_width: frame.groups[0] * constants::DEFORM_GROUP,
                reserved: u32::from(s.draws[i].deform.is_some()),
                object_header: [objects[s.draws[i].object as usize].metadata[2], 0, 0, 0],
                to_stream,
                from_stream,
            };
            changes.upload(&frame.params[pose], 0, bytemuck::bytes_of(&params));
        }
    }
    let uploaded = |buffer: &wgpu::Buffer| {
        changes
            .uploads
            .iter()
            .any(|(destination, ..)| destination == buffer)
    };
    let active: Vec<bool> = frames
        .iter()
        .enumerate()
        .map(|(i, frame)| {
            frame.as_ref().is_some_and(|frame| {
                cached_bounds[i].is_none()
                    || frame.params.iter().any(uploaded)
                    || skin[i].as_ref().is_some_and(|skin| {
                        uploaded(&skin.params) || uploaded(&skin.previous_params)
                    })
            })
        })
        .collect();
    for (i, frame) in frames.iter().enumerate() {
        if frame.is_some() && !active[i] {
            bounds[i] = cached_bounds[i];
        }
    }
    let mut encoder = r
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("evaluated geometry before visibility"),
        });
    if !changes.bytes.is_empty() {
        let upload = init(
            &r.device,
            "geometry value uploads",
            &changes.bytes,
            wgpu::BufferUsages::COPY_SRC,
        );
        for (destination, offset, source, size) in &changes.uploads {
            encoder.copy_buffer_to_buffer(&upload, *source, destination, *offset, *size);
        }
    }
    drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("begin geometry evaluation"),
        timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
            query_set: queries,
            beginning_of_pass_write_index: Some(timing::query("deform")),
            end_of_pass_write_index: None,
        }),
    }));
    passes::deform::dispatch(&mut encoder, s, skin, None);
    for stage in [&pipeline.initialize, &pipeline.evaluate, &pipeline.finish] {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("material geometry stage"),
            timestamp_writes: std::ptr::eq(stage, &pipeline.finish).then_some(
                wgpu::ComputePassTimestampWrites {
                    query_set: queries,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(timing::query("deform") + 1),
                },
            ),
        });
        pass.set_pipeline(stage);
        for pose in 0..if std::ptr::eq(stage, &pipeline.initialize) {
            1
        } else {
            3
        } {
            pass.set_bind_group(
                1,
                materials.map_or_else(|| s.material_table.bind(), |m| &m.bind),
                &[],
            );
            for (_, frame) in frames
                .iter()
                .enumerate()
                .filter(|(i, _)| active[*i])
                .filter_map(|(i, f)| f.as_ref().map(|f| (i, f)))
            {
                pass.set_bind_group(2, &frame.bind[pose], &[]);
                let groups = if std::ptr::eq(stage, &pipeline.initialize) {
                    [1, 1]
                } else if std::ptr::eq(stage, &pipeline.evaluate) && frame.point_vertices != 0 {
                    let count = (frame.count / frame.point_vertices).div_ceil(constants::DEFORM_GROUP);
                    [count.min(frame.groups[0]), count.div_ceil(frame.groups[0])]
                } else {
                    frame.groups
                };
                pass.dispatch_workgroups(groups[0], groups[1], 1);
            }
        }
    }
    for (_, frame) in frames
        .iter()
        .enumerate()
        .filter(|(i, _)| active[*i])
        .filter_map(|(i, f)| f.as_ref().map(|f| (i, f)))
    {
        encoder.copy_buffer_to_buffer(&frame.bounds, 0, &frame.readback, 0, 32);
    }
    let submission = crate::command_finish::Segments::submit(&r.queue, encoder);
    r.errors.check()?;
    let (send, receive) = std::sync::mpsc::channel();
    for (_, frame) in frames
        .iter()
        .enumerate()
        .filter(|(i, _)| active[*i])
        .filter_map(|(i, f)| f.as_ref().map(|f| (i, f)))
    {
        let send = send.clone();
        frame
            .readback
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = send.send(result);
            });
    }
    if !active.iter().any(|active| *active) {
        for (i, draw) in s.draws.iter().enumerate() {
            if let Some(leader) = draw.displacement_leader { bounds[i] = bounds[leader]; }
        }
        return Ok(bounds);
    }
    r.wait(Some(submission))?;
    let decode = |key: u32| {
        f32::from_bits(if key & 0x80000000 != 0 {
            key ^ 0x80000000
        } else {
            !key
        })
    };
    let mut invalid = None;
    for (i, frame) in frames.iter().enumerate() {
        let Some(frame) = frame else { continue };
        if !active[i] {
            continue;
        }
        receive.recv()??;
        let bytes = frame.readback.get_mapped_range(..);
        let words: &[u32] = bytemuck::cast_slice(&bytes);
        let valid = words[6] == 0;
        let minimum = Vec3::new(decode(words[0]), decode(words[1]), decode(words[2]));
        let maximum = Vec3::new(decode(words[3]), decode(words[4]), decode(words[5]));
        drop(bytes);
        frame.readback.unmap();
        if valid && minimum.is_finite() && maximum.is_finite() {
            bounds[i] = Some((minimum, maximum));
        } else {
            invalid.get_or_insert(i);
        }
    }
    if let Some(i) = invalid {
        anyhow::bail!("material displacement produced nonfinite geometry for draw {i}");
    }
    for (i, draw) in s.draws.iter().enumerate() {
        if let Some(leader) = draw.displacement_leader { bounds[i] = bounds[leader]; }
    }
    Ok(bounds)
}

#[cfg(all(test, vork_private_tests))]
include!("displacement_tests_private.rs");
