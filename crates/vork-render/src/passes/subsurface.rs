//! Opaque Burley diffusion: the decoupled shade captures each pixel's diffusing
//! slots (shaders/decoupled.slang), a compute filter diffuses them, and a
//! correction replaces its unfiltered diffusion in the resolved colour and ambient.
use crate::*;

pub(crate) struct Resources {
    composite: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    /// The resolved colour and ambient the correction adds to.
    resolved: wgpu::TextureView,
    resolved_ambient: wgpu::TextureView,
    receiver: Option<(wgpu::RenderPipeline, wgpu::BindGroup, wgpu::TextureView)>,
}

/// Explicit native Slang layout, also inspected by native::tests.
pub(crate) fn entries(composite: bool) -> Vec<wgpu::BindGroupLayoutEntry> {
    use wgpu::{BindingType, ShaderStages, TextureSampleType};
    let stage = if composite {
        ShaderStages::FRAGMENT
    } else {
        ShaderStages::COMPUTE
    };
    let mut entries = Vec::new();
    if composite {
        for binding in [0, 1] {
            let mut e = pipelines::texture_entry(
                binding,
                TextureSampleType::Float { filterable: false },
                false,
            );
            e.visibility = stage;
            entries.push(e);
        }
    } else {
        entries.push(pipelines::buffer_entry(
            0,
            stage,
            wgpu::BufferBindingType::Uniform,
        ));
        for binding in [1, 2] {
            entries.push(pipelines::buffer_entry(
                binding,
                stage,
                wgpu::BufferBindingType::Storage { read_only: true },
            ));
        }
        for binding in [3, 4] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: stage,
                ty: BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            });
        }
        entries.push(pipelines::buffer_entry(5, stage, wgpu::BufferBindingType::Uniform));
        entries.push(pipelines::buffer_entry(6, stage, wgpu::BufferBindingType::Storage {read_only:false}));
    }
    entries
}
impl Resources {
    pub(crate) fn new(
        s: &InstalledScene,
        r: &Renderer,
        size: [u32; 2],
        samples: u32,
        uniform: &wgpu::Buffer,
        resolved: &wgpu::TextureView,
        resolved_ambient: &wgpu::TextureView,
        slots: &wgpu::Buffer,
        records: &wgpu::Buffer,
        receiver: Option<&wgpu::TextureView>,
        recorder: &mut graph::Recorder,
    ) -> Result<Self> {
        let d = &r.device;
        let [w, h] = size;
        let target = |name, format, usage| {
            s.transients
                .target(d, name, w, h, 1, format, usage)
                .create_view(&Default::default())
        };
        let correction = target(
            "subsurface correction",
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let ambient_correction = target(
            "subsurface ambient correction",
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (pipeline, composite) = pipelines(r, s.pipelines.raster_samples);
        let compute_layout = pipeline.get_bind_group_layout(0);
        let composite_layout = composite.get_bind_group_layout(0);
        let ranges = diffusion_ranges(samples)?;
        let stride=d.limits().min_uniform_buffer_offset_alignment.max(16) as usize / 4;
        let mut words=vec![0u32;ranges.len()*stride];
        for (range,words) in ranges.iter().zip(words.chunks_exact_mut(stride)) {
            words[..2].copy_from_slice(range);
        }
        let parameters=init(d,"diffusion probe ranges",bytemuck::cast_slice(&words),wgpu::BufferUsages::UNIFORM);
        // Nine FP32 sums per retained receiver only when the integral spans
        // dispatches. Camera and refraction receivers reuse the same scratch.
        let sums=u64::from(vork_slang::renderer_registry::REG_SUBSURFACE_ACCUMULATION_WORDS);
        let bytes=if ranges.len()>1 {records.size()/if receiver.is_some() {2} else {1}/u64::from(constants::VISIBILITY_DIFFUSION_WORDS)*sums} else {sums*4};
        gpu_memory::storage_binding(&d.limits(),"diffusion accumulation",bytes, "reduce resolution or choose --profile std/draft for a lower sample budget")?;
        let accumulation=s.transients.buffer(d,"diffusion accumulation",bytes,wgpu::BufferUsages::STORAGE);
        let dispatch = |offset, color: &wgpu::TextureView, ambient: &wgpu::TextureView| {
            ranges.iter().enumerate().map(|(index,_)| {
                let bind=d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label:Some("Burley diffusion batch"), layout:&compute_layout, entries:&[
                        entry(0,uniform.as_entire_binding()),
                        entry(1,slots.as_entire_binding()),
                        entry(2,wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer:records,offset,size:receiver.map(|_|std::num::NonZeroU64::new(records.size()/2).unwrap()),
                        })),
                        entry(3,wgpu::BindingResource::TextureView(color)),
                        entry(4,wgpu::BindingResource::TextureView(ambient)),
                        entry(5,wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer:&parameters,offset:(index*stride*4) as u64,size:std::num::NonZeroU64::new(16),
                        })),
                        entry(6,accumulation.as_entire_binding()),
                    ],
                });
                graph::Dispatch::new(&pipeline,bind,[w.div_ceil(8),h.div_ceil(8),1])
            }).collect::<Vec<_>>()
        };
        let mut dispatches=dispatch(0,&correction,&ambient_correction);
        let receiver = receiver.map(|source| {
            let color = target(
                "subsurface receiver correction",
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let ambient = target(
                "subsurface receiver ambient correction",
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            dispatches.extend(dispatch(records.size()/2,&color,&ambient));
            let (_, composite) = pipelines_for_receiver(r, s.pipelines.raster_samples, true);
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("opaque receiver diffusion correction"),
                layout: &composite.get_bind_group_layout(0),
                entries: &[
                    entry(0, wgpu::BindingResource::TextureView(&color)),
                    entry(1, wgpu::BindingResource::TextureView(&ambient)),
                ],
            });
            (composite, bind, source.clone())
        });
        recorder.compute(
            graph::Pass::SubsurfaceDiffuse,
            "Burley diffusion",
            "subsurface_diffuse",
            false,
            dispatches,
        );
        let composite_resources = [
            entry(0, wgpu::BindingResource::TextureView(&correction)),
            entry(1, wgpu::BindingResource::TextureView(&ambient_correction)),
        ];
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("subsurface correction"),
            layout: &composite_layout,
            entries: &composite_resources,
        });
        Ok(Self {
            composite,
            bind,
            receiver,
            resolved: resolved.clone(),
            resolved_ambient: resolved_ambient.clone(),
        })
    }
}

pub(crate) fn compose(encoder: &mut wgpu::CommandEncoder, f: &crate::view::ViewState) {
    let Some(s) = &f.subsurface else {
        return;
    };
    let attachment = |view| {
        Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })
    };
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("subsurface correction"),
        color_attachments: &[attachment(&s.resolved), attachment(&s.resolved_ambient)],
        depth_stencil_attachment: None,
        timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
            query_set: &f.queries,
            beginning_of_pass_write_index: Some(timing::query("subsurface_compose")),
            end_of_pass_write_index: s
                .receiver
                .is_none()
                .then(|| timing::query("subsurface_compose") + 1),
        }),
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&s.composite);
    pass.set_bind_group(0, &s.bind, &[]);
    pass.draw(0..3, 0..1);
    drop(pass);
    if let Some((pipeline, bind, source)) = &s.receiver {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("opaque receiver diffusion correction"),
            color_attachments: &[attachment(source), None],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &f.queries,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(timing::query("subsurface_compose") + 1),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub(crate) fn pipelines(
    r: &Renderer,
    samples: u32,
) -> (wgpu::ComputePipeline, wgpu::RenderPipeline) {
    pipelines_for_receiver(r, samples, false)
}
fn pipelines_for_receiver(
    r: &Renderer,
    samples: u32,
    receiver: bool,
) -> (wgpu::ComputePipeline, wgpu::RenderPipeline) {
    let d = &r.device;
    let layout = |composite| {
        d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("subsurface"),
            entries: &entries(composite),
        })
    };
    let compute_layout = layout(false);
    let pipeline_layout = |layout: &wgpu::BindGroupLayout| {
        d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("subsurface"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        })
    };
    let compute = &native::SUBSURFACE_SINGLE_SUBSURFACE_DIFFUSE;
    let module = native::shader_module(d, compute);
    let pipeline = r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Burley diffusion"),
        layout: Some(&pipeline_layout(&compute_layout)),
        module: &module,
        entry_point: Some(compute.entry),
        compilation_options: Default::default(),
        cache: r.cache.as_ref(),
    });
    let composite_layout = layout(true);
    let vs = &native::SUBSURFACE_COMPOSITE_SINGLE_SUBSURFACE_VS;
    let fs = &native::SUBSURFACE_COMPOSITE_SINGLE_SUBSURFACE_COMPOSE;
    let vertex = native::shader_module(d, vs);
    let fragment = native::shader_module(d, fs);
    let additive = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    let composite = r.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("subsurface correction"),
        layout: Some(&pipeline_layout(&composite_layout)),
        vertex: wgpu::VertexState {
            module: &vertex,
            entry_point: Some(vs.entry),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &fragment,
            entry_point: Some(fs.entry),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: if receiver {wgpu::TextureFormat::Rgba16Float} else {wgpu::TextureFormat::Rgba32Float},
                    blend: Some(wgpu::BlendState {
                        color: additive,
                        alpha: additive,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                }),
                (!receiver).then_some(wgpu::ColorTargetState {
                    format: pipelines::resolved_ambient_format(samples),
                    blend: Some(wgpu::BlendState {
                        color: additive,
                        alpha: additive,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                }),
            ],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: r.cache.as_ref(),
    });
    (pipeline, composite)
}

/// Split the complete sequence into dispatches without changing its lattice.
fn diffusion_ranges(samples:u32)->Result<Vec<[u32;2]>> {
    use vork_slang::renderer_registry::{REG_SUBSURFACE_BASE_PROBES,REG_SUBSURFACE_BATCH_PROBES};
    let count=samples.checked_add(REG_SUBSURFACE_BASE_PROBES).context("diffusion probe count overflows u32")?;
    Ok((0..count).step_by(REG_SUBSURFACE_BATCH_PROBES as usize)
        .map(|first|[first,first.saturating_add(REG_SUBSURFACE_BATCH_PROBES).min(count)]).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn diffusion_batches_cover_every_probe_and_partial_tail() {
        use vork_slang::renderer_registry::REG_SUBSURFACE_BASE_PROBES;
        for samples in [1,16,64,192,193,256,1024,4096] {
            let ranges=super::diffusion_ranges(samples).unwrap();
            assert_eq!(ranges.first().unwrap()[0],0);
            assert_eq!(ranges.last().unwrap()[1],samples+REG_SUBSURFACE_BASE_PROBES);
            assert!(ranges.iter().all(|r|r[0]<r[1]));
            assert!(ranges.windows(2).all(|r|r[0][1]==r[1][0]));
        }
        assert!(super::diffusion_ranges(u32::MAX).is_err());
    }
}
