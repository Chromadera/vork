//! SVG path geometry, independent of scene types and rendering.
use anyhow::{Result, ensure};
pub mod dash;
pub mod path;
pub use lyon_path::{FillRule, LineCap, LineJoin, Path};
use lyon_path::{geom::ArcFlags, math::point};
pub use lyon_tessellation::{StrokeOptions, VertexBuffers};
use svgtypes::{PathParser, PathSegment::*};

/// Parse SVG path data without changing its coordinate system. Curves remain curves.
pub fn parse(d: &str) -> Result<Path> {
    let mut b = Path::builder().with_svg();
    let mut current = [0f64; 2];
    let mut first = current;
    let mut cubic: Option<[f64; 2]> = None;
    let mut quadratic: Option<[f64; 2]> = None;
    fn p([x, y]: [f64; 2]) -> Result<lyon_path::math::Point> {
        let p = point(x as f32, y as f32);
        ensure!(
            p.x.is_finite() && p.y.is_finite(),
            "SVG coordinates must be finite f32 values"
        );
        Ok(p)
    }
    for segment in PathParser::from(d) {
        let segment = segment?;
        let absolute = |abs: bool, x: f64, y: f64| {
            if abs {
                [x, y]
            } else {
                [current[0] + x, current[1] + y]
            }
        };
        let reflect = |ctrl: Option<[f64; 2]>| {
            ctrl.map(|c| [2. * current[0] - c[0], 2. * current[1] - c[1]])
                .unwrap_or(current)
        };
        let mut next_cubic = None;
        let mut next_quadratic = None;
        match segment {
            MoveTo { abs, x, y } => {
                current = absolute(abs, x, y);
                b.move_to(p(current)?);
                first = current;
            }
            LineTo { abs, x, y } => {
                current = absolute(abs, x, y);
                b.line_to(p(current)?);
            }
            HorizontalLineTo { abs, x } => {
                current[0] = if abs { x } else { current[0] + x };
                b.line_to(p(current)?);
            }
            VerticalLineTo { abs, y } => {
                current[1] = if abs { y } else { current[1] + y };
                b.line_to(p(current)?);
            }
            CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let c1 = absolute(abs, x1, y1);
                let c2 = absolute(abs, x2, y2);
                let to = absolute(abs, x, y);
                b.cubic_bezier_to(p(c1)?, p(c2)?, p(to)?);
                current = to;
                next_cubic = Some(c2);
            }
            SmoothCurveTo { abs, x2, y2, x, y } => {
                let c1 = reflect(cubic);
                let c2 = absolute(abs, x2, y2);
                let to = absolute(abs, x, y);
                b.cubic_bezier_to(p(c1)?, p(c2)?, p(to)?);
                current = to;
                next_cubic = Some(c2);
            }
            Quadratic { abs, x1, y1, x, y } => {
                let c = absolute(abs, x1, y1);
                let to = absolute(abs, x, y);
                b.quadratic_bezier_to(p(c)?, p(to)?);
                current = to;
                next_quadratic = Some(c);
            }
            SmoothQuadratic { abs, x, y } => {
                let c = reflect(quadratic);
                let to = absolute(abs, x, y);
                b.quadratic_bezier_to(p(c)?, p(to)?);
                current = to;
                next_quadratic = Some(c);
            }
            EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                let to = absolute(abs, x, y);
                let endpoint = p(to)?;
                ensure!(
                    rx.is_finite() && ry.is_finite() && x_axis_rotation.is_finite(),
                    "SVG arc parameters must be finite"
                );
                let radii = p([rx, ry])?;
                let arc = lyon_path::geom::SvgArc::<f64> {
                    from: lyon_path::geom::Point::new(current[0], current[1]),
                    to: lyon_path::geom::Point::new(to[0], to[1]),
                    radii: lyon_path::geom::Vector::new(
                        f64::from(radii.x.abs()),
                        f64::from(radii.y.abs()),
                    ),
                    x_rotation: lyon_path::geom::Angle::degrees(x_axis_rotation % 360.),
                    flags: ArcFlags { large_arc, sweep },
                };
                // Bound radii before geometric arithmetic, including SVG radius correction.
                if p(current)? == endpoint {
                    // SVG omits coincident-endpoint arcs.
                } else if arc.is_straight_line() {
                    b.line_to(endpoint);
                } else {
                    let arc = arc.to_arc();
                    // Cubics spanning at most 22.5 degrees have conversion error
                    // below 1e-6 of the radius, before downstream flattening.
                    let steps = (arc.sweep_angle.radians.abs() / (std::f64::consts::PI / 8.)).ceil()
                        as usize;
                    for i in 0..steps {
                        let part =
                            arc.split_range(i as f64 / steps as f64..(i + 1) as f64 / steps as f64);
                        let mut curves = Vec::new();
                        part.for_each_cubic_bezier(&mut |c| curves.push(*c));
                        for c in curves {
                            b.cubic_bezier_to(
                                p([c.ctrl1.x, c.ctrl1.y])?,
                                p([c.ctrl2.x, c.ctrl2.y])?,
                                if i + 1 == steps {
                                    endpoint
                                } else {
                                    p([c.to.x, c.to.y])?
                                },
                            );
                        }
                    }
                }
                current = to;
            }
            ClosePath { .. } => {
                b.close();
                current = first;
            }
        }
        cubic = next_cubic;
        quadratic = next_quadratic;
    }
    Ok(b.build())
}

/// Flattened subpaths retain whether the author explicitly closed them.
#[derive(Debug, Clone)]
pub struct Contour {
    pub points: Vec<[f32; 2]>,
    pub closed: bool,
}

pub fn flatten(path: &Path, tolerance: f32) -> Result<Vec<Contour>> {
    let mut bez = kurbo::BezPath::new();
    for event in path.iter() {
        let p = |p: lyon_path::math::Point| kurbo::Point::new(f64::from(p.x), f64::from(p.y));
        match event {
            lyon_path::Event::Begin { at } => bez.move_to(p(at)),
            lyon_path::Event::Line { to, .. } => bez.line_to(p(to)),
            lyon_path::Event::Quadratic { ctrl, to, .. } => bez.quad_to(p(ctrl), p(to)),
            lyon_path::Event::Cubic {
                ctrl1, ctrl2, to, ..
            } => bez.curve_to(p(ctrl1), p(ctrl2), p(to)),
            lyon_path::Event::End { close: true, .. } => bez.close_path(),
            _ => {}
        }
    }
    Ok(path::flatten_contours(&bez, f64::from(tolerance))
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|c| Contour {
            points: c
                .points
                .into_iter()
                .map(|[x, y]| [x as f32, y as f32])
                .collect(),
            closed: c.closed,
        })
        .collect())
}

pub type Triangles = VertexBuffers<[f32; 2], u32>;

/// Tessellate fills, including intersecting and nested subpaths.
pub fn fill(path: &Path, rule: FillRule, tolerance: f32) -> Result<Triangles> {
    use lyon_tessellation::*;
    ensure!(
        tolerance.is_finite() && tolerance > 0.,
        "tolerance must be finite and positive"
    );
    let mut mesh = Triangles::new();
    FillTessellator::new().tessellate_path(
        path,
        &FillOptions::default()
            .with_fill_rule(rule)
            .with_tolerance(tolerance),
        &mut BuffersBuilder::new(&mut mesh, |v: FillVertex| v.position().to_array()),
    )?;
    Ok(mesh)
}

/// Tessellate a stroke with Lyon's cap, join, miter and tolerance controls.
pub fn stroke(path: &Path, options: &StrokeOptions) -> Result<Triangles> {
    use lyon_tessellation::*;
    ensure!(
        options.line_width.is_finite() && options.line_width >= 0.,
        "stroke width must be finite and nonnegative"
    );
    ensure!(
        options.tolerance.is_finite() && options.tolerance > 0.,
        "tolerance must be finite and positive"
    );
    ensure!(
        options.miter_limit.is_finite() && options.miter_limit >= 1.,
        "miter limit must be finite and at least 1"
    );
    let mut mesh = Triangles::new();
    StrokeTessellator::new().tessellate_path(
        path,
        options,
        &mut BuffersBuilder::new(&mut mesh, |v: StrokeVertex| v.position().to_array()),
    )?;
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area(mesh: &Triangles) -> f64 {
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [
                    mesh.vertices[t[0] as usize],
                    mesh.vertices[t[1] as usize],
                    mesh.vertices[t[2] as usize],
                ];
                f64::from((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.
            })
            .sum()
    }
    #[test]
    fn relative_repeated_and_smooth_commands_match_absolute() {
        let cases = [
            ("m1 2 3 0 0 4z", "M1 2L4 2L4 6Z"),
            ("M1 2h3v4h-3z", "M1 2L4 2L4 6L1 6Z"),
            ("M0 0q1 2 2 0t2 0", "M0 0Q1 2 2 0Q3-2 4 0"),
            ("M0 0c1 2 2 2 3 0s2-2 3 0", "M0 0C1 2 2 2 3 0C4-2 5-2 6 0"),
            (
                "M1 0a1 1 0 1 1-2 0a1 1 0 1 1 2 0z",
                "M1 0A1 1 0 1 1-1 0A1 1 0 1 1 1 0Z",
            ),
            ("M0 0h1v1zl2 0", "M0 0L1 0L1 1ZM0 0L2 0"),
        ];
        for (a, b) in cases {
            assert_eq!(
                parse(a).unwrap().iter().collect::<Vec<_>>(),
                parse(b).unwrap().iter().collect::<Vec<_>>(),
                "{a}"
            );
        }
    }
    #[test]
    fn fill_rules_distinguish_same_winding_holes() {
        let p = parse("M0 0H4V4H0Z M1 1H3V3H1Z").unwrap();
        assert!((area(&fill(&p, FillRule::NonZero, 0.001).unwrap()) - 16.).abs() < 1e-5);
        assert!((area(&fill(&p, FillRule::EvenOdd, 0.001).unwrap()) - 12.).abs() < 1e-5);
        let p = parse("M0 0H4V4H0Z M1 1V3H3V1Z").unwrap();
        assert!((area(&fill(&p, FillRule::NonZero, 0.001).unwrap()) - 12.).abs() < 1e-5);
    }
    #[test]
    fn arcs_and_degenerate_arcs_follow_svg() {
        let p = parse("M1 0 A1 1 0 1 1-1 0 A1 1 0 1 1 1 0Z").unwrap();
        assert!(
            (area(&fill(&p, FillRule::NonZero, 0.0001).unwrap()) - std::f64::consts::PI).abs()
                < 0.002
        );
        assert_eq!(
            parse("M0 0A0 1 0 0 0 2 0")
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            parse("M0 0L2 0").unwrap().iter().collect::<Vec<_>>()
        );
    }
    #[test]
    fn dashes_reset_per_subpath_and_preserve_closed_seams() {
        let options = StrokeOptions::default().with_line_width(1.);
        let p = parse("M0 0H10 M0 3H10").unwrap();
        assert!((area(&stroke_dashed(&p, &options, &[2., 2.], 0.).unwrap()) - 12.).abs() < 1e-5);
        assert!((area(&stroke_dashed(&p, &options, &[2.], -1.).unwrap()) - 10.).abs() < 1e-5);
        let p = parse("M0 0H4V4H0Z").unwrap();
        let solid = stroke(&p, &options).unwrap();
        assert_eq!(
            solid.vertices,
            stroke_dashed(&p, &options, &[], 0.).unwrap().vertices
        );
        assert!(
            (area(&solid) - area(&stroke_dashed(&p, &options, &[3., 0.], 0.).unwrap())).abs()
                < 1e-5
        );
        let joined = stroke_dashed(&p, &options, &[6., 2.], 1.).unwrap();
        assert!(joined.vertices.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn miter_limit_clips_acute_corners_and_arcs_reset_smooth_controls() {
        let p = parse("M0 0L1 0L0 .1").unwrap();
        let options = StrokeOptions::default().with_line_width(0.1);
        let bound = |m: Triangles| {
            m.vertices
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
        };
        assert!(
            bound(stroke(&p, &options.with_miter_limit(100.)).unwrap())
                > bound(stroke(&p, &options.with_miter_limit(1.)).unwrap()) + 0.5
        );
        for (a, b) in [
            (
                "M0 0A1 1 0 0 1 1 1S2 2 3 1",
                "M0 0A1 1 0 0 1 1 1C1 1 2 2 3 1",
            ),
            ("M0 0A1 1 0 0 1 1 1T2 0", "M0 0A1 1 0 0 1 1 1Q1 1 2 0"),
        ] {
            assert_eq!(
                parse(a).unwrap().iter().collect::<Vec<_>>(),
                parse(b).unwrap().iter().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn arcs_at_f32_precision_do_not_lose_their_endpoint() {
        let tiny = parse("M0 0A1e-300 1e-300 0 0 0 1 0").unwrap();
        assert_eq!(
            tiny.iter().collect::<Vec<_>>(),
            parse("M0 0L1 0").unwrap().iter().collect::<Vec<_>>()
        );
        for d in ["M0 0A1e-40 1e-40 0 0 1 1 0", "M0 0A1 1 0 0 1 1e-300 0"] {
            let p = parse(d).unwrap();
            assert!(
                flatten(&p, 0.01)
                    .unwrap()
                    .iter()
                    .flat_map(|c| &c.points)
                    .flatten()
                    .all(|v| v.is_finite())
            );
        }
    }

    #[test]
    fn caps_joins_and_validation() {
        let p = parse("M0 0L4 0").unwrap();
        let base = StrokeOptions::default().with_line_width(2.);
        assert!((area(&stroke(&p, &base).unwrap()) - 8.).abs() < 1e-5);
        assert!(
            (area(&stroke(&p, &base.with_line_cap(LineCap::Square)).unwrap()) - 12.).abs() < 1e-5
        );
        assert!(
            (area(
                &stroke(
                    &p,
                    &base.with_line_cap(LineCap::Round).with_tolerance(0.0001)
                )
                .unwrap()
            ) - 8.
                - std::f64::consts::PI)
                .abs()
                < 0.002
        );
        for join in [LineJoin::Miter, LineJoin::Bevel, LineJoin::Round] {
            assert!(
                !stroke(&parse("M0 0L2 0L2 2").unwrap(), &base.with_line_join(join))
                    .unwrap()
                    .indices
                    .is_empty()
            );
        }
        for d in [
            "L0 0",
            "M0",
            "M0 0X1 2",
            "M0 0A1 1 0 2 0 2 2",
            "M1e100 0",
            "M3e38 0l3e38 0",
            "M3e38 0a1 1 0 0 0 3e38 0",
            "M0 0Q1 2",
        ] {
            assert!(parse(d).is_err(), "{d}");
        }
        assert!(fill(&p, FillRule::NonZero, 0.).is_err());
        assert!(stroke(&p, &base.with_line_width(f32::NAN)).is_err());
    }
}

/// Stroke SVG dashes. Odd arrays repeat, phase restarts per subpath, and a
/// closed seam inside a dash stays joined. An empty pattern is a solid stroke.
pub fn stroke_dashed(
    path: &Path,
    options: &StrokeOptions,
    pattern: &[f64],
    offset: f64,
) -> Result<Triangles> {
    if pattern.is_empty() {
        ensure!(offset.is_finite(), "dash offset must be finite");
        return stroke(path, options);
    }
    dash::windows(0., false, pattern, offset).map_err(anyhow::Error::msg)?;
    let contours = flatten(path, options.tolerance)?;
    let mut builder = Path::builder();
    for contour in contours {
        let mut points: Vec<_> = contour
            .points
            .into_iter()
            .map(|[x, y]| [f64::from(x), f64::from(y)])
            .collect();
        if contour.closed && points.last() != points.first() {
            if let Some(first) = points.first().copied() {
                points.push(first);
            }
        }
        let length = path::chord_length(&points);
        let windows =
            dash::windows(length, contour.closed, pattern, offset).map_err(anyhow::Error::msg)?;
        for window in windows {
            let whole = contour.closed && window.start == 0. && window.end == length;
            let mut begun = false;
            // A merged closed-seam window can extend past the contour's length.
            for turn in 0..=usize::from(window.end > length) {
                let mut along = turn as f64 * length;
                for pair in points.windows(2) {
                    let distance = (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
                    let start = along.max(window.start);
                    let end = (along + distance).min(window.end);
                    if end > start {
                        let at = |s: f64| {
                            let t = (s - along) / distance;
                            point(
                                (pair[0][0] + t * (pair[1][0] - pair[0][0])) as f32,
                                (pair[0][1] + t * (pair[1][1] - pair[0][1])) as f32,
                            )
                        };
                        if !begun {
                            builder.begin(at(start));
                            begun = true;
                        }
                        builder.line_to(at(end));
                    }
                    along += distance;
                }
            }
            if begun {
                builder.end(whole);
            }
        }
    }
    stroke(&builder.build(), options)
}
