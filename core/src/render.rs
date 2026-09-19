//! Hand-rolled layout engine on top of printpdf's low-level drawing ops.
//!
//! printpdf 0.12 (built with `default-features = false`) has no built-in text
//! wrapping, pagination, lists or tables — this module supplies all of that:
//! word-wrap by measured glyph advances, a top-down page cursor, and simple
//! block renderers for headings/paragraphs/lists/code/tables/rules.

use printpdf::{
    Actions, BorderArray, Color, FontId, HighlightingMode, Line, LinePoint, LinkAnnotation, Mm, Op,
    PaintMode, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt, Rect, Rgb, TextItem,
    TextMatrix,
};

use crate::error::Md2PdfError;
use crate::fonts::{FontSet, text_width_pt};
use crate::ir::{Align, Block, Span};

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
const LINK_COLOR: (f32, f32, f32) = (0.06, 0.32, 0.78);
const BLACK: (f32, f32, f32) = (0.0, 0.0, 0.0);
const RULE_GREY: (f32, f32, f32) = (0.7, 0.7, 0.7);
const BORDER_GREY: (f32, f32, f32) = (0.6, 0.6, 0.6);

#[derive(Clone)]
struct Word {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    link: Option<String>,
    width: f32,
}

enum Token {
    Word(Word),
    Break,
}

fn tokenize(spans: &[Span], fonts: &FontSet, size: f32, force_bold: bool) -> Vec<Token> {
    let mut tokens = Vec::new();
    for span in spans {
        let bold = force_bold || span.bold;
        let font = fonts.resolve(bold, span.italic, span.code);
        for (i, segment) in span.text.split('\n').enumerate() {
            if i > 0 {
                tokens.push(Token::Break);
            }
            for word in segment.split_whitespace() {
                let width = text_width_pt(font, word, size);
                tokens.push(Token::Word(Word {
                    text: word.to_string(),
                    bold,
                    italic: span.italic,
                    code: span.code,
                    link: span.link.clone(),
                    width,
                }));
            }
        }
    }
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
                let font = fonts.resolve(w.bold, w.italic, w.code);
                for chunk in hard_wrap_chars(font, &w.text, size, max_width) {
                    let width = text_width_pt(font, &chunk, size);
                    out.push(Token::Word(Word {
                        text: chunk,
                        bold: w.bold,
                        italic: w.italic,
                        code: w.code,
                        link: w.link.clone(),
                        width,
                    }));
                }
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
        if w > widths[i] {
            widths[i] = w;
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

struct Renderer {
    doc: PdfDocument,
    fonts: FontSet,
    font_ids: FontIds,
    pages: Vec<PdfPage>,
    ops: Vec<Op>,
    /// Distance from the top of the page to the next free line box.
    cursor_top: f32,
}

impl Renderer {
    fn new() -> Result<Self, Md2PdfError> {
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

    fn draw_text(
        &mut self,
        x: f32,
        y: f32,
        text: &str,
        bold: bool,
        italic: bool,
        code: bool,
        size: f32,
        color: (f32, f32, f32),
    ) {
        if text.is_empty() {
            return;
        }
        let font_id = self.font_id_for(bold, italic, code);
        let (r, g, b) = color;
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetFillColor {
            col: Color::Rgb(Rgb {
                r,
                g,
                b,
                icc_profile: None,
            }),
        });
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

    fn render_line(&mut self, line: &[Word], x0: f32, y: f32, size: f32) {
        let space_width = text_width_pt(&self.fonts.sans_regular, " ", size);
        let mut x = x0;
        for word in line {
            let color = if word.link.is_some() {
                LINK_COLOR
            } else {
                BLACK
            };
            self.draw_text(
                x,
                y,
                &word.text,
                word.bold,
                word.italic,
                word.code,
                size,
                color,
            );
            if let Some(url) = word.link.clone() {
                self.add_link_annotation(x, y, word.width, size, url);
            }
            x += word.width + space_width;
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

    fn render_heading(&mut self, level: u8, spans: &[Span], x0: f32, max_width: f32) {
        let idx = (level as usize).saturating_sub(1).min(5);
        let size = HEADING_SIZES[idx];
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
                self.draw_text(x0, y, &wrapped, false, false, true, CODE_SIZE, BLACK);
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

    fn reserve_list_line_baseline(&mut self) -> f32 {
        self.ensure_space(BODY_LEADING);
        self.next_baseline(BODY_LEADING)
    }

    fn draw_list_marker(&mut self, indent: f32, y: f32, marker: &str) {
        self.draw_text(indent, y, marker, false, false, false, BODY_SIZE, BLACK);
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
        marker: &str,
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
        marker: &str,
    ) {
        let y = self.reserve_list_line_baseline();
        self.draw_list_marker(indent, y, marker);
        self.render_block(first, content_x);
        self.render_list_item_blocks(rest, content_x);
    }

    fn render_list_item_marker_only(&mut self, indent: f32, marker: &str) {
        let y = self.reserve_list_line_baseline();
        self.draw_list_marker(indent, y, marker);
    }

    fn render_list_item(
        &mut self,
        item_blocks: &[Block],
        indent: f32,
        content_x: f32,
        max_width: f32,
        marker: &str,
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

    fn render_list(&mut self, ordered: bool, start: u64, items: &[Vec<Block>], indent: f32) {
        let content_x = indent + LIST_INDENT;
        let max_width = (PAGE_WIDTH_PT - MARGIN_PT - content_x).max(1.0);

        for (i, item_blocks) in items.iter().enumerate() {
            let marker = Self::list_marker(ordered, start, i);
            self.render_list_item(item_blocks, indent, content_x, max_width, &marker);
        }
    }

    fn render_rule(&mut self, x0: f32, max_width: f32) {
        self.ensure_space(BODY_LEADING);
        self.cursor_top += BODY_LEADING * 0.5;
        let y = PAGE_HEIGHT_PT - self.cursor_top;
        let (r, g, b) = RULE_GREY;
        self.ops.push(Op::SaveGraphicsState);
        self.ops.push(Op::SetOutlineColor {
            col: Color::Rgb(Rgb {
                r,
                g,
                b,
                icc_profile: None,
            }),
        });
        self.ops.push(Op::SetOutlineThickness { pt: Pt(1.0) });
        self.ops.push(Op::DrawLine {
            line: Line {
                points: vec![
                    LinePoint {
                        p: Point {
                            x: Pt(x0),
                            y: Pt(y),
                        },
                        bezier: false,
                    },
                    LinePoint {
                        p: Point {
                            x: Pt(x0 + max_width),
                            y: Pt(y),
                        },
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
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
            .max(rows.iter().map(|r| r.len()).max().unwrap_or(0))
            .max(1);
        let available_width = (PAGE_WIDTH_PT - MARGIN_PT - x0).max(1.0);

        let mut col_widths = natural_column_widths(&self.fonts, num_cols, header, rows);
        let total: f32 = col_widths.iter().sum();
        if total > available_width {
            let scale = available_width / total;
            for w in col_widths.iter_mut() {
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
            let (r, g, b) = BORDER_GREY;
            self.ops.push(Op::SaveGraphicsState);
            self.ops.push(Op::SetOutlineColor {
                col: Color::Rgb(Rgb {
                    r,
                    g,
                    b,
                    icc_profile: None,
                }),
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

    fn finish(mut self) -> (Vec<u8>, usize) {
        self.pages.push(PdfPage::new(
            Mm::from(Pt(PAGE_WIDTH_PT)),
            Mm::from(Pt(PAGE_HEIGHT_PT)),
            std::mem::take(&mut self.ops),
        ));
        let page_count = self.pages.len();
        self.doc.pages = self.pages;
        let mut warnings = Vec::new();
        let bytes = self.doc.save(&PdfSaveOptions::default(), &mut warnings);
        (bytes, page_count)
    }
}

/// Returns the generated PDF bytes and the number of pages produced.
pub fn render(blocks: &[Block]) -> Result<(Vec<u8>, usize), Md2PdfError> {
    let mut renderer = Renderer::new()?;
    for block in blocks {
        renderer.render_block(block, MARGIN_PT);
    }
    Ok(renderer.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, width: f32) -> Word {
        Word {
            text: text.to_string(),
            bold: false,
            italic: false,
            code: false,
            link: None,
            width,
        }
    }

    #[test]
    fn wrap_lines_breaks_once_max_width_is_exceeded() {
        let tokens = vec![
            Token::Word(word("aaa", 30.0)),
            Token::Word(word("bbb", 30.0)),
            Token::Word(word("ccc", 30.0)),
        ];
        let lines = wrap_lines(tokens, 65.0, 5.0);
        let texts: Vec<Vec<&str>> = lines
            .iter()
            .map(|l| l.iter().map(|w| w.text.as_str()).collect())
            .collect();
        assert_eq!(texts, vec![vec!["aaa", "bbb"], vec!["ccc"]]);
    }

    #[test]
    fn wrap_lines_starts_a_new_line_on_an_explicit_break() {
        let tokens = vec![
            Token::Word(word("aaa", 10.0)),
            Token::Break,
            Token::Word(word("bbb", 10.0)),
        ];
        let lines = wrap_lines(tokens, 1000.0, 5.0);
        let texts: Vec<Vec<&str>> = lines
            .iter()
            .map(|l| l.iter().map(|w| w.text.as_str()).collect())
            .collect();
        assert_eq!(texts, vec![vec!["aaa"], vec!["bbb"]]);
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
        let renderer = Renderer::new().unwrap();
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
        let (bytes, page_count) = render(&blocks).unwrap();
        assert_eq!(page_count, 1);
        assert!(bytes.starts_with(b"%PDF"));
    }
}
