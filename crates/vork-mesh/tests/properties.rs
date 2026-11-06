//! Property tests for subdivision plans over generated polygon cages.
//! `PROPTEST_CASES` overrides the per-property case counts below.
use glam::{Mat3, Quat, Vec3};
use proptest::{prelude::*, sample::Index};
use std::collections::BTreeMap;
use vork_mesh::Plan;

/// A polygon cage: positions, flattened corner indices and corners per face.
#[derive(Clone, Debug)]
struct Cage {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    sizes: Vec<u32>,
    closed: bool,
}

impl Cage {
    fn faces(&self) -> Vec<&[u32]> {
        let mut offset = 0;
        self.sizes
            .iter()
            .map(|&n| {
                offset += n as usize;
                &self.indices[offset - n as usize..offset]
            })
            .collect()
    }
    fn edges(&self) -> Vec<[u32; 2]> {
        let mut edges: Vec<_> = self
            .faces()
            .iter()
            .flat_map(|f| (0..f.len()).map(|i| (f[i], f[(i + 1) % f.len()])))
            .map(|(a, b)| [a.min(b), a.max(b)])
            .collect();
        edges.sort();
        edges.dedup();
        edges
    }
    fn boundary_edges(&self) -> usize {
        let mut uses = BTreeMap::new();
        for f in self.faces() {
            for i in 0..f.len() {
                let (a, b) = (f[i], f[(i + 1) % f.len()]);
                *uses.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        uses.values().filter(|&&n| n == 1).count()
    }
    fn plan(&self, creases: &[([u32; 2], f32)], levels: u32, simple: bool, limit: bool) -> Plan {
        Plan::new(
            self.positions.len(),
            &self.indices,
            Some(&self.sizes),
            creases,
            levels,
            simple,
            limit,
        )
        .expect("manifold cage must plan")
    }
}

/// Closed prism over a regular `n`-gon: two caps and `n` side quads, wound outward.
fn prism(n: u32) -> Cage {
    let mut positions = Vec::new();
    for y in [-1., 1.] {
        for i in 0..n {
            let a = i as f32 * std::f32::consts::TAU / n as f32;
            positions.push([a.cos(), y, -a.sin()]);
        }
    }
    let mut indices: Vec<u32> = (0..n).rev().collect();
    indices.extend(n..2 * n);
    let mut sizes = vec![n, n];
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j, n + j, n + i]);
        sizes.push(4);
    }
    Cage {
        positions,
        indices,
        sizes,
        closed: true,
    }
}

/// Closed genus-one quad grid (`m` around the axis, `n` around the tube).
fn torus(m: u32, n: u32) -> Cage {
    let mut positions = Vec::new();
    for i in 0..m {
        for j in 0..n {
            let (a, b) = (
                i as f32 * std::f32::consts::TAU / m as f32,
                j as f32 * std::f32::consts::TAU / n as f32,
            );
            let r = 2. + 0.75 * b.cos();
            positions.push([r * a.cos(), 0.75 * b.sin(), r * a.sin()]);
        }
    }
    let id = |i: u32, j: u32| (i % m) * n + j % n;
    let mut indices = Vec::new();
    for i in 0..m {
        for j in 0..n {
            indices.extend([id(i, j), id(i, j + 1), id(i + 1, j + 1), id(i + 1, j)]);
        }
    }
    Cage {
        positions,
        indices,
        sizes: vec![4; (m * n) as usize],
        closed: true,
    }
}

/// Open planar `m` by `n` quad grid in XZ.
fn grid(m: u32, n: u32) -> Cage {
    let mut positions = Vec::new();
    for i in 0..=m {
        for j in 0..=n {
            positions.push([i as f32, 0., j as f32]);
        }
    }
    let id = |i: u32, j: u32| i * (n + 1) + j;
    let mut indices = Vec::new();
    for i in 0..m {
        for j in 0..n {
            indices.extend([id(i, j), id(i, j + 1), id(i + 1, j + 1), id(i + 1, j)]);
        }
    }
    Cage {
        positions,
        indices,
        sizes: vec![4; (m * n) as usize],
        closed: false,
    }
}

fn cage() -> impl Strategy<Value = Cage> {
    prop_oneof![
        (3u32..=8).prop_map(prism),
        (3u32..=6, 3u32..=6).prop_map(|(m, n)| torus(m, n)),
        (1u32..=4, 1u32..=4).prop_map(|(m, n)| grid(m, n)),
    ]
}

/// Moves every control point independently, keeping the topology.
fn jittered(cage: Cage) -> impl Strategy<Value = Cage> {
    let count = cage.positions.len();
    prop::collection::vec(prop::array::uniform3(-0.4f32..0.4), count).prop_map(move |offsets| {
        let mut cage = cage.clone();
        for (p, o) in cage.positions.iter_mut().zip(offsets) {
            for axis in 0..3 {
                p[axis] += o[axis];
            }
        }
        cage
    })
}

/// A subset of cage edges with OpenSubdiv/USD sharpness, including fully sharp ones.
fn creased(cage: Cage) -> impl Strategy<Value = (Cage, Vec<([u32; 2], f32)>)> {
    let edges = cage.edges();
    let weights = prop_oneof![Just(10f32), 0f32..=10.];
    prop::collection::vec(prop::option::weighted(0.3, weights), edges.len()).prop_map(
        move |chosen| {
            let creases = edges
                .iter()
                .zip(chosen)
                .filter_map(|(&e, w)| w.map(|w| (e, w)))
                .collect();
            (cage.clone(), creases)
        },
    )
}

fn rigid() -> impl Strategy<Value = (Quat, Vec3)> {
    (
        prop::array::uniform3(-1f32..1.),
        -std::f32::consts::PI..std::f32::consts::PI,
        prop::array::uniform3(-10f32..10.),
    )
        .prop_map(|(axis, angle, t)| {
            let axis = Vec3::from(axis).try_normalize().unwrap_or(Vec3::Y);
            (Quat::from_axis_angle(axis, angle), Vec3::from(t))
        })
}

fn bounds(points: &[[f32; 3]]) -> (Vec3, Vec3) {
    points.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))),
    )
}

/// Directed triangle edges keyed by undirected endpoints.
fn edge_uses(indices: &[u32]) -> BTreeMap<(u32, u32), Vec<(u32, u32)>> {
    let mut uses: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for t in indices.chunks_exact(3) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            uses.entry((a.min(b), a.max(b))).or_default().push((a, b));
        }
    }
    uses
}

/// Accepted plans weight only existing control points and index only planned vertices.
fn check_references(plan: &Plan, count: usize) -> Result<(), TestCaseError> {
    let stencils = plan.stencils.iter().flatten();
    prop_assert!(
        stencils
            .clone()
            .all(|&(i, w)| (i as usize) < count && w.is_finite())
    );
    prop_assert!(
        plan.indices
            .iter()
            .all(|&i| (i as usize) < plan.stencils.len())
    );
    prop_assert_eq!(plan.primitive_ids.len() * 3, plan.indices.len());
    Ok(())
}

fn signed_volume(positions: &[[f32; 3]], indices: &[u32]) -> f32 {
    indices
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(positions[t[k] as usize]));
            a.dot(b.cross(c)) / 6.
        })
        .sum()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Uniform refinement follows V' = V + E + F, splits every n-gon into n quads, keeps the
    /// Euler characteristic, and keeps closed cages closed and consistently oriented.
    #[test]
    fn refinement_topology(cage in cage(), levels in 0u32..=3, simple: bool, limit: bool) {
        let plan = cage.plan(&[], levels, simple, limit);
        let faces = cage.faces();
        let (mut v, mut e) = (cage.positions.len(), cage.edges().len());
        let mut f = faces.len();
        let mut corners: usize = cage.sizes.iter().map(|&n| n as usize).sum();
        for _ in 0..levels {
            (v, e, f) = (v + e + f, 2 * e + corners, corners);
            corners = 4 * f;
        }
        let triangles = if levels == 0 {
            cage.sizes.iter().map(|&n| n as usize - 2).sum()
        } else {
            2 * f
        };
        prop_assert_eq!(plan.stencils.len(), v);
        prop_assert_eq!(plan.indices.len(), 3 * triangles);
        prop_assert_eq!(plan.primitive_ids.len(), triangles);
        prop_assert!(plan.indices.iter().all(|&i| (i as usize) < v));
        for (ordinal, face) in faces.iter().enumerate() {
            let expected = if levels == 0 {
                face.len() - 2
            } else {
                2 * face.len() * 4usize.pow(levels - 1)
            };
            prop_assert_eq!(
                plan.primitive_ids.iter().filter(|&&id| id as usize == ordinal).count(),
                expected
            );
        }
        let uses = edge_uses(&plan.indices);
        let chi = |v: usize, e: usize, f: usize| v as i64 - e as i64 + f as i64;
        prop_assert_eq!(
            chi(v, uses.len(), triangles),
            chi(cage.positions.len(), cage.edges().len(), faces.len())
        );
        let boundary = uses.values().filter(|u| u.len() == 1).count();
        prop_assert!(uses.values().all(|u| u.len() <= 2 && (u.len() < 2 || u[0] != u[1])));
        // Each refinement splits every boundary edge in two.
        prop_assert_eq!(boundary, cage.boundary_edges() << levels);
        prop_assert_eq!(boundary == 0, cage.closed);
    }

    /// Stencils are convex combinations of control points: weights are nonnegative and
    /// sum to one, so every refined or limit point is finite and inside the cage bounds.
    #[test]
    fn stencils_stay_in_cage_hull(
        (cage, creases) in cage().prop_flat_map(jittered).prop_flat_map(creased),
        levels in 1u32..=3,
        simple: bool,
        limit: bool,
    ) {
        let plan = cage.plan(&creases, levels, simple, limit);
        for stencil in &plan.stencils {
            let count = cage.positions.len();
            prop_assert!(stencil.iter().all(|&(i, w)| (i as usize) < count && w >= -1e-6));
            let sum: f32 = stencil.iter().map(|s| s.1).sum();
            prop_assert!((sum - 1.).abs() < 1e-5, "stencil weights sum to {}", sum);
        }
        let (lo, hi) = bounds(&cage.positions);
        let slack = 1e-5 * (hi - lo).max_element().max(1.);
        for p in plan.evaluate(&cage.positions) {
            let p = Vec3::from(p);
            prop_assert!(p.is_finite());
            let inside = p.cmpge(lo - slack).all() && p.cmple(hi + slack).all();
            prop_assert!(inside, "{} outside {}..{}", p, lo, hi);
        }
    }

    /// Evaluating a rigidly moved cage equals moving the evaluated surface.
    #[test]
    fn evaluation_commutes_with_rigid_transforms(
        (cage, creases) in cage().prop_flat_map(jittered).prop_flat_map(creased),
        levels in 1u32..=3,
        simple: bool,
        limit: bool,
        (rotation, translation) in rigid(),
    ) {
        let plan = cage.plan(&creases, levels, simple, limit);
        let moved: Vec<_> = cage
            .positions
            .iter()
            .map(|&p| (rotation * Vec3::from(p) + translation).to_array())
            .collect();
        for (a, b) in plan.evaluate(&moved).iter().zip(plan.evaluate(&cage.positions)) {
            let expected = rotation * Vec3::from(b) + translation;
            prop_assert!((Vec3::from(*a) - expected).length() < 1e-4, "{:?} != {}", a, expected);
        }
    }

    /// On affinely transformed convex prisms, generated normals are unit length and point
    /// away from the cage centroid, and the refined surface keeps positive volume.
    #[test]
    fn convex_cage_normals_point_outward(
        n in 3u32..=8,
        levels in 1u32..=3,
        simple: bool,
        limit: bool,
        scale in prop::array::uniform3(0.2f32..5.),
        (rotation, translation) in rigid(),
    ) {
        let mut cage = prism(n);
        let transform = Mat3::from_quat(rotation) * Mat3::from_diagonal(Vec3::from(scale));
        for p in &mut cage.positions {
            *p = (transform * Vec3::from(*p) + translation).to_array();
        }
        let plan = cage.plan(&[], levels, simple, limit);
        let positions = plan.evaluate(&cage.positions);
        prop_assert!(signed_volume(&positions, &plan.indices) > 0.);
        for (p, normal) in positions.iter().zip(plan.normals(&positions)) {
            let normal = Vec3::from(normal);
            prop_assert!(normal.is_finite() && (normal.length() - 1.).abs() < 1e-4);
            let outward = normal.dot(Vec3::from(*p) - translation);
            prop_assert!(outward > 0., "inward normal {} at {:?}", normal, p);
        }
    }

    /// Valid cages with flipped, dropped, duplicated or welded faces are rejected or
    /// planned without panicking, and accepted plans keep finite, in-range stencils.
    #[test]
    fn damaged_cages_never_panic(
        cage in cage(),
        edits in prop::collection::vec((0u8..4, any::<Index>(), any::<Index>()), 1..4),
        levels in 0u32..=3,
        simple: bool,
        limit: bool,
    ) {
        let mut faces: Vec<Vec<u32>> = cage.faces().iter().map(|f| f.to_vec()).collect();
        let count = cage.positions.len();
        for (kind, a, b) in edits {
            if faces.is_empty() {
                break;
            }
            let f = a.index(faces.len());
            match kind {
                0 => faces[f].reverse(),
                1 => drop(faces.remove(f)),
                2 => faces.push(faces[f].clone()),
                _ => {
                    // Weld one vertex onto another everywhere it is used.
                    let (from, to) = (a.index(count) as u32, b.index(count) as u32);
                    for v in faces.iter_mut().flatten() {
                        if *v == from {
                            *v = to;
                        }
                    }
                }
            }
        }
        let indices: Vec<u32> = faces.iter().flatten().copied().collect();
        let sizes: Vec<u32> = faces.iter().map(|f| f.len() as u32).collect();
        if let Ok(plan) = Plan::new(count, &indices, Some(&sizes), &[], levels, simple, limit) {
            check_references(&plan, count)?;
            prop_assert!(plan.primitive_ids.iter().all(|&id| (id as usize) < faces.len()));
        }
    }

    /// Arbitrary, mostly invalid topology is rejected or planned, never a panic, and any
    /// accepted plan only references existing control points.
    #[test]
    fn arbitrary_topology_never_panics(
        count in 1usize..12,
        faces in prop::collection::vec(prop::collection::vec(0u32..14, 1..7), 0..8),
        creases in prop::collection::vec((prop::array::uniform2(0u32..14), -0.5f32..15.), 0..4),
        levels in 0u32..=3,
        simple: bool,
        limit: bool,
        triangles: bool,
    ) {
        let indices: Vec<u32> = faces.iter().flatten().copied().collect();
        let sizes: Vec<u32> = faces.iter().map(|f| f.len() as u32).collect();
        let sizes = (!triangles).then_some(sizes.as_slice());
        if let Ok(plan) = Plan::new(count, &indices, sizes, &creases, levels, simple, limit) {
            check_references(&plan, count)?;
        }
    }
}
