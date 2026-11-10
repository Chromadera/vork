//! Stage 1 convex carrier admission, with CPU interval and physical oracles.
//! Planes retain f64 here: this does not certify a rounded GPU representation.
use std::collections::BTreeMap;

type Point = [f64; 3];
type Triangle = [[f32; 3]; 3];
const MAX_FACES: usize = 32;

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn key(p: [f32; 3]) -> [u32; 3] {
    p.map(|v| if v == 0. { 0 } else { v.to_bits() })
}

#[derive(Debug)]
pub(super) struct Convex {
    // Keep an anchor on each plane to avoid large translated plane offsets.
    pub(super) planes: Vec<(Point, Point)>,
}
impl Convex {
    pub(super) fn certify(triangles: &[Triangle]) -> Result<Self, String> {
        if !(4..=MAX_FACES).contains(&triangles.len()) {
            return Err(format!(
                "convex medium carrier needs 4..={MAX_FACES} triangle faces, got {}",
                triangles.len()
            ));
        }
        let mut edges = BTreeMap::<_, (u32, i32)>::new();
        let mut points = BTreeMap::new();
        for triangle in triangles {
            for &p in triangle {
                if !p.iter().all(|v| v.is_finite()) {
                    return Err("nonfinite carrier vertex".into());
                }
                points.insert(key(p), p.map(f64::from));
            }
            for i in 0..3 {
                let a = key(triangle[i]);
                let b = key(triangle[(i + 1) % 3]);
                if a == b {
                    return Err("degenerate carrier edge".into());
                }
                let (edge, winding) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                let uses = edges.entry(edge).or_default();
                uses.0 += 1;
                uses.1 += winding;
            }
        }
        if edges
            .values()
            .any(|&(count, winding)| count != 2 || winding != 0)
        {
            return Err("carrier must be a closed, consistently oriented manifold".into());
        }
        // Shift to a vertex before volume evaluation; translation cannot decide
        // winding. All f32 inputs are exactly representable in this f64 oracle.
        let anchor = *points.values().next().unwrap();
        let volume: f64 = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|p| sub(p.map(f64::from), anchor));
                dot(a, cross(b, c))
            })
            .sum();
        if !volume.is_finite() || volume == 0. {
            return Err("carrier has zero or invalid enclosed volume".into());
        }
        let orientation = volume.signum();
        let mut planes = Vec::new();
        for triangle in triangles {
            let [a, b, c] = triangle.map(|p| p.map(f64::from));
            let normal = cross(sub(b, a), sub(c, a)).map(|v| v * orientation);
            if dot(normal, normal) == 0. {
                return Err("degenerate carrier face".into());
            }
            for &p in points.values() {
                let delta = sub(p, a);
                let distance = dot(normal, delta);
                // Rounding allowance scales with the actual dot operands,
                // never with a world-space epsilon or a carrier's translation.
                let magnitude: f64 = normal
                    .into_iter()
                    .zip(delta)
                    .map(|(n, d)| (n * d).abs())
                    .sum();
                if distance > 16. * f64::EPSILON * magnitude {
                    return Err(
                        "carrier is not convex: a vertex lies outside a supporting face".into(),
                    );
                }
            }
            planes.push((normal, a));
        }
        Ok(Self { planes })
    }

    /// Direction need not be normalized: applying an inverse object transform
    /// to both origin and direction preserves the world-distance ray parameter.
    #[cfg(test)]
    fn interval(&self, origin: Point, direction: Point, limit: f64) -> Option<[f64; 2]> {
        assert!(origin.iter().chain(&direction).all(|v| v.is_finite()));
        assert!(direction.iter().any(|&v| v != 0.) && limit >= 0.);
        let (mut enter, mut exit) = (0_f64, limit);
        for &(normal, anchor) in &self.planes {
            let numerator = dot(normal, sub(anchor, origin));
            let denominator = dot(normal, direction);
            if denominator == 0. {
                if numerator < 0. {
                    return None;
                }
            } else {
                let t = numerator / denominator;
                if denominator < 0. {
                    enter = enter.max(t);
                } else {
                    exit = exit.min(t);
                }
                if enter >= exit {
                    return None;
                }
            }
        }
        (enter < exit).then_some([enter, exit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cube() -> Vec<Triangle> {
        let v = [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ];
        [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 4, 7],
            [0, 7, 3],
            [1, 2, 6],
            [1, 6, 5],
            [0, 1, 5],
            [0, 5, 4],
            [3, 7, 6],
            [3, 6, 2],
        ]
        .map(|t| t.map(|i| v[i]))
        .to_vec()
    }
    #[test]
    fn box_inside_outside_parallel_grazing_and_clipped() {
        let c = Convex::certify(&cube()).unwrap();
        assert_eq!(
            c.interval([0., 0., 2.], [0., 0., -1.], 10.),
            Some([1.5, 2.5])
        );
        assert_eq!(c.interval([0.; 3], [0., 0., -1.], 10.), Some([0., 0.5]));
        assert_eq!(c.interval([0.; 3], [0., 0., -1.], 0.25), Some([0., 0.25]));
        assert_eq!(c.interval([0., 0., 2.], [0., 0., -1.], 1.), None);
        assert_eq!(c.interval([0.6, 0., 2.], [0., 0., -1.], 10.), None);
        assert_eq!(
            c.interval([0.5, 0., 2.], [0., 0., -1.], 10.),
            Some([1.5, 2.5])
        );
        assert_eq!(c.interval([0.5, 0.5, 0.5], [1., 1., 1.], 10.), None);
    }
    #[test]
    fn world_metres_survive_nonuniform_scale() {
        let c = Convex::certify(&cube()).unwrap();
        // World carrier scale (2,3,4), world ray (0,0,5) + t*(0,0,-1).
        assert_eq!(
            c.interval([0., 0., 1.25], [0., 0., -0.25], 20.),
            Some([3., 7.])
        );
        // Reflection of the object also preserves the interval.
        assert_eq!(
            c.interval([0., 0., -1.25], [0., 0., 0.25], 20.),
            Some([3., 7.])
        );
    }
    #[test]
    fn reversed_winding_and_large_translation_preserve_membership() {
        let mut triangles = cube();
        for t in &mut triangles {
            t.swap(1, 2);
            for p in t {
                p[0] += 1_000_000.;
            }
        }
        let c = Convex::certify(&triangles).unwrap();
        assert_eq!(
            c.interval([1_000_000., 0., 2.], [0., 0., -1.], 10.),
            Some([1.5, 2.5])
        );
    }
    #[test]
    fn rejects_open_inconsistent_degenerate_and_nonfinite_carriers() {
        let mut t = cube();
        t.pop();
        assert!(Convex::certify(&t).unwrap_err().contains("closed"));
        let mut t = cube();
        t[0].swap(1, 2);
        assert!(Convex::certify(&t).unwrap_err().contains("oriented"));
        let mut t = cube();
        t[0][1] = t[0][0];
        assert!(Convex::certify(&t).unwrap_err().contains("degenerate"));
        let mut t = cube();
        t[0][0][0] = f32::NAN;
        assert!(Convex::certify(&t).unwrap_err().contains("nonfinite"));
        let t = cube()
            .into_iter()
            .map(|t| t.map(|p| [p[0], p[1], 0.]))
            .collect::<Vec<_>>();
        assert!(Convex::certify(&t).is_err());
    }
    #[test]
    fn rejects_closed_concave_and_disconnected_carriers() {
        let mut t = cube();
        for triangle in &mut t {
            for p in triangle {
                if *p == [0.5, 0.5, 0.5] {
                    *p = [0., 0., 0.];
                }
            }
        }
        assert!(Convex::certify(&t).unwrap_err().contains("not convex"));
        let mut t = cube();
        t.extend(cube().into_iter().map(|t| {
            t.map(|mut p| {
                p[0] += 2.;
                p
            })
        }));
        assert!(Convex::certify(&t).unwrap_err().contains("not convex"));
    }
    #[test]
    fn face_limit_is_admission_not_truncation() {
        let mut t = cube();
        t.extend(cube());
        t.extend(cube());
        assert!(
            Convex::certify(&t)
                .unwrap_err()
                .contains("32 triangle faces")
        );
        assert!(Convex::certify(&[]).is_err());
    }
    #[test]
    fn additive_overlap_beer_and_zero_extinction_emission_oracles() {
        let c = Convex::certify(&cube()).unwrap();
        let [a, b] = c.interval([0., 0., 2.], [0., 0., -1.], 10.).unwrap();
        let length = b - a;
        for copies in [1., 2., 8.] {
            for sigma in [0_f64, 1e-12, 0.5, 100.] {
                let extinction = copies * sigma;
                let transmission = (-extinction * length).exp();
                let integral = if extinction == 0. {
                    length
                } else {
                    -(-extinction * length).exp_m1() / extinction
                };
                assert!(integral.is_finite() && integral > 0.);
                assert!((transmission + extinction * integral - 1.).abs() < 1e-14);
                if extinction == 0. {
                    assert_eq!(copies * integral, copies);
                }
            }
        }
    }
}

/// Independent homogeneous reference, including the physical infinite tail.
/// Returning an error is essential: infinity times zero is not a source model.
#[cfg(test)]
fn homogeneous_reference(sigma: f64, emission: f64, distance: f64) -> Result<(f64, f64), &'static str> {
    if !sigma.is_finite() || sigma < 0. || !emission.is_finite() || emission < 0.
        || distance.is_nan() || distance < 0. {
        return Err("invalid medium coefficients or distance");
    }
    if distance == f64::INFINITY {
        return if sigma > 0. { Ok((0., emission / sigma)) }
            else if emission == 0. { Ok((1., 0.)) }
            else { Err("divergent world medium emission") };
    }
    let tau = sigma * distance;
    let integral = if sigma == 0. { distance } else { -(-tau).exp_m1() / sigma };
    Ok(((-tau).exp(), emission * integral))
}

#[cfg(test)]
mod closure_tests {
    use super::*;

    #[test]
    fn infinity_is_closed_per_channel_without_cutoff() {
        assert_eq!(homogeneous_reference(0., 0., f64::INFINITY), Ok((1., 0.)));
        assert_eq!(homogeneous_reference(2., 3., f64::INFINITY), Ok((0., 1.5)));
        assert_eq!(homogeneous_reference(0., 3., f64::INFINITY), Err("divergent world medium emission"));
        // Even arbitrarily thin positive extinction has an opaque infinite tail.
        assert_eq!(homogeneous_reference(1e-30, 0., f64::INFINITY), Ok((0., 0.)));
        for (sigma, emission) in [(0.5, 2.), (1e-8, 1e-10), (100., 1.)] {
            let finite = homogeneous_reference(sigma, emission, 1000. / sigma).unwrap();
            let infinite = homogeneous_reference(sigma, emission, f64::INFINITY).unwrap();
            assert_eq!(finite, infinite);
        }
    }

    #[test]
    fn finite_sources_compose_at_segment_boundaries() {
        for sigma in [0., 1e-12, 0.5, 100.] {
            let (t, e) = homogeneous_reference(sigma, 3., 7.).unwrap();
            let (ta, ea) = homogeneous_reference(sigma, 3., 2.).unwrap();
            let (tb, eb) = homogeneous_reference(sigma, 3., 5.).unwrap();
            assert!((t - ta * tb).abs() <= 1e-14);
            assert!((e - (ea + ta * eb)).abs() <= 1e-13 * e.max(1.));
        }
    }

    #[test]
    fn invalid_physical_values_fail_instead_of_clamping() {
        for bad in [-1., f64::INFINITY, f64::NAN] {
            assert!(homogeneous_reference(bad, 0., 1.).is_err());
            assert!(homogeneous_reference(0., bad, 1.).is_err());
        }
        assert!(homogeneous_reference(0., 0., -1.).is_err());
        assert!(homogeneous_reference(0., 0., f64::NAN).is_err());
    }
}
