//! Installed Slang dispatches over retained named-buffer pages. Queue submission
//! and the simulation cursor share one lock, so frame slots cannot race state.
use crate::buffers::{AttributeBinding, ProgramBuffers};
use crate::*;
use ::vork_scene::v2::{self, ComputeSchedule};
use std::{collections::BTreeSet, fmt::Write as _, sync::Mutex};

pub(crate) const CONTEXT_BINDINGS: &str = "struct VorkTick { uint frame; float time; float dt; uint reserved; };\n[[vk::binding(0,0)]] ConstantBuffer<VorkTick> vork_tick;\nstruct VorkOffset { uint4 value; };\n[[vk::binding(1,0)]] ConstantBuffer<VorkOffset> vork_offset;\n";

// Integer exponent checks remain valid under native fast-math compilation.
pub(crate) const FINITE_WGSL: &str = r#"
fn finite_f32(v:f32)->bool {return (bitcast<u32>(v) & 0x7f800000u)!=0x7f800000u;}
fn finite3(v:vec3<f32>)->bool {return all((bitcast<vec3<u32>>(v) & vec3<u32>(0x7f800000u))!=vec3<u32>(0x7f800000u));}
fn finite4(v:vec4<f32>)->bool {return all((bitcast<vec4<u32>>(v) & vec4<u32>(0x7f800000u))!=vec4<u32>(0x7f800000u));}
"#;

/// Declared compute bindings conservatively publish writes to their named source.
pub(crate) fn writes_buffer(scene: &v2::Scene, name: &str) -> bool {
    scene.programs.values().any(|p|p.is_compute() && p.bindings.values().any(|b|b==name))
}
pub(crate) fn writes_instance_attributes(scene: &v2::Scene, node: &v2::Node, columns: &[&str]) -> bool {
    matches!(&node.kind,v2::NodeKind::Mesh {instances:Some(inst),..} if inst.attributes.iter()
        .any(|(key,name)|columns.contains(&key.as_str()) && writes_buffer(scene,name)))
}
pub(crate) fn spatial_instances(scene: &v2::Scene, node: &v2::Node) -> bool {
    writes_instance_attributes(scene,node,&["TRANSLATION","ROTATION","SCALE"])
}
/// A mesh whose instances a compute program places: it writes their transforms or count
/// on the GPU. Raster, hit shading and native TLAS descriptors share the resulting
/// frame-private object snapshot.
pub(crate) fn places_instances(scene: &v2::Scene, node: &v2::Node) -> bool {
    matches!(&node.kind,v2::NodeKind::Mesh {instances:Some(inst),..} if inst.count.is_some())
        || spatial_instances(scene,node)
}

struct Dispatch {
    pipeline: wgpu::ComputePipeline,
    tiles: Vec<(wgpu::BindGroup, [u32; 3])>,
    schedule: ComputeSchedule,
    inputs: Option<(wgpu::Buffer, Vec<u32>, Option<programs::InputAnimation>)>,
    name: String,
    writes_placement: bool,
    writes: Vec<String>,
}
impl Dispatch {
    fn publish(&self, cursor: &mut Cursor) {
        if self.writes_placement { cursor.publish_placement(); }
        for name in &self.writes { cursor.publish_source(name); }
    }
}
pub(crate) struct Compute {
    dispatches: Vec<Dispatch>,
    pages: Vec<wgpu::Buffer>,
    initial: Vec<wgpu::Buffer>,
    previous: Vec<wgpu::Buffer>,
    device: wgpu::Device,
    timestamp_period: f32,
    simulation: Option<v2::Simulation>,
    simulation_ranges: Vec<(usize, u64, u64)>,
    frame_ranges: Vec<(usize, u64, u64)>,
    frame_sources: Vec<String>,
    checkpoint_store: disk_cache::Store,
    identity: blake3::Hash,
    tick: wgpu::Buffer,
    fps: f32,
    rate: vork_time::Time,
    frames: u32,
    pub(crate) cursor: Mutex<Cursor>,
    instances: Vec<Instance>,
    /// Installed copies from retained source ranges to object-owned material rows.
    material_attributes: Vec<(usize, u64, u64, u64)>,
    material_attribute_sources: Vec<String>,
    bridge: Option<(
        wgpu::ComputePipeline,
        u32,
        wgpu::Buffer,
    )>,
}
// Policy limits, never scene appearance controls. Only one checkpoint is staged per request.
const CHECKPOINT_INTERVAL: u32 = 64;
pub(crate) struct Checkpoint {
    pub buffers: Vec<wgpu::Buffer>,
    path: PathBuf,
}
impl Checkpoint {
    pub fn publish(self) {
        let mapped: Vec<_> = self.buffers.iter().map(|buffer|buffer.get_mapped_range(..)).collect();
        let parts: Vec<&[u8]> = mapped.iter().map(|view| &view[..]).collect();
        // Cache disabled/full/unwritable is an optimization miss. Stream the
        // retained pages through the existing checksum/atomic cache writer.
        let _ = disk_cache::write_parts(&self.path, &parts);
        drop(parts);drop(mapped);
        for buffer in self.buffers {buffer.unmap();}
    }
}
#[derive(Default)]
pub(crate) struct Cursor {
    #[cfg(test)]
    pub(crate) replay_ticks: u64,
    frame: Option<u32>,
    replay_cost: ReplayCost,
    replay_group: Option<ReplayGroup>,
    simulation: Option<u32>,
    resident: std::collections::VecDeque<(u32, crate::residency::Cache<wgpu::Buffer>)>,
    pub(crate) requested: std::collections::BTreeMap<u32,usize>,
    retention_stride: Option<u32>,
    pub(crate) placement_revision: u64,
    pub(crate) publications: std::collections::BTreeMap<String, u64>,
}
impl Cursor {
    fn publish_source(&mut self, name: &str) {
        let revision = self.publications.entry(name.into()).or_default();
        *revision = revision.checked_add(1).expect("geometry source revision exhausted");
    }
    fn publish_placement(&mut self) {
        self.placement_revision = self.placement_revision.checked_add(1).expect("placement revision exhausted");
    }
}
#[derive(Default)]
enum ReplayCost {
    #[default]
    Unknown,
    Pending {buffer:wgpu::Buffer,ready:std::sync::mpsc::Receiver<Result<(),wgpu::BufferAsyncError>>,period:f32},
    Measured(Option<f64>),
}
impl ReplayCost {
    fn value(&mut self) -> Option<f64> {
        if let Self::Pending {buffer,ready,period}=self {
            match ready.try_recv() {
                Ok(Ok(()))=>{
                    let data=buffer.get_mapped_range(..);let values:&[u64]=bytemuck::cast_slice(&data);
                    let cost=crate::timing::elapsed_ms(values[0],values[1],*period)
                        .map(|ms|ms.max(f64::from(*period)/1e6));
                    drop(data);buffer.unmap();*self=Self::Measured(cost);
                }
                Ok(Err(_))|Err(std::sync::mpsc::TryRecvError::Disconnected)=>*self=Self::Measured(None),
                Err(std::sync::mpsc::TryRecvError::Empty)=>{}
            }
        }
        match self {Self::Measured(cost)=>*cost,_=>None}
    }
}
/// Resolve timing for demanded work; mapping starts only after its owning
/// command buffer is submitted. Subsequent normal submissions deliver readiness.
struct TickProbe {queries:wgpu::QuerySet,resolved:wgpu::Buffer,readback:wgpu::Buffer}
impl TickProbe {
    fn new(device:&wgpu::Device)->Self {
        Self {
            queries:device.create_query_set(&wgpu::QuerySetDescriptor {label:Some("simulation tick cost"),ty:wgpu::QueryType::Timestamp,count:2}),
            resolved:crate::gpu_memory::buffer(device,&wgpu::BufferDescriptor {label:Some("simulation timing resolve"),size:wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT,usage:wgpu::BufferUsages::QUERY_RESOLVE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false}),
            readback:crate::gpu_memory::buffer(device,&wgpu::BufferDescriptor {label:Some("simulation timing readback"),size:16,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false}),
        }
    }
    fn resolve(self,encoder:&mut wgpu::CommandEncoder,period:f32)->ReplayCost {
        encoder.resolve_query_set(&self.queries,0..2,&self.resolved,0);
        encoder.copy_buffer_to_buffer(&self.resolved,0,&self.readback,0,16);
        let (tx,rx)=std::sync::mpsc::channel();
        encoder.map_buffer_on_submit(&self.readback,wgpu::MapMode::Read,..,move |result|{let _=tx.send(result);});
        ReplayCost::Pending {buffer:self.readback,ready:rx,period}
    }
}

/// One paged upload supplies replay clocks and animated numeric inputs. Reusing
/// its source buffer avoids an encoder tracker entry for every tick/pass upload.
struct ReplayUpload {
    buffer: wgpu::Buffer,
    first: u32,
    end: u32,
    stride: u64,
    inputs: Vec<Option<(u64,u64)>>,
}
impl ReplayUpload {
    fn offset(&self, step: u32) -> u64 {u64::from(step-self.first)*self.stride}
}

/// Compare median CPU costs and periodically retry larger groups. Production
/// replay callbacks finish through Segments synchronously (simulation views stream),
/// so this includes native validation and queue publication, not just recording.
struct ReplayGroup {
    tick_ms: Option<f64>,
    gpu_ms: f64,
    ticks: u32,
    target: u32,
    unit_cpu_ms: Option<f64>,
    best_target: u32,
    warming_up: bool,
    observations: Vec<f64>,
    retry_after: u32,
    started: std::time::Instant,
}
impl ReplayGroup {
    fn new(tick_ms: Option<f64>) -> Self {Self {tick_ms,gpu_ms:0.,ticks:0,target:1,unit_cpu_ms:None,best_target:1,warming_up:true,observations:Vec::new(),retry_after:0,started:std::time::Instant::now()}}
    fn tick(&mut self) -> bool {
        self.ticks+=1;
        let Some(cost)=self.tick_ms.filter(|v|v.is_finite() && *v>0.) else {return true};
        self.gpu_ms+=cost;
        self.ticks>=self.target || self.gpu_ms+cost>constants::SAMPLE_GROUP_GPU_MS
            || self.started.elapsed().as_secs_f64()*1000.>=constants::SIMULATION_RECORD_MS
    }
    fn finished(&mut self) {
        self.measured(self.started.elapsed().as_secs_f64()*1000.);
    }
    fn measured(&mut self, elapsed_ms: f64) {
        let unit=elapsed_ms/f64::from(self.ticks);
        if self.warming_up {
            // The first cut also retires the view's upload/setup prologue.
            self.warming_up=false;
        } else if elapsed_ms>constants::SIMULATION_RECORD_MS {
            self.target=(f64::from(self.ticks)*constants::SIMULATION_RECORD_MS/elapsed_ms).floor().max(1.) as u32;
            self.observations.clear();
        } else {
            if self.ticks!=self.target {
                // GPU/host scheduling may cut a trial before its target.
                self.target=self.ticks;self.observations.clear();
            }
            self.observations.push(unit);
            if self.observations.len()==constants::SIMULATION_GROUP_OBSERVATIONS {
                self.observations.sort_by(f64::total_cmp);
                let unit=self.observations[self.observations.len()/2];
                self.observations.clear();
                if self.target==self.best_target || self.unit_cpu_ms.is_none_or(|best|unit<best) {
                    // Refresh the baseline rather than comparing forever against
                    // the luckiest observation from an earlier workload.
                    self.unit_cpu_ms=Some(unit);self.best_target=self.target;
                    if self.retry_after==0 {self.target=self.best_target.saturating_add(1);}
                    else {self.retry_after-=1;}
                } else {
                    self.target=self.best_target;
                    self.retry_after=constants::SIMULATION_GROUP_RETRY;
                }
            }
        }
        self.ticks=0;self.gpu_ms=0.;self.started=std::time::Instant::now();
    }

}

/// Convert a render request to a canonical tick, never rounding to output frames.
fn simulation_tick(sim:&v2::Simulation,time:impl Into<vork_time::Request>)->Result<u32> {
    let time=time.into().exact()?;
    let dt=sim.dt.exact();
    ensure!(dt>vork_time::exact::Rational::from_integer(0.into()),"simulation dt must be positive");
    let tick=time.sub(&vork_time::exact::Germ::point(sim.t0.exact())).scale(&dt.recip()).floor();
    if tick<0.into() {return Ok(0)}
    let tick:u32=tick.try_into().context("simulation time exceeds tick capacity")?;
    ensure!(tick<u32::MAX,"simulation time exceeds tick capacity");
    Ok(tick)
}

struct Instance {
    draw: usize,
    trs: [Option<AttributeBinding>; 3],
    capacity: u32,
    count: Option<AttributeBinding>,
    bounds: (Vec3, Vec3),
    indices: Vec<u32>,
}
pub(crate) struct Slot {
    pub(crate) parents: wgpu::Buffer,
    pub(crate) errors: wgpu::Buffer,
    pub(crate) error_readback: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    binds: Vec<wgpu::BindGroup>,
    groups: u32,
    width: u32,
    objects: Vec<std::ops::Range<u32>>,
}
impl Compute {
    pub(crate) fn install(
        r: &Renderer,
        source: &v2::Scene,
        base: &std::path::Path,
        attributes: Option<&ProgramBuffers>,
        sources: &[Option<&v2::Node>],
        roots: &[install::MeshRoot],
        draws: &[install::Draw],
        rate: vork_time::Time,
        duration: impl Into<vork_time::Clock>,
    ) -> Result<Option<Self>> {
        if !source.programs.values().any(|p| p.is_compute()) && !draws.iter().any(|draw|draw.computed) {
            return Ok(None);
        }
        let fps=rate.as_f32();
        let attributes = attributes.context("compute requires named buffers")?;
        let pages = attributes.storage.clone();
        let mut dispatches = Vec::new();
        let mut identity = blake3::Hasher::new();
        identity.update(&serde_json::to_vec(&source.simulation)?);
        let materials: std::collections::BTreeMap<_, _> = source.materials.iter()
            .filter(|(_, material)| source.programs.get(&material.program)
                .is_some_and(|p| p.schedule == Some(ComputeSchedule::Simulation))).collect();
        identity.update(&serde_json::to_vec(&materials)?);
        if let Some(sim) = &source.simulation {
            // Use decoded initial bytes, not reopened files or unrelated render inputs.
            for name in &sim.state {
                identity.update(name.as_bytes());
                identity.update(attributes.initial_identity[name].as_bytes());
            }
        }

        let tick = init(
            &r.device,
            "compute tick",
            &[0; 16],
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let mut placement_sources = BTreeSet::new();
        // Reuse the installed draw sources; no second scene-tree walk or capacity registry.
        for node in sources.iter().flatten() {
            if let v2::NodeKind::Mesh { instances: Some(inst), .. } = &node.kind {
                for key in ["TRANSLATION", "ROTATION", "SCALE"] {
                    if let Some(name) = inst.attributes.get(key) { placement_sources.insert(name.clone()); }
                }
                placement_sources.extend(inst.count.iter().cloned());
            }
        }
        for (name, program) in source.programs.iter().filter(|(_, p)| p.is_compute()) {
            let threads = program
                .dispatch
                .context("compute dispatch required")?
                .threads();
            let groups = [
                threads[0].div_ceil(constants::PROGRAM_WORKGROUP),
                threads[1],
                threads[2],
            ];
            let tiles = dispatch_tiles(
                groups,
                r.device.limits().max_compute_workgroups_per_dimension,
            );
            let used: BTreeSet<_> = program
                .bindings
                .values()
                .flat_map(|b| attributes.bindings[b].spans.iter().map(|s| s.page))
                .collect();
            ensure!(
                used.len() as u32 <= r.device.limits().max_storage_buffers_per_shader_stage,
                "compute {name}: too many buffer pages"
            );
            let atomic_pages: BTreeSet<_> = program.bindings.values()
                .filter(|name| matches!(source.buffers[*name].element_type, v2::BufferType::U32))
                .flat_map(|name| attributes.bindings[name].spans.iter().map(|span| span.page)).collect();
            let mut header = String::from(CONTEXT_BINDINGS);
            for (i, page) in used.iter().enumerate() {
                writeln!(
                    header,
                    "[[vk::binding({},0)]] RWStructuredBuffer<{}> vork_page_{page};",
                    i + 2, if atomic_pages.contains(page) { "Atomic<uint>" } else { "uint" }
                )?;
            }
            for (binding, buffer) in &program.bindings {
                let layout = &attributes.bindings[buffer];
                if let Some([x,y,z]) = source.buffers[buffer].dims {
                    writeln!(header, "static const uint3 {binding}_dims = uint3({x}u,{y}u,{z}u);")?;
                }
                layout
                    .count
                    .checked_mul(layout.width)
                    .context("compute binding exceeds u32 word addressing")?;
                writeln!(
                    header,
                    "static const uint {binding}_count = {};\nstatic const uint _vork_{binding}_width = {};",
                    layout.count, layout.width
                )?;
                writeln!(header, "uint _vork_{binding}_load(uint word) {{")?;
                for span in &layout.spans {
                    let first = span.first * layout.width;
                    let end = (span.first + span.count) * layout.width;
                    writeln!(
                        header,
                        "if(word >= {first} && word < {end}) return vork_page_{}[{} + word - {first}]{};",
                        span.page, span.offset, if atomic_pages.contains(&span.page) { ".load()" } else { "" }
                    )?;
                }
                writeln!(
                    header,
                    "return 0; }}\nvoid _vork_{binding}_store(uint word, uint value) {{"
                )?;
                for span in &layout.spans {
                    let first = span.first * layout.width;
                    let end = (span.first + span.count) * layout.width;
                    writeln!(
                        header,
                        "if(word >= {first} && word < {end}) {{ vork_page_{}[{} + word - {first}]{}; return; }}",
                        span.page, span.offset, if atomic_pages.contains(&span.page) { ".store(value)" } else { " = value" }
                    )?;
                }
                header.push_str("}\n");
                if matches!(source.buffers[buffer].element_type, v2::BufferType::U32) {
                    writeln!(header, "uint {binding}_atomic_min(uint i, uint value) {{")?;
                    for span in &layout.spans {
                        let end = span.first + span.count;
                        writeln!(header, "if(i >= {}u && i < {end}u) {{ return vork_page_{}[{}u + i - {}u].min(value); }}", span.first, span.page, span.offset, span.first)?;
                    }
                    header.push_str("return 0xffffffffu; }\n");
                }
                typed_accessors(&mut header, binding, source.buffers[buffer].element_type)?;
            }
            let (text, origin) = programs::program_source(base, name, program)?;
            let key = blake3::hash(
                format!(
                    "compute-tiled:{threads:?}:{}:{}:{}:{}:{}:{header}:{text}",
                    vork_slang::VERSION,
                    vork_slang::ABI_VERSION,
                    constants::PROGRAM_WORKGROUP,
                    origin.display(),
                    name
                )
                .as_bytes(),
            );
            let abi=programs::abi();
            let compiler=vork_slang::Compiler::discover();
            let inputs=programs::bound_program_identity(&r.disk_cache,&compiler,&abi,&text,&origin,&header)?;
            let compiler=compiler.with_identity(&inputs);
            let artifact = programs::cached(
                r,
                key,
                &compiler,
                &abi, &text, &origin,
                |compiler| {
                    compiler.compile_compute(
                        &text,
                        &origin,
                        &header,
                        constants::PROGRAM_WORKGROUP,
                        threads,
                    )
                },
            )?;
            if program.schedule == Some(ComputeSchedule::Simulation) {
                identity.update(key.as_bytes());
                // Bind retained simulation to the compiled code and reflected
                // defaults, never to dependency bytes reopened after compilation.
                identity.update(&serde_json::to_vec(&artifact)?);
            }
            if !artifact.warnings.trim().is_empty() {
                eprintln!("{}", artifact.warnings.trim());
            }
            let mut types = vec![None, None];
            types.extend(used.iter().map(|_| Some(false)));
            let mut shader = artifact.wgsl.clone();
            let has_inputs = !artifact.inputs.fields.is_empty();
            if has_inputs {
                let binding = types.len();
                types.push(Some(true));
                writeln!(
                    shader,
                    "@group(0) @binding({binding}) var<storage,read> vork_compute_inputs:array<u32>; fn vork_material_word(i:u32)->u32{{return vork_compute_inputs[i];}}"
                )?;
            }
            ensure!(
                used.len() as u32 + u32::from(has_inputs)
                    <= r.device.limits().max_storage_buffers_per_shader_stage,
                "compute {name}: inputs and buffer pages exceed storage binding limit"
            );
            let pipeline = pipeline_layout(r, &shader, name, &types, "vork_entry");
            let materials: Vec<_> = source
                .materials
                .iter()
                .filter(|(_, m)| m.program == *name)
                .collect();
            let instances: Vec<Option<(&String, &v2::Material)>> = if materials.is_empty() {
                vec![None]
            } else {
                materials.into_iter().map(Some).collect()
            };
            for material in instances {
                let values = material
                    .map(|(_, m)| {
                        m.inputs
                            .iter()
                            .map(|(key, value)| Ok((key.clone(), serde_json::to_value(value)?)))
                            .collect::<Result<std::collections::BTreeMap<_, _>>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let mut words = artifact.inputs.pack(&values, |name, _| {
                    bail!("compute input {name} must be numeric")
                })?;
                let animation = material.map(|(name, _)| programs::InputAnimation::new(source, base, name, &artifact.inputs, &mut words)).transpose()?.flatten();
                if program.schedule == Some(ComputeSchedule::Simulation) {
                    if let Some(animation) = &animation { identity.update(animation.identity().as_bytes()); }
                }
                let input = has_inputs.then(|| {
                    init(
                        &r.device,
                        "compute inputs",
                        bytemuck::cast_slice(&words),
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    )
                });
                let mut bindings = Vec::new();
                for &(groups, offset) in &tiles {
                    let offset = init(
                        &r.device,
                        "compute dispatch offset",
                        bytemuck::cast_slice(&offset),
                        wgpu::BufferUsages::UNIFORM,
                    );
                    let mut entries = vec![
                        entry(0, tick.as_entire_binding()),
                        entry(1, offset.as_entire_binding()),
                    ];
                    for (i, page) in used.iter().enumerate() {
                        entries.push(entry(
                            i as u32 + 2,
                            pages[*page as usize].as_entire_binding(),
                        ));
                    }
                    if has_inputs {
                        entries.push(entry(
                            used.len() as u32 + 2,
                            input.as_ref().unwrap().as_entire_binding(),
                        ));
                    }
                    let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(name),
                        layout: &pipeline.get_bind_group_layout(0),
                        entries: &entries,
                    });
                    bindings.push((bind, groups));
                }
                dispatches.push(Dispatch {
                    name: name.clone(),
                    writes: program.bindings.values().cloned().collect(),
                    writes_placement: program.bindings.values().any(|name| placement_sources.contains(name)),
                    pipeline: pipeline.clone(),
                    tiles: bindings,
                    schedule: program.schedule.context("compute schedule required")?,
                    inputs: input.map(|input| (input, words, animation)),
                });
            }
        }
        let mut instances = Vec::new();
        for (i, draw) in draws.iter().enumerate() {
            let root = &roots[draw.object as usize];
            if let Some(node @ ::vork_scene::v2::Node {
                kind:
                    ::vork_scene::v2::NodeKind::Mesh {
                        instances: Some(inst),
                        ..
                    },
                ..
            }) = sources[root.node]
            {
                if places_instances(source, node) {
                    let binding = |key: &str| -> Result<Option<AttributeBinding>> {
                        inst.attributes.get(key).map(|name| attributes.bindings.get(name).cloned()
                            .context("compute instance column must be a retained binding")).transpose()
                    };
                    let trs = [binding("TRANSLATION")?,binding("ROTATION")?,binding("SCALE")?];
                    let capacity = inst.attributes.values().next().and_then(|name| attributes.bindings.get(name))
                        .context("instance columns require a retained binding")?.count;
                    let count = inst
                        .count
                        .as_ref()
                        .map(|b| {
                            attributes
                                .bindings
                                .get(b)
                                .cloned()
                                .context("instance count must be a compute binding")
                        })
                        .transpose()?;
                    ensure!(
                        count.as_ref().is_none_or(|b| b.count == 1 && b.width == 1),
                        "instance count requires exactly one u32"
                    );
                    let bounds = draw.instances.clone().fold(
                        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                        |(lo, hi), object| {
                            let (a, b) = crate::shadow_envelope::transformed(
                                roots[object as usize].local,
                                draw.bounds,
                            );
                            (lo.min(a), hi.max(b))
                        },
                    );
                    instances.push(Instance {
                        draw: i,
                        trs,
                        capacity,
                        count,
                        bounds,
                        indices: draw
                            .instances
                            .clone()
                            .map(|object| {
                                roots[object as usize]
                                    .instance_index
                                    .expect("compute instance index")
                            })
                            .collect(),
                    });
                }
            }
        }
        let mut initial = Vec::new();
        let mut previous = Vec::new();
        let mut encoder = r.device.create_command_encoder(&Default::default());
        for page in &pages {
            let buffer = crate::gpu_memory::buffer(
                &r.device,
                &wgpu::BufferDescriptor {
                    label: Some("compute initial page"),
                    size: page.size(),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                },
            );
            encoder.copy_buffer_to_buffer(page, 0, &buffer, 0, page.size());
            initial.push(buffer);
            previous.push(crate::gpu_memory::buffer(
                &r.device,
                &wgpu::BufferDescriptor {
                    label: Some("previous compute page"),
                    size: page.size(),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            ));
        }
        let count=if dispatches.iter().any(|d|d.schedule==ComputeSchedule::Frame) {
            (duration.into().exact()*vork_time::exact::decimal(&rate.to_string())?).ceil().to_integer()
        }else{1.into()};
        let frames:u32=count.try_into().context("compute frame grid exceeds u32 capacity")?;
        ensure!(frames>0,"compute frame grid must be nonempty");
        // Geometry adds immutable retained columns to these pages. Only producer-
        // owned ranges reset and publish on seeks; sharing a page is not a write.
        let frame_sources: Vec<String> = dispatches.iter()
            .filter(|d| d.schedule != ComputeSchedule::Simulation)
            .flat_map(|d| d.writes.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
        let mut simulation_ranges = Vec::new();
        let mut frame_ranges = Vec::new();
        for (name, binding) in &attributes.bindings {
            let ranges = if source.simulation.as_ref().is_some_and(|s| s.state.contains(name)) {
                &mut simulation_ranges
            } else if frame_sources.contains(name) { &mut frame_ranges } else { continue };
            for span in &binding.spans {
                ranges.push((span.page as usize, u64::from(span.offset) * 4, u64::from(span.count) * u64::from(binding.width) * 4));
            }
        }
        let result = Self {
            dispatches,
            pages,
            initial,
            previous,
            device: r.device.clone(),
            timestamp_period: r.queue.get_timestamp_period(),
            simulation: source.simulation.clone(),
            simulation_ranges,
            frame_ranges,
            frame_sources,
            checkpoint_store: r.disk_cache.clone(),
            identity: identity.finalize(),
            tick,
            fps,
            rate,
            frames,
            cursor: Mutex::new(Cursor::default()),
            instances,
            material_attributes: Vec::new(),
            material_attribute_sources: Vec::new(),
            bridge: None,
        };
        // Install dispatches run now; seeks replay them from the same initial bytes.
        let mut cursor = result.cursor.lock().expect("compute cursor poisoned");
        result.run(&mut encoder, &mut cursor, ComputeSchedule::Install, 0);
        if let Some(sim) = &result.simulation {
            result.set_tick(&mut encoder, 0, sim.t0.as_f64(), 0., sim.seed);
            result.run_named(&mut encoder, &mut cursor, &sim.init, sim.t0.as_f64() as f32);
        }
        crate::command_finish::Segments::submit(&r.queue, encoder);
        drop(cursor);
        Ok(Some(result))
    }
    pub(crate) fn bounds(&self, draw: usize) -> Option<(Vec3, Vec3)> {
        self.instances
            .iter()
            .find(|i| i.draw == draw)
            .map(|i| i.bounds)
    }
    pub(crate) fn varying(&self, sources: &BTreeSet<String>) -> bool {
        self.dispatches.iter().any(|d|d.schedule != ComputeSchedule::Install && d.writes.iter().any(|n|sources.contains(n)))
    }
    pub(crate) fn geometry_uses_simulation_clock(&self, sources: &[String]) -> bool {
        self.simulation.is_some() && !self.dispatches.iter().any(|d|d.schedule==ComputeSchedule::Frame && d.writes.iter().any(|n|sources.contains(n)))
    }
    pub(crate) fn geometry_time_key(&self,sources:&[String],time:impl Into<vork_time::Request>)->Result<[u32;2]> {
        let time=time.into();
        let simulation=self.simulation.as_ref().filter(|_|self.dispatches.iter().any(|d|d.schedule==ComputeSchedule::Simulation && d.writes.iter().any(|n|sources.contains(n))))
            .map(|sim|simulation_tick(sim,&time)).transpose()?.unwrap_or(0);
        let frame=if self.dispatches.iter().any(|d|d.schedule==ComputeSchedule::Frame && d.writes.iter().any(|n|sources.contains(n))) {
            nominal_frame(&time,self.rate,self.frames)?
        }else{0};
        Ok([simulation,frame])
    }
    pub(crate) fn request_key(&self,time:&vork_time::Request)->Result<[u32;2]> {
        let tick=self.simulation.as_ref().map(|sim|simulation_tick(sim,time)).transpose()?.unwrap_or(0);
        let frame=if self.dispatches.iter().any(|d|d.schedule==ComputeSchedule::Frame) {
            nominal_frame(&time,self.rate,self.frames)?
        }else{0};
        Ok([tick,frame])
    }
    pub(crate) fn validate_request(&self,time:&vork_time::Request)->Result<()> {
        time.numeric()?;
        if let Some(sim)=&self.simulation {
            let tick=simulation_tick(sim,time)?;
            vork_time::Request::Tick(tick,sim.t0.clone(),sim.dt.clone()).numeric()?;
        }
        Ok(())
    }
    pub(crate) fn bind_material_attributes(&mut self, buffers: &ProgramBuffers, rows: Vec<(String, u32, u32)>) -> Result<()> {
        for (source, index, destination) in rows {
            let binding = &buffers.bindings[&source];
            ensure!(index < binding.count, "material attribute row exceeds source capacity");
            let span = binding.spans.iter().find(|span| index >= span.first && index < span.first + span.count)
                .context("material attribute row has no installed range")?;
            self.material_attributes.push((span.page as usize,
                u64::from(span.offset + (index - span.first) * binding.width) * 4,
                u64::from(destination) * 4, u64::from(binding.width) * 4));
            self.material_attribute_sources.push(source);
        }
        self.material_attribute_sources.sort();
        self.material_attribute_sources.dedup();
        Ok(())
    }
    pub(crate) fn has_material_attributes(&self) -> bool { !self.material_attributes.is_empty() }
    pub(crate) fn publish_material_attributes(&self, encoder: &mut wgpu::CommandEncoder,
        cursor: &Cursor, changes: &mut values::ChangeTracker, records: &wgpu::Buffer) {
        let versions: Vec<u64> = self.material_attribute_sources.iter()
            .map(|source| cursor.publications.get(source).copied().unwrap_or(0)).collect();
        if changes.changed(values::ChangeKey::MaterialAttributes, bytemuck::cast_slice(&versions)) {
            for &(page, source, destination, size) in &self.material_attributes {
                encoder.copy_buffer_to_buffer(&self.pages[page], source, records, destination, size);
            }
        }
    }
    pub(crate) fn has_simulation(&self) -> bool { self.simulation.is_some() }
    pub(crate) fn has_instances(&self) -> bool {
        !self.instances.is_empty()
    }
    pub(crate) fn computed_draw(&self, draw: usize) -> bool {
        self.instances.iter().any(|i| i.draw == draw)
    }
    fn run(&self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor, schedule: ComputeSchedule, frame: u32) {
        for dispatch in self.dispatches.iter().filter(|d| d.schedule == schedule) {
            dispatch.publish(cursor);
            self.inputs(encoder, dispatch, frame as f32 / self.fps);
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&dispatch.pipeline);
            for (bind, groups) in &dispatch.tiles {
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
            }
        }
    }
    fn inputs(&self, encoder: &mut wgpu::CommandEncoder, dispatch: &Dispatch, time: f32) {
        if let Some((input, words, Some(animation))) = &dispatch.inputs {
            let mut words = words.clone();
            animation.words(time, &mut words);
            let upload = init(&self.device, "compute sampled inputs", bytemuck::cast_slice(&words), wgpu::BufferUsages::COPY_SRC);
            encoder.copy_buffer_to_buffer(&upload, 0, input, 0, upload.size());
        }
    }
    fn set_tick(&self, encoder: &mut wgpu::CommandEncoder, index: u32, time: f64, dt: f64, seed: u32) {
        let bytes = [index, (time as f32).to_bits(), (dt as f32).to_bits(), seed];
        let upload = init(&self.device, "simulation clock", bytemuck::cast_slice(&bytes), wgpu::BufferUsages::COPY_SRC);
        encoder.copy_buffer_to_buffer(&upload, 0, &self.tick, 0, 16);
    }
    fn run_named(&self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor, names: &[String], time: f32) {
        self.run_named_uploaded(encoder,cursor,names,time,None,None);
    }
    fn run_named_uploaded(&self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor, names: &[String], time: f32, upload: Option<(&ReplayUpload,u32)>, queries: Option<&wgpu::QuerySet>) {
        let mut remaining=names.iter().map(|name|self.dispatches.iter().filter(|d|&d.name==name).count()).sum::<usize>();
        let mut first=true;
        for name in names {
            for (index,dispatch) in self.dispatches.iter().enumerate().filter(|(_,d)| &d.name == name) {
                dispatch.publish(cursor);
                if let Some((page,step))=upload.filter(|(p,_)|p.inputs[index].is_some()) {
                    let (offset,bytes)=page.inputs[index].unwrap();
                    let input=&dispatch.inputs.as_ref().unwrap().0;
                    encoder.copy_buffer_to_buffer(&page.buffer,page.offset(step)+offset,input,0,bytes);
                } else {self.inputs(encoder, dispatch, time);}
                // Separate compute passes supply the storage-write -> read/write barrier.
                remaining-=1;
                let timestamp_writes=queries.map(|query_set|wgpu::ComputePassTimestampWrites {
                    query_set,beginning_of_pass_write_index:first.then_some(0),end_of_pass_write_index:(remaining==0).then_some(1),
                }).filter(|_|first || remaining==0);
                first=false;
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:None,timestamp_writes});
                pass.set_pipeline(&dispatch.pipeline);
                for (bind, groups) in &dispatch.tiles {
                    pass.set_bind_group(0, bind, &[]);
                    pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
                }
            }
        }
    }
    fn replay_upload(&self, sim: &v2::Simulation, first: u32, target: u32, page_bytes: u64) -> ReplayUpload {
        let limit=page_bytes.min(self.device.limits().max_buffer_size).max(16);
        let mut words_per_tick=4usize;
        let mut inputs=vec![None;self.dispatches.len()];
        for (index,dispatch) in self.dispatches.iter().enumerate().filter(|(_,d)|sim.steps.contains(&d.name)) {
            if let Some((_,words,Some(_)))=&dispatch.inputs {
                inputs[index]=Some((words_per_tick as u64*4,words.len() as u64*4));
                words_per_tick+=words.len();
            }
        }
        // A single tick may contain more numeric inputs than one upload page.
        // Keep that valid scene on its ordinary per-dispatch input upload path.
        if words_per_tick as u64*4>limit {words_per_tick=4;inputs.fill(None);}
        let stride=words_per_tick as u64*4;
        let count=u64::from(target-first).min(limit/stride) as u32;
        let end=first+count;
        let mut data=Vec::with_capacity(count as usize*words_per_tick);
        for step in first..end {
            let time=sim.t0.as_f64()+f64::from(step)*sim.dt.as_f64();
            data.extend_from_slice(&[step,(time as f32).to_bits(),(sim.dt.as_f64() as f32).to_bits(),sim.seed]);
            for (index,dispatch) in self.dispatches.iter().enumerate() {
                if inputs[index].is_none() {continue;}
                let (_,words,Some(animation))=dispatch.inputs.as_ref().unwrap() else {unreachable!()};
                let start=data.len();data.extend_from_slice(words);
                animation.words(time as f32,&mut data[start..]);
            }
        }
        ReplayUpload {buffer:init(&self.device,"simulation replay clocks and inputs",bytemuck::cast_slice(&data),wgpu::BufferUsages::COPY_SRC),first,end,stride,inputs}
    }
    fn checkpoint_path(&self, tick: u32) -> PathBuf {
        let mut key = blake3::Hasher::new();
        key.update(self.identity.as_bytes());
        key.update(&tick.to_le_bytes());
        self.checkpoint_store.path("simulation", 1, key.finalize())
    }
    fn checkpoint_size(&self) -> u64 {
        self.simulation_ranges.iter().map(|r| r.2).sum()
    }
    fn restore_checkpoint(&self, encoder: &mut wgpu::CommandEncoder, target: u32, minimum: u32) -> Option<u32> {
        let size = self.checkpoint_size();
        if size == 0 { return None; }
        for tick in (1..=target / CHECKPOINT_INTERVAL).rev().map(|n| n * CHECKPOINT_INTERVAL) {
            if tick <= minimum {break;}
            let path=self.checkpoint_path(tick);
            if std::fs::metadata(&path).ok().is_none_or(|m|m.len()!=size+blake3::OUT_LEN as u64) {continue;}
            let Some(bytes) = disk_cache::read_mapped(&path) else {continue;};
            if bytes.len() as u64 != size {continue;}
            // Allocate every upload before mutating state: optional allocation
            // failure must leave canonical replay's starting state untouched.
            let mut src=0;
            let uploads: Option<Vec<_>>=self.simulation_ranges.iter().map(|&(_,_,count)| {
                let upload=crate::gpu_memory::optional_buffer_init(&self.device,"simulation checkpoint restore",
                    &bytes[src as usize..(src+count) as usize],wgpu::BufferUsages::COPY_SRC);
                src+=count;upload
            }).collect();
            let uploads=uploads?;
            for (&(page,offset,count),upload) in self.simulation_ranges.iter().zip(&uploads) {
                encoder.copy_buffer_to_buffer(upload,0,&self.pages[page],offset,count);
            }
            return Some(tick);
        }
        None
    }
    // Resident and disk snapshots represent the same canonical declared state.
    // Cursor ownership and queue order make restoration safe across frame slots.
    pub(crate) fn plan_requests(&self, cursor: &mut Cursor, times: impl Iterator<Item = vork_time::Request>) -> Result<()> {
        cursor.requested.clear();
        cursor.retention_stride=None;
        if let Some(sim) = &self.simulation {
            for time in times { *cursor.requested.entry(simulation_tick(sim,time)?).or_default() += 1; }
        }
        Ok(())
    }
    pub(crate) fn consume_request(&self, cursor: &mut Cursor, time: impl Into<vork_time::Request>) -> Result<()> {
        if let Some(sim) = &self.simulation {
            let tick=simulation_tick(sim,time)?;
            if let Some(count)=cursor.requested.get_mut(&tick) {
                *count-=1;
                if *count==0 { cursor.requested.remove(&tick); }
            }
        }
        Ok(())
    }
    fn restore_resident(&self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor, target: u32) -> Option<u32> {
        let minimum = cursor.simulation.filter(|&tick|tick<=target);
        let (index, _) = cursor.resident.iter().enumerate()
            .filter(|(_, (tick, cache))| minimum.is_none_or(|minimum|*tick>minimum) && *tick <= target && cache.is_resident())
            .max_by_key(|(_, (tick, _))| *tick)?;
        let entry=cursor.resident.remove(index)?;
        let buffer=entry.1.get()?;
        let mut src=0;
        for &(page,offset,bytes) in &self.simulation_ranges {
            encoder.copy_buffer_to_buffer(&buffer,src,&self.pages[page],offset,bytes);src+=bytes;
        }
        let tick=entry.0;
        cursor.resident.push_back(entry);
        Some(tick)
    }
    fn retain_resident(&self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor, target: u32) {
        if !cursor.requested.contains_key(&target) { return; }
        cursor.resident.retain(|(_,cache)|cache.is_resident());
        if cursor.resident.iter().any(|(tick,_)|*tick==target) {return;}
        let size=self.checkpoint_size();
        if size==0 || size>self.device.limits().max_buffer_size {return;}
        let headroom=cursor.resident.iter().find_map(|(_,cache)|cache.get()
            .and_then(|buffer|crate::residency::buffer_available(&self.device,&buffer)));
        if cursor.retention_stride.is_none() {
            if let Some(free)=headroom {
                let capacity=(free/size).saturating_add(cursor.resident.len() as u64).max(1);
                let first=*cursor.requested.first_key_value().unwrap().0;
                let last=*cursor.requested.last_key_value().unwrap().0;
                cursor.retention_stride=Some(if cursor.requested.len() as u64<=capacity {1} else {
                    (u64::from(last-first)+1).div_ceil(capacity).clamp(1,u64::from(u32::MAX)) as u32
                });
            }
        }
        // Preserve prefixes across the exposure when every tick cannot fit.
        // The first retained request supplies the heap identity and an anchor.
        if target%cursor.retention_stride.unwrap_or(1)!=0
            && cursor.requested.first_key_value().is_none_or(|(&first,_)|target!=first) {return;}
        // Copies and consumers use the same ordered queue. A retired request's
        // buffer can be overwritten after its previous restore, without a wait.
        let reusable=cursor.resident.iter().position(|(tick,_)|!cursor.requested.contains_key(tick))
            .and_then(|index|cursor.resident.remove(index))
            .and_then(|(_,cache)|cache.take()).and_then(|buffer|std::sync::Arc::try_unwrap(buffer).ok());
        let buffer = reusable.or_else(|| {
            // A successful virtual allocation does not guarantee that queued
            // copies can commit it. Check the identified native heap first.
            if headroom.is_some_and(|free|free<size) {return None;}
            crate::gpu_memory::retention_buffer(&self.device,&wgpu::BufferDescriptor {
                label:Some("resident simulation checkpoint"),size,
                usage:wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false,
            })
        });
        let Some(buffer)=buffer else {return;};
        let cache=crate::residency::Cache::new(&crate::gpu_memory::ledger(&self.device));
        let mut dst=0;
        for &(page,offset,bytes) in &self.simulation_ranges {
            encoder.copy_buffer_to_buffer(&self.pages[page],offset,&buffer,dst,bytes);dst+=bytes;
        }
        cache.insert(buffer,size);
        cursor.resident.push_back((target,cache));
    }
    fn checkpoint(&self, encoder: &mut wgpu::CommandEncoder, tick: u32) -> Option<Checkpoint> {
        let size = self.checkpoint_size();
        if size == 0 {return None;}
        // Match retained storage ranges: page size is transfer granularity, not
        // a checkpoint or scene-size ceiling. Avoid a monolithic pinned allocation.
        // No copies are recorded until every page has been admitted. Unknown
        // or unrelated heap telemetry cannot suppress a viable disk checkpoint.
        let buffers=crate::gpu_memory::optional_pages(self.simulation_ranges.len(), |index| {
            let count=self.simulation_ranges[index].2;
            crate::gpu_memory::retention_buffer(&self.device,&wgpu::BufferDescriptor {
                label:Some("simulation checkpoint staging"),size:count,
                usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false,
            })
        }, |index,buffer| {
            let (page,offset,count)=self.simulation_ranges[index];
            encoder.copy_buffer_to_buffer(&self.pages[page],offset,buffer,0,count);
        })?;
        Some(Checkpoint {buffers,path:self.checkpoint_path(tick)})
    }
    #[cfg(test)]
    fn advance(&self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor, time: impl Into<vork_time::Request>) -> Result<Option<Checkpoint>> {
        self.advance_with(encoder, cursor, time, |_| {})
    }
    pub(crate) fn advance_with(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        cursor: &mut Cursor,
        time: impl Into<vork_time::Request>,
        boundary: impl FnMut(&mut wgpu::CommandEncoder),
    ) -> Result<Option<Checkpoint>> {
        self.advance_paged(encoder,cursor,time,constants::SCENE_BUFFER_PAGE_BYTES as u64,boundary)
    }
    pub(crate) fn advance_observing(
        &self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor, time: impl Into<vork_time::Request>,
        boundary: impl FnMut(&mut wgpu::CommandEncoder),
        observe: impl FnMut(&mut wgpu::CommandEncoder,u32),
    ) -> Result<Option<Checkpoint>> {
        self.advance_paged_observing(encoder,cursor,time,constants::SCENE_BUFFER_PAGE_BYTES as u64,boundary,observe)
    }
    fn advance_paged(
        &self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor,
        time: impl Into<vork_time::Request>, upload_page_bytes: u64,
        boundary: impl FnMut(&mut wgpu::CommandEncoder),
    ) -> Result<Option<Checkpoint>> {
        self.advance_paged_observing(encoder,cursor,time,upload_page_bytes,boundary,|_,_| {})
    }
    fn advance_paged_observing(
        &self, encoder: &mut wgpu::CommandEncoder, cursor: &mut Cursor,
        time: impl Into<vork_time::Request>, upload_page_bytes: u64,
        mut boundary: impl FnMut(&mut wgpu::CommandEncoder),
        mut observe: impl FnMut(&mut wgpu::CommandEncoder,u32),
    ) -> Result<Option<Checkpoint>> {
        let time=time.into();
        let mut checkpoint = None;
        if let Some(sim) = &self.simulation {
            let target = simulation_tick(sim, &time)?;
            if let Some(tick) = self.restore_resident(encoder,cursor,target) {
                for name in &sim.state {cursor.publish_source(name);}
                if self.dispatches.iter().any(|d|d.schedule==ComputeSchedule::Simulation && d.writes_placement) {cursor.publish_placement();}
                cursor.simulation=Some(tick);
            }
            if cursor.simulation.is_some_and(|last| target < last) {
                for &(page, offset, bytes) in &self.simulation_ranges {
                    encoder.copy_buffer_to_buffer(&self.initial[page], offset, &self.pages[page], offset, bytes);
                }
                for name in &sim.state { cursor.publish_source(name); }
                if self.dispatches.iter().any(|d| d.schedule == ComputeSchedule::Simulation && d.writes_placement) { cursor.publish_placement(); }
                self.set_tick(encoder, 0, sim.t0.as_f64(), 0., sim.seed);
                self.run_named(encoder, cursor, &sim.init, sim.t0.as_f64() as f32);
                cursor.simulation = None;
            }
            if let Some(tick) = self.restore_checkpoint(encoder, target, cursor.simulation.unwrap_or(0)) {
                for name in &sim.state { cursor.publish_source(name); }
                if self.dispatches.iter().any(|d| d.schedule == ComputeSchedule::Simulation && d.writes_placement) { cursor.publish_placement(); }
                cursor.simulation = Some(tick);
            }
            self.retain_resident(encoder,cursor,cursor.simulation.unwrap_or(0));
            let mut group=cursor.replay_group.take().unwrap_or_else(||ReplayGroup::new(cursor.replay_cost.value()));
            group.tick_ms=cursor.replay_cost.value();
            group.ticks=0;group.gpu_ms=0.;group.started=std::time::Instant::now();group.warming_up=true;
            let mut upload: Option<ReplayUpload>=None;
            for step in cursor.simulation.unwrap_or(0)..target {
                #[cfg(test)] { cursor.replay_ticks += 1; }
                if group.tick_ms.is_none() {
                    if let Some(cost)=cursor.replay_cost.value() {group=ReplayGroup::new(Some(cost));}
                }
                let probe=if matches!(cursor.replay_cost,ReplayCost::Unknown) {
                    cursor.replay_cost=ReplayCost::Measured(None);
                    self.device.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(||TickProbe::new(&self.device))
                } else {None};
                if upload.as_ref().is_none_or(|page|step>=page.end) {
                    // Materialize only the next measured group; the upload
                    // policy below bounds bytes independently of the clock.
                    let count=group.target;
                    upload=Some(self.replay_upload(sim,step,target.min(step.saturating_add(count)),upload_page_bytes));
                }
                let page=upload.as_ref().unwrap();
                encoder.copy_buffer_to_buffer(&page.buffer,page.offset(step),&self.tick,0,16);
                self.run_named_uploaded(encoder, cursor, &sim.steps, (sim.t0.as_f64() + f64::from(step) * sim.dt.as_f64()) as f32,Some((page,step)),probe.as_ref().map(|p|&p.queries));
                if let Some(probe)=probe {cursor.replay_cost=probe.resolve(encoder,self.timestamp_period);}
                observe(encoder,step+1);
                self.retain_resident(encoder,cursor,step+1);
                if step + 1 == target / CHECKPOINT_INTERVAL * CHECKPOINT_INTERVAL && step + 1 > 0 {
                    checkpoint = self.checkpoint(encoder, step + 1);
                }
                if group.tick() {boundary(encoder);group.finished();}
            }
            cursor.replay_group=Some(group);
            cursor.simulation = Some(target);
            self.retain_resident(encoder,cursor,target);
            if cursor.requested.is_empty() { cursor.resident.clear(); }
        }
        if !self.dispatches.iter().any(|d| d.schedule == ComputeSchedule::Frame) {
            for (page, previous) in self.pages.iter().zip(&self.previous) {
                encoder.copy_buffer_to_buffer(page, 0, previous, 0, page.size());
            }
            return Ok(checkpoint);
        }
        let frame=nominal_frame(&time,self.rate,self.frames)?;

        if cursor.frame.is_some_and(|last| frame < last) {
            for &(page, offset, bytes) in &self.frame_ranges {
                encoder.copy_buffer_to_buffer(&self.initial[page], offset, &self.pages[page], offset, bytes);
            }
            for name in &self.frame_sources { cursor.publish_source(name); }
            if self.dispatches.iter().any(|d| d.schedule != ComputeSchedule::Simulation && d.writes_placement) { cursor.publish_placement(); }
            self.set_tick(encoder,0,0.,0.,0);
            self.run(encoder, cursor, ComputeSchedule::Install, 0);
            cursor.frame = None;
        }
        let start = cursor.frame.map_or(0, |f| f + 1);
        for step in start..=frame {
            for &(page, offset, bytes) in &self.frame_ranges {
                encoder.copy_buffer_to_buffer(&self.pages[page], offset, &self.previous[page], offset, bytes);
            }
            self.set_tick(encoder,step,f64::from(step)*self.rate.denominator() as f64/self.rate.numerator() as f64,if step==0 {0.}else{self.rate.denominator() as f64/self.rate.numerator() as f64},0);
            self.run(encoder, cursor, ComputeSchedule::Frame, step);
            // Initialization at t=0 has no preceding simulation frame. Its
            // previous transforms must equal the initialized state, including
            // after a backward seek, so first-frame motion stays zero.
            if step == 0 {
                for (page, previous) in self.pages.iter().zip(&self.previous) {
                    encoder.copy_buffer_to_buffer(page, 0, previous, 0, page.size());
                }
            }
            cursor.frame = Some(step);
        }
        for &(page, offset, bytes) in &self.simulation_ranges {
            encoder.copy_buffer_to_buffer(&self.pages[page], offset, &self.previous[page], offset, bytes);
        }
        Ok(checkpoint)
    }
    pub(crate) fn commands(r: &Renderer, draws: &[install::Draw]) -> wgpu::Buffer {
        let args: Vec<_> = draws
            .iter()
            .map(|draw| wgpu::util::DrawIndexedIndirectArgs {
                index_count: draw.count,
                instance_count: draw.instances.len() as u32,
                first_index: 0,
                base_vertex: 0,
                first_instance: draw.instances.start,
            })
            .collect();
        let empty = wgpu::util::DrawIndexedIndirectArgs::default();
        init(
            &r.device,
            "compute draw commands",
            if args.is_empty() {
                bytemuck::bytes_of(&empty)
            } else {
                bytemuck::cast_slice(&args)
            },
            wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC,
        )
    }
    pub(crate) fn slot(
        &self,
        r: &Renderer,
        s: &InstalledScene,
        objects: &wgpu::Buffer,
        commands: wgpu::Buffer,
    ) -> Result<Slot> {
        ensure!(
            r.device
                .features()
                .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            "compute instances require INDIRECT_FIRST_INSTANCE"
        );
        let parent_values: Vec<_> = self
            .instances
            .iter()
            .map(|instance| s.objects[s.draws[instance.draw].instances.start as usize])
            .collect();
        let parents = init(
            &r.device,
            "compute parent objects",
            bytemuck::cast_slice(&parent_values),
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        );
        let count = self.pages.len();
        let (pipeline, groups, indices) = self
            .bridge
            .as_ref()
            .context("missing installed instance bridge")?;
        let errors=init(&r.device,"compute placement errors",&[0;8],wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        let error_readback=r.device.create_buffer(&wgpu::BufferDescriptor {label:Some("compute placement errors readback"),size:8,usage:wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let mut binds = Vec::new();
        for output in [&commands] {
            let mut entries = Vec::new();
            for i in 0..count {
                entries.push(entry(i as u32, self.pages[i].as_entire_binding()));
                entries.push(entry(
                    (count + i) as u32,
                    self.previous[i].as_entire_binding(),
                ));
            }
            entries.push(entry((count * 2) as u32, parents.as_entire_binding()));
            entries.push(entry((count * 2 + 1) as u32, objects.as_entire_binding()));
            entries.push(entry((count * 2 + 2) as u32, output.as_entire_binding()));
            entries.push(entry((count * 2 + 3) as u32, indices.as_entire_binding()));
            entries.push(entry((count * 2 + 4) as u32, errors.as_entire_binding()));
            binds.push(r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("compute instance bridge"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            }));
        }
        Ok(Slot {
            parents, errors, error_readback,
            pipeline: pipeline.clone(),
            binds,
            groups: *groups,
            width: (*groups).min(r.device.limits().max_compute_workgroups_per_dimension),
            objects: self
                .instances
                .iter()
                .map(|i| s.draws[i.draw].instances.clone())
                .collect(),
        })
    }
    pub(crate) fn prepare_bridge(
        &mut self,
        r: &Renderer,
        draws: &[install::Draw],
        objects: &[Object],
    ) -> Result<()> {
        if self.has_instances() {
            self.bridge = Some(self.build_bridge(r, draws, objects)?);
        }
        Ok(())
    }
    fn build_bridge(
        &self,
        r: &Renderer,
        draws: &[install::Draw],
        objects: &[Object],
    ) -> Result<(
        wgpu::ComputePipeline,
        u32,
        wgpu::Buffer,
    )> {
        let mut code = String::from(FINITE_WGSL);
        let count = self.pages.len();
        ensure!(
            2 * count as u32 + 5 <= r.device.limits().max_storage_buffers_per_shader_stage,
            "compute instance bridge exceeds device storage binding limit"
        );
        for i in 0..count {
            writeln!(
                code,
                "@group(0) @binding({}) var<storage,read> current_{i}:array<u32>;\n@group(0) @binding({}) var<storage,read> previous_{i}:array<u32>;",
                i,
                count + i
            )?;
        }
        code.push_str("struct Object { model:mat4x4<f32>, previous_model:mat4x4<f32>, next_model:mat4x4<f32>, opacity:vec4<f32>, material_index:vec4<u32>, metadata:vec4<u32> };\nstruct Args { indices:u32, instances:u32, first:u32, base:i32, object:u32 };\n");
        code.push_str(&crate::object_abi::wgsl("GpuObject"));
        code.push_str("fn affine_rows(m:mat4x4<f32>)->mat3x4<f32> { let t=transpose(m); return mat3x4<f32>(t[0],t[1],t[2]); }\n");
        writeln!(
            code,
            "@group(0) @binding({}) var<storage,read> parents:array<Object>;\n@group(0) @binding({}) var<storage,read_write> objects:array<GpuObject>;\n@group(0) @binding({}) var<storage,read_write> commands:array<Args>;",
            count * 2,
            count * 2 + 1,
            count * 2 + 2
        )?;
        writeln!(
            code,
            "@group(0) @binding({}) var<storage,read> source_indices:array<u32>;",
            count * 2 + 3
        )?;
        let indices: Vec<u32> = self
            .instances
            .iter()
            .flat_map(|instance| {
                instance
                    .indices
                    .iter()
                    .zip(draws[instance.draw].instances.clone())
                    .flat_map(|(&source, object)| {
                        [source, objects[object as usize].opacity[1] as u32, objects[object as usize].metadata[2]]
                    })
            })
            .collect();
        let indices = init(
            &r.device,
            "compute source instance indices",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::STORAGE,
        );
        writeln!(code,"@group(0) @binding({}) var<storage,read_write> placement_errors:array<atomic<u32>>;",count*2+4)?;
        let mut offset = 0;
        for instance in &self.instances {
            // Pose/material grouping partitions the original transform domain.
            writeln!(
                code,
                "fn source_{}(i:u32)->u32 {{ return source_indices[({offset}u+i)*3u]; }}",
                instance.draw
            )?;
            offset += instance.indices.len();
            for previous in [false, true] {
                let prefix = if previous { "previous" } else { "current" };
                for (column, binding) in instance.trs.iter().enumerate() {
                    if let Some(binding) = binding {
                        writeln!(code,"fn load_{prefix}_draw_{}_{}(word:u32)->f32 {{",instance.draw,column)?;
                        for span in &binding.spans {
                            let first = span.first * binding.width;
                            let end = (span.first + span.count) * binding.width;
                            writeln!(code,"if(word >= {first}u && word < {end}u) {{ return bitcast<f32>({prefix}_{}[{}u+word-{first}u]); }}",span.page,span.offset)?;
                        }
                        code.push_str("return 0.; }\n");
                    }
                }
                let vector = |column:usize,width:u32,default:&str| {
                    if instance.trs[column].is_none() { return default.to_owned(); }
                    format!("vec{width}<f32>({})",(0..width).map(|component|
                        format!("load_{prefix}_draw_{}_{}(i*{width}u+{component}u)",instance.draw,column)).collect::<Vec<_>>().join(","))
                };
                let t=vector(0,3,"vec3<f32>(0.)");
                let q=vector(1,4,"vec4<f32>(0.,0.,0.,1.)");
                let scale=vector(2,3,"vec3<f32>(1.)");
                writeln!(code,"fn matrix_{prefix}_{}(i:u32)->mat4x4<f32> {{
                    let t={t}; let raw_q={q}; let s={scale};
                    let q_length=dot(raw_q,raw_q);
                    if(!finite3(t) || !finite3(s) || !finite4(raw_q) || !finite_f32(q_length) || !(abs(q_length-1.)<=0.001)) {{
                        atomicOr(&placement_errors[0],2u);
                        return mat4x4<f32>(vec4(1.,0.,0.,0.),vec4(0.,1.,0.,0.),vec4(0.,0.,1.,0.),vec4(0.,0.,0.,1.));
                    }}
                    let q=normalize(raw_q);
                    let x=q.x; let y=q.y; let z=q.z; let w=q.w;
                    return mat4x4<f32>(
                        vec4<f32>(vec3<f32>(1.-2.*(y*y+z*z),2.*(x*y+w*z),2.*(x*z-w*y))*s.x,0.),
                        vec4<f32>(vec3<f32>(2.*(x*y-w*z),1.-2.*(x*x+z*z),2.*(y*z+w*x))*s.y,0.),
                        vec4<f32>(vec3<f32>(2.*(x*z+w*y),2.*(y*z-w*x),1.-2.*(x*x+y*y))*s.z,0.),
                        vec4<f32>(t,1.)); }}",instance.draw)?;
            }
        }
        writeln!(code, "fn update(i:u32) {{")?;
        let mut groups = 1;
        let mut id_offset = 0;
        for (parent_index, instance) in self.instances.iter().enumerate() {
            let draw = &draws[instance.draw];
            let capacity = draw.instances.end - draw.instances.start;
            groups = groups.max(capacity.div_ceil(constants::PROGRAM_WORKGROUP));
            let identity = instance.indices.iter().copied().eq(0..capacity);
            let source_live = instance
                .count
                .as_ref()
                .map(|b| {
                    let sp = &b.spans[0];
                    format!(
                        "min(current_{}[{}u],{}u)",
                        sp.page, sp.offset, instance.capacity
                    )
                })
                .unwrap_or(format!("{}u", instance.capacity));
            if let Some(binding)=&instance.count {
                let span=&binding.spans[0];
                writeln!(code,"if(i==0u && current_{}[{}u]>{}u) {{ atomicOr(&placement_errors[0],1u); }}",span.page,span.offset,instance.capacity)?;
            }
            let previous_live=source_live.replace("current_","previous_");
            let live = if identity {
                format!("min({source_live},{capacity}u)")
            } else {
                format!("{capacity}u")
            };
            writeln!(
                code,
                "if(i < {capacity}u) {{ let o={}u+i; let parent=parents[{parent_index}u]; var value:GpuObject; value.opacity=vec2(parent.opacity.x,f32(source_indices[({id_offset}u+i)*3u+1u])); value.material_index=parent.material_index.x; value.metadata=parent.metadata.xy; value.reserved=source_indices[({id_offset}u+i)*3u+2u]; let source=source_{}(i); value.padding=vec2(parent.material_index.y,source); if(source >= {source_live}) {{ value.metadata.y=0u; }} var model=parent.model; if(source < {source_live}) {{ model=parent.model * matrix_current_{}(source); }} if(!finite4(model[0]) || !finite4(model[1]) || !finite4(model[2]) || !finite4(model[3])) {{ atomicOr(&placement_errors[0],2u); value.metadata.y=0u; model=mat4x4<f32>(vec4(1.,0.,0.,0.),vec4(0.,1.,0.,0.),vec4(0.,0.,1.,0.),vec4(0.,0.,0.,1.)); }} value.model=affine_rows(model); value.next_model=affine_rows(model); value.previous_model=value.model; if(source < {source_live} && source < {previous_live}) {{ value.previous_model=affine_rows(parent.previous_model * matrix_previous_{}(source)); }} let det=dot(model[0].xyz,cross(model[1].xyz,model[2].xyz)); if(!finite_f32(det)) {{ atomicOr(&placement_errors[0],2u); }} if(det==0. || !finite_f32(det)) {{ value.metadata.y=0u; }} objects[o]=value; if(i==0u) {{ commands[{}u].instances={live}; }} }}",
                draw.instances.start, instance.draw, instance.draw, instance.draw, instance.draw
            )?;
            id_offset += instance.indices.len();
        }
        let width = groups.min(r.device.limits().max_compute_workgroups_per_dimension);
        writeln!(
            code,
            "}}\n@compute @workgroup_size({0}) fn vork_entry(@builtin(workgroup_id) group:vec3<u32>, @builtin(local_invocation_index) lane:u32) {{ let g=group.y*{width}u+group.x; if(g<{groups}u) {{ update(g*{0}u+lane); }} }}",
            constants::PROGRAM_WORKGROUP
        )?;
        let mut types = vec![Some(true); count * 2 + 1];
        types.extend([Some(false), Some(false), Some(true), Some(false)]);
        let pipeline = pipeline_layout(r, &code, "compute instance bridge", &types, "vork_entry");
        Ok((pipeline, groups, indices))
    }
}
impl Slot {
    pub(crate) fn parent_offset(&self, object: usize) -> Option<u64> {
        self.objects
            .iter()
            .position(|range| range.start == object as u32)
            .map(|index| (index * std::mem::size_of::<Object>()) as u64)
    }
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        for bind in &self.binds {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
            let groups = self.groups;
            let width = groups.min(self.width);
            pass.dispatch_workgroups(width, groups.div_ceil(width), 1);
        }
    }
}
// Explicit layouts preserve declared bindings even when Slang eliminates unused accessors.
pub(crate) fn pipeline_layout(
    r: &Renderer,
    source: &str,
    label: &str,
    types: &[Option<bool>],
    entry_point: &str,
) -> wgpu::ComputePipeline {
    let entries: Vec<_> = types
        .iter()
        .enumerate()
        .map(|(i, ty)| wgpu::BindGroupLayoutEntry {
            binding: i as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: ty.map_or(wgpu::BufferBindingType::Uniform, |read_only| {
                    wgpu::BufferBindingType::Storage { read_only }
                }),
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();
    let group = r
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &entries,
        });
    let layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&group)],
            immediate_size: 0,
        });
    let module = crate::timing::shader_module(&r.device, wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        module: &module,
        entry_point: Some(entry_point),
        compilation_options: Default::default(),
        cache: r.cache.as_ref(),
    })
}

// Offset each physical dispatch in the authored thread domain. Every tile is
// independent; the entry guard runs before addition to avoid u32 wrap at the edge.
fn dispatch_tiles(groups: [u32; 3], limit: u32) -> Vec<([u32; 3], [u32; 4])> {
    let mut tiles = Vec::new();
    for z in (0..groups[2]).step_by(limit as usize) {
        for y in (0..groups[1]).step_by(limit as usize) {
            for x in (0..groups[0]).step_by(limit as usize) {
                tiles.push((
                    [
                        (groups[0] - x).min(limit),
                        (groups[1] - y).min(limit),
                        (groups[2] - z).min(limit),
                    ],
                    [x * constants::PROGRAM_WORKGROUP, y, z, 0],
                ));
            }
        }
    }
    tiles
}

fn nominal_frame(time:&vork_time::Request,fps:vork_time::Time,frames:u32)->Result<u32> {
    Ok(time.exact()?.scale(&vork_time::exact::decimal(&fps.to_string())?).floor_clamped(0,i64::from(frames)-1)? as u32)
}

/// Typed element access over the shared tightly packed storage pages.
fn typed_accessors(
    header: &mut String,
    binding: &str,
    kind: vork_scene::v2::BufferType,
) -> Result<()> {
    use vork_scene::v2::BufferType::*;
    let (ty, width, decode, encode) = match kind {
        F32 => ("float", 1, "asfloat", "asuint"),
        I32 => ("int", 1, "asint", "asuint"),
        U32 => ("uint", 1, "", ""),
        Vec2 => ("float2", 2, "asfloat", "asuint"),
        Vec3 => ("float3", 3, "asfloat", "asuint"),
        Vec4 => ("float4", 4, "asfloat", "asuint"),
        Uvec4 => ("uint4", 4, "", ""),
        Mat4 => ("VorkMat4", 16, "asfloat", "asuint"),
    };
    // Column vectors preserve the scene buffer's column-major matrix layout.
    if matches!(kind, Mat4) && !header.contains("struct VorkMat4") {
        header.push_str("struct VorkMat4 { float4 c0; float4 c1; float4 c2; float4 c3; };\n");
    }
    writeln!(
        header,
        "{ty} {binding}_load(uint i) {{ {ty} value; if(i >= {binding}_count) return ({ty})0;"
    )?;
    for word in 0..width {
        let field = if width == 1 {
            String::new()
        } else if width == 16 {
            format!(".c{}[{}]", word / 4, word % 4)
        } else {
            format!("[{word}]")
        };
        writeln!(
            header,
            "value{field} = {decode}(_vork_{binding}_load(i*{width}+{word}));"
        )?;
    }
    writeln!(
        header,
        "return value; }}\nvoid {binding}_store(uint i, {ty} value) {{ if(i >= {binding}_count) return;"
    )?;
    for word in 0..width {
        let field = if width == 1 {
            String::new()
        } else if width == 16 {
            format!(".c{}[{}]", word / 4, word % 4)
        } else {
            format!("[{word}]")
        };
        writeln!(
            header,
            "_vork_{binding}_store(i*{width}+{word}, {encode}(value{field}));"
        )?;
    }
    header.push_str("}\n");
    Ok(())
}

#[cfg(all(test, vork_private_tests))]
include!("compute_tests_private.rs");

#[cfg(test)]
#[test]
#[ignore = "water4 phase timing on a real GPU; VORK_WATER_SCENE names the frozen scene"]
fn water_simulation_and_reconstruction_timing() -> Result<()> {
    let path=std::env::var_os("VORK_WATER_SCENE").map(PathBuf::from)
        .unwrap_or_else(||Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/water/scene.json"));
    let mut scene=Scene::from_json(&std::fs::read(&path)?)?;
    scene.render.resolution=[32,32];
    let r=Renderer::new()?;
    let s=r.install(scene,path.parent().unwrap())?;
    let compute=s.compute.as_ref().context("water simulation")?;
    let sim=compute.simulation.as_ref().context("water clock")?;
    // This example's tick first advances the fluid/markers, then reconstructs
    // their display and pressure fields. Keep the actual ordered programs.
    let split=sim.steps.iter().position(|name|name=="marker_advect").context("water marker advancement")?+1;
    let mut cursor=compute.cursor.lock().unwrap();
    r.wait(None)?;
    let mut totals=[0.;3];
    for tick in 0..6 {
        for (phase,names) in [&sim.steps[..split],&sim.steps[split..]].into_iter().enumerate() {
            let start=std::time::Instant::now();
            let mut encoder=r.device.create_command_encoder(&Default::default());
            compute.set_tick(&mut encoder,tick,sim.t0.as_f64()+f64::from(tick)*sim.dt.as_f64(),sim.dt.as_f64(),sim.seed);
            compute.run_named(&mut encoder,&mut cursor,names,(sim.t0.as_f64()+f64::from(tick)*sim.dt.as_f64()) as f32);
            let submission=r.queue.submit([encoder.finish()]);r.wait(Some(submission))?;
            let seconds=start.elapsed().as_secs_f64();
            eprintln!("water phase tick={tick} phase={} wall_s={seconds:.6}",if phase==0 {"simulation"} else {"reconstruction"});
            if tick>0 {totals[phase]+=seconds;}
        }
        let start=std::time::Instant::now();
        let mut encoder=r.device.create_command_encoder(&Default::default());
        for draw in &s.draws {if let Some(field)=&draw.implicit {field.acceleration.encode(&mut encoder);}}
        let submission=r.queue.submit([encoder.finish()]);r.wait(Some(submission))?;
        let seconds=start.elapsed().as_secs_f64();
        eprintln!("water phase tick={tick} phase=occupancy wall_s={seconds:.6}");
        if tick>0 {totals[2]+=seconds;}
    }
    eprintln!("water warm five-tick frame (48 fps): simulation_wall_s={:.6} reconstruction_wall_s={:.6} occupancy_five_publications_wall_s={:.6}",totals[0],totals[1],totals[2]);
    Ok(())
}
