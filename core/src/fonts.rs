//! Embedded Liberation Sans/Mono fonts (SIL OFL, see `assets/fonts/LICENSE_LIBERATION`).
//!
//! printpdf is used with `default-features = false`, so `ParsedFont` is the
//! lightweight stub backed by `allsorts` (cmap + hmtx only) rather than the
//! full azul text-layout stack we don't need since layout is hand-rolled here.

use printpdf::ParsedFont;

use crate::error::Md2PdfError;

macro_rules! font_bytes {
    ($name:literal) => {
        include_bytes!(concat!("../assets/fonts/", $name))
    };
}

const SANS_REGULAR: &[u8] = font_bytes!("LiberationSans-Regular.ttf");
const SANS_BOLD: &[u8] = font_bytes!("LiberationSans-Bold.ttf");
const SANS_ITALIC: &[u8] = font_bytes!("LiberationSans-Italic.ttf");
const SANS_BOLD_ITALIC: &[u8] = font_bytes!("LiberationSans-BoldItalic.ttf");
const MONO_REGULAR: &[u8] = font_bytes!("LiberationMono-Regular.ttf");
const MONO_BOLD: &[u8] = font_bytes!("LiberationMono-Bold.ttf");
const MONO_ITALIC: &[u8] = font_bytes!("LiberationMono-Italic.ttf");
const MONO_BOLD_ITALIC: &[u8] = font_bytes!("LiberationMono-BoldItalic.ttf");

pub struct FontSet {
    pub sans_regular: ParsedFont,
    pub sans_bold: ParsedFont,
    pub sans_italic: ParsedFont,
    pub sans_bold_italic: ParsedFont,
    pub mono_regular: ParsedFont,
    pub mono_bold: ParsedFont,
    pub mono_italic: ParsedFont,
    pub mono_bold_italic: ParsedFont,
}

impl FontSet {
    pub fn load() -> Result<Self, Md2PdfError> {
        let parse = |bytes: &[u8]| -> Result<ParsedFont, Md2PdfError> {
            let mut warnings = Vec::new();
            ParsedFont::from_bytes(bytes, 0, &mut warnings)
                .ok_or_else(|| Md2PdfError::FontParse("failed to parse embedded font".to_string()))
        };
        Ok(FontSet {
            sans_regular: parse(SANS_REGULAR)?,
            sans_bold: parse(SANS_BOLD)?,
            sans_italic: parse(SANS_ITALIC)?,
            sans_bold_italic: parse(SANS_BOLD_ITALIC)?,
            mono_regular: parse(MONO_REGULAR)?,
            mono_bold: parse(MONO_BOLD)?,
            mono_italic: parse(MONO_ITALIC)?,
            mono_bold_italic: parse(MONO_BOLD_ITALIC)?,
        })
    }

    pub fn resolve(&self, bold: bool, italic: bool, code: bool) -> &ParsedFont {
        match (code, bold, italic) {
            (true, true, true) => &self.mono_bold_italic,
            (true, true, false) => &self.mono_bold,
            (true, false, true) => &self.mono_italic,
            (true, false, false) => &self.mono_regular,
            (false, true, true) => &self.sans_bold_italic,
            (false, true, false) => &self.sans_bold,
            (false, false, true) => &self.sans_italic,
            (false, false, false) => &self.sans_regular,
        }
    }
}

/// Sums per-glyph advance widths (hmtx table, scaled by `units_per_em`) — the
/// same measurement printpdf itself would use, computed up front so the
/// layout engine can wrap and size content before any drawing op is emitted.
pub fn text_width_pt(font: &ParsedFont, text: &str, size_pt: f32) -> f32 {
    let upm = f32::from(font.units_per_em.max(1));
    text.chars()
        .map(|c| {
            let gid = font.lookup_glyph_index(c as u32).unwrap_or(0);
            let w = f32::from(font.get_glyph_width(gid).unwrap_or(0));
            (w / upm) * size_pt
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_parses_every_embedded_font() {
        assert!(FontSet::load().is_ok());
    }

    #[test]
    fn resolve_picks_mono_for_code_and_sans_otherwise() {
        let fonts = FontSet::load().unwrap();
        // Liberation Mono is fixed-pitch; Liberation Sans is not.
        let narrow = |f: &ParsedFont| text_width_pt(f, "i", 10.0);
        let wide = |f: &ParsedFont| text_width_pt(f, "W", 10.0);
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let mono = fonts.resolve(bold, italic, true);
            assert!((narrow(mono) - wide(mono)).abs() < 0.01);
            let sans = fonts.resolve(bold, italic, false);
            assert!(narrow(sans) < wide(sans));
        }
    }

    #[test]
    fn resolve_returns_distinct_faces_for_each_style() {
        let fonts = FontSet::load().unwrap();
        let regular = fonts.resolve(false, false, false);
        let bold = fonts.resolve(true, false, false);
        assert!(std::ptr::eq(regular, &raw const fonts.sans_regular));
        assert!(std::ptr::eq(bold, &raw const fonts.sans_bold));
        assert!(std::ptr::eq(
            fonts.resolve(true, true, true),
            &raw const fonts.mono_bold_italic
        ));
        assert!(text_width_pt(bold, "Hello", 12.0) > text_width_pt(regular, "Hello", 12.0));
    }

    #[test]
    fn text_width_is_zero_for_empty_text() {
        let fonts = FontSet::load().unwrap();
        assert!(text_width_pt(&fonts.sans_regular, "", 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn text_width_scales_linearly_with_size_and_length() {
        let fonts = FontSet::load().unwrap();
        let font = &fonts.sans_regular;
        let one = text_width_pt(font, "abc", 10.0);
        assert!(one > 0.0);
        assert!((text_width_pt(font, "abc", 20.0) - 2.0 * one).abs() < 0.001);
        assert!((text_width_pt(font, "abcabc", 10.0) - 2.0 * one).abs() < 0.001);
    }

    #[test]
    fn mono_advance_is_six_tenths_of_an_em() {
        let fonts = FontSet::load().unwrap();
        let width = text_width_pt(&fonts.mono_regular, "x", 10.0);
        assert!((width - 6.0).abs() < 0.01, "got {width}");
    }
}
