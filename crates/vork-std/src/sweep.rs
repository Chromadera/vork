//! Sweep mesh construction, independent of the authoring option contract.
use crate::text::{Mesh, fill};
use anyhow::{Context, Result, ensure};
use glam::{DQuat, DVec2, DVec3};
use lyon_path::math::point;
use serde::Deserialize;

fn scale() -> serde_json::Value {
    serde_json::json!(1)
}
fn name() -> String {
    "sweep".into()
}
fn caps() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default = "name")]
    name: String,
    profile: Vec<[f32; 2]>,
    path: Vec<[f32; 3]>,
    #[serde(default = "scale")]
    scale: serde_json::Value,
    #[serde(default)]
    twist: f64,
    #[serde(default = "caps")]
    caps: bool,
}

pub(crate) fn evaluate(argument: &str) -> Result<String> {
    let o: Options = serde_json::from_str(argument)?;
    ensure!(
        !o.name.is_empty() && !o.name.contains('/'),
        "sweep name must be nonempty without /; choose a simple name"
    );
    ensure!(o.twist.is_finite(), "twist must be finite; supply radians");
    // Parse directly from JSON numbers: serde's untagged Content representation does
    // not preserve arbitrary-precision decimal numbers in a number-or-array union.
    let scales = if let Some(s) = o.scale.as_f64() {
        vec![s; o.path.len()]
    } else {
        let values = o
            .scale
            .as_array()
            .context("scale must be a number or number array; supply a positive multiplier")?;
        ensure!(
            values.len() == o.path.len(),
            "scale needs one value per path point; match the path length or supply a number"
        );
        values
            .iter()
            .map(|s| {
                s.as_f64()
                    .context("scale entries must be numbers; supply positive multipliers")
            })
            .collect::<Result<Vec<_>>>()?
    };
    ensure!(
        scales.iter().all(|s| s.is_finite() && *s > 0.),
        "scale must be finite and positive; supply values greater than zero"
    );
    let mut profile: Vec<_> = o
        .profile
        .into_iter()
        .map(|p| DVec2::from_array(p.map(f64::from)))
        .collect();
    profile.dedup();
    let section_closed = profile.len() > 2 && profile.first() == profile.last();
    if section_closed {
        profile.pop();
    }
    let mut path = Vec::new();
    let mut scale = Vec::new();
    for (p, s) in o.path.into_iter().zip(scales) {
        let p = DVec3::from_array(p.map(f64::from));
        if path.last() != Some(&p) {
            path.push(p);
            scale.push(s);
        }
    }
    let closed = path.len() > 2 && path.first() == path.last();
    if closed {
        ensure!(
            scale.first() == scale.last(),
            "a closed path needs matching first and last scale; make the endpoint scales equal"
        );
        let turns = o.twist / std::f64::consts::TAU;
        ensure!(
            (turns - turns.round()).abs() <= f64::from(f32::EPSILON) * turns.abs().max(1.),
            "a closed path needs whole turns of twist; use a multiple of 2*pi radians"
        );
        path.pop();
        scale.pop();
    }
    let steps = if closed {
        path.len()
    } else {
        path.len().saturating_sub(1)
    };
    let mut length = vec![0.];
    for i in 0..steps {
        length.push(length.last().unwrap() + path[i].distance(path[(i + 1) % path.len()]));
    }
    let total = length.last().copied().unwrap();
    let twist: Vec<_> = (0..path.len())
        .map(|i| {
            if total > 0. {
                o.twist * length[i] / total
            } else {
                0.
            }
        })
        .collect();
    let mesh = mesh(
        &profile,
        &path,
        &scale,
        &twist,
        o.caps,
        closed,
        section_closed,
    )?;
    Ok(serde_json::to_string(&mesh.fragment(&o.name))?)
}

/// Parallel-transport frames avoid the Frenet frame's undefined orientation on straight paths.
struct Frame {
    origin: DVec3,
    x: DVec3,
    y: DVec3,
}
impl Frame {
    fn position(&self, p: DVec2) -> [f32; 3] {
        (self.origin + self.x * p.x + self.y * p.y)
            .as_vec3()
            .to_array()
    }
}

fn transport(x: DVec3, a: DVec3, b: DVec3) -> Result<DVec3> {
    ensure!(
        a.dot(b) > -1.,
        "path must not reverse direction at a vertex; remove the reversal"
    );
    let x = DQuat::from_rotation_arc(a, b) * x;
    Ok((x - b * x.dot(b)).normalize())
}

/// Internal numeric input. The public API adapter is deliberately separate.
fn mesh(
    profile: &[DVec2],
    path: &[DVec3],
    scale: &[f64],
    twist: &[f64],
    caps: bool,
    closed: bool,
    section_closed: bool,
) -> Result<Mesh> {
    ensure!(
        profile.len() >= if section_closed { 3 } else { 2 },
        "profile needs at least two distinct points; closed profiles need three"
    );
    ensure!(
        path.len() >= if closed { 3 } else { 2 },
        "path needs distinct points"
    );
    ensure!(
        scale.len() == path.len() && twist.len() == path.len(),
        "scale and twist must have one value per path point"
    );
    ensure!(
        profile.iter().all(|p| p.is_finite()) && path.iter().all(|p| p.is_finite()),
        "profile and path points must be finite"
    );
    ensure!(
        scale.iter().all(|s| s.is_finite() && *s > 0.) && twist.iter().all(|t| t.is_finite()),
        "scale must be positive and finite; twist must be finite"
    );
    let n = profile.len();
    let m = path.len();
    let area: f64 = (0..n)
        .map(|i| (profile[i] - profile[0]).perp_dot(profile[(i + 1) % n] - profile[0]))
        .sum();
    ensure!(
        !section_closed || area != 0.,
        "closed profile must enclose an area; use a nondegenerate polygon"
    );
    let edges = if section_closed { n } else { n - 1 };
    // Reject crossing and overlapping edges before tessellation. No profile-specific tolerances.
    let orient = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    let on = |a: DVec2, b: DVec2, p: DVec2| orient(a, b, p) == 0. && (p - a).dot(p - b) <= 0.;
    for i in 0..edges {
        let a = profile[i];
        let b = profile[(i + 1) % n];
        ensure!(
            a != b,
            "profile must not contain consecutive repeated points"
        );
        let incoming = a - profile[(i + n - 1) % n];
        ensure!(
            (!section_closed && i == 0)
                || incoming.perp_dot(b - a) != 0.
                || incoming.dot(b - a) > 0.,
            "profile must not reverse direction at a vertex"
        );
        for j in i + 1..if section_closed { edges } else { 0 } {
            if j == i + 1 || (section_closed && i == 0 && j == n - 1) {
                continue;
            }
            let c = profile[j];
            let d = profile[(j + 1) % n];
            let crosses =
                orient(a, b, c) * orient(a, b, d) < 0. && orient(c, d, a) * orient(c, d, b) < 0.;
            ensure!(
                !crosses && !on(a, b, c) && !on(a, b, d) && !on(c, d, a) && !on(c, d, b),
                "profile edges must not cross or touch"
            );
        }
    }
    let steps = if closed { m } else { m - 1 };
    let mut direction = Vec::with_capacity(steps);
    let mut distance = vec![0.];
    for i in 0..steps {
        let d = path[(i + 1) % m] - path[i];
        ensure!(
            d.length() > 0.,
            "path must not contain consecutive repeated points"
        );
        direction.push(d.normalize());
        distance.push(distance.last().unwrap() + d.length());
    }
    let mut tangent = Vec::with_capacity(m);
    for i in 0..m {
        let a = if i > 0 {
            direction[i - 1]
        } else if closed {
            direction[steps - 1]
        } else {
            direction[0]
        };
        let b = if i < steps {
            direction[i]
        } else {
            direction[steps - 1]
        };
        ensure!(
            (a + b).length_squared() > 0.,
            "path must not reverse direction at a vertex; remove the reversal"
        );
        tangent.push((a + b).normalize());
    }
    // Project the least-aligned world axis onto the initial normal plane.
    let axis = [DVec3::Y, DVec3::Z, DVec3::X]
        .into_iter()
        .min_by(|a, b| a.dot(tangent[0]).abs().total_cmp(&b.dot(tangent[0]).abs()))
        .unwrap();
    let up = (axis - tangent[0] * axis.dot(tangent[0])).normalize();
    let initial = tangent[0].cross(up);
    let mut axes = vec![initial];
    for i in 1..m {
        axes.push(transport(axes[i - 1], tangent[i - 1], tangent[i])?);
    }
    // Distribute the transport holonomy by arc length so a closed path has no frame seam.
    let correction = if closed {
        let returned = transport(axes[m - 1], tangent[m - 1], tangent[0])?;
        tangent[0]
            .dot(returned.cross(initial))
            .atan2(returned.dot(initial))
    } else {
        0.
    };
    let frames: Vec<_> = (0..m)
        .map(|i| {
            let angle = twist[i] + correction * distance[i] / distance[steps];
            let x = DQuat::from_axis_angle(tangent[i], angle) * axes[i];
            Frame {
                origin: path[i],
                x: x * scale[i],
                y: x.cross(tangent[i]) * scale[i],
            }
        })
        .collect();
    // Cache each ring once: normal/UV duplicates must have bit-identical positions.
    let rings: Vec<Vec<_>> = frames
        .iter()
        .map(|f| profile.iter().map(|&p| f.position(p)).collect())
        .collect();
    ensure!(
        rings.iter().flatten().flatten().all(|v| v.is_finite()),
        "sweep positions exceed finite geometry range"
    );
    let mut perimeter = vec![0.];
    for i in 0..edges {
        perimeter.push(perimeter.last().unwrap() + profile[i].distance(profile[(i + 1) % n]));
    }
    let rows = steps * 2;
    let mut mesh = Mesh::default();
    // Each profile edge has its own normal chart. Accumulate actual triangle normals,
    // rather than rotating 2D normals: scale and twist also change longitudinal slopes.
    for e in 0..edges {
        let start = mesh.p.len() as u32;
        for r in 0..rows {
            let ring = r / 2 + r % 2;
            for p in [e, (e + 1) % n] {
                let u = if p == e {
                    perimeter[e]
                } else {
                    perimeter[e + 1]
                } / perimeter[edges];
                mesh.vertex_uv(
                    rings[ring % m][p],
                    [0.; 3],
                    [u as f32, (distance[ring] / distance[steps]) as f32],
                );
            }
        }
        for r in 0..steps {
            let a = start + 4 * r as u32;
            let b = a + 2;
            let triangles = if !section_closed || area > 0. {
                [[a, b + 1, a + 1], [a, b, b + 1]]
            } else {
                [[a, a + 1, b + 1], [a, b + 1, b]]
            };
            for t in triangles {
                mesh.i.extend(t);
            }
        }
    }
    let mut normals = vec![DVec3::ZERO; mesh.p.len()];
    for t in mesh.i.chunks_exact(3) {
        let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
        let p = |i: usize| DVec3::from_array(mesh.p[i].map(f64::from));
        let normal = (p(b) - p(a)).cross(p(c) - p(a));
        ensure!(
            normal.length_squared() > 0.,
            "sweep has collapsed triangles; adjust profile, path or scale"
        );
        for &i in &[a, b, c] {
            normals[i] += normal;
        }
    }
    // Match std's Smooth by Angle convention at profile corners. Keep duplicated
    // UV/chart vertices, but share their accumulated normal on smooth corners.
    for e in 0..n {
        if !section_closed && (e == 0 || e == n - 1) {
            continue;
        }
        let incoming = (profile[e] - profile[(e + n - 1) % n]).normalize();
        let outgoing = (profile[(e + 1) % n] - profile[e]).normalize();
        if incoming.dot(outgoing) >= (30f64.to_radians()).cos() {
            for r in 0..rows {
                let a = ((e + n - 1) % n) * rows * 2 + r * 2 + 1;
                let b = e * rows * 2 + r * 2;
                let sum = normals[a] + normals[b];
                normals[a] = sum;
                normals[b] = sum;
            }
        }
    }
    // Longitudinal corners use the same angle rule as profile corners.
    for r in 0..m {
        if !closed && (r == 0 || r == m - 1) {
            continue;
        }
        let previous = (r + steps - 1) % steps;
        if direction[previous].dot(direction[r]) >= (30f64.to_radians()).cos() {
            for e in 0..edges {
                for k in 0..2 {
                    let a = e * rows * 2 + previous * 4 + 2 + k;
                    let b = e * rows * 2 + r * 4 + k;
                    let sum = normals[a] + normals[b];
                    normals[a] = sum;
                    normals[b] = sum;
                }
            }
        }
    }
    for (out, normal) in mesh.n.iter_mut().zip(normals) {
        ensure!(
            normal.length_squared() > 0.,
            "sweep normals cancel; adjust path to avoid folds"
        );
        *out = normal.normalize().as_vec3().to_array();
    }
    if caps && !closed && section_closed {
        let contour = profile
            .iter()
            .map(|p| point(p.x as f32, p.y as f32))
            .collect();
        let tessellation = fill(&[contour], f32::EPSILON)?;
        let lo = profile.iter().copied().reduce(DVec2::min).unwrap();
        let hi = profile.iter().copied().reduce(DVec2::max).unwrap();
        for (r, sign) in [(0, -1.), (m - 1, 1.)] {
            let base = mesh.p.len() as u32;
            let normal = (tangent[r] * sign).as_vec3().to_array();
            for p in &tessellation.vertices {
                let p = DVec2::new(f64::from(p.x), f64::from(p.y));
                // Use cached ring positions at the cap boundary for exact welding.
                let position = profile
                    .iter()
                    .position(|&q| q == p)
                    .map(|i| rings[r][i])
                    .unwrap_or_else(|| frames[r].position(p));
                let uv = (p - lo) / (hi - lo);
                mesh.vertex_uv(
                    position,
                    normal,
                    [
                        if sign > 0. {
                            uv.x as f32
                        } else {
                            1. - uv.x as f32
                        },
                        1. - uv.y as f32,
                    ],
                );
            }
            for t in tessellation.indices.chunks_exact(3) {
                mesh.tri(base + t[0], base + t[1], base + t[2]);
            }
        }
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    fn square() -> Vec<DVec2> {
        vec![
            DVec2::new(-1., -1.),
            DVec2::new(1., -1.),
            DVec2::new(1., 1.),
            DVec2::new(-1., 1.),
        ]
    }
    fn check(mesh: &Mesh, closed: bool) {
        let p = |i: u32| DVec3::from_array(mesh.p[i as usize].map(f64::from));
        let mut edges = BTreeMap::new();
        let mut volume = 0.;
        for t in mesh.i.chunks_exact(3) {
            let face = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            assert!(face.length() > 0.);
            volume += p(t[0]).dot(p(t[1]).cross(p(t[2]))) / 6.;
            for &i in t {
                let normal = DVec3::from_array(mesh.n[i as usize].map(f64::from));
                assert!((normal.length() - 1.).abs() < 1e-6);
                assert!(
                    normal.dot(face) > 0.,
                    "normal {normal:?} disagrees with face {face:?}"
                );
            }
            for j in 0..3 {
                let key = |i| mesh.p[i as usize].map(|v| if v == 0. { 0 } else { v.to_bits() });
                let a = key(t[j]);
                let b = key(t[(j + 1) % 3]);
                let (edge, sign) = if a < b { ((a, b), 1i32) } else { ((b, a), -1) };
                let entry = edges.entry(edge).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += sign;
            }
        }
        for (count, direction) in edges.values() {
            if closed {
                assert_eq!((*count, *direction), (2, 0));
            } else {
                assert!(*count == 1 || (*count == 2 && *direction == 0));
            }
        }
        if closed {
            assert!(volume > 0.);
        }
        assert!(mesh.uv.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn prism_counts_volume_caps_and_winding() {
        for reversed in [false, true] {
            let mut profile = square();
            if reversed {
                profile.reverse();
            }
            let path = [DVec3::ZERO, DVec3::Z * 3.];
            for caps in [false, true] {
                let m = mesh(&profile, &path, &[1.; 2], &[0.; 2], caps, false, true).unwrap();
                assert_eq!(m.p.len(), if caps { 24 } else { 16 });
                assert_eq!(m.i.len(), if caps { 36 } else { 24 });
                check(&m, caps);
                if caps {
                    for normal in &m.n[16..20] {
                        assert_eq!(*normal, [0., 0., -1.]);
                    }
                    for normal in &m.n[20..24] {
                        assert_eq!(*normal, [0., 0., 1.]);
                    }
                }
            }
        }
    }

    #[test]
    fn concave_caps_and_collinear_edges_weld_exactly() {
        for profile in [
            vec![[0., 0.], [2., 0.], [2., 1.], [1., 1.], [1., 2.], [0., 2.]],
            vec![[0., 0.], [1., 0.], [2., 0.], [2., 2.], [0., 2.]],
        ] {
            let profile: Vec<_> = profile.into_iter().map(DVec2::from_array).collect();
            check(
                &mesh(
                    &profile,
                    &[DVec3::ZERO, DVec3::Z],
                    &[1.; 2],
                    &[0.; 2],
                    true,
                    false,
                    true,
                )
                .unwrap(),
                true,
            );
        }
    }

    #[test]
    fn curved_transport_straight_inflections_and_closed_seam() {
        let profile: Vec<_> = square().into_iter().map(|p| p * 0.1).collect();
        let path = [
            DVec3::ZERO,
            DVec3::Z,
            DVec3::new(0., 0., 2.),
            DVec3::new(0.2, 0., 3.),
            DVec3::new(0., 0., 4.),
        ];
        check(
            &mesh(&profile, &path, &[1.; 5], &[0.; 5], true, false, true).unwrap(),
            true,
        );
        let path = [
            DVec3::new(2., 0., 0.),
            DVec3::new(0., 2., 0.3),
            DVec3::new(-2., 0., 0.),
            DVec3::new(0., -2., 0.),
        ];
        let m = mesh(&profile, &path, &[1.; 4], &[0.; 4], true, true, true).unwrap();
        assert_eq!(m.p.len(), 64);
        assert_eq!(m.i.len(), 96);
        check(&m, true);
    }

    #[test]
    fn taper_normals_include_longitudinal_slope() {
        let m = mesh(
            &square(),
            &[DVec3::ZERO, DVec3::Z * 2.],
            &[1., 0.5],
            &[0.; 2],
            true,
            false,
            true,
        )
        .unwrap();
        check(&m, true);
        for n in &m.n[..16] {
            assert!(n[2] > 0.);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn scaled_twisted_profiles_are_closed_and_outward(
            radius in 0.01f32..0.5,
            length in 2f32..10.,
            scale in 0.5f64..2.,
            twist in -0.4f64..0.4,
            sides in 3usize..17,
            rings in 2usize..10,
        ) {
            let profile: Vec<_> = (0..sides).map(|i| {
                let a = i as f64 * std::f64::consts::TAU/sides as f64;
                DVec2::new(f64::from((a.cos()*f64::from(radius)) as f32),f64::from((a.sin()*f64::from(radius)) as f32))
            }).collect();
            let path: Vec<_> = (0..rings).map(|i| DVec3::new(0.,0.,f64::from(length)*i as f64/(rings-1) as f64)).collect();
            let scales: Vec<_> = (0..rings).map(|i| 1.+(scale-1.)*i as f64/(rings-1) as f64).collect();
            let twists: Vec<_> = (0..rings).map(|i| twist*i as f64/(rings-1) as f64).collect();
            check(&mesh(&profile,&path,&scales,&twists,true,false,true).unwrap(),true);
        }
    }

    fn evaluate_mesh(options: serde_json::Value) -> Mesh {
        let value: serde_json::Value =
            serde_json::from_str(&evaluate(&options.to_string()).unwrap()).unwrap();
        let name = options["name"].as_str().unwrap_or("sweep");
        let data = |key| value["buffers"][format!("{name}_{key}")]["data"].clone();
        Mesh {
            p: serde_json::from_value(data("position")).unwrap(),
            n: serde_json::from_value(data("normal")).unwrap(),
            uv: serde_json::from_value(data("TEXCOORD_0")).unwrap(),
            i: serde_json::from_value(data("indices")).unwrap(),
        }
    }

    #[test]
    fn authoring_open_sections_caps_duplicates_and_initial_frame() {
        use serde_json::json;
        let base =
            json!({"profile":[[-1,-1],[1,-1],[1,1],[-1,1],[-1,-1]],"path":[[0,0,0],[0,0,3]]});
        let m = evaluate_mesh(base.clone());
        check(&m, true);
        assert_eq!(m.p[0], [1., -1., 0.]);
        let mut duplicate = base.clone();
        duplicate["profile"] = json!([[-1, -1], [-1, -1], [1, -1], [1, 1], [-1, 1], [-1, -1]]);
        duplicate["path"] = json!([[0, 0, 0], [0, 0, 0], [0, 0, 3]]);
        assert_eq!(evaluate_mesh(duplicate).p, m.p);
        for caps in [true, false] {
            let strip = evaluate_mesh(
                json!({"profile":[[-1,0],[1,0]],"path":[[0,0,0],[0,0,3]],"caps":caps}),
            );
            assert_eq!(strip.p.len(), 4);
            assert_eq!(strip.i.len(), 6);
            check(&strip, false);
        }
        let mut vertical = base;
        vertical["path"] = json!([[0, 0, 0], [0, 3, 0]]);
        check(&evaluate_mesh(vertical), true);
    }

    #[test]
    fn authoring_total_twist_uses_arc_length_and_scale_accepts_both_forms() {
        use serde_json::json;
        let mut options = json!({"profile":[[-1,0],[1,0]],"path":[[0,0,0],[0,0,1],[0,0,4]],"twist":std::f64::consts::PI,"scale":2});
        let m = evaluate_mesh(options.clone());
        let expected = 2f32.sqrt();
        assert!((m.p[2][0] - expected).abs() < 1e-6);
        assert!((m.p[2][1] - expected).abs() < 1e-6);
        options["scale"] = json!([2, 2, 2]);
        assert_eq!(evaluate_mesh(options).p, m.p);
    }

    #[test]
    fn fractional_uniform_and_per_point_scales_preserve_numeric_values() {
        use serde_json::json;
        let base =
            json!({"profile":[[-1,-1],[1,-1],[1,1],[-1,1],[-1,-1]],"path":[[0,0,0],[0,0,2]]});
        for scale in [json!(0.25), json!([0.25, 0.75])] {
            let mut options = base.clone();
            options["scale"] = scale.clone();
            let m = evaluate_mesh(options);
            check(&m, true);
            assert_eq!(m.p[0], [0.25, -0.25, 0.]);
            assert_eq!(m.p[2][0], if scale.is_array() { 0.75 } else { 0.25 });
        }
        for scale in [json!(null), json!("1"), json!([1, "1"])] {
            let mut bad = base.clone();
            bad["scale"] = scale;
            assert!(evaluate(&bad.to_string()).is_err());
        }
    }

    #[test]
    fn authoring_closed_path_and_hard_longitudinal_corners() {
        use serde_json::json;
        let options = json!({"profile":[[-0.1,-0.1],[0.1,-0.1],[0.1,0.1],[-0.1,0.1],[-0.1,-0.1]],
            "path":[[2,0,0],[0,2,0],[-2,0,0],[0,-2,0],[2,0,0]], "scale":[1,1,1,1,1]});
        let m = evaluate_mesh(options.clone());
        check(&m, true);
        assert_eq!(m.p.len(), 64);
        // A right-angle spine corner has distinct incoming and outgoing normals.
        assert_eq!(m.p[18], m.p[20]);
        assert_ne!(m.n[18], m.n[20]);
        let mut bad = options.clone();
        bad["twist"] = json!(1.);
        assert!(
            evaluate(&bad.to_string())
                .unwrap_err()
                .to_string()
                .contains("whole turns")
        );
        bad = options;
        bad["scale"] = json!([1, 1, 1, 1, 2]);
        assert!(
            evaluate(&bad.to_string())
                .unwrap_err()
                .to_string()
                .contains("endpoint scales")
        );
    }

    #[test]
    fn smooth_closed_spatial_path_with_full_twist_welds() {
        use serde_json::json;
        let profile: Vec<_> = (0..=16)
            .map(|i| {
                let a = (i % 16) as f64 * std::f64::consts::TAU / 16.;
                [0.07 * a.cos(), 0.07 * a.sin()]
            })
            .collect();
        let path: Vec<_> = (0..=32)
            .map(|i| {
                let a = (i % 32) as f64 * std::f64::consts::TAU / 32.;
                [2. * a.cos(), 2. * a.sin(), 0.3 * (2. * a).sin()]
            })
            .collect();
        for twist in [0., std::f64::consts::TAU, -std::f64::consts::TAU] {
            check(
                &evaluate_mesh(json!({"profile":profile,"path":path,"twist":twist})),
                true,
            );
        }
    }

    #[test]
    fn scene_expansion_uses_standard_fragment_contract() {
        use serde_json::json;
        let definition = json!({"std":"sweep","profile":[[0,0],[1,0]],"path":[[0,0,0],[0,0,1]]});
        let mut scene =
            json!({"geometry":{"s":definition},"nodes":[{"children":[{"geometry":definition}]}]});
        crate::expand_scene(&mut scene, std::path::Path::new(".")).unwrap();
        assert_eq!(
            scene["geometry"]["s"],
            scene["nodes"][0]["children"][0]["geometry"]
        );
        assert!(scene["geometry"]["s"]["attributes"]["POSITION"]["data"].is_array());
    }

    #[test]
    fn authoring_refuses_invalid_options_with_actionable_errors() {
        use serde_json::json;
        let base = json!({"profile":[[0,0],[1,0]],"path":[[0,0,0],[0,0,1]]});
        for (key, value, expected) in [
            ("scale", json!([1]), "match the path length"),
            ("scale", json!(0), "greater than zero"),
            ("profile", json!([[0, 0], [0, 0]]), "distinct points"),
            ("path", json!([[0, 0, 0], [0, 0, 0]]), "distinct points"),
            ("name", json!("a/b"), "choose a simple name"),
            ("unknown", json!(1), "unknown field"),
        ] {
            let mut bad = base.clone();
            bad[key] = value;
            assert!(
                evaluate(&bad.to_string())
                    .unwrap_err()
                    .to_string()
                    .contains(expected),
                "{bad}"
            );
        }
    }

    #[test]
    fn invalid_numeric_inputs_and_topology_refuse() {
        let path = [DVec3::ZERO, DVec3::Z];
        assert!(mesh(&square(), &path, &[0., 1.], &[0.; 2], true, false, true).is_err());
        assert!(
            mesh(
                &square(),
                &path,
                &[1.; 2],
                &[f64::NAN, 0.],
                true,
                false,
                true
            )
            .is_err()
        );
        assert!(
            mesh(
                &square(),
                &[DVec3::ZERO, DVec3::ZERO],
                &[1.; 2],
                &[0.; 2],
                true,
                false,
                true
            )
            .is_err()
        );
        assert!(
            mesh(
                &square(),
                &[DVec3::ZERO, DVec3::Z, DVec3::ZERO],
                &[1.; 3],
                &[0.; 3],
                true,
                false,
                true
            )
            .is_err()
        );
        let crossing = [[0., 0.], [2., 2.], [0., 2.], [2., 0.]].map(DVec2::from_array);
        assert!(mesh(&crossing, &path, &[1.; 2], &[0.; 2], true, false, true).is_err());
    }
}
