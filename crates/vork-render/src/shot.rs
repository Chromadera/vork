//! Complete ordered shot rendering, encoding and CPU output delivery.
pub use crate::png_output::Output;
use crate::png_output::{FrameOutputWriter, Outputs};
use crate::{FRAMES_IN_FLIGHT, FrameSlot, InstalledScene, Renderer, constants};
use anyhow::{Context, Result};
use crate::failure::{FailureKind, device, setup};

macro_rules! ensure_usage {
    ($condition:expr, $($message:tt)*) => {
        if !$condition { return Err(anyhow::anyhow!($($message)*).context(FailureKind::Usage)); }
    };
}
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use vork_encode::{Backend, EncodeSettings, Encoder};
/// Output and scheduling values for a shot; structural GPU resources stay installed.
#[derive(Clone)]
pub struct Shot {
    /// Finish submitted frames and close output files when requested by the caller.
    pub stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub scene: PathBuf,
    /// Empty selects the built-in control set and every scene-declared AOV.
    pub outputs: Vec<Output>,
    pub out: Vec<PathBuf>,
    pub at: Option<vork_time::Clock>,
    /// First and last frame index to render (inclusive, absolute; Blender's
    /// frame start/end). None selects the clip's first or last frame.
    pub frame_start: Option<usize>,
    pub frame_end: Option<usize>,
    pub resolution: Option<[u32; 2]>,
    /// Render settings: the profile (None = `std/final`) and explicit values above
    /// it (`settings`), replacing the profile's seed, samples and denoise intent.
    pub profile: Option<vork_scene::Profile>,
    /// Call fields: resolution, fps, seed, samples and denoise.
    pub settings: serde_json::Map<String, serde_json::Value>,
    #[cfg(any(test, feature = "test-support"))]
    pub test_techniques: serde_json::Map<String, serde_json::Value>,
    pub no_write: bool,
    pub gpu_only: bool,
    pub diagnostics: bool,
    /// Blender ImageFormatSettings.exr_codec; defaults to lossless ZIP (16 scanlines).
    pub exr_codec: crate::ExrCodec,
    /// Bits per component of PNG colour (still and directory frames); defaults to 8.
    pub png_depth: crate::PngDepth,
    /// Rewrites loaded content with resolved settings (and the scene file's directory)
    /// before renderer-support validation and the resolution override.
    pub edit: Option<SceneEdit>,
}
/// A caller's scene rewrite, such as a caller-provided content transform.
pub type SceneEdit = fn(&mut vork_scene::Scene, &Path) -> Result<()>;

/// Finished files and their timings, retaining GPU ownership until dropped.
/// Library callers reclaim resources normally. A one-shot process can exit with
/// this value alive after flushing its output, avoiding redundant driver teardown.
/// Input decoder children have already been cancelled and reaped.
/// With hardware video, drop this before std::process::exit/libc exit; only _exit
/// may skip the drop without running driver exit hooks.
pub struct RenderedShot {
    pub report: serde_json::Value,
    // Drop dependent frame and scene resources before the device for library use.
    _encoders: Vec<Encoder>,
    _slots: Vec<FrameSlot>,
    _scene: InstalledScene,
    _renderer: Renderer,
}
impl RenderedShot {
    /// End a successful one-shot process after its caller has flushed reports.
    /// Files, encoders and decoder children are already finished. Library
    /// callers instead drop this value to reclaim GPU owners normally.
    #[doc(hidden)]
    pub fn exit_process(self) -> ! {
        let code = if self.report["interrupted"] == true { 130 } else { 0 };
        // _exit skips the pipeline cache's destructor. Its Metal binary archive
        // backs onto temporary files (read lazily while the process runs);
        // remove them here, after every output is delivered. Live owners and
        // any unfinished background compiles keep the in-memory archive.
        #[cfg(target_os = "macos")]
        wgpu::hal::metal::release_archive_files_for_exit();
        // libc exit can run CUDA/NVENC atexit hooks against intentionally live
        // owners. Match process delivery: the OS reclaims completed resources
        // and discards unfinished optional compiler work without driver teardown.
        #[cfg(unix)]
        // SAFETY: Shot::render joined file writers, reaped muxers and finished
        // outputs; the caller flushed Rust streams before consuming this value.
        unsafe { libc::_exit(code) }
        #[cfg(not(unix))]
        std::process::exit(code)
    }
}
impl Default for Shot {
    fn default() -> Self {
        Self {
            stop: None,
            scene: PathBuf::new(),
            outputs: vec![Output::Color],
            out: Vec::new(),
            at: None,
            frame_start: None,
            frame_end: None,
            resolution: None,
            profile: None,
            settings: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            test_techniques: Default::default(),
            no_write: false,
            gpu_only: false,
            diagnostics: false,
            exr_codec: Default::default(),
            png_depth: Default::default(),
            edit: None,
        }
    }
}
// Thread CPU time excludes time blocked on GPU completion or encoder backpressure.
#[cfg(unix)]
fn thread_cpu_ms() -> Option<f64> {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a valid writable timespec and the current thread's clock ID.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    (result == 0).then_some(t.tv_sec as f64 * 1000. + t.tv_nsec as f64 / 1e6)
}
#[cfg(not(unix))]
fn thread_cpu_ms() -> Option<f64> {
    None
}
fn cpu_elapsed(start: Option<f64>) -> Option<f64> {
    start.zip(thread_cpu_ms()).map(|(a, b)| (b - a).max(0.))
}
// Deliver only accepted frames; native surfaces and portable copies belong to
// the output ring until their consumers release them.
fn push_encoders(encoders: &mut [Encoder], slot: &mut FrameSlot, frame: usize, submission: &wgpu::SubmissionIndex) -> Result<f64> {
    if encoders.is_empty() {
        slot.ring.frame = Some(frame);
        return Ok(0.);
    }
    let start = Instant::now();
    let output = slot.view.output.clone();
    for (encoder, surface) in encoders.iter_mut().zip(&mut slot.ring.encoder_surfaces) {
        encoder.push(frame as u64, output.clone(), surface, submission).map_err(device)?;
    }
    slot.ring.frame = Some(frame);
    Ok(start.elapsed().as_secs_f64() * 1000.)
}
/// File sidecars retain the destination name, including its format, without an
/// EXR frame placeholder: `x.%06d.exr` → `x.exr.meta.json`, `exr/%06d.exr` →
/// `exr/meta.json` (as a directory destination).
pub(crate) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let name = match name.find("%06d") {
        Some(i) => {
            let before = name[..i].trim_end_matches(['.', '_', '-']);
            format!("{before}{}", &name[i + 4..])
        }
        None => name,
    };
    let name = if name.starts_with('.') && name[1..].find('.').is_none() {
        suffix.to_owned()
    } else {
        format!("{name}.{suffix}")
    };
    path.with_file_name(name)
}

/// A trailing separator (`frames.v2/`) marks a directory destination.
pub(crate) fn directory_marked(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().ends_with(std::path::is_separator)
}
fn video_destination(path: &Path) -> bool {
    !directory_marked(path)
        && path.extension().is_some_and(|s| s.eq_ignore_ascii_case("mp4") || s.eq_ignore_ascii_case("mov"))
}
/// A known file extension, or a directory: no extension or a trailing separator
/// (`frames.v2/`). Any other extension is a mistake, never a silent PNG folder.
fn recognized_destination(path: &Path) -> bool {
    directory_marked(path)
        || path.extension().is_none_or(|e| ["png", "exr", "mp4", "mov"].iter().any(|k| e.eq_ignore_ascii_case(k)))
}

/// Frame placeholders belong to the basename; parent directories are literal.
fn has_frame_pattern(path: &Path) -> bool {
    let Some(name) = path.file_name() else { return false; };
    let name = name.to_string_lossy();
    if name.contains("##") {
        return true;
    }
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            i += 1;
            continue;
        }
        i += 1;
        // A doubled percent is a literal percent in printf syntax.
        if bytes.get(i) == Some(&b'%') {
            i += 1;
            continue;
        }
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if bytes.get(i) == Some(&b'd') {
            return true;
        }
    }
    false
}

fn resolve_outputs(outputs: Vec<Output>, aovs: &[vork_scene::v2::Aov]) -> Vec<Output> {
    if !outputs.is_empty() {
        return outputs;
    }
    Output::builtins()
        .chain(aovs.iter().map(|aov| Output::Aov(aov.name.clone())))
        .collect()
}

impl Shot {
    pub fn render(self) -> Result<RenderedShot> {
        // Device startup overlaps shot preparation, but must finish before any error can
        // return to the caller and tear down the process or GPU runtime.
        std::thread::scope(|scope| self.render_scoped(scope))
    }

    pub(crate) fn render_scoped<'scope>(self, scope: &'scope std::thread::Scope<'scope, '_>) -> Result<RenderedShot> {
        let warnings = crate::warnings::Collector::default();
        let _warning_scope = warnings.enter();
        let wall = Instant::now();
        let Self {
            scene: scene_path,
            outputs,
            out,
            at,
            frame_start,
            frame_end,
            resolution,
            profile,
            settings,
            #[cfg(any(test, feature = "test-support"))]
            test_techniques,
            no_write,
            gpu_only,
            diagnostics,
            exr_codec,
            png_depth,
            edit,
            stop,
        } = self;
        ensure_usage!(
            !out.is_empty(),
            "at least one output destination is required"
        );

        ensure_usage!(png_depth != crate::PngDepth::Sixteen
            || (!no_write && !gpu_only && out.iter().any(|p| !video_destination(p) && (directory_marked(p) || !p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exr"))))
                && (outputs.is_empty() || outputs.contains(&Output::Color))),
            "--png-depth 16 requires PNG colour output");
        if !no_write && !gpu_only {
            for path in &out {
                if video_destination(path) {
                    vork_encode::validate_video_output(path).map_err(setup)?;
                }
            }
        }
        let (requirements, scene_requirements) = std::sync::mpsc::sync_channel::<(bool, bool, bool)>(1);
        let device_worker = scope.spawn(move || {
            Renderer::with_requirements(|adapter| {
                let (explicit, refraction, surface_queries) = scene_requirements.recv().context("scene requirements unavailable")?;
                let info = adapter.get_info();
                Ok(explicit
                    || (refraction && crate::rt::automatic_refraction_supported(info.device_type,adapter.features()))
                    || surface_queries)
            })
                .map_err(setup)
        });
        let parse_start = Instant::now();
        let profile = match profile { Some(profile) => profile, None => vork_scene::Profile::load(vork_scene::DEFAULT_PROFILE)? };
        if profile.level == 3 {
            eprintln!("Hint: std/previs omits indirect light; interiors lit by bounced light can look black. Use --profile std/path for full bounce lighting (or std/draft for a quick approximation).");
        }
        let profile_record = serde_json::json!({"name": profile.name, "version": profile.version, "level": profile.level});
        let overrides = vork_scene::CallOverrides::resolve(settings)
            .map_err(|error| error.context(FailureKind::Usage))?;
        let settings = profile.resolve(&overrides.settings).map_err(|error| error.context(FailureKind::Usage))?;
        // A library draft or final profile denoises only where that is fast; an
        // explicit denoise value or profile file asks for it regardless of device.
        let default_denoise = (vork_scene::libraries::profile(&profile.name).is_some() && profile.level <= 1 && !overrides.settings.contains_key("denoise"))
            .then(|| (profile.name.clone(), profile.level, overrides.settings.contains_key("samples")));
        #[cfg(any(test, feature = "test-support"))]
        let settings = vork_scene::test_settings(&settings, &test_techniques)?;
        let mut scene = crate::load_with_settings(&scene_path, settings, &profile.name, edit)?;
        if let Some(resolution)=overrides.resolution {scene.render.resolution=resolution;}
        if let Some(fps)=overrides.fps {scene.render.fps=fps;}
        scene.validate()?;
        // Instance/adapter discovery overlaps parsing. Only device creation waits
        // for the scene's technique requirements; errors drop the sender and join.

        if let Some(resolution) = resolution {
            ensure_usage!(
                resolution.iter().all(|&v| v > 0),
                "resolution must be positive"
            );
            scene.render.resolution = resolution;
        }
        let _ = requirements.send((crate::rt::uses_rays(&scene) || crate::rt::possible_transport_queries(&scene), crate::rt::automatic_refraction(&scene), crate::rt::automatic_reflections_requested(&scene) || crate::rt::automatic_indirect_requested(&scene)));
        for output in &outputs {
            if let Output::Aov(name)=output {ensure_usage!(name=="shadow_catcher" || scene.render.aovs.iter().any(|a| &a.name==name),"unknown AOV output {name}");}
        }
        let outputs = resolve_outputs(outputs, &scene.render.aovs);
        let scene_parse_ms = parse_start.elapsed().as_secs_f64() * 1000.;
        let authored_at=at;
        let at=authored_at.as_ref().map(|time|vork_time::Request::from(time).numeric()).transpose().map_err(|error|error.context(FailureKind::Usage))?;
        if let Some(t) = at {
            ensure_usage!(
                t.is_finite() && authored_at.as_ref().is_none_or(|t|t.exact()>=Default::default()),
                "--at must be nonnegative and finite"
            );
        }
        let [w, h] = scene.render.resolution;
        let fps = scene.render.fps;
        // A trailing separator always names a directory, whatever its extension.
        let has_format = |path: &PathBuf, extension: &str| {
            !directory_marked(path) && path.extension().is_some_and(|s| s.eq_ignore_ascii_case(extension))
        };
        let is_video = |path: &PathBuf| video_destination(path);
        let is_exr = |path: &PathBuf| has_format(path, "exr");
        // One still: colour at the path, other channels beside it as NAME.<channel>.png.
        let is_png = |path: &PathBuf| has_format(path, "png");
        let is_file = |path: &PathBuf| is_exr(path) || is_png(path);
        for path in &out {
            ensure_usage!(
                recognized_destination(path),
                "--out {}: unrecognized extension; use .png (one still), .exr (with %06d), .mp4, .mov, or a directory (no extension, or end the path with /)",
                path.display()
            );
        }
        let video_paths: Vec<_> = out.iter().filter(|p| is_video(p)).collect();
        let directory_paths: Vec<_> = out.iter().filter(|p| !is_video(p)).collect();
        let video = !video_paths.is_empty();
        let clip_frames = clip_frame_count(&scene.render)?;
        if directory_paths.iter().any(|p|is_exr(p)) {
            i32::try_from(fps.numerator()).context("EXR rate numerator exceeds i32")?;
            u32::try_from(fps.denominator()).context("EXR rate denominator exceeds u32")?;
        }
        for path in &directory_paths {
            ensure_usage!(
                !is_png(path) || !has_frame_pattern(path),
                "--out {}: PNG frame patterns are unsupported; use a directory (every frame as NNNNNN.png) or %06d.exr",
                path.display()
            );
        }
        // A .png is one still; with no time or range given it renders t=0, like Blender's
        // current frame. Mixed with frame directories or EXR sequences it stays an error.
        let implicit_still = at.is_none() && frame_start.is_none() && frame_end.is_none() && clip_frames > 1
            && !directory_paths.is_empty() && directory_paths.iter().all(|p| is_png(p));
        let at = if implicit_still { Some(0.) } else { at };
        ensure_usage!(at.is_none() || (frame_start.is_none() && frame_end.is_none()),
            "--at selects one still; it cannot be combined with --frame-start or --frame-end");
        let first_frame = frame_start.unwrap_or(0);
        let last_frame = frame_end.unwrap_or(clip_frames.saturating_sub(1));
        ensure_usage!(first_frame <= last_frame && last_frame < clip_frames,
            "frame range {first_frame}..{last_frame} must lie within the clip's frames 0..{} (render.fps x render.duration) with start <= end",
            clip_frames.saturating_sub(1));
        let frames = if at.is_some() && !video {
            1
        } else {
            last_frame + 1 - first_frame
        };
        let in_flight = FRAMES_IN_FLIGHT.min(frames);
        let ring_depth = constants::OUTPUT_RING_DEPTH;
        let ring_entries = (in_flight * ring_depth).min(frames);
        let still_pass = video && at.is_some() && !directory_paths.is_empty();
        // Loop positions count from the range's first frame; files, sidecars and
        // times use absolute frame indices.
        let index = |frame: usize| first_frame + frame;
        // Frame identity stays integer; evaluation receives numeric seconds.
        let frame_request=|frame:usize| -> vork_time::Request {
            if video || at.is_none() {vork_time::Request::Frame(index(frame) as u32,fps)}
            else {authored_at.as_ref().map(vork_time::Request::from).unwrap_or(vork_time::Request::Numeric(0.))}
        };
        let frame_time = |frame: usize| frame_request(frame).numeric().expect("validated output time");
        let still_request=|| authored_at.as_ref().map(vork_time::Request::from).unwrap_or(vork_time::Request::Numeric(0.));
        let still_index=at.map(|_| -> Result<u32> {
            let frame=still_request().exact()?.scale(&vork_time::exact::decimal(&fps.to_string())?)
                .add(&vork_time::exact::Germ::point(vork_time::exact::decimal("1/2")?)).floor();
            u32::try_from(frame).context("still frame index exceeds u32 identity capacity").context(FailureKind::Usage)
        }).transpose()?;
        let output_index = |frame: usize| if video || at.is_none() { index(frame) as u32 } else { still_index.unwrap() };
        for (i, path) in out.iter().enumerate() {
            ensure_usage!(
                !out[..i].contains(path),
                "duplicate output path: {}",
                path.display()
            );
        }
        if video {
            ensure_usage!(
                at.is_none() || !directory_paths.is_empty(),
                "--at with video requires a .png, directory or EXR still destination"
            );
            ensure_usage!(
                !no_write && !gpu_only,
                "video output cannot be combined with --no-write or --gpu-only"
            );
            ensure_usage!(
                fps.numerator() <= i32::MAX as i64 && fps.denominator() <= i32::MAX as i64,
                "video output rate exceeds encoder limits"
            );
            ensure_usage!(w % 2 == 0 && h % 2 == 0, "video output requires even dimensions");
        }
        if video {
            i64::try_from(last_frame - first_frame)?.checked_mul(fps.denominator()).context("video timestamp overflow")?;
        }
        for path in &video_paths {
            ensure_usage!(!path.is_dir(), "video output path is a directory: {}", path.display());
            std::fs::create_dir_all(
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?;
        }
        for path in &directory_paths {
            ensure_usage!(
                !is_png(path) || frames == 1 || still_pass,
                "--out {}: a PNG file is one still; add --at SECONDS, or use a directory for every frame",
                path.display()
            );
            if is_png(path) {
                std::fs::create_dir_all(
                    path.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new(".")),
                )?;
            } else if is_exr(path) {
                ensure_usage!(
                    path.file_name().unwrap().to_string_lossy().contains("%06d"),
                    "EXR destination requires a %06d frame placeholder"
                );
                std::fs::create_dir_all(
                    path.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new(".")),
                )?;
            } else {
                std::fs::create_dir_all(path)?;
            }
        }
        let output_run = if no_write || gpu_only {
            None
        } else {
            // Explicit selections replace only their numbered files. A still
            // alongside video uses its own absolute identity, not the video range.
            let selected = if let Some(index) = still_index {
                Some(index..=index)
            } else if frame_start.is_some() || frame_end.is_some() {
                Some(first_frame as u32..=last_frame as u32)
            } else {
                None
            };
            Some(crate::output_run::OutputRun::begin(&out, selected)?)
        };
        let outputs = Outputs::new(outputs);
        let readback = outputs.readback(
            directory_paths.iter().any(|p| !is_exr(p)),
            directory_paths.iter().any(|p| is_exr(p)),
            png_depth,
        );
        // GPU-only frames and directory-less video read nothing back, except a still pass.
        let install_readback = if still_pass || !(gpu_only || (video && directory_paths.is_empty()))
        {
            readback
        } else {
            crate::ReadbackOutputs::default()
        };
        let (renderer, mut installed, mut encoders, encoder_prepare_ms, encoder_attach_ms, foreground) =
            std::thread::scope(|prepare_scope| -> Result<_> {
                // Codec/driver preparation needs no render device. Begin it
                // before joining device creation; stills spawn no worker.
                let prepare = video.then(|| {
                    prepare_scope.spawn(|| -> Result<_> {
                        let start = Instant::now();
                        let prepared = video_paths
                            .iter()
                            .map(|path| {
                                Encoder::prepare(
                                    Backend::Auto,
                                    w,
                                    h,
                                    fps,
                                    EncodeSettings { diagnostics, ..EncodeSettings::default() },
                                    path,
                                ).map_err(setup)
                            })
                            .collect::<Result<Vec<_>>>()?;
                        Ok((prepared, start.elapsed().as_secs_f64() * 1000.))
                    })
                });
                vork_slang::progress::phase("Render: preparing scene, shaders and encoder", 0);
                let renderer = match device_worker
                    .join()
                    .map_err(|_| anyhow::anyhow!("device creation worker panicked"))
                    .and_then(|result| result)
                {
                    Ok(renderer) => renderer,
                    Err(error) => {
                        // Consume the scoped handle even on device failure so a
                        // preparation panic cannot replace the original error.
                        if let Some(worker) = prepare {
                            let _ = worker.join();
                        }
                        return Err(error);
                    }
                };
                if diagnostics {
                    eprintln!(
                        "device: {:?}; pipeline cache supported: {}",
                        renderer.adapter_info,
                        renderer.pipeline_cache_supported()
                    );
                }
                // Keep optional variants behind install AND frame-slot pipelines.
                let foreground = renderer.compiler.foreground_preparation();
                let (installed, encoders, encoder_prepare_ms, encoder_attach_ms) =
                    std::thread::scope(|scope| -> Result<_> {
                        let install = scope.spawn(|| {
                            let _warning_scope = warnings.enter();
                            let mut scene = scene;
                            if let Some((profile, level, keep_samples)) = &default_denoise {
                                crate::denoise::resolve_default(&renderer, &mut scene, profile, *level, *keep_samples);
                            }
                            renderer.install_with_options(
                                scene,
                                scene_path.parent().unwrap_or(Path::new(".")),
                                crate::InstallOptions {
                                    initial_time: frame_request(0),
                                    readback: install_readback,
                                    gpu_color: video || gpu_only,
                                    variants: if frames == 1 { crate::pipelines::VariantCompile::Fallback } else { Default::default() },
                                    ..Default::default()
                                },
                            )
                            .with_context(|| format!("install scene {}", scene_path.display()))
                        });
                        let (prepared, preparation_ms) = match prepare {
                            Some(worker) => worker.join().map_err(|_| {
                                anyhow::anyhow!("encoder preparation worker panicked")
                            })??,
                            None => (Vec::new(), 0.),
                        };
                        let attach_start = Instant::now();
                        // Attach is the longer branch; keep pipeline creation from contending.
                        renderer.compiler.set_critical(!prepared.is_empty());
                        let encoders = prepared
                            .into_iter()
                            .map(|prepared| {
                                prepared.open(
                                    renderer.device.clone(),
                                    renderer.queue.clone(),
                                    ring_entries,
                                ).map_err(setup)
                            })
                            .collect::<Result<Vec<_>>>();
                        renderer.compiler.set_critical(false);
                        let encoders = encoders?;
                        let encoder_attach_ms = attach_start.elapsed().as_secs_f64() * 1000.;
                        let installed = install
                            .join()
                            .map_err(|_| anyhow::anyhow!("scene install worker panicked"))??;
                        validate_clock_requests(&installed,(0..frames).map(&frame_request)
                            .chain(still_pass.then(&still_request)))?;
                        Ok((installed, encoders, preparation_ms, encoder_attach_ms))
                    })?;
                Ok((
                    renderer,
                    installed,
                    encoders,
                    encoder_prepare_ms,
                    encoder_attach_ms,
                    foreground,
                ))
            })?;
        let effective = &installed.render().settings;
        let adaptive = installed.path.as_ref().is_some_and(|path| path.adaptive());
        let denoise = effective.integrator.denoiser().is_some();
        let render_settings = serde_json::json!({
            "profile": profile_record, "samples": effective.samples,
            "adaptive": adaptive, "denoise": denoise,
            "resolution": [w, h], "frames": frames,
        });
        eprintln!("render: {}; {}{} samples ({}); denoise {}; {}x{}; {} frames",
            profile.name, if adaptive { "up to " } else { "" }, effective.samples,
            if adaptive { "adaptive" } else { "fixed" },
            if denoise { "on" } else { "off" }, w, h, frames);
        let encoder_readback = video.then(|| encoders.iter().any(|e| e.uses_pixel_readback()));
        let mut slots = (0..in_flight)
            .map(|_| {
                installed.create_frame_slot(
                    &renderer,
                    if directory_paths.iter().any(|p| is_exr(p)) {
                        crate::ReadbackPrecision::Float
                    } else {
                        crate::ReadbackPrecision::Packed
                    },
                )
            })
            .collect::<Result<Vec<_>>>()?;
        // Install in frame order so native entry indices match the output ring.
        for phase in 0..ring_depth {
            for (index, slot) in slots.iter_mut().enumerate() {
                if phase * in_flight + index >= ring_entries {
                    continue;
                }
                if phase != 0 {
                    let spare = crate::ring::Entry::new(
                        &renderer.device,
                        &slot.view.post.pixels,
                        u64::from(w) * u64::from(h) * 4,
                        slot.view.float_output,
                        installed.readback,
                        slot.view.aovs.len(),
                        slot.view.post.display_base,
                    );
                    let previous = std::mem::replace(&mut slot.ring, spare);
                    slot.spare_ring.push_back(previous);
                }
                for encoder in &mut encoders {
                    slot.ring.encoder_surfaces.push(encoder.create_surface()?);
                }
                if let Some(output) = slot
                    .ring
                    .encoder_surfaces
                    .first()
                    .and_then(|s| s.texture.as_ref())
                {
                    if output.format() == renderer.output_format() {
                        slot.set_output(&installed, &renderer, output.clone())?;
                        slot.ring.output =
                            Some((slot.view.output.clone(), slot.view.output_bind.clone()));
                    }
                }
            }
        }
        for slot in &mut slots {
            slot.advance_ring();
        }
        let png_outputs: Vec<_> = if no_write || gpu_only {
            Vec::new()
        } else {
            directory_paths
                .iter()
                .map(|path| {
                    FrameOutputWriter::new(
                        (*path).clone(),
                        w,
                        h,
                        outputs.clone(),
                        is_exr(path),
                        exr_codec,
                        png_depth,
                        ring_entries,
                        if still_pass { 0 } else { first_frame },
                        if is_exr(path) {(fps.numerator() as i32,fps.denominator() as u32)} else {(1,1)},
                    )
                })
                .collect()
        };
        let png_workers: usize = png_outputs.iter().map(|p| p.worker_count).sum();
        // Frame slots add required pipelines after install's checkpoint. Save them
        // while foreground admission still holds optional workers back; after the
        // first frame their driver work can keep the final idle save from running.
        if installed.deferred_variants() > 0 {
            renderer.save_idle_pipelines().map_err(device)?;
        }
        let mut foreground = Some(foreground);
        let ready_ms = wall.elapsed().as_secs_f64() * 1000.;
        vork_slang::progress::phase("Rendering frames", frames);
        let loop_start = Instant::now();
        let mut samples = Vec::new();
        let mut metadata_frames = Vec::new();
        let mut camera_frames = Vec::new();
        let mut still_camera = None;
        let mut still_exposure = None;
        let mut still_dropped_light_samples = 0;
        let mut still_dropped_path_samples = 0;
        let mut still_terminated_medium_paths = 0;
        let mut still_discarded_medium_radiance = 0.;
        let mut frame_exposures = Vec::new();
        let mut first_frame_ms = 0.;
        let mut submit_cpu = vec![None; slots.len()];
        let mut output_release_ms = vec![0.; slots.len()];
        // Frames the shader refused (non-finite radiance, say). A video cannot
        // skip a frame; per-frame files can, so the rest of the range renders.
        let mut refused: Vec<(u32, String)> = Vec::new();
        let mut redos = 0u32;
        let mut redo_ms = 0.;
        for slot in &mut slots {
            slot.profile = diagnostics;
            slot.write_color = video || gpu_only;
        }
        let rendered = (|| -> Result<_> {
        let mut memory_frames = Vec::new();
        let mut frame = 0;
        let mut next_submit = 0;
        let mut redo_started = None;
        while frame < frames {
            let stopping = stop.as_ref().is_some_and(|stop| stop.load(std::sync::atomic::Ordering::Relaxed));
            if stopping && frame > 0 && frame == next_submit { break; }
            let attempt = (|| -> Result<_> {
                // Deliver the first accepted frame before recording successors:
                // advancing their simulations can take minutes on the host.
                // Once output has started, restore the normal overlap window.
                let submit_limit = frame + if samples.is_empty() { 1 } else { in_flight };
                while next_submit < frames && next_submit < submit_limit {
                    if stopping && next_submit > 0 { break; }
                    let i = next_submit % slots.len();
                    let slot = &mut slots[i];
                    let cpu_start = thread_cpu_ms();
                    // `ring.frame` is a delivered frame the entry still holds. Advance
                    // past it once, and free the next entry's frame before reuse; a
                    // redone frame's entry was never delivered, so resubmission keeps
                    // it and frame f stays on encoder entry f % ring_entries.
                    let release_start = Instant::now();
                    if slot.ring.frame.is_some() {
                        slot.advance_ring();
                        if let Some(previous) = slot.ring.frame.take() {
                            for (encoder, surface) in encoders.iter_mut().zip(&mut slot.ring.encoder_surfaces) {
                                encoder.release(previous as u64, surface).map_err(device)?;
                            }
                        }
                    }
                    output_release_ms[i] = release_start.elapsed().as_secs_f64() * 1000.;
                    if gpu_only || (video && (directory_paths.is_empty() || still_pass)) {
                        slot.submit_indexed_outputs(&installed, &renderer, output_index(next_submit), frame_request(next_submit), crate::ReadbackOutputs::default()).map_err(device)?;
                    } else {
                        slot.submit_indexed_outputs(&installed, &renderer, output_index(next_submit), frame_request(next_submit), readback).map_err(device)?;
                    }
                    submit_cpu[i] = cpu_elapsed(cpu_start);
                    next_submit += 1;
                }
                let slot = &mut slots[frame % in_flight];
                let submission = slot.submission.as_ref().expect("submitted frame").clone();
                let cpu_start = thread_cpu_ms();
                let collect_start = Instant::now();
                let p = slot.collect_readback(&renderer).map_err(device)?;
                Ok((p, submission, cpu_start, collect_start))
            })();
            let (p, submission, cpu_start, collect_start) = match attempt {
                Ok(result) => result,
                Err(error) => {
                    if let Some(refusal) = error.downcast_ref::<crate::FrameRefused>().filter(|_| !video && installed.transients.check_recording().is_ok()) {
                        // Only an installation whose publications remain valid can
                        // skip an isolated refusal. Failed retained producers require
                        // reinstallation, so their queued successors cannot be delivered.
                        eprintln!("warning: frame {} refused and not written: {refusal}", output_index(frame));
                        refused.push((output_index(frame), refusal.0.clone()));
                        if !still_pass {
                            for writer in &png_outputs { writer.skip(index(frame))?; }
                        }
                        frame += 1;
                        vork_slang::progress::completed(frame);
                        continue;
                    }
                    let Some(redo) = error.downcast_ref::<crate::FrameRedo>() else { return Err(error); };
                    redo_started.get_or_insert_with(Instant::now);
                    // The exposure's own frame keys its shadow requests: a
                    // still at `at` is not loop frame `index(frame)`.
                    let exposure = output_index(frame) as usize;
                    redos += installed.redo_frame(&renderer, &mut slots, redo.0, exposure)?;
                    next_submit = frame;
                    continue;
                }
            };
            if let Some(started) = redo_started.take() { redo_ms += started.elapsed().as_secs_f64() * 1000.; }
            path_degradation_warnings(output_index(frame), p.timing.terminated_medium_paths,
                p.timing.discarded_medium_radiance, p.timing.dropped_path_samples);
            if let Some(warning) = dropped_light_warning(output_index(frame), p.timing.dropped_light_samples) { eprintln!("{warning}"); }
            let i = frame % slots.len();
            let slot = &mut slots[i];
            let collect_wall_ms = collect_start.elapsed().as_secs_f64() * 1000.;
            // Retain main's CPU/GPU overlap decision; measured sample-group
            // limits still apply independently to every queue submission.
            slot.stream_samples = frame >= in_flight && collect_wall_ms < p.timing.cpu_submit_ms;
            // Portable encoders map the copy already recorded in this ring entry.
            // Deliver after collection, with later frames already queued; wait only
            // for this frame rather than draining unrelated work.
            // The existing final display reduction observes the exact RGBA8
            // image delivered to the encoder, after every compositor entry.
            // Check each accepted frame, including animated alpha, before push.
            if video_paths.iter().any(|path|has_format(path,"mp4")) {
                ensure_usage!(p.exposure.is_some_and(|e|e.coverage==1.),
                    "MP4 requires an opaque final image; transparency requires .mov (ProRes 4444)");
            }
            let encode_push_ms = push_encoders(&mut encoders, slot, frame, &submission)?;
            if diagnostics && (frame == 0 || frame + 1 == frames) {
                if let Some(memory) = renderer.device.generate_allocator_report() {
                    memory_frames.push(serde_json::json!({"frame": index(frame),
                        "allocated_bytes": memory.total_allocated_bytes,
                        "reserved_bytes": memory.total_reserved_bytes}));
                }
            }
            if video || outputs.has(Output::Camera) {
                camera_frames.push(p.camera);
            }
            let cpu_thread_ms = submit_cpu[i]
                .zip(cpu_elapsed(cpu_start))
                .map(|(a, b)| a + b);
            if samples.is_empty() {
                first_frame_ms = loop_start.elapsed().as_secs_f64() * 1000.;
                drop(foreground.take());
            }
            metadata_frames.push(serde_json::json!({"index":output_index(frame),"time":frame_time(frame),"camera":installed.camera_name_at_request(&frame_request(frame))?,"depth_unit_m":p.camera.depth_unit_m,"terminated_medium_paths":p.timing.terminated_medium_paths,"discarded_medium_radiance":p.timing.discarded_medium_radiance,"dropped_path_samples":p.timing.dropped_path_samples,"dropped_light_samples":p.timing.dropped_light_samples,"exposure":p.exposure.map(|e|e.json())}));
            if let Some(e) = p.exposure { frame_exposures.push((index(frame), e)); }
            samples.push(serde_json::json!({"frame":index(frame),"time":frame_time(frame),"cpu_thread_ms":cpu_thread_ms,"encode_push_wall_ms":encode_push_ms,"output_release_wall_ms":output_release_ms[i],"encode_pipe_wait_wall_ms":slot.ring.encoder_surfaces.iter().map(|s|s.pipe_wait_ms()).sum::<f64>(),"collect_wall_ms":collect_wall_ms,"cpu_prepare_ms":p.timing.cpu_prepare_ms,"cpu_submit_ms":p.timing.cpu_submit_ms,"cpu_finish_ms":p.timing.cpu_finish_ms,"denoise_wait_ms":p.timing.denoise_wait_ms,"denoise_pack_ms":p.timing.denoise_pack_ms,"denoise_filter_ms":p.timing.denoise_filter_ms,"denoise_upload_ms":p.timing.denoise_upload_ms,"timing_source":diagnostics.then_some(p.timing.timing_source),"gpu_ms":diagnostics.then_some(p.timing.gpu_ms),"gpu_elapsed_ms":diagnostics.then_some(p.timing.gpu_elapsed_ms),"gpu_spans_ms":diagnostics.then_some(&p.timing.gpu_spans_ms),"nonfinite":p.timing.nonfinite,"terminated_medium_paths":p.timing.terminated_medium_paths,"discarded_medium_radiance":p.timing.discarded_medium_radiance,"dropped_path_samples":p.timing.dropped_path_samples,"dropped_light_samples":p.timing.dropped_light_samples,"sample_count":p.timing.sample_count}));
            if !still_pass {
                for writer in &png_outputs { writer.push(index(frame), p.clone())?; }
            }
            frame += 1;
            vork_slang::progress::completed(frame);
        }
        vork_slang::progress::phase("Render: finishing encoders and writing files", 0);
        let finish_start = Instant::now();
        for encoder in &mut encoders {
            encoder.finish().map_err(device)?;
        }
        let encoder_finish_ms = if video {
            finish_start.elapsed().as_secs_f64() * 1000.
        } else {
            0.
        };
        if still_pass {
            vork_slang::progress::phase("Rendering selected still", 1);
            let still_frame = still_index.unwrap() as usize;
            let pixels = loop {
                let slot = &mut slots[0];
                slot.write_color = false;
                let attempt = slot.submit_indexed_outputs(&installed, &renderer, still_index.unwrap(), still_request(), readback)
                    .and_then(|()| slot.collect_readback(&renderer));
                match attempt {
                    Ok(pixels) => break pixels,
                    Err(error) => {
                        let Some(redo) = error.downcast_ref::<crate::FrameRedo>() else { return Err(error); };
                        let started = Instant::now();
                        redos += installed.redo_frame(&renderer, &mut slots, redo.0, still_frame)?;
                        redo_ms += started.elapsed().as_secs_f64() * 1000.;
                    }
                }
            };
            vork_slang::progress::completed(1);
            still_dropped_light_samples = pixels.timing.dropped_light_samples;
            still_dropped_path_samples = pixels.timing.dropped_path_samples;
            still_terminated_medium_paths = pixels.timing.terminated_medium_paths;
            still_discarded_medium_radiance = pixels.timing.discarded_medium_radiance;
            path_degradation_warnings(still_index.unwrap(), still_terminated_medium_paths,
                still_discarded_medium_radiance, still_dropped_path_samples);
            if let Some(warning) = dropped_light_warning(still_index.unwrap(), still_dropped_light_samples) { eprintln!("{warning}"); }
            still_exposure = pixels.exposure;
            still_camera = Some(pixels.camera);
            for writer in &png_outputs {
                writer.push(0, pixels.clone())?;
            }
        }
        Ok((encoder_finish_ms, memory_frames))
        })();
        let mut output_result = Ok(());
        vork_slang::progress::phase("Render: writing files and reports", 0);
        for writer in png_outputs {
            let result = writer.finish();
            if output_result.is_ok() { output_result = result; }
        }
        // Input decoders can still be blocked writing their next frame. Finish
        // them at the shot boundary, before a one-shot caller skips GPU teardown.
        for footage in installed.footage.values_mut() { footage.finish(); }
        let (encoder_finish_ms, memory_frames) = rendered?;
        output_result?;
        let save_start = Instant::now();
        renderer.save_idle_pipelines().map_err(device)?;
        let pipeline_save_ms = save_start.elapsed().as_secs_f64() * 1000.;
        // The other frames are written; withholding meta.json leaves the output
        // marked incomplete, and the exit status reports the refusal.
        if !refused.is_empty() {
            let mut reasons: Vec<&str> = refused.iter().map(|(_, reason)| reason.as_str()).collect();
            reasons.sort_unstable();
            reasons.dedup();
            let list = refused.iter().map(|(index, _)| index.to_string()).collect::<Vec<_>>().join(", ");
            return Err(device(anyhow::anyhow!("{} of {frames} frames refused and not written ({}): {list}; the other frames were written, without meta.json",
                refused.len(), reasons.join("; "))));
        }
        let interrupted = stop.as_ref().is_some_and(|stop| stop.load(std::sync::atomic::Ordering::Relaxed));
        let frames = if interrupted { metadata_frames.len() } else { frames };
        let mut sidecars = Vec::new();
        for path in &video_paths {
            let camera_path = sidecar(path, "camera.json");
            std::fs::write(
                &camera_path,
                serde_json::to_vec_pretty(&crate::camera::clip(&camera_frames, fps, Some(first_frame))?)?,
            )?;
            sidecars.push(camera_path);
        }
        if outputs.has(Output::Camera) && !no_write && !gpu_only {
            let frames = still_camera
                .as_ref()
                .map(std::slice::from_ref)
                .unwrap_or(&camera_frames);
            let bytes = serde_json::to_vec_pretty(&crate::camera::clip(frames, fps, at.is_none().then_some(first_frame))?)?;
            for path in &directory_paths {
                let camera_path = if is_file(path) {
                    sidecar(path, "camera.json")
                } else {
                    path.join("camera.json")
                };
                if let Some(parent) = camera_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&camera_path, &bytes)?;
                sidecars.push(camera_path);
            }
        }
        if implicit_still && !no_write && !gpu_only {
            eprintln!("rendered t=0; use --at SECONDS for another time, or a directory for every frame");
        }
        let mut report = serde_json::json!({"version":3,"device":renderer.adapter_info.name,"backend":format!("{:?}",renderer.adapter_info.backend),"pipeline_cache_supported":renderer.pipeline_cache_supported(),"pipeline_cache_status":renderer.pipeline_cache_status,"pipeline_cache_input_bytes":renderer.pipeline_cache_input_bytes,"pipeline_cache_rejection":renderer.pipeline_cache_rejection,"frames":frames,"frame_start":at.is_none().then_some(first_frame),"still_time":at,"resolution":[w,h],"png_write":!no_write && !gpu_only && !directory_paths.is_empty(),"video_write":video,"output":out,"renderer_pixel_readback":readback.any() && (!gpu_only && !directory_paths.is_empty()),"encoder_pixel_readback":encoder_readback,"encoder_open_ms":encoder_prepare_ms+encoder_attach_ms,"encoder_prepare_ms":encoder_prepare_ms,"encoder_attach_ms":encoder_attach_ms,"encoder_finish_ms":encoder_finish_ms,"scene_parse_ms":scene_parse_ms,"pipeline_ms":installed.pipeline_ms,"pipeline_creation_ms":renderer.pipeline_timings(),"environment_install_ms":installed.environment_install_ms,"material_bind_ms":installed.material_bind_ms,"asset_decode_ms":installed.asset_decode_ms,"texture_upload_ms":installed.texture_upload_ms,"texture_cache_hits":installed.texture_cache_hits,"first_frame_ms":first_frame_ms,"device_ms":renderer.startup.as_secs_f64()*1000.,"device_stages_ms":renderer.startup_stages_ms,"geometry_vertex_bytes":installed.geometry_vertex_bytes,"install_ms":installed.install_time.as_secs_f64()*1000.,"ready_ms":ready_ms,"loop_wall_ms":loop_start.elapsed().as_secs_f64()*1000.,"wall_ms":wall.elapsed().as_secs_f64()*1000.,"frames_in_flight":in_flight,"transient_bytes":installed.transient_bytes(),"gpu_timing_readback":true,"png_workers":png_workers,"samples":samples});
        // Background feature variants that initially drew with a superset fallback.
        report["render_settings"] = render_settings.clone();
        report["shader_creation"] = serde_json::to_value(crate::timing::creations(&renderer.device))?;
        report["frame_redos"] = redos.into();
        report["frame_redo_ms"] = redo_ms.into();
        report["deferred_variants"] = installed.deferred_variants().into();
        if diagnostics {
            report["memory_frames"] = memory_frames.into();
            report["retained_gpu_bytes"] = renderer.residency.retained_bytes().into();
            report["camera_memory_bytes"] = renderer.camera_memory.into();
            report["camera_record_budget_bytes"] =
                (renderer.camera_memory / constants::CAMERA_RECORD_MEMORY_SHARE).into();
            report["camera_memory_override_lowered"] = renderer.camera_memory_override_lowered.into();
            report["probe_field"] = installed.probes.as_ref().map(|p| p.diagnostics()).into();
            report["compiled_pipeline_count"] = renderer.pipeline_timings().len().into();
            report["program_pipeline_count"] = renderer.program_pipelines.lock().unwrap().len().into();
        }
        report["pipeline_save_ms"] = pipeline_save_ms.into();
        // The profile asked for intent; `settings` records the techniques this
        // device actually ran (automatic choices resolved at install).
        let metadata = |frames: &[serde_json::Value]| serde_json::json!({
            "version": 3, "fps": fps, "frames": frames,
            "object_ids": &*installed.object_ids,
            "warnings": warnings.snapshot(),
            "profile": profile_record, "settings": installed.render().settings,
            "render_settings": render_settings,
            "denoiser": installed.render().settings.integrator.denoiser().and_then(|_| crate::denoise::device_kind(&renderer)),
        });
        let mut exposures = Vec::new();
        let mut pending_metadata = Vec::new();
        for path in out.iter().filter(|_| output_run.is_some()) {
            let file_path = |suffix: &str| {
                if is_video(path) || is_file(path) { sidecar(path, suffix) }
                else { path.join(suffix) }
            };
            let mut frames = if !is_video(path) && still_pass {
                vec![serde_json::json!({"index":still_index.unwrap(),"time":at.unwrap(),"camera":installed.camera_name_at_request(&still_request())?,
                    "depth_unit_m":still_camera.as_ref().unwrap().depth_unit_m,"terminated_medium_paths":still_terminated_medium_paths,"discarded_medium_radiance":still_discarded_medium_radiance,"dropped_path_samples":still_dropped_path_samples,"dropped_light_samples":still_dropped_light_samples,"exposure":still_exposure.map(|e: crate::Exposure|e.json())})]
            } else { metadata_frames.clone() };
            let still_rows: Vec<_> = still_exposure.map(|e| (0, e)).into_iter().collect();
            let mut rows = if !is_video(path) && still_pass { &still_rows[..] } else { &frame_exposures[..] };
            // Only a destination that writes the display image gets its report: video, or PNG
            // colour. EXR holds scene-linear values; control-only directories hold no colour.
            if !(is_video(path) || (!is_exr(path) && outputs.has(Output::Color))) {
                rows = &[];
                for frame in &mut frames { frame["exposure"] = serde_json::Value::Null; }
            }
            let meta_path = file_path("meta.json");
            if let Some(parent) = meta_path.parent() { std::fs::create_dir_all(parent)?; }
            let mut meta = metadata(&frames);
            if let Some(summary) = exposure_summary(rows) {
                meta["exposure"] = summary.clone();
                exposures.push(serde_json::json!({"output": path, "exposure": summary}));
            }
            pending_metadata.push((meta_path.clone(), serde_json::to_vec_pretty(&meta)?));
            sidecars.push(meta_path);
            if diagnostics {
                let timing_path = file_path("timings.json");
                std::fs::write(&timing_path, serde_json::to_vec_pretty(&report)?)?;
                sidecars.push(timing_path);
            }
        }
        report["interrupted"] = interrupted.into();
        report["warnings"] = serde_json::to_value(warnings.snapshot())?;
        report["sidecars"] = serde_json::to_value(sidecars)?;
        report["exposure"] = exposures.into();
        if let Some(run) = &output_run { run.complete(&pending_metadata)?; }
        Ok(RenderedShot {
            report,
            _encoders: encoders,
            _slots: slots,
            _scene: installed,
            _renderer: renderer,
        })
    }
}

// Called only after successful collection, so retries and refused frames cannot
// publish degradation warnings for an image that was never delivered.
fn path_degradation_warnings(frame: u32, terminated_medium_paths: u64, discarded_medium_radiance: f64, dropped_path_samples: u64) {
    if terminated_medium_paths > 0 {
        crate::warnings::dropped("fog_watchdog", serde_json::json!({
            "frame": frame, "terminated_medium_paths": terminated_medium_paths,
            "discarded_medium_radiance": discarded_medium_radiance,
        }), format!("frame {frame} ended {terminated_medium_paths} fog paths at the watchdog; discarded accumulated absolute-luminance sum {discarded_medium_radiance}; untraced continuation energy is unknown. Object, pixel and operation attribution is unavailable. Simplify or remove fog in this frame and rerender; verify terminated_medium_paths is zero before accepting the result."));
    }
    if dropped_path_samples > 0 {
        crate::warnings::dropped("nonfinite_path_samples", serde_json::json!({
            "frame": frame, "dropped_path_samples": dropped_path_samples,
        }), format!("frame {frame} dropped {dropped_path_samples} non-finite path samples; zero samples remain in the average without renormalization. Object, pixel and operation attribution is unavailable; missing energy is unknown. Check scene inputs and programs for non-finite values and rerender; verify dropped_path_samples is zero before accepting the result."));
    }
}

fn dropped_light_warning(frame: u32, count: u64) -> Option<String> {
    (count != 0).then(|| format!("warning: frame {frame}: {count} non-finite light sample{} dropped", if count == 1 { "" } else { "s" }))
}

/// An output's exposure over its frames: mean shares and luma, and the worst frame for
/// clipping and near-black. None when no frame wrote colour.
fn exposure_summary(rows: &[(usize, crate::Exposure)]) -> Option<serde_json::Value> {
    if rows.is_empty() { return None; }
    let coverage = rows.iter().map(|(_, e)| e.coverage).sum::<f64>() / rows.len() as f64;
    let frames = rows.len();
    let rows: Vec<_> = rows.iter().filter(|(_, e)| e.coverage > 0.).copied().collect();
    if rows.is_empty() {
        return Some(serde_json::json!({"frames": frames, "coverage": 0., "clipped": null, "near_black": null, "mean": null, "worst_clipped": null, "worst_near_black": null}));
    }
    let n = rows.len() as f64;
    let mean = |f: fn(&crate::Exposure) -> f64| rows.iter().map(|(_, e)| f(e)).sum::<f64>() / n;
    let worst = |f: fn(&crate::Exposure) -> f64| {
        let (frame, e) = rows.iter().max_by(|a, b| f(&a.1).total_cmp(&f(&b.1))).unwrap();
        serde_json::json!({"share": f(e), "frame": frame})
    };
    Some(serde_json::json!({
        "frames": frames, "coverage": coverage,
        "clipped": mean(|e| e.clipped), "near_black": mean(|e| e.near_black), "mean": mean(|e| e.mean),
        "worst_clipped": worst(|e| e.clipped), "worst_near_black": worst(|e| e.near_black),
    }))
}

#[cfg(test)]
mod sidecar_tests {
    use super::*;

    #[test]
    fn path_degradation_warnings_preserve_frame_witnesses_and_ignore_clean_frames() {
        let warnings = crate::warnings::Collector::default();
        let _scope = warnings.enter();
        path_degradation_warnings(794, 0, 0., 0);
        assert!(warnings.snapshot().is_empty());
        path_degradation_warnings(795, 2, 1.25, 4294967299);
        path_degradation_warnings(795, 2, 1.25, 4294967299);
        path_degradation_warnings(796, 2, 1.25, 0);
        let records = serde_json::to_value(warnings.snapshot()).unwrap();
        let records = records.as_array().unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["code"], "fog_watchdog");
        assert_eq!(records[0]["requested"], serde_json::json!({
            "frame":795, "terminated_medium_paths":2, "discarded_medium_radiance":1.25
        }));
        assert_eq!(records[1]["code"], "nonfinite_path_samples");
        assert_eq!(records[1]["requested"]["dropped_path_samples"], 4294967299_u64);
        assert_eq!(records[2]["requested"]["frame"], 796);
        for record in records {
            let explanation = record["dropped"].as_str().unwrap();
            assert!(explanation.contains("attribution is unavailable"));
            assert!(explanation.contains("rerender"));
        }
    }

    #[test]
    #[ignore = "requires a supported GPU, pinned Slang and ffmpeg; scheduled explicitly in main CI"]
    fn cancellation_closes_a_partial_sequence_and_records_only_saved_frames() -> Result<()> {
        if crate::test_renderer()?.is_none() { return Ok(()); }
        let root = std::env::temp_dir().join(format!("vork-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let scene = root.join("scene.json");
        std::fs::write(&scene, vork_scene::STARTER_SCENE)?;
        let destination = root.join("frames");
        let rendered = Shot {
            stop: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true))),
            scene, out: vec![destination.clone(), root.join("clip.mov")], resolution: Some([32, 24]),
            frame_start: Some(0), frame_end: Some(3),
            settings: [("samples".into(), serde_json::json!(1)), ("denoise".into(), serde_json::json!(false))].into_iter().collect(),
            ..Default::default()
        }.render()?;
        assert_eq!(rendered.report["interrupted"], true);
        assert_eq!(rendered.report["frames"], 1);
        let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(destination.join("meta.json"))?)?;
        assert_eq!(metadata["frames"].as_array().unwrap().len(), 1);
        assert_eq!(metadata["render_settings"], rendered.report["render_settings"]);
        let settings = &rendered.report["render_settings"];
        assert_eq!(settings["samples"], 1);
        assert_eq!(settings["denoise"], false);
        assert_eq!(settings["resolution"], serde_json::json!([32,24]));
        assert_eq!(settings["frames"], 4); // Requested budget, including on interruption.
        assert_eq!(settings["profile"], metadata["profile"]);

        let probe = std::process::Command::new("ffprobe").args([
            "-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames",
            "-of", "default=noprint_wrappers=1:nokey=1",
        ]).arg(root.join("clip.mov")).output()?;
        assert!(probe.status.success(), "{}", String::from_utf8_lossy(&probe.stderr));
        assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "1");
        // Match the CLI ownership contract: files are finished before GPU owners
        // are discarded by process exit. Dropping here would also wait for optional
        // pipeline preparation, which is separate from cancellation/file closure.
        std::mem::forget(rendered);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn dropped_light_warning_names_frame_and_count() {
        assert_eq!(dropped_light_warning(795, 0), None);
        assert_eq!(dropped_light_warning(795, 1).unwrap(), "warning: frame 795: 1 non-finite light sample dropped");
        assert_eq!(dropped_light_warning(796, 4294967299).unwrap(), "warning: frame 796: 4294967299 non-finite light samples dropped");
    }

    #[test]
    fn exposure_summary_excludes_empty_frames() {
        let empty = crate::Exposure { coverage: 0., clipped: f64::NAN, near_black: f64::NAN, mean: f64::NAN };
        let covered = crate::Exposure { coverage: 0.5, clipped: 0.2, near_black: 0.1, mean: 0.6 };
        let e = exposure_summary(&[(1, empty), (2, covered)]).unwrap();
        assert_eq!(e["frames"], 2);
        assert_eq!(e["coverage"], 0.25);
        assert_eq!(e["mean"], 0.6);
        assert_eq!(e["worst_clipped"]["frame"], 2);
        assert!(exposure_summary(&[(1, empty)]).unwrap()["mean"].is_null());
        assert!(exposure_summary(&[]).is_none());
    }

    #[test]
    fn frame_patterns_only_match_filename_placeholders() {
        for path in ["renders/review#2/still.png", "renders/100%/still.png",
            "renders/####/still.png", "renders/%04d/still.png", "part#1.png",
            "100%.png", "frame_%%04d.png"] {
            assert!(!has_frame_pattern(Path::new(path)), "{path}");
        }
        for path in ["frame_####.png", "frame_%04d.png", "frame_##.png", "frame_%d.png"] {
            assert!(has_frame_pattern(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn output_formats_have_distinct_sidecars() {
        for suffix in ["camera.json", "meta.json", "timings.json"] {
            let mp4 = sidecar(Path::new("out/x.mp4"), suffix);
            let mov = sidecar(Path::new("out/x.mov"), suffix);
            let exr = sidecar(Path::new("out/x.%06d.exr"), suffix);
            assert_eq!(mp4, PathBuf::from(format!("out/x.mp4.{suffix}")));
            assert_eq!(mov, PathBuf::from(format!("out/x.mov.{suffix}")));
            assert_eq!(exr, PathBuf::from(format!("out/x.exr.{suffix}")));
            assert_eq!(sidecar(Path::new("exr/%06d.exr"), suffix), PathBuf::from(format!("exr/{suffix}")));
            assert_eq!(sidecar(Path::new("shot_%06d.exr"), suffix), PathBuf::from(format!("shot.exr.{suffix}")));
            assert_eq!(sidecar(Path::new("out/still.png"), suffix), PathBuf::from(format!("out/still.png.{suffix}")));
            assert_ne!(mp4, mov);
        }
    }

    #[test]
    fn unknown_extensions_are_not_directories() {
        for ok in ["out", "out/frames", "out/still.PNG", "x.%06d.exr", "a.mp4", "a.MOV", "frames.v2/", ".", "..", "out/.hidden"] {
            assert!(recognized_destination(Path::new(ok)), "{ok}");
        }
        for bad in ["still.jpg", "frames.v2", "clip.mp5", "out/x.webp", "shot.json"] {
            assert!(!recognized_destination(Path::new(bad)), "{bad}");
        }
        assert!(directory_marked(Path::new("frames.png/")) && !directory_marked(Path::new("frames.png")));
        for directory in ["frames.mp4/", "frames.MOV/", "frames"] {
            assert!(!video_destination(Path::new(directory)), "{directory}");
        }
        assert!(video_destination(Path::new("clip.MP4")) && video_destination(Path::new("clip.mov")));
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;
    #[test]
    fn default_is_color_and_all_includes_declared_aovs() {
        let aovs = vec![
            vork_scene::v2::Aov { name: "heat".into(), kind: vork_scene::v2::AovType::Value },
            vork_scene::v2::Aov { name: "tint".into(), kind: vork_scene::v2::AovType::Color },
        ];
        let mut expected: Vec<Output> = Output::builtins().collect();
        assert_eq!(resolve_outputs(Shot::default().outputs, &aovs), vec![Output::Color]);
        expected.extend([Output::Aov("heat".into()), Output::Aov("tint".into())]);
        assert_eq!(resolve_outputs(Vec::new(), &aovs), expected);
        for explicit in [vec![Output::Color], vec![Output::Aov("heat".into())]] {
            assert_eq!(resolve_outputs(explicit.clone(), &aovs), explicit);
        }
    }
}

/// Admission of the finite output request set, before any replay or exposure.
/// Simulation and shutter consumers need this traversal; ordinary animation
/// does not get an extra evaluation. Each shutter envelope is derived independently from its
/// nominal integer identity and existing numeric shutter controls.
fn validate_clock_requests(scene:&InstalledScene,requests:impl Iterator<Item=vork_time::Request>)->Result<()> {
    let compute=scene.compute.as_ref().filter(|compute|compute.has_simulation());
    let animated_shutter=scene.animation.has_shutter_animation();
    if compute.is_none() && !animated_shutter && scene.render.motion_blur.as_ref().is_none_or(|m| m.shutter == 0.) {return Ok(())}
    use vork_time::exact::{Germ,decimal,ieee};
    let mut samples=scene.animation.samples();
    let rate=decimal(&scene.render.fps.to_string())?;
    for request in requests {
        request.numeric()?;
        if let Some(compute)=compute {compute.validate_request(&request)?;}
        let nominal=request.exact()?;
        let previous=request.previous(scene.render.fps)?.exact()?;
        let cut=scene.animation.clamp_camera_request(&nominal,&previous,scene.camera)?!=previous;
        if animated_shutter {scene.animation.sample_shutter(request.numeric()?,&mut samples,&scene.render);}
        let shutter=if animated_shutter {samples.render.as_ref().unwrap().motion_blur.as_ref()}else{scene.render.motion_blur.as_ref()};
        let span=ieee(if cut {0.}else{shutter.map_or(0.,|s|s.shutter)})?/&rate;
        anyhow::ensure!(span>=Default::default(),"animated shutter must be nonnegative");
        let offset=ieee(shutter.map_or(0.,|s|s.position.offset()))?;
        for position in [decimal("-1/2")?,decimal("1/2")?] {
            let endpoint=nominal.add(&Germ::point((&position+&offset)*&span));
            let endpoint=if endpoint.compare_point(&Default::default()).is_lt() {Germ::point(Default::default())}else{endpoint};
            let endpoint=scene.animation.clamp_camera_request(&nominal,&endpoint,scene.camera)?;
            let endpoint=vork_time::Request::Exact(endpoint);
            endpoint.numeric()?;
            if let Some(compute)=compute {compute.validate_request(&endpoint)?;}
        }
    }
    Ok(())
}

/// Check every nominal frame and its shutter endpoints using render's clock admission.
pub(crate) fn check_timeline(scene: &InstalledScene) -> Result<()> {
    let frames = clip_frame_count(&scene.render)?;
    validate_clock_requests(scene, (0..frames).map(|index| vork_time::Request::Frame(index as u32, scene.render.fps)))
}

fn clip_frame_count(render: &vork_scene::Render) -> Result<usize> {
    let count = render.duration.exact() * vork_time::exact::decimal(&render.fps.to_string())?;
    let frames: usize = count.ceil().to_integer().try_into().context("output frame count exceeds usize")?;
    ensure_usage!(frames > 0 && frames <= u32::MAX as usize, "output frame count exceeds u32");
    Ok(frames)
}
