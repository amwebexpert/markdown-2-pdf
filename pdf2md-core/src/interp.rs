//! Minimal content-stream interpreter: tracks the graphics/text state needed
//! to place every shown glyph in default user space (PDF 32000-1 §9.4).

use std::collections::HashMap;
use std::rc::Rc;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

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
/// Shortest painted line (pt) kept as a ruling.
const MIN_RULING_LENGTH: f32 = 3.0;
/// Largest deviation from the axis (pt) for a line to count as straight.
const AXIS_TOLERANCE: f32 = 0.5;
/// Filled rectangles thinner than this (pt) are rulings along their long axis.
const MAX_RULING_THICKNESS: f32 = 2.0;
/// Strokes wider than this (pt) are bars or decorations, not rulings.
const MAX_STROKE_WIDTH: f32 = 4.0;
/// Gray level above which paint is treated as invisible (white on white).
const NEAR_WHITE: f32 = 0.95;
/// Placed images smaller than this (pt) in either dimension are spacers.
const MIN_IMAGE_SIZE: f32 = 8.0;

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

/// An axis-aligned painted line in default user space (table border, rule).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ruling {
    pub horizontal: bool,
    /// y of a horizontal ruling, x of a vertical one.
    pub pos: f32,
    /// Extent along the ruling's own axis (`from <= to`).
    pub from: f32,
    pub to: f32,
}

/// Where an image's data lives.
#[derive(Debug, Clone)]
pub enum ImageSource {
    /// An image `XObject`, shared by every place that draws it.
    Object(ObjectId),
    /// An inline image (`BI … ID … EI`) or a direct stream object.
    Inline(Rc<Stream>),
}

/// One drawn raster image.
#[derive(Debug, Clone)]
pub struct ImagePlacement {
    pub source: ImageSource,
    /// The unit square mapped through the CTM, `[x0, y0, x1, y1]`.
    pub bbox: [f32; 4],
}

/// Everything a page's content stream paints that the converter uses.
#[derive(Debug, Default)]
pub struct PageContent {
    pub glyphs: Vec<Glyph>,
    pub rulings: Vec<Ruling>,
    pub images: Vec<ImagePlacement>,
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
    /// Line width (`w`) in user space units before the CTM.
    line_width: f32,
    /// Approximate gray level (0 black, 1 white) of stroke and fill colours.
    stroke_gray: f32,
    fill_gray: f32,
}

fn transform(ctm: Matrix, x: f32, y: f32) -> [f32; 2] {
    [x * ctm.a + y * ctm.c + ctm.e, x * ctm.b + y * ctm.d + ctm.f]
}

fn extend(bbox: Option<[f32; 4]>, [px, py]: [f32; 2]) -> [f32; 4] {
    match bbox {
        Some([x0, y0, x1, y1]) => [x0.min(px), y0.min(py), x1.max(px), y1.max(py)],
        None => [px, py, px, py],
    }
}

fn bounds(points: &[[f32; 2]]) -> Option<[f32; 4]> {
    points.iter().fold(None, |bbox, p| Some(extend(bbox, *p)))
}

/// Gray level of a colour given by 1 (gray), 3 (RGB) or 4 (CMYK) components.
fn gray_level(components: &[f32]) -> Option<f32> {
    match *components {
        [g] => Some(g),
        [r, g, b] => Some(0.299 * r + 0.587 * g + 0.114 * b),
        [c, m, y, k] => {
            let ink = |v: f32| (1.0 - v) * (1.0 - k);
            Some(0.299 * ink(c) + 0.587 * ink(m) + 0.114 * ink(y))
        }
        _ => None,
    }
}

/// One subpath in user space; curves are kept only for the bounding box.
#[derive(Default)]
struct Subpath {
    points: Vec<[f32; 2]>,
    closed: bool,
    curved: bool,
}

/// Axis-aligned ruling between two points, if the segment is one.
fn ruling([ax, ay]: [f32; 2], [bx, by]: [f32; 2]) -> Option<Ruling> {
    let (dx, dy) = ((bx - ax).abs(), (by - ay).abs());
    if dy < AXIS_TOLERANCE && dx > MIN_RULING_LENGTH {
        Some(Ruling {
            horizontal: true,
            pos: f32::midpoint(ay, by),
            from: ax.min(bx),
            to: ax.max(bx),
        })
    } else if dx < AXIS_TOLERANCE && dy > MIN_RULING_LENGTH {
        Some(Ruling {
            horizontal: false,
            pos: f32::midpoint(ax, bx),
            from: ay.min(by),
            to: ay.max(by),
        })
    } else {
        None
    }
}

/// Ruling along the long axis of a thin filled rectangle.
fn thin_rect_ruling([x0, y0, x1, y1]: [f32; 4]) -> Option<Ruling> {
    let (w, h) = (x1 - x0, y1 - y0);
    if h < MAX_RULING_THICKNESS && w > MIN_RULING_LENGTH && w > h {
        Some(Ruling {
            horizontal: true,
            pos: f32::midpoint(y0, y1),
            from: x0,
            to: x1,
        })
    } else if w < MAX_RULING_THICKNESS && h > MIN_RULING_LENGTH {
        Some(Ruling {
            horizontal: false,
            pos: f32::midpoint(x0, x1),
            from: y0,
            to: y1,
        })
    } else {
        None
    }
}

fn is_image_mask(dict: &Dictionary) -> bool {
    [b"ImageMask".as_slice(), b"IM"]
        .iter()
        .any(|key| dict.get(key).and_then(Object::as_bool).ok() == Some(true))
}

pub struct Interpreter<'a> {
    doc: &'a Document,
    fonts: HashMap<ObjectId, Rc<Font<'a>>>,
    content: PageContent,
}

impl<'a> Interpreter<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            fonts: HashMap::new(),
            content: PageContent::default(),
        }
    }

    /// Runs one page's content stream and returns what it paints, glyphs in
    /// stream order.
    pub fn run_page(&mut self, page_id: ObjectId) -> lopdf::Result<PageContent> {
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
        Ok(std::mem::take(&mut self.content))
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
            line_width: 1.0,
            stroke_gray: 0.0,
            fill_gray: 0.0,
        };
        let mut stack: Vec<GraphicsState<'a>> = Vec::new();
        let mut cursor = TextCursor::default();
        // Subpaths of the path under construction.
        let mut path: Vec<Subpath> = Vec::new();

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
                "Do" => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                        self.x_object(scope, name, state.ctm, depth);
                    }
                }
                "BI" => {
                    if let Some(Object::Stream(stream)) = op.operands.first() {
                        self.image(
                            ImageSource::Inline(Rc::new(stream.clone())),
                            stream,
                            state.ctm,
                        );
                    }
                }
                "w" => state.line_width = nums.first().copied().unwrap_or(1.0),
                "G" | "RG" | "K" | "SC" | "SCN" => {
                    state.stroke_gray = gray_level(&nums).unwrap_or(state.stroke_gray);
                }
                "g" | "rg" | "k" | "sc" | "scn" => {
                    state.fill_gray = gray_level(&nums).unwrap_or(state.fill_gray);
                }
                "CS" => state.stroke_gray = 0.0,
                "cs" => state.fill_gray = 0.0,
                "m" | "l" | "c" | "v" | "y" | "re" | "h" | "n" | "f" | "F" | "f*" | "B" | "B*"
                | "b" | "b*" | "S" | "s" => {
                    self.path_op(op.operator.as_str(), &nums, &state, &mut path);
                }
                _ => self.text_op(op, &nums, scope, &mut state, &mut cursor),
            }
        }
    }

    /// Path construction and painting; painted shapes may be list bullets or
    /// rulings.
    fn path_op(
        &mut self,
        operator: &str,
        nums: &[f32],
        state: &GraphicsState<'a>,
        path: &mut Vec<Subpath>,
    ) {
        let ctm = state.ctm;
        let points = nums
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[x, y]| transform(ctm, x, y));
        match operator {
            "re" => {
                if let [x, y, w, h] = *nums {
                    path.push(Subpath {
                        points: [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
                            .into_iter()
                            .map(|(px, py)| transform(ctm, px, py))
                            .collect(),
                        closed: true,
                        curved: false,
                    });
                }
            }
            "m" => path.push(Subpath {
                points: points.collect(),
                ..Subpath::default()
            }),
            "l" | "c" | "v" | "y" => {
                if path.is_empty() {
                    path.push(Subpath::default());
                }
                if let Some(current) = path.last_mut() {
                    current.points.extend(points);
                    current.curved |= operator != "l";
                }
            }
            "h" => {
                if let Some(current) = path.last_mut() {
                    current.closed = true;
                }
            }
            "n" => path.clear(),
            _ => {
                let subpaths = std::mem::take(path);
                let all: Vec<[f32; 2]> = subpaths
                    .iter()
                    .flat_map(|s| s.points.iter().copied())
                    .collect();
                let stroked = matches!(operator, "S" | "s" | "B" | "B*" | "b" | "b*");
                let filled = !matches!(operator, "S" | "s");
                if let Some(bbox) = bounds(&all) {
                    self.bullet(bbox, filled);
                }
                if stroked && state.stroke_gray < NEAR_WHITE {
                    let width = state.line_width * f32::midpoint(ctm.x_scale(), ctm.y_scale());
                    let close = matches!(operator, "s" | "b" | "b*");
                    if width <= MAX_STROKE_WIDTH {
                        for subpath in subpaths.iter().filter(|s| !s.curved) {
                            self.stroke_rulings(subpath, close);
                        }
                    }
                }
                if filled && state.fill_gray < NEAR_WHITE {
                    let rulings = subpaths
                        .iter()
                        .filter(|s| !s.curved)
                        .filter_map(|s| bounds(&s.points))
                        .filter_map(thin_rect_ruling);
                    self.content.rulings.extend(rulings);
                }
            }
        }
    }

    fn stroke_rulings(&mut self, subpath: &Subpath, close: bool) {
        let points = &subpath.points;
        let closing = (subpath.closed || close)
            .then(|| Some((*points.last()?, *points.first()?)))
            .flatten();
        let segments = points
            .windows(2)
            .filter_map(|pair| match *pair {
                [a, b] => Some((a, b)),
                _ => None,
            })
            .chain(closing);
        let rulings = segments.filter_map(|(a, b)| ruling(a, b));
        self.content.rulings.extend(rulings);
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
        self.content.glyphs.push(Glyph {
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

    /// `Do`: runs a form `XObject` or records an image placement.
    fn x_object(&mut self, scope: &Scope<'a>, name: &[u8], ctm: Matrix, depth: usize) {
        let doc = self.doc;
        let Some(object) = scope.lookup(doc, b"XObject", name) else {
            return;
        };
        let Ok((id, Ok(stream))) = doc.dereference(object).map(|(id, o)| (id, o.as_stream()))
        else {
            return;
        };
        match stream.dict.get(b"Subtype").and_then(Object::as_name).ok() {
            Some(b"Form") if depth < MAX_FORM_DEPTH => self.run_form(scope, stream, ctm, depth),
            Some(b"Image") => {
                let source = id.map_or_else(
                    || ImageSource::Inline(Rc::new(stream.clone())),
                    ImageSource::Object,
                );
                self.image(source, stream, ctm);
            }
            _ => {}
        }
    }

    /// Records a placed image unless it is a stencil mask or a tiny spacer.
    fn image(&mut self, source: ImageSource, stream: &Stream, ctm: Matrix) {
        if is_image_mask(&stream.dict) {
            return;
        }
        let corners = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
        let points: Vec<[f32; 2]> = corners
            .into_iter()
            .map(|(x, y)| transform(ctm, x, y))
            .collect();
        let Some(bbox @ [x0, y0, x1, y1]) = bounds(&points) else {
            return;
        };
        if x1 - x0 >= MIN_IMAGE_SIZE && y1 - y0 >= MIN_IMAGE_SIZE {
            self.content.images.push(ImagePlacement { source, bbox });
        }
    }

    fn run_form(&mut self, scope: &Scope<'a>, stream: &'a Stream, ctm: Matrix, depth: usize) {
        let doc = self.doc;
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
                self.content.glyphs.push(Glyph {
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

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn run(content: &str, resources: Dictionary) -> PageContent {
        let mut doc = Document::with_version("1.7");
        let contents = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "Contents" => contents,
            "Resources" => resources,
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        Interpreter::new(&doc).run_page(page).unwrap()
    }

    #[test]
    fn stroked_rectangle_yields_four_rulings() {
        let rulings = run("0.6 G 100 100 200 50 re S", dictionary! {}).rulings;
        assert_eq!(rulings.len(), 4);
        assert_eq!(rulings.iter().filter(|r| r.horizontal).count(), 2);
    }

    #[test]
    fn thin_filled_rectangle_yields_one_ruling() {
        let rulings = run("0.5 g 100 100 200 1 re f", dictionary! {}).rulings;
        assert_eq!(
            rulings,
            [Ruling {
                horizontal: true,
                pos: 100.5,
                from: 100.0,
                to: 300.0
            }]
        );
    }

    #[test]
    fn curves_and_white_strokes_are_not_rulings() {
        let curve = run("100 100 m 150 200 200 200 250 100 c S", dictionary! {});
        assert!(curve.rulings.is_empty());
        assert!(
            run("1 G 100 100 m 300 100 l S", dictionary! {})
                .rulings
                .is_empty()
        );
        assert_eq!(
            run("0 G 100 100 m 300 100 l S", dictionary! {})
                .rulings
                .len(),
            1
        );
    }

    #[test]
    fn image_placement_maps_the_unit_square_through_the_ctm() {
        let mut doc = Document::with_version("1.7");
        let image = doc.add_object(Stream::new(
            dictionary! { "Subtype" => "Image", "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
            vec![0],
        ));
        let resources = dictionary! { "XObject" => dictionary! { "Im1" => image } };
        let contents = doc.add_object(Stream::new(
            dictionary! {},
            b"q 50 0 0 40 100 200 cm /Im1 Do Q q 2 0 0 2 0 0 cm /Im1 Do Q".to_vec(),
        ));
        let page = doc.add_object(
            dictionary! { "Type" => "Page", "Contents" => contents, "Resources" => resources },
        );
        let images = Interpreter::new(&doc).run_page(page).unwrap().images;
        // The 2×2 pt spacer is skipped.
        assert_eq!(images.len(), 1);
        let expected = [100.0, 200.0, 150.0, 240.0];
        assert!(
            images[0]
                .bbox
                .iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 0.01)
        );
        assert!(matches!(images[0].source, ImageSource::Object(id) if id == image));
    }
}
