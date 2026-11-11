//! Retained path dispatch. The kernel uses the raster material table, surface
//! evaluation and programs (hit.slang), and the same per-frame TLAS/light
//! buffer; no second geometry, material or lighting install.
use crate::pipelines::{buffer_entry,sampler_entry};
use crate::forward_variant::FeatureMask;
use crate::*;

const TILE: u32 = 8;

fn dispatch_rows([width, height]: [u32; 2]) -> u32 {
    if u64::from(width) * u64::from(height) <= constants::PATH_DISPATCH_PIXELS { return height; }
    ((constants::PATH_DISPATCH_PIXELS / u64::from(width) / u64::from(TILE)) * u64::from(TILE)).max(u64::from(TILE)) as u32
}

// Matches PathSettings in path.slang.
#[repr(C, align(16))]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PathUniform {
    bounces: [u32; 4], limits: [u32; 4], clamp: [u32; 4], sampling: [u32; 4],
    adaptive: [u32; 4],
    // Boundary descriptor offset, subsurface descriptor offset, absolute first row, padding.
    boundaries: [u32; 4],
    environment: [u32; 4],
    implicit: [u32;4],
    matte: [u32; 4],
    propagation: [u32; 4],
}
const _: () = assert!(std::mem::size_of::<PathUniform>() == 160);

/// Renderer object rows are split by material primitives and reordered during
/// draw grouping. Recover the authored mesh-instance identity of every row,
/// across all its material groups (u32::MAX for rows no draw uses).
/// This must never use a material ID, output segmentation ID, or TLAS primitive.
pub(crate) fn mesh_identities(draws: &[install::Draw], roots: &[install::MeshRoot]) -> Result<Vec<u32>> {
    let mut identities=std::collections::HashMap::new();
    let mut objects=vec![u32::MAX;roots.len()];
    for draw in draws {
        for object in draw.instances.clone() {
            let root=&roots[object as usize];
            let key=(root.node,draw.gltf_path.clone(),root.instance_index);
            let next=u32::try_from(identities.len()).context("too many mesh identities")?;
            objects[object as usize]=*identities.entry(key).or_insert(next);
        }
    }
    Ok(objects)
}

/// Dielectric solids: mesh instances with a solid-transmission material group.
#[derive(Default)]
pub(crate) struct Solids {
    /// Per object row: its solid if the row's material transmits, else u32::MAX.
    pub(crate) boundaries: Vec<u32>,
    /// Per object row: the solid owning the row's mesh instance, opaque groups included.
    pub(crate) owners: Vec<u32>,
    pub(crate) count: u32,
}
pub(crate) fn solids(draws: &[install::Draw], roots: &[install::MeshRoot], materials: &[Material]) -> Result<Solids> {
    solids_for(draws, roots, |material|materials[material].solid_transmission)
}
pub(crate) fn solids_for(draws: &[install::Draw], roots: &[install::MeshRoot], transmits:impl Fn(usize)->bool) -> Result<Solids> {
    if !draws.iter().any(|d| transmits(d.material)) {return Ok(Solids::default());}
    let meshes = mesh_identities(draws, roots)?;
    // Mesh identities are dense from 0; u32::MAX marks rows no draw uses.
    let mut solid_of = vec![u32::MAX; meshes.iter().filter(|&&m| m != u32::MAX).map(|&m| m as usize + 1).max().unwrap_or(0)];
    let mut boundaries = vec![u32::MAX; roots.len()];
    let mut count = 0u32;
    for draw in draws.iter().filter(|d| transmits(d.material)) {
        for object in draw.instances.clone() {
            let solid = &mut solid_of[meshes[object as usize] as usize];
            if *solid == u32::MAX {
                *solid = count;
                count = count.checked_add(1).context("too many path dielectric boundaries")?;
            }
            boundaries[object as usize] = *solid;
        }
    }
    let owners = meshes.iter().map(|&mesh| solid_of.get(mesh as usize).copied().unwrap_or(u32::MAX)).collect();
    Ok(Solids { boundaries, owners, count })
}

/// A triangle range's welded edges that a closed, consistently oriented
/// manifold would not have: every edge other than (2 uses, winding 0), with its
/// uses and winding. Material groups of one mesh sum theirs; the mesh is closed
/// only if every sum is (2, 0).
pub(crate) type OpenEdges = Vec<(crate::geometry::Edge, (u32, i32))>;
/// A range with a triangle that welding collapses counts as open: welding
/// must not decide closedness by merging a triangle's own corners.
pub(crate) fn open_edges(indices: &[u32], key: impl Fn(u32) -> [u32; 3]) -> OpenEdges {
    let (edges, collapsed) = crate::geometry::edge_uses(indices, key);
    let mut open: OpenEdges = edges.into_iter().filter(|&(_, uses)| uses != (2, 0)).collect();
    if collapsed {open.push(([[u32::MAX; 3]; 2], (1, 1)));}
    open
}
/// A geometry's welded position keys for open_edges: within 16 ulps of its
/// largest coordinate, above computed seams' few ulps.
pub(crate) fn weld_keys(geometry: &crate::geometry::Geometry) -> Vec<[u32; 3]> {
    let positions: Vec<[f32; 3]> = geometry.vertices.iter().map(|v| v.POSITION).collect();
    let largest = positions.iter().flatten().fold(0_f32, |m, v| m.max(v.abs()));
    crate::geometry::weld_within(&positions, (largest * 16. * f32::EPSILON).max(f32::MIN_POSITIVE))
}
/// Whether a solid made of these triangle ranges is open.
fn open_union<'a>(ranges: impl IntoIterator<Item = &'a OpenEdges>) -> bool {
    let mut sums = std::collections::HashMap::<&crate::geometry::Edge, (u32, i32)>::new();
    for (edge, (uses, winding)) in ranges.into_iter().flatten() {
        let sum = sums.entry(edge).or_default();
        sum.0 += uses;
        sum.1 += winding;
    }
    sums.values().any(|&sum| sum != (2, 0))
}

/// A path solid's bounds in its object space and the object row whose transform places them.
pub(crate) struct PathSolid {
    pub(crate) lo: Vec3, pub(crate) hi: Vec3, pub(crate) row: u32,
    /// Conservative world-space surface bounds; moving/unknown geometry is infinite.
    pub(crate) contact_bounds: (Vec3,Vec3),
    /// Rest bounds of a closed, consistently placed domain. Shadow discovery
    /// refits deforming domains from the existing evaluated geometry bounds.
    pub(crate) shadow_bounds: Option<(Vec3,Vec3)>,
}

/// Each solid's bounds in its object space (the union of its material groups'
/// draw bounds); ±inf for solids that deform, displace, are compute-instanced,
/// whose groups do not share one transform, or that are open (a water plane, a
/// single-surface pane): an open solid stays the half-space its faces bound.
///
/// Also returns the occupancy capacity: a bound on how many solids' bounds can
/// hold one point at one time. Solids that never move overlap as their world
/// bounds do. A solid moved by animation moves rigidly with the nearest node on
/// its chain that animates itself (a transform track, an aim, a glTF clip); the
/// solids moved by one such node keep their relative placement, so they overlap
/// as their bounds do in that node's frame, whatever it does over time, scale
/// 0 included. Each such group can overlap anything, so their capacities add.
/// Animated materials do not move geometry.
#[allow(clippy::too_many_arguments)]
pub(crate) fn occupancy(solids: &Solids, draws: &[install::Draw], roots: &[install::MeshRoot], nodes: &[install::EvalNode], objects: &[Object], compute: Option<&crate::compute::Compute>, animation: &crate::animation::Plan, targets: &crate::targets::Plan) -> (Vec<PathSolid>, u32) {
    const EMPTY: (Vec3, Vec3) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let count = solids.count as usize;
    if count == 0 {return (Vec::new(), 0);}
    let mut local = vec![EMPTY; count];
    let mut first = vec![None::<usize>; count];
    let mut unbounded = vec![false; count];
    let mut geometry_unbounded=vec![false;count];
    let mut moving_geometry = vec![false; count];
    let mut ranges = vec![Vec::<&OpenEdges>::new(); count];
    for (index, draw) in draws.iter().enumerate() {
        let moves = draw.evaluated() || compute.is_some_and(|c|c.computed_draw(index));
        for object in draw.instances.clone() {
            let object = object as usize;
            let Some(&id) = solids.owners.get(object).filter(|&&id| id != u32::MAX) else {continue};
            let id = id as usize;
            local[id] = (local[id].0.min(draw.bounds.0), local[id].1.max(draw.bounds.1));
            moving_geometry[id] |= moves;
            let placement_unbounded=compute.is_some_and(|c|c.computed_draw(index)) || first[id].is_some_and(|other| !roots[other].same_placement(&roots[object]));
            geometry_unbounded[id] |= placement_unbounded;
            unbounded[id] |= placement_unbounded;
            first[id].get_or_insert(object);
            // Without retained geometry, closure is unknown: unbounded.
            match &draw.open_edges {
                Some(edges) => ranges[id].push(edges),
                None => unbounded[id] = true,
            }
        }
    }
    for (unbounded, ranges) in unbounded.iter_mut().zip(&ranges) {
        *unbounded = *unbounded || open_union(ranges.iter().map(|r| r.as_ref()));
    }
    let placed = |id: usize, model: Mat4| placed_bounds(local[id], model);
    let moves_itself = |node: usize| animation.transforms_node(node) || targets.aims[node].is_some()
        || (nodes[node].embedded.is_some() && nodes[node].dynamic);
    let mut fixed = Vec::new();
    let mut contact_bounds=vec![UNBOUNDED;count];
    let mut groups = std::collections::HashMap::<(usize, u32), Vec<(Vec3, Vec3)>>::new();
    for id in 0..count {
        let Some(object) = first[id] else {continue};
        let root = &roots[object];
        // Static nodes between the solid and the node that moves it.
        let mut chain = Vec::new();
        let mut node = Some(root.node);
        let mover = loop {
            let Some(n) = node else {break None};
            if moves_itself(n) {break Some(n);}
            chain.push(n);
            node = nodes[n].parent;
        };
        if count<=8 && !geometry_unbounded[id] && !moving_geometry[id] && mover.is_none() {
            contact_bounds[id]=placed(id,objects[object].model);
        }
        if unbounded[id] || moving_geometry[id] {fixed.push(UNBOUNDED); continue;}
        match mover {
            None => fixed.push(placed(id, objects[object].model)),
            // An embedded glTF node's world comes from its asset pose, not this chain.
            Some(_) if chain.iter().any(|&n| nodes[n].embedded.is_some()) => fixed.push(UNBOUNDED),
            Some(mover) => {
                let relative = chain.iter().rev().fold(Mat4::IDENTITY, |m, &n| m * nodes[n].node.transform.matrix()) * root.local;
                // An instance time offset shifts only its own node's animation.
                let offset = if mover == root.node {root.time_offset.to_bits()} else {0};
                // Relative placement: the kernel's margin also grows with the
                // node's world translation, which this does not bound.
                groups.entry((mover, offset)).or_default().push(placed(id, relative));
            }
        }
    }
    let capacity = groups.values().fold(overlap_capacity(&fixed), |sum, group| sum.saturating_add(overlap_capacity(group)));
    // A solid without a row is in no draw, so never hit or tracked; its row is unused.
    let solids = local.into_iter().zip(unbounded).zip(moving_geometry).zip(first).zip(contact_bounds).map(|(((((lo, hi), unbounded), moving), first), contact_bounds)| {
        let shadow_bounds=(!unbounded).then_some((lo,hi));
        let (lo, hi) = if unbounded || moving {UNBOUNDED} else {(lo, hi)};
        PathSolid { lo, hi, row: first.unwrap_or_default() as u32, shadow_bounds, contact_bounds }
    }).collect();
    (solids, capacity)
}
/// Small register-resident interior list. Actual deeper nesting spills to the
/// device-backed material arena; scene size never grows invocation-private state.
pub(crate) const INLINE_BOUNDARIES: u32 = vork_slang::renderer_registry::REG_PATH_REGISTER_BOUNDARIES;
pub(crate) fn kernel_boundaries(capacity: u32) -> u32 {capacity.min(INLINE_BOUNDARIES)}
/// Equal priorities preserve ordinary last-entered ownership. Disjoint static
/// surface bounds exclude simultaneous crossings even for open/unbounded media.
/// Unknown or moving surfaces retain the general contact transaction.
pub(crate) fn plain_boundaries(solids: &[PathSolid], priorities: impl Iterator<Item=i32>) -> bool {
    let mut priorities=priorities;
    if let Some(first)=priorities.next() {if priorities.any(|p|p!=first) {return false;}}
    if solids.iter().any(|s|!s.contact_bounds.0.is_finite() || !s.contact_bounds.1.is_finite()) {return false;}
    !solids.iter().enumerate().any(|(i,a)|solids[i+1..].iter().any(|b|
        a.contact_bounds.0.cmple(b.contact_bounds.1).all() && b.contact_bounds.0.cmple(a.contact_bounds.1).all()))
}
/// Object-space bounds under `model`, padded twice:
/// - by the f32 error of transforming them (cancellation included),
///   γ7·(|L|·r+|t|), with L, t the model's linear part and translation and r
///   the bounds' reach max(|lo|,|hi|);
/// - by the margin path.slang's solid_bounds_hold grants a point in object
///   space, γ7·(|W|·|p|+|W_t|) with W = model⁻¹. That margin grows with the
///   points it lets in: an object-space padding e covers it when
///   e ≥ A·e + c, with A = γ7·|L⁻¹|·|L| and c = γ7·|L⁻¹|·(|L|·r+2|t|), which
///   holds for e = ‖c‖∞/(1-‖A‖∞) while ‖A‖∞ < 1/2. |L|·e is its world size.
///
/// Every point the kernel lets a solid hold under `model` lies within the
/// result. Placements too ill-conditioned for that bound (‖A‖∞ ≥ 1/2, which
/// leaves room for e's own rounding), and singular ones (the kernel's test
/// holds every point there), are UNBOUNDED.
fn placed_bounds(bounds: (Vec3, Vec3), model: Mat4) -> (Vec3, Vec3) {
    let gamma = vork_slang::registry::FP32_GAMMA7;
    let (lo, hi) = crate::shadow_envelope::transformed(model, bounds);
    if crate::collapsed(model) {return UNBOUNDED;}
    let linear = glam::Mat3::from_mat4(model);
    let (l_abs, l_inv_abs) = (linear.abs(), linear.inverse().abs());
    let translation = model.w_axis.truncate().abs();
    let reach = bounds.0.abs().max(bounds.1.abs());
    let a = l_inv_abs * l_abs * gamma;
    let c = l_inv_abs * (l_abs * reach + 2. * translation) * gamma;
    let a_norm = (0..3).map(|i| a.row(i).element_sum()).fold(0_f32, f32::max);
    if !(a_norm < 0.5) {return UNBOUNDED;}
    let e = Vec3::splat(c.max_element() / (1. - a_norm));
    let error = (l_abs * reach + translation) * gamma + l_abs * e;
    let (lo, hi) = (lo - error, hi + error);
    if !lo.is_finite() || !hi.is_finite() {return UNBOUNDED;}
    (lo, hi)
}
/// Bounds that hold every point.
const UNBOUNDED: (Vec3, Vec3) = (Vec3::splat(f32::NEG_INFINITY), Vec3::splat(f32::INFINITY));
/// At most this many boxes hold any one point. Boxes holding a common point
/// overlap on every axis, so this is at most the fewest boxes overlapping on
/// one axis, and at most one more than the most boxes any one box intersects
/// (path.slang's camera_occupancy relies on it).
fn overlap_capacity(bounds: &[(Vec3,Vec3)]) -> u32 {
    // An empty box (a degenerate transform, or no geometry) bounds nothing.
    let mut boxes:Vec<_>=bounds.iter().copied().filter(|(lo,hi)|lo.cmple(*hi).all()).collect();
    let axis=(0..3).map(|axis| {
        let mut events:Vec<_>=boxes.iter().flat_map(|&(lo,hi)|[(lo[axis],false),(hi[axis],true)]).collect();
        // Entries precede exits at touching endpoints; roundoff cannot remove a solid.
        events.sort_by(|a,b|a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut active=0u32;let mut peak=0;
        for (_,exit) in events {if exit {active-=1;} else {active+=1;peak=peak.max(active);}}
        peak
    }).min().unwrap_or(0);
    // Sweep x; each box is compared with the boxes still open at its start.
    boxes.sort_by(|a,b|a.0.x.total_cmp(&b.0.x));
    let mut degree=vec![0u32;boxes.len()];
    let mut open:Vec<usize>=Vec::new();
    for i in 0..boxes.len() {
        let (lo,hi)=boxes[i];
        open.retain(|&j|boxes[j].1.x>=lo.x);
        for &j in &open {
            let (a,b)=boxes[j];
            if a.y<=hi.y && lo.y<=b.y && a.z<=hi.z && lo.z<=b.z {degree[i]+=1;degree[j]+=1;}
        }
        open.push(i);
    }
    let pairwise=degree.iter().max().map_or(0,|d|d+1);
    axis.min(pairwise)
}

pub(crate) fn layout_entries(adaptive: bool) -> Vec<wgpu::BindGroupLayoutEntry> {
    use wgpu::ShaderStages as Stages;
    let mut entries = crate::pipelines::global_entries();
    entries.extend([
        wgpu::BindGroupLayoutEntry {
            binding: 10,
            visibility: Stages::COMPUTE,
            ty: wgpu::BindingType::AccelerationStructure {
                vertex_return: false,
            },
            count: None,
        },
        buffer_entry(
            11,
            Stages::COMPUTE,
            wgpu::BufferBindingType::Storage { read_only: true },
        ),
        wgpu::BindGroupLayoutEntry {
            binding: 12, visibility: Stages::COMPUTE,
            ty: wgpu::BindingType::Buffer {ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true, min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<PathUniform>() as u64)},
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 13,
            visibility: Stages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba32Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        },
        buffer_entry(
            14,
            Stages::COMPUTE,
            wgpu::BufferBindingType::Storage { read_only: true },
        ),
        pipelines::environment_texture_entry(15),
        sampler_entry(16, false),
        buffer_entry(17, Stages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: true }),
        buffer_entry(18, Stages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: true }),
        buffer_entry(19, Stages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: false }),
        buffer_entry(20, Stages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: false }),
        buffer_entry(24, Stages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: false }),
        buffer_entry(25, Stages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: false }),
    ]);
    if adaptive {
        entries.push(buffer_entry(26, Stages::COMPUTE, wgpu::BufferBindingType::Storage { read_only: false }));
    }
    for (binding,format) in [(21,wgpu::TextureFormat::Rg32Float),(22,wgpu::TextureFormat::Rgba16Float),(23,wgpu::TextureFormat::R32Uint)] {
        entries.push(wgpu::BindGroupLayoutEntry {binding,visibility:Stages::COMPUTE,ty:wgpu::BindingType::StorageTexture {access:wgpu::StorageTextureAccess::WriteOnly,format,view_dimension:wgpu::TextureViewDimension::D2},count:None});
    }
    for e in &mut entries {
        e.visibility = Stages::COMPUTE;
    }
    entries
}

pub(crate) struct Pipeline {
    volume_bounces: u32,
    medium_batch_rows: Option<u32>,
    global_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    kernel: crate::pipelines::ProgramKernel,
}
impl Pipeline {
    pub(crate) fn adaptive(&self)->bool {matches!(self.kernel,crate::pipelines::ProgramKernel::Path {adaptive:true,..})}
    pub(crate) fn new(r: &Renderer, adaptive: bool, catchers: bool, random_walk: bool, features: FeatureMask, boundary_capacity: u32, emitter_profiles: bool, media: Option<u32>, options: &crate::InstallOptions, pipelines: &crate::pipelines::CachedPipelines) -> Result<Self> {
        #[cfg(any(test, feature = "test-support"))]
        let adaptive=adaptive && r.reference_transport_bounces.is_none();
        let d = &r.device;
        let kernel = crate::pipelines::ProgramKernel::Path { adaptive, catchers, random_walk, features, boundaries: boundary_capacity, emitter_profiles, media };
        let (module, entry) = pipelines.program_kernel(kernel)?;
        let entries = layout_entries(adaptive);
        let global_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("path globals"),
            entries: &entries,
        });
        let material_layout = pipelines.mat_layout.clone();
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("path layout"),
            bind_group_layouts: &[Some(&global_layout), Some(&material_layout), None, Some(&crate::rt::hits_layout(d))],
            immediate_size: 0,
        });
        let pipeline = r.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("unidirectional path"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: r.cache.as_ref(),
        });
        #[cfg(not(any(test, feature = "test-support")))]
        let volume_bounces = crate::constants::PATH_MAX_BOUNCES;
        #[cfg(any(test, feature = "test-support"))]
        let volume_bounces = if options.test_participating_media {options.test_volume_bounces} else {crate::constants::PATH_MAX_BOUNCES};
        // Record one workgroup row per settings entry. At execution, measured
        // GPU time combines adjacent entries into watchdog-safe row bands.
        let medium_batch_rows = media.map(|_| TILE);
        #[cfg(any(test, feature = "test-support"))]
        let medium_batch_rows = if options.test_participating_media {
            (media.is_some() && options.test_convex_media && options.test_convex_work_limit.is_some()).then_some(options.test_medium_batch_rows.unwrap_or(32).max(TILE).next_multiple_of(TILE))
        } else {medium_batch_rows};
        #[cfg(any(test, feature="test-support"))]
        let medium_batch_rows = if media.is_some() {
            match options.test_medium_batch_rows {
                Some(0)=>None,
                Some(rows)=>Some(rows.max(TILE).next_multiple_of(TILE)),
                None=>medium_batch_rows,
            }
        } else {None};
        let _ = options;
        Ok(Self { volume_bounces, global_layout, pipeline, kernel,
            medium_batch_rows,
        })
    }
}

pub(crate) fn record(
    s: &InstalledScene,
    r: &Renderer,
    uniform: &wgpu::Buffer,
    lights: &wgpu::Buffer,
    light_tree: &wgpu::Buffer,
    cluster_config: &wgpu::Buffer,
    cluster_lights: &wgpu::Buffer,
    objects: &wgpu::Buffer,
    rays: Option<&crate::rt::FrameRays>,
    material: &wgpu::BindGroup,
    target: &wgpu::TextureView,
    aovs: &wgpu::Buffer,
    controls: &crate::view::Targets,
    catcher: &wgpu::Buffer,
    pixels: &wgpu::Buffer,
    catcher_base: u32,
    recorder: &mut crate::graph::Recorder,
    view: &crate::views::ViewDesc,
) -> Result<()> {
    if !s.graph.runs(crate::graph::Pass::Path) {return Ok(());}
    let guides=controls.denoising_guides.as_ref();
    let render=match &view.kind {crate::views::ViewKind::Image {render,..}=>&render.settings,_=>&s.render};
    let Some(_) = render.settings.integrator.path() else { return Ok(()); };
    let d = &r.device;
    let rays = rays.context("path integrator requires the installed frame TLAS")?;
    let table = &s
        .rays
        .as_ref()
        .context("path integrator requires hit attributes")?
        .alpha;
    let retained = s.path.as_ref().context("path pipeline installed")?;
    let crate::pipelines::ProgramKernel::Path { adaptive, .. } = retained.kernel else { unreachable!() };
    // Adaptive sampling is an entry-point specialization, not a runtime flag.
    let flags = u32::from(crate::constants::PATH_USE_ANIMATED_SEED) << 2;
    let rays_table = s.rays.as_ref().context("path ray table")?;
    let mut params = PathUniform {
        propagation: [s.propagation_base,0,0,0],
        implicit: rays_table.implicit,
        bounces: [crate::constants::PATH_MAX_BOUNCES, crate::constants::PATH_DIFFUSE_BOUNCES, crate::constants::PATH_GLOSSY_BOUNCES, crate::constants::PATH_TRANSMISSION_BOUNCES],
        limits: [retained.volume_bounces, rays_table.film_filter, crate::constants::PATH_TRANSPARENT_MAX_BOUNCES,
            constants::PATH_CAUSTICS],
        clamp: [crate::constants::PATH_SAMPLE_CLAMP_DIRECT.to_bits(), crate::constants::PATH_SAMPLE_CLAMP_INDIRECT.to_bits(), 0, 0],
        sampling: [s.render.settings.samples, s.render.settings.seed, flags, crate::constants::PATH_ADAPTIVE_MIN_SAMPLES],
        adaptive: [crate::constants::PATH_ADAPTIVE_THRESHOLD.to_bits(), s.rays.as_ref().context("path emitter topology")?.emitters,
            crate::constants::PATH_LIGHT_SAMPLING_THRESHOLD.to_bits(), 0],
        boundaries: [rays_table.boundaries, rays_table.subsurface, 0, rays_table.occupancy_solids],
        environment: [s.environment.distribution_size[0], s.environment.distribution_size[1], s.render.aovs.len() as u32,
            u32::from(guides.is_some())],
        matte: [if s.has_catchers {catcher_base} else {0},0,0,0],
    };
    // Previs keeps required ray camera visibility (IES, hair, custom cameras,
    // catchers and propagation) but spends no diffuse or glossy continuation.
    // Existing direct-light MIS handles a zero continuation budget.
    if render.settings.previs {
        params.bounces[1] = 0;
        params.bounces[2] = 0;
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(limit)=r.reference_transport_bounces {
        params.bounces=[limit;4];
        params.clamp[0]=0f32.to_bits();
        params.clamp[1]=0f32.to_bits();
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(limit)=r.reference_transparent_bounces {params.limits[2]=limit;}
    let width = target.texture().width();
    let height = target.texture().height();
    let moments = if adaptive {
        let bytes = u64::from(width) * u64::from(height) * constants::PATH_MOMENT_BYTES;
        ensure!(
            bytes <= d.limits().max_buffer_size
                && bytes <= u64::from(d.limits().max_storage_buffer_binding_size),
            "{}", crate::gpu_memory::limit_message("path adaptive moments", format_args!("{bytes} bytes"), format_args!("storage binding={} bytes, buffer={} bytes",d.limits().max_storage_buffer_binding_size,d.limits().max_buffer_size), "reduce resolution")
        );
        // Interleaved captures retain their own convergence
        // history, just as their resolve keeps a separate exposure sum.
        let owned;
        let label = if let crate::views::ViewKind::Image {name,..} = &view.kind {
            owned = format!("image {name}: path adaptive moments"); owned.as_str()
        } else { "path adaptive moments" };
        Some(s.transients.buffer(d,label,bytes,wgpu::BufferUsages::STORAGE))
    } else {
        None
    };
    let rows = dispatch_rows([width, height]);
    let rows = retained.medium_batch_rows.map_or(rows, |cap| rows.min(cap));
    let chunks = height.div_ceil(rows) as usize;
    let alignment = d.limits().min_uniform_buffer_offset_alignment as usize;
    let stride = std::mem::size_of::<PathUniform>().next_multiple_of(alignment);
    let mut uniforms = vec![0u8; stride * chunks];
    for chunk in 0..chunks {
        params.boundaries[2] = chunk as u32 * rows;
        uniforms[chunk * stride..chunk * stride + std::mem::size_of::<PathUniform>()]
            .copy_from_slice(bytemuck::bytes_of(&params));
    }
    let config = init(d, "path settings", &uniforms, wgpu::BufferUsages::UNIFORM);
    let mut entries = vec![
        entry(0, uniform.as_entire_binding()),
        entry(1, objects.as_entire_binding()),
        entry(2, lights.as_entire_binding()),
        entry(
            3,
            wgpu::BindingResource::TextureView(&s.environment.specular),
        ),
        entry(4, wgpu::BindingResource::Sampler(&s.environment.sampler)),
        entry(
            5,
            wgpu::BindingResource::TextureView(&s.environment.diffuse),
        ),
        entry(7, cluster_config.as_entire_binding()),
        entry(8, cluster_lights.as_entire_binding()),
        entry(6, wgpu::BindingResource::TextureView(&s.environment.brdf)),
        entry(
            9,
            wgpu::BindingResource::TextureView(&s.environment.ltc),
        ),
        entry(10, rays.tlas.as_binding()),
        entry(11, table.as_entire_binding()),
        entry(12, wgpu::BindingResource::Buffer(wgpu::BufferBinding {buffer: &config, offset: 0,
            size: std::num::NonZeroU64::new(std::mem::size_of::<PathUniform>() as u64)})),
        entry(13, wgpu::BindingResource::TextureView(target)),
        entry(14, light_tree.as_entire_binding()),
        entry(15, wgpu::BindingResource::TextureView(&s.environment.source)),
        entry(16, wgpu::BindingResource::Sampler(&s.environment.image_sampler)),
        entry(17, s.environment.distribution.as_ref().unwrap_or(table).as_entire_binding()),
        entry(18,rays.emitter_weights.as_ref().map_or(table,|weights|&weights.buffer).as_entire_binding()),
    ];
    entries.push(entry(19, aovs.as_entire_binding()));
    let unused_guides = guides.is_none().then(|| s.transients.buffer(d,"unused path guides",32,wgpu::BufferUsages::STORAGE));
    entries.push(entry(20,guides.or(unused_guides.as_ref()).unwrap().as_entire_binding()));
    entries.extend([entry(21,wgpu::BindingResource::TextureView(&controls.post_depth)),entry(22,wgpu::BindingResource::TextureView(&controls.post_normals)),entry(23,wgpu::BindingResource::TextureView(&controls.post_ids))]);
    entries.push(entry(24, catcher.as_entire_binding()));
    entries.push(entry(25, pixels.as_entire_binding()));
    if let Some(moments) = &moments {
        entries.push(entry(26, moments.as_entire_binding()));
    }
    let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("path scene"),
        layout: &retained.global_layout,
        entries: &entries,
    });
    let hits = rays.hits_bind(d);
    let dispatches = (0..chunks).map(|chunk| {
        let row = chunk as u32 * rows;
        let dispatch=crate::graph::Dispatch::new(&retained.pipeline, bind.clone(),
            [width.div_ceil(TILE), (height - row).min(rows).div_ceil(TILE), 1])
            .with_material(material).with_hits(hits.clone()).with_offset((chunk * stride) as u32);
        if rays_table.implicit[1]>0 || s.scene.propagation.is_some() || s.plan.effects.implicit {dispatch.all_targets()} else {dispatch}
    }).collect();
    recorder.compute(crate::graph::Pass::Path, "path transport", "path", false, dispatches);
    if retained.medium_batch_rows == Some(TILE) { recorder.time_budget_last(); }
    if chunks > 1 { recorder.separate_last(); }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn path_layout_fits_every_backend_per_stage() {
        for adaptive in [false, true] {
            let globals = layout_entries(adaptive);
            let materials = crate::pipelines::material_entries_for(
                vork_slang::registry::MATERIAL_TEXTURE_CAPACITY,
                vork_slang::registry::MATERIAL_SAMPLER_CAPACITY);
            let hits = [crate::rt::hit_buffers_entry(0, wgpu::ShaderStages::COMPUTE)];
            let groups: [&[wgpu::BindGroupLayoutEntry]; 3] = [&globals, &materials, &hits];
            crate::pipelines::check_stage_limits("path layout", &groups, wgpu::ShaderStages::COMPUTE).unwrap();
        }
    }

    use super::*;

    #[test]
    fn native_path_bindings_are_dense_for_both_sampling_entries() {
        // Vulkan passthrough consumes wgpu's packed binding numbers directly.
        for adaptive in [false,true] {
            let mut bindings=layout_entries(adaptive).into_iter().map(|e|e.binding).collect::<Vec<_>>();
            bindings.sort_unstable();
            assert_eq!(bindings,(0..bindings.len() as u32).collect::<Vec<_>>());
        }
    }

    /// A copy of path.slang's solid_bounds_hold (its object-space margin
    /// γ7·(|W|·|p|+|W_t|)), in f32 as the kernel evaluates it.
    fn kernel_holds(bounds: (Vec3, Vec3), model: Mat4, p: Vec3) -> bool {
        let w = model.inverse();
        let local = w.transform_point3(p);
        let error = Vec3::from_array([0, 1, 2].map(|i| w.row(i).abs().dot(p.abs().extend(1.)))) * vork_slang::registry::FP32_GAMMA7;
        !(local + error).cmplt(bounds.0).any() && !(local - error).cmpgt(bounds.1).any()
    }
    #[test]
    fn placed_bounds_hold_every_point_the_kernel_holds() {
        let unit = (Vec3::splat(-0.5), Vec3::splat(0.5));
        for model in [
            Mat4::from_translation(Vec3::new(1000., 0., 0.)),
            Mat4::from_translation(Vec3::new(-4000., 250., 9000.)),
            Mat4::from_scale_rotation_translation(Vec3::new(0.01, 3., 100.), glam::Quat::from_rotation_y(0.7), Vec3::new(1000., -20., 5.)),
            Mat4::from_scale_rotation_translation(Vec3::splat(1e-3), glam::Quat::from_rotation_x(1.1), Vec3::new(0., 0., 2000.)),
            // A sheared placement (condition number about 100) at the origin.
            Mat4::from_cols(glam::Vec4::new(0.5, 0.49, 0., 0.), glam::Vec4::new(0.49, 0.5, 0., 0.), glam::Vec4::Z, glam::Vec4::W),
        ] {
            let (lo, hi) = placed_bounds(unit, model);
            assert!(lo.is_finite() && hi.is_finite(), "{model:?}");
            // Along each axis from the solid's centre, find the farthest point the
            // kernel holds (the held set is convex, so bisect).
            let centre = model.transform_point3(Vec3::ZERO);
            for axis in 0..3 {
                for side in [-1_f32, 1.] {
                    let at = |d: f32| {let mut q = centre; q[axis] += side * d; q};
                    let (mut held, mut out) = (0_f32, 2. * (hi[axis] - lo[axis]));
                    assert!(!kernel_holds(unit, model, at(out)));
                    loop {
                        let mid = 0.5 * (held + out);
                        if mid == held || mid == out {break;}
                        if kernel_holds(unit, model, at(mid)) {held = mid} else {out = mid}
                    }
                    let p = at(held);
                    assert!(p[axis] >= lo[axis] && p[axis] <= hi[axis], "{model:?} axis {axis} side {side}: kernel holds {p:?} outside {lo:?}..{hi:?}");
                }
            }
        }
        // Too ill-conditioned to bound (eigenvalues 1 and 2^-22): unbounded.
        let (a, b) = (0.5 + 2_f32.powi(-23), 0.5 - 2_f32.powi(-23));
        let ill = Mat4::from_cols(glam::Vec4::new(a, b, 0., 0.), glam::Vec4::new(b, a, 0., 0.), glam::Vec4::Z, glam::Vec4::W);
        assert_eq!(placed_bounds(unit, ill), UNBOUNDED);
    }

    #[test]
    fn open_solids_are_found_across_groups() {
        // A unit cube: 12 triangles over 8 corners, outward winding.
        let corners: Vec<[f32; 3]> = (0..8).map(|i| [(i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2 & 1) as f32]).collect();
        let faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let quads = |faces: &[[u32; 4]]| -> Vec<u32> {faces.iter().flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]]).collect()};
        let edges = |indices: &[u32]| open_edges(indices, |i| crate::geometry::weld_key(corners[i as usize]));
        let closed = |ranges: &[&OpenEdges]| !open_union(ranges.iter().copied());
        assert!(edges(&quads(&faces)).is_empty());
        // Split vertices (UV seams) weld by position, -0 with +0: corners 8..16 repeat 0..8.
        let seams: Vec<u32> = quads(&faces).iter().enumerate().map(|(k, &i)| if k >= 18 {i + 8} else {i}).collect();
        assert!(open_edges(&seams, |i| crate::geometry::weld_key(corners[(i % 8) as usize].map(|v| if i >= 8 && v == 0. {-0.} else {v}))).is_empty());
        // Computed seams a few ulps apart weld within the tolerance, not by bits.
        let nudged: Vec<[f32; 3]> = (0..16).map(|i| corners[i % 8].map(|v| if i >= 8 {v + 4. * f32::EPSILON} else {v})).collect();
        let welded = crate::geometry::weld_within(&nudged, 16. * f32::EPSILON);
        assert!(!open_edges(&seams, |i| crate::geometry::weld_key(nudged[i as usize])).is_empty());
        assert!(open_edges(&seams, |i| welded[i as usize]).is_empty());
        // One face missing: the open box has its four rim edges.
        assert_eq!(edges(&quads(&faces[..5])).len(), 4);
        // Material groups: the open box and its lid close each other; either alone is open.
        let (body, lid) = (edges(&quads(&faces[..5])), edges(&quads(&faces[5..])));
        assert!(closed(&[&body, &lid]) && !closed(&[&body]) && !closed(&[&lid]));
        // A lid wound the wrong way does not close it, nor does the same lid twice.
        let flipped = edges(&quads(&[[1, 5, 7, 3]]));
        assert!(!closed(&[&body, &flipped]) && !closed(&[&body, &lid, &lid]));
        // A plane stays open, also stored twice with the same winding.
        let plane = [0, 1, 3, 0, 3, 2];
        assert!(!closed(&[&edges(&plane)]) && !closed(&[&edges(&[plane, plane].concat())]) && !closed(&[&edges(&plane), &edges(&plane)]));
        // A small sheet far from its origin keeps its corners (weld_keys' tolerance).
        let far = [[100000_f32, 0., 0.], [100001., 0., 0.], [100001., 0., 1.], [100000., 0., 1.]];
        let keys = crate::geometry::weld_within(&far, 100001. * 16. * f32::EPSILON);
        assert!(!closed(&[&open_edges(&[0, 2, 1, 0, 3, 2], |i| keys[i as usize])]));
        // A triangle that welding collapses (here one stored degenerate) counts as open.
        assert!(!closed(&[&edges(&[quads(&faces), vec![0, 0, 1]].concat())]));
    }

    #[test]
    fn plain_boundary_proof_requires_disjoint_surfaces_and_equal_priorities() {
        let solid=|lo,hi|PathSolid {lo:Vec3::splat(f32::NEG_INFINITY),hi:Vec3::splat(f32::INFINITY),row:0,
            contact_bounds:(Vec3::new(lo,0.,0.),Vec3::new(hi,1.,1.)),shadow_bounds:None};
        assert!(plain_boundaries(&[],[].into_iter()));
        assert!(plain_boundaries(&[solid(0.,1.),solid(2.,3.)],[0,0].into_iter()));
        assert!(plain_boundaries(&[solid(0.,1.),solid(2.,3.)],[-2,-2].into_iter()));
        assert!(!plain_boundaries(&[solid(0.,1.),solid(2.,3.)],[0,1].into_iter()));
        assert!(!plain_boundaries(&[solid(0.,1.),solid(1.,2.)],[0,0].into_iter()));
        assert!(!plain_boundaries(&[solid(0.,2.),solid(1.,3.)],[0,0].into_iter()));
        assert!(!plain_boundaries(&[solid(f32::NEG_INFINITY,f32::INFINITY)],[0].into_iter()));
        let mut scene=crate::slang_forward::SlangScene::default();
        let key=|s:&crate::slang_forward::SlangScene|crate::pipelines::CachedPipelines::identity("","",1,true,s,wgpu::TextureFormat::Rgba8Unorm);
        let ordinary=key(&scene);scene.no_boundary_transactions=true;
        assert_ne!(ordinary,key(&scene),"the proof must separate persistent pipeline identities");
    }

    #[test]
    fn scene_size_only_selects_the_inline_storage_class() {
        // Larger scenes share one private layout; actual deeper nesting uses
        // device storage and is covered by the dielectric render contracts.
        assert_eq!(kernel_boundaries(0),0);
        assert_eq!(kernel_boundaries(3),3);
        assert_eq!(kernel_boundaries(INLINE_BOUNDARIES),INLINE_BOUNDARIES);
        assert_eq!(kernel_boundaries(62),INLINE_BOUNDARIES);
        assert_eq!(kernel_boundaries(u32::MAX),INLINE_BOUNDARIES);
    }

    #[test]
    fn dielectric_storage_bounds_disjoint_nested_and_touching_solids() {
        let b=|lo:f32,hi:f32|(Vec3::new(lo,-1.,-1.),Vec3::new(hi,1.,1.));
        assert_eq!(overlap_capacity(&[]),0);
        assert_eq!(overlap_capacity(&[b(0.,1.),b(2.,3.),b(4.,5.)]),1);
        assert_eq!(overlap_capacity(&[b(0.,5.),b(1.,4.),b(2.,3.)]),3);
        assert_eq!(overlap_capacity(&[b(0.,1.),b(1.,2.)]),2);
        assert_eq!(overlap_capacity(&[b(0.,1.),b(2.,3.),(Vec3::splat(f32::NEG_INFINITY),Vec3::splat(f32::INFINITY))]),2);
        // Choose a separating axis rather than summing mutually disjoint solids.
        let y=|lo:f32,hi:f32|(Vec3::new(-1.,lo,-1.),Vec3::new(1.,hi,1.));
        assert_eq!(overlap_capacity(&[y(0.,1.),y(2.,3.)]),1);
        // Empty boxes (degenerate transforms) bound nothing and never underflow.
        let empty=(Vec3::splat(f32::INFINITY),Vec3::splat(f32::NEG_INFINITY));
        assert_eq!(overlap_capacity(&[empty]),0);
        assert_eq!(overlap_capacity(&[empty,empty,b(0.,1.)]),1);
        assert_eq!(overlap_capacity(&[b(0.,5.),empty,b(1.,4.),empty]),2);
        // Every axis has two boxes overlapping, but no two boxes meet.
        let xy=|x:f32,y:f32|(Vec3::new(x,y,0.),Vec3::new(x+1.,y+1.,1.));
        assert_eq!(overlap_capacity(&[xy(0.,0.),xy(0.,2.),xy(2.,0.)]),1);
        assert_eq!(overlap_capacity(&[xy(0.,0.),xy(0.5,0.5),xy(2.,0.)]),2);
    }

    #[test]
    fn retained_pipeline_and_optional_moments() -> Result<()> {
        let Some(r) = crate::test_renderer()? else {
            return Ok(());
        };
        if !r
                .device
                .features()
                .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        {
            return Ok(());
        }
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        // Compare raw path specialization and adaptive moments independently of OIDN.
        let mut physical = Vec::new();
        for adaptive in [false, true] {
            let mut scene: serde_json::Value =
                vork_scene::v2::with_test_settings(include_str!("../../tests/fixtures/analytic.json"), include_str!("../../tests/presets/analytic-7fefd52a.json"))?;
            scene["render"]["settings"]["samples"] = 2.into();
            scene["nodes"]
                .as_array_mut()
                .unwrap()
                .extend((0..3).map(|i| {
                    serde_json::json!({
                        "type":"light", "name":format!("light-{i}"), "shape":"sun", "energy":i+1
                    })
                }));
            vork_scene::v2::replace_integrator(&mut scene, serde_json::json!({
                "integrator":"path",  "denoiser":null
            }));
            let mut s = r.install(vork_scene::v2::from_test_value(scene)?, &base)?;
            // Exercise both internal specializations without restoring an authored knob.
            let crate::pipelines::ProgramKernel::Path { random_walk, features, boundaries, emitter_profiles, media, .. } =
                s.path.as_ref().unwrap().kernel else { unreachable!() };
            assert_eq!(features & crate::forward_variant::FEATURE_IMPLICIT_GEOMETRY, 0, "triangle-only scene retains implicit graphs");
            s.path = Some(Pipeline::new(&r, adaptive, false, random_walk, features, boundaries,
                emitter_profiles, media, &crate::InstallOptions::default(), &s.pipelines)?);
            assert!(matches!(s.path.as_ref().unwrap().kernel, crate::pipelines::ProgramKernel::Path { media: None, .. }), "no-medium paths installed medium state/code");
            let rays = s.rays.as_ref().expect("path rays");
            assert_eq!(rays.film_filter, rays.boundaries,
                "no-solid paths installed an unused boundary descriptor table");
            assert_eq!(rays.alpha.size(), u64::from(rays.film_filter)*4
                + std::mem::size_of::<crate::sampling::PathFilterTable>() as u64);
            assert!(s.checked_shader_capacity, "display output must retain finite-result diagnostics without solids");
            assert!(matches!(s.path.as_ref().unwrap().kernel,
                crate::pipelines::ProgramKernel::Path { boundaries: 0, .. }),
                "no-solid paths installed camera containment work");
            let mut a = s.create_frame_slot(&r, crate::ReadbackPrecision::Float)?;
            // Attribute only slot work to the count, after install variants settle.
            r.compiler.settle()?;
            let count = r.compiler.created();
            let mut b = s.create_frame_slot(&r, crate::ReadbackPrecision::Float)?;
            r.compiler.settle()?;
            assert_eq!(
                count,
                r.compiler.created(),
                "creating a second slot compiled a pipeline"
            );
            a.submit(&s, &r, 0.)?;
            let first = a.collect(&r)?;
            b.submit(&s, &r, 0.)?;
            let second = b.collect(&r)?;
            crate::test_linear::assert_linear_eq(first.linear_rgba.as_ref().expect("linear readback"), second.linear_rgba.as_ref().expect("linear readback"));
            // Same transport through the conservative fallback must preserve the
            // installed specialization's image, for both sampling entries.
            s.path = Some(Pipeline::new(&r, adaptive, false, false, 0, 0, false, None, &crate::InstallOptions::default(), &s.pipelines)?);
            b.submit(&s, &r, 0.)?;
            let generic = b.collect(&r)?;
            crate::test_linear::assert_linear_eq(first.linear_rgba.as_ref().expect("linear readback"), generic.linear_rgba.as_ref().expect("linear readback"));
            physical.push(s.transient_bytes().1);
        }
        assert_eq!(physical[1] - physical[0], 96 * 64 * constants::PATH_MOMENT_BYTES);
        Ok(())
    }
}
