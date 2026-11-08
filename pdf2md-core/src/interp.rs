//! Minimal content-stream interpreter: tracks the graphics/text state needed
//! to place every shown glyph in default user space (PDF 32000-1 §9.4).

use std::collections::HashMap;
use std::rc::Rc;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::font::{Font, FontStyle};
use crate::geom::Matrix;

/// Upper bound for any single decompressed stream (decompression-bomb guard).
pub const MAX_DECOMPRESSED: usize = 64 * 1024 * 1024;
const MAX_FORM_DEPTH: usize = 8;
/// Painted shapes no larger than this (pt) and roughly square are read as
/// list bullets drawn with vector graphics (as browsers do).
const MAX_BULLET_SIZE: f32 = 8.0;
/// Bullet diameter as a fraction of the surrounding font size.
const BULLET_TO_FONT: f32 = 0.45;
/// Height of a bullet's centre above the baseline, in font sizes.
const BULLET_RAISE: f32 = 0.27;

/// One shown glyph, positioned on its baseline in default user space (y up).
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub size: f32,
    pub style: FontStyle,
    /// Target of the link annotation covering the glyph, if any.
    pub link: Option<Rc<str>>,
    /// A bullet inferred from a painted shape rather than shown text.
    pub synthetic: bool,
}

/// Resource dictionaries in lookup order (innermost first).
#[derive(Clone)]
struct Scope<'a> {
    dicts: Vec<&'a Dictionary>,
}

impl<'a> Scope<'a> {
    fn lookup(&self, doc: &'a Document, category: &[u8], key: &[u8]) -> Option<&'a Object> {
        self.dicts.iter().find_map(|resources| {
            let entries = resources.get_deref(category, doc).ok()?.as_dict().ok()?;
            entries.get(key).ok()
        })
    }
}

#[derive(Clone)]
struct TextParams<'a> {
    font: Option<Rc<Font<'a>>>,
    size: f32,
    char_spacing: f32,
    word_spacing: f32,
    horizontal_scale: f32,
    leading: f32,
    rise: f32,
}

/// Text matrix and text line matrix (`Tm` / `Tlm`), reset by `BT`.
#[derive(Default)]
struct TextCursor {
    matrix: Matrix,
    line: Matrix,
}

impl TextCursor {
    fn next_line(&mut self, tx: f32, ty: f32) {
        self.line = Matrix::translate(tx, ty).then(self.line);
        self.matrix = self.line;
    }
}

#[derive(Clone)]
struct GraphicsState<'a> {
    ctm: Matrix,
    text: TextParams<'a>,
}

fn extend(bbox: Option<[f32; 4]>, ctm: Matrix, x: f32, y: f32) -> [f32; 4] {
    let px = x * ctm.a + y * ctm.c + ctm.e;
    let py = x * ctm.b + y * ctm.d + ctm.f;
    match bbox {
        Some([x0, y0, x1, y1]) => [x0.min(px), y0.min(py), x1.max(px), y1.max(py)],
        None => [px, py, px, py],
    }
}

pub struct Interpreter<'a> {
    doc: &'a Document,
    fonts: HashMap<ObjectId, Rc<Font<'a>>>,
    glyphs: Vec<Glyph>,
}

impl<'a> Interpreter<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            fonts: HashMap::new(),
            glyphs: Vec::new(),
        }
    }

    /// Runs one page's content stream and returns its glyphs in stream order.
    pub fn run_page(&mut self, page_id: ObjectId) -> lopdf::Result<Vec<Glyph>> {
        let (direct, inherited) = self.doc.get_page_resources(page_id)?;
        let mut dicts: Vec<&Dictionary> = direct.into_iter().collect();
        dicts.extend(
            inherited
                .into_iter()
                .filter_map(|id| self.doc.get_dictionary(id).ok()),
        );
        let data = self
            .doc
            .get_page_content_with_limit(page_id, MAX_DECOMPRESSED)?;
        let content = Content::decode(&data)?;
        self.run(&content.operations, &Scope { dicts }, Matrix::IDENTITY, 0);
        Ok(std::mem::take(&mut self.glyphs))
    }

    fn run(&mut self, operations: &[Operation], scope: &Scope<'a>, ctm: Matrix, depth: usize) {
        let mut state = GraphicsState {
            ctm,
            text: TextParams {
                font: None,
                size: 0.0,
                char_spacing: 0.0,
                word_spacing: 0.0,
                horizontal_scale: 1.0,
                leading: 0.0,
                rise: 0.0,
            },
        };
        let mut stack: Vec<GraphicsState<'a>> = Vec::new();
        let mut cursor = TextCursor::default();
        // Bounding box `[x0, y0, x1, y1]` of the path under construction.
        let mut path: Option<[f32; 4]> = None;

        for op in operations {
            let nums: Vec<f32> = op
                .operands
                .iter()
                .filter_map(|o| o.as_float().ok())
                .collect();
            match op.operator.as_str() {
                "q" => stack.push(state.clone()),
                "Q" => {
                    if let Some(saved) = stack.pop() {
                        state = saved;
                    }
                }
                "cm" => {
                    if let Some(m) = Matrix::from_slice(&nums) {
                        state.ctm = m.then(state.ctm);
                    }
                }
                "Do" if depth < MAX_FORM_DEPTH => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                        self.run_form(scope, name, state.ctm, depth);
                    }
                }
                "m" | "l" | "c" | "v" | "y" | "re" | "n" | "f" | "F" | "f*" | "B" | "B*" | "b"
                | "b*" | "S" | "s" => {
                    self.path_op(op.operator.as_str(), &nums, state.ctm, &mut path);
                }
                _ => self.text_op(op, &nums, scope, &mut state, &mut cursor),
            }
        }
    }

    /// Path construction and painting; painted shapes may be list bullets.
    fn path_op(&mut self, operator: &str, nums: &[f32], ctm: Matrix, path: &mut Option<[f32; 4]>) {
        match operator {
            "re" => {
                if let [x, y, w, h] = *nums {
                    for (px, py) in [(x, y), (x + w, y + h), (x + w, y), (x, y + h)] {
                        *path = Some(extend(*path, ctm, px, py));
                    }
                }
            }
            "n" => *path = None,
            "m" | "l" | "c" | "v" | "y" => {
                for &[x, y] in nums.as_chunks::<2>().0 {
                    *path = Some(extend(*path, ctm, x, y));
                }
            }
            _ => {
                if let Some(bbox) = path.take() {
                    self.bullet(bbox, !matches!(operator, "S" | "s"));
                }
            }
        }
    }

    /// Text state, positioning and showing operators (PDF 32000-1 §9.3-9.4).
    fn text_op(
        &mut self,
        op: &Operation,
        nums: &[f32],
        scope: &Scope<'a>,
        state: &mut GraphicsState<'a>,
        cursor: &mut TextCursor,
    ) {
        let num = |i: usize| nums.get(i).copied().unwrap_or(0.0);
        let text = &mut state.text;
        match op.operator.as_str() {
            "BT" => *cursor = TextCursor::default(),
            "Tf" => {
                text.font = op
                    .operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .and_then(|name| self.font(scope, name));
                text.size = op
                    .operands
                    .get(1)
                    .and_then(|o| o.as_float().ok())
                    .unwrap_or(0.0);
            }
            "Tc" => text.char_spacing = num(0),
            "Tw" => text.word_spacing = num(0),
            "Tz" => text.horizontal_scale = num(0) / 100.0,
            "TL" => text.leading = num(0),
            "Ts" => text.rise = num(0),
            "Td" => cursor.next_line(num(0), num(1)),
            "TD" => {
                text.leading = -num(1);
                cursor.next_line(num(0), num(1));
            }
            "Tm" => {
                if let Some(m) = Matrix::from_slice(nums) {
                    cursor.line = m;
                    cursor.matrix = m;
                }
            }
            "T*" => cursor.next_line(0.0, -text.leading),
            "Tj" | "'" | "\"" => {
                if op.operator == "\"" {
                    text.word_spacing = num(0);
                    text.char_spacing = num(1);
                }
                if op.operator != "Tj" {
                    cursor.next_line(0.0, -text.leading);
                }
                if let Some(Object::String(bytes, _)) = op.operands.last() {
                    self.show(bytes, state, &mut cursor.matrix);
                }
            }
            "TJ" => {
                let items = op.operands.first().and_then(|o| o.as_array().ok());
                for item in items.into_iter().flatten() {
                    if let Object::String(bytes, _) = item {
                        self.show(bytes, state, &mut cursor.matrix);
                    } else {
                        let adjust = item.as_float().unwrap_or(0.0);
                        let tx = -adjust / 1000.0 * state.text.size * state.text.horizontal_scale;
                        cursor.matrix = Matrix::translate(tx, 0.0).then(cursor.matrix);
                    }
                }
            }
            _ => {}
        }
    }

    /// Emits a synthetic bullet glyph for a small, roughly square painted shape.
    fn bullet(&mut self, [x0, y0, x1, y1]: [f32; 4], filled: bool) {
        let (w, h) = (x1 - x0, y1 - y0);
        let square = w > 0.5 && h > 0.5 && w <= MAX_BULLET_SIZE && h <= MAX_BULLET_SIZE;
        if !square || w > 2.0 * h || h > 2.0 * w {
            return;
        }
        let size = w.max(h) / BULLET_TO_FONT;
        self.glyphs.push(Glyph {
            text: if filled { "•" } else { "◦" }.to_string(),
            x: x0,
            y: f32::midpoint(y0, y1) - BULLET_RAISE * size,
            width: w,
            size,
            style: FontStyle::default(),
            link: None,
            synthetic: true,
        });
    }

    fn run_form(&mut self, scope: &Scope<'a>, name: &[u8], ctm: Matrix, depth: usize) {
        let doc = self.doc;
        let Some(object) = scope.lookup(doc, b"XObject", name) else {
            return;
        };
        let Ok(stream) = doc.dereference(object).and_then(|(_, o)| o.as_stream()) else {
            return;
        };
        if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form") {
            return;
        }
        let matrix = stream
            .dict
            .get(b"Matrix")
            .and_then(Object::as_array)
            .ok()
            .and_then(|m| {
                let values: Vec<f32> = m.iter().filter_map(|o| o.as_float().ok()).collect();
                Matrix::from_slice(&values)
            })
            .unwrap_or(Matrix::IDENTITY);
        let Ok(data) = stream.decompressed_content_with_limit(MAX_DECOMPRESSED) else {
            return;
        };
        let Ok(content) = Content::decode(&data) else {
            return;
        };
        let mut inner = scope.clone();
        if let Ok(resources) = stream
            .dict
            .get_deref(b"Resources", doc)
            .and_then(Object::as_dict)
        {
            inner.dicts.insert(0, resources);
        }
        self.run(&content.operations, &inner, matrix.then(ctm), depth + 1);
    }

    fn font(&mut self, scope: &Scope<'a>, name: &[u8]) -> Option<Rc<Font<'a>>> {
        let doc = self.doc;
        let object = scope.lookup(doc, b"Font", name)?;
        match object {
            Object::Reference(id) => {
                if let Some(font) = self.fonts.get(id) {
                    return Some(Rc::clone(font));
                }
                let dict = doc.get_dictionary(*id).ok()?;
                let font = Rc::new(Font::load(doc, dict));
                self.fonts.insert(*id, Rc::clone(&font));
                Some(font)
            }
            Object::Dictionary(dict) => Some(Rc::new(Font::load(doc, dict))),
            _ => None,
        }
    }

    fn show(&mut self, bytes: &[u8], state: &GraphicsState<'a>, tm: &mut Matrix) {
        let params = &state.text;
        let Some(font) = params.font.as_ref() else {
            return;
        };
        let size = params.size;
        let scale = params.horizontal_scale;
        for (code, len) in font.codes(bytes) {
            let advance = font.width(code);
            let rendering = Matrix {
                a: size * scale,
                d: size,
                f: params.rise,
                ..Matrix::IDENTITY
            }
            .then(*tm)
            .then(state.ctm);
            // Only horizontal text is reconstructed; rotated labels are skipped.
            let horizontal = rendering.b.abs() <= rendering.a.abs() * 0.05 && rendering.a > 0.0;
            let text = font.decode(code, len);
            if horizontal && !text.is_empty() {
                self.glyphs.push(Glyph {
                    text,
                    x: rendering.e,
                    y: rendering.f,
                    width: advance * rendering.x_scale(),
                    size: rendering.y_scale(),
                    style: font.style,
                    link: None,
                    synthetic: false,
                });
            }
            let is_space = len == 1 && code == 32;
            let tx = (advance * size
                + params.char_spacing
                + if is_space { params.word_spacing } else { 0.0 })
                * scale;
            *tm = Matrix::translate(tx, 0.0).then(*tm);
        }
    }
}
