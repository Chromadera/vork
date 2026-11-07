//! InstalledScene: the immutable GPU scene built once per shot.
use super::*;

/// Attribute normalized resources to every authored slot, preserving sharing.
pub(crate) fn medium_use_sites(nodes: &[Node], media: &std::collections::BTreeSet<&str>) -> Vec<String> {
    fn visit(nodes: &[Node], parent: &str, media: &std::collections::BTreeSet<&str>, labels: &mut Vec<String>) {
        for (index, node) in nodes.iter().enumerate() {
            let path = format!("{parent}[{index}]");
            let label = format!("{path} ({:?})", node.name);
            let mut slot = |property: String, material: &str| {
                if media.contains(material) {
                    labels.push(format!("{label}.{property} (medium material {material:?})"));
                }
            };
            match &node.kind {
                NodeKind::Mesh { materials, .. } => for (index, material) in materials.iter().enumerate() {
                    slot(format!("materials[{index}]"), material);
                },
                NodeKind::Asset { materials, .. } => for (name, material) in materials {
                    slot(format!("materials[{name:?}]"), material);
                },
                NodeKind::Light { medium: Some(material), .. } => slot("medium (world medium)".into(), material),
                _ => {},
            }
            visit(&node.children, &format!("{path}.children"), media, labels);
        }
    }
    let mut labels = Vec::new();
    visit(nodes, "nodes", media, &mut labels);
    labels
}

/// Ignore retired medium bindings before planning any GPU work. Stage reflection
/// distinguishes medium-only boundaries from combined surface/medium programs.
pub(crate) fn ignore_participating_media(r: &Renderer, scene: &mut Scene, base: &Path) -> Result<Vec<String>> {
    let stages = programs::mesh_stages(r, scene, base)?;
    let mut media: std::collections::BTreeSet<String> = stages.iter()
        .filter(|(_, stage)| stage.medium).map(|(name, _)| name.clone()).collect();
    fn worlds(nodes: &[Node], media: &mut std::collections::BTreeSet<String>) {
        for node in nodes {
            if let NodeKind::Light { medium: Some(name), .. } = &node.kind { media.insert(name.clone()); }
            worlds(&node.children, media);
        }
    }
    worlds(&scene.nodes, &mut media);
    let mut sites = medium_use_sites(&scene.nodes, &media.iter().map(String::as_str).collect());
    fn volumes(nodes: &[Node], geometry: &vork_scene::v2::Table<vork_scene::v2::Geometry>, sites: &mut Vec<String>) {
        for node in nodes {
            if let NodeKind::Mesh { geometry: name, .. } = &node.kind {
                if matches!(geometry[name].topology, vork_scene::v2::Topology::Volume) {
                    sites.push(format!("node {:?}.geometry {name:?} (volume topology)", node.name));
                }
            }
            volumes(&node.children, geometry, sites);
        }
    }
    volumes(&scene.nodes, &scene.geometry, &mut sites);
    fn clear(nodes: &mut [Node], stages: &std::collections::BTreeMap<String, programs::Stages>, geometry: &vork_scene::v2::Table<vork_scene::v2::Geometry>) {
        for node in nodes {
            match &mut node.kind {
                NodeKind::Light { medium, .. } => *medium=None,
                NodeKind::Mesh { materials, geometry: name, .. } if matches!(geometry[&*name].topology,vork_scene::v2::Topology::Volume) || (!materials.is_empty() && materials.iter().all(|name| stages.get(name).is_some_and(|s|s.medium && !s.surface))) => node.kind=NodeKind::Node {},
                _ => {},
            }
            clear(&mut node.children, stages, geometry);
        }
    }
    let medium_request = serde_json::json!({"sites":sites,"materials":media.iter().filter_map(|name| scene.materials.get(name).map(|m|(name,m))).collect::<std::collections::BTreeMap<_,_>>()});
    clear(&mut scene.nodes, &stages, &scene.geometry);
    fn one_world(nodes:&mut [Node],found:&mut Option<String>,ignored:&mut std::collections::BTreeSet<String>,next:&mut usize) {
        for node in nodes {
            let index = *next; *next += 1;
            if matches!(node.kind,NodeKind::Light {shape:vork_scene::LightShape::Environment,..}) {
                if let Some(first) = found.as_ref() {
                    crate::warnings::additional_environment(node, first, index);
                    ignored.insert(node.name.clone());node.kind=NodeKind::Node {};
                } else {*found=Some(format!("node {index} ({:?})", node.name));}
            }
            one_world(&mut node.children,found,ignored,next);
        }
    }
    let mut ignored_worlds=std::collections::BTreeSet::new();
    one_world(&mut scene.nodes,&mut None,&mut ignored_worlds,&mut 0);
    // Report the exact authored keys as well as the dropped light itself.
    for layer in &scene.animation {
        for track in layer.tracks.iter().flatten() {
            if matches!(track.path.as_slice(),[root,name,field,..] if root=="nodes" && ignored_worlds.contains(name) && !matches!(field.as_str(),"translation"|"rotation"|"scale"|"visible")) {
                crate::warnings::dropped("additional_environment_track", serde_json::json!({"track":track}), "additional environment light-property animation; retaining the first environment".into());
            }
        }
    }
    for layer in &mut scene.animation {
        if let Some(tracks)=&mut layer.tracks {tracks.retain(|t|!matches!(t.path.as_slice(),[root,name,field,..] if root=="nodes" && ignored_worlds.contains(name) && !matches!(field.as_str(),"translation"|"rotation"|"scale"|"visible")));}
    }
    // Asset remaps and mixed material slots keep their surface geometry. Pure
    // medium slots become invisible, rather than an opaque default material.
    let pure: std::collections::BTreeSet<_> = media.iter().filter(|name| stages.get(*name).is_none_or(|s|!s.surface)).cloned().collect();
    for name in &pure {
        scene.materials.insert(name.clone(), serde_json::from_value(serde_json::json!({"program":"std/openpbr","inputs":{"geometry_opacity":0}}))?);
    }
    for layer in &mut scene.animation {
        if let Some(tracks)=&mut layer.tracks { tracks.retain(|t| {
            let dropped = t.path.first().is_some_and(|p|p=="materials") && t.path.get(1).is_some_and(|p|pure.contains(p));
            if dropped { crate::warnings::dropped("participating_media_track", serde_json::json!({"track":t}), "participating-medium material animation; medium transport is disabled".into()); }
            !dropped
        }); }
    }
    scene.media_disabled=true;
    if !sites.is_empty() { crate::warnings::dropped("participating_media", medium_request, format!("warning: ignored participating media for this release: {}", sites.join(", "))); }
    Ok(sites)
}

/// Authored material names bound to a mesh's material slots (glTF material
/// indices or native geometry groups); unbound imported slots keep glTF materials.
struct MaterialBindings {
    slots: Vec<Option<String>>,
    default: Option<String>,
    /// Program instances of the bound non-OpenPBR materials.
    programs: std::collections::BTreeMap<String, std::sync::Arc<programs::Instance>>,
}
impl MaterialBindings {
    fn compile(input: &Scene, source: &Node, doc: Option<&gltf::Document>, stages: &std::collections::BTreeMap<String, programs::Stages>) -> Result<Self> {
        let mut slots = vec![None; doc.map_or(0, |doc| doc.materials().len())];
        let mut default = None;
        match &source.kind {
            NodeKind::Mesh { materials, .. } => {
                slots = materials.iter().cloned().map(Some).collect();
                if slots.is_empty() { slots.push(None); }
            }
            NodeKind::Asset { materials, .. } => {
                default = materials.get("*").cloned();
                slots.fill(default.clone());
                let mut specific = vec![false; slots.len()];
                for (key, material) in materials.iter().filter(|(key, _)| key.as_str() != "*") {
                    let targets: Vec<_> = if let Some(index) = key
                        .strip_prefix('#')
                        .and_then(|index| index.parse::<usize>().ok())
                    {
                        ensure!(index < slots.len(), "unknown imported material index {key}");
                        vec![index]
                    } else {
                        let targets: Vec<_> = doc
                            .context("asset material bindings require a glTF document")?
                            .materials()
                            .filter(|candidate| candidate.name() == Some(key.as_str()))
                            .map(|material| material.index().unwrap())
                            .collect();
                        ensure!(!targets.is_empty(), "unknown imported material {key:?}");
                        targets
                    };
                    for slot in targets {
                        ensure!(
                            !specific[slot],
                            "multiple remaps target imported material #{slot}"
                        );
                        specific[slot] = true;
                        slots[slot] = Some(material.clone());
                    }
                }
            }
            _ => {}
        }
        let mut programs = std::collections::BTreeMap::new();
        for name in slots.iter().flatten().chain(default.iter()) {
            if stages.get(name).is_none_or(|s| s.surface)
                && !programs.contains_key(name)
                && let Some(program) = programs::instance(input, name)?
            {
                programs.insert(name.clone(), std::sync::Arc::new(program));
            }
        }
        Ok(Self {
            slots,
            default,
            programs,
        })
    }
    fn select(&self, slot: Option<usize>) -> Option<&str> {
        match slot.and_then(|slot| self.slots.get(slot)) {
            Some(name) => name.as_deref(),
            None => self.default.as_deref(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ImplicitField {
    pub source: String,
    pub dims: [u32;3],
    pub acceleration: std::sync::Arc<crate::implicit_acceleration::Acceleration>,
    pub spans: Vec<(wgpu::Buffer, buffers::AttributeSpan)>,
}
#[derive(Clone)]
pub(crate) struct Draw {
    pub(crate) implicit: Option<ImplicitField>,
    pub(crate) sort_order: usize,
    pub(crate) vertices: wgpu::Buffer,
    pub(crate) attributes: wgpu::Buffer,
    pub(crate) layout: crate::geometry::VertexLayout,
    /// Bit n: the geometry has TEXCOORD_n (material taps may read only those).
    pub(crate) uv_sets: u8,
    pub(crate) exit_geometry: Option<std::sync::Arc<crate::geometry::Geometry>>,
    pub(crate) exit_bvh: Option<wgpu::Buffer>,
    /// Draws of meshes that own a path solid: this draw's triangles' edges a
    /// closed mesh would not have (passes::path::open_edges); None where the
    /// geometry is not retained.
    pub(crate) open_edges: Option<std::sync::Arc<crate::passes::path::OpenEdges>>,
    pub(crate) indices: wgpu::Buffer,
    /// Shared static page; evaluated-layout and compute geometry own their buffers.
    pub(crate) geometry_page: Option<usize>,
    pub(crate) vertex_offset: u64,
    pub(crate) index_offset: u64,
    pub(crate) count: u32,
    /// Vertices addressed by this draw's indices (ray BLAS input range).
    pub(crate) vertex_count: u32,
    /// Alpha-tested draws of ray scenes: base-colour UV and vertex-colour alpha per
    /// index, for ray candidates. Released once install has built the alpha table.
    pub(crate) ray_corners: Option<std::sync::Arc<Vec<[f32; 5]>>>,
    pub(crate) material: usize,
    pub(crate) material_slot: Option<usize>,
    pub(crate) object: u32,
    pub(crate) instances: std::ops::Range<u32>,
    pub(crate) sort_stride: usize,
    pub(crate) local: Mat4,
    pub(crate) gltf_node: (usize, String),
    pub(crate) gltf_path: Vec<(usize, String)>,
    pub(crate) primitive_start: u32,
    pub(crate) center: Vec3,
    pub(crate) bounds: (Vec3, Vec3),
    pub(crate) generated_bounds: (Vec3, Vec3),
    pub(crate) computed: bool,
    pub(crate) deform: Option<Deform>,
    pub(crate) displacement: Option<std::sync::Arc<displacement::Mesh>>,
    /// Matching complete evaluation inputs may share a frame-private range.
    pub(crate) displacement_leader: Option<usize>,
}
impl Draw {
    pub(crate) fn evaluated(&self) -> bool {self.computed || self.deform.is_some() || self.displacement.is_some()}
    pub(crate) fn evaluation_leader(&self) -> Option<usize> {
        self.displacement_leader.or_else(|| self.deform.as_ref().map(|d| d.leader))
    }
    pub(crate) fn attribute_offset(&self) -> u64 {
        self.vertex_offset / self.layout.vertex_stride() * self.layout.attribute_stride()
    }
}
pub(crate) struct EvalNode {
    pub(crate) camera: Option<crate::camera_program::State>,
    pub(crate) node: Node,
    pub(crate) parent: Option<usize>,
    pub(crate) world: Mat4,
    pub(crate) dynamic: bool,
    pub(crate) embedded: Option<(usize, usize, usize, Option<f32>)>,
}
#[derive(Clone)]
pub(crate) struct MeshRoot {
    pub(crate) node: usize,
    pub(crate) local: Mat4,
    pub(crate) time_offset: f32,
    pub(crate) instance_index: Option<u32>,
    pub(crate) imported_visibility: u32,
    pub(crate) imported_pose: Option<(usize, usize)>,
}
impl MeshRoot {
    /// Placed by the same node, local transform and time offset as `other`.
    pub(crate) fn same_placement(&self, other: &Self) -> bool {
        self.node == other.node && self.local == other.local && self.time_offset == other.time_offset
    }
    pub(crate) fn world(&self, worlds: &[Mat4]) -> Mat4 {
        worlds[self.node] * self.local
    }
}

/// Structural choices fixed for one install, independent of other installs.
pub struct InstallOptions {
    /// Initial requested scene time in seconds, used for pose and camera initialization.
    pub initial_time: f32,
    /// Per-install render settings; None uses the scene's render settings.
    pub render: Option<vork_scene::Render>,
    /// CPU planes frames may read back. The graph prunes readback, and its
    /// pixel buffer, when nothing is read; frames may request only these planes.
    pub readback: crate::ReadbackOutputs,
    /// Initial frame storage; collection can request growth through `FrameRedo`.
    pub reservations: crate::Reservations,
    pub(crate) variants: pipelines::VariantCompile,
    /// Internal transport fixtures only; delivery drivers retain release admission.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) test_participating_media: bool,
    /// Scattering continuation budget for analytic path-medium fixtures.
    #[cfg(test)]
    pub(crate) test_volume_bounces: u32,
}
impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            initial_time: 0.,
            render: None,
            readback: crate::ReadbackOutputs {
                float: crate::FloatOutputs::ALL,
                ..crate::ReadbackOutputs::ALL
            },
            reservations: Default::default(),
            variants: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            test_participating_media: cfg!(test),
            #[cfg(test)]
            test_volume_bounces: 0,
        }
    }
}

pub struct InstalledScene {
    pub(crate) has_catchers: bool,
    pub(crate) footage: std::collections::BTreeMap<String, crate::footage::Installed>,
    pub(crate) file_images: std::collections::BTreeMap<String, wgpu::TextureView>,
    pub(crate) reservations: crate::Reservations,
    pub(crate) sampling: crate::sampling::Parameters,
    /// Shared exposure error word, protected by the normal submission lock.
    pub(crate) displacement: Option<displacement::Pipeline>,
    pub(crate) shadows: crate::residency::Cache<crate::shadows::Persistent>,
    /// Colour history shared by every frame slot for `integrator.aa(): temporal`.
    pub(crate) compute: Option<compute::Compute>,
    pub(crate) dense: Option<crate::dense::Dense>,
    pub(crate) computed_geometry: Vec<Option<crate::computed_geometry::Mesh>>,
    pub(crate) compositor: std::collections::HashMap<String, compositor::Program>,
    pub(crate) main_subset: Option<crate::render_images::Subset>,
    pub(crate) capture_controls: bool,
    /// CPU planes this install may read back.
    pub(crate) readback: crate::ReadbackOutputs,
    pub(crate) animation: std::sync::Arc<animation::Plan>,
    pub(crate) pipelines: std::sync::Arc<CachedPipelines>,
    pub(crate) plan: crate::plan::ScenePlan,
    pub(crate) graph: crate::graph::Graph,
    pub(crate) transients: crate::graph::Transients,
    pub(crate) path: Option<crate::passes::path::Pipeline>,
    pub(crate) reflections: Option<wgpu::ComputePipeline>,
    /// Decoupled shading kernels and the camera's draw table.
    pub(crate) decoupled: Option<crate::passes::decoupled::Install>,
    pub(crate) gi: Option<wgpu::ComputePipeline>,
    /// The irradiance volume when the scene enables probes.
    pub(crate) probes: Option<crate::probes::Volume>,
    pub(crate) fibre: Option<crate::fibre_transport::Persistent>,
    pub(crate) geometry_pipelines:
        std::collections::HashMap<pipelines::GeometryKey, pipelines::Drawn>,
    pub(crate) vertex_defaults: wgpu::Buffer,
    pub(crate) environment: environment::Environment,
    pub(crate) spectral: bool,
    pub(crate) propagation_base: u32,
    pub(crate) camera_programs: Vec<crate::camera_program::Instance>,
    pub(crate) surface_dependencies: Vec<vork_slang::ProgramDependencies>,
    pub(crate) links: lights::links::Links,
    pub(crate) medium_base: u32,
    pub(crate) medium_boundaries: usize,
    pub(crate) medium_certified_times: Option<[f32;2]>,
    pub(crate) emitter_profiles: std::collections::HashMap<usize,u32>,
    pub(crate) assets: Vec<Asset>,
    pub(crate) scene: Scene,
    pub(crate) render: vork_scene::Render,
    pub(crate) nodes: Vec<EvalNode>,
    pub(crate) targets: crate::targets::Plan,
    pub(crate) dynamic_objects: Vec<usize>,
    pub(crate) roots: Vec<MeshRoot>,
    pub(crate) object_ids: std::sync::Arc<serde_json::Value>,
    pub(crate) camera: usize,
    pub(crate) light_plan: LightPlan,
    pub(crate) draws: Vec<Draw>,
    pub(crate) implicit_fields: Vec<ImplicitField>,
    /// Static acceleration structures when the scene chooses a ray technique.
    pub(crate) rays: Option<crate::rt::Rays>,
    pub(crate) draw_groups: Vec<std::ops::Range<usize>>,
    pub(crate) materials: Vec<Material>,
    /// The scene material table: records, texture views and samplers.
    pub(crate) material_table: materials::MaterialTable,
    pub(crate) checked_shader_capacity: bool,
    pub(crate) objects: Vec<Object>,
    pub(crate) display_program: crate::display_program::Stage,
    /// Unique raster vertex and attribute buffer bytes; excludes indices and deformation workspace.
    pub geometry_vertex_bytes: u64,
    pub install_time: Duration,
    pub pipeline_ms: f64,
    pub environment_install_ms: f64,
    pub material_bind_ms: f64,
    pub asset_decode_ms: f64,
    pub texture_upload_ms: f64,
    pub texture_cache_hits: usize,
}
// A screen capture has no interior scene-boundary provider. Refuse possible
// contents rather than render a picture that drops them. Bounds are conservative:
// hardware transport or path is the complete provider when these overlap.
/// Material slots reachable from the same glTF scene used by GeometryUpload.
/// Other document scenes and unreferenced meshes do not create scene programs.
fn imported_material_slots(doc:&gltf::Document)->Result<std::collections::BTreeSet<Option<usize>>> {
    fn visit(node:gltf::Node,slots:&mut std::collections::BTreeSet<Option<usize>>) {
        if let Some(mesh)=node.mesh() {for p in mesh.primitives() {slots.insert(p.material().index());}}
        for child in node.children() {visit(child,slots);}
    }
    let scene=doc.default_scene().or_else(||doc.scenes().next()).context("glTF has no scene")?;
    let mut slots=std::collections::BTreeSet::new();
    for node in scene.nodes() {visit(node,&mut slots);}
    Ok(slots)
}

fn refuse_screen_contents(draws:&[Draw],roots:&[MeshRoot],objects:&[Object],materials:&[Material])->Result<()> {
    ensure!(!draws.iter().any(|draw|materials[draw.material].thin_transmission),
        "thin-sheet refraction needs scene boundary transport; use ray refraction on a ray-query capable adapter, or std/path (screen refraction cannot represent rough transmission and rear interfaces)");
    if !draws.iter().any(|draw|materials[draw.material].solid_transmission) {return Ok(());}
    let identities=crate::passes::path::mesh_identities(draws,roots)?;
    let placed:Vec<_>=draws.iter().flat_map(|draw|draw.instances.clone().map(move |object|(draw,object))).map(|(draw,object)| {
        let model=objects[object as usize].model;
        (draw,object,crate::shadow_envelope::transformed(model,draw.bounds))
    }).collect();
    for &(glass,object,(lo,hi)) in &placed {
        if !materials[glass.material].solid_transmission {continue;}
        for &(_,other,(other_lo,other_hi)) in &placed {
            if objects[other as usize].metadata[1]&constants::RAY_TRANSMISSION==0 {continue;}
            if identities[object as usize]==identities[other as usize] {continue;}
            if lo.cmplt(other_hi).all() && other_lo.cmplt(hi).all() {
                anyhow::bail!("solid glass contents need scene boundary transport; use a ray-query capable adapter with static placements, or std/path (screen refraction cannot represent overlapping contents)");
            }
        }
    }
    Ok(())
}

/// A light whose emission reaches receivers through shadow rays. Environment lighting never does; an extracted sun is its own node.
fn shadowed_light(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Light { shape, .. } if !matches!(shape, vork_scene::LightShape::Environment)) && kind.casts_shadows()
}

impl InstalledScene {
    /// The frame's passes and the scene's programs, to name a GPU failure. A
    /// lost device reports only the failed submission, which holds the whole frame.
    pub(crate) fn gpu_work(&self) -> String {
        let passes: Vec<_> = self.graph.passes().iter().map(|p| format!("{p:?}")).collect();
        let mut work = format!("passes {}", passes.join(", "));
        if !self.scene.programs.is_empty() {
            work += &format!("; scene programs {}", self.scene.programs.keys().cloned().collect::<Vec<_>>().join(", "));
        }
        work
    }
    pub(crate) fn retain_shadows(&self, r: &Renderer) -> Result<std::sync::Arc<crate::shadows::Persistent>> {
        self.shadows.get_or_try_init(|| {
            let shadows = crate::shadows::Persistent::new(self, r)?;
            let bytes = shadows.bytes();
            Ok((shadows, bytes))
        })
    }
    pub(crate) fn shadows(&self) -> std::sync::Arc<crate::shadows::Persistent> {
        self.shadows.get().expect("installed shadows")
    }
    pub(crate) fn new(
        r: &Renderer,
        scene: Scene,
        base: &Path,
        effects: EffectPlan,
        background_pipelines: bool,
        automatic_probes: bool,
        options: &InstallOptions,
    ) -> Result<Self> {
        std::thread::scope(|scope| Self::new_scoped(r, scene, base, effects, background_pipelines, automatic_probes, options, scope))
    }

    fn new_scoped<'scope>(
        r: &'scope Renderer,
        scene: Scene,
        base: &Path,
        mut effects: EffectPlan,
        _background_pipelines: bool,
        automatic_probes: bool,
        options: &InstallOptions,
        scope: &'scope std::thread::Scope<'scope, '_>,
    ) -> Result<Self> {
        let InstallOptions { readback, ref variants, reservations, .. } = *options;
        let input = &scene;
        if !input.render.aovs.is_empty() {
            crate::aov::check_storage(&r.device.limits(), &input.render)?;
            for name in input.image_order(None)? {
                crate::aov::check_storage(&r.device.limits(), &input.images[&name].render.as_ref().unwrap().resolve(&input.render))?;
            }
        }
        let display_program = crate::display_program::prepare(r, &scene, base)?.install(r)?;
        let mut render = scene.render.clone();
        let environment_start = Instant::now();
        let mut environment = environment::Environment::new(
            r,
            environment_node::Settings::from_scene(&scene)?,
            &scene,
            base,
        )?;
        let environment_install_ms = environment_start.elapsed().as_secs_f64() * 1000.;
        // Preorder: an authored node's index is its position in the scene tree walk.
        let mut nodes = Vec::new();
        fn flatten(input: &[Node], parent: Option<usize>, nodes: &mut Vec<EvalNode>) {
            for n in input {
                let id = nodes.len();
                let dynamic = parent.is_some_and(|p| nodes[p].dynamic);
                let world =
                    parent.map_or(Mat4::IDENTITY, |p| nodes[p].world) * n.transform.matrix();
                let mut node = n.without_children();
                if let NodeKind::Light {
                    shape, energy: intensity, ..
                } = &mut node.kind
                {
                    intensity.get_or_insert(shape.default_energy());
                }
                nodes.push(EvalNode { camera: None,
                    node,
                    parent,
                    world,
                    dynamic,
                    embedded: None,
                });
                flatten(&n.children, Some(id), nodes);
            }
        }
        flatten(&scene.nodes, None, &mut nodes);
        let propagation=crate::propagation::prepare(r,input,base)?;
        let buffer_data = buffers::SceneBuffers::decode(input, base)?;
        let mut animation = animation::Plan::compile_with_buffers(input, base, Some(&buffer_data.data))?;
        let camera_views=crate::camera_program::views(input,&nodes,&animation,options.initial_time);
        let mut camera_programs=crate::camera_program::install(r,input,&mut nodes,base,&camera_views)?;
        let spectral=crate::camera_program::scene_spectral(r,input,base)?;
        let camera_native=(spectral || !camera_programs.is_empty()).then(||std::sync::Arc::new(crate::camera_program::native(&camera_programs)));
        environment.node = environment.settings.source;
        let mut links = lights::links::Links::new(&nodes)?;
        if render.settings.integrator.probe_volume().is_some() {
            ensure!(environment.node.is_none_or(|world| links.illuminates(world).all),
                "retained indirect lighting cannot represent environment illuminates receiver links: the aggregate field mixes World and local sources; omit illuminates or choose --profile std/path");
        }
        for node in &mut nodes {
            if let NodeKind::Mesh {
                geometry,
                morph_weights,
                ..
            } = &mut node.node.kind
            {
                for name in input.geometry[geometry.as_str()].morphs.keys() {
                    morph_weights.entry(name.clone()).or_insert(0.);
                }
            }
        }
        let mut node_names: std::collections::HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| !node.node.name.is_empty())
            .map(|(index, node)| (node.node.name.clone(), index))
            .collect();
        let curve_sides = crate::curves::projected_sides(input, &nodes, &buffer_data, &animation, &camera_views)?;
        buffers::admit_points(input, &buffer_data, &curve_sides)?;
        let mut embedded = Vec::new();
        let mut imported_camera_materials=vork_scene::v2::Table::new();
        let pipeline_start = Instant::now();
        let mut program_material_names = std::collections::BTreeSet::new();
        for node in &nodes {
            match &node.node.kind {
                NodeKind::Mesh { materials, .. } => program_material_names.extend(materials.iter()),
                NodeKind::Asset { materials, .. } => {
                    program_material_names.extend(materials.values())
                }
                _ => {}
            }
        }
        let stages = programs::mesh_stages(r, input, base)?;
        let mut decoded = std::collections::HashMap::new();
        let mut image_decoders = materials::ImageDecoders::default();
        // Imported glTF keeps its factors and texture taps in the shared material table.
        let image_order = scene.image_order(None)?;
        let image_slots = image_order.iter().enumerate().map(|(slot,name)|(name.clone(),slot)).collect();
        let mut textures = TextureInstaller::new(r, &image_slots);
        textures.fps = render.fps;
        let mut imported_materials=std::collections::BTreeMap::new();
        for node in &nodes {
            let NodeKind::Asset {file,..}=&node.node.kind else {continue;};
            let asset_key=format!("asset:{file}");
            if !decoded.contains_key(&asset_key) {
                let ((doc,buffers,images),keys)=materials::load_cached(&base.join(file),&r.disk_cache,&mut image_decoders,r.device.limits().max_texture_dimension_2d)?;
                decoded.insert(asset_key.clone(),std::rc::Rc::new(((Some(doc),buffers,images),None,keys)));
            }
            let ((doc,_,images),_,keys)=&**decoded.get(&asset_key).unwrap();
            let doc=doc.as_deref().unwrap();
            let bindings=MaterialBindings::compile(input,&node.node,Some(doc),&stages)?;
            let reachable=imported_material_slots(doc)?;
            for source in std::iter::once(None).chain(doc.materials().map(Some)) {
                let slot=source.as_ref().and_then(|m|m.index());
                let key=(file.clone(),slot);
                if !reachable.contains(&slot) || bindings.select(slot).is_some() || imported_materials.contains_key(&key) {continue;}
                let material=materials::install_gltf(&mut textures,doc,source.as_ref(),images,keys)?;
                imported_materials.insert(key,material);
            }
        }
        let native_program_materials = program_material_names
            .iter()
            .filter(|name| stages[name.as_str()].surface)
            .map(|name| Ok(programs::instance(input, name)?.map(|p| (name.as_str(), p))))
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>>>()?;
        let attribute_names = crate::attributes::names(input)?;
        let attributes=attribute_names.len() as u32;
        let required_locations=vork_slang::registry::VISIBILITY_LENS_LOCATION+1+attributes.saturating_sub(8);
        let limits=r.device.limits();
        ensure!(required_locations <= limits.max_vertex_attributes,
            "material attribute vertex layout needs {required_locations} locations ({attributes} named attributes); device max_vertex_attributes={}", limits.max_vertex_attributes);

        if attributes>8 {
            let varying_locations=14+attributes;
            ensure!(varying_locations<=limits.max_inter_stage_shader_variables,
                "material attribute pipeline needs {varying_locations} inter-stage locations; device max_inter_stage_shader_variables={}",limits.max_inter_stage_shader_variables);
        }
        let programs = programs::install(r, input, native_program_materials.iter().map(|(name, p)| (*name, p)), base)?;
        let mut physical_hair_materials = std::collections::HashSet::new();
        for name in &program_material_names {
            if let Some(instance) = programs::instance(input, name)? {
                if programs.as_ref().is_some_and(|p| p.is_physical_hair(&instance)) {
                    physical_hair_materials.insert(name.as_str());
                }
            }
        }
        let animated_inputs = materials::animated_inputs(input)?;
        // One installed capability summary drives both graph reservation and
        // material validity. Program outputs need no sentinel input names.
        let mut transmission_materials=std::collections::BTreeSet::new();
        let mut spatial_subsurface = false;
        for name in &program_material_names {
            if !stages[*name].surface { continue; }
            if let Some(instance) = programs::instance(input, name)? {
                let capabilities=programs.as_ref().context("material program was not compiled")?.capabilities(&instance);
                spatial_subsurface|=capabilities.spatial_subsurface;
                if capabilities.transmission {transmission_materials.insert(*name);}
            } else {
                let record=materials::openpbr_values(input, Some(name), animated_inputs.get(*name).unwrap_or(&Default::default()), |_| Ok(0))?;
                spatial_subsurface|=record.spatial_subsurface();
                if record.transmissive {transmission_materials.insert(*name);}
            }
        }

        // Shader-to-RGB results are final (surface_lighting::unreplaced):
        // their diffuse and reflection payloads are zero. Keep the original
        // surface's probe and deferred connection lighting for the lit input,
        // but install no screen replacement consumers when every surface is lit.
        let only_lit=nodes.iter().all(|node|match &node.node.kind {
            NodeKind::Mesh {materials,..}=>!materials.is_empty() && materials.iter().all(|name| {
                !stages[name].surface || programs::instance(input,name).ok().flatten().is_some_and(|instance|
                    programs.as_ref().is_some_and(|p|p.capabilities(&instance).lit))
            }),
            NodeKind::Asset {..}=>false,
            _=>true,
        });
        // Refraction's first reflected event also consumes projection.z; keep
        // that transport mode intact whenever a material may transmit.
        if only_lit && transmission_materials.is_empty() && !effects.path {
            effects.indirect=false;
            effects.ray_indirect=false;
            effects.reflections=false;
            render.settings.integrator.raster_mut().unwrap().reflections=vork_scene::Reflections::Environment;
        }

        // Solid scattering needs mesh chords. Any transmitting caster needs RGB
        // straight connections for every receiver; scalar depth maps cannot
        // represent its closure throughput. Both use the shared ray providers.
        // Without a shadowed light no receiver issues a light connection, so
        // the caster needs neither provider. Assets may import lights.
        let material_spatial_subsurface=spatial_subsurface;
        let spatial_subsurface=material_spatial_subsurface && !effects.path;
        let transmitting_casters=!effects.path
            && nodes.iter().any(|node| shadowed_light(&node.node.kind) || matches!(node.node.kind, NodeKind::Asset{..}))
            && nodes.iter().enumerate().any(|(id,node)| {
            if buffers::node_visibility_mask(&nodes,id)&constants::RAY_SHADOW==0
                && !crate::plan::shadow_visibility_animated(&nodes,id,&animation) {return false;}
            match &node.node.kind {
                NodeKind::Mesh{materials,..}=>materials.iter().any(|name|transmission_materials.contains(name)),
                NodeKind::Asset{..}=>true,
                _=>false,
            }
        });
        let mut promoted_transport=spatial_subsurface || transmitting_casters;
        if promoted_transport {
            let raster=render.settings.integrator.raster_mut().unwrap();
            if spatial_subsurface {raster.subsurface_method=Some(vork_scene::v2::SubsurfaceMethod::Thickness);}
            // Transmitting casters require RGB ray connections.
            if transmitting_casters {raster.shadow_method=vork_scene::ShadowMethod::Ray;}
        }
        effects.subsurface = effects.raster_diffusion(spatial_subsurface);
        effects.fibre = !effects.path && programs.as_ref().is_some_and(|p|p.has_physical_hair());

        let mut medium = medium::install(r, input, environment.settings.medium.as_deref(), base, &stages)?;
        let filtered_media=medium.is_some() && !effects.path;
        if effects.path && let Some(error)=medium.as_ref().and_then(|m|m.native.sampled_error.as_ref()) {return Err(anyhow::anyhow!("{error}").context(crate::failure::FailureKind::Scene));}
        let raster_samples = raster_samples(&scene.render);
        let mut light_shadow_rays=crate::rt::installed_shadow_rays(&scene,&nodes);
        let mut rays = crate::rt::uses_rays(&scene) || light_shadow_rays || promoted_transport;
        // Decoded mesh emitters can request volume rays after geometry upload.
        // Reserve the native input usage while scattering is possible; the
        // decoded source decision still owns actual ray/topology installation.
        let geometry_rays=rays || (medium.as_ref().is_some_and(|m|!m.native.scattering_free)
            && r.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY));
        // Device preflight treats every unknown program/asset as transmissive.
        // Installation already knows authored surface capabilities; a medium,
        // environment or opaque program must not specialize surface connections.
        let connection_transmission = rays && (!transmission_materials.is_empty()
            || input.geometry.values().any(|g|matches!(g.topology,vork_scene::v2::Topology::Implicit))
            || nodes.iter().any(|node|matches!(node.node.kind,NodeKind::Asset{..})));
        if let Some(medium) = &mut medium {
            medium.native.implicit = input.geometry.values().any(|g|matches!(g.topology,vork_scene::v2::Topology::Implicit));
            // The resolved provider changes both code and volume descriptors.
            medium.native.ray_shadows = light_shadow_rays || promoted_transport;
            medium.native.identity.push(u8::from(medium.native.ray_shadows));
            medium.native.link_shadow_programs(programs.as_ref().and_then(|p| p.native.as_deref()), connection_transmission);
        }
        let mut ray_refraction = crate::rt::ray_refraction(&scene);
        let mut ray_shadows = light_shadow_rays || ray_refraction || effects.probes || promoted_transport;
        let mut slang = crate::slang_forward::SlangScene {
                propagation: propagation.clone(),
                camera: camera_native,
                environment: environment.primary.clone(),
                programs: programs.as_ref().and_then(|p| p.native.clone()),
                aov_count: input.render.aovs.len(),
                attributes: attribute_names.len() as u32,
                textures: 1, samplers: 1,
                ray_refraction,
                connection_transmission, surface_coverage:false, shadow_capacity:0,
                no_shadow_containment: false,
                probes: effects.probes,
                fibre: effects.fibre,
                denoise: effects.denoise,
                volume_visibility: filtered_media,
                field_controls: !effects.path && input.geometry.values().any(|g|matches!(g.topology,vork_scene::v2::Topology::Implicit)),
                medium: medium.as_ref().map(|m|std::sync::Arc::new(m.native.clone())),
            };
        // Program input words follow the material records in one buffer.
        let mut material_words = environment.primary.as_ref().map_or_else(Vec::new,|p|p.words.clone());
        let propagation_offset=material_words.len();
        if let Some(p)=&propagation {material_words.extend_from_slice(&p.words);}
        for camera in &mut camera_programs {camera.offset=material_words.len();material_words.extend_from_slice(&camera.words);}
        let mut program_word_lengths = std::collections::HashMap::<u32, usize>::new();
        let mut hair_geometry_words = std::collections::HashMap::<String, u32>::new();
        let mut word_offsets = std::collections::HashMap::<(u32, Vec<u32>), u32>::new();
        let mut pipeline_ms = pipeline_start.elapsed().as_secs_f64() * 1000.;
        let mut asset_decode_ms = 0.;
        let image_order = scene.image_order(None)?;
        let mut views: Vec<_> = std::iter::once(render.clone()).chain(image_order.iter()
            .map(|name| input.images[name].render.as_ref().unwrap().resolve(&render))).map(|mut view| {
                if let Some(raster)=view.settings.integrator.raster_mut() {
                    if spatial_subsurface {raster.subsurface_method=Some(vork_scene::v2::SubsurfaceMethod::Thickness);}
                    if transmitting_casters {raster.shadow_method=vork_scene::ShadowMethod::Ray;}
                }
                view
            }).collect();
        let mut assets = Vec::new();
        let mut draws: Vec<Draw> = Vec::new();
        let mut materials = Vec::new();
        let mut objects = Vec::new();
        let mut roots = Vec::new();
        let mut source_cache = animation::SourceCache::default();
        let mut animation_sources = std::collections::HashMap::<String, Asset>::new();
        let mut material_ranges = std::collections::HashMap::<String, (Vec<usize>, usize)>::new();
        let mut material_ids = std::collections::HashMap::new();
        // Record index of each authored material name, shared by all its uses.
        let mut native_ids = std::collections::BTreeMap::<String, usize>::new();
        // Install static geometry in shared, bounded vertex/index pages. Draw order and
        // local indices remain unchanged; each draw keeps its byte offsets.
        let mut geometry_pages = Vec::new();
        let placeholder = init(
            &r.device,
            "geometry placeholder",
            &[0; 96],
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::INDEX | wgpu::BufferUsages::STORAGE,
        );
        let mut geometry_ranges = std::collections::HashMap::<
            (String, bool, Option<usize>),
            std::ops::Range<usize>,
        >::new();
        for (node_id, node) in nodes.iter().enumerate() {
            let (native_mesh, asset_source) = match &node.node.kind {
                NodeKind::Mesh {
                    geometry,
                    materials,
                    ..
                } => (Some((geometry, materials)), None),
                NodeKind::Asset {
                    file,
                    materials,
                    nodes: overrides,
                } => (None, Some((file, materials, overrides))),
                _ => continue,
            };
            let object = objects.len() as u32;
            objects.push(Object::new(node.world));
            let decode_start = Instant::now();
            let (key, material_key) = match (native_mesh, asset_source) {
                (Some((geometry, materials)), _) => (
                    format!("mesh:{geometry}"),
                    format!("mesh:{geometry}:{}", serde_json::to_string(materials)?),
                ),
                (_, Some((path, materials, _))) => (
                    format!("asset:{path}"),
                    format!("asset:{path}:{}", serde_json::to_string(materials)?),
                ),
                _ => unreachable!(),
            };
            let stream_layout = native_mesh.is_some_and(|(name,_)| crate::computed_geometry::dynamic(input, &input.geometry[name.as_str()]));
            let evaluated_layout=stream_layout || match (native_mesh,asset_source) {
                (Some((_,materials)),_)=>materials.iter().any(|name|stages.get(name).is_some_and(|s|s.displacement)),
                (_,Some((_,materials,_)))=>materials.values().any(|name|stages.get(name).is_some_and(|s|s.displacement)),
                _=>false,
            };
            let key=if evaluated_layout {format!("{key}:evaluated")} else {key};
            let loaded = if let Some(loaded) = decoded.get(&key) {
                std::rc::Rc::clone(loaded)
            } else {
                let (loaded, geometry, image_keys) = if let Some((path, ..)) = asset_source {
                    let path = base.join(path);
                    let (loaded, keys) = materials::load_cached(
                        &path,
                        &r.disk_cache,
                        &mut image_decoders,
                        r.device.limits().max_texture_dimension_2d,
                    )
                        .with_context(|| format!("load {}", path.display()))?;
                    (
                        {
                            let (doc, buffers, images) = loaded;
                            (Some(doc), buffers, images)
                        },
                        None,
                        keys,
                    )
                } else {
                    let (name, _) = native_mesh.unwrap();
                    let geometry = buffers::geometry_v2(
                        &buffer_data,
                        &input.geometry[name.as_str()],
                        &attribute_names,
                        curve_sides.get(name.as_str()).copied().unwrap_or(8),
                        Some(&r.disk_cache),
                    )?;
                    let geometry = crate::subdivision::prepare(r, &buffer_data, &input.geometry[name.as_str()], geometry)?;
                    ((None, std::sync::Arc::new(vec![]), vec![]), Some(geometry), Vec::new())
                };
                let loaded = std::rc::Rc::new((loaded, geometry, image_keys));
                decoded.insert(key.clone(), std::rc::Rc::clone(&loaded));
                loaded
            };
            let ((doc, buffers, _), geometry, _) = &*loaded;
            if native_mesh.is_some_and(|(_, names)| names.iter().any(|name| physical_hair_materials.contains(name.as_str()))) {
                if let Some(geometry) = geometry.as_ref().filter(|g| g.vertices.first().is_some_and(|v| v.TANGENT[3].abs() >= 2.)) {
                    let offset = if let Some(&offset) = hair_geometry_words.get(&key) { offset } else {
                        let offset = u32::try_from(material_words.len())?;
                        material_words.extend(crate::hair::geometry_words(geometry)?);
                        hair_geometry_words.insert(key.clone(), offset);
                        offset
                    };
                    objects[object as usize].material_index[1] = offset;
                    objects[object as usize].material_index[2] = 1;
                }
            }
            if let Some((_, _, overrides)) = asset_source {
                let doc = doc.as_ref().context("asset requires a glTF document")?;
                let mut targets = std::collections::HashSet::new();
                for key in overrides.keys() {
                    let indices: Vec<_> = if let Some(index) =
                        key.strip_prefix('#').and_then(|s| s.parse::<usize>().ok())
                    {
                        doc.nodes()
                            .filter(|node| node.index() == index)
                            .map(|node| node.index())
                            .collect()
                    } else {
                        doc.nodes()
                            .filter(|node| node.name() == Some(key.as_str()))
                            .map(|node| node.index())
                            .collect()
                    };
                    ensure!(
                        indices.len() == 1,
                        "asset override {key:?} must name exactly one imported node or #index"
                    );
                    ensure!(
                        targets.insert(indices[0]),
                        "multiple overrides target imported node #{}",
                        indices[0]
                    );
                }
            }
            asset_decode_ms += decode_start.elapsed().as_secs_f64() * 1000.;
            let asset_id = asset_source.is_some().then_some(assets.len());
            if let (Some(asset), Some((_, _, overrides))) = (asset_id, asset_source) {
                lights::import(
                    doc.as_ref().context("asset requires a glTF document")?,
                    node_id,
                    asset,
                    node.world,
                    &node.node.name,
                    overrides,
                    &mut embedded, &mut imported_camera_materials,
                )?;
            }
            let native_deformation = match &node.node.kind {
                NodeKind::Mesh { geometry, skin, .. } => {
                    skin.is_some() || !input.geometry[geometry.as_str()].morphs.is_empty()
                }
                _ => false,
            };
            let native_binding = if native_deformation {
                Some(buffers::native_binding(
                    input,
                    &nodes,
                    &node_names,
                    node_id,
                    &buffer_data,
                )?)
            } else {
                None
            };
            if let Some((path, ..)) = asset_source {
                let doc = doc.as_ref().context("asset requires a glTF document")?;
                // Clip layers bound to this asset, by plan layer index.
                let clips: Vec<_> = animation.clips_for_asset(&node.node.name).collect();
                let animation_key = format!("{path}:{}", serde_json::to_string(&clips)?);
                let template = if let Some(template) = animation_sources.get(&animation_key) {
                    template.clone()
                } else {
                    let template =
                        Asset::cached(doc, buffers, &clips, base, path, &mut source_cache)?;
                    animation_sources.insert(animation_key, template.clone());
                    template
                };
                assets.push(template.register_native(
                    &mut animation,
                    doc,
                    &node.node.name,
                    asset_id.expect("glTF asset registered"),
                    asset_source.unwrap().2,
                )?);
            }
            let asset_animated = if native_mesh.is_some() {
                native_deformation
            } else {
                assets[asset_id.expect("glTF asset registered")].animated()
            };
            let (material_map, default_material) = if let Some(range) =
                material_ranges.get(&material_key)
            {
                range.clone()
            } else {
                let bindings = MaterialBindings::compile(input, &node.node, doc.as_deref(), &stages)?;
                if let Some(doc) = &doc {
                    for primitive in doc.meshes().flat_map(|m| m.primitives()) {
                        if let Some(material) = bindings
                            .select(primitive.material().index())
                            .and_then(|name| input.materials.get(name))
                        {
                            for (input_name,value) in &material.inputs {
                                if let vork_scene::v2::Input::Attribute(binding) = value {
                                    let accessor=primitive.attributes().find(|(semantic,_)|semantic.to_string()==binding.attribute).map(|(_,a)|a)
                                        .with_context(||format!("asset material input {input_name}: missing attribute {:?}",binding.attribute))?;
                                    let width=if material.program=="std/openpbr" {
                                        match vork_scene::OpenPbrParam::find(input_name).context("OpenPBR attribute input")?.kind {
                                            vork_scene::OpenPbrKind::Float=>1,vork_scene::OpenPbrKind::Color3=>3,_=>anyhow::bail!("attribute requires a numeric destination")
                                        }
                                    } else {
                                        let instance=programs::instance(input,bindings.select(primitive.material().index()).unwrap())?.context("attribute program instance")?;
                                        programs.as_ref().and_then(|p|p.attribute_width(&instance,input_name)).context("attribute bindings require Attribute<T> Inputs")?
                                    };
                                    attributes::validate(binding,width,accessor.dimensions().multiplicity() as u32)?;
                                }
                            }
                        }
                    }
                }
                // Tracks write one material's record, so animated records are never shared.
                let mut retain = |material: materials::PendingMaterial, shared: bool| {
                    if !shared {
                        materials.push(material);
                        return materials.len() - 1;
                    }
                    *material_ids.entry(material.key()).or_insert_with(|| {
                        let index = materials.len();
                        materials.push(material);
                        index
                    })
                };
                let mut make_material = |source: Option<&gltf::Material<'_>>,
                                         slot: Option<usize>|
                 -> Result<usize> {
                    let Some(name) = bindings.select(slot) else {
                        let material = if doc.is_some() {
                            let key=(asset_source.unwrap().0.clone(),source.and_then(|m|m.index()));
                            imported_materials[&key].clone()
                        } else {
                            materials::install_openpbr(&mut textures, input, None, base, &Default::default())?
                        };
                        return Ok(retain(material, true));
                    };
                    if let Some(&index) = native_ids.get(name) {
                        return Ok(index);
                    }
                    let index = if !stages[name].surface {
                        retain(materials::PendingMaterial::medium_boundary(), false)
                    } else if let Some(program) = bindings.programs.get(name) {
                        let programs = programs
                            .as_ref()
                            .context("material program was not compiled")?;
                        // Program images pack their table words into the input words.
                        let mut image_words = std::collections::BTreeMap::new();
                        for image in &program.images {
                            let (view, sampler) = textures.program_image(image, base)?;
                            image_words.insert(image.input.clone(), textures.slot(view, sampler));
                        }
                        let (id, words) = programs.pack(program, &image_words, input, base)
                            .with_context(|| format!("materials.{name}.inputs: program {}", program.name))?;
                        let authored = &input.materials[name];
                        let mut material = materials::PendingMaterial::program(
                            id,
                            authored.double_sided,
                        );
                        material.hair=programs.is_hair(program);
                        material.physical_hair=programs.is_physical_hair(program);
                        let capabilities = programs.capabilities(program);
                        material.response_program = if capabilities.anisotropy {crate::forward_variant::FEATURE_ANISOTROPY} else {0}
                            | if capabilities.film {crate::forward_variant::FEATURE_FILM} else {0}
                            | if capabilities.coat {materials::FEATURE_COAT} else {0}
                            | if capabilities.fuzz {materials::FEATURE_FUZZ} else {0}
                            | if capabilities.tint {materials::FEATURE_TINT} else {0};
                        if !programs.is_hair(program) && (capabilities.opacity || capabilities.transmission) {
                            material.integrate_opacity(authored.opacity, capabilities.transmission, programs.coverage_varies(program));
                        }
                        material.lobes = capabilities.lobes;
                        material.lit = capabilities.lit;
                        material.source_emission = capabilities.emission;
                        material.record.tag[3] = if animated_inputs.contains_key(name) {
                            let offset = material_words.len() as u32;
                            material_words.extend_from_slice(&words);
                            offset
                        } else {
                            *word_offsets.entry((id, words.clone())).or_insert_with(|| {
                                let offset = material_words.len() as u32;
                                material_words.extend_from_slice(&words);
                                offset
                            })
                        };
                        if animated_inputs.contains_key(name) {
                            program_word_lengths.insert(material.record.tag[3], words.len());
                        }
                        retain(material, !animated_inputs.contains_key(name) && !stages[name].medium)
                    } else {
                        let animated = animated_inputs.get(name);
                        let material = materials::install_openpbr(
                            &mut textures,
                            input,
                            Some(name),
                            base,
                            animated.unwrap_or(&Default::default()),
                        )?;
                        retain(material, animated.is_none() && !stages[name].medium)
                    };
                    native_ids.insert(name.to_owned(), index);
                    Ok(index)
                };
                let mut map = Vec::new();
                if let Some((_, slots)) = native_mesh {
                    for slot in 0..slots.len().max(1) {
                        map.push(make_material(None, Some(slot))?);
                    }
                } else {
                    let document=doc.as_ref().context("asset requires a glTF document")?;
                    let reachable=imported_material_slots(document)?;
                    for material in document.materials() {
                        // Keep glTF's document indices for the geometry uploader;
                        // unreachable slots are never read and need no installation.
                        map.push(if reachable.contains(&material.index()) {make_material(Some(&material),material.index())?} else {0});
                    }
                }
                let default = if native_mesh.is_some() {
                    map.first().copied().context("native mesh requires material slots")?
                } else if imported_material_slots(doc.as_ref().unwrap())?.contains(&None) {
                    make_material(None,None)?
                } else {0};
                material_ranges.insert(material_key, (map.clone(), default));
                (map, default)
            };
            // Bind geometry is immutable even for animated instances. Only
            // the asset binding, palettes and frame outputs are instance-private.
            let animated = asset_animated;
            let geometry_key = (
                key.clone(),
                animated,
                native_binding.as_ref().map(|binding| binding.joints.len()),
            );
            if let Some(range) = geometry_ranges.get(&geometry_key) {
                for i in range.clone() {
                    let source = &draws[i];
                    draws.push(Draw {
                        implicit: source.implicit.clone(),
                        sort_order: 0,
                        vertices: source.vertices.clone(),
                        attributes: source.attributes.clone(),
                        layout: source.layout,
                        uv_sets: source.uv_sets,
                        exit_geometry: source.exit_geometry.clone(),
                        exit_bvh: source.exit_bvh.clone(),
                        open_edges: source.open_edges.clone(),
                        indices: source.indices.clone(),
                        geometry_page: source.geometry_page,
                        vertex_offset: source.vertex_offset,
                        index_offset: source.index_offset,
                        count: source.count,
                        vertex_count: source.vertex_count,
                        ray_corners: source.ray_corners.clone(),
                        material: source
                            .material_slot
                            .map_or(default_material, |slot| material_map[slot]),
                        material_slot: source.material_slot,
                        object,
                        instances: object..object + 1,
                        sort_stride: 1,
                        local: source.local,
                        gltf_node: source.gltf_node.clone(),
                        gltf_path: source.gltf_path.clone(),
                        primitive_start: source.primitive_start,
                        center: source.center,
                        bounds: source.bounds,
                        generated_bounds: source.generated_bounds,
                        computed: source.computed,
                        displacement: None,
                        displacement_leader: None,
                        deform: source.deform.as_ref().map(|d| {
                            let mut d = d.clone();
                            d.asset = asset_id;
                            if let Some(binding) = &native_binding {
                                d.native = Some(std::sync::Arc::clone(binding));
                            }
                            d
                        }),
                    });
                }
                roots.push(MeshRoot {
                    node: node_id,
                    local: Mat4::IDENTITY,
                    time_offset: 0.,
                    instance_index: None,
                    imported_visibility: constants::RAY_VISIBILITY_ALL,
                    imported_pose: None,
                });
                continue;
            }
            let draw_start = draws.len();
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            let gltf_scene = doc
                .as_ref()
                .map(|doc| {
                    doc.default_scene()
                        .or_else(|| doc.scenes().next())
                        .context("glTF has no scene")
                })
                .transpose()?;
            let mut import = import::GeometryUpload {
                attribute_names: &attribute_names,
                evaluated_layout,
                stream_layout,
                asset: asset_id,
                animated: asset_animated,
                r,
                rays: geometry_rays,
                buffers: &buffers,
                draws: &mut draws,
                object,
                material_map: &material_map,
                materials: &materials,
                default: default_material,
                pages: &mut geometry_pages,
                placeholder: &placeholder,
                min: &mut min,
                max: &mut max,
                meshes: std::collections::HashMap::new(),
                path: Vec::new(),
            };
            if let Some(geometry) = geometry {
                let (name, slots) = native_mesh.expect("native geometry has a mesh node");
                let source = &input.geometry[name.as_str()];
                let deform = if let Some(binding) = &native_binding {
                    let skin = match &node.node.kind {
                        NodeKind::Mesh { skin, .. } => skin.as_ref(),
                        _ => None,
                    };
                    buffers::deformation(
                        r,
                        &buffer_data,
                        source,
                        skin,
                        geometry,
                        std::sync::Arc::clone(binding),
                    )?
                } else {
                    None
                };
                let groups = buffers::groups(source, geometry.indices.len(), slots.len().max(1))?;
                import.upload(
                    geometry,
                    deform,
                    default_material,
                    (0, String::new()),
                    Mat4::IDENTITY,
                )?;
                if import.draws.len() > draw_start {
                    let upload = import.draws.pop().unwrap();
                    for (range, slot) in groups {
                        if range.is_empty() {
                            continue;
                        }
                        let mut draw = upload.clone();
                        draw.index_offset += (range.start * std::mem::size_of::<u32>()) as u64;
                        draw.count = range.len() as u32;
                        draw.primitive_start = (range.start / 3) as u32;
                        draw.bounds = geometry.range_bounds(range);
                        draw.center = (draw.bounds.0 + draw.bounds.1) * 0.5;
                        draw.material = material_map[slot];
                        draw.material_slot = Some(slot);
                        import.draws.push(draw);
                    }
                }
            } else {
                for node in gltf_scene.iter().flat_map(|scene| scene.nodes()) {
                    import.visit(node, Mat4::IDENTITY)?;
                }
            }
            // Light- or camera-only glTF assets are valid and draw nothing.
            ensure!(
                draws.len() == draw_start || (min.is_finite() && max.is_finite()),
                "glTF geometry has no finite bounds"
            );
            geometry_ranges.insert(geometry_key, draw_start..draws.len());
            roots.push(MeshRoot {
                node: node_id,
                local: Mat4::IDENTITY,
                time_offset: 0.,
                instance_index: None,
                imported_visibility: constants::RAY_VISIBILITY_ALL,
                imported_pose: None,
            });
        }
        drop(decoded);
        buffers::instances(&nodes, &mut roots, &mut objects, &mut draws, &buffer_data)?;
        for draw in &mut draws {
            let node = &nodes[roots[draw.object as usize].node].node;
            if let NodeKind::Mesh { geometry, skin, .. } = &node.kind {
                draw.computed = crate::computed_geometry::dynamic(input, &input.geometry[geometry]);
                if draw.computed && (skin.is_some() || materials[draw.material].record.extra[1]>0. || materials[draw.material].transmissive) {
                    crate::warnings::dropped("computed_geometry", serde_json::json!({"node":node}), format!("warning: node {:?}: ignored computed geometry updates combined with skinning, opacity or transmission; using authored geometry",node.name));
                    draw.computed=false;
                }
            }
        }
        // Segmentation IDs are authored metadata, independent of transform slots.
        let mut segmentation = buffers::SegmentationIds::new(input, &buffer_data)?;
        let source_roots = std::mem::take(&mut roots);
        let source_objects = std::mem::take(&mut objects);
        for draw in &mut draws {
            let node = source_roots[draw.object as usize].node;
            let imported_visibility = match &nodes[node].node.kind {
                NodeKind::Asset {
                    nodes: overrides, ..
                } => buffers::imported_visibility_mask(overrides, &draw.gltf_path),
                _ => constants::RAY_VISIBILITY_ALL,
            };
            let object_groups = links.object_groups(node, &nodes[node].node, &draw.gltf_path)?;
            let draw_id = segmentation.prepare(node, &draw.gltf_path, &nodes, &buffer_data)?;
            let first = objects.len() as u32;
            for original in draw.instances.clone() {
                let mut root = source_roots[original as usize].clone();
                let id = draw_id.instance(root.instance_index)?;
                root.imported_visibility = imported_visibility;
                root.imported_pose = draw.deform.as_ref().and_then(|d| d.asset.map(|asset| (asset,d.node)));
                root.local *= draw.local;
                let world = nodes[node].world * root.local;
                let mut value = source_objects[original as usize];
                value.model = world;
                value.previous_model = world;
                value.next_model = world;
                value.opacity[1] = id as f32;
                value.metadata[3] = root.instance_index.unwrap_or(0);
                value.metadata[0] = object_groups;
                value.metadata[1] =
                    (if buffers::node_collapsed(&nodes, node) {0} else {buffers::node_visibility_mask(&nodes, node)}) & root.imported_visibility | buffers::node_matte_flags(&nodes,node);
                objects.push(value);
                roots.push(root);
            }
            draw.object = first;
            draw.instances = first..objects.len() as u32;
        }

        let object_ids = segmentation.metadata();
        for (page, data) in geometry_pages.into_iter().enumerate() {
            if data.vertices.is_empty() {
                continue;
            }
            let attributes = init(
                &r.device,
                "scene static attributes",
                &data.attributes,
                // Ray hits read attribute streams in place (rt::hit_record).
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
            );
            let vertices = init(
                &r.device,
                "scene static vertices",
                &data.vertices,
                wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::STORAGE
                    | crate::rt::geometry_usage(geometry_rays),
            );
            let indices = init(
                &r.device,
                "scene static indices",
                bytemuck::cast_slice(&data.indices),
                wgpu::BufferUsages::INDEX
                    | wgpu::BufferUsages::STORAGE
                    | crate::rt::geometry_usage(geometry_rays),
            );
            for draw in draws
                .iter_mut()
                .filter(|d| d.geometry_page == Some(page))
            {
                draw.vertices = vertices.clone();
                draw.attributes = attributes.clone();
                draw.indices = indices.clone();
            }
        }
        for (index, node) in embedded.iter().enumerate() {
            ensure!(
                node_names
                    .insert(node.node.name.clone(), nodes.len() + index)
                    .is_none(),
                "imported camera/light name {:?} is ambiguous",
                node.node.name
            );
        }
        // Imported asset cameras remain selectable, but do not make the authored
        // scene camera ambiguous. Authored nodes precede imports for default selection.
        let authored_camera_count = nodes
            .iter()
            .filter(|n| matches!(n.node.kind, NodeKind::Camera { .. }))
            .count();
        for node in &mut embedded {
            if let Some((asset, _, _, _)) = node.embedded { node.dynamic |= assets[asset].animated(); }
        }
        nodes.extend(embedded);
        if !imported_camera_materials.is_empty() {
            let imported: vork_scene::Scene=serde_json::from_value(serde_json::json!({"version":2,"nodes":[],"materials":imported_camera_materials,"render":{"resolution":render.resolution}}))?;
            let camera_views=crate::camera_program::views(input,&nodes,&animation,options.initial_time);
            let mut added=crate::camera_program::install(r,&imported,&mut nodes,base,&camera_views)?;
            for camera in &mut added {camera.id+=camera_programs.len() as u32;camera.offset=material_words.len();material_words.extend_from_slice(&camera.words);}
            camera_programs.extend(added);
            slang.camera=Some(std::sync::Arc::new(crate::camera_program::native(&camera_programs)));
        }
        // Resolve only the omitted start camera, after buffers and clips are compiled.
        // Use the same ordered layer sampling as every rendered frame.
        if render.camera.is_none() {
            render.camera = animation.start_camera(&render, options.initial_time);
        }
        let camera = nodes
            .iter()
            .position(|n| {
                matches!(n.node.kind, NodeKind::Camera { .. })
                    && render
                        .camera
                        .as_deref()
                        .is_none_or(|name| n.node.name == name)
            })
            .context("install requires a camera matching render.camera")?;
        ensure!(
            render.camera.is_some() || authored_camera_count <= 1,
            "a scene with {authored_camera_count} authored cameras requires render.camera"
        );
        // Instances with a time offset phase animated materials as they phase motion:
        // each (record, offset) pair gets its own record copy, written at time + offset.
        let material_inputs = animation.material_inputs();
        let animated: std::collections::BTreeSet<usize> = material_inputs
            .iter()
            .filter_map(|(material, _)| native_ids.get(material).copied())
            .collect();
        let mut phased = std::collections::BTreeMap::<(usize, u32), usize>::new();
        for draw in draws.iter().filter(|d| animated.contains(&d.material)) {
            for object in draw.instances.clone() {
                let offset = roots[object as usize].time_offset;
                if offset != 0. {
                    phased
                        .entry((draw.material, offset.to_bits()))
                        .or_insert_with(|| {
                            let mut material = materials[draw.material].clone();
                            if material.record.tag[0] != 0 {
                                // Resource descriptors follow the reflected prefix and use
                                // relative addresses: phase the complete packed block.
                                let width = program_word_lengths[&material.record.tag[3]];
                                let start = material.record.tag[3] as usize;
                                let words = material_words[start..start + width].to_vec();
                                material.record.tag[3] = material_words.len() as u32;
                                program_word_lengths.insert(material.record.tag[3], width);
                                material_words.extend(words);
                            }
                            materials.push(material);
                            materials.len() - 1
                        });
                }
            }
        }
        // Material tracks write the record of their material, wherever it is used.
        {
            let records: Vec<_> = materials.iter().map(|m| m.record).collect();
            let records: &[u8] = bytemuck::cast_slice(&records);
            for target in material_inputs {
                if input.materials[&target.0].program != "std/openpbr" { continue; }
                let mut slots = vec![];
                if let Some(&slot) = native_ids.get(&target.0) {
                    slots.push((slot, 0.));
                    slots.extend(
                        phased
                            .iter()
                            .filter(|((s, _), _)| *s == slot)
                            .map(|(&(_, bits), &copy)| (copy, f32::from_bits(bits))),
                    );
                }
                let offsets: std::collections::HashMap<_, _> = slots.iter().copied().collect();
                let slots: Vec<usize> = slots.iter().map(|s| s.0).collect();
                let bindings = materials::track_targets((&target.0, &target.1), &slots, records)?
                    .into_iter()
                    .map(|(slot, parameter, value)| animation::MaterialBinding {
                        slot,
                        parameter,
                        storage: None, enum_tokens: None,
                        default: animation::Sample::Numeric(value),
                        offset: offsets[&slot],
                    })
                    .collect();
                animation.bind_material(&target, bindings)?;
            }
        }
        // Non-OpenPBR consumers resolve words after the table's final placement.
        for target in animation.material_inputs() {
            if input.materials[&target.0].program != "std/openpbr" {
                animation.bind_material(&target, vec![])?;
            }
        }
        animation.mark_nodes(&mut nodes, camera)?;
        let targets = crate::targets::Plan::compile(&mut nodes)?;
        animation.validate_asset_bindings()?;
        let mut samples = animation.samples();
        let mut values = crate::values::sample_storage(&nodes);
        let mut worlds = nodes.iter().map(|node| node.world).collect::<Vec<_>>();
        // Reference sampling coordinates use the canonical pose, including
        // camera targets, rather than the still call's seek time.
        let (probe_frame,probe_eyes) = if automatic_probes && render.settings.integrator.probe_volume().is_some() {
            let mut canonical_samples=animation.samples();
            let mut canonical_values=crate::values::sample_storage(&nodes);
            let mut canonical_worlds=worlds.clone();
            crate::values::sample_pose(&nodes,&assets,&animation,&targets,&environment,0.,
                &mut canonical_samples,&mut canonical_values,&mut canonical_worlds)?;
            let canonical_objects=objects.iter().zip(&roots).map(|(object,root)| {
                let mut object=*object;object.model=root.world(&canonical_worlds);object
            }).collect::<Vec<_>>();
            let eyes=nodes.iter().enumerate().filter(|(_,n)|matches!(n.node.kind,NodeKind::Camera {..}))
                .map(|(i,_)|canonical_worlds[i].w_axis.truncate()).collect::<Vec<_>>();
            (Some(crate::probes::placement::Frame::for_camera(&nodes[camera],canonical_worlds[camera],&draws,&canonical_objects,&render)),Some(eyes))
        } else {(None,None)};
        crate::values::sample_pose(&nodes, &assets, &animation, &targets, &environment, options.initial_time,
            &mut samples, &mut values, &mut worlds)?;
        for (node, world) in nodes.iter_mut().zip(&worlds) { node.world = *world; }
        // Object tables predate animation compilation; downstream bounds and probe
        // fitting must see the same evaluated placement as node/camera initialization.
        for (root, object) in roots.iter().zip(&mut objects) {
            let world = root.world(&worlds);
            object.model = world;
            object.previous_model = world;
            object.next_model = world;
        }
        // Asset contents were unknown during reservation. Opaque imports, and
        // imports without shadowed lights, retain the original shadow
        // representation after material decoding.
        let resolved_shadow_rays=crate::rt::installed_shadow_rays(&scene,&nodes);
        let shadow_changed=resolved_shadow_rays!=light_shadow_rays;
        light_shadow_rays=resolved_shadow_rays;
        let connection_changed=transmitting_casters && (!nodes.iter().any(|node| shadowed_light(&node.node.kind)) || !draws.iter().any(|draw| {
            materials[draw.material].transmissive && draw.instances.clone().any(|object| {
                let id=roots[object as usize].node;
                buffers::node_visibility_mask(&nodes,id)&constants::RAY_SHADOW!=0
                    || crate::plan::shadow_visibility_animated(&nodes,id,&animation)
            })
        }));
        if connection_changed {
            promoted_transport=spatial_subsurface;
            if let Some(medium)=&mut medium {
                medium.native.ray_shadows=light_shadow_rays || promoted_transport;
                medium.native.program_coverage &= medium.native.ray_shadows;
                medium.native.identity.push(u8::from(medium.native.ray_shadows));
                slang.medium=Some(std::sync::Arc::new(medium.native.clone()));
            }
            for view in std::iter::once(&mut render).chain(views.iter_mut()) {
                if let Some(raster)=view.settings.integrator.raster_mut() {
                    raster.shadow_method=scene.render.settings.integrator.shadow_method();
                }
            }
        }
        if shadow_changed && !connection_changed {
            if let Some(medium)=&mut medium {
                let required=light_shadow_rays || promoted_transport;
                if required!=medium.native.ray_shadows {
                    medium.native.ray_shadows=required;
                    medium.native.program_coverage &= required;
                    if required {medium.native.link_shadow_programs(programs.as_ref().and_then(|p|p.native.as_deref()),connection_transmission);}
                    medium.native.identity.push(u8::from(required));
                    slang.medium=Some(std::sync::Arc::new(medium.native.clone()));
                }
            }
        }
        let limits = r.device.limits();
        // Captures share this installed light_plan. Media also consumes map transport.
        // The profile resolution is a quality floor. Large authored views and
        // captures need correspondingly sized maps, bounded by the actual GPU.
        let fibre_view_side=std::iter::once(render.resolution).chain(input.images.values().filter_map(|i|i.render.as_ref()).map(|view|view.resolve(&render).resolution)).flatten().max().unwrap_or(1);
        let mut light_plan = LightPlan::logical(&nodes, &limits, medium.is_some())?;
        if let Some(medium) = &mut medium {
            // Keep animated/imported reservations: an empty installed plan is
            // the proof, not the current frame's zero light count or energy.
            medium.native.analytic_lights_absent=light_plan.nodes.is_empty();
            medium.native.identity.push(u8::from(medium.native.analytic_lights_absent));
            medium.native.visibility_lights=if medium.native.scattering_free {0} else {u32::try_from(light_plan.nodes.iter().filter(|&&id| {
                let mut ancestor=Some(id);
                while let Some(i)=ancestor {
                    if nodes[i].dynamic {return true;}
                    ancestor=nodes[i].parent;
                }
                buffers::node_visibility_mask(&nodes,id)&constants::RAY_VOLUME!=0
            }).count())?};
            #[allow(unused_mut)]
            let mut limits = r.device.limits();
            #[cfg(test)]
            if let Some(bytes) = r.volume_storage_limit {limits.max_storage_buffer_binding_size = limits.max_storage_buffer_binding_size.min(bytes);}
            medium.native.configure_storage(input, &limits)?;
            slang.medium=Some(std::sync::Arc::new(medium.native.clone()));
        }
        light_plan.blackbody = crate::light_temperature::table(r, &nodes)?;
        ensure!(r.device.features().contains(wgpu::Features::BUFFER_BINDING_ARRAY),
            "shadow depth-bounds tables require buffer binding arrays on this backend");
        // Thin and solid interfaces use the same scene continuation. Refine
        // transport against installed materials, independently of solid occupancy.
        let transport_rays = ray_refraction && draws.iter().any(|draw| materials[draw.material].transmissive);
        let refraction_changed = transport_rays != ray_refraction;
        if refraction_changed || connection_changed || shadow_changed {
            ray_refraction = transport_rays;
            ray_shadows = light_shadow_rays || ray_refraction || effects.probes || promoted_transport;
            rays = crate::rt::installed_rays(&scene, ray_refraction) || light_shadow_rays || promoted_transport;
            slang.ray_refraction = ray_refraction;
        }
        // Refine imported reservations against the same decoded material table
        // used by the graph. Keep the preflight conservative only until here.
        // Mesh emission is a medium light source even when no other effect
        // requested scene rays. Reuse the installed source and hit tables.
        let medium_sources=medium.as_ref().is_some_and(|m|!m.native.scattering_free) && draws.iter().any(|draw| draw.implicit.is_none() && draw.count>0
            && materials[draw.material].physical_emitter() && draw.instances.clone().any(|object| {
                let root=&roots[object as usize];let node=root.node;
                let visible_track=std::iter::successors(Some(node),|&id|nodes[id].parent).any(|id|animation.targets().any(|target|matches!(target,
                    vork_scene::TrackTarget::Node {node,property:vork_scene::NodeProperty::Visibility(vork_scene::Ray::All)} if *node==nodes[id].node.name)));
                let imported_track=root.imported_pose.is_some() && animation.targets().any(|target|matches!(target,
                    vork_scene::TrackTarget::Imported {asset,property:vork_scene::NodeProperty::Visibility(vork_scene::Ray::All),..} if *asset==nodes[node].node.name));
                (buffers::node_visibility_mask(&nodes,node)&constants::RAY_VOLUME!=0 || visible_track)
                    && (root.imported_visibility&constants::RAY_VOLUME!=0 || imported_track)
                    && (objects[object as usize].metadata[1]&constants::RAY_VOLUME!=0
                        || visible_track || imported_track || nodes[node].dynamic)
            }));
        rays |= medium_sources;
        ray_shadows |= medium_sources;
        // This is the lowered/compiled proof, including imported and program alpha.
        // Resolve it before choosing the shared opaque pipeline registry.
        slang.surface_coverage = draws.iter().any(|draw|materials[draw.material].surface_coverage());
        slang.connection_transmission = rays && (draws.iter().any(|draw|materials[draw.material].transmissive) || input.geometry.values().any(|g|matches!(g.topology,vork_scene::v2::Topology::Implicit)));
        // Field carriers are absent from the triangle ray masks. Their exact
        // crossing walk owns absorption; triangle containment cannot find them.
        slang.no_shadow_containment=!draws.iter().any(|draw| {
            if !materials[draw.material].solid_absorption() {return false;}
            let node=&nodes[roots[draw.object as usize].node].node;
            !matches!(&node.kind,NodeKind::Mesh {geometry,..} if matches!(input.geometry[geometry].topology,vork_scene::v2::Topology::Implicit))
        });
        // Reuse the proven overlap capacity before launching shader compilation.
        // Compute placement is not installed yet; reserve all domains when it
        // can change placements. Ordinary animation/deformation is already known.
        let shadow_solids=if rays {crate::passes::path::solids_for(&draws,&roots,|m|materials[m].solid_transmission())?} else {Default::default()};
        // Retain the closure proof before compiling capacity-specialized shadows.
        // Shared-index deformation preserves closure; welded attribute seams
        // without that proof remain unknown, since deformation may open them.
        let mut welds=std::collections::HashMap::new();
        let mut open_edges=std::collections::HashMap::new();
        for draw in &mut draws {
            if !draw.instances.clone().any(|object|shadow_solids.owners.get(object as usize).is_some_and(|&solid|solid!=u32::MAX)) {continue;}
            if programs.as_ref().is_some_and(|p|p.displaces(materials[draw.material].record.tag[0])) {continue;}
            let Some(geometry)=&draw.exit_geometry else {continue;};
            let start=draw.primitive_start as usize*3;let end=start+draw.count as usize;
            let address=std::sync::Arc::as_ptr(geometry);
            let keys=welds.entry(address).or_insert_with(||if draw.deform.is_some() {
                let identity=address as usize as u64;
                (0..geometry.vertices.len()).map(|i|[identity as u32,(identity>>32) as u32,i as u32]).collect()
            } else {crate::passes::path::weld_keys(geometry)});
            draw.open_edges=Some(open_edges.entry((address,start,end)).or_insert_with(||std::sync::Arc::new(crate::passes::path::open_edges(&geometry.indices[start..end],|i|keys[i as usize]))).clone());
        }
        drop(welds);drop(open_edges);
        // Only absorbing domains enter containment. Clear transmitting solids
        // still need the closure proof above for camera occupancy, but must not
        // reserve shadow records (especially with unrelated compute placement).
        let contained=if shadow_solids.count==0 {Default::default()}
            else {crate::passes::path::solids_for(&draws,&roots,|m|materials[m].solid_absorption())?};
        slang.shadow_capacity=if nodes.iter().any(|n|crate::compute::places_instances(input,&n.node)) {contained.count}
            else {crate::passes::path::occupancy(&contained,&draws,&roots,&nodes,&objects,None,&animation,&targets).1};
        let program_coverage = medium_sources || slang.connection_transmission
            || programs.as_ref().and_then(|p|p.native.as_ref()).is_some_and(|p|p.coverage)
            || (effects.fibre && rays);
        if let Some(medium)=&mut medium {
            medium.native.program_coverage &= program_coverage;
            medium.native.domains=medium.domain_count(&draws,&native_ids);
            medium.native.mesh_sources=medium_sources;
            medium.native.shadow_capacity=slang.shadow_capacity;
            if medium_sources {
                medium.native.ray_shadows=true;
                if !medium.native.program_coverage {medium.native.link_shadow_programs(programs.as_ref().and_then(|p|p.native.as_deref()),true);}
                medium.native.identity.push(1);
            }
            slang.medium=Some(std::sync::Arc::new(medium.native.clone()));
        }
        // Imported assets reserve possible light connections before their contents
        // are known. Enforce transport requirements only after that reservation
        // has been refined against the actual lights and material draws.
        ensure!(!light_shadow_rays || r.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
            "effect unavailable: light shadows need hardware ray queries; use a ray-query capable adapter");
        if promoted_transport {
            ensure!(r.device.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY | crate::rt::HIT_BUFFER_FEATURES),
                "surface transport needs mesh-thickness or RGB light-connection ray queries and hit buffers; use a GPU supporting ray queries");

        }
        ensure!(!rays || !program_coverage || r.device.features().contains(crate::rt::HIT_BUFFER_FEATURES), "ray material visibility requires storage buffer binding arrays");
        (slang.textures,slang.samplers)=textures.counts()?;
        let descriptor_elements=u64::from(slang.textures)+u64::from(slang.samplers)
            +u64::from(crate::passes::decoupled::evaluation_pages(&limits,slang.aov_count))
            +u64::from(medium.as_ref().map_or(1,|m|m.native.sample_pages.max(1)))
            +u64::from(constants::VISIBILITY_REFLECTION_WORDS)+u64::from(constants::RAY_HIT_BUFFERS)+crate::camera_native::TABLES.iter().map(|t|t.resources.len() as u64).sum::<u64>();
        ensure!(slang.samplers <= limits.max_binding_array_sampler_elements_per_shader_stage,
            "scene needs {} material samplers; device max_binding_array_sampler_elements_per_shader_stage={}",slang.samplers,limits.max_binding_array_sampler_elements_per_shader_stage);
        ensure!(descriptor_elements <= u64::from(limits.max_binding_array_elements_per_shader_stage),
            "scene material/ray tables need {descriptor_elements} descriptors; device max_binding_array_elements_per_shader_stage={}", limits.max_binding_array_elements_per_shader_stage);
        let builtin=propagation.is_none() && !spectral && camera_programs.is_empty() && slang.environment.is_none() && programs.is_none() && medium.is_none()
            && !ray_shadows && !slang.ray_refraction && !slang.connection_transmission && !slang.surface_coverage && !effects.denoise && !effects.probes && render.aovs.is_empty()
            && slang.attributes==0 && slang.textures==vork_slang::registry::MATERIAL_TEXTURE_CAPACITY && slang.samplers==vork_slang::registry::MATERIAL_SAMPLER_CAPACITY;
        let pipelines = if builtin {r.cached_pipelines(raster_samples)} else {r.program_pipelines(programs.as_ref().map_or("", |p| p.identity.as_str()),
            medium.as_ref().filter(|_|filtered_media).map_or(RESOLVE, |m| m.shader.as_str()), medium.as_ref().filter(|_|filtered_media).map(|m| &m.native),
            raster_samples, ray_shadows, slang.clone())?};
        let effects_compiler=Some({let pipelines=pipelines.clone(); scope.spawn(move ||pipelines.ensure_effects(r,effects))});
        let mat_layout=pipelines.mat_layout.clone();
        let cached = &*pipelines;
        for d in &draws {
            if let Some(a) = &d.deform {
                if a.native.is_none() {
                    assets[a.asset.expect("glTF deformation has an asset")].active = true;
                }
            }
        }
        let material_bind_start = Instant::now();
        // One immutable storage array in the scene material table.
        // Keep a sentinel for scenes without geometry (zero-sized bindings are invalid).
        // A material's taps may read only texture coordinate sets its geometry has.
        for draw in &draws {
            let missing = materials[draw.material].uv_sets() & !draw.uv_sets;
            ensure!(
                missing == 0,
                "a material texture reads TEXCOORD_{} (uv{}), which its geometry lacks",
                materials[draw.material].uv_sources[missing.trailing_zeros() as usize],
                materials[draw.material].uv_sources[missing.trailing_zeros() as usize]
            );
        }
        // Texture taps follow the program words; identical tap lists share words.
        let mut tap_offsets = std::collections::HashMap::<Vec<u32>, u32>::new();
        let mut records = Vec::with_capacity(materials.len().max(1));
        for material in &materials {
            let mut record = material.record;
            if !material.taps.is_empty() {
                let words: Vec<u32> = material.taps.iter().flatten().copied().collect();
                record.tag[1] = *tap_offsets.entry(words).or_insert_with_key(|words| {
                    let offset = material_words.len() as u32;
                    material_words.extend_from_slice(words);
                    offset
                });
            }
            records.push(record);
        }
        if records.is_empty() {
            records.push(materials::MaterialRecord::zeroed());
        }
        let record_words =
            (records.len() * std::mem::size_of::<materials::MaterialRecord>() / 4) as u32;
        for camera in &mut camera_programs {
            let start=camera.offset;
            programs::bind_inputs(&mut animation,&camera.material,&camera.program.inputs,&mut material_words[start..start+camera.words.len()],record_words as usize+start,0.)?;
            if let Some(state)=&mut nodes[camera.node].camera {state.offset=record_words as usize+start;state.words.clone_from_slice(&material_words[start..start+camera.words.len()]);}
            camera.offset+=record_words as usize;
        }
        if let Some(primary)=&environment.primary {
            ensure!(records.len()==1 && materials.is_empty(),"spatial environments require an empty geometry table");
            programs::bind_inputs(&mut animation,&primary.material,&primary.layout,
                &mut material_words[..primary.words.len()],record_words as usize,0.)?;
        }
        for object in &mut objects {
            if object.material_index[2] != 0 { object.material_index[1] += record_words; }
        }
        for (record, material) in records.iter_mut().zip(&materials) {
            if record.tag[0] != 0 {
                record.tag[3] += record_words;
            }
            if !material.taps.is_empty() {
                record.tag[1] += record_words;
            }
        }
        if let Some(medium) = &mut medium {
            medium.mesh_words(r, input, &draws, &roots, &native_ids).with_context(|| {
                let names=stages.iter().filter(|(_,stage)|stage.medium).map(|(name,_)|name.as_str())
                    .chain(environment.settings.medium.as_deref()).collect();
                format!("medium domains: {}",medium_use_sites(&scene.nodes,&names).join(", "))
            })?;
        }
        // Only lights explicitly consuming an IES asset retain source-knot data.
        // The material-word table is shared by every radiance consumer.
        let mut emitter_profiles=std::collections::HashMap::new();
        let mut profile_assets=std::collections::HashMap::<String,u32>::new();
        for (id,node) in nodes.iter().enumerate() {
            if let NodeKind::Light { shape, image: Some(name), .. } = &node.node.kind {
                if matches!(shape,vork_scene::LightShape::Environment) {continue;}
                let address=if let Some(address)=profile_assets.get(name) {*address} else {
                    let image=input.images.get(name).context("missing emitter profile image")?;
                    let file=image.file.as_ref().context("local emission profiles require an IES file")?;
                    let path=base.join(file);
                    let profile=crate::ies::parse(&std::fs::read(&path).with_context(||format!("read emission profile {}",path.display()))?)?;
                    let address=record_words+u32::try_from(material_words.len())?;
                    material_words.extend(profile.words());
                    profile_assets.insert(name.clone(),address);address
                };
                emitter_profiles.insert(id,address);
            }
        }
        if !emitter_profiles.is_empty() {
            ensure!(effects.path, "local IES emission currently requires path beauty consumers; raster sampled-emitter integration is not installed yet");
        }
        let medium_base = record_words + material_words.len() as u32;
        if let Some(medium) = &medium {
            material_words.extend_from_slice(&medium.words);
        }
        // Generated bounds are geometry data, shared across all instances. The
        // existing reserved object word addresses them in the retained table.
        let mut generated_offsets = std::collections::HashMap::new();
        let mut instance_attribute_rows = std::collections::HashMap::new();
        for (draw_index, draw) in draws.iter().enumerate() {
            let (minimum, maximum) = draw.generated_bounds;
            let extent = maximum - minimum;
            let (lo, hi) = draw.bounds;
            // Deformation may leave the installed bounds; keep those instances eligible.
            let bounded = !draw.evaluated() && lo.is_finite() && hi.is_finite();
            let words = [minimum.x, minimum.y, minimum.z, extent.x, extent.y, extent.z,
                lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, if bounded { 1. } else { 0. }].map(f32::to_bits);
            let provenance = programs.as_ref().is_some_and(|p|p.native.is_some()).then_some(draw.exit_geometry.as_ref()).flatten();
            let identity=provenance.map(|geometry|(std::sync::Arc::as_ptr(geometry) as usize,draw.primitive_start,draw.count));
            for object in draw.instances.clone() {
                let root = &roots[object as usize];
                let node = &nodes[root.node].node;
                let mut row = vec![0u32; 1 + attribute_names.len() * 4];
                for (slot, name) in attribute_names.iter().enumerate() {
                    if let Some(value) = crate::attributes::instance_value(node, root.instance_index, name, &buffer_data) {
                        row[0] |= 1 << slot;
                        row[1+slot*4..1+(slot+1)*4].copy_from_slice(&value.map(f32::to_bits));
                    }
                }
                let attribute_row = if row[0] == 0 { 0 } else {
                    *instance_attribute_rows.entry(row).or_insert_with_key(|row| {
                        let offset = record_words + material_words.len() as u32;
                        material_words.extend_from_slice(row);
                        offset
                    })
                };
                let offset = *generated_offsets.entry((words, identity, attribute_row,
                    draw.deform.as_ref().map(|_|draw_index),
                    programs.as_ref().is_some_and(|p|p.displaces(records[draw.material].tag[0]))
                        .then_some((draw.material,root.time_offset.to_bits())))).or_insert_with(|| {
                    let offset = record_words + material_words.len() as u32;
                    material_words.extend_from_slice(&words);
                    material_words.push(attribute_row);
                    if let Some(geometry)=provenance {
                        let first=draw.primitive_start as usize;
                        material_words.extend_from_slice(&geometry.primitive_ids[first..first+draw.count as usize/3]);
                    }
                    offset
                });
                objects[object as usize].metadata[2] = offset;
            }
        }
        for name in animation.material_inputs().into_iter().map(|(name, _)| name).collect::<std::collections::BTreeSet<_>>() {
            if input.materials[&name].program == "std/openpbr" { continue; }
            if let Some(&slot) = native_ids.get(&name) {
                for (slot, offset) in std::iter::once((slot, 0.)).chain(phased.iter().filter(|((s, _), _)| *s == slot).map(|(&(_, bits), &copy)| (copy, f32::from_bits(bits)))) {
                    let record = records[slot];
                    if record.tag[0] != 0 {
                        let layout = programs.as_ref().unwrap().input_layout(record.tag[0]);
                        let relative = record.tag[3] as usize - record_words as usize;
                        programs::bind_inputs(&mut animation, &name, layout,
                            &mut material_words[relative..relative + layout.words], record.tag[3] as usize, offset)?;
                    }
                }
            }
        }
        if let Some(medium) = &medium {
            medium.bind_inputs(&mut animation, &mut material_words[(medium_base - record_words) as usize..], medium_base as usize)?;
        }
        let animation = std::sync::Arc::new(animation);
        for asset in &mut assets {
            asset.attach_plan(std::sync::Arc::clone(&animation));
        }
        links.finish(&mut material_words, record_words, &mut objects)?;
        let mut record_bytes = bytemuck::cast_slice::<_, u8>(&records).to_vec();
        record_bytes.extend_from_slice(bytemuck::cast_slice(&material_words));
        // Surface/compute consumers sample the exposure clock. Main final-only
        // compositor images sample the nominal output clock after accumulation.
        let mut sampled_media:std::collections::BTreeSet<_>=textures.footage.keys().cloned().collect();
        for image in input.images.values().filter_map(|i|i.render.as_ref()) {
            sampled_media.extend(input.compositor_image_reads(&image.compositor));
        }
        let final_media=input.compositor_image_reads(&input.render.compositor);
        let mut file_images = std::collections::BTreeMap::new();
        // Compositor image handles reuse the exact scene texture installation.
        let post_materials: std::collections::BTreeSet<_> = input.render.compositor.iter()
            .chain(input.images.values().filter_map(|i|i.render.as_ref()).flat_map(|r|r.compositor.iter())).map(|entry|entry.material()).collect();
        for name in post_materials {
            let material=&input.materials[name];
            for value in material.inputs.values() {
                if let vork_scene::v2::Input::Texture(binding) = value {
                    let image = &input.images[&binding.image];
                    if image.render.is_none() && !file_images.contains_key(&binding.image) {
                        let (slot, _) = textures.scene_image(&binding.image,image,base)?;
                        if let materials::TextureSlot::File(view) = slot {file_images.insert(binding.image.clone(),view);}
                    }
                }
            }
        }
        for (name,image) in &mut textures.footage {
            image.per_sample=sampled_media.contains(name);
            image.final_compositor=final_media.contains(name);
        }
        let material_table = textures.table(&mat_layout, record_bytes)?;
        let mut materials: Vec<_> = materials
            .into_iter()
            .map(materials::PendingMaterial::finish)
            .collect();
        if rays {
            crate::rt::prepare_coverage(&mut draws, &materials)?;
        }
        let displacement = if programs.as_ref().is_some_and(|p| p.has_displacement()) {
            let mut displacement_seam_nodes = std::collections::HashSet::new();
            let mut assignment_groups = std::collections::HashMap::<_, Vec<&Draw>>::new();
            for draw in &draws {
                let root = &roots[draw.object as usize];
                assignment_groups.entry((root.node, root.instance_index, draw.gltf_node.0))
                    .or_default().push(draw);
            }
            let displaces = |draw: &Draw| programs.as_ref().unwrap()
                .displaces(records[draw.material].tag[0]);
            for group in assignment_groups.values() {
                // Single-material domains and surface-only differences cannot conflict.
                // Avoid per-vertex storage for those common cases.
                if group.iter().all(|draw| draw.material == group[0].material)
                    || !group.iter().any(|draw| displaces(draw)) { continue; }
                let mut assignments = std::collections::HashMap::new();
                // Record displaced assignments first, then check rigid siblings.
                for displaced_pass in [true, false] {
                    for draw in group {
                        if displaces(draw) != displaced_pass { continue; }
                        let Some(geometry) = draw.exit_geometry.as_ref() else { continue; };
                        // Source POSITION accessor/index identities survive imported
                        // normal/attribute splits. Only referenced vertices participate.
                        let first = draw.primitive_start as usize * 3;
                        for &vertex in &geometry.indices[first..first + draw.count as usize] {
                            let identity = geometry.vertex_ids.get(vertex as usize)
                                .copied().unwrap_or(u64::from(vertex));
                            if let Some(&material) = assignments.get(&identity) {
                                if material!=draw.material && displacement_seam_nodes.insert(roots[draw.object as usize].node) {
                                    crate::warnings::dropped("conflicting_displacement", serde_json::json!({"node":nodes[roots[draw.object as usize].node].node}), format!("warning: node {:?}: ignored conflicting displacement materials on shared geometry",nodes[roots[draw.object as usize].node].node.name));
                                }
                            }
                            if displaced_pass { assignments.insert(identity, draw.material); }
                        }
                    }
                }
            }
            let mut displacement_seams = std::collections::HashMap::<_, Vec<u32>>::new();
            for draw in &draws {
                let record = &records[draw.material];
                let Some(programs) = programs.as_ref().filter(|p| p.displaces(record.tag[0])) else {
                    continue;
                };
                let node=&nodes[roots[draw.object as usize].node].node;
                let implicit=matches!(&node.kind,NodeKind::Mesh {geometry,..} if matches!(input.geometry[geometry].topology,vork_scene::v2::Topology::Implicit));
                if draw.computed || implicit { continue; }
                let root = &roots[draw.object as usize];
                let descriptors = programs.displacement_descriptors(
                    record.tag[0],
                    (record.tag[3] - record_words) as usize,
                    &material_words,
                );
                let geometry = draw
                    .exit_geometry
                    .as_ref()
                    .context("displacement requires triangle geometry")?;
                if !geometry.point_centers.is_empty() {
                    if programs.displacement_uses_normal(record.tag[0]) { continue; }
                }
                // With no vertex-varying inputs, every seam comparison is the
                // same empty tuple, independent of instance identity or time.
                if descriptors.is_empty() && !programs.displacement_uses_normal(record.tag[0]) {continue;}
                let first = draw.primitive_start as usize * 3;
                for &index in &geometry.indices[first..first + draw.count as usize] {
                    let vertex = index as usize;
                    let identity = geometry
                        .vertex_ids
                        .get(vertex)
                        .copied()
                        .unwrap_or(vertex as u64);
                    let mut values = Vec::new();
                    if programs.displacement_uses_normal(record.tag[0]) {
                        values.extend(geometry.vertices[vertex].NORMAL.map(f32::to_bits));
                    }
                    for (width, descriptor) in &descriptors {
                        let slot = (descriptor[0] & 255) as usize;
                        let channel = descriptor[0] >> 8;
                        let instance = crate::attributes::instance_value(&nodes[root.node].node,
                            root.instance_index, &attribute_names[slot], &buffer_data);
                        let attribute = instance.as_ref().or_else(|| geometry.attributes
                            .get(&attribute_names[slot]).map(|values| &values[vertex]))
                            .context("bound displacement attribute missing")?;
                        for component in 0..*width as usize {
                            let source = if channel < 4 {
                                channel as usize
                            } else {
                                component
                            };
                            values.push(
                                (attribute[source] * f32::from_bits(descriptor[component + 1]))
                                    .to_bits(),
                            );
                        }
                    }
                    let key = (root.node, root.instance_index, draw.gltf_node.0, identity);
                    if let Some(before) = displacement_seams.insert(key, values.clone()) {
                        if before!=values && displacement_seam_nodes.insert(root.node) {
                            crate::warnings::dropped("displacement_attributes", serde_json::json!({"node":nodes[root.node].node,"vertex":identity}), format!("warning: node {:?}: ignored displacement across inconsistent attributes at shared vertex {identity}",nodes[root.node].node.name));
                        }
                    }
                }
            }
            let mut displacement_meshes = std::collections::HashMap::new();
            for draw in &mut draws {
                if programs
                    .as_ref()
                    .is_some_and(|p| p.displaces(records[draw.material].tag[0]))
                {
                    let node=&nodes[roots[draw.object as usize].node].node;
                    let implicit=matches!(&node.kind,NodeKind::Mesh {geometry,..} if matches!(input.geometry[geometry].topology,vork_scene::v2::Topology::Implicit));
                    let point_normal=draw.exit_geometry.as_ref().is_some_and(|g|!g.point_centers.is_empty()) && programs.as_ref().unwrap().displacement_uses_normal(records[draw.material].tag[0]);
                    if displacement_seam_nodes.contains(&roots[draw.object as usize].node) {continue;}
                    if draw.computed || implicit || point_normal {
                        crate::warnings::dropped("displacement_geometry", serde_json::json!({"node":node}), format!("warning: node {:?}: ignored displacement on computed/implicit geometry or points using NORMAL",node.name));
                        continue;
                    }
                    ensure!(
                        draw.layout.deform,
                        "displacement geometry must carry evaluated vertex frames"
                    );
                    let geometry = draw
                        .exit_geometry
                        .as_ref()
                        .context("displacement requires retained triangle geometry")?;
                    let timed = animated.contains(&draw.material) || programs.as_ref().unwrap().displacement_timed(records[draw.material].tag[0]);
                    let key = (std::sync::Arc::as_ptr(geometry), timed);
                    let mesh = if let Some(mesh) = displacement_meshes.get(&key) {
                        std::sync::Arc::clone(mesh)
                    } else {
                        let mesh = displacement::Mesh::new(r, geometry, &attribute_names, timed)?;
                        displacement_meshes.insert(key, mesh.clone());
                        mesh
                    };
                    draw.displacement = Some(mesh);
                }
            }
            draws
                .iter()
                .any(|d| d.displacement.is_some())
                .then(|| {
                    displacement::Pipeline::new(
                        r,
                        programs.as_ref().and_then(|p| p.native.as_deref()).unwrap(),
                        &mat_layout,
                        attribute_names.len(),
                    )
                })
                .transpose()?
        } else {
            None
        };
        // Only beauty views consume solid exit traversal. Path controls and
        // shadow passes do not. Include every reachable render-image capture.
        if views.iter().any(|view|view.settings.integrator.raster().is_some_and(|r|r.refraction==vork_scene::Refraction::Screen)) {
            refuse_screen_contents(&draws,&roots,&objects,&materials)?;
        }
        let mut needs_raster_solid_exit = vec![false; nodes.len()];
        if !effects.path {
            for settings in views.iter().filter(|settings| settings.settings.integrator.raster().is_some_and(|r| r.refraction != vork_scene::Refraction::Ray)) {
                let included = crate::render_images::included_nodes(&nodes, settings.root.as_deref())?;
                for (needed, included) in needs_raster_solid_exit.iter_mut().zip(included) {
                    *needed |= included;
                }
            }
        }
        let mut exit_trees = std::collections::HashMap::new();
        // Whether a path solid is closed spans all its material groups, opaque
        // ones included: every draw of a mesh that owns a solid records its range's edges.
        // Rows are renumbered by group_draws below, so occupancy's solids are built again after it.
        let shadow_occupancy=rays && draws.iter().any(|d|materials[d.material].solid_absorption);
        let probe_field=render.settings.integrator.probe_volume().map(|(volume,_)| {
            let eyes=probe_eyes.clone().unwrap_or_else(||nodes.iter().filter(|n| matches!(n.node.kind, NodeKind::Camera { .. }))
                .map(|n| n.world.w_axis.truncate()).collect::<Vec<_>>());
            let demand=nodes.iter().any(|n| crate::compute::places_instances(input,&n.node));
            crate::probes::placement::Field::in_frame(&volume,&draws,&objects,&eyes,demand,probe_frame.unwrap_or_default(),&r.device)
        }).transpose()?;
        for draw in &mut draws {
            if let Some(geometry) = draw.exit_geometry.take() {
                if materials[draw.material].solid_transmission
                    && !ray_refraction
                    && needs_raster_solid_exit[roots[draw.object as usize].node]
                {
                    let key = (draw.vertices.clone(), draw.vertex_offset);
                    draw.exit_bvh = Some(exit_trees
                        .entry(key)
                        .or_insert_with(|| crate::transmission::mesh(&r.device, draw, &geometry))
                        .clone());
                }
                if draw.computed { draw.exit_geometry=Some(geometry); }
            }
        }
        let mut jobs = std::collections::HashMap::new();
        for draw in &mut draws {
            if let Some(deform) = &mut draw.deform {
                let key = (
                    deform.source.clone(),
                    deform.native.as_ref().map(|binding| binding.mesh),
                    deform.asset,
                    deform.node,
                    deform.skin,
                    roots[draw.object as usize].time_offset.to_bits(),
                );
                let next = jobs.len();
                deform.job = *jobs.entry(key).or_insert(next);
            }
        }
        // A compute node owns one transform/count domain; never coalesce it with
        // another node even if geometry and material happen to match.
        let compute_nodes: std::collections::HashSet<usize> = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| {
                if !crate::compute::places_instances(input,&node.node) {return None;}
                if draws.iter().any(|d| roots[d.object as usize].node==i && materials[d.material].transparent) {
                    crate::warnings::dropped("computed_instances", serde_json::json!({"node":node.node}), format!("warning: node {:?}: ignored computed instance placement on transparent material; using authored placements",node.node.name));
                    None
                } else {Some(i)}
            })
            .collect();
        // CPU-fitted fibre captures cannot bound GPU-written placements.
        // Probe support covers the cameras' complete clip regions and captures
        // through the current computed TLAS after placement publication.
        if rays && !compute_nodes.is_empty() {
            ensure!(!effects.fibre,
                "computed ray placement does not support CPU-fitted fibre captures; choose --profile std/path");
            let moving_medium=compute_nodes.iter().any(|&index| matches!(&nodes[index].node.kind,
                NodeKind::Mesh { materials, .. } if materials.iter().any(|name|stages.get(name).is_some_and(|stages|stages.medium))));
            ensure!(!moving_medium,
                "computed ray placement does not support moving medium boundaries; certified medium domains must follow the same GPU placement snapshot");
        }
        let draw_groups = group_draws(
            &nodes,
            &compute_nodes,
            &materials,
            &mut draws,
            &mut objects,
            &mut roots,
            r.device
                .features()
                .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
        );
        let mut leaders = std::collections::HashMap::new();
        for (index, draw) in draws.iter_mut().enumerate() {
            if let Some(deform) = &mut draw.deform {
                deform.leader = *leaders.entry(deform.job).or_insert(index);
            }
        }
        let mut evaluations = std::collections::HashMap::new();
        for (index, draw) in draws.iter_mut().enumerate() {
            if let Some(mesh) = &draw.displacement {
                let object = draw.object as usize;
                let key = (std::sync::Arc::as_ptr(mesh), draw.material,
                    roots[object].time_offset.to_bits(), objects[object].metadata[2],
                    draw.deform.as_ref().map(|d| d.job));
                draw.displacement_leader = Some(*evaluations.entry(key).or_insert(index));
            }
        }
        let program_buffers = input
            .programs
            .values()
            .any(|p| p.is_compute())
            .then_some(()).or_else(|| (input.buffers.values().any(|b| b.dims.is_some()) || draws.iter().any(|d|d.computed)).then_some(()))
            .map(|_| buffers::ProgramBuffers::install(r, input, &buffer_data))
            .transpose()?;
        let (dense_destinations,dense_certificates)=medium.as_ref().map(|m|m.dense_bindings(input,medium_base,&draws,&native_ids)).transpose()?.unwrap_or_default();
        let dense=crate::dense::Dense::install(r,input,program_buffers.as_ref(),dense_destinations,dense_certificates)?;
        ensure!(!draws.iter().any(|d|d.computed) || render.settings.integrator.probe_volume().is_none(),
            "computed geometry cannot use CPU-planned probe support; automatic lighting selects complete path transport");
        let computed_geometry = draws.iter().map(|draw| {
            if !draw.computed { return Ok(None); }
            ensure!(!materials[draw.material].surface_coverage && !materials[draw.material].transmissive, "computed geometry requires opaque coverage; dynamic ray alpha and moving medium boundaries are unsupported");
            ensure!(draw.displacement.is_none(), "computed geometry with material displacement is unsupported");
            let node = &nodes[roots[draw.object as usize].node].node;
            let NodeKind::Mesh { geometry, .. } = &node.kind else { unreachable!() };
            crate::computed_geometry::Mesh::new(r, &input.geometry[geometry], draw, &attribute_names, program_buffers.as_ref().unwrap()).map(Some)
        }).collect::<Result<Vec<_>>>()?;

        let mut field_objects=std::collections::BTreeMap::<String,u32>::new();
        for draw in &draws {
            if let NodeKind::Mesh {geometry,..}=&nodes[roots[draw.object as usize].node].node.kind {
                let geometry=&input.geometry[geometry];
                if matches!(geometry.topology,vork_scene::v2::Topology::Implicit) {
                    let name=geometry.attributes.values().next().unwrap();
                    field_objects.entry(name.clone()).and_modify(|object|*object=(*object).min(draw.instances.start)).or_insert(draw.instances.start);
                }
            }
        }
        let mut implicit_fields=std::collections::BTreeMap::<String,ImplicitField>::new();
        for draw in &mut draws {
            let node=&nodes[roots[draw.object as usize].node].node;
            if let NodeKind::Mesh {geometry,..}=&node.kind {
                let geometry=&input.geometry[geometry];
                if matches!(geometry.topology,vork_scene::v2::Topology::Implicit) {
                    ensure!(!draw.evaluated(), "implicit geometry cannot use skinning or displacement; evolve the scalar field instead");
                    if let NodeKind::Mesh {materials:bindings,..}=&node.kind {
                        for name in bindings {
                            ensure!(!stages[name].medium, "implicit geometry does not yet support a material medium stage");
                            let subsurface=if let Some(instance)=programs::instance(input,name)? {
                                programs.as_ref().context("implicit material program")?.is_subsurface(&instance)
                            } else {materials::openpbr_values(input,Some(name),animated_inputs.get(name).unwrap_or(&Default::default()),|_|Ok(0))?.capabilities.subsurface};
                            ensure!(!subsurface, "implicit geometry does not yet support subsurface scattering");
                        }
                    }
                    let name=geometry.attributes.values().next().unwrap();
                    let pages=program_buffers.as_ref().unwrap();
                    let dims=input.buffers[name].dims.unwrap();
                    let spans:Vec<_>=pages.bindings[name].spans.iter().map(|s|(pages.storage[s.page as usize].clone(),s.clone())).collect();
                    if !implicit_fields.contains_key(name) {
                        let acceleration=std::sync::Arc::new(crate::implicit_acceleration::Acceleration::new(r,dims,&spans,field_objects[name])?);
                        implicit_fields.insert(name.clone(),ImplicitField {source:name.clone(),dims,spans,acceleration});
                    }
                    for object in draw.instances.clone() {objects[object as usize].metadata[1] |= vork_slang::registry::OBJECT_IMPLICIT;}
                    draw.implicit=Some(implicit_fields[name].clone());
                }
            }
        }
        let sources: Vec<_> = nodes.iter().enumerate().map(|(i,n)|
            (!crate::compute::places_instances(input,&n.node) || compute_nodes.contains(&i)).then_some(&n.node)).collect();
        let mut compute = compute::Compute::install(
            r,
            input,
            base,
            program_buffers.as_ref(),
            &sources,
            &roots,
            &draws,
            render.fps,
            render.duration,
        )?;
        if let Some(compute) = &compute {
            for (i, draw) in draws.iter().enumerate() {
                if compute.computed_draw(i) {
                    ensure!(
                        !materials[draw.material].transparent,
                        "compute instances require opaque or alpha-mask material"
                    );
                    for object in draw.instances.clone() {
                        let root = &mut roots[object as usize];
                        nodes[root.node].dynamic = true;
                        root.local = Mat4::IDENTITY;
                        let previous = objects[object as usize];
                        objects[object as usize] = Object {
                            model: nodes[root.node].world,
                            previous_model: nodes[root.node].world,
                            next_model: nodes[root.node].world,
                            ..previous
                        };
                    }
                }
            }
        }
        let path_media=medium.as_ref().map(|m|m.words[1]);
        let plan = crate::plan::ScenePlan::new(
            effects, &render, &draws, &materials, &nodes, &roots, &objects, &light_plan, &animation,
            rays, compute.as_ref(), filtered_media,
        );
        let mut solids = if plan.effects.path || ray_refraction || shadow_occupancy {
            crate::passes::path::solids(&draws, &roots, &materials)?
        } else { Default::default() };
        if ray_refraction && solids.boundaries.is_empty() {
            // Thin-only transport still queries the common per-object/instance
            // identity tables. Every entry explicitly denotes no solid owner.
            solids.boundaries.resize(objects.len(),u32::MAX);
            solids.owners.resize(objects.len(),u32::MAX);
        }
        let (occupancy_solids, boundary_capacity)=crate::passes::path::occupancy(&solids,&draws,&roots,&nodes,&objects,compute.as_ref(),&animation,&targets);
        // A thin-only walk retains an empty occupancy state. One unused slot
        // keeps the existing transport specialization distinct from no transport;
        // real solid capacity remains the geometry-derived bound.
        let refraction_capacity=if ray_refraction {boundary_capacity.max(1)} else {0};
        if plan.decoupled {
            crate::passes::decoupled::supported(&plan, !ray_refraction && medium.as_ref().is_some_and(|m|m.words[1]>0) && draws.iter().any(|d|materials[d.material].solid_transmission))?;
            crate::passes::decoupled::check_view(&r.device.limits(), render.resolution,
                plan.visibility_samples, crate::passes::decoupled::ViewShape {transparent: plan.transparent,
                diffusion: plan.effects.subsurface, reflections: plan.effects.reflections, screen_refraction: plan.screen_refraction, table_only: records.iter().all(|r| r.tag[0] == 0),
                boundaries: refraction_capacity, reserved: reservations.visibility_pool})?;
        }
        let mut plan = plan;
        plan.effects.image_feedback = (rays || effects.probes || effects.fibre)
            && input.image_reads(None).iter().any(|name|
                input.images.get(name).is_some_and(|image| image.render.is_some()));
        let graph = crate::graph::Graph::new(&plan, readback.any())?;
        let transients = crate::graph::Transients::new(&graph, &r.residency);
        let random_walk=plan.effects.path && material_spatial_subsurface
            && render.settings.integrator.subsurface_method()==Some(vork_scene::v2::SubsurfaceMethod::RandomWalk);
        let subsurface_ids=if random_walk {crate::passes::path::mesh_identities(&draws,&roots)?} else {Vec::new()};
        // Node topology is fixed at install, including imported and extracted suns.
        // Dynamic lights may cross zero size; retain both emitter implementations.
        // The light_plan covers installed/imported sources and render-image map
        // consumers. Shadow permission survives zero energy and animation.
        let mut light_features = 0;

        if light_plan.nodes.iter().any(|&i| crate::rt::traces_shadows(&nodes[i].node.kind)) {
            light_features |= materials::FEATURE_SHADOW_RAYS;
        }
        for node in &nodes {
            if let NodeKind::Light {
                shape,
                radius,
                angle,
                ..
            } = &node.node.kind
            {
                use vork_scene::LightShape;
                let (kind, finite) = match shape {
                    LightShape::Environment => continue,
                    LightShape::Sun => (0, *angle * 0.5 != 0.),
                    LightShape::Point => (1, *radius != 0.),
                    LightShape::Spot => (2, *radius != 0.),
                    LightShape::Rectangle | LightShape::Ellipse => (3, true),
                };
                light_features |= 1 << (materials::FEATURE_LIGHT_SHIFT + kind);
                light_features |= if finite {
                    materials::FEATURE_FINITE
                } else {
                    materials::FEATURE_PUNCTUAL
                };
                if node.dynamic {
                    light_features |= materials::FEATURE_FINITE | materials::FEATURE_PUNCTUAL;
                }
                if finite || node.dynamic {
                    light_features |= if matches!(shape, LightShape::Rectangle) {
                        materials::FEATURE_RECTANGLE
                    } else {
                        materials::FEATURE_ELLIPSE
                    };
                }
            }
        }
        let path = if plan.effects.path {
            // Hits and mesh-light connections can reach every installed material,
            // including off-camera draws. Capture the union before raster's variant
            // budget can replace individual material keys with its generic fallback.
            let features = materials.iter().fold(materials::FEATURE_SPECIALIZED, |union, m| union | m.features);
            // A programmed lit receiver evaluates the shared direct-light service
            // at path hits. Its variant needs the installed light capabilities
            // as well as the material response proofs.
            let features = if features & materials::FEATURE_LIT != 0 { features | light_features } else { features };
            // Beauty convergence does not establish independently authored AOV convergence.
            Some(crate::passes::path::Pipeline::new(r, crate::constants::PATH_USE_ADAPTIVE_SAMPLING && render.aovs.is_empty() && !crate::catcher::has_catchers(input), crate::catcher::has_catchers(input), random_walk, features, boundary_capacity, !emitter_profiles.is_empty(), path_media, options, &pipelines)?)
        } else { None };
        // Reflections can hit every material, before raster variant budgeting.
        let hit_material_features = materials.iter().fold(materials::FEATURE_SPECIALIZED, |union, m| union | m.features);
        if let Some(compute) = &mut compute {
            compute.prepare_bridge(r, &draws, &objects)?;
        }
        let material_bind_ms = material_bind_start.elapsed().as_secs_f64() * 1000.;
        // Ray shadow candidates read other materials' opacity, not only the
        // receiver's textures. Never prune that lookup using the receiver key.
        let ray_alpha = if cached.ray_shadows {
            materials::texture_feature("geometry_opacity")
        } else {
            0
        };
        for material in &mut materials {
            material.features |= light_features | ray_alpha;
        }
        // Independent hit kernels share immutable installed material facts. Compile
        // Slang and create driver pipelines concurrently, joining before drawing.
        // The hit union already contains every candidate’s texture facts;
        // raster’s receiver-only alpha expansion would erase that global proof.
        let features = hit_material_features | light_features;
        let mut visibility = vec![plan.visibility_samples];
        if plan.decoupled {
            for settings in &views[1..] {
                visibility.push(crate::render_images::camera_sampling(&nodes, settings, camera)?.2);
            }
        }
        let mut camera_program_features=vec![materials::FEATURE_SPECIALIZED; records.iter().map(|r|r.tag[0] as usize).max().unwrap_or(0)+1];
        for material in &materials {camera_program_features[material.program as usize]|=material.features;}
        materials::bound_features(&mut materials, draws.iter().map(|d| (d.layout, d.material)));
        // Controls and camera evaluations compile the same authored source for
        // independent GPU stages. Install both concurrently before any draw.
        let geometry_keys =
            draws
                .iter()
                .zip(&plan.draws)
                .flat_map(|(draw, flags)| {
                    pipelines::Role::kinds(flags, plan.decoupled).iter().copied().chain(crate::fibre_transport::Stage::ALL.into_iter().filter(|_|plan.effects.fibre && flags.hair).map(|stage|crate::bundles::Kind::Fibre(stage,false))).map(|kind| {
                        (
                            draw.layout,
                            pipelines::Role::of(kind, flags, &materials[draw.material]),
                        )
                    })
                })
                .collect();
        let asset_pipeline_start = Instant::now();
        let (decoupled, reflections, gi, geometry_pipelines) = std::thread::scope(|scope| {
            let assets = scope.spawn(|| cached.ensure_assets(r, &plan));
            let geometry = scope.spawn(|| pipelines.install_geometry(r, geometry_keys, variants));
            let reflections = plan.effects.reflections.then(|| scope.spawn(||
                crate::passes::reflections::pipeline(r, &pipelines, features, plan.effects.probes, variants)));
            let gi = plan.effects.ray_indirect.then(|| scope.spawn(||
                crate::passes::ao::ray_pipeline(r, &pipelines, features, plan.effects.probes, variants)));
            let decoupled = plan.decoupled.then(||
                crate::passes::decoupled::Install::new(r, &pipelines, &draws, features, &plan, &render, &visibility, &records, &camera_program_features, ray_refraction, refraction_capacity, !emitter_profiles.is_empty(), variants)).transpose();
            use pipelines::join;
            join(assets)?;
            anyhow::Ok((decoupled?,
             reflections.map(join).transpose()?,
             gi.map(join).transpose()?,
             join(geometry)?))
        })?;
        // Stop at a driver failure creating these pipelines instead of building
        // the rest of the scene on a lost device.
        r.errors.check()?;
        let vertex_defaults = init(
            &r.device,
            "default vertex channels",
            bytemuck::cast_slice(&[0f32, 0., 0., 0., 1., 1., 1., 1.]),
            wgpu::BufferUsages::VERTEX,
        );
        pipeline_ms +=
            asset_pipeline_start.elapsed().as_secs_f64() * 1000. + textures.pipeline_ms.get();
        // Texture GPU work runs while geometry and its pipelines are installed.
        // Complete readback/alpha correction before any shadow or colour draw.
        // Decoder ownership outlives source metadata; unused images retain the
        // same error behavior and every worker finishes before scene readiness.
        image_decoders.finish()?;
        let texture_finish = Instant::now();
        textures.preparation.finish(r)?;
        textures.upload_ms.set(textures.upload_ms.get() + texture_finish.elapsed().as_secs_f64() * 1000.);
        for draw in &draws {
            for object in draw.instances.clone() {
                objects[object as usize].material_index[0] = phased
                    .get(&(draw.material, roots[object as usize].time_offset.to_bits()))
                    .copied()
                    .unwrap_or(draw.material)
                    as u32;
            }
        }
        let instances: Vec<_> = draws
            .iter()
            .enumerate()
            .flat_map(|(i, d)| d.instances.clone().map(move |object| (i, object)))
            .collect();
        let object_bytes = (objects.len().max(1) * std::mem::size_of::<GpuObject>()) as u64;
        let object_limit = limits.max_buffer_size.min(limits.max_storage_buffer_binding_size);
        ensure!(
            object_bytes <= object_limit,
            "object storage exceeds device storage binding/allocation limit ({object_limit} bytes); \
             required {object_bytes} bytes; backend {:?}; adapter {:?}; driver {:?}",
            r.adapter_info.backend,
            r.adapter_info.name,
            r.adapter_info.driver
        );
        let geometry_vertex_bytes = draws
            .iter()
            .flat_map(|d| [d.vertices.clone(), d.attributes.clone()])
            .collect::<std::collections::HashSet<_>>()
            .iter()
            .map(|buffer| buffer.size())
            .sum();
        // The compute bridge broadcasts one evaluated parent per draw. GPU-written
        // transforms must never enter the per-instance CPU dirty/upload path.
        let dynamic_objects = roots
            .iter()
            .enumerate()
            .filter_map(|(i, root)| {
                let computed = compute.as_ref().and_then(|c| {
                    draws.iter().enumerate().find(|(id, d)| {
                        c.computed_draw(*id) && d.instances.contains(&(i as u32))
                    }).map(|(_, d)| d.instances.start as usize)
                });
                match computed {
                    Some(parent) => (i == parent).then_some(i),
                    None => (nodes[root.node].dynamic || root.imported_pose.is_some()).then_some(i),
                }
            })
            .collect();
        let rays = rays
            .then(|| crate::rt::Rays::new(r, &draws, &instances, &objects, &plan.casters.instances[0], ray_refraction || crate::rt::other_hit_surfaces(&scene) || program_coverage, &materials, &render.settings.integrator, &solids.boundaries, &solids.owners, &subsurface_ids, &occupancy_solids, medium_sources))
            .transpose()?;
        let capture_controls = input.materials.values().any(|m| {
            m.program != "std/openpbr"
                && m.program != "std/fog"
                && m.inputs.values().any(|v| {
                    let name = match v {
                        vork_scene::v2::Input::Texture(t) => Some(&t.image),
                        _ => None,
                    };
                    name.is_some_and(|n| input.images.get(n).is_some_and(|i| i.render.is_some()))
                })
        });
        let fibre = if effects.fibre {
            let suns:Vec<_>=light_plan.nodes.iter().map(|&i|matches!(&nodes[i].node.kind,NodeKind::Light {shape:vork_scene::LightShape::Sun,..})).collect();
            let resolution=light_plan.nodes.iter().map(|&i| match &nodes[i].node.kind {
                NodeKind::Light {..}=>crate::constants::FIBRE_CAPTURE_RESOLUTION.max(fibre_view_side),_=>1
            }).max().unwrap_or(1);
            let capture_plan=crate::fibre_transport::Plan::new(&suns,resolution,r.max_texture_allocation,&limits)?;
            Some(crate::fibre_transport::Persistent::new(r,capture_plan,suns.len(),&pipelines.fibre_capture_layout,
                programs.as_ref().map_or(Default::default(), |p|p.dependencies()),plan.deform))
        } else {None};
        let probes = render.settings.integrator.probe_volume().map(|(_volume, bounces)| {
            let dependencies = programs.as_ref().map_or(Default::default(), |p| p.dependencies());
            let timed = dependencies.time || medium.as_ref().is_some_and(|m|m.native.time_dependent) || compute.is_some() || plan.deform || animation.has_material_bindings();
            {
                let mut field=probe_field.context("retained indirect lighting: missing receiver field")?;
                if let Some(rays)=rays.as_ref().filter(|r|r.probe_emitters) {field.header.visibility_policy[3]=rays.emitters;}
                crate::probes::Volume::new(r, &pipelines, field, bounces, timed, plan.exposure_samples, dependencies, dependencies.time || compute.is_some() || plan.deform || animation.has_material_bindings(), !programs.as_ref().is_some_and(|p|p.has_lit()), features, variants)
            }
        }).transpose()?;
        for draw in &mut draws {
            draw.ray_corners = None;
            if draw.computed {draw.exit_geometry=None;}
        }
        let compositor = compositor::install(r, &scene, base)?;
        let effect_wait = Instant::now();
        if let Some(compiler) = effects_compiler {
            compiler.join().map_err(|_| anyhow::anyhow!("scene effect pipeline compiler panicked"))??;
        }
        pipeline_ms += effect_wait.elapsed().as_secs_f64() * 1000.;
        let sampling = crate::sampling::Parameters::new(&r.device, &scene, &render);
        Ok(Self {
            has_catchers: crate::catcher::has_catchers(&scene),
            footage: std::mem::take(&mut textures.footage), file_images,
            reservations,
            sampling,
            displacement,
            compositor,
            shadows: crate::residency::Cache::new(&r.residency),
            compute,
            dense,
            computed_geometry,
            rays,
            animation,
            dynamic_objects,
            pipelines,
            plan,
            graph,
            transients,
            path,
            reflections,
            decoupled,
            gi,
            probes,
            fibre,
            geometry_vertex_bytes,
            geometry_pipelines,
            vertex_defaults,
            environment,
            propagation_base: record_words + propagation_offset as u32,
            camera_programs,
            spectral,
            links,
            medium_base,
            medium_boundaries: path_media.unwrap_or(0) as usize,
            medium_certified_times: medium.as_ref().filter(|m|m.native.tracking).map(|_|medium::certification_times(input)),
            emitter_profiles,
            assets,
            pipeline_ms,
            environment_install_ms,
            checked_shader_capacity: true,
            material_table,
            material_bind_ms,
            asset_decode_ms,
            texture_upload_ms: textures.upload_ms.get(),
            texture_cache_hits: textures.cache_hits.get(),
            render,
            scene,
            nodes,
            targets,
            roots,
            object_ids: std::sync::Arc::new(serde_json::Value::Array(object_ids)),
            camera,
            light_plan,
            draws,
            implicit_fields:implicit_fields.into_values().collect(),
            draw_groups,
            materials,
            objects,
            display_program,
            surface_dependencies: programs.as_ref().map_or(Default::default(), |p|p.program_dependencies()),
            main_subset: None,
            capture_controls,
            readback,
            install_time: Duration::ZERO,
        })
    }
    pub fn render(&self) -> &vork_scene::Render {
        &self.render
    }
    /// The name of the camera that renders at `time` (camera cuts included).
    pub fn camera_name_at(&self, time: f32) -> &str {
        &self.nodes[self.animation.camera_at(time, self.camera)].node.name
    }
    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    /// Transient bytes all frame slots and views requested, and bytes the
    /// frame graph allocated for them.
    pub fn transient_bytes(&self) -> (u64, u64) {
        self.transients.bytes()
    }
    pub fn depth_unit_m(&self) -> f32 {
        self.nodes[self.camera].camera.as_ref().and_then(|c|c.calibration(self.render.resolution[0] as f32/self.render.resolution[1] as f32).ok().flatten()).map_or(1.,|c|c[1][1]/65535.)
    }
    fn refresh_frame_slot(&self, r: &Renderer, slot: &mut FrameSlot) -> Result<()> {
        ensure!(!slot.pending, "cannot replace a submitted frame slot");
        let precision = if slot.view.float_output { crate::ReadbackPrecision::Float } else { crate::ReadbackPrecision::Packed };
        let mut replacement = self.create_frame_slot(r, precision)?;
        std::mem::swap(&mut replacement.ring, &mut slot.ring);
        std::mem::swap(&mut replacement.spare_ring, &mut slot.spare_ring);
        replacement.ring.rebind_pixels(&replacement.view.post.pixels);
        for ring in &mut replacement.spare_ring { ring.rebind_pixels(&replacement.view.post.pixels); }
        if let Some((output, bind)) = &replacement.ring.output {
            replacement.view.output = output.clone();
            replacement.view.output_bind = bind.clone();
        } else {
            // Library attachments set through set_output are not ring-owned.
            replacement.view.output = slot.view.output.clone();
            replacement.view.output_bind = slot.view.output_bind.clone();
        }
        replacement.profile = slot.profile;
        replacement.write_color = slot.write_color;
        replacement.completed = slot.completed.take();
        *slot = replacement;
        Ok(())
    }

    /// Render one frame, growing frame storage on the same installation if needed.
    pub fn render_frame(&mut self, r: &Renderer, slot: &mut FrameSlot, time: f32) -> Result<crate::FrameOutput> {
        if slot.probe_generation != self.reservations.probe_pages {
            self.refresh_frame_slot(r, slot)?;
        }
        loop {
            match slot.submit(self, r, time).and_then(|()| slot.collect(r)) {
                Ok(output) => return Ok(output),
                Err(error) => {
                    let Some(redo) = error.downcast_ref::<crate::FrameRedo>() else { return Err(error); };
                    self.redo_frame(r, std::slice::from_mut(slot), redo.0, (time * self.render.fps).round() as usize)?;
                }
            }
        }
    }

    /// Drain rejected work and rebuild only frame storage, preserving encoder surfaces
    /// and outstanding output leases. The scene and its shared caches stay installed.
    /// Callers must include every frame slot of this install: all are replaced
    /// (Transients::begin_generation); on error they are unusable.
    pub fn redo_frame(
        &mut self, r: &Renderer, slots: &mut [FrameSlot],
        mut demand: crate::Reservations, frame: usize,
    ) -> Result<u32> {
        ensure!(!slots.is_empty(), "redo needs a frame slot");
        let mut growths = 0;

        loop {
            for slot in slots.iter_mut().filter(|slot| slot.pending) {
                // Captures can reuse an earlier exposure's provisional image.
                // Until its producers are accepted, a successor is not independent.
                match slot.collect_for_redo(r, self.graph.views.images.is_empty()) {
                    Ok(()) => {},
                    Err(error) => match error.downcast_ref::<crate::FrameRedo>() {
                        Some(redo) => {
                            demand=demand.max(redo.0);

                        },
                        None => return Err(error),
                    },
                }
            }
            let grown = self.reservations.max(demand);
            ensure!(grown != self.reservations,
                "frame storage cannot grow beyond {:?}: a shading band of camera evaluations, or a merge batch of transparent fragments, needs more records than the device can address",
                self.reservations);

            if grown.probe_pages>self.reservations.probe_pages {
                self.probes.as_mut().context("probe demand without installed probes")?.rebuild(r)?;
                // Every shadow reader pins the old probe bindings. Rebuild the
                // shared dependency and invalidate all provisional captures.
                let replacement=self.shadows().rebind_probes(self.probes.as_ref().unwrap().storage());
                let bytes=replacement.bytes();
                self.shadows.insert(replacement,bytes);
            }
            self.reservations = grown;
            growths += 1;
            self.transients.begin_generation();
            for slot in slots.iter_mut() {
                self.refresh_frame_slot(r, slot)?;
            }
            self.transients.release_previous_generations();
            self.transients.submit.lock().unwrap().invalidate_captures();
            let previous_images = self.probes.as_ref().is_some_and(|p| p.dependencies.images)
                || self.fibre.as_ref().is_some_and(|p| p.dependencies.images);
            if !previous_images || self.graph.views.images.is_empty() { return Ok(growths); }
            if frame == 0 {
                let mut encoder = r.device.create_command_encoder(&Default::default());
                for image in self.graph.views.image_renders() { image.clear(&mut encoder); }
                crate::command_finish::Segments::submit(&r.queue, encoder);
                self.transients.submit.lock().unwrap().image_revision += 1;
                return Ok(growths);
            }
            let mut warm_slot = self.create_frame_slot(r, Default::default())?;
            let slot = &mut warm_slot;
            slot.write_color = true;
            // Restore the previous published generation before pre-capture readers.
            // Seek replay also restores temporal history and frame simulations.
            let warm = slot.submit_outputs(self, r, (frame - 1) as f32 / self.render.fps, crate::ReadbackOutputs::default())
                .and_then(|()| slot.collect_readback(r).map(drop));
            match warm {
                Ok(()) => return Ok(growths),
                Err(error) => match error.downcast_ref::<crate::FrameRedo>() {
                    Some(redo) => {demand=redo.0;},
                    None => return Err(error),
                },
            }
        }
    }

    pub fn create_frame_slot(
        &self,
        r: &Renderer,
        precision: crate::ReadbackPrecision,
    ) -> Result<FrameSlot> {
        let _foreground = r.compiler.foreground_preparation();
        r.errors.check()?;
        let slot = FrameSlot::new(self, r, precision);
        r.errors.check()?;
        slot
    }
}

/// Primitive-major slots let the persistent recorder issue one instanced draw per
/// geometry/material group, even when an instance has several primitives.
fn group_draws(
    nodes: &[EvalNode],
    compute_nodes: &std::collections::HashSet<usize>,
    materials: &[Material],
    draws: &mut Vec<Draw>,
    objects: &mut Vec<Object>,
    roots: &mut Vec<MeshRoot>,
    transparent_instancing: bool,
) -> Vec<std::ops::Range<usize>> {
    let mut group_ids = std::collections::HashMap::new();
    let mut groups: Vec<Vec<Draw>> = Vec::new();
    for draw in draws.drain(..) {
        let node = &nodes[roots[draw.object as usize].node];
        let group = if transparent_instancing || (!materials[draw.material].transparent) {
            let key = (
                (
                    draw.vertices.clone(),
                    draw.attributes.clone(),
                    draw.indices.clone(),
                    draw.vertex_offset,
                    draw.index_offset,
                    draw.count,
                    draw.material,
                    draw.primitive_start,
                ),
                (
                    node.dynamic,
                    compute_nodes
                        .contains(&roots[draw.object as usize].node)
                        .then_some(roots[draw.object as usize].node),
                    materials[draw.material].transparent,
                    // Sorted transparency remaps instances into any draw slot in the
                    // group. Every slot must bind the same exit geometry.
                    draw.exit_bvh.clone(),
                    draw.deform.as_ref().map(|deform| deform.job),
                    draw.displacement.as_ref().map(|_|(roots[draw.object as usize].time_offset.to_bits(), objects[draw.object as usize].metadata[2])),
                ),
            );
            *group_ids.entry(key).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            })
        } else {
            // Preserve transparent tie order and keep per-instance deform outputs private.
            groups.push(Vec::new());
            groups.len() - 1
        };
        groups[group].push(draw);
    }
    let old_objects = std::mem::take(objects);
    let old_roots = std::mem::take(roots);
    let mut draw_groups = Vec::new();
    for group in groups {
        let first_draw = draws.len();
        let first_object = objects.len() as u32;
        let transparent = materials[group[0].material].transparent;
        let mut shared = group[0].clone();
        // Preserve authored primitive/instance order even when two primitives
        // share geometry. This also preserves opaque depth ties.
        let mut instances: Vec<_> = group
            .iter()
            .flat_map(|draw| {
                draw.instances
                    .clone()
                    .enumerate()
                    .map(move |(i, object)| (draw.sort_order + i * draw.sort_stride, draw, object))
            })
            .collect();
        instances.sort_by_key(|&(order, _, _)| order);
        for (order, draw, original) in instances {
            let object = objects.len() as u32;
            objects.push(old_objects[original as usize]);
            roots.push(old_roots[original as usize].clone());
            if transparent {
                let mut instance = draw.clone();
                instance.object = object;
                instance.instances = object..object + 1;
                instance.sort_order = order;
                draws.push(instance);
            }
        }
        if !transparent {
            shared.object = first_object;
            shared.instances = first_object..objects.len() as u32;
            draws.push(shared);
        }
        let range = first_draw..draws.len();
        draw_groups.extend(std::iter::repeat_n(range.clone(), range.len()));
    }
    draw_groups
}

#[cfg(all(test, vork_private_tests))]
include!("install_material_binding_tests_private.rs");
