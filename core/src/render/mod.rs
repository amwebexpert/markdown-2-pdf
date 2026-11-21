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
use std::ops::Range;

mod table;

use table::{RowLayout, column_demands, column_widths, row_height};

use crate::error::Md2PdfError;
use crate::fonts::{FontSet, text_width_pt};
use crate::image::{self, Decoded};
use crate::ir::{Align, Block, Image as ImageBlock, ListItem, Span};

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
/// Room a heading reserves for its section's start: two body lines.
const HEADING_KEEP_WITH_NEXT: f32 = 2.0 * BODY_LEADING;
const CODE_SPACING_BEFORE: f32 = 4.0;
/// Widow/orphan control: only paragraphs of this many lines are kept from
/// leaving fewer than `MIN_LINES_PER_SIDE` lines on either side of a break.
const MIN_LINES_TO_SPLIT: usize = 4;
const MIN_LINES_PER_SIDE: usize = 2;
const CODE_PADDING: f32 = 4.0;
const CODE_BACKGROUND: (f32, f32, f32) = (0.96, 0.96, 0.96);
/// Blockquote side bar: x offset inside the quote indent, and stroke width.
const BLOCKQUOTE_BAR_INSET: f32 = 6.0;
const BLOCKQUOTE_BAR_WIDTH: f32 = 2.0;
const LIST_INDENT: f32 = 18.0;
const BLOCKQUOTE_INDENT: f32 = 18.0;
const TABLE_FONT_SIZE: f32 = 10.0;
const TABLE_ROW_LEADING: f32 = 13.0;
const TABLE_CELL_PADDING: f32 = 4.0;
const TABLE_BORDER_THICKNESS: f32 = 0.6;
const TABLE_SPACING_BEFORE: f32 = 4.0;
/// A header taller than this fraction of the printable height is drawn once
/// rather than repeated on every page.
const MAX_REPEATED_HEADER: f32 = 0.5;
/// Narrowest column content width, so an empty column stays visible.
const MIN_COLUMN_CONTENT: f32 = 10.0;
/// Baseline distance below the top of a line box, as a fraction of leading —
/// an approximation (fonts don't share one ascent ratio) good enough since no
/// pixel-perfect typography was requested.
const BASELINE_RATIO: f32 = 0.82;
/// Strike line height above the baseline and thickness, as fractions of the
/// font size (roughly the x-height middle of Liberation Sans).
const STRIKE_RISE: f32 = 0.3;
const STRIKE_THICKNESS: f32 = 0.06;
/// Task checkbox: side and drop below the baseline as fractions of the body
/// font size, stroke widths in pt, and the check mark's polyline as fractions
/// of the box side (from its bottom-left corner).
const CHECKBOX_SIZE: f32 = 0.7;
const CHECKBOX_DROP: f32 = 0.05;
const CHECKBOX_STROKE: f32 = 0.7;
const CHECK_MARK_STROKE: f32 = 1.1;
const CHECK_MARK: [(f32, f32); 3] = [(0.2, 0.5), (0.42, 0.22), (0.82, 0.8)];
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

/// Shared geometry of one table being drawn.
struct TableFrame<'t> {
    x0: f32,
    col_widths: &'t [f32],
    alignments: &'t [Align],
    header: &'t RowLayout,
    /// False for a header too tall to repeat (see `MAX_REPEATED_HEADER`): it is then
    /// drawn once, split like any tall row.
    repeat_header: bool,
}

/// Lookout for the first content a block draws: `(page, top)` once found.
#[derive(Clone, Copy, PartialEq)]
enum Probe {
    Off,
    Waiting,
    Found(PagePos),
}

/// A spot in the document: page index and distance from the page top.
#[derive(Clone, Copy, PartialEq)]
struct PagePos {
    page: usize,
    top: f32,
}

/// Where a block started, and how many ops its first page already had —
/// see `Renderer::decorate`.
struct Mark {
    at: PagePos,
    op_index: usize,
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
    /// Where each finished page's content ended (see `content_bottom`).
    page_ends: Vec<f32>,
    /// Where the next line, table row or image lands, when a list item wants
    /// its marker there (see `occupy`).
    first_content: Probe,
    /// Bottom of the last thing drawn on the current page, excluding the
    /// spacing after it: where a decoration of the block above should end.
    content_bottom: f32,
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
            page_ends: Vec::new(),
            content_bottom: MARGIN_PT,
            first_content: Probe::Off,
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
        self.page_ends.push(self.content_bottom);
        self.content_bottom = MARGIN_PT;
        let ops = std::mem::take(&mut self.ops);
        self.pages.push(PdfPage::new(
            Mm::from(Pt(PAGE_WIDTH_PT)),
            Mm::from(Pt(PAGE_HEIGHT_PT)),
            ops,
        ));
        self.cursor_top = MARGIN_PT;
    }

    /// Starts a new page unless `needed` fits under the cursor (or the
    /// cursor is already at the top, where a new page would not help).
    fn ensure_space(&mut self, needed: f32) {
        if !self.at_page_top() && self.cursor_top + needed > Self::page_bottom() {
            self.new_page();
        }
    }

    fn position(&self) -> PagePos {
        PagePos {
            page: self.pages.len(),
            top: self.cursor_top,
        }
    }

    /// Takes `height` of the page for something being drawn there and returns
    /// its top. Every drawing primitive goes through this, so the first-line
    /// probe and `content_bottom` stay right.
    fn occupy(&mut self, height: f32) -> f32 {
        if self.first_content == Probe::Waiting {
            self.first_content = Probe::Found(self.position());
        }
        let top = self.cursor_top;
        self.cursor_top += height;
        self.content_bottom = self.cursor_top;
        top
    }

    /// Advances the cursor by `leading` and returns the PDF-space baseline y
    /// for the line box just reserved.
    fn next_baseline(&mut self, leading: f32) -> f32 {
        let top = self.occupy(leading);
        PAGE_HEIGHT_PT - (top + leading * BASELINE_RATIO)
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
        self.add_link(rect, url);
    }

    fn add_link(&mut self, rect: Rect, url: String) {
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

    /// Strike line in the struck text's color (`color`).
    fn draw_strike(
        &mut self,
        x_from: f32,
        x_to: f32,
        baseline_y: f32,
        size: f32,
        color: (f32, f32, f32),
    ) {
        let y = baseline_y + size * STRIKE_RISE;
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetOutlineColor { col: rgb(color) });
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
        // Start x and color of the strike line being extended, if any:
        // consecutive struck fragments of one color (and the spaces between
        // them) share one line.
        let mut strike: Option<(f32, (f32, f32, f32))> = None;
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
                if let Some((from, strike_color)) = strike
                    && (!fragment.strike || strike_color != color)
                {
                    self.draw_strike(from, x, y, size, strike_color);
                    strike = None;
                }
                if fragment.strike && strike.is_none() {
                    strike = Some((x, color));
                }
                x += fragment.width;
            }
            let strike_continues =
                word.ends_struck() && line.get(i + 1).is_some_and(Word::starts_struck);
            if let Some((from, strike_color)) = strike
                && !strike_continues
            {
                self.draw_strike(from, x, y, size, strike_color);
                strike = None;
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
            Block::Image(image) => self.render_image(image, x0, max_width),
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
            ..Span::image_fallback(ImageBlock {
                alt: alt.to_string(),
                ..ImageBlock::default()
            })
        };
        self.render_paragraph(&[span], x0, max_width);
    }

    /// Vertical gap before a block, dropped at the top of a page.
    fn space_before(&mut self, gap: f32) {
        if !self.at_page_top() {
            self.cursor_top += gap;
        }
    }

    fn render_heading(&mut self, level: u8, spans: &[Span], x0: f32, max_width: f32) {
        let idx = (level as usize).saturating_sub(1).min(5);
        let size = HEADING_SIZES.get(idx).copied().unwrap_or(BODY_SIZE);
        let leading = size * HEADING_LEADING_FACTOR;
        let lines = self.wrap_spans_to_lines(spans, max_width, size, true, &self.fonts.sans_bold);
        // Keep the heading with the start of its section.
        self.ensure_space(
            HEADING_SPACING_BEFORE
                + lines.len() as f32 * leading
                + HEADING_SPACING_AFTER
                + HEADING_KEEP_WITH_NEXT,
        );
        self.space_before(HEADING_SPACING_BEFORE);
        self.render_lines(&lines, x0, size, leading);
        self.cursor_top += HEADING_SPACING_AFTER;
    }

    /// Index of the first line to move to the next page, if the lines don't
    /// all fit under the cursor. Paragraphs of [`MIN_LINES_TO_SPLIT`]+ lines
    /// keep at least [`MIN_LINES_PER_SIDE`] lines on each side of the break
    /// (no orphan at the page bottom, no widow at the next page's top).
    fn paragraph_break(&self, count: usize, leading: f32) -> Option<usize> {
        let fit = (1..=count)
            .take_while(|&n| self.cursor_top + n as f32 * leading <= Self::page_bottom())
            .count();
        if fit == count {
            return None;
        }
        if count < MIN_LINES_TO_SPLIT {
            return Some(fit);
        }
        if fit < MIN_LINES_PER_SIDE {
            return Some(0);
        }
        Some(fit.min(count - MIN_LINES_PER_SIDE))
    }

    fn render_paragraph(&mut self, spans: &[Span], x0: f32, max_width: f32) {
        let lines =
            self.wrap_spans_to_lines(spans, max_width, BODY_SIZE, false, &self.fonts.sans_regular);
        let split = self
            .paragraph_break(lines.len(), BODY_LEADING)
            .unwrap_or(lines.len());
        let (before, after) = lines.split_at(split);
        self.render_lines(before, x0, BODY_SIZE, BODY_LEADING);
        if !after.is_empty() && !self.at_page_top() {
            self.new_page();
        }
        self.render_lines(after, x0, BODY_SIZE, BODY_LEADING);
        self.cursor_top += PARAGRAPH_SPACING_AFTER;
    }

    /// Ops of page `page`: a finished page, or the one being drawn.
    fn ops_of_page(&mut self, page: usize) -> &mut Vec<Op> {
        match self.pages.get_mut(page) {
            Some(finished) => &mut finished.ops,
            None => &mut self.ops,
        }
    }

    fn mark(&self) -> Mark {
        Mark {
            at: self.position(),
            op_index: self.ops.len(),
        }
    }

    /// Decorates the content drawn since `mark`, once per page it spans:
    /// `draw(top, bottom)` gets the fragment's extent (PDF y, top > bottom) and
    /// returns ops inserted before that fragment's content, so they sit
    /// beneath it. Pages where the block drew nothing are skipped.
    fn decorate(&mut self, mark: &Mark, draw: impl Fn(f32, f32) -> Vec<Op>) {
        for page in mark.at.page..=self.pages.len() {
            let top = if page == mark.at.page {
                mark.at.top
            } else {
                MARGIN_PT
            };
            let bottom = self
                .page_ends
                .get(page)
                .copied()
                .unwrap_or(self.content_bottom);
            if bottom <= top {
                continue;
            }
            let at = if page == mark.at.page {
                mark.op_index
            } else {
                0
            };
            let ops = draw(PAGE_HEIGHT_PT - top, PAGE_HEIGHT_PT - bottom);
            let target = self.ops_of_page(page);
            let at = at.min(target.len());
            target.splice(at..at, ops);
        }
    }

    fn render_blockquote(&mut self, blocks: &[Block], x0: f32) {
        let quote_x = x0 + BLOCKQUOTE_INDENT;
        let mark = self.mark();
        for block in blocks {
            self.render_block(block, quote_x);
        }
        let bar_x = x0 + BLOCKQUOTE_BAR_INSET;
        self.decorate(&mark, |top, bottom| {
            vec![
                Op::SaveGraphicsState,
                Op::SetOutlineColor {
                    col: rgb(RULE_GREY),
                },
                Op::SetOutlineThickness {
                    pt: Pt(BLOCKQUOTE_BAR_WIDTH),
                },
                Op::DrawLine {
                    line: Line {
                        points: [top, bottom]
                            .map(|y| LinePoint {
                                p: Point {
                                    x: Pt(bar_x),
                                    y: Pt(y),
                                },
                                bezier: false,
                            })
                            .to_vec(),
                        is_closed: false,
                    },
                },
                Op::RestoreGraphicsState,
            ]
        });
    }

    fn render_code_block(&mut self, text: &str, x0: f32, max_width: f32) {
        self.space_before(CODE_SPACING_BEFORE);
        let text_x = x0 + CODE_PADDING;
        let wrap_width = (max_width - 2.0 * CODE_PADDING).max(1.0);
        let mut lines: Vec<String> = text
            .lines()
            .flat_map(|line| wrap_monospace(&self.fonts, line, CODE_SIZE, wrap_width))
            .collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        // Start where the first line will be drawn, not on a page it leaves.
        self.ensure_space(CODE_LEADING + CODE_PADDING);
        let mark = self.mark();
        self.cursor_top += CODE_PADDING;
        for line in &lines {
            self.ensure_space(CODE_LEADING);
            let y = self.next_baseline(CODE_LEADING);
            let style = TextStyle {
                code: true,
                ..TextStyle::regular(CODE_SIZE)
            };
            self.draw_text(text_x, y, line, style);
        }
        self.occupy(CODE_PADDING);
        self.decorate(&mark, |top, bottom| {
            vec![
                Op::SaveGraphicsState,
                Op::SetFillColor {
                    col: rgb(CODE_BACKGROUND),
                },
                Op::DrawRectangle {
                    rectangle: Rect {
                        x: Pt(x0),
                        y: Pt(bottom),
                        width: Pt(max_width),
                        height: Pt(top - bottom),
                        mode: Some(PaintMode::Fill),
                        winding_order: None,
                    },
                },
                Op::RestoreGraphicsState,
            ]
        });
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
        let bottom = baseline_y - BODY_SIZE * CHECKBOX_DROP;
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetOutlineColor { col: rgb(BLACK) });
        self.ops.push(Op::SetOutlineThickness {
            pt: Pt(CHECKBOX_STROKE),
        });
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
            self.ops.push(Op::SetOutlineThickness {
                pt: Pt(CHECK_MARK_STROKE),
            });
            self.ops.push(Op::DrawLine {
                line: Line {
                    points: CHECK_MARK.iter().map(|&(fx, fy)| point(fx, fy)).collect(),
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
        // The marker goes on the block's first line, wherever (and on
        // whichever page) that ends up. An enclosing item may be waiting for
        // the same first line, so its probe is restored afterwards.
        let outer = std::mem::replace(&mut self.first_content, Probe::Waiting);
        self.render_block(first, content_x);
        let first_line = match std::mem::replace(&mut self.first_content, outer) {
            Probe::Found(at) => at,
            // A block that drew nothing still gets its marker, on a line of its own.
            Probe::Off | Probe::Waiting => {
                self.ensure_space(BODY_LEADING);
                let at = self.position();
                self.occupy(BODY_LEADING);
                at
            }
        };
        if self.first_content == Probe::Waiting {
            self.first_content = Probe::Found(first_line);
        }
        let saved = std::mem::take(&mut self.ops);
        let baseline = PAGE_HEIGHT_PT - first_line.top - BODY_LEADING * BASELINE_RATIO;
        self.draw_list_marker(indent, baseline, marker);
        let marker_ops = std::mem::replace(&mut self.ops, saved);
        self.ops_of_page(first_line.page).extend(marker_ops);
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
        let y = PAGE_HEIGHT_PT - (self.occupy(BODY_LEADING) + BODY_LEADING * 0.5);
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetOutlineColor {
            col: rgb(RULE_GREY),
        });
        self.ops.push(Op::SetOutlineThickness { pt: Pt(1.0) });
        self.ops.push(Op::DrawLine {
            line: horizontal_line(x0, x0 + max_width, y),
        });
        self.ops.push(Op::RestoreGraphicsState);
    }

    fn page_bottom() -> f32 {
        PAGE_HEIGHT_PT - MARGIN_PT
    }

    fn printable_height() -> f32 {
        Self::page_bottom() - MARGIN_PT
    }

    fn at_page_top(&self) -> bool {
        self.cursor_top <= MARGIN_PT
    }

    /// Tables paginate row by row: the header is kept with the first row and
    /// repeated at the top of every continuation page. A row never splits
    /// unless it is taller than a whole page under the header; then it is cut
    /// between lines, each part with its own borders.
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

        let demands = column_demands(&self.fonts, num_cols, header, rows);
        let col_widths = column_widths(&demands, available_width);
        let header = RowLayout::new(&self.fonts, &col_widths, header, true);
        let rows: Vec<RowLayout> = rows
            .iter()
            .map(|r| RowLayout::new(&self.fonts, &col_widths, r, false))
            .collect();
        let frame = TableFrame {
            x0,
            col_widths: &col_widths,
            alignments,
            header: &header,
            repeat_header: header.height() <= Self::printable_height() * MAX_REPEATED_HEADER,
        };

        // Keep the header with the first row (or the first line of it).
        let first = rows.first().map_or(0.0, |r| row_height(r.lines.min(1)));
        self.ensure_space(TABLE_SPACING_BEFORE + header.height() + first);
        self.space_before(TABLE_SPACING_BEFORE);
        if frame.repeat_header {
            self.draw_row_part(&frame, &header, 0..header.lines);
        } else {
            self.draw_table_row(&frame, &header);
        }
        for row in &rows {
            self.draw_table_row(&frame, row);
        }
        self.cursor_top += PARAGRAPH_SPACING_AFTER;
    }

    fn continue_table_on_new_page(&mut self, frame: &TableFrame<'_>) {
        self.new_page();
        if frame.repeat_header {
            self.draw_row_part(frame, frame.header, 0..frame.header.lines);
        }
    }

    fn draw_table_row(&mut self, frame: &TableFrame<'_>, row: &RowLayout) {
        let repeated = if frame.repeat_header {
            frame.header.height()
        } else {
            0.0
        };
        let fresh_page_room = Self::printable_height() - repeated;
        if self.cursor_top + row.height() > Self::page_bottom() && row.height() <= fresh_page_room {
            self.continue_table_on_new_page(frame);
        }
        let mut from = 0;
        while from < row.lines {
            // Whole lines that fit under the cursor; a fresh page fits one.
            let fit = (1..=row.lines - from)
                .take_while(|&n| self.cursor_top + row_height(n) <= Self::page_bottom())
                .count();
            if fit == 0 {
                self.continue_table_on_new_page(frame);
                continue;
            }
            self.draw_row_part(frame, row, from..from + fit);
            from += fit;
            if from < row.lines {
                self.continue_table_on_new_page(frame);
            }
        }
    }

    /// Draws lines `range` of every cell of `row` as one bordered row.
    fn draw_row_part(&mut self, frame: &TableFrame<'_>, row: &RowLayout, range: Range<usize>) {
        let row_height = row_height(range.len());
        let row_top = self.occupy(row_height);
        let row_top_pdf_y = PAGE_HEIGHT_PT - row_top;
        let row_bottom_pdf_y = row_top_pdf_y - row_height;
        let space_width = text_width_pt(&self.fonts.sans_regular, " ", TABLE_FONT_SIZE);

        let mut x = frame.x0;
        for (i, &w) in frame.col_widths.iter().enumerate() {
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
            let align = frame.alignments.get(i).copied().unwrap_or(Align::None);
            let lines = row.cells.get(i).map_or(&[][..], Vec::as_slice);
            let shown = lines
                .get(range.start..range.end.min(lines.len()))
                .unwrap_or_default();
            for (li, line) in shown.iter().enumerate() {
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
                    Decoded::Raw(raw) => self.doc.add_image(&raw),
                    Decoded::Jpeg(jpeg) => self.doc.add_xobject(&jpeg),
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

    fn render_image(&mut self, image: &ImageBlock, x0: f32, max_width: f32) {
        let Some(placed) = self.place_image(&image.src) else {
            self.render_image_fallback(&image.alt, x0, max_width);
            return;
        };
        let title = image.title.as_str();
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
        let caption_height = if caption.is_empty() {
            0.0
        } else {
            CAPTION_GAP + caption.len() as f32 * CAPTION_LEADING
        };
        let printable_height = Self::printable_height();
        // Never upscale; keep the caption on the image's page.
        let scale = PT_PER_PX
            .min(max_width / placed.width_px)
            .min(((printable_height - caption_height) / placed.height_px).max(0.0));
        let (width, height) = (placed.width_px * scale, placed.height_px * scale);
        self.ensure_space(height + caption_height);
        let bottom = PAGE_HEIGHT_PT - (self.occupy(height) + height);
        self.ops.push(Op::UseXobject {
            id: placed.id,
            transform: XObjectTransform {
                translate_x: Some(Pt(x0)),
                translate_y: Some(Pt(bottom)),
                scale_x: Some(scale),
                scale_y: Some(scale),
                // 1 px = 1 pt before scaling.
                dpi: Some(72.0),
                ..Default::default()
            },
        });
        if let Some(url) = image.link.clone() {
            self.add_link(
                Rect::from_xywh(Pt(x0), Pt(bottom), Pt(width), Pt(height)),
                url,
            );
        }
        if !caption.is_empty() {
            self.cursor_top += CAPTION_GAP;
        }
        for line in &caption {
            let y = self.next_baseline(CAPTION_LEADING);
            self.render_line(line, x0, y, CAPTION_SIZE);
        }
        self.cursor_top += PARAGRAPH_SPACING_AFTER;
    }

    fn finish(mut self) -> (Vec<u8>, usize, Vec<String>) {
        // An empty document still gets one page; nothing else gets a blank one.
        if !self.ops.is_empty() || self.pages.is_empty() {
            self.pages.push(PdfPage::new(
                Mm::from(Pt(PAGE_WIDTH_PT)),
                Mm::from(Pt(PAGE_HEIGHT_PT)),
                std::mem::take(&mut self.ops),
            ));
        }
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
        Block::Image(ImageBlock {
            src: src.into(),
            alt: "the alt".into(),
            title: title.into(),
            link: None,
        })
    }

    fn link_rects(ops: &[Op]) -> Vec<(Rect, String)> {
        ops.iter()
            .filter_map(|op| match op {
                Op::LinkAnnotation { link } => match &link.actions {
                    Actions::Uri(uri) => Some((link.rect.clone(), uri.clone())),
                    Actions::Goto(_) => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn linked_image_gets_a_link_annotation_over_it() {
        let images = Images::from([("a.png".to_string(), png_bytes(200, 100))]);
        let block = Block::Image(ImageBlock {
            link: Some("http://l".into()),
            ..match image("a.png", "") {
                Block::Image(i) => i,
                _ => unreachable!(),
            }
        });
        let ops = render_with(&[block], &images).ops();
        let (x, y, _, _) = placements(&ops)[0];
        let links = link_rects(&ops);
        assert_eq!(links.len(), 1);
        let (rect, uri) = &links[0];
        assert_eq!(uri, "http://l");
        assert!((rect.x.0 - x).abs() < 0.01 && (rect.y.0 - y).abs() < 0.01);
        assert!((rect.width.0 - 150.0).abs() < 0.01 && (rect.height.0 - 75.0).abs() < 0.01);
    }

    #[test]
    fn full_page_image_with_caption_stays_inside_the_margins() {
        let images = Images::from([("t.png".to_string(), png_bytes(100, 3000))]);
        let ops = render_with(&[image("t.png", "Caption")], &images).ops();
        let caption_baseline = text_runs(&ops)[0].2;
        assert!(caption_baseline >= MARGIN_PT, "{caption_baseline}");
    }

    #[test]
    fn strike_through_a_link_uses_the_link_color() {
        let ops = rendered_ops(&[paragraph(vec![Span {
            text: "gone".into(),
            strike: true,
            link: Some("u".into()),
            ..Default::default()
        }])]);
        let strike_color = ops
            .iter()
            .find_map(|op| match op {
                Op::SetOutlineColor { col } => Some(col.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(strike_color, rgb(LINK_COLOR));
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

    fn table(header: &[&str], rows: &[Vec<String>]) -> Block {
        Block::Table {
            alignments: vec![Align::None; header.len()],
            header: header.iter().map(|h| vec![Span::plain(*h)]).collect(),
            rows: rows
                .iter()
                .map(|r| r.iter().map(|c| vec![Span::plain(c.as_str())]).collect())
                .collect(),
        }
    }

    /// Table cell rectangles of every page, page by page.
    fn cell_boxes(rendered: &Rendered) -> Vec<Vec<Rect>> {
        rendered
            .pages
            .iter()
            .map(|ops| rectangles(ops).into_iter().cloned().collect())
            .collect()
    }

    #[test]
    fn long_unbreakable_cell_keeps_its_word_whole_when_another_column_can_wrap() {
        let url = "https://example.com/a/rather/long/path/to/some/resource";
        let prose = "words that can wrap onto several lines without any trouble at all, ".repeat(4);
        let rows = vec![vec![url.to_string(), prose]];
        let ops = rendered_ops(&[table(&["Link", "Notes"], &rows)]);
        let texts: Vec<String> = text_runs(&ops).into_iter().map(|r| r.0).collect();
        assert!(texts.iter().any(|t| t == url), "URL was split: {texts:?}");
    }

    #[test]
    fn table_fits_the_printable_width() {
        let wide = "x".repeat(400);
        let rows = vec![vec![wide.clone(), wide.clone(), wide]];
        let rendered = render_with(&[table(&["a", "b", "c"], &rows)], &Images::new());
        for rect in cell_boxes(&rendered).concat() {
            assert!(rect.x.0 >= MARGIN_PT - 0.01);
            assert!(rect.x.0 + rect.width.0 <= PAGE_WIDTH_PT - MARGIN_PT + 0.01);
        }
    }

    #[test]
    fn mixed_style_cell_is_measured_per_span() {
        // One bold word must not make the whole cell be measured in bold.
        let fonts = FontSet::load().unwrap();
        let spans = vec![
            Span::plain("regular text "),
            Span {
                text: "b".into(),
                bold: true,
                ..Default::default()
            },
        ];
        let demand = table::cell_demand(&fonts, &spans, false);
        let expected = text_width_pt(&fonts.sans_regular, "regular text ", TABLE_FONT_SIZE)
            + text_width_pt(&fonts.sans_bold, "b", TABLE_FONT_SIZE)
            + 2.0 * TABLE_CELL_PADDING;
        assert!(
            (demand.max - expected).abs() < 0.01,
            "{} vs {expected}",
            demand.max
        );
        let longest = text_width_pt(&fonts.sans_regular, "regular", TABLE_FONT_SIZE);
        assert!((demand.min - (longest + 2.0 * TABLE_CELL_PADDING)).abs() < 0.01);
    }

    fn page_texts(rendered: &Rendered) -> Vec<Vec<String>> {
        rendered
            .pages
            .iter()
            .map(|ops| text_runs(ops).into_iter().map(|r| r.0).collect())
            .collect()
    }

    fn numbered_rows(count: usize) -> Vec<Vec<String>> {
        (1..=count)
            .map(|i| vec![format!("row{i}"), format!("value{i}")])
            .collect()
    }

    #[test]
    fn long_table_repeats_its_header_on_every_page() {
        let rendered = render_with(
            &[table(&["Name", "Value"], &numbered_rows(120))],
            &Images::new(),
        );
        let pages = page_texts(&rendered);
        assert!(pages.len() >= 3, "{} pages", pages.len());
        for (i, texts) in pages.iter().enumerate() {
            assert_eq!(texts[..2], ["Name", "Value"], "page {}", i + 1);
        }
        // Every row appears exactly once, in order.
        let rows: Vec<&String> = pages
            .iter()
            .flatten()
            .filter(|t| t.starts_with("row"))
            .collect();
        let expected: Vec<String> = (1..=120).map(|i| format!("row{i}")).collect();
        assert_eq!(rows, expected.iter().collect::<Vec<_>>());
    }

    #[test]
    fn long_table_starts_on_the_current_page_instead_of_leaving_it_blank() {
        let mut blocks = vec![paragraph(vec![Span::plain("intro")])];
        blocks.push(table(&["Name", "Value"], &numbered_rows(120)));
        let pages = page_texts(&render_with(&blocks, &Images::new()));
        assert_eq!(pages[0][..3], ["intro", "Name", "Value"]);
    }

    #[test]
    fn header_is_never_left_alone_at_the_bottom_of_a_page() {
        // 28 one-line paragraphs (22 pt each) leave 32 pt: room for the
        // header row (4 + 21 pt) but not for header + first row (46 pt).
        let rendered = {
            let mut blocks = filler(28);
            blocks.push(table(&["Name", "Value"], &numbered_rows(3)));
            render_with(&blocks, &Images::new())
        };
        let pages = page_texts(&rendered);
        assert_eq!(pages.len(), 2);
        assert!(!pages[0].contains(&"Name".to_string()), "{pages:?}");
        assert_eq!(pages[1][..3], ["Name", "Value", "row1"]);
    }

    #[test]
    fn row_taller_than_a_page_is_split_between_lines_on_every_page() {
        let cell: Vec<String> = (1..=120).map(|i| format!("line{i}")).collect();
        // Hard breaks in the cell, as the parser emits for `<br>`.
        let rows = vec![vec!["key".to_string(), cell.join("\n")]];
        let block = table(&["Name", "Value"], &rows);
        let rendered = render_with(&[block], &Images::new());
        let pages = page_texts(&rendered);
        assert!(pages.len() >= 2, "{} pages", pages.len());
        let lines: Vec<&String> = pages
            .iter()
            .flatten()
            .filter(|t| t.starts_with("line"))
            .collect();
        assert_eq!(lines.len(), 120, "every line drawn once");
        for (i, boxes) in cell_boxes(&rendered).iter().enumerate() {
            for rect in boxes {
                assert!(
                    rect.y.0 >= MARGIN_PT - 0.01,
                    "page {} box below margin",
                    i + 1
                );
                assert!(rect.y.0 + rect.height.0 <= PAGE_HEIGHT_PT - MARGIN_PT + 0.01);
            }
        }
        for texts in &pages {
            assert_eq!(texts[..2], ["Name", "Value"]);
        }
    }

    #[test]
    fn header_taller_than_a_page_is_split_and_not_repeated() {
        let huge: String = (1..=80)
            .map(|i| format!("h{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let block = Block::Table {
            alignments: vec![Align::None],
            header: vec![vec![Span::plain(huge)]],
            rows: vec![vec![vec![Span::plain("body")]]],
        };
        let pages = page_texts(&render_with(&[block], &Images::new()));
        let header_lines = pages
            .iter()
            .flatten()
            .filter(|t| t.starts_with('h'))
            .count();
        assert_eq!(header_lines, 80, "header drawn exactly once");
        assert!(pages.iter().flatten().any(|t| t == "body"));
    }

    fn heading(level: u8, text: &str) -> Block {
        Block::Heading {
            level,
            spans: vec![Span::plain(text)],
        }
    }

    fn filler(count: usize) -> Vec<Block> {
        (0..count)
            .map(|_| paragraph(vec![Span::plain("x")]))
            .collect()
    }

    #[test]
    fn heading_is_not_stranded_at_the_bottom_of_a_page() {
        // 27 one-line paragraphs leave 54 pt: enough for the level-2 heading
        // (12 + 25 pt) but not for the heading plus a line of its section.
        let mut blocks = filler(27);
        blocks.push(heading(2, "Section"));
        blocks.push(paragraph(vec![Span::plain("body")]));
        let pages = page_texts(&render_with(&blocks, &Images::new()));
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1], ["Section", "body"]);
    }

    #[test]
    fn spacing_before_a_block_is_dropped_at_the_top_of_a_page() {
        let ops = rendered_ops(&[heading(1, "Title")]);
        let baseline = text_runs(&ops)[0].2;
        let leading = HEADING_SIZES[0] * HEADING_LEADING_FACTOR;
        let expected = PAGE_HEIGHT_PT - MARGIN_PT - leading * BASELINE_RATIO;
        assert!(
            (baseline - expected).abs() < 0.01,
            "{baseline} vs {expected}"
        );
    }

    #[test]
    fn no_empty_page_is_emitted_after_a_page_break() {
        let images = Images::new();
        let mut renderer = Renderer::new(&images).unwrap();
        renderer.render_block(&paragraph(vec![Span::plain("x")]), MARGIN_PT);
        renderer.new_page();
        let (_, page_count, _) = renderer.finish();
        assert_eq!(page_count, 1);
    }

    fn filled_rects(ops: &[Op]) -> Vec<Rect> {
        ops.iter()
            .filter_map(|op| match op {
                Op::DrawRectangle { rectangle } if rectangle.mode == Some(PaintMode::Fill) => {
                    Some(rectangle.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn contains(rect: &Rect, x: f32, y: f32) -> bool {
        x >= rect.x.0
            && x <= rect.x.0 + rect.width.0
            && y >= rect.y.0
            && y <= rect.y.0 + rect.height.0
    }

    fn code(lines: usize) -> Block {
        Block::CodeBlock {
            text: (1..=lines)
                .map(|i| format!("code{i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    #[test]
    fn code_block_sits_on_a_background_behind_its_text() {
        let ops = rendered_ops(&[code(3)]);
        let backgrounds = filled_rects(&ops);
        assert_eq!(backgrounds.len(), 1);
        let first_text = ops
            .iter()
            .position(|op| matches!(op, Op::ShowText { .. }))
            .unwrap();
        let background = ops
            .iter()
            .position(|op| matches!(op, Op::DrawRectangle { .. }))
            .unwrap();
        assert!(background < first_text, "background drawn first");
        for (_, x, y) in text_runs(&ops) {
            assert!(contains(&backgrounds[0], x, y), "text at {x},{y} outside");
        }
    }

    #[test]
    fn code_block_across_pages_gets_one_background_per_page() {
        let rendered = render_with(&[code(80)], &Images::new());
        assert_eq!(rendered.pages.len(), 2);
        for ops in &rendered.pages {
            let backgrounds = filled_rects(ops);
            assert_eq!(backgrounds.len(), 1);
            let bg = &backgrounds[0];
            assert!(
                bg.y.0 >= MARGIN_PT - 0.01
                    && bg.y.0 + bg.height.0 <= PAGE_HEIGHT_PT - MARGIN_PT + 0.01
            );
            for (_, x, y) in text_runs(ops) {
                assert!(contains(bg, x, y));
            }
        }
    }

    #[test]
    fn code_block_pushed_to_the_next_page_leaves_no_background_behind() {
        let mut blocks = filler(29);
        blocks.push(code(2));
        let rendered = render_with(&blocks, &Images::new());
        assert_eq!(rendered.pages.len(), 2);
        assert!(filled_rects(&rendered.pages[0]).is_empty());
        assert_eq!(filled_rects(&rendered.pages[1]).len(), 1);
    }

    #[test]
    fn blockquote_has_a_bar_on_each_page_it_spans() {
        let quote = Block::BlockQuote(filler(40));
        let rendered = render_with(&[quote], &Images::new());
        assert_eq!(rendered.pages.len(), 2);
        for ops in &rendered.pages {
            let bars: Vec<&Line> = lines_drawn(ops)
                .into_iter()
                .filter(|l| (l.points[0].p.x.0 - l.points[1].p.x.0).abs() < 0.01)
                .collect();
            assert_eq!(bars.len(), 1);
            let text_x = text_runs(ops)[0].1;
            assert!(bars[0].points[0].p.x.0 < text_x);
            let (top, bottom) = (bars[0].points[0].p.y.0, bars[0].points[1].p.y.0);
            for (_, _, y) in text_runs(ops) {
                assert!(
                    y <= top && y >= bottom,
                    "text at {y} outside bar {bottom}..{top}"
                );
            }
        }
    }

    #[test]
    fn blockquote_bar_ends_at_its_last_line_whatever_the_last_block() {
        for last in [
            paragraph(vec![Span::plain("end")]),
            list(vec![(None, "end")]),
            heading(3, "end"),
        ] {
            let ops = rendered_ops(&[Block::BlockQuote(vec![last])]);
            let bar = lines_drawn(&ops)[0].clone();
            let baseline = text_runs(&ops).last().unwrap().2;
            let bottom = bar.points[1].p.y.0;
            assert!(bottom < baseline, "bar stops above the last baseline");
            assert!(
                baseline - bottom < BODY_SIZE,
                "bar runs past the last line: {bottom} vs {baseline}"
            );
        }
    }

    /// A paragraph of `count` lines `{prefix}1`, `{prefix}2`, … (hard breaks).
    fn lines_paragraph(prefix: &str, count: usize) -> Block {
        let text = (1..=count)
            .map(|i| format!("{prefix}{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        paragraph(vec![Span::plain(text)])
    }

    /// Lines of the `p…` paragraph on each page.
    fn p_lines_per_page(blocks: &[Block]) -> Vec<usize> {
        page_texts(&render_with(blocks, &Images::new()))
            .iter()
            .map(|texts| texts.iter().filter(|t| t.starts_with('p')).count())
            .collect()
    }

    #[test]
    fn paragraph_does_not_leave_a_single_line_at_the_bottom_of_a_page() {
        // 44 filler lines (+ 8 pt) leave 24 pt: room for one 14 pt line.
        let blocks = [lines_paragraph("f", 44), lines_paragraph("p", 5)];
        assert_eq!(p_lines_per_page(&blocks), [0, 5]);
    }

    #[test]
    fn paragraph_does_not_leave_a_single_line_at_the_top_of_a_page() {
        // 41 filler lines leave 66 pt: room for 4 of the 5 lines.
        let blocks = [lines_paragraph("f", 41), lines_paragraph("p", 5)];
        assert_eq!(p_lines_per_page(&blocks), [3, 2]);
    }

    #[test]
    fn short_paragraphs_break_wherever_they_must() {
        let blocks = [lines_paragraph("f", 44), lines_paragraph("p", 3)];
        assert_eq!(p_lines_per_page(&blocks), [1, 2]);
    }

    fn item(blocks: Vec<Block>) -> Block {
        Block::List {
            ordered: false,
            start: 1,
            items: vec![ListItem { task: None, blocks }],
        }
    }

    #[test]
    fn marker_sits_on_the_first_line_of_a_leading_code_block() {
        let ops = rendered_ops(&[item(vec![code(2)])]);
        let runs = text_runs(&ops);
        let bullet = runs.iter().find(|r| r.0 == "\u{2022}").unwrap();
        let first_code = runs.iter().find(|r| r.0 == "code1").unwrap();
        assert!(
            (bullet.2 - first_code.2).abs() < 3.0,
            "{bullet:?} vs {first_code:?}"
        );
    }

    #[test]
    fn marker_follows_a_leading_image_to_the_next_page() {
        let images = Images::from([("a.png".to_string(), png_bytes(200, 200))]);
        let mut blocks = filler(25);
        blocks.push(item(vec![image("a.png", "")]));
        let rendered = render_with(&blocks, &images);
        assert_eq!(rendered.pages.len(), 2);
        let marker_page = rendered
            .pages
            .iter()
            .position(|ops| text_runs(ops).iter().any(|r| r.0 == "\u{2022}"))
            .unwrap();
        let image_page = rendered
            .pages
            .iter()
            .position(|ops| !placements(ops).is_empty())
            .unwrap();
        assert_eq!(marker_page, image_page);
    }

    #[test]
    fn every_list_item_keeps_its_marker_whatever_it_starts_with() {
        let leads = [
            Block::ThematicBreak,
            code(1),
            table(&["h"], &[vec!["c".to_string()]]),
            Block::BlockQuote(Vec::new()),
        ];
        for lead in leads {
            let ops = rendered_ops(&[item(vec![lead.clone()])]);
            let bullets = text_runs(&ops).iter().filter(|r| r.0 == "\u{2022}").count();
            assert_eq!(bullets, 1, "{lead:?}");
        }
    }

    #[test]
    fn nested_block_led_items_put_both_markers_on_the_first_line() {
        let ops = rendered_ops(&[item(vec![item(vec![code(1)])])]);
        let runs = text_runs(&ops);
        let bullets: Vec<f32> = runs
            .iter()
            .filter(|r| r.0 == "\u{2022}")
            .map(|r| r.2)
            .collect();
        assert_eq!(bullets.len(), 2);
        assert!((bullets[0] - bullets[1]).abs() < 0.01, "{bullets:?}");
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
