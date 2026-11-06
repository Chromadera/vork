//! Post resources and the compose and resolve steps: MSAA resolve, shutter
//! accumulation, display conversion and packed controls; optional pixel readback.
use crate::*;

pub(crate) struct Resources {
    pub pixels: wgpu::Buffer,
    pub(crate) display_inputs: Option<wgpu::Buffer>,
    pub(crate) compositor_inputs: crate::compositor::FrameInputs,
    pub nonfinite: wgpu::Buffer,
    pub(crate) display_base: Option<u32>,
    pub(crate) denoiser: Option<crate::denoise::Stage>,
    pub(crate) reconstruction: Option<crate::denoise::spatial::Stage>,
}
impl Resources {
    pub(crate) fn new(
        s: &InstalledScene,
        r: &Renderer,
        uniform: &wgpu::Buffer,
        sample_buffers: &[wgpu::Buffer],
        ao_guides: Option<(&wgpu::TextureView, &wgpu::TextureView)>,
        lights: &wgpu::Buffer,
        path_light_tree: Option<&wgpu::Buffer>,
        cluster_config: &wgpu::Buffer,
        cluster_lights: &wgpu::Buffer,
        material_records: &wgpu::Buffer,
        objects: &wgpu::Buffer,
        targets: &crate::view::Targets,
        reflections: &passes::reflections::Resources,
        ao: &passes::ao::Resources,
        output: &wgpu::Texture,
        planes: u32,
        exposure_samples: u32,
        visibility_samples: u32,
        rays: Option<&crate::rt::FrameRays>,
        material: &wgpu::BindGroup,
        shader_errors: Option<&wgpu::Buffer>,
        recorder: &mut crate::graph::Recorder,
        view_desc: &crate::views::ViewDesc,
        compositor: &[vork_scene::v2::CompositeEntry],
        denoiser: Option<&vork_scene::v2::Denoiser>,
        reconstruct: bool,
    ) -> Result<(Self, wgpu::BindGroup)> {
        let resolve_pipeline = s.pipelines.output_resolve(s.plan.effects)
            .get()
            .context("base pipelines installed")?;
        let d = &r.device;
        let (w, h) = (
            targets.resolved.texture().width(),
            targets.resolved.texture().height(),
        );
        let crate::view::Targets {
            post_depth,
            post_ids,
            post_normals,
            post_ambient,
            post_motion,
            resolved,
            ..
        } = targets;
        let ao_view = &ao.output;
        let capture = view_desc.passes == crate::views::PassSet::Image;
        // Captures share the camera output texture binding, but their planar
        // pixels and dispatches belong to their own logical output extent.
        let [output_width, output_height] = view_desc.extent;
        let size = u64::from(output_width) * u64::from(output_height) * u64::from(planes) * 4;
        let buffer = |label: &str, size: u64, usage| s.transients.buffer(d, label, size, usage);
        // Resolve stages unassociated scene-linear colour for the display program.
        let pixels = buffer("packed PNG pixels", size,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let catcher_base=planes - 4 - if !capture {4} else {0};
        passes::path::record(s,r,uniform,lights,path_light_tree.unwrap_or(cluster_lights),cluster_config,cluster_lights,objects,rays,material,resolved,&targets.aov_values,targets,&targets.catcher,&pixels,catcher_base,recorder,view_desc)?;
        let accumulation = buffer(
            "linear sample accumulation",
            if exposure_samples > 1 || !compositor.is_empty() || s.has_catchers || denoiser.is_some() || reconstruct {
                u64::from(w) * u64::from(h) * constants::ACCUMULATION_PIXEL_BYTES
            } else {
                constants::ACCUMULATION_PIXEL_BYTES
            },
            wgpu::BufferUsages::STORAGE
                | if denoiser.is_some() {
                    wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST
                } else {
                    wgpu::BufferUsages::empty()
                },
        );
        let output_view = output.create_view(&Default::default());
        let view_render = match &view_desc.kind {
            crate::views::ViewKind::Image {render,..} => &render.settings,
            _ => &s.render,
        };
        let camera = match &view_desc.kind {
            crate::views::ViewKind::Camera(camera) => *camera,
            crate::views::ViewKind::Image {render,..} => render.camera,
        };
        // Ray generation alone does not change the guide meaning. Only the
        // active view's detector/throughput can invalidate surface RGB guides;
        // an unrelated camera program must not disable them for this view.
        let camera_program=s.camera_programs.iter().find(|p|p.node==camera);
        let use_guides = camera_program
            .is_none_or(|p|p.unit_weight && !p.spectral && p.program.sensor_entry.is_none());
        let spectral=s.spectral || camera_program.is_some_and(|p|p.spectral);
        let guides = (denoiser.is_some() || reconstruct).then(|| targets.denoising_guides.as_ref()
            .context("OIDN guide buffer missing")).transpose()?;
        let denoised = (denoiser.is_some() || reconstruct).then(|| {
            buffer(
                "denoised colour",
                u64::from(w) * u64::from(h) * 16,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            )
        });
        let reconstruction = reconstruct.then(|| crate::denoise::spatial::Stage::new(r, &accumulation, denoised.as_ref().unwrap(), guides.unwrap(), [w,h], exposure_samples)).transpose()?;
        let denoiser = denoiser
            .map(|settings| {
                let mut stage=crate::denoise::Stage::new(
                    r,
                    settings,
                    &accumulation,
                    denoised.as_ref().unwrap(),
                    guides.expect("validated guide buffer"),
                    view_render.background.transparent(),
                    use_guides,
                    [w, h],
                )?;
                stage.spectral=spectral;
                Ok::<_,anyhow::Error>(stage)
            })
            .transpose()?;

        let compositor_inputs = crate::compositor::FrameInputs::new(s, r, compositor);
        let mut post_color = if let Some(denoised) = &denoised {
            denoised.clone()
        } else {
            buffer("unused post colour",constants::ACCUMULATION_PIXEL_BYTES,wgpu::BufferUsages::STORAGE)
        };
        if !compositor.is_empty() || s.has_catchers {
            post_color = crate::compositor::record(
                s,
                r,
                uniform,
                targets,
                &accumulation,
                compositor,
                &compositor_inputs,
                [w, h],
                crate::compositor::Normalization::Frame,
                recorder,
                crate::graph::Pass::Compositor,
            )?;
        }
        let froxels=s.pipelines.resolve_native.as_ref().map(|n|medium::paged_buffers(s,d,"volume froxel transport",n.froxel_bytes([w,h]),n.froxel_pages.max(1),n.froxel_page_elements));
        let stride=d.limits().min_uniform_buffer_offset_alignment.max(32);
        let sample_parameters=s.pipelines.resolve_native.as_ref().map(|_| {
            let mut bytes=vec![0u8;visibility_samples.max(1) as usize*stride as usize];
            for sample in 0..visibility_samples.max(1) {
                let [x,y]=crate::sampling::clip_offset([w,h],sample as usize,visibility_samples.max(1) as usize);
                bytes[sample as usize*stride as usize..sample as usize*stride as usize+16].copy_from_slice(bytemuck::cast_slice(&[sample,x.to_bits(),y.to_bits(),0]));
                let disk=crate::sampling::lens_disk(sample as usize,visibility_samples.max(1) as usize);
                bytes[sample as usize*stride as usize+16..sample as usize*stride as usize+32].copy_from_slice(bytemuck::cast_slice(&[disk.x,disk.y,0.,0.]));
            }
            gpu_memory::buffer_init(d,"volume sample indices",&bytes,wgpu::BufferUsages::UNIFORM)
        });
        let parameter=|sample:u32|wgpu::BindingResource::Buffer(wgpu::BufferBinding {buffer:sample_parameters.as_ref().unwrap(),offset:u64::from(sample)*u64::from(stride),size:std::num::NonZeroU64::new(32)});
        let sample_pages:Vec<_>=sample_buffers.iter().map(|b|wgpu::BufferBinding {buffer:b,offset:0,size:None}).collect();
        let froxel_pages:Vec<_>=froxels.iter().flatten().map(|b|wgpu::BufferBinding {buffer:b,offset:0,size:None}).collect();
        let mut resolve_entries = vec![
                entry(47, targets.nonfinite.as_entire_binding()),
                entry(70, shader_errors.context("radiance diagnostics")?.as_entire_binding()),
                entry(51, wgpu::BindingResource::TextureView(&targets.depth)),
                entry(52, wgpu::BindingResource::TextureView(&targets.ids)),
                entry(53, wgpu::BindingResource::TextureView(&targets.normals)),
                entry(54, wgpu::BindingResource::TextureView(&targets.ambient)),
                entry(55, wgpu::BindingResource::TextureView(&targets.motion)),
                entry(45, post_color.as_entire_binding()),
                entry(24, wgpu::BindingResource::TextureView(&reflections.output)),
                entry(41, wgpu::BindingResource::TextureView(&ao.incident)),
                entry(58, wgpu::BindingResource::TextureView(&ao.receivers)),
                entry(0, wgpu::BindingResource::TextureView(&resolved)),
                entry(1, wgpu::BindingResource::TextureView(&post_depth)),
                entry(2, wgpu::BindingResource::TextureView(&post_ids)),
                entry(4, pixels.as_entire_binding()),
                entry(67, s.sampling.buffer.as_entire_binding()),
                entry(11, targets.diffuse_response.as_entire_binding()),
                // Disabled AO keeps ordinary view placeholders; enabled AO requires guides.
                entry(68, wgpu::BindingResource::TextureView(ao_guides.map_or(post_depth, |g| g.0))),
                entry(69, wgpu::BindingResource::TextureView(ao_guides.map_or(post_normals, |g| g.1))),
                entry(5, uniform.as_entire_binding()),
                entry(6, wgpu::BindingResource::TextureView(&post_normals)),
                entry(7, wgpu::BindingResource::TextureView(post_ambient)),
                entry(8, wgpu::BindingResource::TextureView(&ao_view)),
                entry(9, wgpu::BindingResource::TextureView(&post_motion)),
                entry(10, accumulation.as_entire_binding()),
                entry(12, wgpu::BindingResource::Sampler(&s.environment.sampler)),
                entry(43, material_records.as_entire_binding()),
                entry(44, lights.as_entire_binding()),
                entry(46, objects.as_entire_binding()),
                entry(
                    16,
                    wgpu::BindingResource::TextureView(&s.environment.specular),
                ),
            ];
        if s.pipelines.resolve_native.as_ref().is_some_and(|n|n.program_coverage) {
            resolve_entries.extend([
                entry(13,wgpu::BindingResource::TextureView(&s.environment.brdf)),
                entry(18,wgpu::BindingResource::TextureView(&s.environment.ltc)),
            ]);
        }
        if s.pipelines.resolve_native.is_some() {
            resolve_entries.push(entry(64,cluster_config.as_entire_binding()));
            resolve_entries.push(entry(65,cluster_lights.as_entire_binding()));
            resolve_entries.push(entry(3,parameter(0)));
            if s.pipelines.resolve_native.as_ref().unwrap().ray_shadows {
                resolve_entries.push(entry(59,rays.context("volume ray shadows require TLAS")?.tlas.as_binding()));
                resolve_entries.push(entry(60,s.rays.as_ref().context("volume ray shadows require alpha table")?.alpha.as_entire_binding()));
                resolve_entries.push(entry(71,rays.unwrap().emitter_weights.as_ref().map_or(&s.rays.as_ref().unwrap().alpha,|w|&w.buffer).as_entire_binding()));
            }
        }
        let resolve_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resolve plan"),
            layout: &resolve_pipeline.get_bind_group_layout(0),
            entries: &resolve_entries,
        });
        let output_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("encoder surface"),
            layout: &resolve_pipeline.get_bind_group_layout(1),
            entries: &[entry(0, wgpu::BindingResource::TextureView(&output_view))],
        });
        let pipeline = |p: &OnceLock<wgpu::ComputePipeline>| {
            p.get().cloned().context("incomplete installed post plan")
        };
        let full = [w.div_ceil(8), h.div_ceil(8), 1];
        if let Some(native)=s.pipelines.resolve_native.as_ref().filter(|_|match &view_desc.kind {crate::views::ViewKind::Image {render,..}=>!render.settings.settings.integrator.is_path(),_=>!s.render.settings.integrator.is_path()}) {
            // Mesh descriptors are installed after the shared pipeline registry.
            // Specialize only this scene's lighting traversal to its actual count;
            // resource layouts and the shared registry remain unchanged.
            let mut column_native=native.clone();
            column_native.boundaries=s.medium_boundaries;
            column_native.world_only=s.medium_boundaries==0;
            let froxel_groups=vec![resolve_pipeline.get_bind_group_layout(0),s.pipelines.mat_layout.clone()];
            let hit_layout = native.program_coverage.then(||crate::rt::hits_layout(d));
            let shadow_layout = crate::shadows::volume_layout(d);
            let page_layout = medium::page_layout(d,native);
            let mut froxel_group_refs:Vec<_>=froxel_groups.iter().map(Some).collect();
            froxel_group_refs.push(Some(&shadow_layout));
            froxel_group_refs.push(hit_layout.as_ref());
            debug_assert_eq!(froxel_group_refs.len() as u32,crate::resolve_native::PAGE_GROUP);
            froxel_group_refs.push(Some(&page_layout));
            let pages=d.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("volume pages"),layout:&page_layout,entries:&[
                entry(crate::resolve_native::FROXEL_PAGES,wgpu::BindingResource::BufferArray(&froxel_pages)),
                entry(crate::resolve_native::SAMPLE_PAGES,wgpu::BindingResource::BufferArray(&sample_pages)),
            ]});
            let froxel_layout=d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label:Some("volume froxel layout"),bind_group_layouts:&froxel_group_refs,immediate_size:0,
            });
            let support_pipeline=self::pipeline(r,&s.pipelines.resolve_module,Some(&column_native),"volume_receiver_support",Some(&froxel_layout));
            let lighting_pipeline=self::pipeline(r,&s.pipelines.resolve_module,Some(&column_native),"volume_lighting",Some(&froxel_layout));
            let visibility_world=(native.visibility_lights>0).then(|| self::pipeline(r,&s.pipelines.resolve_module,Some(&column_native),"volume_visibility_world",Some(&froxel_layout)));
            let world_side=vork_slang::registry::MEDIUM_VISIBILITY_SIDE;
            let tile=native.tile.max(1);
            let [cols,rows]=[w.div_ceil(tile),h.div_ceil(tile)];
            let cells=[cols.div_ceil(8),rows.div_ceil(8),if s.medium_boundaries>0 {(native.slices+1).div_ceil(vork_slang::registry::MEDIUM_LIGHTING_DEPTH_BATCH)} else {native.slices+1}];
            let shadow_bindings=|dispatch:crate::graph::Dispatch| {
                let dispatch=dispatch.with_read_table(Some(s.shadows().volume_bind.clone()));
                let dispatch=dispatch.with_material(material).with_pages(pages.clone());
                if native.program_coverage {dispatch.with_hits(rays.unwrap().hits_bind(d))}else{dispatch}
            };
            let apply_pipeline=self::pipeline(r,&s.pipelines.resolve_module,Some(&column_native),"volume_apply",Some(&froxel_layout));
            let mut dispatches=Vec::new();
            // Complete the shared world visibility average before any reader.
            // Camera cells still stream through their single froxel allocation.
            let mut sample_binds=Vec::new();
            for sample in 0..visibility_samples.max(1) {
                let bind=if sample==0 {resolve_bind.clone()} else {
                    resolve_entries.iter_mut().find(|e|e.binding==3).unwrap().resource=parameter(sample);
                    d.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("volume sample"),layout:&resolve_pipeline.get_bind_group_layout(0),entries:&resolve_entries})
                };
                if let Some(pipeline)=&visibility_world {
                    dispatches.push(shadow_bindings(crate::graph::Dispatch::new(pipeline,bind.clone(),[world_side.div_ceil(8),world_side.div_ceil(8),world_side])));
                }
                sample_binds.push(bind);
            }
            for bind in sample_binds {
                dispatches.push(shadow_bindings(crate::graph::Dispatch::new(&support_pipeline,bind.clone(),[cols.div_ceil(8),rows.div_ceil(8),1])));
                dispatches.push(shadow_bindings(crate::graph::Dispatch::new(&lighting_pipeline,bind.clone(),cells)));
                dispatches.push(shadow_bindings(crate::graph::Dispatch::new(&apply_pipeline,bind,full)));
            }
            recorder.compute(crate::graph::Pass::Volume,"stream volume samples","volume",false,dispatches);
        }
        let mut dispatches = Vec::new();
        if !compositor.is_empty() || s.has_catchers || denoiser.is_some() || reconstruct {
            dispatches.push(crate::graph::Dispatch::new(
                &pipeline(&s.pipelines.post_source_pipeline)?,
                resolve_bind.clone(),
                full,
            ));
            recorder.compute(
                crate::graph::Pass::PostSource,
                "resolve scene-linear post source",
                "post_source",
                true,
                std::mem::take(&mut dispatches),
            );
        }
        dispatches.push(
            crate::graph::Dispatch::new(resolve_pipeline, resolve_bind.clone(), full).all_targets(),
        );
        let display_inputs = if capture { None } else { s.display_program.frame_inputs(r) };
        let display_base = (!capture).then_some(planes - 4);
        if !capture {
            let stage = &s.display_program;
            dispatches.push(stage.dispatch(r, uniform, display_inputs.as_ref(), &pixels, shader_errors.context("display diagnostics")?, [output_width,output_height]));
        }
        recorder.compute(
            crate::graph::Pass::Resolve,
            "display conversion + exact controls",
            "resolve",
            true,
            dispatches,
        );
        Ok((Self { pixels, display_inputs, compositor_inputs, denoiser, reconstruction, nonfinite:targets.nonfinite.clone(), display_base }, output_bind))
    }
}

/// Original API bindings; native sources map these to Vulkan's dense indices.
pub(crate) fn layout_entries(entry: &str) -> Vec<wgpu::BindGroupLayoutEntry> {
    use wgpu::{BindingType as T, BufferBindingType as B};
    use crate::resolve_native::Kind;
    let binding_type = |binding| match crate::resolve_native::kind(binding) {
        Kind::Uniform => T::Buffer {
            ty: B::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        Kind::Read | Kind::Write => T::Buffer {
            ty: B::Storage {
                read_only: matches!(crate::resolve_native::kind(binding), Kind::Read),
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        Kind::Sampler => T::Sampler(wgpu::SamplerBindingType::Filtering),
        Kind::Storage => T::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: match binding {
                27 => wgpu::TextureFormat::R32Float,
                32 | 37 => wgpu::TextureFormat::Rg16Float,
                _ => wgpu::TextureFormat::Rgba16Float,
            },
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        Kind::Texture => T::Texture {
            sample_type: if binding == 51 {wgpu::TextureSampleType::Depth} else if matches!(binding, 2 | 52) {
                wgpu::TextureSampleType::Uint
            } else {
                wgpu::TextureSampleType::Float {
                    filterable: matches!(binding, 13 | 16),
                }
            },
            view_dimension: if binding == 24 {wgpu::TextureViewDimension::D2Array} else {
                wgpu::TextureViewDimension::D2
            },
            multisampled: (51..=55).contains(&binding) && entry != "resolve_fused_single",
        },
        Kind::ArgumentTable | Kind::Acceleration => unreachable!("resolve base binding {binding}"),
    };
    crate::resolve_native::bindings(entry)
        .iter()
        .map(|&binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: binding_type(binding),
            count: None,
        })
        .collect()
}
/// Lookup tables used by the shared straight-segment closure evaluator.
pub(crate) fn shadow_closure_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    crate::resolve_native::SHADOW_CLOSURE_BINDINGS.into_iter().map(|binding| wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float {filterable:true},
            view_dimension: wgpu::TextureViewDimension::D2, multisampled:false,
        }, count:None,
    }).collect()
}
pub(crate) fn shaders() -> [&'static crate::native::Compiled; 4] {
    use crate::native::*;
    [
        &RESOLVE_RESOLVE_COLOR,
        &RESOLVE_POST_SOURCE,
        &RESOLVE_RESOLVE_FUSED,
        &RESOLVE_RESOLVE_FUSED_SINGLE,
    ]
}
pub(crate) fn pipeline(
    r: &Renderer,
    source: &str,
    native: Option<&medium::Native>,
    entry: &str,
    installed_layout: Option<&wgpu::PipelineLayout>,
) -> wgpu::ComputePipeline {
    let entry = if entry == "main" {
        "resolve_color"
    } else {
        entry
    };
    let d = &r.device;
    let local_layout = installed_layout.is_none().then(|| {
        let bind = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(entry),
            entries: &layout_entries(entry),
        });
        d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(entry),
            bind_group_layouts: &[Some(&bind)],
            immediate_size: 0,
        })
    });
    let module = if source == RESOLVE || installed_layout.is_none() {
        let shader = shaders()
            .into_iter()
            .find(|s| s.entry == entry)
            .expect("registered resolve entry");
        crate::native::shader_module(d, shader)
    } else {
        let metal = r.adapter_info.backend == wgpu::Backend::Metal;
        let native = native.expect("installed medium modules");
        let code = cached_native(r, source, native, entry, metal).expect("compile native medium resolve");
        crate::native::module(
            d,
            entry,
            (!metal).then_some(code.as_slice()),
            metal.then(|| std::str::from_utf8(&code).expect("resolve MSL")),
            native_threads(entry),
        )
    };
    r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: installed_layout.or(local_layout.as_ref()),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: r.cache.as_ref(),
    })
}

/// The runtime artifact path also drives cross-target setup validation; repeated
/// setup jobs share the same content-addressed native shader cache.
pub(crate) fn cached_native(r: &Renderer, source: &str, native: &medium::Native, entry: &str, metal: bool) -> Result<Vec<u8>> {
    let mut hash = blake3::Hasher::new();
    let volume=matches!(entry,"volume_receiver_support"|"volume_lighting"|"volume_apply"|"volume_visibility_world");
    let selected=source_for_entry(source,native,entry)?;
    hash.update(selected.as_bytes());
    hash.update(include_bytes!("../resolve_native.rs"));
    hash.update(entry.as_bytes());hash.update(&[u8::from(metal),u8::from(native.ray_shadows),u8::from(native.program_coverage)]);
    hash.update(&[u8::from(native.implicit)]);
    if native.implicit {hash.update(b"field-medium-inline-v2");}
    hash.update(crate::forward_variant::NATIVE_OPTIMIZATION.as_bytes());
    hash.update(b"medium-driver-inline-v1");
    hash.update(&native.froxel_pages.to_le_bytes());hash.update(&native.sample_pages.to_le_bytes());
    for (name,text) in crate::native::SOURCES {hash.update(name.as_bytes());hash.update(text.as_bytes());}
    if volume {
        hash.update(&native.froxel_page_elements.to_le_bytes());
        for define in volume_defines(native,metal) {hash.update(define.as_bytes());}
        if metal {hash.update(METAL_VOLUME_MATH.as_bytes());}
    }
    if matches!(entry,"volume_receiver_support"|"volume_lighting"|"volume_apply"|"volume_visibility_world") {
        hash.update(&native.identity);
        for (name,text) in &native.modules {hash.update(name.as_bytes());hash.update(text.as_bytes());}
        for path in &native.includes {hash.update(path.to_string_lossy().as_bytes());}
    }
    let path = r.disk_cache.path("slang-resolve", 2, hash.finalize());
    if let Some(code) = crate::disk_cache::read(&path) { return Ok(code); }
    let code = compile_native(source, native, entry, metal)?;
    let _ = crate::disk_cache::write(&path, &code);
    Ok(code)
}

// Metal passthrough modules need explicit launch dimensions; SPIR-V retains
// the shader's execution mode and ignores this metadata.
pub(crate) fn native_threads(entry: &str) -> (u32,u32,u32) {
    let _=entry;
    (8,8,1)
}

fn source_for_entry(source:&str,native:&medium::Native,entry:&str) -> Result<String> {
    // Only volume entries link transport. Resolve/post keep installed resource
    // numbering without importing point programs or volume accessors.
    Ok(match entry {
        "volume_receiver_support"=>medium::receiver_support_source(native),
        "volume_lighting"|"volume_apply"|"volume_visibility_world"=>native.paged_source(source),
        _=>RESOLVE.to_owned(),
    })
}

// Volume transport carries divergent tails and opaque horizons as IEEE
// infinities (vork_medium: emission*inf, infinite optical depth) and refuses
// non-finite background transport. Metal's default fast math assumes neither
// infinities nor NaNs and loses them (a diverging emissive tail rendered a
// finite colour); relaxed math keeps them and still allows reassociation.
pub(crate) const METAL_VOLUME_MATH: &str = "#pragma METAL fp math_mode(relaxed)\n";

// Raster SPIR-V retains its existing rounding contract. Metal's precise mode
// additionally installs whole-kernel safe-math/contraction pragmas; only sampled
// path consumers need that policy to preserve certified tracking bounds.
fn volume_defines(native: &medium::Native, metal: bool) -> Vec<String> {
    let mut flags=native.defines();
    if !metal {flags.extend(vork_slang::MEDIUM_FP_FLAGS.map(str::to_owned));}
    flags
}

pub(crate) fn compile_native(
    source: &str,
    native: &medium::Native,
    entry: &str,
    metal: bool,
) -> Result<Vec<u8>> {
    let selected=source_for_entry(source,native,entry)?;
    let source=selected.as_str();
    let lighting=matches!(entry,"volume_lighting"|"volume_apply"|"volume_visibility_world");
    let volume=lighting || entry=="volume_receiver_support";
    let layout = crate::resolve_native::Layout {volume: true, rays: native.ray_shadows, shadow_closures: native.program_coverage};
    let mut modules: Vec<(String, String)> = crate::native::SOURCES
        .iter()
        .filter(|(name, _)| !name.starts_with("resolve"))
        .map(|(name, text)| {
            (
                (*name).to_owned(),
                if *name == "vork_medium" || *name == "vork_shadow_visibility" || *name == "frame" {
                    crate::resolve_native::source(text, entry, layout)
                } else {
                    (*text).to_owned()
                },
            )
        })
        .collect();
    modules.push((
        "resolve".into(),
        crate::resolve_native::source(source, entry, layout),
    ));
    if lighting {
        // Table-only transmitting casters install NoPrograms but still need
        // the ABI and std modules used by the medium program itself.
        if !native.modules.iter().any(|(name,_)|name=="vork") {
            modules.push(("vork".into(), programs::abi().module()));
            modules.extend(vork_slang::library_modules().map(|module| (module.name.to_owned(), module.source.to_owned())));
        }
        modules.extend(native.modules.clone());
    }
    let (_, target, flags) = if metal {
        crate::forward_variant::METAL_TARGET
    } else {
        crate::forward_variant::SPIRV_TARGET
    };
    let mut args: Vec<String> = ["-target", target]
        .into_iter()
        .chain(crate::forward_variant::SLANG_FLAGS.iter().copied())
        .chain([crate::forward_variant::NATIVE_OPTIMIZATION])
        .chain(flags.iter().copied())
        .map(str::to_owned)
        .collect();
    if metal && volume {
        let base=crate::resolve_native::material_base(entry,layout);
        args.extend(crate::forward_variant::material_registers(base));
        if native.program_coverage {
            args.push(format!("-DVORK_HIT_REGISTER=b{}", crate::resolve_native::hit_base(entry, layout)));
        }
        let pages=crate::resolve_native::page_base(entry, layout);
        args.push(format!("-DVORK_FROXEL_PAGES_REGISTER=b{pages}"));
        args.push(format!("-DVORK_SAMPLE_PAGES_REGISTER=b{}", pages+1));
    }
    if volume {args.extend(volume_defines(native,metal));args.push("-DVORK_FROXEL".into());}
    if entry=="volume_lighting" {args.push("-DVORK_VOLUME_LIGHTING".into());}
    if lighting {
        // A finite froxel already has one coefficient cell, but its unbounded
        // tail can contain a complete march. Share one RGB light evaluation
        // and select one tail lighting cell while retaining all coefficients.
        args.extend(["-DVORK_MEDIUM_ENVIRONMENT","-DVORK_MEDIUM_SAMPLED_LIGHTING","-DVORK_MEDIUM_JITTERED_LIGHTING"].map(str::to_owned));
        if !native.implicit {args.push("-DVORK_NO_IMPLICIT".into());}
        args.extend(["-DVORK_NATIVE_MEDIUM","-DVORK_MEDIUM_STEPS=1","-DVORK_VOLUME_BINDINGS"].map(str::to_owned));
        if native.program_coverage {args.push("-DVORK_PROGRAM_COVERAGE".into());}
        if native.mesh_sources {args.push("-DVORK_MEDIUM_MESH_SOURCES".into());}
        if native.ray_shadows {args.push("-DVORK_VOLUME_RAY".into());}
        for path in &native.includes {args.push("-I".into());args.push(path.to_string_lossy().into_owned());}
    }
    let refs: Vec<_> = modules
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let mut code = vork_slang::Compiler::discover().compile_modules(
        &refs,
        "resolve",
        &[(entry, "compute")],
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    if !metal {crate::slang_forward::allow_driver_inlining(&mut code)?;}
    if metal && volume {code.splice(0..0, METAL_VOLUME_MATH.bytes());}
    Ok(code)
}
