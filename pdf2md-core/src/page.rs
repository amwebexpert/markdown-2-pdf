//! Per-page extraction: page box, positioned glyphs, rulings, images, and URI
//! link targets.

use std::rc::Rc;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::interp::{Glyph, ImagePlacement, Interpreter, Ruling};

const US_LETTER: [f32; 4] = [0.0, 0.0, 612.0, 792.0];

#[derive(Debug, Clone)]
pub struct Page {
    /// 1-based page number.
    pub number: u32,
    /// Visible area `[x0, y0, x1, y1]` in default user space.
    pub bbox: [f32; 4],
    pub glyphs: Vec<Glyph>,
    /// Axis-aligned painted lines (table borders, rules).
    pub rulings: Vec<Ruling>,
    /// Raster images in drawing order.
    pub images: Vec<ImagePlacement>,
}

impl Page {
    pub fn height(&self) -> f32 {
        self.bbox[3] - self.bbox[1]
    }
}

pub fn extract(
    doc: &Document,
    interpreter: &mut Interpreter<'_>,
    number: u32,
    page_id: ObjectId,
) -> lopdf::Result<Page> {
    let content = interpreter.run_page(page_id)?;
    let mut glyphs = content.glyphs;
    let links = links(doc, page_id);
    if !links.is_empty() {
        for glyph in &mut glyphs {
            let cx = glyph.x + glyph.width / 2.0;
            let cy = glyph.y + glyph.size * 0.3;
            glyph.link = links
                .iter()
                .find(|(r, _)| cx >= r[0] && cx <= r[2] && cy >= r[1] && cy <= r[3])
                .map(|(_, uri)| Rc::clone(uri));
        }
    }
    Ok(Page {
        number,
        bbox: page_box(doc, page_id),
        glyphs,
        rulings: content.rulings,
        images: content.images,
    })
}

fn rect(doc: &Document, object: &Object) -> Option<[f32; 4]> {
    let array = doc.dereference(object).ok()?.1.as_array().ok()?;
    let values: Vec<f32> = array
        .iter()
        .filter_map(|o| doc.dereference(o).ok()?.1.as_float().ok())
        .collect();
    match *values.as_slice() {
        [x0, y0, x1, y1] => Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]),
        _ => None,
    }
}

/// `/CropBox`, else `/MediaBox`, both inheritable from ancestor page tree nodes.
fn page_box(doc: &Document, page_id: ObjectId) -> [f32; 4] {
    let mut node: Option<&Dictionary> = doc.get_dictionary(page_id).ok();
    let mut media = None;
    let mut crop = None;
    let mut hops = 0;
    while let Some(dict) = node {
        crop = crop.or_else(|| dict.get(b"CropBox").ok().and_then(|o| rect(doc, o)));
        media = media.or_else(|| dict.get(b"MediaBox").ok().and_then(|o| rect(doc, o)));
        hops += 1;
        node = (hops < 32)
            .then(|| {
                dict.get_deref(b"Parent", doc)
                    .and_then(Object::as_dict)
                    .ok()
            })
            .flatten();
    }
    crop.or(media).unwrap_or(US_LETTER)
}

/// URI link annotations. `/Annots` entries may be references or inline
/// dictionaries (lopdf's own helper only follows references).
fn links(doc: &Document, page_id: ObjectId) -> Vec<([f32; 4], Rc<str>)> {
    let Some(annots) = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get_deref(b"Annots", doc).ok())
        .and_then(|annots| annots.as_array().ok())
    else {
        return Vec::new();
    };
    annots
        .iter()
        .filter_map(|a| doc.dereference(a).ok()?.1.as_dict().ok())
        .filter(|a| a.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Link"))
        .filter_map(|a| {
            let area = rect(doc, a.get(b"Rect").ok()?)?;
            let action = a.get_deref(b"A", doc).ok()?.as_dict().ok()?;
            let uri = action.get_deref(b"URI", doc).ok()?.as_str().ok()?;
            let uri = String::from_utf8_lossy(uri).trim().to_string();
            (!uri.is_empty()).then(|| (area, Rc::from(uri)))
        })
        .collect()
}
