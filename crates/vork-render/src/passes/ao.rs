//! Retained AO visibility and tile-local separable reconstruction.
use crate::*;
use wgpu::util::DeviceExt;
/// Retain one GI hit-shading specialization over every installed material.
pub(crate) fn ray_pipeline(r: &Renderer, cached: &crate::pipelines::CachedPipelines, features: crate::forward_variant::FeatureMask, probes: bool, mode: &crate::pipelines::VariantCompile) -> Result<wgpu::ComputePipeline> {
    let kernel=cached.drawn_kernel(crate::pipelines::ProgramKernel::Gi { features }, mode,&r.compiler);
    let (module, entry)=cached.program_kernel(kernel)?;
    let layout=crate::rt::gi_layout(&r.device,&cached.mat_layout,probes,cached.resolve_native.is_some());
    let pipeline=r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label:Some("ray-traced diffuse indirect"),layout:Some(&layout),module:&module,entry_point:Some(entry),
        compilation_options:Default::default(),cache:r.cache.as_ref(),
    });
    r.compiler.remember(cached.kernel_content_key(kernel));
    Ok(pipeline)
}

pub(crate) struct Resources {
    pub output: wgpu::TextureView,
    pub incident: wgpu::TextureView,
    pub receivers: wgpu::TextureView,
}
impl Resources {
    pub(crate) fn new(
        s: &InstalledScene,
        d: &wgpu::Device,
        uniform: &wgpu::Buffer,
        guides: Option<(&wgpu::TextureView, &wgpu::TextureView)>,
        extent: [u32; 2],
        ambient: &wgpu::TextureView,
        diffuse_response: &wgpu::Buffer,
        rays: Option<&crate::rt::FrameRays>,
        lights: &wgpu::Buffer,
        objects: &wgpu::Buffer,
        cluster_config: &wgpu::Buffer,
        cluster_lights: &wgpu::Buffer,
        material: &wgpu::BindGroup,
        visibility_samples: u32,
        recorder: &mut crate::graph::Recorder,
    ) -> Result<Self> {
        let [w, h] = extent;
        let divisor = constants::GI_RESOLUTION_DIVISOR;
        let (iw, ih) = if s.plan.effects.indirect {
            (w.div_ceil(divisor), h.div_ceil(divisor))
        } else {
            (1, 1)
        };
        let image = |label, width, height, format| {
            s.transients
                .target(
                    d,
                    label,
                    width,
                    height,
                    1,
                    format,
                    wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                )
                .create_view(&Default::default())
        };
        let (aw, ah) = if s.plan.effects.indirect { (w.div_ceil(divisor), h.div_ceil(divisor)) } else { (1, 1) };
        let raw_ao = image("AO per-sample visibility depth normal", aw, ah, wgpu::TextureFormat::Rgba16Float);
        let incident_image = |label| image(label, iw, ih, wgpu::TextureFormat::Rgba16Float);
        let incident = incident_image("hemisphere per-sample incident radiance");
        // Query selection is shared by both blur axes and every upsampling tap.
        let cell_receivers = image("AO selected receivers", aw, ah, wgpu::TextureFormat::Rgba32Float);
        let mut dispatches = Vec::new();
        if s.plan.effects.indirect {
            let ao_view = image("AO filtered visibility depth normal", aw, ah, wgpu::TextureFormat::Rgba16Float);
            let incident_horizontal = incident_image("hemisphere incident radiance");
            let (receivers, post_normals) = guides.context("AO needs retained slot receivers")?;
            let groups = [
                w.div_ceil(divisor).div_ceil(8),
                h.div_ceil(divisor).div_ceil(8),
                1,
            ];
            let frame_rays = rays.context("diffuse GI needs the scene TLAS")?;
            let medium=s.pipelines.resolve_native.is_some();
            let batch=vork_slang::registry::MEDIUM_GI_RAY_BATCH;
            let batches=if medium {(visibility_samples.max(1)*constants::GI_RAYS_PER_SAMPLE).div_ceil(batch)} else {1};
            let hits=if medium {
                let accumulation=s.transients.buffer(d,"GI medium prepared queries and accumulation",u64::from(aw)*u64::from(ah)*constants::GI_MEDIUM_QUERY_BYTES,wgpu::BufferUsages::STORAGE);
                let geometry=frame_rays.hit_bindings();
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label:Some("GI hit and accumulation buffers"),layout:&crate::rt::gi_read_layout(d,true),
                    entries:&[entry(0,wgpu::BindingResource::BufferArray(&geometry)),entry(1,accumulation.as_entire_binding())],
                })
            } else {frame_rays.hits_bind(d)};
            let mut source_words=vec![0u32;if medium {batches as usize*64} else {4}];
            let topology=s.rays.as_ref().unwrap();
            for i in 0..batches {source_words[i as usize*64..i as usize*64+4].copy_from_slice(&[topology.emitters,topology.implicit[2],topology.implicit[3],0]);}
            if medium {for i in 0..batches {source_words[i as usize*64+4]=i*batch;}}
            let sources=d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label:Some("diffuse source topology"),
                    contents:bytemuck::cast_slice(&source_words),
                    usage:wgpu::BufferUsages::UNIFORM,
            });
            let visibility = {
                let frame_rays = rays.context("ray GI needs the scene TLAS")?;
                let tlas = &frame_rays.tlas;
                let table = &s
                    .rays
                    .as_ref()
                    .context("ray GI needs the scene TLAS")?
                    .alpha;
                (
                    s.gi.as_ref(),
                    "ray GI plan",
                    // rt::gi_entries: the scene globals, then GI's images and rays.
                    { let mut entries=pipelines::ray_globals(s,uniform,objects,lights,cluster_config,cluster_lights);
                      entries.extend([
                        entry(10, wgpu::BindingResource::TextureView(receivers)),
                        entry(11, wgpu::BindingResource::TextureView(post_normals)),
                        entry(12, wgpu::BindingResource::TextureView(&raw_ao)),
                        entry(13, wgpu::BindingResource::TextureView(&incident)),
                        entry(14, tlas.as_binding()),
                        entry(15, table.as_entire_binding()),
                        entry(16, wgpu::BindingResource::TextureView(&cell_receivers)),
                        entry(17,sources.as_entire_binding()),
                        entry(18,frame_rays.emitter_weights.as_ref().map_or(table,|weights|&weights.buffer).as_entire_binding()),
                        entry(19,wgpu::BindingResource::TextureView(ambient)),
                        entry(20,diffuse_response.as_entire_binding()),
                    ]);
                    entries },
                    Some(material),
                )
            };
            let reconstruction = |pipeline, label, vertical| {
                let (output, input, out_incident, in_incident) = if vertical {
                    (&raw_ao, &ao_view, &incident, &incident_horizontal)
                } else {
                    (&ao_view, &raw_ao, &incident_horizontal, &incident)
                };
                (pipeline, label, vec![
                    entry(0, wgpu::BindingResource::TextureView(&cell_receivers)),
                    entry(1, uniform.as_entire_binding()),
                    entry(2, wgpu::BindingResource::TextureView(output)),
                    entry(3, wgpu::BindingResource::TextureView(input)),
                    entry(4, wgpu::BindingResource::TextureView(out_incident)),
                    entry(5, wgpu::BindingResource::TextureView(in_incident)),
                ], None)
            };
            let mut stages=vec![visibility];
            stages.push(reconstruction(s.pipelines.ao_blur_horizontal_pipeline.get(),"AO horizontal blur",false));
            stages.push(reconstruction(s.pipelines.ao_blur_vertical_pipeline.get(),"AO vertical blur",true));
            for (pipeline, label, entries, material) in stages {
              for batch in 0..if material.is_some() {batches} else {1} {
                let mut entries=entries.clone();
                if material.is_some() {
                    entries.iter_mut().find(|e|e.binding==17).unwrap().resource=wgpu::BindingResource::Buffer(wgpu::BufferBinding {buffer:&sources,offset:u64::from(batch)*256,size:std::num::NonZeroU64::new(if medium {32} else {16})});
                }
                let pipeline = pipeline.context("incomplete installed AO plan")?;
                let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &entries,
                });
                let mut dispatch = crate::graph::Dispatch::new(pipeline, bind, groups);
                if material.is_some() {
                    // Ray GI hits read the irradiance volume for further bounces.
                    let probes = (s.plan.effects.ray_indirect).then(|| s.probes.as_ref().map(|p| p.read_bind(d))).flatten();
                    dispatch = dispatch.with_hits(hits.clone()).with_read_table(probes);
                }
                dispatches.push(match material {
                    Some(material) => dispatch.with_material(material),
                    None => dispatch,
                });
              }
            }
        }
        if !dispatches.is_empty() {
            recorder.compute(
                crate::graph::Pass::Ao,
                "diffuse query reconstruction",
                "ao",
                false,
                dispatches,
            );
            if s.pipelines.resolve_native.is_some() {recorder.separate_last();}
        }
        Ok(Self {
            output: raw_ao,
            incident,
            receivers: cell_receivers,
        })
    }
}

/// Native shaders need explicit layouts; Metal indices follow these bindings.
pub(crate) fn slang_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    (0..6).map(|binding| wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: match binding {
            1 => wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},
            2 | 4 => wgpu::BindingType::StorageTexture {access:wgpu::StorageTextureAccess::WriteOnly,format:wgpu::TextureFormat::Rgba16Float,view_dimension:wgpu::TextureViewDimension::D2},
            _ => wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},
        },count:None,
    }).collect()
}

pub(crate) fn slang_layout(d: &wgpu::Device) -> wgpu::PipelineLayout {
    let entries = slang_entries();
    let group = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Slang AO resources"),
        entries: &entries,
    });
    d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Slang AO"),
        bind_group_layouts: &[Some(&group)],
        immediate_size: 0,
    })
}

#[cfg(test)]
mod prepared_query_tests {
    use crate::*;
    use serde_json::json;

    /// Exercise empty demand and inexpensive active rays with the same medium
    /// topology. Log warmed AO timings; both cases have an analytic furnace.
    #[test]
    #[ignore = "requires a real ray-query GPU"]
    fn prepared_diffuse_queries_preserve_sparse_and_active_furnaces() -> Result<()> {
        let r=Renderer::new()?;
        for active in [false,true] {
            let mut scene=crate::test_scene(json!({"version":2,
                "render":{"resolution":[64,64]},
                "buffers":{"p":{"type":"vec3","data":[[-100,-100,0],[100,-100,0],[0,100,0]]}},
                "geometry":{"plane":{"attributes":{"POSITION":"p"}}},
                "materials":{"receiver":{"inputs":{"base_weight":if active {1.} else {0.},"base_color":[1,1,1],"specular_weight":0,"emission_color":[1,1,1],"emission_luminance":if active {0.} else {1.}}},
                    "fog":{"program":"std/fog","inputs":{"absorption":[0.1,0.1,0.1],"scattering":[0,0,0]}}},
                "nodes":[{"type":"camera","translation":[0,0,1],"material":{"program":"std/camera","inputs":{"projection":"ortho","ortho_scale":2}}},
                    {"type":"mesh","geometry":"plane","materials":["receiver"]},
                    {"type":"mesh","geometry":{"std":"box","size":[1,1,1]},"translation":[10000,0,0],"materials":["fog"]},
                    {"type":"light","shape":"environment","color":[1,1,1],"energy":1}]}),Path::new("."))?;
            scene.render.settings=serde_json::from_value(json!({"integrator":"raster","samples":32,"fast_gi":{"method":"ray"},"reflections":"environment","denoiser":null}))?;
            let mut installed=r.install(scene,Path::new("."))?;
            assert!(installed.pipelines.resolve_native.is_some() && installed.plan.effects.indirect);
            let mut slot=installed.create_frame_slot(&r,ReadbackPrecision::Float)?;slot.profile=true;
            for repetition in 0..4 {
                let image=installed.render_frame(&r,&mut slot,0.)?;
                let pixels=image.linear_rgba.unwrap();
                let error=pixels.iter().map(|p|(p[0]-1.).abs()).fold(0_f32,f32::max);
                assert!(error<0.01,"active={active}: furnace error {error}");
                eprintln!("prepared queries active={active} repetition={repetition} max_error={error} gpu_ms={} spans={:?}",image.timing.gpu_ms,image.timing.gpu_spans_ms);
            }
        }
        Ok(())
    }
}
