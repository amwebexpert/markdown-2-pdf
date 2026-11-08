//! PDF font dictionaries: character code → Unicode, glyph advances, and the
//! bold/italic/monospace flags the structure pass relies on.

use std::collections::HashMap;

use lopdf::{Dictionary, Document, Encoding, Object};

/// Fallback advance (glyph-space / 1000) when a font carries no width table,
/// e.g. the standard 14 fonts.
const FALLBACK_WIDTH: f32 = 0.5;
const FALLBACK_MONO_WIDTH: f32 = 0.6;

// FontDescriptor /Flags bits (PDF 32000-1 §9.8.2).
const FLAG_FIXED_PITCH: i64 = 1;
const FLAG_ITALIC: i64 = 1 << 6;
const FLAG_FORCE_BOLD: i64 = 1 << 18;
/// Vertical stem width from which a face reads as bold (regular faces sit
/// around 70-90, bold ones above 120).
const BOLD_STEM_V: f32 = 120.0;
/// Width tables with at least this many distinct non-zero glyphs, all equal,
/// identify a monospace face even when its name is an opaque subset tag.
const MIN_MONO_SAMPLES: usize = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
}

#[derive(Debug)]
pub struct Font<'a> {
    pub style: FontStyle,
    encoding: Option<Encoding<'a>>,
    /// Type0 fonts use 2-byte codes (Identity-H and nearly every embedded CID font).
    two_byte: bool,
    widths: HashMap<u32, f32>,
    default_width: f32,
}

impl<'a> Font<'a> {
    pub fn load(doc: &'a Document, dict: &'a Dictionary) -> Self {
        let subtype = name(dict, b"Subtype").unwrap_or_default();
        let two_byte = subtype == b"Type0";
        let descendant = two_byte
            .then(|| descendant_font(doc, dict))
            .flatten()
            .unwrap_or(dict);
        let descriptor = descendant
            .get_deref(b"FontDescriptor", doc)
            .and_then(Object::as_dict)
            .ok();

        let base_font = name(dict, b"BaseFont").unwrap_or_default();
        let mut style = style_from(base_font, descriptor);

        let (widths, default_width) = if two_byte {
            cid_widths(doc, descendant)
        } else {
            simple_widths(doc, dict, descriptor, style.mono)
        };
        style.mono |= uniform_widths(&widths);

        Self {
            style,
            encoding: encoding(doc, dict),
            two_byte,
            widths,
            default_width,
        }
    }

    /// Splits a shown string into `(code, byte length)` pairs.
    pub fn codes<'b>(&self, bytes: &'b [u8]) -> impl Iterator<Item = (u32, usize)> + 'b {
        let step = if self.two_byte { 2 } else { 1 };
        bytes.chunks(step).map(|chunk| {
            let code = chunk
                .iter()
                .fold(0u32, |acc, byte| (acc << 8) | u32::from(*byte));
            (code, chunk.len())
        })
    }

    /// Advance of `code` in text space units (1 = font size).
    pub fn width(&self, code: u32) -> f32 {
        self.widths
            .get(&code)
            .copied()
            .unwrap_or(self.default_width)
    }

    /// Unicode text for one character code; empty when it can't be decoded.
    pub fn decode(&self, code: u32, len: usize) -> String {
        let Some(encoding) = &self.encoding else {
            return String::new();
        };
        let bytes = code.to_be_bytes();
        let start = bytes.len().saturating_sub(len);
        let Some(code_bytes) = bytes.get(start..) else {
            return String::new();
        };
        encoding
            .bytes_to_string(code_bytes)
            .map(|text| expand_ligatures(&text.replace('\u{FFFD}', "")))
            .unwrap_or_default()
    }
}

/// Typographic ligatures (U+FB00 block) back to plain letters.
fn expand_ligatures(text: &str) -> String {
    if !text.chars().any(|c| ('\u{FB00}'..='\u{FB06}').contains(&c)) {
        return text.to_string();
    }
    text.chars()
        .map(|c| match c {
            '\u{FB00}' => "ff".to_string(),
            '\u{FB01}' => "fi".to_string(),
            '\u{FB02}' => "fl".to_string(),
            '\u{FB03}' => "ffi".to_string(),
            '\u{FB04}' => "ffl".to_string(),
            '\u{FB05}' | '\u{FB06}' => "st".to_string(),
            other => other.to_string(),
        })
        .collect()
}

fn name<'a>(dict: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    dict.get(key).and_then(Object::as_name).ok()
}

fn number(object: &Object) -> Option<f32> {
    object.as_float().ok()
}

fn descendant_font<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
    let descendants = dict
        .get_deref(b"DescendantFonts", doc)
        .ok()?
        .as_array()
        .ok()?;
    let first = descendants.first()?;
    doc.dereference(first).ok()?.1.as_dict().ok()
}

/// Prefers `/ToUnicode` (authoritative for extraction) over `/Encoding`.
/// lopdf consults `/Encoding` first, so the `CMap` is reached by resolving the
/// encoding of a copy of the font dictionary with `/Encoding` removed.
fn encoding<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<Encoding<'a>> {
    if dict.has(b"ToUnicode") {
        let mut without_encoding = dict.clone();
        without_encoding.remove(b"Encoding");
        if let Ok(Encoding::UnicodeMapEncoding(cmap)) = without_encoding.get_font_encoding(doc) {
            return Some(Encoding::UnicodeMapEncoding(cmap));
        }
    }
    dict.get_font_encoding(doc).ok()
}

fn style_from(base_font: &[u8], descriptor: Option<&Dictionary>) -> FontStyle {
    // Drop the subset tag: "ABCDEF+Helvetica-Bold" → "helvetica-bold".
    let full = String::from_utf8_lossy(base_font);
    let original = full.split_once('+').map_or(full.as_ref(), |(_, rest)| rest);
    let font_name = original.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| font_name.contains(needle));

    let flags = descriptor
        .and_then(|d| d.get(b"Flags").and_then(Object::as_i64).ok())
        .unwrap_or(0);
    let italic_angle = descriptor
        .and_then(|d| d.get(b"ItalicAngle").ok().and_then(number))
        .unwrap_or(0.0);
    let weight = descriptor
        .and_then(|d| d.get(b"FontWeight").ok().and_then(number))
        .unwrap_or(0.0);
    // StemV is only trusted for opaque generated names (e.g. printpdf's
    // all-uppercase tags); real family names are better evidence, and some
    // writers (Chrome) store arbitrary StemV values.
    let opaque = !original.chars().any(|c| c.is_ascii_lowercase() || c == '-');
    let stem_v = descriptor
        .filter(|_| opaque)
        .and_then(|d| d.get(b"StemV").ok().and_then(number))
        .unwrap_or(0.0);

    FontStyle {
        bold: has(&["bold", "black", "heavy", "semibold", "demi", "cmbx"])
            || flags & FLAG_FORCE_BOLD != 0
            || weight >= 600.0
            || stem_v >= BOLD_STEM_V,
        italic: has(&["italic", "oblique", "cmti", "cmsl"])
            || flags & FLAG_ITALIC != 0
            || italic_angle.abs() > 0.5,
        mono: has(&[
            "mono",
            "courier",
            "code",
            "consol",
            "menlo",
            "typewriter",
            "cmtt",
        ]) || flags & FLAG_FIXED_PITCH != 0,
    }
}

fn uniform_widths(widths: &HashMap<u32, f32>) -> bool {
    let mut non_zero = widths.values().filter(|w| **w > 0.0);
    let Some(first) = non_zero.next() else {
        return false;
    };
    let mut count = 1;
    for w in non_zero {
        if (w - first).abs() > 0.001 {
            return false;
        }
        count += 1;
    }
    count >= MIN_MONO_SAMPLES
}

fn simple_widths(
    doc: &Document,
    dict: &Dictionary,
    descriptor: Option<&Dictionary>,
    mono: bool,
) -> (HashMap<u32, f32>, f32) {
    // Type3 glyph space is defined by /FontMatrix instead of the usual 1/1000.
    let scale = dict
        .get(b"FontMatrix")
        .and_then(Object::as_array)
        .ok()
        .and_then(|m| m.first().and_then(number))
        .unwrap_or(0.001);
    let first_char = dict
        .get(b"FirstChar")
        .and_then(Object::as_i64)
        .ok()
        .and_then(|c| u32::try_from(c).ok())
        .unwrap_or(0);
    let widths: HashMap<u32, f32> = dict
        .get_deref(b"Widths", doc)
        .and_then(Object::as_array)
        .map(|array| {
            (first_char..)
                .zip(array)
                .filter_map(|(code, w)| {
                    let w = doc.dereference(w).ok().and_then(|(_, o)| number(o))?;
                    Some((code, w * scale))
                })
                .collect()
        })
        .unwrap_or_default();
    let missing = descriptor
        .and_then(|d| d.get(b"MissingWidth").ok().and_then(number))
        .map(|w| w * scale)
        .filter(|w| *w > 0.0);
    let default = missing.unwrap_or(if mono {
        FALLBACK_MONO_WIDTH
    } else {
        FALLBACK_WIDTH
    });
    (widths, default)
}

/// Parses a `CIDFont` `/W` array: `c [w1 w2 …]` or `c_first c_last w` entries.
fn cid_widths(doc: &Document, descendant: &Dictionary) -> (HashMap<u32, f32>, f32) {
    let default = descendant
        .get(b"DW")
        .ok()
        .and_then(number)
        .unwrap_or(1000.0)
        / 1000.0;
    let mut widths = HashMap::new();
    let Ok(array) = descendant.get_deref(b"W", doc).and_then(Object::as_array) else {
        return (widths, default);
    };
    let resolve = |o: &Object| doc.dereference(o).map(|(_, o)| o.clone()).ok();
    let items: Vec<Object> = array.iter().filter_map(resolve).collect();
    let mut i = 0;
    while let Some(first) = items.get(i).and_then(|o| o.as_i64().ok()) {
        let Ok(first) = u32::try_from(first) else {
            break;
        };
        match items.get(i + 1) {
            Some(Object::Array(list)) => {
                for (code, w) in (first..).zip(list) {
                    if let Some(w) = resolve(w).as_ref().and_then(number) {
                        widths.insert(code, w / 1000.0);
                    }
                }
                i += 2;
            }
            Some(last) => {
                let last = last.as_i64().ok().and_then(|l| u32::try_from(l).ok());
                let w = items.get(i + 2).and_then(number);
                let (Some(last), Some(w)) = (last, w) else {
                    break;
                };
                for code in first..=last.min(first.saturating_add(0xFFFF)) {
                    widths.insert(code, w / 1000.0);
                }
                i += 3;
            }
            None => break,
        }
    }
    (widths, default)
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    #[test]
    fn style_reads_the_base_font_name_without_subset_tag() {
        let style = style_from(b"ABCDEF+LiberationMono-BoldItalic", None);
        let wide_stem = dictionary! { "StemV" => 250 };
        assert!(!style_from(b"DAAAAA+HelveticaNeue-Italic", Some(&wide_stem)).bold);
        assert_eq!(
            style,
            FontStyle {
                bold: true,
                italic: true,
                mono: true
            }
        );
        assert_eq!(style_from(b"Helvetica", None), FontStyle::default());
    }

    #[test]
    fn style_falls_back_to_descriptor_flags() {
        let descriptor = dictionary! { "Flags" => FLAG_FIXED_PITCH | FLAG_ITALIC };
        let style = style_from(b"F1", Some(&descriptor));
        assert!(style.mono && style.italic && !style.bold);
    }

    #[test]
    fn style_reads_bold_from_a_wide_vertical_stem() {
        let descriptor = dictionary! { "StemV" => 166 };
        assert!(style_from(b"HGJBDAIG", Some(&descriptor)).bold);
    }

    #[test]
    fn ligatures_are_expanded() {
        assert_eq!(expand_ligatures("\u{FB01}eld \u{FB03}x"), "field ffix");
    }

    #[test]
    fn equal_widths_mean_monospace() {
        let mono: HashMap<u32, f32> = [(1, 0.6), (2, 0.6), (3, 0.6), (4, 0.0)].into();
        let proportional: HashMap<u32, f32> = [(1, 0.6), (2, 0.5), (3, 0.6)].into();
        assert!(uniform_widths(&mono));
        assert!(!uniform_widths(&proportional));
    }

    #[test]
    fn cid_widths_handles_both_w_array_forms() {
        let doc = Document::with_version("1.7");
        let descendant = dictionary! {
            "DW" => 500,
            "W" => vec![
                3.into(), vec![Object::Integer(250), Object::Integer(300)].into(),
                10.into(), 12.into(), 700.into(),
            ],
        };
        let (widths, default) = cid_widths(&doc, &descendant);
        assert!((default - 0.5).abs() < f32::EPSILON);
        assert_eq!(widths.get(&4), Some(&0.3));
        assert_eq!(widths.get(&11), Some(&0.7));
        assert_eq!(widths.get(&13), None);
    }

    #[test]
    fn simple_widths_start_at_first_char() {
        let doc = Document::with_version("1.7");
        let dict = dictionary! {
            "FirstChar" => 32,
            "Widths" => vec![Object::Integer(278), Object::Integer(556)],
        };
        let (widths, _) = simple_widths(&doc, &dict, None, false);
        assert!(widths.get(&33).is_some_and(|w| (w - 0.556).abs() < 1e-6));
    }
}
