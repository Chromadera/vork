//! `vork setup` compiles the bundled profiles' reachable program and material
//! configurations. Library roles come from their stage declarations; uniform
//! and varying medium inputs share the normal compilation and cache paths.
use super::*;
use ::vork_scene::v2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Role {
    Surface,
    Hair,
    Medium,
    Environment,
    Camera,
    Composite,
    Display,
}

/// The role a std program's entry point declares; a module without one of these entries
/// is a surface.
pub(crate) fn role(source: &str) -> Role {
    let declares = |ret: &str, entry: &str, context: &str| {
        source.lines().any(|line| {
            let line = line.trim_start().trim_start_matches("public ");
            line.starts_with(ret) && line[ret.len()..].trim_start().starts_with(entry)
                && line.contains(context)
        })
    };
    if declares("CameraRay", "camera(", "CameraContext") {
        Role::Camera
    } else if declares("float3", "environment(", "EnvironmentContext") {
        Role::Environment
    } else if declares("MediumCoefficients", "medium(", "MediumContext") {
        Role::Medium
    } else if source.contains("CompositeContext") && declares("float4", "composite(", "CompositeContext") {
        Role::Composite
    } else if declares("float3", "display(", "DisplayContext") {
        Role::Display
    } else if declares("Hair", "surface(", "ShadingContext") {
        Role::Hair
    } else {
        Role::Surface
    }
}

/// The warm scenes for these std programs (each scene before its frame configurations).
pub(crate) fn scenes(r: &Renderer, programs: impl Iterator<Item = &'static str>) -> Result<Vec<serde_json::Value>> {
    let mut scenes = Vec::new();
    for name in programs {
        let source = vork_slang::module_source(name).with_context(|| format!("std module {name}"))?;
        scenes.extend(program_scenes(r, name, source)?);
    }
    Ok(scenes)
}

/// Scenes without programs: the built-in material table, opaque, as a transmissive
/// solid (refraction tracks its boundary; transmission changes every hit kernel), and
/// that solid beside a subsurface material (thickness subsurface transport).
pub(crate) fn table_scenes(r: &Renderer) -> Vec<serde_json::Value> {
    let triangle = |material: &str| serde_json::json!({"type": "mesh", "geometry": "triangle", "materials": [material]});
    let solid = serde_json::json!({"type": "mesh", "geometry": "box", "materials": ["glass"], "translation": [0, 0, -1]});
    let materials = serde_json::json!({"table": {"inputs": {"base_color": [0.8, 0.8, 0.8]}},
        "glass": {"inputs": {"transmission_weight": 1.0}},
        "skin": {"inputs": {"subsurface_weight": 0.5}}});
    let meshes = [vec![triangle("table")], vec![solid.clone()], vec![solid, triangle("skin")]];
    meshes.iter().flat_map(|meshes| shadows(r).into_iter().map(|sun| {
        let mut scene = scene(serde_json::json!([]), serde_json::Value::Null,
            meshes.clone(), serde_json::json!({"type": "camera", "name": "c", "translation": [0,0,3]}),
            serde_json::json!({"type": "light", "name": "world", "shape": "environment"}), materials.clone(), sun);
        scene["geometry"]["box"] = serde_json::json!({"std": "box", "size": [1, 1, 1]});
        scene
    })).collect()
}

/// A process-owned, ordinary image binding shared by setup configurations.
fn setup_image() -> Result<PathBuf> {
    static IMAGE_LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
    let _lock=IMAGE_LOCK.lock().unwrap();
    let file=std::env::temp_dir().join(format!("vork-setup-image-{}.png",std::process::id()));
    if !file.exists() {image::RgbaImage::from_pixel(4,4,image::Rgba([255;4])).save(&file)?;}
    Ok(file)
}

/// One minimal setup scene.
fn scene(compositor: serde_json::Value, motion_blur: serde_json::Value, meshes: Vec<serde_json::Value>, camera: serde_json::Value,
    world: serde_json::Value, materials: serde_json::Value, sun: serde_json::Value) -> serde_json::Value {
    let mut nodes = meshes;
    nodes.extend([camera, serde_json::json!({"type":"light","name":"sun","shape":"sun"}), world]);
    serde_json::json!({"version": 2,
        "render": {"resolution": [64, 64], "compositor": compositor, "motion_blur": motion_blur, "settings": {"samples":1,"shadow_method":sun}},
        "buffers": {"p": {"type": "vec3", "data": [[0, 0, 0], [1, 0, 0], [0, 1, 0]]},
            "s": {"type": "vec3", "data": [[0, -1, 0], [0, 1, 0]]},
            "w": {"type": "f32", "data": [0.1, 0.1]}, "c": {"type": "u32", "data": [2]}},
        "geometry": {"triangle": {"attributes": {"POSITION": "p"}},
            "strand": {"topology": "curves", "attributes": {"POSITION": "s"}, "widths": "w", "curve_vertex_counts": "c"}},
        "nodes": nodes,
        "materials": materials})
}

/// The scenes of one library program, in the role its entry point declares.
fn program_scenes(r: &Renderer, name: &'static str, source: &str) -> Result<Vec<serde_json::Value>> {
    let mut scenes = Vec::new();
    let mut materials = serde_json::Map::new();
    let mut light = serde_json::json!({"type": "light", "name": "world", "shape": "environment"});
    let mut mesh = serde_json::json!({"type": "mesh", "name": "m", "geometry": "triangle"});
    let mut compositor = serde_json::json!([]);
    let mut camera = serde_json::json!({"type": "camera", "name": "c", "translation": [0,0,3]});
    let mut images=serde_json::json!({});
    let material = true;
    let motion_blur = serde_json::Value::Null;
    let mut variants = vec![serde_json::json!({})];
    let mut suns = vec![serde_json::json!("auto")];
    match role(source) {
        Role::Surface => mesh["materials"] = serde_json::json!(["std"]),
        Role::Hair => mesh = serde_json::json!({"type": "mesh", "name": "m", "geometry": "strand", "materials": ["std"]}),
        Role::Camera => camera["material"]="std".into(),
        Role::Environment => {
            light["material"]="std".into();
            let program=environment_program::compiled(r,source,&PathBuf::from(format!("{name}.slang")))?;
            let mut inputs=serde_json::Map::new();
            for field in program.inputs.fields.iter().filter(|field|field.ty==vork_slang::Type::Image) {
                let file=setup_image()?;
                images["setup"]=serde_json::json!({"file":file,"color_space":"Non-Color","wrap_t":"clamp_to_edge"});
                inputs.insert(field.name.clone(),serde_json::json!({"image":"setup"}));
            }
            variants=vec![serde_json::Value::Object(inputs)];
        },
        Role::Medium => {
            light["medium"] = "std".into();
            // Varying scalars select the nonuniform kernel; defaults may be uniform.
            let program = medium::compiled(r, source, &PathBuf::from(format!("{name}.slang")))?;
            let varied: serde_json::Map<_, _> = program.inputs.fields.iter()
                .filter(|f| matches!(f.ty, vork_slang::Type::Float) && f.enumeration.is_none()
                    && f.range.as_ref().is_none_or(|range| range.min <= 0.5 && 0.5 <= range.max))
                .map(|f| (f.name.clone(), serde_json::json!(0.5)))
                .collect();
            variants.push(serde_json::Value::Object(varied));
            // Ray-traced shadows compile their own volume kernels.
            suns = shadows(r);
        }
        Role::Composite => {
            compositor=serde_json::json!(["std"]);
            if name=="std/bloom" {
                // All shipped stages need their real producer bindings, including
                // the retained pre-bloom beauty read by prepare and combine.
                compositor=serde_json::json!([
                    {"name":"setup.prepare","material":{"program":name,"inputs":{"stage":"prepare","original":{"channel":"color"}}}},
                    {"name":"setup.down","resolution":[32,32],"material":{"program":name,"inputs":{"stage":"downsample","source":{"node":"setup.prepare"},"divisor":2}}},
                    {"name":"setup.up","material":{"program":name,"inputs":{"stage":"upsample","source":{"node":"setup.down"},"fine":{"node":"setup.prepare"}}}},
                    "std"]);
                variants=vec![serde_json::json!({"stage":"combine","original":{"channel":"color"},"source":{"node":"setup.up"},"levels":2})];
            }
        },
        Role::Display => {},
    }
    for (inputs, sun) in variants.iter().flat_map(|inputs| suns.iter().map(move |sun| (inputs, sun))) {
        if material {
            materials.insert("std".into(), serde_json::json!({"program": name, "inputs": inputs}));
        }
        let mut value=scene(compositor.clone(), motion_blur.clone(), vec![mesh.clone()], camera.clone(), light.clone(),
            serde_json::Value::Object(materials.clone()), sun.clone());
        value["images"]=images.clone();
        scenes.push(value);
    }
    if role(source) == Role::Display {
        for scene in &mut scenes { scene["render"]["display"] = "std".into(); }
    }
    Ok(scenes)
}

/// Warm the triangle shadow provider where the device can execute it.
fn shadows(r:&Renderer)->Vec<serde_json::Value> {
    if r.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {vec![serde_json::json!("ray")]} else {Vec::new()}
}

/// Compile each scene in every frame configuration that selects different passes:
/// what each bundled profile resolves to, both raster coverage counts, with and without
/// screen AO (which selects the unfused colour resolve).
pub(crate) fn compile(r: &Renderer, scenes: &[serde_json::Value]) -> Result<()> {
    run(r, compile_jobs(scenes)?.into_iter(), InstallOptions { variants: pipelines::VariantCompile::Setup, ..Default::default() })
}

/// Frame configurations: the profiles' own settings (a render's, before the renderer
/// resolves automatic techniques for the device), and the raster technique axes.
fn configurations() -> Result<Vec<v2::Settings>> {
    let mut settings: Vec<_> = vork_scene::libraries::profile_names()
        .map(|name|v2::Profile::load(name)?.resolve(&Default::default())).collect::<Result<_>>()?;
    for name in ["std/draft", "std/final"] {
        settings.push(v2::Profile::load(name)?.resolve(
            &serde_json::from_value(serde_json::json!({"denoise":false}))?)?);
    }
    Ok(settings)
}

fn compile_jobs(scenes: &[serde_json::Value]) -> Result<Vec<(serde_json::Value, v2::Settings)>> {
    let configurations = configurations()?;
    Ok(scenes.iter().flat_map(|scene| configurations.iter().map(|c| (scene.clone(), c.clone()))).collect())
}

/// A setup job's scene with its resolved settings, validated as a render would.
fn prepare(mut value: serde_json::Value, mut settings: v2::Settings) -> Result<v2::Scene> {
    let authored=value["render"].as_object_mut().unwrap().remove("settings").unwrap();
    if let Some(raster)=settings.integrator.raster_mut() {
        raster.shadow_method=match authored["shadow_method"].as_str() {
            Some("ray")=>v2::ShadowMethod::Ray,
            _=>raster.shadow_method,
        };
    }
    // Previs has a small shipped budget; retain it so setup reaches the merged
    // visibility path as well as the transmitting camera variants it will draw.
    if settings.integrator.is_path() { settings.samples=1; }
    vork_std::expand_scene(&mut value, Path::new("."))?;
    let mut scene = v2::Scene::from_json(&serde_json::to_vec(&value)?)?;
    scene.render.settings=settings;
    scene.validate()?;
    Ok(scene)
}

/// Install each (scene, integrator) and construct its frame resources. Constructors
/// compile all passes; submission would evaluate arbitrary library inputs as an
/// infinite world, which may have no finite emission integral. Setup certifies
/// compilation, not the semantics of those synthetic scenes.
/// Bound live installs: each job already fans out into compiler/driver workers.
/// A configuration the renderer refuses for a scene (path transport of an unbounded
/// sampled world medium) is one no render compiles: setup skips it.
fn run(r: &Renderer, jobs: impl Iterator<Item = (serde_json::Value, v2::Settings)>, options: InstallOptions) -> Result<()> {
    let jobs:Vec<_>=jobs.collect();
    let phase=if jobs.iter().all(|(_,settings)|settings.integrator.is_path()) {
        "Setup: warming path configurations"
    } else {"Setup: warming configurations"};
    vork_slang::progress::phase(phase,jobs.len());
    let jobs = std::sync::Mutex::new(jobs.into_iter());
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(1, usize::from).min(4);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..workers).map(|_| scope.spawn(|| -> Result<()> {
            loop {
                let Some((scene, integrator)) = jobs.lock().unwrap().next() else { break };
                let name = scene["materials"]["std"]["program"].as_str().unwrap_or("built-in materials").to_owned();
                let explicit_camera = scene["nodes"].as_array().unwrap().iter()
                    .any(|node| node["type"] == "camera" && !node["material"].is_null());
                // The explicit standard-camera jobs retain the broad hit keys.
                // Ordinary table jobs warm the smaller material-compatible keys
                // through the same install constructors, without extra installs.
                let variants = if explicit_camera && matches!(options.variants, pipelines::VariantCompile::SetupPlain) {
                    pipelines::VariantCompile::Setup
                } else { options.variants.clone() };
                let compile = || -> Result<()> {
                    let scene = prepare(scene, integrator)?;
                    let installed = r.install_with_options(scene, Path::new("."), InstallOptions {
                        variants, readback: options.readback, gpu_color: options.gpu_color,
                        ..Default::default() })?;
                    // Volume, resolve and composite pipelines belong to the frame slot.
                    let _frame = installed.create_frame_slot(r, Default::default())?;
                    Ok(())
                };
                match compile() {
                    Err(error) if crate::failure::is_scene(&error) => {}
                    result => result.with_context(|| format!("setup pipelines for {name}"))?,
                }
                vork_slang::progress::completed(completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1);
            }
            Ok(())
        })).collect();
        workers.into_iter().try_for_each(pipelines::join)
    })
}

/// The exhaustive setup catalogue includes table materials with both camera links.
fn warm_raster_tables(r: &Renderer) -> Result<()> {
    let Some(opaque)=table_scenes(r).into_iter().next() else {return Ok(());};
    let mut scene=opaque.clone();
    if r.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY | crate::rt::HIT_BUFFER_FEATURES) {
        scene["materials"]["glass"]["inputs"]["geometry_thin_walled"]=true.into();
        scene["nodes"].as_array_mut().unwrap().push(serde_json::json!({"type":"mesh","geometry":"box","materials":["glass"],"translation":[0,0,-1]}));
    }
    let mut jobs=Vec::new();
    for profile in ["std/draft", v2::DEFAULT_PROFILE, "std/previs"] {
        let settings=v2::Profile::load(profile)?.resolve(&Default::default())?;
        // Transmission changes the camera/shadow registry. Warm the opaque-only
        // default and previs variants too, so an ordinary first clip does not compile them.
        jobs.push((opaque.clone(),settings.clone()));
        jobs.push((scene.clone(),settings));
    }
    // Authored physical cameras use the bundled camera program and its ray
    // evaluator; warming only the implicit camera leaves a cold first clip.
    let camera_jobs:Vec<_>=jobs.iter().map(|(scene,settings)| {
        let mut scene=scene.clone();
        scene["materials"]["setup_camera"]=serde_json::json!({"program":"std/camera"});
        let camera=scene["nodes"].as_array_mut().unwrap().iter_mut().find(|n|n["type"]=="camera").unwrap();
        camera["material"]="setup_camera".into();
        (scene,settings.clone())
    }).collect();
    jobs.extend(camera_jobs);
    run(r,jobs.into_iter(), InstallOptions {variants:pipelines::VariantCompile::SetupPlain,..Default::default()})
}

/// The exact init scene and the render command's default profile. Use ordinary
/// color delivery and the same foreground choices as a one-frame render.
fn starter_scene() -> Result<v2::Scene> {
    let mut value: serde_json::Value = serde_json::from_str(vork_scene::STARTER_SCENE)?;
    vork_std::expand_scene(&mut value, Path::new("."))?;
    let mut scene = v2::Scene::from_json(&serde_json::to_vec(&value)?)?;
    scene.render.settings = v2::Profile::load(v2::DEFAULT_PROFILE)?.resolve(&Default::default())?;
    scene.validate()?;
    Ok(scene)
}

pub(crate) fn warm_startup(r: &Renderer, variants: pipelines::VariantCompile) -> Result<()> {
    vork_slang::progress::phase("Setup: warming the init starter at the default profile", 1);
    let mut scene = starter_scene()?;
    if !r.release_preparation {
        let profile = v2::Profile::load(v2::DEFAULT_PROFILE)?;
        crate::denoise::resolve_default(r, &mut scene, &profile.name, profile.level, false);
    }
    let installed = r.install_with_options(scene, Path::new("."), InstallOptions {
        variants,
        readback: ReadbackOutputs { color: true, ..Default::default() },
        gpu_color: false,
        ..Default::default()
    })?;
    let _frame = installed.create_frame_slot(r, Default::default())?;
    vork_slang::progress::completed(1);
    Ok(())
}

pub(crate) fn warm(r: &Renderer) -> Result<()> {
    warm_raster_tables(r)?;
    // Retain exact starter keys even after the exhaustive catalogue recorded
    // generic alternatives. Portable preparation captures these artifacts too.
    warm_startup(r, pipelines::VariantCompile::Inline)?;
    let mut all = table_scenes(r);
    all.extend(scenes(r, vork_scene::libraries::prebuilt_programs())?);
    compile(r, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_plain_hits_match_exact_specializations() -> Result<()> {
        for metalness in [0., 1.] {
            // Adapter-free runners skip with test_renderer's diagnostic; renderer
            // failures and actual hit-selection/pixel assertions still fail on a GPU.
            let Some(mut r) = crate::test_renderer()? else { return Ok(()) };
            // Isolate the device-key catalogue from disk and other tests. Setup
            // must be the sole source of known keys for the first install.
            let keys=r.disk_cache.path("test-plain-hit-keys",1,blake3::hash(b""));
            r.compiler = pipelines::Compiler::new(&r.device, r.cache.as_ref(), pipelines::Known::load(keys,false));
            // This regression covers the exhaustive catalogue's raster hit selection.
            passes::decoupled::warm_shaders(&r);
            warm_raster_tables(&r)?;
            r.finish_shader_warmup()?;
            let mut scene = crate::test_scene(serde_json::json!({"version":2,
                "render":{"resolution":[64,48]},
                "nodes":[
                    {"type":"mesh","geometry":{"std":"sphere"},"materials":[{"inputs":{"base_color":[0.2,0.4,0.7],"base_metalness":metalness,"specular_roughness":0.3}}]},
                    {"type":"mesh","translation":[0,-1,0],"geometry":{"std":"grid","size":[8,8]}},
                    {"type":"camera","translation":[3,2,5],"target":[0,0,0]},
                    {"type":"light","shape":"rectangle","translation":[-3,5,4],"target":[0,0,0],"energy":500,"size":3},
                    {"type":"light","shape":"sun","energy":0.1},
                    {"type":"light","shape":"point","translation":[2,3,2],"energy":10},
                    {"type":"light","shape":"spot","translation":[0,4,0],"target":[0,0,0],"energy":10}
                ]}), Path::new("."))?;
            scene.render.settings = v2::Profile::load(v2::DEFAULT_PROFILE)?.resolve(
                &serde_json::from_value(serde_json::json!({"denoise":false}))?)?;
            let draw = |mode, expect_plain| -> Result<Vec<[f32;4]>> {
                let gate = match &mode {pipelines::VariantCompile::Gated(gate) => Some(gate.clone()), _ => None};
                struct Release(Option<std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>>);
                impl Drop for Release {
                    fn drop(&mut self) {if let Some(gate)=&self.0 {*gate.0.lock().unwrap()=true;gate.1.notify_all();}}
                }
                let _release = Release(gate.clone());
                r.compiler.hit_selections.lock().unwrap().clear();
                let mut installed = r.install_with_options(scene.clone(), Path::new("."), InstallOptions { variants: mode, ..Default::default() })?;
                let selections = r.compiler.hit_selections.lock().unwrap().clone();
                for is_gi in [true,false] {
                    let hits: Vec<_> = selections.iter().filter(|(requested,_)| if is_gi {
                        matches!(requested,pipelines::ProgramKernel::Gi {..})
                    } else {matches!(requested,pipelines::ProgramKernel::Reflections {..})}).collect();
                    assert_eq!(hits.len(),1,"must observe the installed hit pipeline: {selections:?}");
                    let (requested,selected)=*hits[0];
                    let plain=requested.plain_hit().expect("plain-compatible scene");
                    assert_ne!(requested,plain,"scene must distinguish exact and plain keys");
                    assert_eq!(selected,if expect_plain {plain} else {requested},"actual installed hit selection");
                }
                let mut slot = installed.create_frame_slot(&r, ReadbackPrecision::Float)?;
                let first = installed.render_frame(&r, &mut slot, 0.)?.linear_rgba.context("float readback")?;
                if let Some(gate) = gate {
                    assert!(!installed.variants_ready(), "hit variants must not block the first frame");
                    assert!(slot.view.variants_pending());
                    *gate.0.lock().unwrap() = true;
                    gate.1.notify_all();
                    r.settle_pipelines()?;
                    let promoted = installed.render_frame(&r, &mut slot, 0.)?.linear_rgba.context("promoted float readback")?;
                    assert!(!slot.view.variants_pending(), "the retained slot must promote at an exposure boundary");
                    assert_eq!(first, promoted, "promoting hit kernels changed scene-linear pixels");
                }
                Ok(first)
            };
            let plain = draw(pipelines::VariantCompile::Fallback, true)?;
            let gate = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
            let background = draw(pipelines::VariantCompile::Gated(gate), true)?;
            r.settle_pipelines()?;
            let promoted = draw(pipelines::VariantCompile::Background, false)?;
            let exact = draw(pipelines::VariantCompile::Inline, false)?;
            assert_eq!(plain, exact, "metalness {metalness}: setup hit evaluator changed the frame");
            assert_eq!(background, exact, "background hit promotion changed the frame");
            assert_eq!(promoted, exact, "settled installs must select exact hit keys");
        }
        Ok(())
    }

    #[test]
    fn setup_covers_draft_and_final_oidn_and_off_at_shipped_camera_budgets() -> Result<()> {
        let settings = configurations()?;
        for (name, budget) in [("std/draft",4), ("std/final",8)] {
            let shipped = v2::Profile::load(name)?.resolve(&Default::default())?;
            assert!(shipped.integrator.denoiser().is_some());
            assert_eq!(shipped.samples, budget);
            assert!(settings.contains(&shipped));
            let off = v2::Profile::load(name)?.resolve(
                &serde_json::from_value(serde_json::json!({"denoise":false}))?)?;
            assert!(settings.contains(&off));
            let value = serde_json::json!({"version":2,"nodes":[{"type":"camera"}],"render":{"settings":{}}});
            let scene = prepare(value, shipped)?;
            assert_eq!(scene.render.settings.samples, budget);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires lavapipe or Metal and pinned Slang; emits setup volume variants as MSL"]
    fn setup_volume_variants_emit_msl() -> Result<()> {
        let Some(r) = crate::test_renderer()? else { anyhow::bail!("GPU required"); };
        let scenes = scenes(&r, vork_scene::libraries::prebuilt_programs())?;
        let jobs = compile_jobs(&scenes)?;
        let output = std::env::temp_dir().join(format!("vork-setup-metal-{}", std::process::id()));
        std::fs::create_dir_all(&output)?;
        eprintln!("MSL setup sources: {}", output.display());
        for (scene, integrator) in jobs {
            if !scene["nodes"].as_array().unwrap().iter().any(|node| node.get("medium").is_some()) { continue; }
            let label = format!("materials={}, integrator={integrator:?}", scene["materials"]);
            eprintln!("MSL setup: {label}");
            let scene = prepare(scene, integrator)?;
            let stages = programs::mesh_stages(&r, &scene, Path::new("."))?;
            let environment = environment_node::Settings::from_scene(&scene)?;
            if let Some(mut installed) = medium::install(&r, &scene, environment.medium.as_deref(), Path::new("."), &stages)? {
                installed.native.configure_storage(&scene, &r.device.limits())?;
                let native = installed.native;
                let mut column = native.clone();
                column.boundaries = 0;
                column.world_only = true;
                for (entry, native) in [("volume_visibility_world", &column), ("volume_lighting", &column), ("volume_apply", &native)] {
                    for rays in [false, true] {
                        let mut native = native.clone();
                        native.ray_shadows = rays;
                        native.link_shadow_programs(None, crate::rt::may_refract(&scene));
                        let code = passes::resolve::cached_native(&r, &installed.shader, &native, entry, true)
                            .with_context(|| format!("{label}: {entry}, rays={rays}"))?;
                        let path = output.join(format!("{entry}-{}.metal", blake3::hash(&code).to_hex()));
                        std::fs::write(&path, &code)?;
                        if native.program_coverage {
                            let code = std::str::from_utf8(&code)?;
                            let signature = code.split("[[kernel]]").nth(1).context("Metal entry")?
                                .split('{').next().unwrap();
                            ensure!(!signature.contains("array<uint device*"), "{label}: unbound by-value hit buffer array in {entry}");
                            let layout = crate::resolve_native::Layout { volume:true, rays, shadow_closures:true };
                            let slot = crate::resolve_native::hit_base(entry, layout);
                            let hit = signature.split(" medium_hit_buffers").nth(1).context("Metal hit table parameter")?
                                .split(',').next().unwrap();
                            ensure!(hit.contains(&format!("[[buffer({slot})]]")),
                                "{label}: {entry} hit table must bind at Metal buffer {slot}: {signature}");
                        }
                        #[cfg(target_vendor = "apple")]
                        {
                            if path.with_extension("air").is_file() { continue; }
                            let result = std::process::Command::new("xcrun")
                                .args(["-sdk", "macosx", "metal", "-c", "-std=metal3.2"])
                                .arg(&path).arg("-o").arg(path.with_extension("air")).output()?;
                            ensure!(result.status.success(), "{label}: {entry}, rays={rays}: {}", String::from_utf8_lossy(&result.stderr));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn every_std_program_has_a_role() {
        let roles: Vec<_> = vork_scene::libraries::prebuilt_programs()
            .map(|name| (name, role(vork_slang::module_source(name).unwrap()))).collect();
        for (name, expected) in [("std/sky", Role::Environment), ("std/bloom", Role::Composite),
            ("std/hair", Role::Hair), ("std/camera",Role::Camera), ("std/environment_image",Role::Environment)] {
            assert!(roles.contains(&(name, expected)), "{name}: {roles:?}");
        }
        // Medium programs are authored on demand and absent from the shipped
        // catalog; role detection must still recognize their entry signature.
        assert_eq!(role("MediumCoefficients medium(MediumContext ctx) {}"), Role::Medium);
        // blender and post programs compile on first use.
        assert!(roles.iter().all(|(name, _)| name.starts_with("std/")), "{roles:?}");
    }

    /// The synthetic scenes still obey the scene schema and settings contract.
    #[test]
    fn setup_scenes_validate() -> Result<()> {
        let Some(r) = crate::test_renderer()? else { return Ok(()) };
        let mut scenes = table_scenes(&r);
        scenes.extend(self::scenes(&r, vork_scene::libraries::prebuilt_programs())?);
        let mut count = 0;
        for (scene, integrator) in compile_jobs(&scenes)?.into_iter() {
            let name = scene["materials"]["std"]["program"].as_str().map_or_else(|| "built-in table".into(), str::to_owned);
            prepare(scene, integrator).with_context(|| format!("setup scene for {name}"))?;
            count += 1;
        }
        assert!(count > 0);
        Ok(())
    }

    /// Exercise setup's complete compile core, including both fog variants. Running
    /// synthetic frames here used to fail the unbounded-emission diagnostic.
    #[test]
    #[ignore = "requires Vulkan/Metal and pinned Slang toolchain; complete setup warmup"]
    fn setup_compiles_all_library_pipelines() -> Result<()> {
        let r = Renderer::new()?;
        r.warm_shaders()?;
        // Compilation must also accept inputs whose infinite-world integral is
        // divergent. The same inputs must still be refused by a real frame.
        let mut scenes = scenes(&r, ["std/fog"].into_iter())?;
        for scene in &mut scenes {
            scene["materials"]["std"]["inputs"] = serde_json::json!({"density": 0., "emission": [1., 1., 1.]});
        }
        compile(&r, &scenes)?;
        let (scene, integrator) = compile_jobs(&scenes)?.into_iter().next().context("fog compile job")?;
        let installed = r.install_with_options(prepare(scene, integrator)?, Path::new("."),
            InstallOptions { variants: pipelines::VariantCompile::Fallback, ..Default::default() })?;
        let mut frame = installed.create_frame_slot(&r, Default::default())?;
        frame.submit(&installed, &r, 0.)?;
        let error = frame.collect(&r).err().context("real frame accepted divergent world emission")?;
        assert!(format!("{error:#}").contains("unbounded medium emission"), "{error:#}");
        Ok(())
    }

    /// After setup's built-in-material scenes, a one-frame render of another scene
    /// with shipped programs (other materials, lights and glass) compiles no ray-hit or
    /// transmissive camera kernel: it draws the supersets setup stored, with the
    /// pixels of its own specializations.
    #[test]
    fn one_frame_renders_draw_setup_supersets() -> Result<()> {
        let Some(mut r) = crate::test_renderer()? else { return Ok(()) };
        if !r.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) { return Ok(()) }
        let root = std::env::temp_dir().join(format!("vork-setup-supersets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        r.disk_cache = disk_cache::Store::for_test(root.clone());
        // Warmth comes only from this store: drop keys loaded from the shared one.
        let known = pipelines::Known::load(r.disk_cache.path("pipeline-keys", 1, blake3::hash(b"")), false);
        r.compiler = pipelines::Compiler::new(&r.device, r.cache.as_ref(), known);
        // Setup's shipped camera generics, then its built-in-material scenes.
        passes::decoupled::warm_shaders(&r);
        compile(&r, &table_scenes(&r))?;
        let variants = || -> Result<std::collections::BTreeSet<std::ffi::OsString>> {
            Ok(std::fs::read_dir(&root)?.flatten().map(|e| e.file_name())
                .filter(|n| n.to_string_lossy().contains(".slang-variant-")).collect())
        };
        let before = variants()?;
        let settings = v2::Profile::load(v2::DEFAULT_PROFILE)?.resolve(&serde_json::Map::new())?;
        let mut scene = crate::test_scene(serde_json::json!({"version": 2,
            "render": {"resolution": [48, 32]},
            "geometry": {"box": {"std": "box", "size": [1, 1, 1]}, "floor": {"std": "box", "size": [4, 0.1, 4]}},
            "nodes": [{"type": "mesh", "geometry": "box", "materials": ["glass"]},
                {"type": "mesh", "geometry": "floor", "materials": ["metal"], "translation": [0, -0.6, 0]},
                {"type": "camera", "translation": [0.5, 0.8, 3]},
                {"type": "light", "shape": "rectangle", "energy": 40, "translation": [1, 2, 1]},
                {"type": "light", "shape": "environment"}],
            "materials": {"glass": {"inputs": {"transmission_weight": 1.0, "coat_weight": 0.5}},
                "metal": {"inputs": {"base_color": [0.9, 0.6, 0.3], "base_metalness": 1.0, "specular_roughness": 0.3}}}}), Path::new("."))?;
        scene.render.settings = settings;
        let draw = |r: &Renderer, variants| -> Result<Vec<u8>> {
            let mut installed = r.install_with_options(scene.clone(), Path::new("."), InstallOptions { variants, ..Default::default() })?;
            let mut frame = installed.create_frame_slot(r, Default::default())?;
            // Setup warmth is independent of receiver-storage demand. Exercise
            // the same growth/retry path as a user's first rendered frame.
            Ok(installed.render_frame(r, &mut frame, 0.)?.rgba)
        };
        let superset = draw(&r, pipelines::VariantCompile::Fallback)?;
        let added: Vec<_> = variants()?.difference(&before).cloned().collect();
        assert!(added.is_empty(), "a one-frame render after setup compiled kernels: {added:?}");
        let specialized = draw(&r, pipelines::VariantCompile::Inline)?;
        assert!(variants()?.len() > before.len(), "the scene's own specializations are not setup's");
        let _ = std::fs::remove_dir_all(&root);
        let differs = superset.iter().zip(&specialized).filter(|(a, b)| a.abs_diff(**b) > 2).count();
        assert!(differs * 1000 < superset.len(), "superset and specialized kernels draw different pixels: {differs} channels");
        Ok(())
    }

    #[test]
    fn setup_prebuilds_std_worlds() -> Result<()> {
        let Some(r) = crate::test_renderer()? else { return Ok(()) };
        compile(&r, &scenes(&r, ["std/sky"].into_iter())?)?;
        // Any std/sky world now finds its compiled program; only its input words differ.
        let source = vork_slang::module_source("std/sky").context("std/sky module")?;
        let key = environment_program::program_key(&programs::abi(), source, Path::new("std/sky.slang"))?;
        let _:vork_slang::EnvironmentProgram=programs::cached(&r, key, &vork_slang::Compiler::discover(), &programs::abi(), source, Path::new("std/sky.slang"), |_| bail!("std/sky compiled again"))?;
        let scene: v2::Scene = serde_json::from_value(serde_json::json!({"version": 2,
            "nodes": [{"type": "light", "shape": "environment", "material": "sky"}],
            "materials": {"sky": {"program": "std/sky", "inputs": {"sun_elevation": 0.5, "altitude": 10}}}}))?;
        let (image, program) = environment_program::install(&r, &scene, "sky", Path::new("."))?;
        assert!(program.is_none() && image.image.pixels().any(|p| p.0[2] > 0.), "a constant, lit sky");
        // Fog at its defaults and varied, under each shadow topology: every medium kernel.
        assert_eq!(scenes(&r, ["std/fog"].into_iter())?.len(), 2 * shadows(&r).len());
        Ok(())
    }
}
