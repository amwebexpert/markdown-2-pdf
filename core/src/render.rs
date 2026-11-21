//! Hand-rolled layout engine on top of printpdf's low-level drawing ops.
//!
//! printpdf 0.12 (built with `default-features = false`) has no built-in text
//! wrapping, pagination, lists or tables — this module supplies all of that:
//! word-wrap by measured glyph advances, a top-down page cursor, and simple
//! block renderers for headings/paragraphs/lists/code/tables/rules.

use printpdf::{
    Actions, BorderArray, Color, FontId, HighlightingMode, Line, LinePoint, LinkAnnotation, Mm, Op,
    PaintMode, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt, Rect, Rgb, TextItem,
    TextMatrix, XObjectId, XObjectTransform,
};

use std::collections::BTreeMap;

use crate::error::Md2PdfError;
use crate::fonts::{FontSet, text_width_pt};
use crate::image::{self, Image};
use crate::ir::{Align, Block, ImageRef, ListItem, Span};

const PAGE_WIDTH_PT: f32 = 612.0; // US Letter, 8.5in
const PAGE_HEIGHT_PT: f32 = 792.0; // 11in
const MARGIN_PT: f32 = 72.0; // 1in

const BODY_SIZE: f32 = 11.0;
const BODY_LEADING: f32 = 14.0;
const CODE_SIZE: f32 = 10.0;
const CODE_LEADING: f32 = 13.0;
const HEADING_SIZES: [f32; 6] = [24.0, 20.0, 17.0, 14.5, 12.5, 11.0];
const HEADING_LEADING_FACTOR: f32 = 1.25;
const PARAGRAPH_SPACING_AFTER: f32 = 8.0;
const HEADING_SPACING_BEFORE: f32 = 12.0;
const HEADING_SPACING_AFTER: f32 = 6.0;
const LIST_INDENT: f32 = 18.0;
const BLOCKQUOTE_INDENT: f32 = 18.0;
const TABLE_FONT_SIZE: f32 = 10.0;
const TABLE_ROW_LEADING: f32 = 13.0;
const TABLE_CELL_PADDING: f32 = 4.0;
const TABLE_BORDER_THICKNESS: f32 = 0.6;
/// Baseline distance below the top of a line box, as a fraction of leading —
/// an approximation (fonts don't share one ascent ratio) good enough since no
/// pixel-perfect typography was requested.
const BASELINE_RATIO: f32 = 0.82;
/// Strike line height above the baseline and thickness, as fractions of the
/// font size (roughly the x-height middle of Liberation Sans).
const STRIKE_RISE: f32 = 0.3;
const STRIKE_THICKNESS: f32 = 0.06;
/// Task checkbox side, as a fraction of the body font size.
const CHECKBOX_SIZE: f32 = 0.7;
/// Superscript (footnote marks): font size scale and baseline rise, as
/// fractions of the surrounding text size.
const SUPERSCRIPT_SCALE: f32 = 0.7;
const SUPERSCRIPT_RISE: f32 = 0.35;
/// Images are laid out at 96 dpi (1 px = 0.75 pt), the CSS reference pixel.
const PT_PER_PX: f32 = 0.75;
const CAPTION_SIZE: f32 = 9.0;
const CAPTION_LEADING: f32 = 12.0;
const CAPTION_GAP: f32 = 2.0;
const LINK_COLOR: (f32, f32, f32) = (0.06, 0.32, 0.78);
const BLACK: (f32, f32, f32) = (0.0, 0.0, 0.0);
const RULE_GREY: (f32, f32, f32) = (0.7, 0.7, 0.7);
const BORDER_GREY: (f32, f32, f32) = (0.6, 0.6, 0.6);

fn horizontal_line(x_from: f32, x_to: f32, y: f32) -> Line {
    let point = |x: f32| LinePoint {
        p: Point { x: Pt(x), y: Pt(y) },
        bezier: false,
    };
    Line {
        points: vec![point(x_from), point(x_to)],
        is_closed: false,
    }
}

fn rgb((r, g, b): (f32, f32, f32)) -> Color {
    Color::Rgb(Rgb {
        r,
        g,
        b,
        icc_profile: None,
    })
}

/// What sits in a list item's marker slot.
enum Marker {
    Text(String),
    Task(bool),
}

/// Font selection, size and fill color for a single run of text.
#[derive(Clone, Copy)]
struct TextStyle {
    bold: bool,
    italic: bool,
    code: bool,
    size: f32,
    color: (f32, f32, f32),
}

impl TextStyle {
    const fn regular(size: f32) -> Self {
        Self {
            bold: false,
            italic: false,
            code: false,
            size,
            color: BLACK,
        }
    }
}

/// A run of one style inside a word.
#[derive(Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent inline styles that combine freely, as in Markdown"
)]
struct Fragment {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    superscript: bool,
    link: Option<String>,
    width: f32,
}

impl Fragment {
    fn new(text: String, span: &Span, bold: bool, fonts: &FontSet, size: f32) -> Self {
        let size = if span.superscript {
            size * SUPERSCRIPT_SCALE
        } else {
            size
        };
        let width = text_width_pt(fonts.resolve(bold, span.italic, span.code), &text, size);
        Fragment {
            text,
            bold,
            italic: span.italic,
            code: span.code,
            strike: span.strike,
            superscript: span.superscript,
            link: span.link.clone(),
            width,
        }
    }

    /// Font size and baseline rise of this fragment on a line of `size`.
    fn metrics(&self, size: f32) -> (f32, f32) {
        if self.superscript {
            (size * SUPERSCRIPT_SCALE, size * SUPERSCRIPT_RISE)
        } else {
            (size, 0.0)
        }
    }
}

/// Text between two whitespace boundaries. Style changes without whitespace
/// (`foo**bar**`) stay in one word as several fragments.
#[derive(Clone, Default)]
struct Word {
    fragments: Vec<Fragment>,
    width: f32,
}

impl Word {
    fn push(&mut self, fragment: Fragment) {
        self.width += fragment.width;
        self.fragments.push(fragment);
    }

    fn starts_struck(&self) -> bool {
        self.fragments.first().is_some_and(|f| f.strike)
    }

    fn ends_struck(&self) -> bool {
        self.fragments.last().is_some_and(|f| f.strike)
    }
}

enum Token {
    Word(Word),
    Break,
}

fn finish_word(tokens: &mut Vec<Token>, word: &mut Word) {
    if !word.fragments.is_empty() {
        tokens.push(Token::Word(std::mem::take(word)));
    }
}

fn tokenize(spans: &[Span], fonts: &FontSet, size: f32, force_bold: bool) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = Word::default();
    for span in spans {
        let bold = force_bold || span.bold;
        for (i, segment) in span.text.split('\n').enumerate() {
            if i > 0 {
                finish_word(&mut tokens, &mut word);
                tokens.push(Token::Break);
            }
            for (j, piece) in segment.split(char::is_whitespace).enumerate() {
                if j > 0 {
                    finish_word(&mut tokens, &mut word);
                }
                if !piece.is_empty() {
                    word.push(Fragment::new(piece.to_string(), span, bold, fonts, size));
                }
            }
        }
    }
    finish_word(&mut tokens, &mut word);
    tokens
}

fn wrap_lines(tokens: Vec<Token>, max_width: f32, space_width: f32) -> Vec<Vec<Word>> {
    let mut lines = Vec::new();
    let mut current: Vec<Word> = Vec::new();
    let mut current_width = 0.0f32;
    for token in tokens {
        match token {
            Token::Break => {
                lines.push(std::mem::take(&mut current));
                current_width = 0.0;
            }
            Token::Word(w) => {
                let needed = if current.is_empty() {
                    w.width
                } else {
                    current_width + space_width + w.width
                };
                if needed > max_width && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    current_width = w.width;
                    current.push(w);
                } else {
                    current_width = needed;
                    current.push(w);
                }
            }
        }
    }
    lines.push(current);
    lines
}

fn line_width(line: &[Word], space_width: f32) -> f32 {
    if line.is_empty() {
        return 0.0;
    }
    let words: f32 = line.iter().map(|w| w.width).sum();
    words + space_width * (line.len() as f32 - 1.0)
}

/// Hard character-wrap (no word boundaries), so a long run of text overflows
/// onto extra lines instead of running off the page edge.
fn hard_wrap_chars(
    font: &printpdf::ParsedFont,
    text: &str,
    size: f32,
    max_width: f32,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0.0f32;
    for ch in text.chars() {
        let w = text_width_pt(font, &ch.to_string(), size);
        if current_width + w > max_width && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_width = 0.0;
        }
        current.push(ch);
        current_width += w;
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

/// Hard character-wrap for monospace code lines. See `hard_wrap_chars`.
fn wrap_monospace(fonts: &FontSet, line: &str, size: f32, max_width: f32) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let font = fonts.resolve(false, false, true);
    hard_wrap_chars(font, line, size, max_width)
}

/// Hard character-wraps one word, keeping each character's fragment style.
fn hard_wrap_word(word: Word, fonts: &FontSet, size: f32, max_width: f32) -> Vec<Word> {
    let mut out = Vec::new();
    let mut current = Word::default();
    for fragment in word.fragments {
        let font = fonts.resolve(fragment.bold, fragment.italic, fragment.code);
        let mut piece = Fragment {
            text: String::new(),
            width: 0.0,
            ..fragment.clone()
        };
        let (char_size, _) = fragment.metrics(size);
        for ch in fragment.text.chars() {
            let w = text_width_pt(font, ch.encode_utf8(&mut [0; 4]), char_size);
            if current.width + piece.width + w > max_width
                && (!current.fragments.is_empty() || !piece.text.is_empty())
            {
                if !piece.text.is_empty() {
                    current.push(std::mem::replace(
                        &mut piece,
                        Fragment {
                            text: String::new(),
                            width: 0.0,
                            ..fragment.clone()
                        },
                    ));
                }
                out.push(std::mem::take(&mut current));
            }
            piece.text.push(ch);
            piece.width += w;
        }
        if !piece.text.is_empty() {
            current.push(piece);
        }
    }
    if !current.fragments.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

/// Replaces any `Token::Word` wider than `max_width` on its own (a single
/// unbreakable token, e.g. a path or URL with no whitespace) with several
/// narrower `Token::Word`s hard-split at the character level, so `wrap_lines`
/// naturally spreads it across lines instead of letting it overflow the page.
fn split_oversized_words(
    tokens: Vec<Token>,
    fonts: &FontSet,
    size: f32,
    max_width: f32,
) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len());
    for token in tokens {
        match token {
            Token::Word(w) if w.width > max_width => {
                out.extend(
                    hard_wrap_word(w, fonts, size, max_width)
                        .into_iter()
                        .map(Token::Word),
                );
            }
            other => out.push(other),
        }
    }
    out
}

fn measure_row_natural(fonts: &FontSet, cells: &[Vec<Span>], widths: &mut [f32]) {
    let num_cols = widths.len();
    for (i, spans) in cells.iter().enumerate().take(num_cols) {
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        let bold = spans.iter().any(|s| s.bold);
        let italic = spans.iter().any(|s| s.italic);
        let code = spans.iter().any(|s| s.code);
        let font = fonts.resolve(bold, italic, code);
        let w = text_width_pt(font, &text, TABLE_FONT_SIZE) + TABLE_CELL_PADDING * 2.0;
        if let Some(width) = widths.get_mut(i) {
            *width = width.max(w);
        }
    }
}

fn natural_column_widths(
    fonts: &FontSet,
    num_cols: usize,
    header: &[Vec<Span>],
    rows: &[Vec<Vec<Span>>],
) -> Vec<f32> {
    let mut widths = vec![TABLE_CELL_PADDING * 2.0 + 10.0; num_cols];
    let mut header_bold: Vec<Vec<Span>> = header.to_vec();
    for cell in &mut header_bold {
        for span in cell.iter_mut() {
            span.bold = true;
        }
    }
    measure_row_natural(fonts, &header_bold, &mut widths);
    for row in rows {
        measure_row_natural(fonts, row, &mut widths);
    }
    widths
}

/// Wraps every cell of a row within its column width and returns the row's
/// total height — computed up front (separately from drawing) so a whole
/// table's height is known before any of it is committed to the page.
fn layout_table_row(
    fonts: &FontSet,
    col_widths: &[f32],
    cells: &[Vec<Span>],
    force_bold: bool,
) -> (Vec<Vec<Vec<Word>>>, f32) {
    let space_width = text_width_pt(&fonts.sans_regular, " ", TABLE_FONT_SIZE);
    let mut cell_lines = Vec::with_capacity(col_widths.len());
    for (i, &col_width) in col_widths.iter().enumerate() {
        let spans = cells.get(i).cloned().unwrap_or_default();
        let content_width = (col_width - TABLE_CELL_PADDING * 2.0).max(1.0);
        let tokens = tokenize(&spans, fonts, TABLE_FONT_SIZE, force_bold);
        let tokens = split_oversized_words(tokens, fonts, TABLE_FONT_SIZE, content_width);
        cell_lines.push(wrap_lines(tokens, content_width, space_width));
    }
    let line_count = cell_lines.iter().map(|l| l.len().max(1)).max().unwrap_or(1);
    let height = line_count as f32 * TABLE_ROW_LEADING + TABLE_CELL_PADDING * 2.0;
    (cell_lines, height)
}

struct FontIds {
    sans_regular: FontId,
    sans_bold: FontId,
    sans_italic: FontId,
    sans_bold_italic: FontId,
    mono_regular: FontId,
    mono_bold: FontId,
    mono_italic: FontId,
    mono_bold_italic: FontId,
}

/// Image bytes supplied by the caller, keyed by the URL as written in the
/// Markdown. The renderer never reads files or fetches URLs itself.
pub type Images = BTreeMap<String, Vec<u8>>;

/// An image already added to the document: its `XObject` and natural size.
#[derive(Clone)]
struct Placed {
    id: XObjectId,
    width_px: f32,
    height_px: f32,
}

struct Renderer<'a> {
    images: &'a Images,
    /// Decoded images by source; `None` when the source failed (already warned).
    placed: BTreeMap<String, Option<Placed>>,
    warnings: Vec<String>,
    doc: PdfDocument,
    fonts: FontSet,
    font_ids: FontIds,
    pages: Vec<PdfPage>,
    ops: Vec<Op>,
    /// Distance from the top of the page to the next free line box.
    cursor_top: f32,
}

impl<'a> Renderer<'a> {
    fn new(images: &'a Images) -> Result<Self, Md2PdfError> {
        let fonts = FontSet::load()?;
        let mut doc = PdfDocument::new("Markdown to PDF");
        let font_ids = FontIds {
            sans_regular: doc.add_font(&fonts.sans_regular),
            sans_bold: doc.add_font(&fonts.sans_bold),
            sans_italic: doc.add_font(&fonts.sans_italic),
            sans_bold_italic: doc.add_font(&fonts.sans_bold_italic),
            mono_regular: doc.add_font(&fonts.mono_regular),
            mono_bold: doc.add_font(&fonts.mono_bold),
            mono_italic: doc.add_font(&fonts.mono_italic),
            mono_bold_italic: doc.add_font(&fonts.mono_bold_italic),
        };
        Ok(Renderer {
            images,
            placed: BTreeMap::new(),
            warnings: Vec::new(),
            doc,
            fonts,
            font_ids,
            pages: Vec::new(),
            ops: Vec::new(),
            cursor_top: MARGIN_PT,
        })
    }

    fn font_id_for(&self, bold: bool, italic: bool, code: bool) -> FontId {
        match (code, bold, italic) {
            (true, true, true) => self.font_ids.mono_bold_italic.clone(),
            (true, true, false) => self.font_ids.mono_bold.clone(),
            (true, false, true) => self.font_ids.mono_italic.clone(),
            (true, false, false) => self.font_ids.mono_regular.clone(),
            (false, true, true) => self.font_ids.sans_bold_italic.clone(),
            (false, true, false) => self.font_ids.sans_bold.clone(),
            (false, false, true) => self.font_ids.sans_italic.clone(),
            (false, false, false) => self.font_ids.sans_regular.clone(),
        }
    }

    fn new_page(&mut self) {
        let ops = std::mem::take(&mut self.ops);
        self.pages.push(PdfPage::new(
            Mm::from(Pt(PAGE_WIDTH_PT)),
            Mm::from(Pt(PAGE_HEIGHT_PT)),
            ops,
        ));
        self.cursor_top = MARGIN_PT;
    }

    fn ensure_space(&mut self, needed: f32) {
        if self.cursor_top + needed > PAGE_HEIGHT_PT - MARGIN_PT {
            self.new_page();
        }
    }

    /// Advances the cursor by `leading` and returns the PDF-space baseline y
    /// for the line box just reserved.
    fn next_baseline(&mut self, leading: f32) -> f32 {
        let baseline_from_top = self.cursor_top + leading * BASELINE_RATIO;
        self.cursor_top += leading;
        PAGE_HEIGHT_PT - baseline_from_top
    }

    fn draw_text(&mut self, x: f32, y: f32, text: &str, style: TextStyle) {
        if text.is_empty() {
            return;
        }
        let TextStyle {
            bold,
            italic,
            code,
            size,
            color,
        } = style;
        let font_id = self.font_id_for(bold, italic, code);
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetFillColor { col: rgb(color) });
        self.ops.push(Op::StartTextSection);
        self.ops.push(Op::SetFont {
            font: PdfFontHandle::External(font_id),
            size: Pt(size),
        });
        self.ops.push(Op::SetTextMatrix {
            matrix: TextMatrix::Translate(Pt(x), Pt(y)),
        });
        self.ops.push(Op::ShowText {
            items: vec![TextItem::Text(text.to_string())],
        });
        self.ops.push(Op::EndTextSection);
        self.ops.push(Op::RestoreGraphicsState);
    }

    fn add_link_annotation(&mut self, x: f32, baseline_y: f32, width: f32, size: f32, url: String) {
        let rect = Rect::from_xywh(
            Pt(x),
            Pt(baseline_y - size * 0.2),
            Pt(width),
            Pt(size * 1.05),
        );
        self.ops.push(Op::LinkAnnotation {
            link: LinkAnnotation::new(
                rect,
                Actions::uri(url),
                Some(BorderArray::Solid([0.0, 0.0, 0.0])),
                None,
                Some(HighlightingMode::None),
            ),
        });
    }

    fn draw_strike(&mut self, x_from: f32, x_to: f32, baseline_y: f32, size: f32) {
        let y = baseline_y + size * STRIKE_RISE;
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetOutlineColor { col: rgb(BLACK) });
        self.ops.push(Op::SetOutlineThickness {
            pt: Pt(size * STRIKE_THICKNESS),
        });
        self.ops.push(Op::DrawLine {
            line: horizontal_line(x_from, x_to, y),
        });
        self.ops.push(Op::RestoreGraphicsState);
    }

    fn render_line(&mut self, line: &[Word], x0: f32, y: f32, size: f32) {
        let space_width = text_width_pt(&self.fonts.sans_regular, " ", size);
        let mut x = x0;
        // Start x of the strike line being extended, if any: consecutive
        // struck fragments (and the spaces between them) share one line.
        let mut strike_from: Option<f32> = None;
        for (i, word) in line.iter().enumerate() {
            for fragment in &word.fragments {
                let color = if fragment.link.is_some() {
                    LINK_COLOR
                } else {
                    BLACK
                };
                let (fragment_size, rise) = fragment.metrics(size);
                let style = TextStyle {
                    bold: fragment.bold,
                    italic: fragment.italic,
                    code: fragment.code,
                    size: fragment_size,
                    color,
                };
                self.draw_text(x, y + rise, &fragment.text, style);
                if let Some(url) = fragment.link.clone() {
                    self.add_link_annotation(x, y, fragment.width, size, url);
                }
                match (fragment.strike, strike_from) {
                    (true, None) => strike_from = Some(x),
                    (false, Some(from)) => {
                        self.draw_strike(from, x, y, size);
                        strike_from = None;
                    }
                    _ => {}
                }
                x += fragment.width;
            }
            let strike_continues =
                word.ends_struck() && line.get(i + 1).is_some_and(Word::starts_struck);
            if let Some(from) = strike_from
                && !strike_continues
            {
                self.draw_strike(from, x, y, size);
                strike_from = None;
            }
            x += space_width;
        }
    }

    fn render_lines(&mut self, lines: &[Vec<Word>], x0: f32, size: f32, leading: f32) {
        for line in lines {
            self.ensure_space(leading);
            let y = self.next_baseline(leading);
            self.render_line(line, x0, y, size);
        }
    }

    fn render_block(&mut self, block: &Block, x0: f32) {
        let max_width = (PAGE_WIDTH_PT - MARGIN_PT - x0).max(1.0);
        match block {
            Block::Heading { level, spans } => self.render_heading(*level, spans, x0, max_width),
            Block::Paragraph { spans } => self.render_paragraph(spans, x0, max_width),
            Block::CodeBlock { text } => self.render_code_block(text, x0, max_width),
            Block::List {
                ordered,
                start,
                items,
            } => self.render_list(*ordered, *start, items, x0),
            Block::BlockQuote(blocks) => self.render_blockquote(blocks, x0),
            Block::Table {
                alignments,
                header,
                rows,
            } => self.render_table(alignments, header, rows, x0),
            Block::ThematicBreak => self.render_rule(x0, max_width),
            Block::Image { src, alt, title } => self.render_image(src, alt, title, x0, max_width),
        }
    }

    fn wrap_spans_to_lines(
        &self,
        spans: &[Span],
        max_width: f32,
        size: f32,
        force_bold: bool,
        space_font: &printpdf::ParsedFont,
    ) -> Vec<Vec<Word>> {
        let space_width = text_width_pt(space_font, " ", size);
        let tokens = tokenize(spans, &self.fonts, size, force_bold);
        let tokens = split_oversized_words(tokens, &self.fonts, size, max_width);
        wrap_lines(tokens, max_width, space_width)
    }

    /// Italic `[image: alt]` paragraph standing in for an image that can't
    /// be embedded.
    fn render_image_fallback(&mut self, alt: &str, x0: f32, max_width: f32) {
        let span = Span {
            italic: true,
            ..Span::image_fallback(alt, ImageRef::default())
        };
        self.render_paragraph(&[span], x0, max_width);
    }

    fn render_heading(&mut self, level: u8, spans: &[Span], x0: f32, max_width: f32) {
        let idx = (level as usize).saturating_sub(1).min(5);
        let size = HEADING_SIZES.get(idx).copied().unwrap_or(BODY_SIZE);
        let leading = size * HEADING_LEADING_FACTOR;
        self.cursor_top += HEADING_SPACING_BEFORE;
        let lines = self.wrap_spans_to_lines(spans, max_width, size, true, &self.fonts.sans_bold);
        self.render_lines(&lines, x0, size, leading);
        self.cursor_top += HEADING_SPACING_AFTER;
    }

    fn render_paragraph(&mut self, spans: &[Span], x0: f32, max_width: f32) {
        let lines =
            self.wrap_spans_to_lines(spans, max_width, BODY_SIZE, false, &self.fonts.sans_regular);
        self.render_lines(&lines, x0, BODY_SIZE, BODY_LEADING);
        self.cursor_top += PARAGRAPH_SPACING_AFTER;
    }

    fn render_blockquote(&mut self, blocks: &[Block], x0: f32) {
        let quote_x = x0 + BLOCKQUOTE_INDENT;
        for block in blocks {
            self.render_block(block, quote_x);
        }
    }

    fn render_code_block(&mut self, text: &str, x0: f32, max_width: f32) {
        self.cursor_top += 4.0;
        let mut any = false;
        for line in text.lines() {
            any = true;
            for wrapped in wrap_monospace(&self.fonts, line, CODE_SIZE, max_width) {
                self.ensure_space(CODE_LEADING);
                let y = self.next_baseline(CODE_LEADING);
                let style = TextStyle {
                    code: true,
                    ..TextStyle::regular(CODE_SIZE)
                };
                self.draw_text(x0, y, &wrapped, style);
            }
        }
        if !any {
            self.ensure_space(CODE_LEADING);
            self.next_baseline(CODE_LEADING);
        }
        self.cursor_top += PARAGRAPH_SPACING_AFTER;
    }

    fn list_marker(ordered: bool, start: u64, index: usize) -> String {
        if ordered {
            format!("{}.", start + index as u64)
        } else {
            "\u{2022}".to_string()
        }
    }

    fn item_marker(ordered: bool, start: u64, index: usize, task: Option<bool>) -> Marker {
        match task {
            Some(checked) => Marker::Task(checked),
            None => Marker::Text(Self::list_marker(ordered, start, index)),
        }
    }

    /// Task checkbox drawn with vector strokes, so it never depends on a font
    /// having a ballot-box glyph. Sits on the baseline like a lowercase letter.
    fn draw_checkbox(&mut self, x: f32, baseline_y: f32, checked: bool) {
        let side = BODY_SIZE * CHECKBOX_SIZE;
        let bottom = baseline_y - BODY_SIZE * 0.05;
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetOutlineColor { col: rgb(BLACK) });
        self.ops.push(Op::SetOutlineThickness { pt: Pt(0.7) });
        self.ops.push(Op::DrawRectangle {
            rectangle: Rect {
                x: Pt(x),
                y: Pt(bottom),
                width: Pt(side),
                height: Pt(side),
                mode: Some(PaintMode::Stroke),
                winding_order: None,
            },
        });
        if checked {
            let point = |fx: f32, fy: f32| LinePoint {
                p: Point {
                    x: Pt(x + side * fx),
                    y: Pt(bottom + side * fy),
                },
                bezier: false,
            };
            self.ops.push(Op::SetOutlineThickness { pt: Pt(1.1) });
            self.ops.push(Op::DrawLine {
                line: Line {
                    points: vec![point(0.2, 0.5), point(0.42, 0.22), point(0.82, 0.8)],
                    is_closed: false,
                },
            });
        }
        self.ops.push(Op::RestoreGraphicsState);
    }

    fn reserve_list_line_baseline(&mut self) -> f32 {
        self.ensure_space(BODY_LEADING);
        self.next_baseline(BODY_LEADING)
    }

    fn draw_list_marker(&mut self, indent: f32, y: f32, marker: &Marker) {
        match marker {
            Marker::Text(text) => self.draw_text(indent, y, text, TextStyle::regular(BODY_SIZE)),
            Marker::Task(checked) => self.draw_checkbox(indent, y, *checked),
        }
    }

    fn render_list_continuation_lines(
        &mut self,
        lines: impl IntoIterator<Item = impl AsRef<[Word]>>,
        content_x: f32,
    ) {
        for line in lines {
            let y = self.reserve_list_line_baseline();
            self.render_line(line.as_ref(), content_x, y, BODY_SIZE);
        }
    }

    fn render_list_item_blocks(&mut self, blocks: &[Block], content_x: f32) {
        for block in blocks {
            self.render_block(block, content_x);
        }
    }

    fn render_list_item_paragraph_lead(
        &mut self,
        spans: &[Span],
        rest: &[Block],
        indent: f32,
        content_x: f32,
        max_width: f32,
        marker: &Marker,
    ) {
        let lines =
            self.wrap_spans_to_lines(spans, max_width, BODY_SIZE, false, &self.fonts.sans_regular);
        let mut lines_iter = lines.iter();
        if let Some(first_line) = lines_iter.next() {
            let y = self.reserve_list_line_baseline();
            self.draw_list_marker(indent, y, marker);
            self.render_line(first_line, content_x, y, BODY_SIZE);
        }
        self.render_list_continuation_lines(lines_iter, content_x);
        self.cursor_top += PARAGRAPH_SPACING_AFTER * 0.4;
        self.render_list_item_blocks(rest, content_x);
    }

    fn render_list_item_block_lead(
        &mut self,
        first: &Block,
        rest: &[Block],
        indent: f32,
        content_x: f32,
        marker: &Marker,
    ) {
        let y = self.reserve_list_line_baseline();
        self.draw_list_marker(indent, y, marker);
        self.render_block(first, content_x);
        self.render_list_item_blocks(rest, content_x);
    }

    fn render_list_item_marker_only(&mut self, indent: f32, marker: &Marker) {
        let y = self.reserve_list_line_baseline();
        self.draw_list_marker(indent, y, marker);
    }

    fn render_list_item(
        &mut self,
        item_blocks: &[Block],
        indent: f32,
        content_x: f32,
        max_width: f32,
        marker: &Marker,
    ) {
        match item_blocks.split_first() {
            Some((Block::Paragraph { spans }, rest)) => {
                self.render_list_item_paragraph_lead(
                    spans, rest, indent, content_x, max_width, marker,
                );
            }
            Some((first, rest)) => {
                self.render_list_item_block_lead(first, rest, indent, content_x, marker);
            }
            None => self.render_list_item_marker_only(indent, marker),
        }
    }

    fn render_list(&mut self, ordered: bool, start: u64, items: &[ListItem], indent: f32) {
        let content_x = indent + LIST_INDENT;
        let max_width = (PAGE_WIDTH_PT - MARGIN_PT - content_x).max(1.0);

        for (i, item) in items.iter().enumerate() {
            let marker = Self::item_marker(ordered, start, i, item.task);
            self.render_list_item(&item.blocks, indent, content_x, max_width, &marker);
        }
    }

    fn render_rule(&mut self, x0: f32, max_width: f32) {
        self.ensure_space(BODY_LEADING);
        self.cursor_top += BODY_LEADING * 0.5;
        let y = PAGE_HEIGHT_PT - self.cursor_top;
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetOutlineColor {
            col: rgb(RULE_GREY),
        });
        self.ops.push(Op::SetOutlineThickness { pt: Pt(1.0) });
        self.ops.push(Op::DrawLine {
            line: horizontal_line(x0, x0 + max_width, y),
        });
        self.ops.push(Op::RestoreGraphicsState);
        self.cursor_top += BODY_LEADING * 0.5;
    }

    fn render_table(
        &mut self,
        alignments: &[Align],
        header: &[Vec<Span>],
        rows: &[Vec<Vec<Span>>],
        x0: f32,
    ) {
        let num_cols = header
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0))
            .max(1);
        let available_width = (PAGE_WIDTH_PT - MARGIN_PT - x0).max(1.0);

        let mut col_widths = natural_column_widths(&self.fonts, num_cols, header, rows);
        let total: f32 = col_widths.iter().sum();
        if total > available_width {
            let scale = available_width / total;
            for w in &mut col_widths {
                *w *= scale;
            }
        }

        let (header_lines, header_height) =
            layout_table_row(&self.fonts, &col_widths, header, true);
        let row_layouts: Vec<(Vec<Vec<Vec<Word>>>, f32)> = rows
            .iter()
            .map(|r| layout_table_row(&self.fonts, &col_widths, r, false))
            .collect();
        let total_height: f32 = header_height + row_layouts.iter().map(|(_, h)| h).sum::<f32>();

        if self.cursor_top > MARGIN_PT
            && self.cursor_top + total_height > PAGE_HEIGHT_PT - MARGIN_PT
        {
            self.new_page();
        }

        self.cursor_top += 4.0;
        self.draw_table_row(&col_widths, alignments, &header_lines, header_height, x0);
        for (lines, height) in &row_layouts {
            self.ensure_space(*height);
            self.draw_table_row(&col_widths, alignments, lines, *height, x0);
        }
        self.cursor_top += PARAGRAPH_SPACING_AFTER;
    }

    fn draw_table_row(
        &mut self,
        col_widths: &[f32],
        alignments: &[Align],
        cell_lines: &[Vec<Vec<Word>>],
        row_height: f32,
        x0: f32,
    ) {
        let row_top = self.cursor_top;
        self.cursor_top += row_height;
        let row_top_pdf_y = PAGE_HEIGHT_PT - row_top;
        let row_bottom_pdf_y = row_top_pdf_y - row_height;
        let space_width = text_width_pt(&self.fonts.sans_regular, " ", TABLE_FONT_SIZE);

        let mut x = x0;
        for (i, &w) in col_widths.iter().enumerate() {
            self.ops.push(Op::SaveGraphicsState);
            self.ops.push(Op::SetOutlineColor {
                col: rgb(BORDER_GREY),
            });
            self.ops.push(Op::SetOutlineThickness {
                pt: Pt(TABLE_BORDER_THICKNESS),
            });
            self.ops.push(Op::DrawRectangle {
                rectangle: Rect {
                    x: Pt(x),
                    y: Pt(row_bottom_pdf_y),
                    width: Pt(w),
                    height: Pt(row_height),
                    mode: Some(PaintMode::Stroke),
                    winding_order: None,
                },
            });
            self.ops.push(Op::RestoreGraphicsState);

            let text_x0 = x + TABLE_CELL_PADDING;
            let content_width = (w - TABLE_CELL_PADDING * 2.0).max(1.0);
            let align = alignments.get(i).copied().unwrap_or(Align::None);
            if let Some(lines) = cell_lines.get(i) {
                for (li, line) in lines.iter().enumerate() {
                    let lw = line_width(line, space_width);
                    let extra = (content_width - lw).max(0.0);
                    let start_x = match align {
                        Align::Right => text_x0 + extra,
                        Align::Center => text_x0 + extra / 2.0,
                        Align::Left | Align::None => text_x0,
                    };
                    let baseline_y = row_top_pdf_y
                        - TABLE_CELL_PADDING
                        - (li as f32 + BASELINE_RATIO) * TABLE_ROW_LEADING;
                    self.render_line(line, start_x, baseline_y, TABLE_FONT_SIZE);
                }
            }
            x += w;
        }
    }

    /// Decodes and embeds `src` on first use; later uses share the `XObject`.
    /// Warns once and returns `None` when the image is missing or unusable.
    fn place_image(&mut self, src: &str) -> Option<Placed> {
        if let Some(placed) = self.placed.get(src) {
            return placed.clone();
        }
        let decoded = match self.images.get(src) {
            None => Err("image not found".to_string()),
            Some(bytes) => image::decode(bytes),
        };
        let placed = match decoded {
            Ok(decoded) => {
                let (width, height) = decoded.size_px();
                let id = match decoded {
                    Image::Raw(raw) => self.doc.add_image(&raw),
                    Image::Jpeg(jpeg) => self.doc.add_xobject(&jpeg),
                };
                Some(Placed {
                    id,
                    width_px: width as f32,
                    height_px: height as f32,
                })
            }
            Err(reason) => {
                self.warnings.push(format!("image {src}: {reason}"));
                None
            }
        };
        self.placed.insert(src.to_string(), placed.clone());
        placed
    }

    fn render_image(&mut self, src: &str, alt: &str, title: &str, x0: f32, max_width: f32) {
        let Some(placed) = self.place_image(src) else {
            self.render_image_fallback(alt, x0, max_width);
            return;
        };
        let caption = if title.is_empty() {
            Vec::new()
        } else {
            let span = Span {
                italic: true,
                ..Span::plain(title)
            };
            self.wrap_spans_to_lines(
                &[span],
                max_width,
                CAPTION_SIZE,
                false,
                &self.fonts.sans_italic,
            )
        };
        let caption_height = caption.len() as f32 * CAPTION_LEADING;
        let printable_height = PAGE_HEIGHT_PT - 2.0 * MARGIN_PT;
        // Never upscale; keep the caption on the image's page.
        let scale = (PT_PER_PX)
            .min(max_width / placed.width_px)
            .min(((printable_height - caption_height) / placed.height_px).max(0.0));
        let height = placed.height_px * scale;
        if self.cursor_top > MARGIN_PT
            && self.cursor_top + height + caption_height > PAGE_HEIGHT_PT - MARGIN_PT
        {
            self.new_page();
        }
        self.cursor_top += height;
        self.ops.push(Op::UseXobject {
            id: placed.id,
            transform: XObjectTransform {
                translate_x: Some(Pt(x0)),
                translate_y: Some(Pt(PAGE_HEIGHT_PT - self.cursor_top)),
                scale_x: Some(scale),
                scale_y: Some(scale),
                // 1 px = 1 pt before scaling.
                dpi: Some(72.0),
                ..Default::default()
            },
        });
        self.cursor_top += CAPTION_GAP;
        for line in &caption {
            let y = self.next_baseline(CAPTION_LEADING);
            self.render_line(line, x0, y, CAPTION_SIZE);
        }
        self.cursor_top += PARAGRAPH_SPACING_AFTER;
    }

    fn finish(mut self) -> (Vec<u8>, usize, Vec<String>) {
        self.pages.push(PdfPage::new(
            Mm::from(Pt(PAGE_WIDTH_PT)),
            Mm::from(Pt(PAGE_HEIGHT_PT)),
            std::mem::take(&mut self.ops),
        ));
        let page_count = self.pages.len();
        self.doc.pages = self.pages;
        let mut warnings = Vec::new();
        let bytes = self.doc.save(&PdfSaveOptions::default(), &mut warnings);
        (bytes, page_count, self.warnings)
    }
}

/// Returns the generated PDF bytes, the number of pages produced and the
/// warnings about content that had to fall back (missing images, …).
pub fn render(
    blocks: &[Block],
    images: &Images,
) -> Result<(Vec<u8>, usize, Vec<String>), Md2PdfError> {
    let mut renderer = Renderer::new(images)?;
    for block in blocks {
        renderer.render_block(block, MARGIN_PT);
    }
    Ok(renderer.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, width: f32) -> Word {
        let mut word = Word::default();
        word.push(Fragment {
            text: text.to_string(),
            bold: false,
            italic: false,
            code: false,
            strike: false,
            superscript: false,
            link: None,
            width,
        });
        word
    }

    fn line_texts(lines: &[Vec<Word>]) -> Vec<Vec<String>> {
        lines
            .iter()
            .map(|l| {
                l.iter()
                    .map(|w| w.fragments.iter().map(|f| f.text.as_str()).collect())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn wrap_lines_breaks_once_max_width_is_exceeded() {
        let tokens = vec![
            Token::Word(word("aaa", 30.0)),
            Token::Word(word("bbb", 30.0)),
            Token::Word(word("ccc", 30.0)),
        ];
        let lines = wrap_lines(tokens, 65.0, 5.0);
        assert_eq!(line_texts(&lines), vec![vec!["aaa", "bbb"], vec!["ccc"]]);
    }

    #[test]
    fn wrap_lines_starts_a_new_line_on_an_explicit_break() {
        let tokens = vec![
            Token::Word(word("aaa", 10.0)),
            Token::Break,
            Token::Word(word("bbb", 10.0)),
        ];
        let lines = wrap_lines(tokens, 1000.0, 5.0);
        assert_eq!(line_texts(&lines), vec![vec!["aaa"], vec!["bbb"]]);
    }

    #[test]
    fn list_marker_is_bullet_when_unordered() {
        assert_eq!(Renderer::list_marker(false, 1, 3), "\u{2022}");
    }

    #[test]
    fn list_marker_counts_up_from_start_when_ordered() {
        assert_eq!(Renderer::list_marker(true, 3, 0), "3.");
        assert_eq!(Renderer::list_marker(true, 3, 2), "5.");
    }

    #[test]
    fn wrap_monospace_hard_wraps_without_word_boundaries() {
        let fonts = FontSet::load().unwrap();
        let lines = wrap_monospace(&fonts, "abcdefghij", 10.0, 1.0);
        assert_eq!(lines.len(), "abcdefghij".len());
        assert_eq!(lines.concat(), "abcdefghij");
    }

    #[test]
    fn wrap_monospace_keeps_a_short_line_on_one_row() {
        let fonts = FontSet::load().unwrap();
        let lines = wrap_monospace(&fonts, "hi", 10.0, 1000.0);
        assert_eq!(lines, vec!["hi".to_string()]);
    }

    #[test]
    fn hard_wrap_chars_splits_long_text_within_max_width() {
        let fonts = FontSet::load().unwrap();
        let font = fonts.resolve(false, false, false);
        let text = "src/features/Configuration/report_templates/ReportTemplateEditor.tsx";
        let max_width = 100.0;
        let chunks = hard_wrap_chars(font, text, 24.0, max_width);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), text);
        for chunk in &chunks {
            assert!(text_width_pt(font, chunk, 24.0) <= max_width);
        }
    }

    #[test]
    fn long_unbreakable_heading_token_wraps_instead_of_overflowing() {
        let images = Images::new();
        let renderer = Renderer::new(&images).unwrap();
        let long_token = "src/features/Configuration/report_templates/ReportTemplateEditor.tsx";
        let max_width = 200.0;
        let lines = renderer.wrap_spans_to_lines(
            &[Span::plain(long_token)],
            max_width,
            HEADING_SIZES[0],
            true,
            &renderer.fonts.sans_bold,
        );
        assert!(lines.len() > 1);
        let space_width = text_width_pt(&renderer.fonts.sans_bold, " ", HEADING_SIZES[0]);
        for line in &lines {
            assert!(line_width(line, space_width) <= max_width);
        }
    }

    struct Rendered {
        /// Ops of each page, the last one still open.
        pages: Vec<Vec<Op>>,
        warnings: Vec<String>,
        xobjects: usize,
    }

    impl Rendered {
        fn ops(&self) -> Vec<Op> {
            self.pages.concat()
        }
    }

    fn render_with(blocks: &[Block], images: &Images) -> Rendered {
        let mut renderer = Renderer::new(images).unwrap();
        for block in blocks {
            renderer.render_block(block, MARGIN_PT);
        }
        let mut pages: Vec<Vec<Op>> = renderer.pages.iter().map(|p| p.ops.clone()).collect();
        pages.push(renderer.ops);
        Rendered {
            pages,
            warnings: renderer.warnings,
            xobjects: renderer.doc.resources.xobjects.map.len(),
        }
    }

    fn rendered_ops(blocks: &[Block]) -> Vec<Op> {
        render_with(blocks, &Images::new()).ops()
    }

    /// `(text, x, y)` of every text run, in drawing order.
    fn text_runs(ops: &[Op]) -> Vec<(String, f32, f32)> {
        let mut at = (0.0, 0.0);
        let mut runs = Vec::new();
        for op in ops {
            match op {
                Op::SetTextMatrix {
                    matrix: TextMatrix::Translate(x, y),
                } => at = (x.0, y.0),
                Op::ShowText { items } => {
                    for item in items {
                        if let TextItem::Text(t) = item {
                            runs.push((t.clone(), at.0, at.1));
                        }
                    }
                }
                _ => {}
            }
        }
        runs
    }

    fn lines_drawn(ops: &[Op]) -> Vec<&Line> {
        ops.iter()
            .filter_map(|op| match op {
                Op::DrawLine { line } => Some(line),
                _ => None,
            })
            .collect()
    }

    fn paragraph(spans: Vec<Span>) -> Block {
        Block::Paragraph { spans }
    }

    #[test]
    fn adjacent_styles_inside_a_word_get_no_space() {
        let fonts = FontSet::load().unwrap();
        let ops = rendered_ops(&[paragraph(vec![
            Span::plain("foo"),
            Span {
                text: "bar".into(),
                bold: true,
                ..Default::default()
            },
            Span::plain(" baz"),
        ])]);
        let runs = text_runs(&ops);
        let texts: Vec<&str> = runs.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(texts, ["foo", "bar", "baz"]);
        let foo_width = text_width_pt(&fonts.sans_regular, "foo", BODY_SIZE);
        let bar_width = text_width_pt(&fonts.sans_bold, "bar", BODY_SIZE);
        let space = text_width_pt(&fonts.sans_regular, " ", BODY_SIZE);
        assert!((runs[1].1 - (runs[0].1 + foo_width)).abs() < 0.01);
        assert!((runs[2].1 - (runs[1].1 + bar_width + space)).abs() < 0.01);
    }

    #[test]
    fn strikethrough_draws_a_line_through_the_struck_text_only() {
        let fonts = FontSet::load().unwrap();
        let struck = |t: &str| Span {
            text: t.into(),
            strike: true,
            ..Default::default()
        };
        let ops = rendered_ops(&[paragraph(vec![
            Span::plain("keep "),
            struck("gone too"),
            Span::plain(" keep"),
        ])]);
        let runs = text_runs(&ops);
        let lines = lines_drawn(&ops);
        assert_eq!(lines.len(), 1, "one continuous strike over both words");
        let points = &lines[0].points;
        let (gone_x, baseline) = (runs[1].1, runs[1].2);
        let too_end = runs[2].1 + text_width_pt(&fonts.sans_regular, "too", BODY_SIZE);
        assert!((points[0].p.x.0 - gone_x).abs() < 0.01);
        assert!((points[1].p.x.0 - too_end).abs() < 0.01);
        assert!(points[0].p.y.0 > baseline && points[0].p.y.0 < baseline + BODY_SIZE * 0.5);
    }

    #[test]
    fn plain_text_draws_no_strike_line() {
        let ops = rendered_ops(&[paragraph(vec![Span::plain("nothing struck")])]);
        assert!(lines_drawn(&ops).is_empty());
    }

    fn list(items: Vec<(Option<bool>, &str)>) -> Block {
        Block::List {
            ordered: false,
            start: 1,
            items: items
                .into_iter()
                .map(|(task, text)| ListItem {
                    task,
                    blocks: vec![paragraph(vec![Span::plain(text)])],
                })
                .collect(),
        }
    }

    fn rectangles(ops: &[Op]) -> Vec<&Rect> {
        ops.iter()
            .filter_map(|op| match op {
                Op::DrawRectangle { rectangle } => Some(rectangle),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn task_items_draw_a_vector_box_instead_of_a_bullet() {
        let ops = rendered_ops(&[list(vec![
            (Some(true), "done"),
            (Some(false), "todo"),
            (None, "plain"),
        ])]);
        let runs = text_runs(&ops);
        let texts: Vec<&str> = runs.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(texts, ["done", "todo", "\u{2022}", "plain"]);
        // Task text lines up with bulleted text.
        assert!((runs[0].1 - runs[3].1).abs() < 0.01);
        assert!((runs[1].1 - runs[3].1).abs() < 0.01);

        let boxes = rectangles(&ops);
        assert_eq!(boxes.len(), 2, "one box per task item");
        for (b, run) in boxes.iter().zip(&runs) {
            assert!(b.x.0 >= MARGIN_PT && b.x.0 + b.width.0 < run.1);
            assert!(b.y.0 < run.2 + BODY_SIZE && b.y.0 + b.height.0 > run.2);
        }
        let checks = lines_drawn(&ops)
            .into_iter()
            .filter(|l| l.points.len() == 3)
            .count();
        assert_eq!(checks, 1, "only the checked item gets a check mark");
    }

    #[test]
    fn superscript_is_smaller_raised_and_attached_to_its_word() {
        let fonts = FontSet::load().unwrap();
        let ops = rendered_ops(&[paragraph(vec![
            Span::plain("One"),
            Span {
                text: "12".into(),
                superscript: true,
                ..Default::default()
            },
            Span::plain(" two"),
        ])]);
        let runs = text_runs(&ops);
        let sizes: Vec<f32> = ops
            .iter()
            .filter_map(|op| match op {
                Op::SetFont { size, .. } => Some(size.0),
                _ => None,
            })
            .collect();
        assert_eq!(sizes, [BODY_SIZE, BODY_SIZE * SUPERSCRIPT_SCALE, BODY_SIZE]);
        let one_width = text_width_pt(&fonts.sans_regular, "One", BODY_SIZE);
        let mark_width = text_width_pt(&fonts.sans_regular, "12", BODY_SIZE * SUPERSCRIPT_SCALE);
        let space = text_width_pt(&fonts.sans_regular, " ", BODY_SIZE);
        assert!((runs[1].1 - (runs[0].1 + one_width)).abs() < 0.01);
        assert!(runs[1].2 > runs[0].2);
        assert!((runs[2].1 - (runs[1].1 + mark_width + space)).abs() < 0.01);
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Grayscale);
            let mut writer = encoder.write_header().unwrap();
            let pixels = vec![128; (width * height) as usize];
            writer.write_image_data(&pixels).unwrap();
        }
        out
    }

    fn image(src: &str, title: &str) -> Block {
        Block::Image {
            src: src.into(),
            alt: "the alt".into(),
            title: title.into(),
        }
    }

    /// `(x, y, scale_x, scale_y)` of every placed `XObject`.
    fn placements(ops: &[Op]) -> Vec<(f32, f32, f32, f32)> {
        ops.iter()
            .filter_map(|op| match op {
                Op::UseXobject { transform, .. } => Some((
                    transform.translate_x.unwrap().0,
                    transform.translate_y.unwrap().0,
                    transform.scale_x.unwrap(),
                    transform.scale_y.unwrap(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn image_is_drawn_at_96_dpi_below_the_cursor() {
        let images = Images::from([("a.png".to_string(), png_bytes(200, 100))]);
        let rendered = render_with(&[image("a.png", "")], &images);
        assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);
        let placed = placements(&rendered.ops());
        assert_eq!(placed.len(), 1);
        let (x, y, sx, sy) = placed[0];
        // 200 × 100 px at 96 dpi = 150 × 75 pt, with 1 px = 1 pt before scaling.
        assert!((sx * 200.0 - 150.0).abs() < 0.01 && (sy * 100.0 - 75.0).abs() < 0.01);
        assert!((x - MARGIN_PT).abs() < 0.01);
        assert!(y < PAGE_HEIGHT_PT - MARGIN_PT - 75.0 + 0.01);
    }

    #[test]
    fn wide_image_shrinks_to_the_content_width_keeping_aspect_ratio() {
        let images = Images::from([("w.png".to_string(), png_bytes(2000, 500))]);
        let ops = render_with(&[image("w.png", "")], &images).ops();
        let (_, _, sx, sy) = placements(&ops)[0];
        let content_width = PAGE_WIDTH_PT - 2.0 * MARGIN_PT;
        assert!((sx * 2000.0 - content_width).abs() < 0.01);
        assert!((sx - sy).abs() < 1e-6, "aspect ratio kept");
    }

    #[test]
    fn tall_image_shrinks_to_the_printable_height() {
        let images = Images::from([("t.png".to_string(), png_bytes(100, 3000))]);
        let ops = render_with(&[image("t.png", "")], &images).ops();
        let (_, y, _, sy) = placements(&ops)[0];
        assert!((sy * 3000.0 - (PAGE_HEIGHT_PT - 2.0 * MARGIN_PT)).abs() < 0.01);
        assert!((y - MARGIN_PT).abs() < 0.01);
    }

    #[test]
    fn image_that_does_not_fit_starts_a_new_page() {
        let images = Images::from([("a.png".to_string(), png_bytes(400, 600))]);
        let filler: Vec<Block> = (0..30)
            .map(|_| paragraph(vec![Span::plain("line")]))
            .collect();
        let mut blocks = filler;
        blocks.push(image("a.png", ""));
        let rendered = render_with(&blocks, &images);
        assert_eq!(rendered.pages.len(), 2);
        assert!(placements(&rendered.pages[0]).is_empty());
        assert_eq!(placements(&rendered.pages[1]).len(), 1);
    }

    #[test]
    fn title_becomes_a_caption_under_the_image() {
        let images = Images::from([("a.png".to_string(), png_bytes(200, 100))]);
        let ops = render_with(&[image("a.png", "Figure one")], &images).ops();
        let (_, image_bottom, _, _) = placements(&ops)[0];
        let runs = text_runs(&ops);
        let texts: Vec<&str> = runs.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(texts, ["Figure", "one"]);
        assert!(runs[0].2 < image_bottom);
    }

    #[test]
    fn repeated_image_is_embedded_once() {
        let images = Images::from([("a.png".to_string(), png_bytes(10, 10))]);
        let rendered = render_with(&[image("a.png", ""), image("a.png", "")], &images);
        assert_eq!(placements(&rendered.ops()).len(), 2);
        assert_eq!(rendered.xobjects, 1);
    }

    #[test]
    fn missing_or_broken_image_falls_back_to_alt_text_with_a_warning() {
        let images = Images::from([("bad.png".to_string(), b"not an image".to_vec())]);
        let rendered = render_with(&[image("missing.png", ""), image("bad.png", "")], &images);
        let ops = rendered.ops();
        assert!(placements(&ops).is_empty());
        let texts: Vec<String> = text_runs(&ops).into_iter().map(|r| r.0).collect();
        assert_eq!(texts, ["[image:", "the", "alt]", "[image:", "the", "alt]"]);
        assert_eq!(rendered.warnings.len(), 2);
        assert!(rendered.warnings[0].contains("missing.png"));
        assert!(rendered.warnings[1].contains("bad.png"));
    }

    #[test]
    fn render_produces_a_single_page_pdf() {
        let blocks = vec![
            Block::Heading {
                level: 1,
                spans: vec![Span::plain("Title")],
            },
            Block::Paragraph {
                spans: vec![Span::plain("Body text.")],
            },
        ];
        let (bytes, page_count, _) = render(&blocks, &Images::new()).unwrap();
        assert_eq!(page_count, 1);
        assert!(bytes.starts_with(b"%PDF"));
    }
}
