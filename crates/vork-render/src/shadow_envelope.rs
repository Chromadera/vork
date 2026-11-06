//! Conservative affine bounds shared by pose envelopes and frame fitting.
use crate::*;

type Bounds = (Vec3, Vec3);

pub(crate) fn transformed(matrix: Mat4, bounds: Bounds) -> Bounds {
    let center = (bounds.0 + bounds.1) * 0.5;
    let extent = (bounds.1 - bounds.0) * 0.5;
    let center = matrix.transform_point3(center);
    let extent = matrix.x_axis.truncate().abs() * extent.x
        + matrix.y_axis.truncate().abs() * extent.y
        + matrix.z_axis.truncate().abs() * extent.z;
    (center - extent, center + extent)
}

/// Vertices of the convex intersection, including edges crossing a face when
/// neither original polyhedron has a vertex inside the other.
pub(crate) fn clipped_bounds(corners: &[Vec3; 8], bounds: (Vec3, Vec3)) -> Vec<Vec3> {
    let center = corners.iter().copied().sum::<Vec3>() / 8.;
    let faces = [
        [0, 2, 4],
        [1, 5, 3],
        [0, 4, 1],
        [2, 3, 6],
        [0, 1, 2],
        [4, 6, 5],
    ];
    let frustum: Vec<glam::Vec4> = faces
        .iter()
        .map(|&[a, b, c]| {
            let normal = (corners[b] - corners[a])
                .cross(corners[c] - corners[a])
                .normalize_or_zero();
            let plane = normal.extend(-normal.dot(corners[a]));
            if plane.dot(center.extend(1.)) < 0. {
                -plane
            } else {
                plane
            }
        })
        .collect();
    let (lo, hi) = bounds;
    let box_corners: [Vec3; 8] = std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    });
    let box_planes = [
        glam::Vec4::new(1., 0., 0., -lo.x),
        glam::Vec4::new(-1., 0., 0., hi.x),
        glam::Vec4::new(0., 1., 0., -lo.y),
        glam::Vec4::new(0., -1., 0., hi.y),
        glam::Vec4::new(0., 0., 1., -lo.z),
        glam::Vec4::new(0., 0., -1., hi.z),
    ];
    let mut points = Vec::new();
    for (vertices, planes) in [
        (corners, box_planes.as_slice()),
        (&box_corners, frustum.as_slice()),
    ] {
        for i in 0..8 {
            for bit in [1, 2, 4] {
                if i & bit != 0 {
                    continue;
                }
                let a = vertices[i];
                let b = vertices[i | bit];
                let (mut start, mut end) = (0_f32, 1_f32);
                for plane in planes {
                    let da = plane.dot(a.extend(1.));
                    let db = plane.dot(b.extend(1.));
                    if da < 0. && db < 0. {
                        end = -1.;
                        break;
                    }
                    if da < 0. {
                        start = start.max(da / (da - db));
                    }
                    if db < 0. {
                        end = end.min(da / (da - db));
                    }
                }
                if start <= end {
                    points.push(a.lerp(b, start));
                    points.push(a.lerp(b, end));
                }
            }
        }
    }
    points
}
