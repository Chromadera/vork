// Display-encoded RGBA8 comparisons. One RGB code permits adjacent quantization
// bins at a rounding boundary; it is not a scene-linear radiance tolerance.
// Alpha is unaffected by the display program and must agree exactly.
fn mismatch(actual: &[u8], expected: &[u8]) -> Option<(usize, u8, u8)> {
    assert!(!actual.is_empty(), "missing display readback");
    assert_eq!(actual.len() % 4, 0, "incomplete RGBA pixel");
    assert_eq!(actual.len(), expected.len(), "display dimensions differ");
    actual
        .iter()
        .zip(expected)
        .enumerate()
        .find_map(|(i, (&a, &b))| {
            (a.abs_diff(b) > if i % 4 == 3 { 0 } else { 1 }).then_some((i, a, b))
        })
}

pub fn assert_display_eq(actual: &[u8], expected: &[u8]) {
    if let Some((i, a, b)) = mismatch(actual, expected) {
        panic!(
            "display pixel {} channel {}: {a} != {b} (RGB tolerance 1 code, alpha exact)",
            i / 4,
            i % 4
        );
    }
}

pub fn assert_display_ne(actual: &[u8], expected: &[u8]) {
    let difference = mismatch(actual, expected);
    assert!(
        difference.is_some(),
        "negative control must exceed the display equality tolerance"
    );
    eprintln!(
        "display negative control rejected: first mismatch {difference:?}, max code difference {}",
        actual
            .iter()
            .zip(expected)
            .map(|(&a, &b)| a.abs_diff(b))
            .max()
            .unwrap()
    );
}

#[test]
fn display_tolerance_detects_local_rgb_and_alpha_errors() {
    let reference = [0, 118, 254, 255, 32, 64, 128, 255];
    assert_display_eq(&[1, 117, 255, 255, 32, 64, 128, 255], &reference);
    for (channel, value) in [(4, 34), (7, 254)] {
        let mut wrong = reference;
        wrong[channel] = value;
        assert!(std::panic::catch_unwind(|| assert_display_eq(&wrong, &reference)).is_err());
    }
}
