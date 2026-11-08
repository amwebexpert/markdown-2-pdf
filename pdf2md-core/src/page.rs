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

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "test boxes hold exact integer coordinates"
)]
mod tests {
    use super::*;
    use lopdf::{Stream, dictionary};

    /// One-page document; `page` and `tree` entries are merged into the page
    /// and page-tree dictionaries.
    fn doc_with(page: &Dictionary, tree: &Dictionary, content: &str) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.7");
        let contents = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let tree_id = doc.new_object_id();
        let mut leaf = dictionary! {
            "Type" => "Page",
            "Parent" => tree_id,
            "Contents" => contents,
        };
        leaf.extend(page);
        let leaf_id = doc.add_object(leaf);
        let mut root =
            dictionary! { "Type" => "Pages", "Kids" => vec![leaf_id.into()], "Count" => 1 };
        root.extend(tree);
        doc.objects.insert(tree_id, Object::Dictionary(root));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree_id });
        doc.trailer.set("Root", catalog);
        (doc, leaf_id)
    }

    fn rect_obj(r: &[i64]) -> Object {
        Object::Array(r.iter().copied().map(Object::Integer).collect())
    }

    fn link_annot(r: [i64; 4], uri: &str) -> Dictionary {
        dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => rect_obj(&r),
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal(uri) },
        }
    }

    #[test]
    fn page_box_defaults_to_us_letter() {
        let (doc, page) = doc_with(&dictionary! {}, &dictionary! {}, "");
        assert_eq!(page_box(&doc, page), US_LETTER);
    }

    #[test]
    fn page_box_prefers_crop_box_over_media_box() {
        let (doc, page) = doc_with(
            &dictionary! {
                "MediaBox" => rect_obj(&[0, 0, 600, 800]),
                "CropBox" => rect_obj(&[10, 20, 500, 700]),
            },
            &dictionary! {},
            "",
        );
        assert_eq!(page_box(&doc, page), [10.0, 20.0, 500.0, 700.0]);
    }

    #[test]
    fn page_box_inherits_media_box_from_the_page_tree() {
        let (doc, page) = doc_with(
            &dictionary! {},
            &dictionary! { "MediaBox" => rect_obj(&[0, 0, 595, 842]) },
            "",
        );
        assert_eq!(page_box(&doc, page), [0.0, 0.0, 595.0, 842.0]);
    }

    #[test]
    fn page_box_normalizes_inverted_corners() {
        let (doc, page) = doc_with(
            &dictionary! { "MediaBox" => rect_obj(&[612, 792, 0, 0]) },
            &dictionary! {},
            "",
        );
        assert_eq!(page_box(&doc, page), US_LETTER);
    }

    #[test]
    fn page_box_ignores_malformed_rectangles() {
        let (doc, page) = doc_with(
            &dictionary! { "MediaBox" => rect_obj(&[0, 0, 100]) },
            &dictionary! {},
            "",
        );
        assert_eq!(page_box(&doc, page), US_LETTER);
    }

    #[test]
    fn links_reads_inline_and_referenced_uri_annotations() {
        let (mut doc, page) = doc_with(&dictionary! {}, &dictionary! {}, "");
        let referenced = doc.add_object(link_annot([0, 0, 10, 10], "https://a.test"));
        let annots = vec![
            referenced.into(),
            Object::Dictionary(link_annot([20, 20, 30, 30], " https://b.test ")),
        ];
        doc.get_dictionary_mut(page).unwrap().set("Annots", annots);

        let found: Vec<([f32; 4], String)> = links(&doc, page)
            .into_iter()
            .map(|(r, uri)| (r, uri.to_string()))
            .collect();
        assert_eq!(
            found,
            [
                ([0.0, 0.0, 10.0, 10.0], "https://a.test".to_string()),
                ([20.0, 20.0, 30.0, 30.0], "https://b.test".to_string()),
            ]
        );
    }

    #[test]
    fn links_skips_non_link_annotations_and_blank_uris() {
        let mut text_annot = link_annot([0, 0, 10, 10], "https://a.test");
        text_annot.set("Subtype", "Text");
        let annots = vec![
            Object::Dictionary(text_annot),
            Object::Dictionary(link_annot([0, 0, 10, 10], "   ")),
        ];
        let (doc, page) = doc_with(&dictionary! { "Annots" => annots }, &dictionary! {}, "");
        assert!(links(&doc, page).is_empty());
    }

    #[test]
    fn extract_attaches_link_targets_to_covered_glyphs() {
        let font =
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" };
        let (doc, page) = doc_with(
            &dictionary! {
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                "Annots" => vec![Object::Dictionary(link_annot([90, 690, 300, 720], "https://x.test"))],
            },
            &dictionary! {},
            "BT /F1 12 Tf 100 700 Td (in) Tj 300 0 Td (out) Tj ET",
        );
        let mut interpreter = Interpreter::new(&doc);
        let extracted = extract(&doc, &mut interpreter, 1, page).unwrap();

        assert_eq!(extracted.number, 1);
        assert!(extracted.glyphs.iter().any(|g| g.link.is_some()));
        assert!(extracted.glyphs.iter().any(|g| g.link.is_none()));
        for glyph in &extracted.glyphs {
            let expected = (glyph.x < 300.0).then_some("https://x.test");
            assert_eq!(glyph.link.as_deref(), expected, "glyph {:?}", glyph.text);
        }
    }

    #[test]
    fn height_is_the_bbox_extent() {
        let page = Page {
            number: 1,
            bbox: [0.0, 10.0, 612.0, 802.0],
            glyphs: Vec::new(),
            rulings: Vec::new(),
            images: Vec::new(),
        };
        assert!((page.height() - 792.0).abs() < f32::EPSILON);
    }
}
