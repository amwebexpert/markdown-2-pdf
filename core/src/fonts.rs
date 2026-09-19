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
    let upm = (font.units_per_em.max(1)) as f32;
    text.chars()
        .map(|c| {
            let gid = font.lookup_glyph_index(c as u32).unwrap_or(0);
            let w = font.get_glyph_width(gid).unwrap_or(0) as f32;
            (w / upm) * size_pt
        })
        .sum()
}
