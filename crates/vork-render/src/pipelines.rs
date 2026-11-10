//! Pipelines and bind-group layouts, compiled once per Renderer and shared by scenes.
use super::*;
use crate::forward_variant::FeatureMask;

/// One owner for device pipeline slots and the shared installed-scene set.
#[derive(Default)]
pub(crate) struct Registry {
    pub(crate) scene: [OnceLock<std::sync::Arc<CachedPipelines>>; 2],
    pub(crate) environment: OnceLock<wgpu::ComputePipeline>,
    pub(crate) environment_reduce: OnceLock<wgpu::ComputePipeline>,
    pub(crate) display_stats: OnceLock<[wgpu::ComputePipeline; 2]>,
    pub(crate) light_temperature: OnceLock<wgpu::ComputePipeline>,
    pub(crate) camera_indirect_args: OnceLock<wgpu::ComputePipeline>,
    pub(crate) mips: OnceLock<([wgpu::RenderPipeline; 3], wgpu::Sampler)>,
    pub(crate) reconstruction: OnceLock<std::sync::Arc<crate::denoise::spatial::Pipelines>>,
    pub(crate) bc7: OnceLock<crate::gpu_bc7::Codec>,
}

/// Prewarm only a registry this scene can use. Media, denoising and ray
/// refraction own scene-specific layouts even without a material program.
/// The final registry is selected after material and medium installation.
pub(crate) fn prewarm_builtin(scene: &Scene, effects: EffectPlan) -> Result<bool> {
    Ok(scene.render.aovs.is_empty()
        && scene.images.is_empty()
        && crate::attributes::names(scene)?.is_empty()
        && !programs::has_programs(scene)
        && !crate::rt::ray_shadows(scene)
        && !crate::rt::ray_refraction(scene)
        && !effects.denoise
        && !effects.probes
        && environment_node::Settings::from_scene(scene)?.medium.is_none())
}

pub(crate) fn control_formats(denoise: bool) -> &'static [Option<wgpu::TextureFormat>] {
    const FORMATS: [Option<wgpu::TextureFormat>;5] = [
        Some(wgpu::TextureFormat::Rgba32Uint),
        Some(wgpu::TextureFormat::Rgba16Float),Some(wgpu::TextureFormat::Rgba16Float),
        Some(wgpu::TextureFormat::Rgba16Float),Some(wgpu::TextureFormat::Rgba32Float),
    ];
    &FORMATS[..if denoise {5} else {4}]
}

/// All lazy descriptor jobs use the same compile-once and scoped-worker path.
/// A failed build (a scene shader that does not compile) leaves its slot empty
/// and returns the error through the job's handle.
type Job<'scope> = std::thread::ScopedJoinHandle<'scope, Result<()>>;
fn compile<'scope, T: Send + Sync + 'scope, F: FnOnce() -> Result<T> + Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    slot: &'scope OnceLock<T>,
    needed: bool,
    build: F,
) -> Option<Job<'scope>> {
    (needed && slot.get().is_none()).then(|| scope.spawn(move || try_init(slot, build).map(drop)))
}
/// `OnceLock::get_or_init` for a fallible build: concurrent callers still wait for
/// one build; a failure leaves the slot empty and returns its error to that caller.
pub(crate) fn try_init<T>(slot: &OnceLock<T>, build: impl FnOnce() -> Result<T>) -> Result<&T> {
    struct Failed;
    let mut failure = None;
    // OnceLock is not poisoned by an unwinding initializer; resume_unwind skips the panic hook.
    let init = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| slot.get_or_init(|| {
        build().unwrap_or_else(|error| {
            failure = Some(error);
            std::panic::resume_unwind(Box::new(Failed))
        })
    })));
    match (init, failure) {
        (Ok(value), _) => Ok(value),
        (Err(_), Some(error)) => Err(error),
        (Err(panic), None) => std::panic::resume_unwind(panic),
    }
}
/// A scoped worker's result, re-raising its panic.
pub(crate) fn join<T>(job: std::thread::ScopedJoinHandle<'_, T>) -> T {
    job.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}
/// Join lazy descriptor jobs, returning the first failure.
fn joined<'scope>(jobs: impl IntoIterator<Item = Option<Job<'scope>>>) -> Result<()> {
    jobs.into_iter().flatten().try_for_each(join)
}

/// Owned pipeline-creation handle: background variant workers outlive the install
/// call that started them, so they cannot borrow the Renderer.
#[derive(Clone)]
pub(crate) struct Compiler {
    #[cfg(test)]
    pub(crate) hit_selections: std::sync::Arc<Mutex<Vec<(ProgramKernel, ProgramKernel)>>>,
    device: wgpu::Device,
    cache: Option<wgpu::PipelineCache>,
    /// Wall time per driver pipeline creation call; concurrent times overlap.
    pub(crate) timings: std::sync::Arc<Mutex<std::collections::BTreeMap<String, f64>>>,
    known: std::sync::Arc<Mutex<Known>>,
    pending: std::sync::Arc<(Mutex<Pending>, std::sync::Condvar)>,
    schedule: std::sync::Arc<(Mutex<Schedule>, std::sync::Condvar)>,
    /// Driver creations so far; unchanged since the last save means nothing to persist.
    created: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
/// Background variant workers still compiling, and the first one that failed.
#[derive(Default)]
struct Pending {
    workers: usize,
    failure: Option<String>,
}
/// Driver pipeline creation around critical-path work (the hardware encoder attach).
#[derive(Default)]
struct Schedule {
    critical: bool,
    foreground_preparations: usize,
    active: usize,
}
/// One admitted driver pipeline creation; frees its place when dropped.
struct Permit<'a>(&'a (Mutex<Schedule>, std::sync::Condvar));
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().active -= 1;
        self.0.1.notify_all();
    }
}
/// Defer optional variants until every foreground preparation has finished.
/// This covers Slang admission as well as driver calls, including error exits.
pub(crate) struct ForegroundPreparation(std::sync::Arc<(Mutex<Schedule>, std::sync::Condvar)>);
impl Drop for ForegroundPreparation {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().foreground_preparations -= 1;
        self.0.1.notify_all();
    }
}
impl<'a> Permit<'a> {
    fn new(schedule: &'a (Mutex<Schedule>, std::sync::Condvar), background: bool) -> Self {
        let (state, signal) = schedule;
        let mut state = signal.wait_while(state.lock().unwrap(), |s| {
            (background && s.foreground_preparations != 0)
                || s.critical && (background || s.active >= crate::constants::CRITICAL_PIPELINE_THREADS)
        }).unwrap();
        state.active += 1;
        Self(schedule)
    }
}
impl Compiler {
    pub(crate) fn new(
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
        known: Known,
    ) -> Self {
        Self {
            #[cfg(test)]
            hit_selections: Default::default(),
            device: device.clone(),
            cache: cache.cloned(),
            timings: Default::default(),
            known: std::sync::Arc::new(Mutex::new(known)),
            pending: Default::default(),
            schedule: Default::default(),
            created: Default::default(),
        }
    }
    pub(crate) fn created(&self) -> usize {
        self.created.load(std::sync::atomic::Ordering::Acquire)
    }
    /// Mark critical-path driver work (encoder attach) in flight or finished. While it
    /// runs, install creations are throttled and background variants wait.
    pub(crate) fn set_critical(&self, critical: bool) {
        self.schedule.0.lock().unwrap().critical = critical;
        self.schedule.1.notify_all();
    }
    /// Admit one creation: at most CRITICAL_PIPELINE_THREADS at once during critical
    /// work; background variants only once it has finished.
    fn permit(&self, background: bool) -> Permit<'_> {
        Permit::new(&self.schedule, background)
    }
    pub(crate) fn foreground_preparation(&self) -> ForegroundPreparation {
        self.schedule.0.lock().unwrap().foreground_preparations += 1;
        ForegroundPreparation(self.schedule.clone())
    }
    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.pending.0.lock().unwrap().workers == 0
    }
    /// Extract a cache only between driver creations. Holding admission closed
    /// prevents a deferred worker entering the driver during the snapshot; workers
    /// still compiling Slang need not finish. Never wait for an active creation.
    pub(crate) fn when_idle<T>(&self, save: impl FnOnce() -> T) -> Option<T> {
        let state = self.schedule.0.try_lock().ok()?;
        if state.active != 0 {
            return None;
        }
        Some(save())
    }
    /// Block until every background variant has finished; a failed worker is an error.
    pub(crate) fn settle(&self) -> Result<()> {
        let (pending, done) = &*self.pending;
        let pending = done.wait_while(pending.lock().unwrap(), |p| p.workers > 0).unwrap();
        match &pending.failure {
            Some(failure) => Err(anyhow::anyhow!("pipeline variant: {failure}")),
            None => Ok(()),
        }
    }
    /// Run one optional variant compilation after foreground preparation. A failure
    /// or panic still retires the worker (its drawn key keeps the superset fallback),
    /// so settling reports it instead of waiting forever.
    pub(crate) fn spawn_variant(&self, mode: VariantCompile, build: impl FnOnce() -> Result<()> + Send + 'static) {
        self.pending.0.lock().unwrap().workers += 1;
        let c = self.clone();
        std::thread::spawn(move || {
            if let VariantCompile::Gated(gate) = &mode {
                let (open, signal) = &**gate;
                let _open = signal.wait_while(open.lock().unwrap(), |open| !*open);
            }
            let failure = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(c.permit(true));
                build()
            })) {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(format!("{error:#}")),
                Err(_) => Some("worker panicked".into()),
            };
            let (pending, done) = &*c.pending;
            let mut pending = pending.lock().unwrap();
            pending.workers -= 1;
            if pending.failure.is_none() { pending.failure = failure; }
            done.notify_all();
        });
    }
    fn time(&self, label: Option<&str>, start: std::time::Instant) {
        let ms=start.elapsed().as_secs_f64()*1000.;
        crate::timing::creation(&self.device,label.unwrap_or("pipeline"),"pipeline",0.,ms);
        *self
            .timings
            .lock()
            .unwrap()
            .entry(label.unwrap_or("pipeline").into())
            .or_default() += ms;
    }
    pub(crate) fn render(&self, descriptor: &wgpu::RenderPipelineDescriptor<'_>) -> wgpu::RenderPipeline {
        let _permit = self.permit(false);
        let start = std::time::Instant::now();
        let _progress = vork_slang::progress::Job::start(true);
        let pipeline = crate::gpu_memory::render_pipeline(&self.device, descriptor);
        self.created.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.time(descriptor.label, start);
        pipeline
    }
    pub(crate) fn compute(&self, descriptor: &wgpu::ComputePipelineDescriptor<'_>) -> wgpu::ComputePipeline {
        let _permit = self.permit(false);
        let start = std::time::Instant::now();
        let _progress = vork_slang::progress::Job::start(true);
        let pipeline = crate::gpu_memory::compute_pipeline(&self.device, descriptor);
        self.created.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.time(descriptor.label, start);
        pipeline
    }
    /// Publish an optional pipeline through the same retained slot for every install.
    pub(crate) fn install_variant<P:Clone+Send+Sync+'static>(&self,mode:&VariantCompile,slot:std::sync::Arc<OnceLock<P>>,fallback:Option<P>,build:impl FnOnce(&Compiler)->Result<P>+Send+'static)->Result<Drawn<P>> {
        if let (true,Some(pipeline))=(mode.fallback(),&fallback) {return Ok(Drawn::inline(pipeline.clone()));}
        let deferred=fallback.is_some();
        self.publish_variant(mode,slot,fallback,deferred,build)
    }
    /// The caller keeps its existing pass sequence until this optional kernel is ready.
    pub(crate) fn install_optional<P:Clone+Send+Sync+'static>(&self,mode:&VariantCompile,slot:std::sync::Arc<OnceLock<P>>,warm:bool,build:impl FnOnce(&Compiler)->Result<P>+Send+'static)->Result<Drawn<P>> {
        let deferred=mode.defer(slot.get().is_some() || warm,true);
        self.publish_variant(mode,slot,None,deferred,build)
    }
    fn publish_variant<P:Clone+Send+Sync+'static>(&self,mode:&VariantCompile,slot:std::sync::Arc<OnceLock<P>>,fallback:Option<P>,deferred:bool,build:impl FnOnce(&Compiler)->Result<P>+Send+'static)->Result<Drawn<P>> {
        if deferred {
            let (worker,target)=(self.clone(),slot.clone());
            self.spawn_variant(mode.clone(),move || try_init(&target,||build(&worker)).map(drop));
        } else {try_init(&slot,||build(self))?;}
        Ok(Drawn {variant:slot,fallback,deferred})
    }
    pub(crate) fn compute_entry(&self, layout:&wgpu::PipelineLayout, module:&wgpu::ShaderModule, entry:&str, label:&str)->wgpu::ComputePipeline {
        self.compute(&wgpu::ComputePipelineDescriptor {label:Some(label),layout:Some(layout),module,entry_point:Some(entry),compilation_options:Default::default(),cache:self.cache.as_ref()})
    }
    pub(crate) fn is_known(&self, key: &blake3::Hash) -> bool {
        self.known.lock().unwrap().keys.contains(key.as_bytes())
    }
    pub(crate) fn remember(&self, key: blake3::Hash) {
        let mut known = self.known.lock().unwrap();
        known.added |= known.keys.insert(*key.as_bytes());
    }
    /// Snapshot without consuming dirty state: every unsuccessful save can retry.
    /// The renderer serializes saves before taking this snapshot.
    pub(crate) fn known_snapshot(&self) -> Option<Known> {
        let known = self.known.lock().unwrap();
        known.added.then(|| Known {
            path: known.path.clone(),
            keys: known.keys.clone(),
            added: true,
        })
    }
    /// Acknowledge only a published ledger, after its blob or system-cache marker.
    pub(crate) fn save_known(&self, snapshot: Known) -> Result<()> {
        if snapshot.save()? {
            let mut known = self.known.lock().unwrap();
            // Keys only accumulate. A later insertion still needs another save;
            // remembering an existing key does not dirty a committed snapshot.
            known.added = known.keys.len() != snapshot.keys.len();
        }
        Ok(())
    }
}
/// Content keys of geometry pipelines the persistent pipeline cache holds: a known
/// variant is a warm hit and compiles inline; an unknown one compiles in the background.
#[derive(Default)]
pub(crate) struct Known {
    path: std::path::PathBuf,
    keys: std::collections::HashSet<[u8; 32]>,
    added: bool,
}
impl Known {
    pub(crate) fn load(path: std::path::PathBuf, blob_accepted: bool) -> Self {
        let keys = blob_accepted
            .then(|| crate::disk_cache::read(&path))
            .flatten()
            .map(|bytes| {
                bytes
                    .chunks_exact(32)
                    .map(|key| key.try_into().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            path,
            keys,
            added: false,
        }
    }
    fn save(&self) -> Result<bool> {
        let mut keys: Vec<_> = self.keys.iter().collect();
        keys.sort_unstable();
        Ok(crate::disk_cache::write_entry(
            &self.path,
            &keys.into_iter().flatten().copied().collect::<Vec<u8>>(),
        )?)
    }
}
#[cfg(test)]
mod known_persistence_tests {
    use super::*;

    #[test]
    fn committed_snapshot_preserves_later_keys() -> Result<()> {
        let (device, _) = wgpu::Device::noop(&Default::default());
        let root = std::env::temp_dir().join(format!("vork-key-snapshot-{}", std::process::id()));
        let path = root.join("keys.entry");
        let compiler = Compiler::new(&device, None, Known::load(path.clone(), false));
        let first = blake3::hash(b"first pipeline");
        let later = blake3::hash(b"later pipeline");
        compiler.remember(first);
        let snapshot = compiler.known_snapshot().unwrap();
        compiler.remember(later);
        compiler.save_known(snapshot)?;
        let loaded = Known::load(path.clone(), true);
        assert!(loaded.keys.contains(first.as_bytes()));
        assert!(!loaded.keys.contains(later.as_bytes()));
        let snapshot = compiler.known_snapshot().expect("later key must remain dirty");
        compiler.remember(first); // Duplicate insertion needs no further save.
        compiler.save_known(snapshot)?;
        assert!(compiler.known_snapshot().is_none());
        let loaded = Known::load(path.clone(), true);
        assert_eq!(loaded.keys.len(), 2);
        assert!(loaded.keys.contains(later.as_bytes()));
        assert!(!loaded.added);
        assert!(Known::load(path, false).keys.is_empty());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn abandoned_snapshot_keeps_keys_dirty() {
        let (device, _) = wgpu::Device::noop(&Default::default());
        let compiler = Compiler::new(&device, None, Known::default());
        let key = blake3::hash(b"pipeline");
        compiler.remember(key);
        drop(compiler.known_snapshot().unwrap());
        assert!(compiler.known_snapshot().unwrap().keys.contains(key.as_bytes()));
    }
}

/// How install obtains feature variants that are not yet in the persistent cache.
#[derive(Clone, Default)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum VariantCompile {
    /// Draw with the shared superset pipeline until the variant compiles in the background.
    #[default]
    Background,
    /// Draw only the generic; one-frame shots have no use for a later variant.
    Fallback,
    /// `vork setup`: Fallback's choices, and the hit kernels compile the superset
    /// key a later Fallback install draws with (CachedPipelines::drawn_kernel).
    Setup,
    /// Exhaustive setup's ordinary tables: retain material proofs while
    /// sharing hit evaluators across light mixes. Other kernels use Setup.
    SetupPlain,
    /// Compile every variant before install returns.
    Inline,
    /// Tests: background workers start only once the gate opens, regardless of warmth.
    Gated(std::sync::Arc<(Mutex<bool>, std::sync::Condvar)>),
}
impl VariantCompile {
    /// The caller admits a generic only when it is suitable for foreground use.
    /// Geometry generics are cheap shipped controls; camera generics need a warm
    /// driver cache, otherwise their larger JIT loses to direct specialization.
    pub(crate) fn defer(&self,warm:bool,generic_ready:bool)->bool {
        match self {
            Self::Inline=>false,
            Self::Gated(_)=>true,
            Self::Background|Self::Fallback|Self::Setup|Self::SetupPlain=>!warm && generic_ready,
        }
    }
    /// One-frame choices: draw generics and compile no later variant.
    pub(crate) fn fallback(&self)->bool {matches!(self,Self::Fallback|Self::Setup|Self::SetupPlain)}
}
/// A retained variant; the caller supplies a pipeline fallback or keeps its existing passes.
#[derive(Clone)]
pub(crate) struct Drawn<P = wgpu::RenderPipeline> {
    variant: std::sync::Arc<OnceLock<P>>,
    fallback: Option<P>,
    deferred: bool,
}
impl<P> Drawn<P> {
    pub(crate) fn inline(pipeline:P)->Self {
        Self {variant:std::sync::Arc::new(OnceLock::from(pipeline)),fallback:None,deferred:false}
    }
    pub(crate) fn current(&self, prefer_variant:bool) -> &P {
        if prefer_variant {self.variant.get().or(self.fallback.as_ref())} else {self.fallback.as_ref().or(self.variant.get())}.expect("installed variant or fallback")
    }
    pub(crate) fn deferred(&self)->bool {self.deferred}
    pub(crate) fn ready(&self) -> bool {
        self.variant.get().is_some()
    }
}

impl crate::InstalledScene {
    /// Whether every drawn pipeline is its final variant (no superset fallback in use).
    pub(crate) fn variants_ready(&self) -> bool {
        self.geometry_pipelines.values().all(Drawn::ready) && self.decoupled.as_ref().is_none_or(|d|d.variants_ready())
            && self.gi.as_ref().is_none_or(Drawn::ready) && self.reflections.as_ref().is_none_or(Drawn::ready)
    }
    /// Drawn pipelines installed behind a superset fallback (cold variants).
    pub(crate) fn deferred_variants(&self) -> usize {
        self.geometry_pipelines
            .values()
            .filter(|d| d.deferred())
            .count() + self.decoupled.as_ref().map_or(0,|d|d.deferred_variants())
            + usize::from(self.gi.as_ref().is_some_and(Drawn::deferred))
            + usize::from(self.reflections.as_ref().is_some_and(Drawn::deferred))
    }
}

/// The shader one drawn key uses: module plus fragment override constants. Variant
/// backends implement this; the fallback key must cover every drawn feature and each
/// feature gate must stay `FEATURE && weight > 0`, so fallback pixels match.
pub(crate) trait VariantShader: Send + Sync {
    /// The vertex and fragment modules (one module may serve both stages).
    fn modules(&self, key: GeometryKey) -> Result<[wgpu::ShaderModule; 2]>;
    /// Whether the generic control shader ships with the renderer.
    fn generic_shipped(&self) -> bool;
    fn constants(&self, key: GeometryKey) -> Vec<(&'static str, f64)>;
    /// Shipped table evaluator, when it can serve the scene's transport.
    fn camera_generic(&self, kernel: ProgramKernel) -> Option<&'static crate::native::Compiled>;
    /// A shipped executable's warm hint, independent of unrelated scene state.
    /// Constructors still create the current scene's complete pipeline layout.
    fn shared_kernel_key(&self, _kernel: ProgramKernel) -> Option<blake3::Hash> { None }
    /// A compute kernel with the scene's programs at hits, and its entry.
    fn program_kernel(&self, kernel: ProgramKernel) -> Result<(wgpu::ShaderModule, &'static str)>;
    /// Whether `program_kernel` would compile nothing: shipped, or its code is stored.
    fn kernel_stored(&self, kernel: ProgramKernel) -> Result<bool>;
}
/// Compute kernels that evaluate surfaces at ray hits. GI, reflections and path
/// transport specialize over installed capabilities; other kernels compile per
/// scene when programs or attributes require it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ProgramKernel {
    /// Path transport with adaptive or fixed sampling.
    Path { adaptive: bool, catchers: bool, random_walk: bool, features: FeatureMask, boundaries: u32, emitter_profiles: bool, media: Option<u32> },
    /// Ray-traced diffuse indirect.
    Gi { features: FeatureMask },
    /// Ray-traced reflections that shade unseen hits.
    Reflections { features: FeatureMask },
    /// The irradiance volume update (probes.rs).
    Probes { features:FeatureMask, directional:bool },
    /// Fixed camera stages and the scene's authored-program evaluation.
    Decoupled { stage: ShadeStage, features: FeatureMask, subsurface: bool, emitter_profiles: bool, boundaries: u32 },
}
impl ProgramKernel {
    /// Untextured, unlayered table materials share one hit evaluator across light
    /// mixes. Keep runtime metalness, sidedness and every analytic light kind;
    /// do not turn on unrelated texture/layer/transport capabilities.
    pub(crate) fn plain_hit(self) -> Option<Self> {
        use crate::forward_variant::*;
        let lighting = FEATURE_LIGHT_TYPES | FEATURE_PUNCTUAL | FEATURE_FINITE
            | FEATURE_RECTANGLE | FEATURE_ELLIPSE | FEATURE_SHADOW_RAYS;
        let allowed = lighting | FEATURE_SPECIALIZED | FEATURE_LIT | FEATURE_SIDED
            | FeatureMask::from(SHADING_METAL);
        match self {
            Self::Gi { features } if features & FEATURE_SPECIALIZED != 0 && features & !allowed == 0 => Some(Self::Gi { features: allowed }),
            Self::Reflections { features } if features & FEATURE_SPECIALIZED != 0 && features & !allowed == 0 => Some(Self::Reflections { features: allowed }),
            _ => None,
        }
    }
    /// The same kernel over the generic feature key (variant.slang Unspecialized:
    /// layered and metallic shading kept), for kernels that otherwise specialize to
    /// the scene and have no shipped generic: ray GI, reflection and probe hits,
    /// camera indirect work, and transmissive table/occupancy stages with boundaries.
    pub(crate) fn superset(self) -> Option<Self> {
        let all = (crate::forward_variant::SHADING_LAYERS | crate::forward_variant::SHADING_METAL) as FeatureMask;
        match self {
            Self::Gi {..} => Some(Self::Gi { features: all }),
            Self::Reflections {..} => Some(Self::Reflections { features: all }),
            Self::Probes {directional,..} => Some(Self::Probes { features: all, directional }),
            // These stages store evaluations, whose record size belongs to the scene.
            Self::Decoupled { stage: ShadeStage::Indirect | ShadeStage::Evaluate | ShadeStage::EvaluateOpaque, .. } => {
                use crate::forward_variant::{FEATURE_EXACT_RECORD,FEATURE_RECORD_WORDS};
                let mut superset = self;
                if let Self::Decoupled { features, .. } = &mut superset { *features = all | FEATURE_EXACT_RECORD | *features & FEATURE_RECORD_WORDS; }
                Some(superset)
            }
            Self::Decoupled { stage: ShadeStage::Occupancy, .. } => {
                let mut superset = self;
                if let Self::Decoupled { features, .. } = &mut superset { *features = all; }
                Some(superset)
            }
            _ => None,
        }
    }
    pub(crate) fn camera_label(self)->Option<String> {
        let Self::Decoupled {stage,..}=self else {return None;};
        Some(match stage {
            ShadeStage::EvaluateOpaque=>"camera material evaluate opaque table".into(),
            ShadeStage::Evaluate=>"camera material evaluate table".into(),
            ShadeStage::EvaluateProgram(id)=>format!("camera material evaluate program {id}"),
            ShadeStage::Compose=>"camera result composition".into(),
            ShadeStage::Fused=>"camera fused table shading".into(),
            ShadeStage::Indirect=>"camera unsupported indirect work".into(),
            ShadeStage::Occupancy=>"camera dielectric occupancy".into(),
            ShadeStage::Index=>"camera occupied slots".into(),
            ShadeStage::Classify=>"camera material classify".into(),
            ShadeStage::Prefix=>"camera material prefix".into(),
            ShadeStage::Scatter=>"camera material scatter".into(),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ShadeStage {
    Indirect, Fused, Occupancy, Index, Classify, Prefix, Scatter, EvaluateOpaque, Evaluate, EvaluateProgram(u32), Compose }
impl ShadeStage {
    pub(crate) fn composes(self)->bool {matches!(self,Self::Compose|Self::Fused)}
    /// Stages that evaluate the built-in material table (Evaluate, EvaluateOpaque, Fused);
    /// each program evaluates its own bin.
    pub(crate) fn table(self)->bool {matches!(self,Self::Evaluate|Self::EvaluateOpaque|Self::Fused)}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,Hash)]
pub(crate) enum CoverageStage {Depth,Visibility}
/// One drawn geometry pipeline: its pass role and, for shading roles, the material
/// feature mask it specializes. Install creates exactly the roles its draws record.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Role {
    ProgramCoverage {program:u32, stage:CoverageStage},
    /// Alpha-tested shadow caster.
    Fibre(crate::fibre_transport::Stage),
    /// Double-sided opaque depth, position only.
    OpaqueDepth,
    OpaqueSingleDepth,
    /// Alpha-tested depth; `hashed` for dithered coverage (its entry reads SV_PrimitiveID).
    MaskedDepth {hashed:bool},
    /// Decoupled visibility keys, double- and single-sided and alpha-tested (passes/decoupled.rs).
    Visibility,
    VisibilitySingle,
    VisibilityMasked,
    /// Transparent draws' fragment lists in front of the visibility passes' depth.
    TransparentFragments,
    Controls(FeatureMask),
    ProgramControls { program:u32, features:FeatureMask, transparent:bool },
    TransparentControls(FeatureMask),
}
impl Role {
    /// The single selection rule shared by install (what to compile) and bundles (what to bind).
    pub(crate) fn of(
        kind: crate::bundles::Kind,
        flags: &crate::plan::DrawFlags,
        material: &crate::materials::Material,
    ) -> Self {
        // Program entries always read the primitive (ctx.primitive_id).
        let hashed=material.dithered && material.program==0;
        let role=Self::of_capabilities(kind, flags, material.double_sided, hashed, material.features);
        if material.program!=0 {
            match role {
                Self::MaskedDepth {..}=>return Self::ProgramCoverage {program:material.program,stage:CoverageStage::Depth},
                Self::VisibilityMasked=>return Self::ProgramCoverage {program:material.program,stage:CoverageStage::Visibility},
                Self::Controls(features)=>return Self::ProgramControls {program:material.program,features,transparent:false},
                Self::TransparentControls(features)=>return Self::ProgramControls {program:material.program,features,transparent:true},
                _=>{}
            }
        }
        role
    }
    /// `hashed`: table entries that declare SV_PrimitiveID (forward_variant::FEATURE_DITHERED).
    fn of_capabilities(kind: crate::bundles::Kind, flags: &crate::plan::DrawFlags,
        double_sided: bool, hashed: bool, features: FeatureMask) -> Self {
        use crate::bundles::Kind;
        let features = features | if hashed {crate::forward_variant::FEATURE_DITHERED} else {0};
        match kind {
            Kind::Fibre(stage,_) => Self::Fibre(stage),
            Kind::Depth if flags.alpha_mask => Self::MaskedDepth {hashed},
            Kind::Depth if double_sided => Self::OpaqueDepth,
            Kind::Depth => Self::OpaqueSingleDepth,
            Kind::Visibility(_) if flags.alpha_mask => Self::VisibilityMasked,
            Kind::Visibility(_) if double_sided => Self::Visibility,
            Kind::Visibility(_) => Self::VisibilitySingle,
            Kind::TransparentFragments(_) => Self::TransparentFragments,
            Kind::Controls => Self::Controls(
                features
                    & !(crate::materials::FEATURE_LIGHT_TYPES
                        | crate::materials::FEATURE_PUNCTUAL
                        | crate::materials::FEATURE_FINITE
                        | crate::materials::FEATURE_RECTANGLE
                        | crate::materials::FEATURE_ELLIPSE),
            ),
            // Controls evaluate the surface/environment response, never finite lights.
            Kind::TransparentControls => Self::TransparentControls(
                features
                    & !(crate::materials::FEATURE_LIGHT_TYPES
                        | crate::materials::FEATURE_PUNCTUAL
                        | crate::materials::FEATURE_FINITE
                        | crate::materials::FEATURE_RECTANGLE
                        | crate::materials::FEATURE_ELLIPSE),
            ),
        }
    }
    /// The material feature mask this role specializes (0 for depth and shadow roles).
    pub(crate) fn shading(self) -> FeatureMask {
        match self {
            Self::Controls(m) | Self::TransparentControls(m) | Self::ProgramControls {features:m,..} => m,
            _ => 0,
        }
    }
    /// The shared pipeline evaluating every material feature, when this role specializes.
    /// The superset keeps the key's hashed coverage: the generic is shipped with and without it.
    fn superset(self) -> Option<Self> {
        let (Self::Controls(m) | Self::TransparentControls(m)) = self else {return None};
        let all = FeatureMask::from(crate::materials::SHADING_LAYERS | crate::materials::SHADING_METAL)
            | m & crate::forward_variant::FEATURE_DITHERED;
        if m == all {return None;}
        Some(if matches!(self, Self::Controls(_)) {Self::Controls(all)} else {Self::TransparentControls(all)})
    }
    /// Bundle kinds a draw is recorded in; mirrors the recorder's draw filters.
    /// Raster colour is decoupled: opaque draws record visibility keys and
    /// controls, transparent ones fragment lists and controls, and the shade
    /// pass produces colour. Path frames record controls.
    pub(crate) fn kinds(
        flags: &crate::plan::DrawFlags,
        decoupled: bool,
    ) -> &'static [crate::bundles::Kind] {
        use crate::bundles::Kind;
        match (flags.transparent, decoupled) {
            (false, true) => &[Kind::Depth, Kind::Visibility(0), Kind::Controls],
            (false, false) => &[Kind::Depth, Kind::Controls],
            (true, true) => &[Kind::TransparentFragments(0), Kind::TransparentControls],
            (true, false) => &[Kind::TransparentControls],
        }
    }
}
/// Installed pipelines per (vertex layout, role), shared by every scene on this registry.
pub(crate) type GeometryKey = (crate::geometry::VertexLayout, Role);
pub(crate) struct CachedPipelines {
    pub(crate) raster_samples: u32,
    pub(crate) denoise: bool,
    /// Hash of every source and state input shared by this registry's pipelines.
    identity: blake3::Hash,
    kernel_identity: blake3::Hash,
    /// Camera ray transport: shadows, refraction/thickness, or probes.
    pub(crate) ray_shadows: bool,
    /// Material programs provide procedural coverage to ray transport.
    geometry: Mutex<
        std::collections::HashMap<GeometryKey, std::sync::Arc<OnceLock<wgpu::RenderPipeline>>>,
    >,
    pub(crate) camera_evaluations: Mutex<std::collections::HashMap<ProgramKernel,std::sync::Arc<OnceLock<wgpu::ComputePipeline>>>>,
    /// Module and override constants per drawn key (the variant backend seam).
    pub(crate) shader: Box<dyn VariantShader>,
    /// Decoupled slot merge (passes/decoupled.rs), built on first use.
    pub(crate) decoupled_merge: OnceLock<wgpu::ComputePipeline>,
    cluster_module: OnceLock<wgpu::ShaderModule>,
    deform_module: OnceLock<wgpu::ShaderModule>,
    depth_layout: wgpu::PipelineLayout,
    /// Decoupled transparent fragments' group 2 (passes/decoupled.rs Fragments) and layout.
    pub(crate) fragments_layout: wgpu::BindGroupLayout,
    pub(crate) fragments_geometry_layout: wgpu::BindGroupLayout,
    pub(crate) field_controls: bool,
    pub(crate) program_trace: bool,
    fragments_pipeline_layout: wgpu::PipelineLayout,
    fibre_layout: wgpu::PipelineLayout,
    pub(crate) fibre_capture_layout: wgpu::BindGroupLayout,
    control_targets: Vec<Option<wgpu::ColorTargetState>>,

    pub(crate) resolve_module: String,
    control_module: OnceLock<wgpu::ShaderModule>,
    resolve_layout: wgpu::PipelineLayout,
    pub(crate) deform_pipeline: OnceLock<wgpu::ComputePipeline>,
    pub(crate) cluster_pipeline: OnceLock<wgpu::ComputePipeline>,
    pub(crate) camera_cull_pipeline: OnceLock<[wgpu::ComputePipeline; 3]>,
    pub(crate) global_layout: wgpu::BindGroupLayout,
    pub(crate) mat_layout: wgpu::BindGroupLayout,
    pub(crate) camera_pages: u32,
    pub(crate) shadow_layout: wgpu::BindGroupLayout,
    pub(crate) resolve_pipeline: OnceLock<wgpu::ComputePipeline>,
    pub(crate) fused_resolve_pipeline: OnceLock<wgpu::ComputePipeline>,
    pub(crate) post_source_pipeline: OnceLock<wgpu::ComputePipeline>,
    pub(crate) control_resolve_pipeline: OnceLock<wgpu::ComputePipeline>,
    pub(crate) gi_demand_pipeline: OnceLock<wgpu::ComputePipeline>,
    pub(crate) ao_blur_horizontal_pipeline: OnceLock<wgpu::ComputePipeline>,
    pub(crate) ao_blur_vertical_pipeline: OnceLock<wgpu::ComputePipeline>,
}
impl CachedPipelines {
    /// Shared registry/ledger identity, including shader inputs and descriptor
    /// array capacities. Keys remembered by setup must describe the same pipeline
    /// a later process creates, independently of the installed scene's values.
    pub(crate) fn identity(source: &str, resolve: &str, raster_samples: u32, ray_shadows: bool,
        slang: &crate::slang_forward::SlangScene, output_format: wgpu::TextureFormat) -> blake3::Hash {
        let mut identity = blake3::Hasher::new();
        identity.update(source.as_bytes());
        identity.update(resolve.as_bytes());
        identity.update(&slang.attributes.to_le_bytes());
        identity.update(&slang.textures.max(1).to_le_bytes());
        identity.update(&slang.samplers.max(1).to_le_bytes());
        identity.update(&slang.shadow_capacity.to_le_bytes());
        identity.update(&slang.aov_count.to_le_bytes());
        identity.update(&[u8::from(slang.denoise), u8::from(slang.field_controls), u8::from(slang.no_shadow_containment)]);
        identity.update(&[u8::from(ray_shadows), u8::from(slang.ray_refraction), u8::from(slang.probes), u8::from(slang.connection_transmission)]);
        // Raster fibre resources change the shadow layout even when the material
        // program is identical to one installed for a path scene.
        identity.update(&[u8::from(slang.fibre)]);
        // Coverage changes fallback admission. Preserve opaque setup identities.
        if slang.surface_coverage {identity.update(b"surface coverage");}
        if slang.attribute_coverage {identity.update(b"attribute coverage");}
        if slang.no_boundary_transactions {identity.update(b"no boundary transactions");}
        if slang.no_points {identity.update(b"no points");}
        if slang.bounded_secondary_compilation {identity.update(b"bounded secondary compilation");}
        if let Some(native)=&slang.medium {
            identity.update(native.point.as_bytes());identity.update(&native.identity);

            for flag in native.defines() { identity.update(flag.as_bytes()); identity.update(&[0]); }
            // Path compilation specializes homogeneous versus tracked transport.
            identity.update(&[u8::from(native.mesh_sources),u8::from(native.homogeneous)]);
            for (name,source) in &native.modules {identity.update(name.as_bytes());identity.update(source.as_bytes());}
            for path in &native.includes {identity.update(path.to_string_lossy().as_bytes());}
        }
        if let Some(p)=&slang.propagation {
            for (name,source) in &p.modules {identity.update(name.as_bytes());identity.update(source.as_bytes());}
            for path in &p.include {identity.update(path.to_string_lossy().as_bytes());}
        }
        if let Some(camera)=&slang.camera {
            for (name,source) in &camera.modules {identity.update(name.as_bytes());identity.update(source.as_bytes());}
            for path in &camera.include {identity.update(path.to_string_lossy().as_bytes());}
        }
        if let Some(environment)=&slang.environment {
            for (name,source) in &environment.modules {identity.update(name.as_bytes());identity.update(source.as_bytes());}
            for path in &environment.include {identity.update(path.to_string_lossy().as_bytes());}
        }
        identity.update(&raster_samples.to_le_bytes());
        identity.update(format!("{output_format:?}").as_bytes());
        identity.finalize()
    }

    /// The pipeline for one drawn role, compiled once per registry. Different keys
    /// compile concurrently; the registry lock is held only to find the slot.
    pub(crate) fn geometry(&self, c: &Compiler, key: GeometryKey) -> Result<wgpu::RenderPipeline> {
        try_init(&self.slot(key), || self.build_geometry(c, key)).cloned()
    }
    fn slot(&self, key: GeometryKey) -> std::sync::Arc<OnceLock<wgpu::RenderPipeline>> {
        std::sync::Arc::clone(self.geometry.lock().unwrap().entry(key).or_default())
    }
    pub(crate) fn camera_slot(&self,key:ProgramKernel)->std::sync::Arc<OnceLock<wgpu::ComputePipeline>> {
        self.camera_evaluations.lock().unwrap().entry(key).or_default().clone()
    }
    pub(crate) fn content_key(&self, key: impl std::fmt::Debug) -> blake3::Hash {
        let mut hash = blake3::Hasher::new();
        hash.update(self.identity.as_bytes());
        hash.update(format!("{key:?}").as_bytes());
        hash.finalize()
    }

    /// Surface compute kernels do not consume the display/resolve program or
    /// the raster control pass's denoising attachments. Keep their ledger separate
    /// from the complete registry identity so setup entries survive those edits.
    fn kernel_identity(source: &str, raster_samples: u32, ray_shadows: bool,
        slang: &crate::slang_forward::SlangScene) -> blake3::Hash {
        let mut slang = slang.clone();
        slang.denoise = false;
        // Surface kernels never link camera-ray or path-propagation programs.
        slang.camera = None;
        slang.propagation = None;
        Self::identity(source, "", raster_samples, ray_shadows, &slang, wgpu::TextureFormat::Rgba8Unorm)
    }
    pub(crate) fn kernel_content_key(&self, kernel: ProgramKernel) -> blake3::Hash {
        if matches!(kernel, ProgramKernel::Path {..}) { return self.content_key(kernel); }
        if let Some(key) = self.shader.shared_kernel_key(kernel) { return key; }
        let mut hash = blake3::Hasher::new();
        hash.update(self.kernel_identity.as_bytes());
        hash.update(format!("{kernel:?}").as_bytes());
        hash.finalize()
    }

    /// Install the drawn keys. Warm variants (in the persistent cache) compile inline;
    /// cold ones draw with their shared superset until a background worker lands them.
    pub(crate) fn install_geometry(
        self: &std::sync::Arc<Self>,
        r: &Renderer,
        keys: std::collections::HashSet<GeometryKey>,
        mode: &VariantCompile,
    ) -> Result<std::collections::HashMap<GeometryKey, Drawn>> {
        let c = &r.compiler;
        let warm =
            |key: GeometryKey| self.slot(key).get().is_some() || c.is_known(&self.content_key(key));
        // The shipped generic needs no Slang compilation. Even one cold key
        // starts behind it; compiling a scene-union fallback would block frame 1.
        let plan: Vec<_> = keys.iter().map(|&key| {
            let fallback = key.1.superset().and_then(|family| {
                let defer=mode.defer(warm(key),self.shader.generic_shipped() || warm((key.0,family)));
                defer.then_some((key.0, family))
            });
            (key, fallback)
        }).collect();
        let inline: std::collections::HashSet<_> = plan
            .iter()
            .map(|&(key, fallback)| fallback.unwrap_or(key))
            .collect();
        let inline:Vec<_>=inline.into_iter().collect();
        let cursor=std::sync::atomic::AtomicUsize::new(0);
        let compiled: std::collections::HashMap<_, _> = std::thread::scope(|scope| {
            (0..inline.len().min(std::thread::available_parallelism().map_or(1,usize::from)))
                .map(|_|scope.spawn(|| {
                    let mut pipelines=Vec::new();
                    loop {
                        let index=cursor.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                        let Some(&key)=inline.get(index) else {break;};
                        pipelines.push((key,self.geometry(c,key)?));
                    }
                    Ok(pipelines)
                })).collect::<Vec<_>>().into_iter()
                .map(join).collect::<Result<Vec<_>>>()
        })?.into_iter().flatten().collect();
        plan.into_iter()
            .map(|(key, fallback)| {
                let this=self.clone();
                let drawn=c.install_variant(mode,self.slot(key),fallback.map(|f|compiled[&f].clone()),move |worker|this.build_geometry(worker,key))?;
                Ok((key, drawn))
            })
            .collect()
    }

    fn build_geometry(&self, c: &Compiler, key: GeometryKey) -> Result<wgpu::RenderPipeline> {
        let pipeline = self.build_role(c, key)?;
        c.remember(self.content_key(key));
        Ok(pipeline)
    }

    fn build_role(&self, c: &Compiler, (packed, role): GeometryKey) -> Result<wgpu::RenderPipeline> {
        let fibre_near_targets=[Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rg32Uint,blend:None,write_mask:wgpu::ColorWrites::ALL})];
        let fibre_targets=vec![Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba16Float,
            blend:Some(wgpu::BlendState {color:wgpu::BlendComponent {src_factor:wgpu::BlendFactor::One,dst_factor:wgpu::BlendFactor::One,operation:wgpu::BlendOperation::Add},alpha:wgpu::BlendComponent {src_factor:wgpu::BlendFactor::One,dst_factor:wgpu::BlendFactor::One,operation:wgpu::BlendOperation::Add}}),write_mask:wgpu::ColorWrites::ALL});1];
        let fragment_targets = [Some(wgpu::ColorTargetState {
            format: crate::passes::decoupled::KEY_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty(),
        })];
        let visibility_targets = [Some(wgpu::ColorTargetState {
            format: crate::passes::decoupled::KEY_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL,
        })];
        let depth = &self.depth_layout;
        let samples = self.raster_samples;
        let (layout, label, vs, fs, samples, write, targets): (
            _,
            _,
            _,
            _,
            _,
            _,
            &[Option<wgpu::ColorTargetState>],
        ) = match role {
            Role::Fibre(stage) => (
                if stage.additive() {&self.fibre_layout} else {depth},
                "fibre transport capture","shadow_vs",stage.entry(),1,
                !stage.additive(),
                if stage.additive() {&fibre_targets} else if stage==crate::fibre_transport::Stage::Near {&fibre_near_targets} else {&[]}
            ),
            Role::OpaqueDepth => (
                depth,
                "packed opaque depth",
                "opaque_depth_vs",
                "",
                samples,
                true,
                &[],
            ),
            Role::OpaqueSingleDepth => (
                depth,
                "packed single-sided opaque depth",
                "opaque_single_depth_vs",
                "opaque_single_depth_fs",
                samples,
                true,
                &[],
            ),
            Role::MaskedDepth {..} | Role::ProgramCoverage {stage:CoverageStage::Depth,..} => (
                depth,
                "packed depth",
                "main_vs",
                if role==(Role::MaskedDepth {hashed:true}) {"hashed_depth_fs"} else {"depth_fs"},
                samples,
                true,
                &[],
            ),
            Role::Visibility | Role::VisibilitySingle | Role::VisibilityMasked | Role::ProgramCoverage {stage:CoverageStage::Visibility,..} => (
                &self.fragments_pipeline_layout,
                "decoupled visibility",
                if matches!(role,Role::VisibilityMasked | Role::ProgramCoverage {stage:CoverageStage::Visibility,..}) { "masked_vs" } else { "vs" },
                match role { Role::Visibility => "fs", Role::VisibilitySingle => "single_fs", _ => "masked_fs" },
                1,
                true,
                &visibility_targets,
            ),
            // In the visibility pass after its opaque keys, writing no key.
            Role::TransparentFragments => (
                &self.fragments_pipeline_layout,
                "decoupled transparent fragments",
                "masked_vs",
                if self.field_controls {"transparent_fragments_fs"} else {"transparent_triangles_fs"},
                1,
                false,
                &fragment_targets,
            ),
            Role::Controls(_) | Role::ProgramControls {transparent:false,..} => (
                if self.field_controls {&self.fragments_pipeline_layout} else {depth},
                "packed opaque controls",
                "main_vs",
                "controls_fs",
                samples,
                self.field_controls,
                &self.control_targets,
            ),
            Role::TransparentControls(_) | Role::ProgramControls {transparent:true,..} => (
                if self.field_controls {&self.fragments_pipeline_layout} else {depth},
                "packed controls",
                "main_vs",
                "transparent_controls_fs",
                samples,
                true,
                &self.control_targets,
            ),
        };
        let shading = role.shading();
        let variant_label = if shading & crate::materials::FEATURE_SPECIALIZED != 0 {
            format!("{label} key={shading:016x}")
        } else {
            label.to_owned()
        };
        Ok(packed_geometry_pipeline(
            c,
            &self.shader.modules((packed, role))?,
            &self.shader.constants((packed, role)),
            layout,
            &variant_label,
            vs,
            fs,
            samples,
            write,
            targets,
            packed,
        ))
    }

    // Slang registration: ("clusters", "main", "compute"). Per-file discovery
    // registers shaders/clusters.slang; no shared build registry edit is needed.
    fn build_cluster_pipeline(&self, r: &Renderer) -> wgpu::ComputePipeline {
        let entries = cluster_entries();
        let bind_layout = r
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cluster light culling"),
                entries: &entries,
            });
        let layout = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cluster light culling"),
                bind_group_layouts: &[Some(&bind_layout)],
                immediate_size: 0,
            });
        r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cluster light culling"),
            layout: Some(&layout),
            module: self
                .cluster_module
                .get_or_init(|| native::shader_module(&r.device, &native::CLUSTERS_CULL)),
            entry_point: Some("cull"),
            compilation_options: Default::default(),
            cache: r.cache.as_ref(),
        })
    }
    fn build_deform_pipeline(&self, r: &Renderer) -> wgpu::ComputePipeline {
        passes::deform::pipeline(r, &self.deform_module)
    }
    pub(crate) fn ensure_assets(&self, r: &Renderer, plan: &crate::plan::ScenePlan) -> Result<()> {
        std::thread::scope(|scope| {
            let compute_jobs: [(_, _, fn(&Self, &Renderer) -> wgpu::ComputePipeline); 2] = [
                (
                    &self.cluster_pipeline,
                    plan.cluster,
                    Self::build_cluster_pipeline,
                ),
                (
                    &self.deform_pipeline,
                    plan.deform,
                    Self::build_deform_pipeline,
                ),
            ];
            joined(compute_jobs.map(|(slot, needed, build)| compile(scope, slot, needed, move || Ok(build(self, r)))))
        })
    }

    /// The path transport module and entry, with the scene's programs at hits.
    pub(crate) fn program_kernel(&self, kernel: ProgramKernel) -> Result<(wgpu::ShaderModule, &'static str)> {
        self.shader.program_kernel(kernel)
    }
    /// The kernel an install draws for `kernel`. Kernels specialized to the scene's
    /// material features have a superset key that shades every material the same way
    /// and stores the scene's evaluation record;
    /// a one-frame install draws with the superset when `vork setup` stored it and the
    /// specialization is not stored, instead of compiling (a still has no later frame
    /// to use the specialization). Setup compiles the superset. Every other install
    /// specializes: a cold specialization compiles faster than the superset. The
    /// lookups are content-addressed, so a superset of a scene setup never saw
    /// (other programs or inputs) is simply absent.
    /// Shipped late-specialization artifacts exist for every capability key;
    /// prefer a remembered device superset before consulting artifact presence.
    /// Hit consumers retain a Drawn slot so multi-frame installs can promote
    /// this fallback after their exact pipeline compiles in the background.
    pub(crate) fn drawn_kernel(&self, kernel: ProgramKernel, mode: &VariantCompile, compiler: &Compiler) -> ProgramKernel {
        if matches!(mode, VariantCompile::Background | VariantCompile::Fallback | VariantCompile::Gated(_))
            && !compiler.is_known(&self.kernel_content_key(kernel))
        {
            if let Some(plain) = kernel.plain_hit().filter(|plain| compiler.is_known(&self.kernel_content_key(*plain))) {
                return plain;
            }
        }
        let Some(superset) = kernel.superset() else { return kernel };
        let stored = |kernel| self.shader.kernel_stored(kernel).unwrap_or(false);
        match mode {
            VariantCompile::Setup => superset,
            VariantCompile::SetupPlain => kernel.plain_hit().unwrap_or(superset),
            VariantCompile::Fallback if !compiler.is_known(&self.kernel_content_key(kernel))
                && compiler.is_known(&self.kernel_content_key(superset)) => superset,
            VariantCompile::Fallback if !stored(kernel) && stored(superset) => superset,
            _ => kernel,
        }
    }
    /// Hit keys use the existing deferred compiler and retained compute slots.
    /// A warm setup fallback must not permanently select the movie's evaluator.
    pub(crate) fn install_hit(
        self: &std::sync::Arc<Self>, r: &Renderer, requested: ProgramKernel,
        mode: &VariantCompile, layout: wgpu::PipelineLayout, label: &'static str,
    ) -> Result<Drawn<wgpu::ComputePipeline>> {
        let selected = self.drawn_kernel(requested, mode, &r.compiler);
        #[cfg(test)]
        r.compiler.hit_selections.lock().unwrap().push((requested, selected));
        let build = |compiler: &Compiler, kernel| -> Result<wgpu::ComputePipeline> {
            let (module, entry) = self.program_kernel(kernel)?;
            let pipeline = compiler.compute(&wgpu::ComputePipelineDescriptor {
                label: Some(label), layout: Some(&layout), module: &module,
                entry_point: Some(entry), compilation_options: Default::default(),
                cache: compiler.cache.as_ref(),
            });
            compiler.remember(self.kernel_content_key(kernel));
            Ok(pipeline)
        };
        let fallback = (selected != requested).then(|| build(&r.compiler, selected)).transpose()?;
        let pipelines = self.clone();
        r.compiler.install_variant(mode, self.camera_slot(requested), fallback, move |compiler| {
            let (module, entry) = pipelines.program_kernel(requested)?;
            let pipeline = compiler.compute(&wgpu::ComputePipelineDescriptor {
                label: Some(label), layout: Some(&layout), module: &module,
                entry_point: Some(entry), compilation_options: Default::default(),
                cache: compiler.cache.as_ref(),
            });
            compiler.remember(pipelines.kernel_content_key(requested));
            Ok(pipeline)
        })
    }
    /// The output layout and dispatch must use the same planned resolve, including
    /// encoder surfaces attached after frame-slot creation.
    pub(crate) fn output_resolve(&self, effects: EffectPlan) -> &OnceLock<wgpu::ComputePipeline> {
        if effects.fuse_controls() {
            &self.fused_resolve_pipeline
        } else {
            &self.resolve_pipeline
        }
    }

    pub(crate) fn ensure_output(&self, r: &Renderer, effects: EffectPlan) -> Result<()> {
        let entry = if effects.fuse_controls() {
            if self.raster_samples > 1 { "resolve_fused" } else { "resolve_fused_single" }
        } else { "main" };
        try_init(self.output_resolve(effects), || Ok(passes::resolve::pipeline(
            r, &self.resolve_module, entry, Some(&self.resolve_layout),
        )))?;
        Ok(())
    }

    pub(crate) fn ensure_effects(&self, r: &Renderer, features: EffectPlan) -> Result<()> {
        let ao_blur_layout = crate::passes::ao::slang_layout(&r.device);
        let gi_demand_layout = crate::passes::ao::demand_layout(&r.device);
        enum Module {
            Slang(&'static crate::native::Compiled),
            Resolve,
        }
        let jobs = [
            (
                &self.gi_demand_pipeline,
                features.indirect,
                Module::Slang(&crate::native::GI_DEMAND_BUILD),
                Some(&gi_demand_layout),
                "build",
                "GI exact consumer demand",
            ),
            (
                &self.fused_resolve_pipeline,
                features.fuse_controls() && features.outputs.is_none_or(|o|o.beauty),
                Module::Resolve,
                Some(&self.resolve_layout),
                "main",
                "fused control and output resolve",
            ),
            (
                &self.resolve_pipeline,
                !features.fuse_controls() && features.outputs.is_none_or(|o|o.beauty),
                Module::Resolve,
                Some(&self.resolve_layout),
                "main",
                "resolve",
            ),
            (
                &self.post_source_pipeline,
                (features.compositor || features.denoise) && features.outputs.is_none_or(|o|o.beauty),
                Module::Resolve,
                Some(&self.resolve_layout),
                "post_source",
                "scene-linear post source",
            ),
            (
                &self.ao_blur_horizontal_pipeline,
                features.indirect,
                Module::Slang(&crate::native::AO_BLUR_HORIZONTAL),
                Some(&ao_blur_layout),
                "horizontal",
                "AO per-sample horizontal reconstruction",
            ),
            (
                &self.ao_blur_vertical_pipeline,
                features.indirect,
                Module::Slang(&crate::native::AO_BLUR_VERTICAL),
                Some(&ao_blur_layout),
                "vertical",
                "AO per-sample vertical reconstruction",
            ),
        ];
        std::thread::scope(|scope| {
            scope.spawn(|| {
                if features.outputs.is_none_or(|o|o.beauty || o.controls() || o.aovs || o.catcher) {
                    environment::Environment::pipeline(r);
                }
            });
            let fixed = [
                // Implicit path writes controls itself; implicit raster still
                // schedules Controls, just like graph::Graph's output pruning.
                compile(scope, &self.control_resolve_pipeline, features.standalone_controls(features.outputs), || {
                let shader = control_resolve_shader(self.raster_samples, self.denoise);
                let module = self
                    .control_module
                    .get_or_init(|| crate::native::shader_module(&r.device, shader));
                let entries = control_resolve_entries(self.raster_samples, self.denoise);
                let group = r
                    .device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("control resolve"),
                        entries: &entries,
                    });
                let layout = r
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("control resolve"),
                        bind_group_layouts: &[Some(&group)],
                        immediate_size: 0,
                    });
                Ok(r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("single-sample controls"),
                    layout: Some(&layout),
                    module,
                    entry_point: Some(shader.entry),
                    compilation_options: Default::default(),
                    cache: r.cache.as_ref(),
                }))
                }),
            ];
            let descriptors = jobs.map(|(slot, needed, module, layout, entry, label)| {
                compile(scope, slot, needed, move || {
                    if matches!(&module, Module::Resolve) {
                        let entry=if label == "fused control and output resolve" {
                            if self.raster_samples > 1 {"resolve_fused"} else {"resolve_fused_single"}
                        } else {entry};
                        return Ok(passes::resolve::pipeline(r,&self.resolve_module,entry,layout));
                    }
                    let native_module;
                    let module = match module {
                        Module::Resolve => unreachable!(),
                        Module::Slang(shader) => {
                            native_module = crate::native::shader_module(&r.device, shader);
                            &native_module
                        }
                    };
                    Ok(r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(label),
                        layout,
                        module,
                        entry_point: Some(entry),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: if label == "fused control and output resolve" { &[("FUSED_CONTROLS", 1.)] } else { &[] },
                            ..Default::default()
                        },
                        cache: r.cache.as_ref(),
                    }))
                })
            });
            joined(fixed.into_iter().chain(descriptors))
        })
    }
    pub(crate) fn with_sources(
        r: &Renderer,
        source: &str,
        resolve: &str,
        raster_samples: u32,
        ray_shadows: bool,
        slang: crate::slang_forward::SlangScene,
    ) -> Self {
        let d = &r.device;
        let globals=raster_global_entries();
        let global_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame + objects"),
            entries: &globals,
        });
        let mat_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &material_entries_for(slang.textures.max(1),slang.samplers.max(1)),
        });
        let shadow_entries = camera_shadow_entries_for(ray_shadows,slang.program_trace);
        let shadow_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow read"),
            entries: &shadow_entries,
        });
        let fibre_capture_layout=d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("fibre capture inputs"),entries:&crate::fibre_transport::capture_entries()});
        let fibre_layout=d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("fibre transport capture"),bind_group_layouts:&[Some(&global_layout),Some(&mat_layout),Some(&fibre_capture_layout)],immediate_size:0});
        // Depth and controls read only frame/object + material tables.
        let depth_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("packed depth layout"),
            bind_group_layouts: &if slang.program_trace {vec![Some(&global_layout),Some(&mat_layout),Some(&shadow_layout)]} else {vec![Some(&global_layout),Some(&mat_layout)]},
            immediate_size: 0,
        });
        let fragments_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transparent fragments"),
            entries: &crate::passes::decoupled::fragments_entries(),
        });
        let fragments_geometry_layout=crate::passes::decoupled::fragments_geometry_layout(d);
        let fragments_pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("decoupled transparent fragments"),
            bind_group_layouts: &if slang.program_trace {vec![Some(&global_layout),Some(&mat_layout),Some(&fragments_layout),Some(&fragments_geometry_layout),Some(&shadow_layout)]} else {vec![Some(&global_layout),Some(&mat_layout),Some(&fragments_layout),Some(&fragments_geometry_layout)]},
            immediate_size: 0,
        });

        let denoise=slang.denoise;
        let camera_pages=crate::passes::decoupled::evaluation_pages(&d.limits(),slang.aov_count);
        // The controls pass writes ambient, IDs, normals, motion (and denoising albedo);
        // colour comes from shading.
        let control_targets: Vec<_> = control_formats(denoise).iter().map(|format|
            format.map(wgpu::ColorTargetState::from)).collect();
        let resolve_module = resolve.to_owned();
        let resolve_entries = passes::resolve::layout_entries(
            if raster_samples > 1 { "resolve_color" } else { "resolve_fused_single" },
        );
        let output_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: r.output_format(),
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let output_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("encoder surface"),
            entries: &[output_entry],
        });
        let resolve_bind_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resolve and motion resources"),
            entries: &resolve_entries,
        });
        let resolve_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("resolve and motion layout"),
            bind_group_layouts: &[Some(&resolve_bind_layout), Some(&output_layout)],
            immediate_size: 0,
        });

        let identity = Self::identity(source, resolve, raster_samples, ray_shadows, &slang, r.output_format());
        let kernel_identity = Self::kernel_identity(source, raster_samples, ray_shadows, &slang);
        Self {
            kernel_identity,
            decoupled_merge: OnceLock::new(),
            raster_samples,
            denoise,
            identity,
            ray_shadows,
            field_controls: slang.field_controls,
            program_trace: slang.program_trace,
            // Every raster role uses native Slang.
            shader: Box::new(crate::slang_forward::SlangForward::new(
                r,
                ray_shadows,
                raster_samples > 1,
                slang,
            )),
            cluster_module: OnceLock::new(),
            deform_module: OnceLock::new(),
            depth_layout,
            fragments_layout,
            fragments_geometry_layout,
            fragments_pipeline_layout,
            fibre_layout,
            fibre_capture_layout,
            geometry: Mutex::default(),
            camera_evaluations: Mutex::default(),
            control_targets,
            resolve_module,
            control_module: OnceLock::new(),
            resolve_layout,
            deform_pipeline: OnceLock::new(),
            cluster_pipeline: OnceLock::new(),
            camera_cull_pipeline: OnceLock::new(),
            global_layout,
            mat_layout,
            camera_pages,
            shadow_layout,
            resolve_pipeline: OnceLock::new(),
            fused_resolve_pipeline: OnceLock::new(),
            post_source_pipeline: OnceLock::new(),
            control_resolve_pipeline: OnceLock::new(),
            gi_demand_pipeline: OnceLock::new(),
            ao_blur_horizontal_pipeline: OnceLock::new(),
            ao_blur_vertical_pipeline: OnceLock::new(),
        }
    }
}
/// Group 0 of every geometry pass (shaders/frame.slang): frame, objects, lights,
/// environment, LTC tables and clusters.
/// Scene globals (frame.slang), read by raster stages and the compute kernels that shade surfaces.
pub(crate) fn raster_global_entries()->Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries=global_entries();
    entries.push(buffer_entry(10,wgpu::ShaderStages::VERTEX,wgpu::BufferBindingType::Storage {read_only:true}));
    entries
}
pub(crate) fn global_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = vec![
                buffer_entry(
                    0,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                buffer_entry(
                    1,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer_entry(
                    2,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer_entry(
                    7,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                buffer_entry(
                    8,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                texture_entry(
                    3,
                    wgpu::TextureSampleType::Float { filterable: true },
                    false,
                ),
                sampler_entry(4, false),
                texture_entry(
                    5,
                    wgpu::TextureSampleType::Float { filterable: true },
                    false,
                ),
                texture_entry(
                    6,
                    wgpu::TextureSampleType::Float { filterable: true },
                    false,
                ),
                texture_entry(
                    9,
                    wgpu::TextureSampleType::Float { filterable: true },
                    false,
                ),
            ];
    for e in &mut entries {e.visibility |= wgpu::ShaderStages::COMPUTE;}
    entries
}
/// Group 1: records plus scene-sized material texture and sampler tables. Ray-traced AO also reads it from compute for alpha-tested hits.
pub(crate) fn material_entries(_rays: bool) -> Vec<wgpu::BindGroupLayoutEntry> {material_entries_for(1,1)}
pub(crate) fn material_entries_for(textures:u32,samplers:u32) -> Vec<wgpu::BindGroupLayoutEntry> {
    let textures=textures.max(vork_slang::registry::MATERIAL_TEXTURE_CAPACITY);
    let samplers=samplers.max(vork_slang::registry::MATERIAL_SAMPLER_CAPACITY);
    use crate::binding_abi::{MATERIAL, Resource};
    let visibility = wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE;
    MATERIAL.iter().enumerate().map(|(binding, description)| {
        let binding=binding as u32;
        let (ty,count)=match description.resource {
            Resource::ReadBuffer(_) | Resource::WriteBuffer(_) => (wgpu::BindingType::Buffer {
                ty:wgpu::BufferBindingType::Storage {read_only:matches!(description.resource,Resource::ReadBuffer(_))},
                has_dynamic_offset:false,min_binding_size:None,
            },None),
            Resource::Textures=>(texture_entry(binding,wgpu::TextureSampleType::Float {filterable:true},false).ty,std::num::NonZeroU32::new(textures)),
            Resource::Samplers=>(sampler_entry(binding,false).ty,std::num::NonZeroU32::new(samplers)),
        };
        wgpu::BindGroupLayoutEntry {binding,visibility: if binding == 3 {visibility | wgpu::ShaderStages::VERTEX} else {visibility},ty,count}
    }).collect()
}
/// Group 2: immutable emitter-sampling rows.
pub(crate) fn shadow_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![crate::shadows::sequence_entry()]
}
/// Stable camera resource table. Optional consumers bind empty records.
pub(crate) fn camera_shadow_entries(rays:bool)->Vec<wgpu::BindGroupLayoutEntry> {
    camera_shadow_entries_for(rays,false)
}
fn camera_shadow_entries_for(rays:bool,trace:bool)->Vec<wgpu::BindGroupLayoutEntry> {
    let stages=if trace {wgpu::ShaderStages::COMPUTE|wgpu::ShaderStages::FRAGMENT} else {wgpu::ShaderStages::COMPUTE};
    let mut entries=shadow_entries();

    if rays {
        entries.extend(crate::rt::layout_entries(1,stages));
        entries.push(crate::rt::hit_buffers_entry(3,stages));
    } else {
        entries.extend((1..4).map(|i|buffer_entry(i,stages,wgpu::BufferBindingType::Storage {read_only:true})));
    }
    entries.extend(crate::fibre_transport::read_entries(4));
    entries.extend(crate::probes::read_entries(9,stages));

    for e in &mut entries {e.visibility=stages;}
    entries
}
pub(crate) fn buffer_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
pub(crate) fn environment_texture_entry(binding:u32)->wgpu::BindGroupLayoutEntry {
    let mut entry=texture_entry(binding,wgpu::TextureSampleType::Uint,false);
    if let wgpu::BindingType::Texture {view_dimension,..}=&mut entry.ty {*view_dimension=wgpu::TextureViewDimension::D2Array;}
    entry
}

pub(crate) fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    multisampled: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled,
        },
        count: None,
    }
}
pub(crate) fn sampler_entry(binding: u32, comparison: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(if comparison {
            wgpu::SamplerBindingType::Comparison
        } else {
            wgpu::SamplerBindingType::Filtering
        }),
        count: None,
    }
}
fn packed_geometry_pipeline(
    c: &Compiler,
    [vertex, fragment]: &[wgpu::ShaderModule; 2],
    constants: &[(&str, f64)],
    layout: &wgpu::PipelineLayout,
    label: &str,
    vs: &str,
    fs: &str,
    samples: u32,
    write: bool,
    targets: &[Option<wgpu::ColorTargetState>],
    packed: crate::geometry::VertexLayout,
) -> wgpu::RenderPipeline {
    // Every vertex stage but the fibre captures' is a camera stage (camera-instance remap).
    let camera = vs != "shadow_vs";
    let mut attributes = packed.attributes().to_vec();

    // wgpu compacts empty vertex layouts. Put the instance stream before the
    // optional constant stream, keeping every active binding contiguous.
    if camera {attributes.insert(3,vec![wgpu::VertexAttribute {shader_location:vork_slang::registry::CAMERA_INSTANCE_LOCATION,offset:0,format:wgpu::VertexFormat::Uint32}]);}
    // Decoupled visibility reads its pass's parameter block as a constant stream after it.
    let visibility = matches!(fs, "fs" | "single_fs" | "masked_fs" | "transparent_fragments_fs" | "transparent_triangles_fs");
    if visibility {attributes.insert(4,vec![wgpu::VertexAttribute {shader_location:vork_slang::registry::VISIBILITY_PASS_LOCATION,offset:0,format:wgpu::VertexFormat::Uint32x4},wgpu::VertexAttribute {shader_location:vork_slang::registry::VISIBILITY_LENS_LOCATION,offset:16,format:wgpu::VertexFormat::Float32x2}]);}
    let buffers: Vec<_> = attributes
        .iter()
        .enumerate()
        .map(|(slot, attributes)| wgpu::VertexBufferLayout {
            array_stride: if packed.points && slot<3 {0} else {match slot {
                0 => packed.vertex_stride(),
                1 => packed.vertex_stride() * if packed.deform {2} else {1},
                2 => {
                    if packed.constant_attributes() {
                        0
                    } else {
                        packed.attribute_stride()
                    }
                }
                3 if camera => 4,
                _ => 0,
            }},
            step_mode: if camera && slot==3 {wgpu::VertexStepMode::Instance} else {wgpu::VertexStepMode::Vertex},
            attributes,
        })
        .collect();
    let buffer_count = attributes.iter().rposition(|a| !a.is_empty()).unwrap() + 1;
    c.render(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: vertex,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: &buffers[..buffer_count],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(write),
            depth_compare: Some(if fs=="fibre_far_fs" {
                wgpu::CompareFunction::Greater
            } else if fs=="fibre_accumulate_fs" {
                wgpu::CompareFunction::Always
            } else if write && fs=="controls_fs" {
                wgpu::CompareFunction::LessEqual
            } else if write {
                wgpu::CompareFunction::Less
            } else if matches!(fs,"transparent_fs"|"transparent_receiver_fs"|"transparent_fragments_fs"|"transparent_triangles_fs") {
                wgpu::CompareFunction::LessEqual
            } else {
                wgpu::CompareFunction::Equal
            }),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: (!fs.is_empty()).then_some(
            wgpu::FragmentState {
                module: fragment,
                entry_point: Some(fs),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    ..Default::default()
                },
                targets,
            },
        ),
        multiview_mask: None,
        cache: c.cache.as_ref(),
    })
}

#[cfg(test)]
mod control_resolve_tests {
    use super::*;

    #[test]
    fn composited_previs_subsurface_installs_control_resolve() -> Result<()> {
        let Some(renderer) = crate::test_renderer()? else { return Ok(()); };
        let features = EffectPlan {
            compositor: true, subsurface: true,
            outputs: Some(crate::compositor::Dependencies {
                color: true, beauty: true, ..Default::default()
            }),
            ..Default::default()
        };
        let pipelines = renderer.cached_pipelines(1);
        pipelines.ensure_effects(&renderer, features)?;
        assert!(pipelines.control_resolve_pipeline.get().is_some(),
            "subsurface composition consumes controls even without GI, reflections or denoising");
        Ok(())
    }
}

#[cfg(all(test, vork_private_tests))]
include!("pipelines_tests_private.rs");

/// Control resolve for the raster coverage sample count (msaa4 or single-sample jitter).

/// Native single-sample/MSAA entries, with optional denoising guide output.
pub(crate) fn control_resolve_shader(samples: u32, denoise: bool) -> &'static crate::native::Compiled {
    match (samples, denoise) {
        (1, false) => &crate::native::CONTROL_RESOLVE_SINGLE_RESOLVE,
        (1, true) => &crate::native::CONTROL_RESOLVE_SINGLE_RESOLVE_DENOISE,
        (crate::constants::RASTER_MSAA_SAMPLES, false) => &crate::native::CONTROL_RESOLVE_MSAA_RESOLVE,
        (crate::constants::RASTER_MSAA_SAMPLES, true) => &crate::native::CONTROL_RESOLVE_MSAA_RESOLVE_DENOISE,
        _ => panic!("unsupported control resolve sample count: {samples}"),
    }
}

/// Single-sample controls copy an RGBA16F texel; MSAA RGB averages retain f32.
pub(crate) fn resolved_ambient_format(samples: u32) -> wgpu::TextureFormat {
    if samples == 1 { wgpu::TextureFormat::Rgba16Float } else { wgpu::TextureFormat::Rgba32Float }
}

pub(crate) fn control_resolve_entries(samples: u32, denoise: bool) -> Vec<wgpu::BindGroupLayoutEntry> {
    use wgpu::{BindingType, TextureFormat as Format, TextureSampleType as Sample};
    let mut entries = Vec::new();
    for (binding, sample_type) in [
        Sample::Depth,
        Sample::Uint,
        Sample::Float { filterable: false },
        Sample::Float { filterable: false },
        Sample::Float { filterable: false },
    ]
    .into_iter()
    .enumerate()
    {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: binding as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            count: None,
            ty: BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: samples > 1,
            },
        });
    }
    for (index, format) in [
        Format::Rg32Float,
        Format::R32Uint,
        Format::Rgba16Float,
        resolved_ambient_format(samples),
        Format::Rgba16Float,
    ]
    .into_iter()
    .enumerate()
    {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 5 + index as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            count: None,
            ty: BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        });
    }
    entries.push(buffer_entry(10, wgpu::ShaderStages::COMPUTE, wgpu::BufferBindingType::Uniform));
    if denoise {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 11, visibility: wgpu::ShaderStages::COMPUTE, count: None,
            ty: BindingType::Texture { sample_type: Sample::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: samples > 1 },
        });
        entries.push(buffer_entry(12, wgpu::ShaderStages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: false }));
    }
    entries
}

#[cfg(test)]
mod raster_control_tests {
    #[test]
    fn raster_coverage_counts_have_distinct_shared_registry_entries() -> anyhow::Result<()> {
        let Some(r) = crate::test_renderer()? else { return Ok(()); };
        let single = r.cached_pipelines(1);
        let msaa = r.cached_pipelines(crate::constants::RASTER_MSAA_SAMPLES);
        assert!(!std::sync::Arc::ptr_eq(&single, &msaa));
        assert!(std::sync::Arc::ptr_eq(&single, &r.cached_pipelines(1)));
        assert!(std::sync::Arc::ptr_eq(&msaa, &r.cached_pipelines(crate::constants::RASTER_MSAA_SAMPLES)));
        Ok(())
    }
}

pub(crate) fn cluster_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    [
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: false },
        ]
        .into_iter()
        .enumerate()
        .map(|(binding, ty)| buffer_entry(binding as u32, wgpu::ShaderStages::COMPUTE, ty))
    .collect()
}

/// The same scene globals for ray kernels, independent of their image bindings.
pub(crate) fn ray_globals<'a>(s: &'a InstalledScene, uniform: &'a wgpu::Buffer, objects: &'a wgpu::Buffer, lights: &'a wgpu::Buffer, clusters: &'a wgpu::Buffer, cluster_lights: &'a wgpu::Buffer) -> Vec<wgpu::BindGroupEntry<'a>> {
    vec![entry(0,uniform.as_entire_binding()),entry(1,objects.as_entire_binding()),entry(2,lights.as_entire_binding()),
        entry(3,wgpu::BindingResource::TextureView(&s.environment.specular)),entry(4,wgpu::BindingResource::Sampler(&s.environment.sampler)),
        entry(5,wgpu::BindingResource::TextureView(&s.environment.diffuse)),entry(6,wgpu::BindingResource::TextureView(&s.environment.brdf)),
        entry(7,clusters.as_entire_binding()),entry(8,cluster_lights.as_entire_binding()),entry(9,wgpu::BindingResource::TextureView(&s.environment.ltc))]
}

#[cfg(test)]
mod foreground_admission_tests {
    use super::*;
    #[test]
    fn deferred_compilation_waits_for_foreground_preparation() {
        let schedule = std::sync::Arc::new((Mutex::new(Schedule { foreground_preparations: 1, ..Default::default() }), std::sync::Condvar::new()));
        let foreground = ForegroundPreparation(schedule.clone());
        std::thread::scope(|scope| {
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (admitted_tx, admitted_rx) = std::sync::mpsc::channel();
            let schedule = &schedule;
            let worker = scope.spawn(move || {
                started_tx.send(()).unwrap();
                let _permit = Permit::new(schedule, true);
                admitted_tx.send(()).unwrap();
            });
            started_rx.recv().unwrap();
            // Foreground work remains admitted; only optional compilation waits.
            drop(Permit::new(schedule, false));
            assert!(admitted_rx.recv_timeout(std::time::Duration::from_millis(20)).is_err());
            drop(foreground);
            admitted_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            worker.join().unwrap();
        });
    }
    #[test]
    fn panicked_variant_worker_fails_settle_instead_of_hanging() -> Result<()> {
        let Some(r) = crate::test_renderer()? else { return Ok(()); };
        // A standalone compiler: the renderer's own drop also settles its workers.
        let compiler = Compiler::new(&r.device, None, Known::default());
        compiler.spawn_variant(VariantCompile::Background, || panic!("injected variant failure"));
        compiler.spawn_variant(VariantCompile::Background, || Err(anyhow::anyhow!("injected compile failure")));
        let (settled_tx, settled) = std::sync::mpsc::channel();
        let settling = compiler.clone();
        std::thread::spawn(move || settled_tx.send(settling.settle()).unwrap());
        let result = settled.recv_timeout(std::time::Duration::from_secs(10)).expect("settle hung after a variant worker panic");
        assert!(result.is_err());
        Ok(())
    }
}

/// Populate shipped pipeline descriptors through the same constructors used at
/// install and frame-slot creation. No scene-keyed material variants are needed.
pub(crate) fn warm_builtin(r: &Renderer) -> Result<()> {
    materials::mip_resources(r);
    if crate::gpu_bc7::supported(r, 4, 4, 64) {
        crate::gpu_bc7::Context::new(r)?;
    }
    environment::Environment::pipeline(r);
    environment::reduce_pipeline(r);
    render_images::publication_pipeline(r);
    for samples in [1, crate::constants::RASTER_MSAA_SAMPLES] {
        passes::subsurface::pipelines(r, samples);
        // Geometry layouts do not depend on device ray-query support. Camera
        // shadow resources are warmed separately with their actual ray axis.
        for attributes in 0..=crate::constants::PRECOMPILED_ATTRIBUTE_SLOTS as u32 {
            for denoise in [false, true] {
                let p = if denoise || attributes>0 {
                    std::sync::Arc::new(CachedPipelines::with_sources(r, "", RESOLVE,
                        samples, false, crate::slang_forward::SlangScene { denoise, attributes, ..Default::default() }))
                } else { r.cached_pipelines(samples) };
                if attributes == 0 {
                    p.ensure_effects(r, EffectPlan::default())?;
                    let effects = EffectPlan { indirect:true, reflections:true,
                        compositor:true, ..Default::default() };
                    p.ensure_effects(r, effects)?;
                    passes::camera_cull::pipelines(r, &p);
                    passes::decoupled::merge_pipeline(&p, r);
                    p.deform_pipeline.get_or_init(|| p.build_deform_pipeline(r));
                    p.cluster_pipeline.get_or_init(|| p.build_cluster_pipeline(r));
                    if r.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) && !denoise && samples == 1 {
                        crate::probes::pipelines(r, &p, 0, &VariantCompile::Inline)?;
                    }
                }
                let all = crate::forward_variant::ForwardKey::generic(crate::forward_variant::Stage::Controls,false).features;
                let mut roles = std::collections::HashSet::new();
                for transparent in [false,true] { for alpha_mask in [false,true] {
                    for double_sided in [false,true] { for dithered in [false,true] {
                        let flags = crate::plan::DrawFlags { transparent, alpha_mask,
                            casts_shadows:true, hair:false, moving:false };
                        for &kind in Role::kinds(&flags, true) {
                            roles.insert(Role::of_capabilities(kind, &flags, double_sided, dithered, all));
                        }
                    }}
                }}
                let mut keys = Vec::new();
                for layout in crate::geometry::VertexLayout::builtin_layouts(attributes as u8) {
                    for &role in &roles {
                        if denoise && !matches!(role, Role::Controls(_) | Role::TransparentControls(_) | Role::ProgramControls {..}) { continue; }
                        if samples > 1 && matches!(role,
                            Role::Visibility | Role::VisibilitySingle | Role::VisibilityMasked | Role::TransparentFragments) { continue; }
                        keys.push((layout,role));
                    }
                }
                warm_geometry(r,&p,keys)?;
            }
        }
    }
    r.errors.check()
}

/// Setup bounds driver concurrency independently of the finite descriptor count.
fn warm_geometry(r:&Renderer, p:&CachedPipelines, keys:Vec<GeometryKey>) -> Result<()> {
    // Exhaustive vertex-fetch/role descriptors only help when another process
    // can reuse them. Without a pipeline cache, Vulkan drivers such as lavapipe
    // repeat this entire matrix at startup; library programs are still compiled
    // by their frame-slot constructors. Metal retains pipelines in its system cache.
    if !r.release_preparation && r.cache.is_none() && r.adapter_info.backend != wgpu::Backend::Metal {
        return Ok(());
    }
    let next=std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        (0..std::thread::available_parallelism().map_or(1,usize::from).min(8)).map(|_| {
            let (keys,next)=(&keys,&next);
            scope.spawn(move || {
                while let Some(&key)=keys.get(next.fetch_add(1,std::sync::atomic::Ordering::Relaxed)) {
                    p.geometry(&r.compiler,key)?;
                }
                Ok(())
            })
        }).collect::<Vec<_>>().into_iter()
            .try_for_each(join)
    })
}

#[cfg(test)]
mod registry_identity_tests {
    use super::*;

    #[test]
    fn plain_hit_preserves_capabilities_and_rejects_complex_materials() {
        use crate::forward_variant::*;
        for features in [FEATURE_SPECIALIZED, FEATURE_SPECIALIZED | FeatureMask::from(SHADING_METAL)
            | FEATURE_SIDED | FEATURE_LIGHT_TYPES | FEATURE_FINITE | FEATURE_RECTANGLE] {
            let ProgramKernel::Gi { features: plain } = (ProgramKernel::Gi { features }).plain_hit().unwrap() else { unreachable!() };
            assert_eq!(features & !plain, 0, "a plain hit key may only add capabilities");
            assert_eq!(plain & ((1 << (TEXTURE_FEATURES + 2)) - 4), 0, "no texture lookups");
            assert_eq!(plain & FeatureMask::from(SHADING_LAYERS), 0, "no layered material evaluation");
            assert_eq!((ProgramKernel::Reflections { features }).plain_hit(), Some(ProgramKernel::Reflections { features: plain }));
        }
        for features in [0, FEATURE_SPECIALIZED | FEATURE_COAT, FEATURE_SPECIALIZED | FEATURE_TRANSMISSION,
            FEATURE_SPECIALIZED | FEATURE_LOBES, FEATURE_SPECIALIZED | (1 << 2)] {
            assert!((ProgramKernel::Gi { features }).plain_hit().is_none());
        }
    }
    #[test]
    fn surface_warm_keys_ignore_display_denoise_and_camera_programs() {
        let base=crate::slang_forward::SlangScene::default();
        let mut changed=base.clone();
        changed.denoise=true;
        changed.camera=Some(std::sync::Arc::new(crate::camera_program::Native {
            modules:vec![("camera".into(),"different camera rays".into())],include:vec![],
        }));
        changed.propagation=Some(crate::propagation::Native {
            modules:vec![("propagation".into(),"path-only segment hook".into())],include:vec![],words:vec![],
        });
        assert_eq!(CachedPipelines::kernel_identity("",1,true,&base),CachedPipelines::kernel_identity("",1,true,&changed));
        // The complete registry still distinguishes those consumers and output state.
        assert_ne!(CachedPipelines::identity("","standard",1,true,&base,wgpu::TextureFormat::Rgba8Unorm),
            CachedPipelines::identity("","agx",1,true,&changed,wgpu::TextureFormat::Rgba16Float));
        changed.connection_transmission=true;
        assert_ne!(CachedPipelines::kernel_identity("",1,true,&base),CachedPipelines::kernel_identity("",1,true,&changed));
        changed=base.clone();changed.textures=8;
        assert_ne!(CachedPipelines::kernel_identity("",1,true,&base),CachedPipelines::kernel_identity("",1,true,&changed));
    }

    #[test]
    fn remembered_keys_cover_shader_inputs_and_descriptor_capacities() {
        let base=crate::slang_forward::SlangScene::default();
        let key=|s:&crate::slang_forward::SlangScene|CachedPipelines::identity("",RESOLVE,1,false,s,wgpu::TextureFormat::Rgba8Unorm);
        let mut sentinel=base.clone();sentinel.textures=1;sentinel.samplers=1;
        assert_eq!(key(&base),key(&sentinel),"zero counts create the same one-element sentinel tables");
        for changed in [
            crate::slang_forward::SlangScene {attributes:1,..base.clone()},
            crate::slang_forward::SlangScene {textures:2,..base.clone()},
            crate::slang_forward::SlangScene {samplers:2,..base.clone()},
            crate::slang_forward::SlangScene {aov_count:1,..base.clone()},
            crate::slang_forward::SlangScene {surface_coverage:true,..base.clone()},
            crate::slang_forward::SlangScene {field_controls:true,..base.clone()},
            crate::slang_forward::SlangScene {bounded_secondary_compilation:true,..base.clone()},
            crate::slang_forward::SlangScene {no_points:true,..base.clone()},
            crate::slang_forward::SlangScene {no_shadow_containment:true,..base.clone()},
            crate::slang_forward::SlangScene {probes:true,..base.clone()},
        ] {assert_ne!(key(&base),key(&changed));}
        let medium=|homogeneous|crate::slang_forward::SlangScene {
            medium:Some(std::sync::Arc::new(medium::Native {homogeneous,..Default::default()})),..base.clone()
        };
        assert_ne!(key(&medium(true)),key(&medium(false)));
    }
}
