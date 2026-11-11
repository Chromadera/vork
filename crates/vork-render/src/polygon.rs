//! Install-time ear clipping of simple planar polygon cages. Vertex channels stay indexed.
use anyhow::{Context, Result, ensure};
#[cfg(test)]
use glam::Vec2;
use glam::{DVec2, DVec3, Vec3};

pub(crate) fn triangulate(
    positions: &[[f32; 3]],
    indices: &[u32],
    sizes: &[u32],
) -> Result<Vec<u32>> {
    ensure!(
        sizes.iter().map(|n| u64::from(*n)).sum::<u64>() == indices.len() as u64,
        "face_vertex_counts must cover the complete index buffer"
    );
    ensure!(
        indices.iter().all(|i| (*i as usize) < positions.len()),
        "polygon index out of range"
    );
    let mut out = Vec::new();
    let mut offset = 0;
    for &size in sizes {
        ensure!(size >= 3, "polygon requires at least three corners");
        let face = &indices[offset..offset + size as usize];
        offset += size as usize;
        // Center before accumulating: absolute-coordinate cross products lose area
        // under translation. Stored f32 corners need not remain exactly coplanar.
        let origin = Vec3::from(positions[face[0] as usize]).as_dvec3();
        let vertices: Vec<_> = face
            .iter()
            .map(|&id| Vec3::from(positions[id as usize]).as_dvec3() - origin)
            .collect();
        let mut normal = DVec3::ZERO;
        for i in 0..face.len() {
            normal += vertices[i].cross(vertices[(i + 1) % face.len()]);
        }
        ensure!(
            normal.is_finite() && normal.length_squared() > 0.,
            "degenerate polygon"
        );
        let axis = normal.abs().max_element();
        let project = |p: &DVec3| {
            if normal.x.abs() == axis {
                DVec2::new(p[1], p[2])
            } else if normal.y.abs() == axis {
                DVec2::new(p[2], p[0])
            } else {
                DVec2::new(p[0], p[1])
            }
        };
        let points: Vec<_> = vertices.iter().map(project).collect();
        let area: f64 = (0..points.len())
            .map(|i| points[i].perp_dot(points[(i + 1) % points.len()]))
            .sum();
        ensure!(
            area.is_finite() && area != 0.,
            "degenerate polygon projection"
        );
        let sign = area.signum();
        let cross =
            |u: usize, v: usize, p: DVec2| (points[v] - points[u]).perp_dot(p - points[u]) * sign;
        // Projection alone does not establish facing for rounded, nearly
        // collinear corners. Check both the full normal and the f32 geometry
        // consumed by the renderer, without discarding any authored corners.
        let valid = |a: usize, b: usize, c: usize| {
            let n = (vertices[b] - vertices[a]).cross(vertices[c] - vertices[a]);
            let [a32, b32, c32] = [a, b, c].map(|i| Vec3::from(positions[face[i] as usize]));
            let n32 = (b32 - a32).cross(c32 - a32);
            cross(a, b, points[c]) > 0.
                && n.dot(normal) > 0.
                && n32.is_finite()
                && n32.as_dvec3().dot(normal) > 0.
        };
        let mut remaining: Vec<usize> = (0..face.len()).collect();
        // Prefer broad ears so nearly collinear residual diagonals do not
        // amplify the slight nonplanarity introduced by stored f32 corners.
        while remaining.len() > 3 {
            let area = |k: usize| {
                cross(
                    remaining[(k + remaining.len() - 1) % remaining.len()],
                    remaining[k],
                    points[remaining[(k + 1) % remaining.len()]],
                )
            };
            let mut candidates: Vec<_> = (0..remaining.len()).collect();
            candidates
                .sort_unstable_by(|&i, &j| area(j).total_cmp(&area(i)).then_with(|| i.cmp(&j)));
            // Test containment only until the first valid ear in this order.
            let ear = candidates.into_iter().find(|&i| {
                let a = remaining[(i + remaining.len() - 1) % remaining.len()];
                let b = remaining[i];
                let c = remaining[(i + 1) % remaining.len()];
                valid(a, b, c)
                        // Do not leave an invalid last triangle when another
                        // diagonal of this quadrilateral can preserve facing.
                        && (remaining.len() != 4 || {
                            valid(a, c, remaining[(i + 2) % remaining.len()])
                        })
                        && !remaining.iter().any(|&p| {
                            p != a
                                && p != b
                                && p != c
                                && cross(a, b, points[p]) >= 0.
                                && cross(b, c, points[p]) >= 0.
                                && cross(c, a, points[p]) >= 0.
                        })
            }).context("polygon cannot be triangulated with consistent facing; remove duplicate or collinear corners and ensure a simple planar outline")?;
            out.extend([
                face[remaining[(ear + remaining.len() - 1) % remaining.len()]],
                face[remaining[ear]],
                face[remaining[(ear + 1) % remaining.len()]],
            ]);
            remaining.remove(ear);
        }
        ensure!(
            valid(remaining[0], remaining[1], remaining[2]),
            "degenerate polygon triangle; remove duplicate or collinear corners and ensure a simple planar outline"
        );
        out.extend(remaining.iter().map(|&i| face[i]));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    #[test]
    fn clips_concave_polygons_with_both_windings() -> Result<()> {
        let p = [
            [0., 0., 0.],
            [2., 0., 0.],
            [2., 2., 0.],
            [1., 1., 0.],
            [0., 2., 0.],
        ];
        for i in [vec![0, 1, 2, 3, 4], vec![4, 3, 2, 1, 0]] {
            let triangles = triangulate(&p, &i, &[5])?;
            assert_eq!(triangles.len(), 9);
            let area: f32 = triangles
                .chunks_exact(3)
                .map(|t| {
                    (Vec3::from(p[t[1] as usize]) - Vec3::from(p[t[0] as usize]))
                        .cross(Vec3::from(p[t[2] as usize]) - Vec3::from(p[t[0] as usize]))
                        .z
                })
                .sum();
            assert!((area.abs() - 6.).abs() < 1e-6);
            assert_eq!(area.signum(), if i[0] == 0 { 1. } else { -1. });
        }
        assert!(triangulate(&p, &[0, 1, 2], &[4]).is_err());
        assert!(triangulate(&p, &[0, 1, 8], &[3]).is_err());
        Ok(())
    }

    #[test]
    fn clips_nearly_collinear_terminal_polygons() {
        for count in [3, 4, 7] {
            for offset in [Vec3::ZERO, Vec3::new(8., -16., 32.)] {
                for axis in [Vec3::X, Vec3::Y, Vec3::new(1., 2., 3.).normalize()] {
                    let rotation = glam::Quat::from_axis_angle(axis, 0.7);
                    let outline = [
                        [0., 0.],
                        [1., 0.00001],
                        [2., 0.],
                        [3., 0.00001],
                        [4., 0.],
                        [4., 2.],
                        [0., 2.],
                    ];
                    let selected: &[usize] = match count {
                        3 => &[0, 1, 6],
                        4 => &[0, 1, 2, 6],
                        _ => &[0, 1, 2, 3, 4, 5, 6],
                    };
                    let positions: Vec<_> = selected
                        .iter()
                        .map(|&i| {
                            (rotation * Vec2::from(outline[i]).extend(0.) + offset).to_array()
                        })
                        .collect();
                    for reverse in [false, true] {
                        let mut indices: Vec<_> = (0..count).collect();
                        if reverse {
                            indices.reverse();
                        }
                        let triangles = triangulate(&positions, &indices, &[count]).unwrap();
                        assert_eq!(triangles.len(), 3 * (count as usize - 2));
                        let facing = rotation * Vec3::Z * if reverse { -1. } else { 1. };
                        for t in triangles.chunks_exact(3) {
                            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(positions[t[k] as usize]));
                            assert!((b - a).cross(c - a).dot(facing) > 0.);
                        }
                        assert!(indices.iter().all(|i| triangles.contains(i)));
                    }
                }
            }
        }
        for indices in [&[0, 1, 2][..], &[0, 1, 2, 1][..]] {
            assert!(
                triangulate(
                    &[[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]],
                    indices,
                    &[indices.len() as u32]
                )
                .is_err()
            );
        }
    }

    #[test]
    fn refuses_duplicate_boundary_corners() {
        let mut positions: Vec<_> = (0..32)
            .map(|i| {
                Vec2::from_angle(i as f32 * std::f32::consts::TAU / 32.)
                    .extend(0.)
                    .to_array()
            })
            .collect();
        positions.insert(1, positions[0]);
        let indices: Vec<_> = (0..positions.len() as u32).collect();
        assert!(triangulate(&positions, &indices, &[indices.len() as u32]).is_err());
    }

    #[test]
    fn clips_rounded_nearly_collinear_polygon() {
        let points = vec![
            Vec2::new(0.18372832, 0.07901839),
            Vec2::new(0.14397466, 0.1388211),
            Vec2::new(0.08366115, 0.18166126),
            Vec2::new(0.034287248, 0.19703905),
            Vec2::new(-0.037972543, 0.19636212),
            Vec2::new(-0.10727817, 0.16879395),
            Vec2::new(-0.14928702, 0.13309166),
            Vec2::new(-0.18076341, 0.08558381),
            Vec2::new(-0.19819614, 0.026800955),
            Vec2::new(-0.19673105, -0.036012486),
            Vec2::new(-1.4309818, -0.6859125),
            Vec2::new(-1.1546293, -1.0620306),
            Vec2::new(-0.85554606, -1.7945865),
            Vec2::new(-0.0358887, -0.6737665),
            Vec2::new(0.0989302, -0.41140997),
            Vec2::new(0.42885885, -0.67896396),
            Vec2::new(0.20025675, -0.14865503),
            Vec2::new(1.07707, -0.2887231),
            Vec2::new(0.65160775, 1.1393072e-7),
        ];
        let rotation = glam::Quat::from_axis_angle(
            Vec3::new(-0.91170937, 0.8981672, 0.51308775).normalize(),
            -0.63403845,
        );
        let positions: Vec<_> = points
            .iter()
            .map(|p| (rotation * p.extend(0.) + Vec3::new(0., 3.958028, 7.6756897)).to_array())
            .collect();
        let indices: Vec<u32> = (0..points.len() as u32).collect();
        let out = triangulate(&positions, &indices, &[points.len() as u32]).unwrap();
        assert_eq!(out.len(), 3 * (points.len() - 2));
        assert!(out.chunks_exact(3).all(|t| {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(positions[t[k] as usize]));
            (b - a).cross(c - a).dot(rotation * Vec3::Z) > 0.
        }));
    }

    /// A simple polygon star-shaped about its centre: increasing angles with every gap below
    /// half a turn (each under 1 / 2.2 of the total), and positive radii.
    fn star() -> impl Strategy<Value = Vec<Vec2>> {
        prop::collection::vec((0.6f32..1., 0.2f32..2.), 3..24).prop_map(|corners| {
            let total: f32 = corners.iter().map(|c| c.0).sum();
            let mut angle = 0.;
            corners
                .iter()
                .map(|&(gap, radius)| {
                    angle += gap / total * std::f32::consts::TAU;
                    Vec2::from_angle(angle) * radius
                })
                .collect()
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Simple polygons in any plane and winding clip into n - 2 nondegenerate triangles
        /// that keep the polygon's corners, area and facing.
        #[test]
        fn clips_star_polygons_in_any_plane(
            polygons in prop::collection::vec((star(), any::<bool>()), 1..4),
            axis in prop::array::uniform3(-1f32..1.),
            angle in -std::f32::consts::PI..std::f32::consts::PI,
            offset in prop::array::uniform3(-10f32..10.),
        ) {
            let rotation = glam::Quat::from_axis_angle(
                Vec3::from(axis).try_normalize().unwrap_or(Vec3::Z),
                angle,
            );
            let (mut positions, mut indices, mut sizes) = (Vec::new(), Vec::new(), Vec::new());
            for (points, reversed) in &polygons {
                let first = positions.len() as u32;
                let mut face: Vec<u32> = (first..first + points.len() as u32).collect();
                if *reversed {
                    face.reverse();
                }
                positions.extend(
                    points
                        .iter()
                        .map(|p| (rotation * p.extend(0.) + Vec3::from(offset)).to_array()),
                );
                indices.extend(face);
                sizes.push(points.len() as u32);
            }
            let triangles = triangulate(&positions, &indices, &sizes).unwrap();
            let mut start = 0;
            let mut corner = 0;
            for (points, reversed) in &polygons {
                let n = points.len();
                let face = &indices[corner..corner + n];
                let own = &triangles[start..start + 3 * (n - 2)];
                prop_assert!(own.iter().all(|i| face.contains(i)));
                let area = (0..n)
                    .map(|i| points[i].perp_dot(points[(i + 1) % n]))
                    .sum::<f32>()
                    / 2.;
                let facing = rotation * Vec3::Z * if *reversed { -1. } else { 1. };
                let mut clipped = 0.;
                for t in own.chunks_exact(3) {
                    let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(positions[t[k] as usize]));
                    let normal = (b - a).cross(c - a);
                    prop_assert!(normal.dot(facing) > 0., "triangle {:?} faces away", t);
                    clipped += normal.length() / 2.;
                }
                // Rounding grows with the offset plane's coordinates and the triangle count.
                let tolerance = 1e-5 * n as f32 * area.max(1.);
                prop_assert!((clipped - area).abs() <= tolerance, "area {} != {}", clipped, area);
                start += 3 * (n - 2);
                corner += n;
            }
            prop_assert_eq!(start, triangles.len());
        }
    }
}
