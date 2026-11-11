// Shared dash-window generation; see ../PROVENANCE.md.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcWindow {
    pub subpath: usize,
    pub start: f64,
    pub end: f64,
}

// Shared dash windows and typed coverage classification.

pub fn windows(
    length: f64,
    closed: bool,
    pattern: &[f64],
    offset: f64,
) -> Result<Vec<ArcWindow>, String> {
    if !length.is_finite() || length < 0. {
        return Err("dash path length must be finite and nonnegative".into());
    }
    let repeat = if pattern.len() % 2 == 1 { pattern } else { &[] };
    let period: f64 = pattern.iter().chain(repeat).sum();
    if pattern.is_empty()
        || pattern
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        || !period.is_finite()
        || period <= 0.0
        || !offset.is_finite()
    {
        return Err(
            "Use finite nonnegative dash lengths with a positive sum and finite dashOffset".into(),
        );
    }
    let mut phase = offset.rem_euclid(period);
    let mut index = 0;
    let mut drawing = true;
    while phase >= pattern[index] {
        phase -= pattern[index];
        index = (index + 1) % pattern.len();
        drawing = !drawing;
    }
    let mut remaining = pattern[index] - phase;
    let mut position = 0.0;
    let mut windows: Vec<ArcWindow> = Vec::new();
    let mut steps = 0;
    while position < length {
        steps += 1;
        if steps > 400_000 {
            return Err(
                "Dash pattern requires too many stroke pieces; use longer dash lengths".into(),
            );
        }
        let end = (position + remaining).min(length);
        if drawing && end > position {
            match windows.last_mut() {
                Some(previous) if previous.end == position => previous.end = end,
                _ => windows.push(ArcWindow {
                    subpath: 0,
                    start: position,
                    end,
                }),
            }
        }
        position = end;
        index = (index + 1) % pattern.len();
        drawing = !drawing;
        remaining = pattern[index];
    }
    if closed
        && windows.len() > 1
        && windows[0].start == 0.0
        && windows.last().unwrap().end == length
    {
        let tail = windows.pop().unwrap();
        windows[0].start = tail.start;
        windows[0].end += length;
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn odd_patterns_offsets_and_closed_seams() {
        let bounds =
            |w: Vec<ArcWindow>| w.into_iter().map(|w| (w.start, w.end)).collect::<Vec<_>>();
        assert_eq!(
            bounds(windows(6., false, &[2.], 0.).unwrap()),
            vec![(0., 2.), (4., 6.)]
        );
        assert_eq!(
            bounds(windows(6., false, &[2., 2.], -1.).unwrap()),
            vec![(1., 3.), (5., 6.)]
        );
        assert_eq!(
            bounds(windows(8., true, &[3., 1.], 1.).unwrap()),
            vec![(7., 10.), (3., 6.)]
        );
        assert_eq!(
            bounds(windows(8., true, &[3., 0.], 0.).unwrap()),
            vec![(0., 8.)]
        );
        for pattern in [vec![], vec![0.], vec![-1., 2.], vec![f64::NAN, 1.]] {
            assert!(windows(0., false, &pattern, 0.).is_err());
        }
        assert!(windows(1., false, &[1., 1.], f64::INFINITY).is_err());
        assert!(windows(10., false, &[0.000001, 0.000001], 0.).is_err());
    }
}
