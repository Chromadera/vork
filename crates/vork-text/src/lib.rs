//! Authoring-time text shaping and glyph outlines, in Y-up font units.
//! Bidi and script itemization adapted from moda-text (see PROVENANCE.md).
use anyhow::{Context, Result, ensure};
use harfrust::{Buffer, Direction, ShapeOptions, ShaperFont};
use lyon_path::{Path, math::point};

/// A positioned glyph in visual order. Clusters are UTF-8 byte offsets in its line.
#[derive(Debug, Clone)]
pub struct Glyph {
    pub id: u16,
    pub cluster: usize,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
}

/// One explicit line, measured in font units, including trailing whitespace.
#[derive(Debug, Clone)]
pub struct Line {
    pub glyphs: Vec<Glyph>,
    pub width: f32,
}

/// A borrowed TrueType/OpenType face; callers own font selection and font bytes.
pub struct Font<'a> {
    face: ttf_parser::Face<'a>,
    shaper: harfrust::Font,
}
impl<'a> Font<'a> {
    pub fn new(bytes: &'a [u8], index: u32) -> Result<Self> {
        Ok(Self {
            face: ttf_parser::Face::parse(bytes, index).context("parse text font")?,
            shaper: harfrust::Font::new(bytes.to_vec(), index).context("parse shaping font")?,
        })
    }
    pub fn units_per_em(&self) -> u16 {
        self.face.units_per_em() as u16
    }
    pub fn ascender(&self) -> i16 {
        self.face.ascender()
    }
    pub fn descender(&self) -> i16 {
        self.face.descender()
    }

    /// Shape explicit newline-separated lines with UAX #9 visual ordering and
    /// script itemization. OpenType defaults supply ligatures, kerning and marks.
    pub fn layout(&self, text: &str) -> Result<Vec<Line>> {
        text.split('\n').map(|line| self.shape_line(line)).collect()
    }
    fn shape_line(&self, text: &str) -> Result<Line> {
        for ch in text.chars() {
            ensure!(
                !ch.is_control(),
                "unsupported text control character {ch:?}"
            );
        }
        let mut glyphs = Vec::new();
        let mut pen = 0.;
        for (range, rtl) in visual_runs(text) {
            for run in script_runs(text, range, rtl) {
                let mut buffer = Buffer::new();
                buffer.push_str(&text[run.clone()]);
                buffer.set_direction(if rtl {
                    Direction::RightToLeft
                } else {
                    Direction::LeftToRight
                });
                buffer.guess_segment_properties();
                harfrust::shape(
                    &ShaperFont::new(&self.shaper),
                    &mut buffer,
                    ShapeOptions::new(),
                )
                .context("shape text")?;
                let shaped = buffer;
                for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                    ensure!(
                        info.glyph_id != 0,
                        "font has no glyph for cluster {:?}",
                        &text[run.start + info.cluster as usize..]
                    );
                    glyphs.push(Glyph {
                        id: info.glyph_id as u16,
                        cluster: run.start + info.cluster as usize,
                        x: pen + position.x_offset as f32,
                        y: position.y_offset as f32,
                        advance: position.x_advance as f32,
                    });
                    pen += position.x_advance as f32;
                }
            }
        }
        Ok(Line { glyphs, width: pen })
    }

    /// Unflattened outline at the glyph's positioned origin, in font units.
    pub fn outline(&self, glyph: &Glyph) -> Option<Path> {
        let mut outline = Outline {
            builder: Path::builder(),
            x: glyph.x,
            y: glyph.y,
        };
        self.face
            .outline_glyph(ttf_parser::GlyphId(glyph.id), &mut outline)?;
        Some(outline.builder.build())
    }
}
struct Outline {
    builder: lyon_path::path::Builder,
    x: f32,
    y: f32,
}
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.begin(point(x + self.x, y + self.y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(point(x + self.x, y + self.y));
    }
    fn quad_to(&mut self, x: f32, y: f32, ex: f32, ey: f32) {
        self.builder.quadratic_bezier_to(
            point(x + self.x, y + self.y),
            point(ex + self.x, ey + self.y),
        );
    }
    fn curve_to(&mut self, x: f32, y: f32, x2: f32, y2: f32, ex: f32, ey: f32) {
        self.builder.cubic_bezier_to(
            point(x + self.x, y + self.y),
            point(x2 + self.x, y2 + self.y),
            point(ex + self.x, ey + self.y),
        );
    }
    fn close(&mut self) {
        self.builder.end(true);
    }
}

fn script_runs(
    text: &str,
    range: std::ops::Range<usize>,
    rtl: bool,
) -> impl Iterator<Item = std::ops::Range<usize>> {
    use unicode_script::{Script, UnicodeScript};
    let mut spans = Vec::new();
    let mut start = range.start;
    let mut current = None;
    for (offset, character) in text[range.clone()].char_indices() {
        let script = character.script();
        if matches!(script, Script::Common | Script::Inherited | Script::Unknown) {
            continue;
        }
        if current.is_some_and(|current| current != script) {
            let end = range.start + offset;
            spans.push(start..end);
            start = end;
        }
        current = Some(script);
    }
    // Keep the only span inline; the first script transition above allocates the vector.
    let single = if spans.is_empty() {
        Some(start..range.end)
    } else {
        spans.push(start..range.end);
        None
    };
    if rtl {
        spans.reverse();
    }
    single.into_iter().chain(spans)
}

/// UAX #9 visual runs with the paragraph base inferred from its first strong character.
fn visual_runs(text: &str) -> Vec<(std::ops::Range<usize>, bool)> {
    let bidi = unicode_bidi::BidiInfo::new(text, None);
    let mut out = Vec::new();
    // Segment text is paragraph-free by construction (sanitize + paragraph
    // split upstream), but iterate defensively — same output either way.
    for para in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(para, para.range.clone());
        for run in runs {
            let rtl = levels[run.start].is_rtl();
            out.push((run, rtl));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_direction_scripts_are_itemized_and_marks_stay_with_base() {
        let text = "ab क्षि";
        assert_eq!(
            script_runs(text, 0..text.len(), false).collect::<Vec<_>>(),
            [0..3, 3..15]
        );
    }
}
