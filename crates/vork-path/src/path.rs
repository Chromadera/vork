//! Shared typed path traversal, measurement and polyline contours.

use kurbo::{BezPath, ParamCurveArclen, PathEl};

/// One flattened subpath. Closed contours repeat their first point only when
/// the last drawing point is different; move-only contours remain present.
#[derive(Debug, Clone, PartialEq)]
pub struct Contour {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
}

/// Streaming chord lengths in path order. A repeated endpoint contributes zero;
/// closing edges are present only when the caller repeats the contour start.
/// Keeping this iterator ungrouped preserves the profile's floating-point sum
/// across multiple contours and adds no per-segment storage.
pub fn chord_lengths(points: &[[f64; 2]]) -> impl Iterator<Item = f64> + '_ {
    points
        .windows(2)
        .map(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]))
}

pub fn chord_length(points: &[[f64; 2]]) -> f64 {
    chord_lengths(points).sum()
}

fn validate_coordinates(path: &BezPath) -> Result<(), String> {
    let finite = |point: &kurbo::Point| point.x.is_finite() && point.y.is_finite();
    if path.elements().iter().all(|element| match element {
        PathEl::MoveTo(point) | PathEl::LineTo(point) => finite(point),
        PathEl::QuadTo(control, point) => finite(control) && finite(point),
        PathEl::CurveTo(first, second, point) => finite(first) && finite(second) && finite(point),
        PathEl::ClosePath => true,
    }) {
        Ok(())
    } else {
        Err("path coordinates must be finite".into())
    }
}

/// Curve arc length in local units, using the measurement API's existing
/// absolute accuracy. This deliberately differs from flattened chord length.
pub fn path_length(path: &BezPath) -> Result<f64, String> {
    validate_coordinates(path)?;
    let length = path
        .segments()
        .map(|segment| segment.arclen(1e-6))
        .sum::<f64>();
    if length.is_finite() {
        Ok(length)
    } else {
        Err("Path length must be finite".into())
    }
}

/// Stream flattened elements with caller-owned admission and output errors.
/// The caller validates coordinates and tolerance before entering this driver.
/// `before_curve` runs immediately before each drawable curve, so expensive
/// subdivision can be rejected without changing output/error ordering. Closing
/// a subpath clears its current point, matching kurbo's traversal semantics.
#[doc(hidden)]
pub fn flatten_elements<E>(
    path: &BezPath,
    tolerance: f64,
    mut before_curve: impl FnMut(kurbo::Point, PathEl) -> Result<(), E>,
    mut on_element: impl FnMut(PathEl) -> Result<(), E>,
) -> Result<(), E> {
    use std::cell::RefCell;
    debug_assert!(tolerance.is_finite() && tolerance > 0.0);
    let failure = RefCell::new(None);
    let elements = path.iter().scan(None, |last, element| {
        if failure.borrow().is_some() {
            return None;
        }
        if matches!(element, PathEl::QuadTo(..) | PathEl::CurveTo(..)) {
            if let Some(start) = *last {
                if let Err(error) = before_curve(start, element) {
                    *failure.borrow_mut() = Some(error);
                    return None;
                }
            }
        }
        *last = element.end_point();
        Some(element)
    });
    kurbo::flatten(elements, tolerance, |element| {
        if failure.borrow().is_none() {
            if let Err(error) = on_element(element) {
                *failure.borrow_mut() = Some(error);
            }
        }
    });
    match failure.into_inner() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Flatten at an explicit local-space tolerance, preserving the width-profile
/// pipeline's subpath boundaries, repeated closure points and point order.
pub fn flatten_contours(path: &BezPath, tolerance: f64) -> Result<Vec<Contour>, String> {
    validate_coordinates(path)?;
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err("flattening tolerance must be finite and positive".into());
    }
    let mut contours = Vec::new();
    let mut points = Vec::new();
    flatten_elements(
        path,
        tolerance,
        |_, _| Ok::<(), String>(()),
        |element| {
            match element {
                PathEl::MoveTo(point) => {
                    if !points.is_empty() {
                        contours.push(Contour {
                            points: std::mem::take(&mut points),
                            closed: false,
                        });
                    }
                    points.push([point.x, point.y]);
                }
                PathEl::LineTo(point) => points.push([point.x, point.y]),
                PathEl::ClosePath => {
                    if let Some(first) = points.first().copied() {
                        if points.last() != Some(&first) {
                            points.push(first);
                        }
                        contours.push(Contour {
                            points: std::mem::take(&mut points),
                            closed: true,
                        });
                    }
                }
                _ => unreachable!(),
            }
            Ok(())
        },
    )?;
    if !points.is_empty() {
        contours.push(Contour {
            points,
            closed: false,
        });
    }
    if contours
        .iter()
        .flat_map(|contour| &contour.points)
        .flatten()
        .any(|coordinate| !coordinate.is_finite())
    {
        return Err("path coordinates must be finite".into());
    }
    Ok(contours)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_errors_stop_before_later_curve_admission() {
        let path = BezPath::from_svg("M0 0Q10 20 30 0C40 10 50 10 60 0").unwrap();
        let mut admitted = 0;
        let mut emitted = 0;
        let result = flatten_elements(
            &path,
            0.1,
            |_, _| {
                admitted += 1;
                Ok(())
            },
            |_| {
                emitted += 1;
                Err("output limit")
            },
        );
        assert_eq!(result, Err("output limit"));
        assert_eq!(emitted, 1);
        assert_eq!(admitted, 0);
        let mut emitted = Vec::new();
        let result = flatten_elements(
            &path,
            0.1,
            |_, _| Err("curve limit"),
            |element| {
                emitted.push(element);
                Ok(())
            },
        );
        assert_eq!(result, Err("curve limit"));
        assert_eq!(emitted, vec![PathEl::MoveTo((0.0, 0.0).into())]);
    }

    #[test]
    fn streaming_preserves_curves_after_close_and_missing_current_points() {
        let path = BezPath::from_vec(vec![
            PathEl::MoveTo((0.0, 0.0).into()),
            PathEl::ClosePath,
            PathEl::QuadTo((10.0, 10.0).into(), (20.0, 0.0).into()),
            PathEl::CurveTo((25.0, 10.0).into(), (30.0, 10.0).into(), (40.0, 0.0).into()),
        ]);
        let mut reference = Vec::new();
        kurbo::flatten(path.iter(), 0.1, |element| reference.push(element));
        let mut actual = Vec::new();
        let mut admitted = 0;
        flatten_elements(
            &path,
            0.1,
            |_, _| {
                admitted += 1;
                Ok::<(), ()>(())
            },
            |element| {
                actual.push(element);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(admitted, 1);
        assert_eq!(actual, reference);
    }

    #[test]
    fn chord_sums_keep_global_segment_order_across_contours() {
        let contours = [
            vec![[0.0, 0.0], [1e16, 0.0]],
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]],
        ];
        let total: f64 = contours
            .iter()
            .flat_map(|points| chord_lengths(points))
            .sum();
        assert_eq!(total.to_bits(), 1e16_f64.to_bits());
        // Summing each contour first changes the rounding and would move profile knots.
        let grouped: f64 = contours.iter().map(|points| chord_length(points)).sum();
        assert_ne!(total.to_bits(), grouped.to_bits());
    }

    #[test]
    fn closed_contours_preserve_explicit_and_implicit_seams() {
        let path = BezPath::from_svg("M1 2L4 6ZM10 20L12 20L10 20Z").unwrap();
        assert_eq!(
            flatten_contours(&path, 0.1).unwrap(),
            vec![
                Contour {
                    points: vec![[1.0, 2.0], [4.0, 6.0], [1.0, 2.0]],
                    closed: true
                },
                Contour {
                    points: vec![[10.0, 20.0], [12.0, 20.0], [10.0, 20.0]],
                    closed: true
                },
            ]
        );
        assert_eq!(path_length(&path).unwrap(), 14.0);
    }

    #[test]
    fn empty_and_move_only_subpaths_keep_their_distinction() {
        assert_eq!(flatten_contours(&BezPath::new(), 0.1).unwrap(), vec![]);
        assert_eq!(path_length(&BezPath::new()).unwrap(), 0.0);
        let mut path = BezPath::new();
        path.move_to((1.0, 2.0));
        path.move_to((3.0, 4.0));
        path.close_path();
        assert_eq!(
            flatten_contours(&path, 0.1).unwrap(),
            vec![
                Contour {
                    points: vec![[1.0, 2.0]],
                    closed: false
                },
                Contour {
                    points: vec![[3.0, 4.0]],
                    closed: true
                },
            ]
        );
    }

    #[test]
    fn nonfinite_controls_are_rejected_before_sampling_or_measurement() {
        for coordinate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut path = BezPath::new();
            path.move_to((0.0, 0.0));
            path.curve_to((coordinate, 1.0), (2.0, 3.0), (4.0, 5.0));
            assert_eq!(
                flatten_contours(&path, 0.1).unwrap_err(),
                "path coordinates must be finite"
            );
            assert_eq!(
                path_length(&path).unwrap_err(),
                "path coordinates must be finite"
            );
        }
    }

    #[test]
    fn invalid_flattening_tolerance_is_rejected_even_for_empty_paths() {
        for tolerance in [0.0, -0.1, f64::NAN, f64::INFINITY] {
            assert!(flatten_contours(&BezPath::new(), tolerance).is_err());
        }
    }
}
