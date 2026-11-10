use super::*;

// Shutter samples are recorded ahead of submission. Every UPDATE must consume
// the compatible build immediately preceding it in queue order, including when
// several recorded buffers originally observed the same submitted build.
#[test]
fn refits_recorded_ahead_of_submission_preserve_query_results() -> anyhow::Result<()> {
    let Some(r) = crate::test_renderer()? else {
        return Ok(());
    };
    let d = &r.device;
    for (deform, native) in [(false, false), (true, false), (false, true), (true, true)] {
        let vertices = init(
            d,
            "refit vertices",
            bytemuck::cast_slice(&[[-1f32, -1., 0.], [1., -1., 0.], [0., 1., 0.]]),
            wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::COPY_DST,
        );
        let indices = init(
            d,
            "immutable refit topology",
            bytemuck::cast_slice(&[0u32, 1, 2]),
            wgpu::BufferUsages::BLAS_INPUT,
        );
        let size = triangles(false, 3, 3);
        let bottom = d.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some("deferred refit BLAS"),
                flags: BUILD_FLAGS,
                update_mode: wgpu::AccelerationStructureUpdateMode::PreferUpdate,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![size.clone()],
            },
        );
        let mut top = d.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("deferred refit TLAS"),
            max_instances: 1,
            flags: BUILD_FLAGS,
            update_mode: wgpu::AccelerationStructureUpdateMode::PreferUpdate,
        });
        let transform = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.];
        top[0] = Some(wgpu::TlasInstance::new(&bottom, transform, 0, 255));
        let entry = wgpu::BlasBuildEntry {
            blas: &bottom,
            geometry: geometry(&size, &vertices, 0, 12, &indices, 0),
        };
        let mut initial = d.create_command_encoder(&Default::default());
        initial.build_acceleration_structures([&entry], [&top]);
        r.queue.submit([initial.finish()]);

        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("deferred refit query"), source: wgpu::ShaderSource::Wgsl(r#"
                enable wgpu_ray_query;
                @group(0) @binding(0) var scene: acceleration_structure;
                @group(0) @binding(1) var<storage, read_write> output: u32;
                @compute @workgroup_size(1) fn main() {
                    var query: ray_query;
                    rayQueryInitialize(&query, scene, RayDesc(0u, 255u, 0., 10., vec3(0.,0.,1.), vec3(0.,0.,-1.)));
                    rayQueryProceed(&query);
                    let hit = rayQueryGetCommittedIntersection(&query);
                    output = select(hit.instance_custom_data + 1u, 0u, hit.kind == RAY_QUERY_INTERSECTION_NONE);
                }
            "#.into()),
        });
        let group_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                layout_entries(0, wgpu::ShaderStages::COMPUTE)[0],
                pipelines::buffer_entry(
                    1,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ),
            ],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&group_layout)],
            immediate_size: 0,
        });
        let pipeline = r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let output = d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 48,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &group_layout,
            entries: &[
                crate::entry(0, top.as_binding()),
                crate::entry(1, output.as_entire_binding()),
            ],
        });
        let mut expected = Vec::new();
        for batch in 0..3 {
            let mut pending = Vec::new();
            for sample in 0..4 {
                let i = batch * 4 + sample;
                let x = if deform && i % 2 == 1 { 4. } else { 0. };
                let source = init(
                    d,
                    "sample vertices",
                    bytemuck::cast_slice(&[[-1f32 + x, -1., 0.], [1. + x, -1., 0.], [x, 1., 0.]]),
                    wgpu::BufferUsages::COPY_SRC,
                );
                top[0] = Some(wgpu::TlasInstance::new(&bottom, transform, i, 255));
                let mut encoder = d.create_command_encoder(&Default::default());
                if deform {
                    encoder.copy_buffer_to_buffer(&source, 0, &vertices, 0, 36);
                    encoder.build_acceleration_structures([&entry], std::iter::empty());
                    // Also cover a compatible rebuild within one recording.
                    if sample == 1 {
                        encoder.build_acceleration_structures([&entry], std::iter::empty());
                    }
                }
                if native {
                    let (_, bytes) = bottom.native_tlas_instance(i, 255).expect("native TLAS template");
                    let descriptors = init(d, "native refit descriptors", &bytes, wgpu::BufferUsages::TLAS_INPUT);
                    // Alternate forced full builds with refits, including buffers
                    // recorded before earlier builds have reached submission.
                    unsafe { encoder.build_tlas_from_buffer(&top, &descriptors, 0, 1, &[&bottom], sample == 1); }
                } else {
                    encoder.build_acceleration_structures(std::iter::empty(), [&top]);
                }
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &readback, u64::from(i) * 4, 4);
                pending.push(encoder.finish());
                expected.push(if x == 0. { i + 1 } else { 0 });
            }
            if batch == 1 {
                for command in pending {
                    r.queue.submit([command]);
                }
            } else {
                r.queue.submit(pending);
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read, .., move |result| {
            tx.send(result).unwrap();
        });
        r.wait(None)?;
        rx.recv()??;
        let mapped = readback.get_mapped_range(..);
        assert_eq!(
            bytemuck::cast_slice::<u8, u32>(&mapped),
            expected,
            "deform={deform}, native={native}"
        );
        drop(mapped);
        readback.unmap();
        r.errors.check()?;

        // Compatibility still matters: changing topology or instance count
        // after recording an UPDATE must reject it before native submission.
        let mut stale = d.create_command_encoder(&Default::default());
        let mut replacement = d.create_command_encoder(&Default::default());
        if deform {
            stale.build_acceleration_structures([&entry], std::iter::empty());
            let other_indices = init(
                d,
                "replacement topology",
                bytemuck::cast_slice(&[0u32, 2, 1]),
                wgpu::BufferUsages::BLAS_INPUT,
            );
            let other = wgpu::BlasBuildEntry {
                blas: &bottom,
                geometry: geometry(&size, &vertices, 0, 12, &other_indices, 0),
            };
            replacement.build_acceleration_structures([&other], std::iter::empty());
        } else {
            stale.build_acceleration_structures(std::iter::empty(), [&top]);
            top[0] = None;
            replacement.build_acceleration_structures(std::iter::empty(), [&top]);
        }
        let stale = stale.finish();
        r.queue.submit([replacement.finish()]);

        // A later failed submission must not replace the actual source's
        // compatibility metadata with its merely validated build of A.
        let mut restore = d.create_command_encoder(&Default::default());
        if deform {
            restore.build_acceleration_structures([&entry], std::iter::empty());
        } else {
            top[0] = Some(wgpu::TlasInstance::new(&bottom, transform, 0, 255));
            restore.build_acceleration_structures(std::iter::empty(), [&top]);
        }
        let mapped_source = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("force submission failure after AS validation"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        let destination = d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut invalid = d.create_command_encoder(&Default::default());
        invalid.copy_buffer_to_buffer(&mapped_source, 0, &destination, 0, 4);
        let scope = d.push_error_scope(wgpu::ErrorFilter::Validation);
        r.queue.submit([restore.finish(), invalid.finish()]);
        assert!(pollster::block_on(scope.pop()).is_some());
        mapped_source.unmap();

        let scope = d.push_error_scope(wgpu::ErrorFilter::Validation);
        r.queue.submit([stale]);
        let error =
            pollster::block_on(scope.pop()).expect("incompatible update source must be rejected");
        assert!(
            error.to_string().contains("update source changed"),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn tlas_motion_budget_rebuilds_and_resets() {
    let mut motion = 0;
    assert!(!rebuild_tlas(&mut motion, 0, 100, false));
    assert!(!rebuild_tlas(&mut motion, 10, 100, false));
    assert!(!rebuild_tlas(&mut motion, 10, 100, false));
    assert!(rebuild_tlas(&mut motion, 5, 100, false));
    assert_eq!(motion, 0);
    assert!(!rebuild_tlas(&mut motion, 1, 100, false));
    assert!(rebuild_tlas(&mut motion, 0, 100, true));
    assert_eq!(motion, 0);
    assert!(rebuild_tlas(&mut motion, 100, 100, false));
}
