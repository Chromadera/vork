//! Display programs execute once per output pixel. Scene-linear outputs bypass this stage.
use super::*;
use std::fmt::Write;
use vork_scene::v2;
use vork_slang::{Compiler, DisplayProgram};

/// A compiled display program with its packed inputs and image data, ready to evaluate.
pub(crate) struct Prepared {
    program: DisplayProgram,
    words: Vec<u32>,
    animation: Option<programs::InputAnimation>,
    sources: Vec<ImageSource>,
}
/// One `Image` input as linear RGBA32F texels.
struct ImageData {
    width: u32,
    height: u32,
    texels: Vec<f32>,
}
enum ImageSource {
    /// A bundled program's library data (OpenEXR bytes).
    Library(&'static [u8]),
    /// A scene `images` entry with a file.
    File {
        path: PathBuf,
        bytes: Vec<u8>,
        srgb: bool,
    },
}

pub(crate) fn prepare(
    r: &Renderer,
    scene: &v2::Scene,
    base: &Path,
) -> Result<Prepared> {
    let (material, display) = match &scene.render.display {
        v2::NameOr::Name(name) => (Some(name.as_str()), &scene.materials[name]),
        v2::NameOr::Inline(display) => (None, display),
    };
    let name = &display.program;
    let (source, origin) = match vork_slang::module_source(name) {
        Some(source) => (source.to_owned(), PathBuf::from(format!("{name}.slang"))),
        None => programs::program_source(
            base,
            name,
            scene
                .programs
                .get(name)
                .with_context(|| format!("render.display: unknown program {name}"))?,
        )?,
    };
    let abi = programs::abi();
    let key = {
        let mut hash = blake3::Hasher::new();
        hash.update(b"display-stage");
        hash.update(abi.cache_identity()?.as_bytes());
        hash.update(source.as_bytes());
        hash.update(origin.to_string_lossy().as_bytes());
        hash.finalize()
    };
    let tag = key.to_hex();
    let program = programs::cached(
        r,
        key,
        &Compiler::discover(),
        &abi, &source, &origin,
        |compiler| compiler.compile_display(&abi, &source, &origin, &tag[..12]),
    )
    .with_context(|| format!("render.display: program {name}"))?;
    if !program.warnings.trim().is_empty() {
        eprintln!("{}", program.warnings.trim());
    }
    let mut values = display
        .inputs
        .iter()
        .map(|(k, v)| Ok((k.clone(), serde_json::to_value(v)?)))
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    // A bundled program's unbound image inputs read its library data.
    let mut defaults = std::collections::BTreeMap::new();
    for field in program
        .inputs
        .fields
        .iter()
        .filter(|f| f.ty == vork_slang::Type::Image)
    {
        if values.contains_key(&field.name) {
            continue;
        }
        if let Some(image) = vork_scene::libraries::program_image(name, &field.name) {
            defaults.insert(field.name.clone(), image);
            values.insert(field.name.clone(), serde_json::Value::Null);
        }
    }
    let mut sources = Vec::new();
    let mut words = program.inputs.pack(&values, |input, value| {
        let source = if let Some(image) = defaults.get(input) {
            let executable = match image {
                vork_scene::libraries::ProgramImage::File(_) => Some(std::env::current_exe()?.canonicalize()?),
                _ => None,
            };
            library_image(*image, executable.as_deref())?
        } else {
            let v2::Input::Texture(binding) = serde_json::from_value::<v2::Input>(value.clone())
                .with_context(|| format!("input {input}: expected {{\"image\": name}}"))? else {
                bail!("input {input}: expected {{\"image\": name}}, got {value}");
            };
            let image = scene.images.get(&binding.image).with_context(|| format!("input {input}: {}",
                v2::diagnostics::unknown("image", &binding.image, scene.images.keys().map(String::as_str))))?;
            let file = image.file.as_ref().with_context(|| format!("input {input}: image {} is a render image; display programs read file images", binding.image))?;
            let path = base.join(file);
            let bytes = std::fs::read(&path).with_context(|| format!("read image {}", path.display()))?;
            ImageSource::File { path, bytes, srgb: image.is_srgb() }
        };
        sources.push(source);
        Ok(sources.len() as u32 - 1)
    }).with_context(|| format!("render.display: program {name}"))?;
    let animation = match material {
        Some(name) => programs::InputAnimation::with_discrete(scene, base, name, &program.inputs, &mut words, true)?,
        None => None,
    };
    Ok(Prepared {
        animation,
        program,
        words,
        sources,
    })
}

/// Follow the companion-tool layout: resolve data beside the canonical executable.
fn library_image(image: vork_scene::libraries::ProgramImage, executable: Option<&Path>) -> Result<ImageSource> {
    match image {
        vork_scene::libraries::ProgramImage::Embedded(bytes) => Ok(ImageSource::Library(bytes)),
        vork_scene::libraries::ProgramImage::File(relative) => {
            let path = executable.context("locate executable for library image")?
                .parent().context("locate executable directory for library image")?.join(relative);
            let bytes = std::fs::read(&path)
                .with_context(|| format!("read library image {}", path.display()))?;
            Ok(ImageSource::File { path, bytes, srgb: false })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filmic_uses_companion_file_and_reports_expected_path() -> Result<()> {
        let root = std::env::temp_dir().join(format!("vork-filmic-data-{}", std::process::id()));
        let executable = root.join("vork");
        let path = root.join("libraries/blender/filmic.exr");
        let image = vork_scene::libraries::program_image("blender/filmic", "lut").unwrap();
        let error = library_image(image, Some(&executable)).err().unwrap();
        assert!(error.to_string().contains(&path.display().to_string()), "{error:#}");
        std::fs::create_dir_all(path.parent().unwrap())?;
        let original = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../libraries/blender/filmic.exr"))?;
        std::fs::write(&path, &original)?;
        let loaded = library_image(image, Some(&executable))?;
        match &loaded {
            ImageSource::File { bytes, srgb, .. } => { assert_eq!(bytes, &original); assert!(!srgb); }
            _ => panic!("Filmic must load from disk"),
        }
        let decoded = loaded.decode(16384)?;
        assert!(decoded.width > 0 && decoded.height > 0);
        assert!(matches!(library_image(vork_scene::libraries::program_image("std/agx", "lut").unwrap(), None)?, ImageSource::Library(_)));
        assert!(vork_scene::libraries::program_image("std/openpbr", "lut").is_none());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}

impl ImageSource {
    /// Decode once at installation; images remain resident for per-pixel evaluation.
    fn decode(&self, max_side: u32) -> Result<ImageData> {
        let (decoded, srgb) = match self {
            Self::Library(bytes) => (
                crate::texture_source::decode_format(image::ImageFormat::OpenExr, bytes, max_side)?,
                false,
            ),
            Self::File { path, bytes, srgb } => (
                crate::texture_source::decode(path, bytes, max_side)
                    .with_context(|| format!("decode image {}", path.display()))?,
                *srgb,
            ),
        };
        let rgba = decoded.to_rgba32f();
        let (width, height) = rgba.dimensions();
        let mut texels = rgba.into_raw();
        if srgb {
            for texel in texels.chunks_exact_mut(4) {
                for c in &mut texel[..3] {
                    *c = texture_source::decode_srgb(*c);
                }
            }
        }
        Ok(ImageData {
            width,
            height,
            texels,
        })
    }
}

pub(crate) struct Stage {
    pipeline: wgpu::ComputePipeline,
    words: wgpu::Buffer,
    images: Vec<wgpu::TextureView>,
    input_words: Vec<u32>,
    animation: Option<programs::InputAnimation>,
}
impl Prepared {
    pub(crate) fn install(self, r: &Renderer) -> Result<Stage> {
        let d = &r.device;
        let decoded = self
            .sources
            .iter()
            .map(|source| source.decode(d.limits().max_texture_dimension_2d))
            .collect::<Result<Vec<_>>>()
            .context("render.display: display program image")?;
        let mut shader = self.program.wgsl.clone();
        shader.push_str(include_str!(concat!(env!("OUT_DIR"), "/constants.wgsl")));
        let mut loads = String::new();
        let mut extents = String::new();
        for k in 0..decoded.len() {
            writeln!(
                shader,
                "@group(0) @binding({}) var display_image_{k}:texture_2d<f32>;",
                k + 4
            )?;
            write!(
                loads,
                "case {k}u: {{let last=vec2<i32>(textureDimensions(display_image_{k}))-1;return textureLoad(display_image_{k},clamp(p,vec2(0),last),0);}}"
            )?;
            write!(
                extents,
                "case {k}u: {{return vec2<f32>(textureDimensions(display_image_{k}));}}"
            )?;
        }
        let format = if r.output_format() == wgpu::TextureFormat::Bgra8Unorm {
            "bgra8unorm"
        } else {
            "rgba8unorm"
        };
        writeln!(
            shader,
            r#"
struct DisplayFrame {{words:array<vec4<f32>,{frame_words}>}};
@group(0) @binding(0) var<storage,read> display_words:array<u32>;
@group(0) @binding(1) var<storage,read_write> display_pixels:array<u32>;
@group(0) @binding(2) var<uniform> display_frame:DisplayFrame;
@group(0) @binding(3) var<storage,read_write> display_errors:array<atomic<u32>>;
@group(1) @binding(0) var display_output:texture_storage_2d<{format},write>;
fn {word}(i:u32)->u32 {{return display_words[i];}}
fn display_load(slot:u32,p:vec2<i32>)->vec4<f32> {{switch slot {{{loads}default: {{return vec4(1.);}}}}}}
fn display_extent(slot:u32)->vec2<f32> {{switch slot {{{extents}default: {{return vec2(1.);}}}}}}
fn {sample}(slot:u32,uv:vec2<f32>,dx:vec2<f32>,dy:vec2<f32>)->vec4<f32> {{
    let p=uv*display_extent(slot)-.5;let q=vec2<i32>(floor(p));let f=p-floor(p);
    return mix(mix(display_load(slot,q),display_load(slot,q+vec2(1,0)),f.x),mix(display_load(slot,q+vec2(0,1)),display_load(slot,q+vec2(1,1)),f.x),f.y);
}}
@compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) gid:vec3<u32>) {{
    let temporal=display_frame.words[{temporal}];
    if(temporal.x+1.<temporal.y) {{return;}}
    let flags=u32(display_frame.words[{projection}].w);
    if((flags&(REG_OUTPUT_GPU_COLOR|REG_OUTPUT_COLOR_PLANE))==0u) {{return;}}
    let settings=display_frame.words[{display}];let size=vec2<u32>(settings.yz);
    if any(gid.xy>=size) {{return;}}
    let count=size.x*size.y;let i=gid.y*size.x+gid.x;let base=u32(settings.w);
    let value=vec4(bitcast<f32>(display_pixels[base*count+i]),bitcast<f32>(display_pixels[(base+1u)*count+i]),bitcast<f32>(display_pixels[(base+2u)*count+i]),bitcast<f32>(display_pixels[(base+3u)*count+i]));
    var result:array<vec4<f32>,1>;
    {entry}(value.rgb,gid.xy,bitcast<u32>(settings.x),0u,&result);
    var mapped=result[0].rgb;
    if !all(abs(mapped)<=vec3(3.402823466e38)) {{atomicMax(&display_errors[0],REG_DISPLAY_NONFINITE);mapped=vec3(0.);}}
    mapped=clamp(mapped,vec3(0.),vec3(1.));
    if((flags&REG_OUTPUT_GPU_COLOR)!=0u) {{textureStore(display_output,vec2<i32>(gid.xy),vec4(mapped,value.a));}}
    if((flags&REG_OUTPUT_COLOR_PLANE)!=0u) {{display_pixels[i]=pack4x8unorm(vec4(mapped,value.a));}}
}}
"#,
            frame_words = std::mem::size_of::<Uniform>() / 16,
            projection = std::mem::offset_of!(Uniform, projection) / 16,
            temporal = std::mem::offset_of!(Uniform, temporal) / 16,
            display = std::mem::offset_of!(Uniform, display) / 16,
            word = vork_slang::MATERIAL_WORD,
            sample = vork_slang::SAMPLE_IMAGE,
            entry = self.program.entry
        )?;
        let module = crate::timing::shader_module(
            d,
            wgpu::ShaderModuleDescriptor {
                label: Some("per-pixel display program"),
                source: wgpu::ShaderSource::Wgsl(shader.into()),
            },
        );
        let mut entries = vec![
            pipelines::buffer_entry(
                0,
                wgpu::ShaderStages::COMPUTE,
                wgpu::BufferBindingType::Storage { read_only: true },
            ),
            pipelines::buffer_entry(
                1,
                wgpu::ShaderStages::COMPUTE,
                wgpu::BufferBindingType::Storage { read_only: false },
            ),
            pipelines::buffer_entry(
                2,
                wgpu::ShaderStages::COMPUTE,
                wgpu::BufferBindingType::Uniform,
            ),
            pipelines::buffer_entry(
                3,
                wgpu::ShaderStages::COMPUTE,
                wgpu::BufferBindingType::Storage { read_only: false },
            ),
        ];
        entries.extend(
            (0..decoded.len() as u32).map(|k| wgpu::BindGroupLayoutEntry {
                binding: k + 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        );
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("display program"),
            entries: &entries,
        });
        let output = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("display output"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: r.output_format(),
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }],
        });
        let pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("display program"),
            bind_group_layouts: &[Some(&layout), Some(&output)],
            immediate_size: 0,
        });
        let pipeline = r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("display program"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: r.cache.as_ref(),
        });
        let words = init(
            d,
            "display program inputs",
            bytemuck::cast_slice(if self.words.is_empty() {
                &[0]
            } else {
                &self.words
            }),
            wgpu::BufferUsages::STORAGE,
        );
        let images = decoded
            .iter()
            .map(|image| {
                let texture = crate::gpu_memory::texture(
                    d,
                    &wgpu::TextureDescriptor {
                        label: Some("display program image"),
                        size: wgpu::Extent3d {
                            width: image.width,
                            height: image.height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba32Float,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    },
                );
                r.queue.write_texture(
                    texture.as_image_copy(),
                    bytemuck::cast_slice(&image.texels),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(image.width * 16),
                        rows_per_image: Some(image.height),
                    },
                    texture.size(),
                );
                texture.create_view(&Default::default())
            })
            .collect();
        Ok(Stage {
            pipeline,
            words,
            images,
            input_words: self.words,
            animation: self.animation,
        })
    }
}
impl Stage {
    pub(crate) fn frame_inputs(&self, r: &Renderer) -> Option<wgpu::Buffer> {
        self.animation.as_ref()?;
        Some(init(&r.device, "animated display inputs", bytemuck::cast_slice(&self.input_words),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST))
    }
    pub(crate) fn upload(&self, buffer: &wgpu::Buffer, time: f32, changes: &mut crate::values::ChangeTracker) {
        let mut words = self.input_words.clone();
        self.animation.as_ref().unwrap().words(time, &mut words);
        changes.upload(buffer, 0, bytemuck::cast_slice(&words));
    }
    pub(crate) fn dispatch(
        &self,
        r: &Renderer,
        uniform: &wgpu::Buffer,
        words: Option<&wgpu::Buffer>,
        pixels: &wgpu::Buffer,
        errors: &wgpu::Buffer,
        size: [u32; 2],
    ) -> graph::Dispatch {
        let mut entries = vec![
            entry(0, words.unwrap_or(&self.words).as_entire_binding()),
            entry(1, pixels.as_entire_binding()),
            entry(2, uniform.as_entire_binding()),
            entry(3, errors.as_entire_binding()),
        ];
        entries.extend(
            self.images
                .iter()
                .enumerate()
                .map(|(i, view)| entry(i as u32 + 4, wgpu::BindingResource::TextureView(view))),
        );
        let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("display program"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        graph::Dispatch::new(
            &self.pipeline,
            bind,
            [size[0].div_ceil(8), size[1].div_ceil(8), 1],
        )
    }
}
