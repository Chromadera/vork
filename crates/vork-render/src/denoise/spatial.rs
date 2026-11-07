//! Small stateless edge-aware reconstruction for the final raster budget.
use crate::*;

pub(crate) fn entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    (0..6)
        .map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            count: None,
            ty: wgpu::BindingType::Buffer {
                ty: if binding == 0 {
                    wgpu::BufferBindingType::Uniform
                } else {
                    wgpu::BufferBindingType::Storage {
                        read_only: matches!(binding, 2 | 3),
                    }
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        })
        .collect()
}
pub(crate) struct Pipelines {
    layout: wgpu::BindGroupLayout,
    prepare: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    finish: wgpu::ComputePipeline,
}
impl Pipelines {
    fn new(r: &Renderer) -> Self {
        let layout = r
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("final reconstruction"),
                entries: &entries(),
            });
        let pipeline_layout = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("final reconstruction"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = |shader: &crate::native::Compiled| {
            let module = crate::native::shader_module(&r.device, shader);
            r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(shader.name),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(shader.entry),
                compilation_options: Default::default(),
                cache: r.cache.as_ref(),
            })
        };
        let prepare = pipeline(&crate::native::RECONSTRUCT_PREPARE);
        let filter = pipeline(&crate::native::RECONSTRUCT_FILTER);
        let finish = pipeline(&crate::native::RECONSTRUCT_FINISH);
        Self {
            layout,
            prepare,
            filter,
            finish,
        }
    }
}
pub(crate) struct Stage {
    pipelines: std::sync::Arc<Pipelines>,
    bindings: Vec<wgpu::BindGroup>,
    groups: [u32; 3],
}
impl Stage {
    pub(crate) fn new(
        r: &Renderer,
        accumulation: &wgpu::Buffer,
        output: &wgpu::Buffer,
        guides: &wgpu::Buffer,
        size: [u32; 2],
        samples: u32,
    ) -> Result<Self> {
        let pipelines = r
            .pipelines
            .reconstruction
            .get_or_init(|| std::sync::Arc::new(Pipelines::new(r)))
            .clone();
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 16;
        let scratch = std::array::from_fn::<_, 2, _>(|_| {
            crate::gpu_memory::buffer(
                &r.device,
                &wgpu::BufferDescriptor {
                    label: Some("final reconstruction scratch"),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                },
            )
        });
        let filtered_guides = crate::gpu_memory::buffer(
            &r.device,
            &wgpu::BufferDescriptor {
                label: Some("final reconstruction guides"),
                size: bytes * 2,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
        );
        let mut bindings = Vec::new();
        let steps = vork_slang::registry::RECONSTRUCT_STEPS;
        for pass in 0..steps + 2 {
            let params = crate::gpu_memory::buffer_init(
                &r.device,
                "final reconstruction parameters",
                bytemuck::cast_slice(&[
                    size[0],
                    size[1],
                    samples,
                    if pass == 0 { 0 } else { 1 << (pass - 1) },
                ]),
                wgpu::BufferUsages::UNIFORM,
            );
            let source = &scratch[((pass + 1) % 2) as usize];
            let destination = if pass == steps + 1 {
                output
            } else {
                &scratch[(pass % 2) as usize]
            };
            bindings.push(r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("final reconstruction"),
                layout: &pipelines.layout,
                entries: &[
                    entry(0, params.as_entire_binding()),
                    entry(1, accumulation.as_entire_binding()),
                    entry(2, guides.as_entire_binding()),
                    entry(3, source.as_entire_binding()),
                    entry(4, destination.as_entire_binding()),
                    entry(5, filtered_guides.as_entire_binding()),
                ],
            }));
        }
        Ok(Self {
            pipelines,
            bindings,
            groups: [size[0].div_ceil(8), size[1].div_ceil(8), 1],
        })
    }
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, queries: Option<&wgpu::QuerySet>) {
        for (i, bind) in self.bindings.iter().enumerate() {
            let pipeline = if i == 0 {
                &self.pipelines.prepare
            } else if i == self.bindings.len() - 1 {
                &self.pipelines.finish
            } else {
                &self.pipelines.filter
            };
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("final spatial reconstruction"),
                timestamp_writes: queries.and_then(|query_set| {
                    let first=i==0;let last=i==self.bindings.len()-1;
                    (first || last).then_some(wgpu::ComputePassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index:first.then_some(timing::query("reconstruction")),
                        end_of_pass_write_index:last.then_some(timing::query("reconstruction")+1),
                    })
                }),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(self.groups[0], self.groups[1], 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::util::DeviceExt;
    #[test]
    #[ignore = "requires GPU and pinned Slang; runs in the main GPU contracts"]
    #[cfg(vork_private_tests)]
    fn raster_hdr_silhouette_without_surface_guides_stays_unfiltered() -> Result<()> {
        let r = crate::test_renderer()?.expect("GPU required");
        let mut value: serde_json::Value = serde_json::from_str(include_str!("../../../../examples/compositor-bloom.json"))?;
        value["render"]["resolution"] = serde_json::json!([128, 72]);
        value["render"]["compositor"] = serde_json::json!([]);
        value["render"]["settings"] = serde_json::json!({"integrator":"raster","samples":32,"denoiser":null,"reconstruct":false});
        let render = |value: &serde_json::Value| -> Result<Vec<[f32; 4]>> {
            let scene = r.install(vork_scene::v2::from_test_value(value.clone())?, std::path::Path::new("."))?;
            let mut slot = scene.create_frame_slot(&r, crate::ReadbackPrecision::Float)?;
            slot.submit_outputs(&scene, &r, 0., crate::ReadbackOutputs {
                float: crate::FloatOutputs { color: true, ..Default::default() }, ..Default::default()
            })?;
            Ok(slot.collect(&r)?.linear_rgba.unwrap())
        };
        let raw = render(&value)?;
        value["render"]["settings"]["reconstruct"] = true.into();
        let filtered = render(&value)?;
        assert!(raw.iter().any(|p| p[0] > 50.), "fixture must contain HDR emission");
        // Surface pixels are protected specular receivers. Film coverage also
        // puts nonzero radiance just outside their representative control sample.
        // Those pixels have no guide and must not seed coarse background taps.
        for (a, b) in raw.iter().zip(&filtered) {
            for c in 0..4 {
                assert!((a[c] - b[c]).abs() < 1e-5, "HDR silhouette changed: {a:?} -> {b:?}");
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "requires GPU and pinned Slang; runs in the main GPU contracts"]
    fn path_reconstruction_preserves_detail_behind_glass() -> Result<()> {
        let r = crate::test_renderer()?.expect("GPU required");
        let mut scene = serde_json::json!({
            "version":2,
            "render":{"resolution":[24,16],"settings":{
                "integrator":"path","samples":16,"denoiser":null,"reconstruct":false}},
            "geometry":{"plane":{"attributes":{"POSITION":{"type":"vec3","data":[
                [-10,-10,0],[10,-10,0],[10,10,0],[-10,-10,0],[10,10,0],[-10,10,0]]}}}},
            "materials":{
                "glass":{"inputs":{"transmission_weight":1,"specular_ior":1,
                    "specular_roughness":0,"geometry_thin_walled":true}},
                "dark":{"inputs":{"base_weight":0,"specular_weight":0,
                    "emission_color":[1,1,1],"emission_luminance":0.1}},
                "bright":{"inputs":{"base_weight":0,"specular_weight":0,
                    "emission_color":[1,1,1],"emission_luminance":1}}
            },
            "nodes":[
                {"type":"camera","translation":[0,0,3],"target":[0,0,0]},
                {"type":"mesh","geometry":"plane","materials":["glass"],"translation":[0,0,1]},
                {"type":"mesh","geometry":"plane","materials":["dark"],"translation":[-10,0,0]},
                {"type":"mesh","geometry":"plane","materials":["bright"],"translation":[10,0,0]}
            ]
        });
        let render = |value: &serde_json::Value| -> Result<Vec<[f32;4]>> {
            let installed=r.install(vork_scene::v2::from_test_value(value.clone())?,std::path::Path::new("."))?;
            let mut slot=installed.create_frame_slot(&r,crate::ReadbackPrecision::Float)?;
            slot.submit_outputs(&installed,&r,0.,crate::ReadbackOutputs {
                float:crate::FloatOutputs {color:true,..Default::default()},..Default::default()
            })?;
            Ok(slot.collect(&r)?.linear_rgba.expect("float colour"))
        };
        let raw=render(&scene)?;
        scene["render"]["settings"]["reconstruct"]=true.into();
        let filtered=render(&scene)?;
        let range=raw.iter().map(|p|p[0]).fold((f32::INFINITY,f32::NEG_INFINITY),
            |(lo,hi),v|(lo.min(v),hi.max(v)));
        assert!(range.1-range.0>0.5,"test must see both rear emission levels: {range:?}");
        for (a,b) in raw.iter().zip(&filtered) {
            for c in 0..4 { assert!((a[c]-b[c]).abs()<1e-5,"glass detail changed: {a:?} -> {b:?}"); }
        }
        Ok(())
    }
    #[test]
    #[ignore = "requires GPU and pinned Slang; runs in the main GPU contracts"]
    fn reconstruction_preserves_dc_alpha_and_edges_while_reducing_noise() -> Result<()> {
        let Some(r) = crate::test_renderer()? else {
            anyhow::bail!("GPU required");
        };
        let size = [32, 16];
        let count = (size[0] * size[1]) as usize;
        let samples = 16u32;
        let filter = |pixels: &[[f32; 4]], guides: &[[f32; 4]]| -> Result<Vec<[f32; 4]>> {
            let make = |data: &[[f32; 4]]| {
                r.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(data),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    })
            };
            let accumulated: Vec<_> = pixels
                .iter()
                .map(|p| p.map(|v| v * samples as f32))
                .collect();
            let accumulation = make(&accumulated);
            let guide = make(guides);
            let output = make(&vec![[0.; 4]; count]);
            let stage = Stage::new(&r, &accumulation, &output, &guide, size, samples)?;
            let mut encoder = r.device.create_command_encoder(&Default::default());
            stage.encode(&mut encoder,None);
            let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (count * 16) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, (count * 16) as u64);
            let submission = r.queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback.map_async(wgpu::MapMode::Read, .., move |v| {
                let _ = tx.send(v);
            });
            r.wait(Some(submission))?;
            rx.recv()??;
            let data = readback.get_mapped_range(..);
            let result = data
                .chunks_exact(16)
                .map(|p| {
                    std::array::from_fn(|c| {
                        f32::from_ne_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())
                    })
                })
                .collect();
            drop(data);
            readback.unmap();
            Ok(result)
        };
        let guides: Vec<_> = (0..count)
            .flat_map(|i| {
                let left = i % 32 < 16;
                [
                    [
                        if left { 16. } else { 0. },
                        0.,
                        if left { 0. } else { 16. },
                        16.,
                    ],
                    [0., 0., 16., 1.],
                ]
            })
            .collect();
        let dc: Vec<_> = (0..count)
            .map(|i| {
                if i % 32 < 16 {
                    [0.25, 0.125, 0.0625, 0.5]
                } else {
                    [0., 0., 0., 0.]
                }
            })
            .collect();
        let clean = filter(&dc, &guides)?;
        for (got, want) in clean.iter().zip(&dc) {
            for c in 0..4 {
                assert!(
                    (got[c] - want[c]).abs() < 1e-6,
                    "constant/alpha/edge: {got:?} != {want:?}"
                );
            }
        }
        let noisy: Vec<_> = (0..count)
            .map(|i| {
                let v = if (i % 32 + i / 32) % 2 == 0 { 0.7 } else { 0.3 };
                [v, v, v, 1.]
            })
            .collect();
        // A checker reflected/transmitted by one smooth surface has identical
        // primary albedo and normal. Its transported guide must preserve it.
        let protected: Vec<_>=(0..count).flat_map(|_|[[8.,8.,8.,16.],[0.,0.,16.,-1.]]).collect();
        let detailed=filter(&noisy,&protected)?;
        assert_eq!(detailed,noisy,"transported detail must not enter the spatial filter");
        // Equal-looking primary materials on different objects are distinct
        // receivers even when their colour and normal guides agree.
        let separate: Vec<_>=(0..count).flat_map(|i|[[8.,8.,8.,16.],[0.,0.,16.,if i%32<16 {1.} else {2.}]]).collect();
        let edge: Vec<_>=(0..count).map(|i|if i%32<16 {[0.25,0.25,0.25,1.]} else {[0.75,0.75,0.75,1.]}).collect();
        let preserved=filter(&edge,&separate)?;
        for (got,want) in preserved.iter().zip(&edge) {
            assert!((got[0]-want[0]).abs()<1e-6,"receiver edge: {got:?} != {want:?}");
        }
        // A film-filtered silhouette can carry HDR colour without a surface
        // guide. Filtering these samples under background identity zero makes
        // sparse coarse-scale halos, even though actual receivers are protected.
        let unguided = vec![[0.; 4]; count * 2];
        let silhouette: Vec<_> = (0..count).map(|i| {
            let v = if i % 32 == 16 && i / 32 == 8 {100.} else {0.};
            [v, v, v, 1.]
        }).collect();
        assert_eq!(filter(&silhouette, &unguided)?, silhouette,
            "radiance without a surface guide must remain exact");
        let smooth = filter(&noisy, &guides)?;
        let rms = (smooth.iter().map(|p| (p[0] - 0.5).powi(2)).sum::<f32>() / count as f32).sqrt();
        assert!(rms < 0.04, "noise residual {rms}");
        let mean = smooth.iter().map(|p| p[0]).sum::<f32>() / count as f32;
        assert!((mean - 0.5).abs() < 0.005, "mean energy {mean}");
        // Nonuniform HDR signal, including image borders, fractional coverage,
        // material edges and protected/no-guide pixels. The global RGB sum is
        // an independent conservation invariant, not another filter reference.
        let mut input=Vec::<[f32;4]>::new();let mut varied_guides=Vec::<[f32;4]>::new();
        for i in 0..count {
            let coverage=[0.25,0.5,0.75,1.][i%4];
            let rgb=if i%23==0 {[8.,2.,0.4]} else {[0.02*(i%7) as f32,0.03,0.01]};
            input.push([rgb[0]*coverage,rgb[1]*coverage,rgb[2]*coverage,coverage]);
            let identity=if i%19==0 {-1.} else if i%17==0 {0.} else if i%5==0 {2.} else {1.};
            let a=coverage*samples as f32;let albedo=if i%11==0 {0.8} else {0.2};
            varied_guides.extend([[albedo*a,albedo*a,albedo*a,a],[0.,a,0.,identity]]);
        }
        let result=filter(&input,&varied_guides)?;
        for (i,p) in result.iter().enumerate() {
            assert!(p.iter().all(|x|x.is_finite()));
            assert!((p[3]-input[i][3]).abs()<1e-7);
            for c in 0..3 {
                assert!(p[c]>=-1e-7 && p[c]/p[3]<=[8.,2.,0.4][c]+1e-5,"new radiance extremum: {p:?}");
                if varied_guides[2*i+1][3]<=0. {assert!((p[c]-input[i][c]).abs()<1e-6,"protected pixel changed");}
            }
        }
        for c in 0..3 {
            let before:f64=input.iter().map(|p|f64::from(p[c])).sum();
            let after:f64=result.iter().map(|p|f64::from(p[c])).sum();
            eprintln!("reconstruction RGB[{c}] {before} -> {after}");
            assert!((after-before).abs()<before*2e-6,"reconstruction changed image energy");
        }
        Ok(())
    }
}
