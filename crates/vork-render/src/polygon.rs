//! Install-time ear clipping of simple planar polygon cages. Vertex channels stay indexed.
use anyhow::{Context, Result, ensure};
use glam::{Vec2, Vec3};

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
        let mut normal = Vec3::ZERO;
        for i in 0..face.len() {
            normal += Vec3::from(positions[face[i] as usize])
                .cross(Vec3::from(positions[face[(i + 1) % face.len()] as usize]));
        }
        ensure!(
            normal.is_finite() && normal.length_squared() > 0.,
            "degenerate polygon"
        );
        let axis = normal.abs().max_element();
        let project = |id: u32| {
            let p = positions[id as usize];
            if normal.x.abs() == axis {
                Vec2::new(p[1], p[2])
            } else if normal.y.abs() == axis {
                Vec2::new(p[2], p[0])
            } else {
                Vec2::new(p[0], p[1])
            }
        };
        let points: Vec<_> = face.iter().copied().map(project).collect();
        let area: f32 = (0..points.len())
            .map(|i| points[i].perp_dot(points[(i + 1) % points.len()]))
            .sum();
        ensure!(
            area.is_finite() && area != 0.,
            "degenerate polygon projection"
        );
        let sign = area.signum();
        let mut remaining: Vec<usize> = (0..face.len()).collect();
        while remaining.len() > 3 {
            let ear = (0..remaining.len())
                .find(|&i| {
                    let a = remaining[(i + remaining.len() - 1) % remaining.len()];
                    let b = remaining[i];
                    let c = remaining[(i + 1) % remaining.len()];
                    let cross = |u: usize, v: usize, p: Vec2| {
                        (points[v] - points[u]).perp_dot(p - points[u]) * sign
                    };
                    cross(a, b, points[c]) > 0.
                        && !remaining.iter().any(|&p| {
                            p != a
                                && p != b
                                && p != c
                                && cross(a, b, points[p]) >= 0.
                                && cross(b, c, points[p]) >= 0.
                                && cross(c, a, points[p]) >= 0.
                        })
                })
                .context("polygon must be simple and nondegenerate")?;
            out.extend([
                face[remaining[(ear + remaining.len() - 1) % remaining.len()]],
                face[remaining[ear]],
                face[remaining[(ear + 1) % remaining.len()]],
            ]);
            remaining.remove(ear);
        }
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
