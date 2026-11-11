use vork_text::Font;
const ARABIC: &[u8] = include_bytes!("fonts/NotoSansArabic-Regular.ttf");
const DEVANAGARI: &[u8] = include_bytes!("fonts/NotoSansDevanagari-Regular.ttf");

#[test]
fn arabic_and_devanagari_sequences() {
    // Pinned Noto glyph sequences: Arabic lam-alef ligature in visual RTL order,
    // and the Devanagari pre-base vowel preceding the ksha conjunct.
    for (bytes, text, expected) in [
        (ARABIC, "سلام", vec![(770, 6), (705, 2), (1083, 0)]),
        (DEVANAGARI, "क्षि", vec![(610, 0), (179, 0)]),
    ] {
        let font = Font::new(bytes, 0).unwrap();
        let lines = font.layout(text).unwrap();
        let glyphs = &lines[0].glyphs;
        assert_eq!(
            glyphs.iter().map(|g| (g.id, g.cluster)).collect::<Vec<_>>(),
            expected
        );
        assert!(glyphs.iter().all(|g| font.outline(g).is_some()));
    }
}

#[test]
fn mixed_bidi_keeps_digits_ltr_and_arabic_rtl() {
    let font = Font::new(ARABIC, 0).unwrap();
    let text = "12 سلام 34";
    let line = font.layout(text).unwrap().remove(0);
    let clusters: Vec<_> = line.glyphs.iter().map(|g| g.cluster).collect();
    assert_eq!(clusters, [12, 13, 11, 9, 5, 3, 2, 0, 1]);
    assert!(line.width > 0.);
    assert!(line.glyphs.windows(2).all(|g| g[1].x >= g[0].x));
}

#[test]
fn explicit_lines_and_invalid_inputs() {
    let font = Font::new(DEVANAGARI, 0).unwrap();
    let lines = font.layout("क्षि\n\nक्षि").unwrap();
    assert_eq!(lines.len(), 3);
    assert!(lines[1].glyphs.is_empty());
    assert_eq!(lines[1].width, 0.);
    assert_eq!(lines[0].width, lines[2].width);
    assert!(font.layout("\t").is_err());
    assert!(font.layout("\u{10ffff}").is_err());
    assert!(Font::new(b"invalid font", 0).is_err());
}
