//! Structure pass: reading-ordered items → Markdown blocks.
//!
//! Headings come from font sizes above the body size (the most common size),
//! list items from bullet/number markers, code blocks from runs of monospace
//! lines, and paragraphs from vertical gaps and short closing lines. Tables,
//! images and rules close the open block and stand on their own.

use std::collections::HashMap;
use std::rc::Rc;

use crate::font::FontStyle;
use crate::interp::Glyph;
use crate::layout::{Item, Line, dominant_size, half_points};
use crate::tables::{Align, Cell, Table};

/// Smallest size ratio over the body size that counts as a heading.
const HEADING_RATIO: f32 = 1.1;
const MAX_HEADING_LINES: usize = 3;
const MAX_HEADING_CHARS: usize = 200;
/// Gap between glyphs (in font sizes) read as a word space.
const WORD_GAP: f32 = 0.15;
/// Baseline distance over the expected leading that starts a new paragraph.
const PARAGRAPH_GAP: f32 = 1.35;
const BULLETS: &[char] = &[
    '•', '◦', '▪', '▫', '■', '□', '●', '○', '‣', '⁃', '–', '—', '-', '*', '·', '∙', '►', '▶', '❖',
];
const CLOSING_PUNCTUATION: &[char] = &[',', '.', ')'];
const SENTENCE_END: &[char] = &['.', '!', '?', ':', ';', '"', '”', '»', ')'];

/// A run of text sharing one style and link target.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: FontStyle,
    pub link: Option<Rc<str>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading {
        level: usize,
        runs: Vec<Run>,
    },
    Paragraph(Vec<Run>),
    Code(String),
    ListItem {
        /// Ordered list number, `None` for bullets.
        number: Option<u64>,
        /// x of the marker from the column's left edge, used to derive nesting levels.
        marker_x: f32,
        runs: Vec<Run>,
    },
    Comment(String),
    Image {
        src: String,
        alt: String,
    },
    Table {
        align: Vec<Align>,
        /// `rows[0]` is the header; a `"\n"` run is a line break in a cell.
        rows: Vec<Vec<Vec<Run>>>,
    },
    Rule,
}

/// Items from one page, or a marker for a page whose content failed to parse.
#[derive(Debug)]
pub enum PageItems {
    Items(Vec<Item>),
    Failed(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Body,
    Code,
    Heading(usize),
    Item { number: Option<u64>, marker_x: f32 },
}

/// What the block being built needs to know to accept or refuse a line.
struct Open {
    kind: Kind,
    lines: Vec<Line>,
    /// x where list item content starts (continuation lines align there).
    content_x: f32,
}

struct Metrics {
    body_size: f32,
    /// Typical baseline distance per unit of font size.
    leading_ratio: f32,
    heading_levels: HashMap<i32, usize>,
}

/// Builds blocks; `image` turns an image item (page, index) into its block.
pub fn build(pages: Vec<PageItems>, image: &mut dyn FnMut(u32, usize) -> Block) -> Vec<Block> {
    let all: Vec<&Line> = pages
        .iter()
        .filter_map(|p| match p {
            PageItems::Items(items) => Some(items),
            PageItems::Failed(_) => None,
        })
        .flatten()
        .filter_map(|item| match item {
            Item::Line(line) => Some(line),
            _ => None,
        })
        .collect();
    let metrics = metrics(&all);

    let mut blocks = Vec::new();
    let mut open: Option<Open> = None;
    for page in pages {
        match page {
            PageItems::Failed(number) => {
                close(open.take(), &mut blocks);
                blocks.push(Block::Comment(format!("page {number}: extraction failed")));
            }
            PageItems::Items(items) => {
                for item in items {
                    let block = match item {
                        Item::Line(line) => {
                            open = Some(push_line(open.take(), line, &metrics, &mut blocks));
                            continue;
                        }
                        Item::Image { page, index } => image(page, index),
                        Item::Table { table, .. } => table_block(table),
                        Item::Rule { .. } => Block::Rule,
                    };
                    close(open.take(), &mut blocks);
                    blocks.push(block);
                }
            }
        }
    }
    close(open, &mut blocks);
    blocks
}

fn is_mono(line: &Line) -> bool {
    let total = line.glyphs.len();
    let mono = line.glyphs.iter().filter(|g| g.style.mono).count();
    total > 0 && mono * 10 >= total * 9
}

fn line_text(line: &Line) -> String {
    line.glyphs.iter().map(|g| g.text.as_str()).collect()
}

fn metrics(lines: &[&Line]) -> Metrics {
    let glyphs: Vec<Glyph> = lines
        .iter()
        .filter(|l| !is_mono(l))
        .flat_map(|l| l.glyphs.iter().cloned())
        .collect();
    let body_size = match dominant_size(&glyphs) {
        s if s > 0.0 => s,
        _ => 10.0,
    };

    let mut ratios: Vec<f32> = lines
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] if a.page == b.page && a.region == b.region => {
                let same_size =
                    (a.size - body_size).abs() < 0.5 && (b.size - body_size).abs() < 0.5;
                let delta = a.y - b.y;
                (same_size && delta > 0.9 * body_size && delta < 2.5 * body_size)
                    .then_some(delta / body_size)
            }
            _ => None,
        })
        .collect();
    ratios.sort_by(f32::total_cmp);
    // Lower quartile: within-paragraph spacing, not the larger paragraph gaps.
    let leading_ratio = ratios
        .get(ratios.len() / 4)
        .copied()
        .unwrap_or(1.2)
        .max(1.0);

    let mut sizes: Vec<i32> = lines
        .iter()
        .filter(|l| !is_mono(l) && l.size >= body_size * HEADING_RATIO)
        .map(|l| half_points(l.size))
        .collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes.dedup();
    let heading_levels = sizes
        .into_iter()
        .enumerate()
        .map(|(i, size)| (size, (i + 1).min(6)))
        .collect();

    Metrics {
        body_size,
        leading_ratio,
        heading_levels,
    }
}

/// Recognises a list marker at the start of a line: returns the marker's
/// number (`None` for bullets), its x, and the index of the first content glyph.
fn list_marker(line: &Line) -> Option<(Option<u64>, f32, usize)> {
    let glyphs = &line.glyphs;
    let first = glyphs.first()?;
    let followed_by_gap = |i: usize| match (glyphs.get(i), glyphs.get(i + 1)) {
        (Some(a), Some(b)) => b.x - (a.x + a.width) > WORD_GAP * a.size,
        _ => false,
    };

    let mut chars = first.text.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && BULLETS.contains(&c)
        && followed_by_gap(0)
    {
        return Some((None, first.x, 1));
    }

    let digits = glyphs
        .iter()
        .take_while(|g| g.text.chars().all(|c| c.is_ascii_digit()))
        .count();
    if (1..=3).contains(&digits)
        && glyphs
            .get(digits)
            .is_some_and(|g| g.text == "." || g.text == ")")
        && followed_by_gap(digits)
    {
        let number: String = glyphs
            .iter()
            .take(digits)
            .map(|g| g.text.as_str())
            .collect();
        return Some((number.parse().ok(), first.x, digits + 1));
    }
    None
}

fn classify(line: &Line, metrics: &Metrics) -> Kind {
    if is_mono(line) {
        return Kind::Code;
    }
    if let Some((number, marker_x, _)) = list_marker(line) {
        return Kind::Item {
            number,
            marker_x: marker_x - column_x0(line),
        };
    }
    if line.size >= metrics.body_size * HEADING_RATIO
        && let Some(level) = metrics.heading_levels.get(&half_points(line.size))
    {
        return Kind::Heading(*level);
    }
    Kind::Body
}

/// Left edge of the text column the line belongs to.
fn column_x0(line: &Line) -> f32 {
    line.column_x1 - line.column_width
}

fn ends_sentence(line: &Line) -> bool {
    line_text(line).trim_end().ends_with(SENTENCE_END)
}

fn push_line(open: Option<Open>, line: Line, metrics: &Metrics, blocks: &mut Vec<Block>) -> Open {
    let kind = classify(&line, metrics);
    let Some(mut current) = open else {
        return start(kind, line);
    };
    let Some(prev) = current.lines.last() else {
        return start(kind, line);
    };

    // A jump in reading order (new page, column or region) has no meaningful gap.
    let adjacent = prev.page == line.page && prev.region == line.region && prev.y > line.y;
    let gap = prev.y - line.y;
    let expected = metrics.leading_ratio * line.size.max(prev.size);
    let tight = adjacent && gap <= PARAGRAPH_GAP * expected;
    let flows = if adjacent {
        tight
    } else {
        !ends_sentence(prev)
    };
    let same_size = (prev.size - line.size).abs() < 0.6;
    let short_prev = prev.x1 < prev.column_x1 - 0.3 * prev.column_width;

    let continues = match (current.kind, kind) {
        (Kind::Code, Kind::Code) => !adjacent || gap <= 4.0 * expected,
        (Kind::Heading(a), Kind::Heading(b)) => {
            a == b && tight && current.lines.len() < MAX_HEADING_LINES
        }
        (Kind::Body, Kind::Body) => same_size && flows && !(short_prev && ends_sentence(prev)),
        // A monospace line wrapped inside a running paragraph (e.g. a long URL)
        // is inline code, not a code block.
        (Kind::Body, Kind::Code) => tight && !short_prev,
        (Kind::Item { marker_x, .. }, Kind::Body) => {
            let hanging = (line.x0 - current.content_x).abs() < 0.6 * line.size;
            // Numbered paragraphs without a hanging indent wrap back to the marker.
            let flush =
                (line.x0 - column_x0(&line) - marker_x).abs() < 0.6 * line.size && !short_prev;
            // Across a column or page jump the x positions are not comparable.
            same_size && flows && (hanging || flush || !adjacent)
        }
        _ => false,
    };

    if continues {
        current.lines.push(line);
        current
    } else {
        close(Some(current), blocks);
        start(kind, line)
    }
}

fn start(kind: Kind, line: Line) -> Open {
    let content_x = match kind {
        Kind::Item { .. } => list_marker(&line)
            .and_then(|(_, _, i)| line.glyphs.get(i))
            .map_or(line.x0, |g| g.x),
        _ => line.x0,
    };
    Open {
        kind,
        lines: vec![line],
        content_x,
    }
}

fn close(open: Option<Open>, blocks: &mut Vec<Block>) {
    let Some(Open { kind, lines, .. }) = open else {
        return;
    };
    let block = match kind {
        Kind::Code => Block::Code(code_text(&lines)),
        Kind::Heading(level) => {
            let runs = join_lines(&lines, 0);
            let chars: usize = runs.iter().map(|r| r.text.chars().count()).sum();
            if chars <= MAX_HEADING_CHARS {
                Block::Heading { level, runs }
            } else {
                Block::Paragraph(runs)
            }
        }
        Kind::Item { number, marker_x } => {
            let skip = lines.first().and_then(list_marker).map_or(0, |(_, _, i)| i);
            Block::ListItem {
                number,
                marker_x,
                runs: join_lines(&lines, skip),
            }
        }
        Kind::Body => Block::Paragraph(join_lines(&lines, 0)),
    };
    blocks.push(block);
}

/// Inline runs for a line, inserting spaces at word gaps.
fn line_runs(glyphs: &[Glyph]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut prev: Option<&Glyph> = None;
    for glyph in glyphs {
        if let Some(p) = prev {
            let gap = glyph.x - (p.x + p.width);
            // Some writers draw every word separately, leaving a visual space
            // before trailing punctuation ("bold ,"); never meaningful here.
            let closes = glyph.text.starts_with(CLOSING_PUNCTUATION);
            if gap > WORD_GAP * p.size.max(glyph.size) && !closes {
                let same = p.style == glyph.style && p.link == glyph.link;
                let (style, link) = if same {
                    (p.style, p.link.clone())
                } else {
                    (FontStyle::default(), None)
                };
                push_text(&mut runs, " ", style, link);
            }
        }
        push_text(&mut runs, &glyph.text, glyph.style, glyph.link.clone());
        prev = Some(glyph);
    }
    runs
}

fn push_text(runs: &mut Vec<Run>, text: &str, style: FontStyle, link: Option<Rc<str>>) {
    match runs.last_mut() {
        Some(last) if last.style == style && last.link == link => last.text.push_str(text),
        _ => runs.push(Run {
            text: text.to_string(),
            style,
            link,
        }),
    }
}

/// Joins wrapped lines into one run list, undoing end-of-line hyphenation.
fn join_lines(lines: &[Line], skip_first: usize) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let glyphs = if i == 0 {
            line.glyphs.get(skip_first..).unwrap_or_default()
        } else {
            line.glyphs.as_slice()
        };
        let next = line_runs(glyphs);
        if let (Some(last), Some(first)) = (runs.last_mut(), next.first()) {
            let hyphenated = {
                let mut tail = last.text.chars().rev();
                tail.next() == Some('-') && tail.next().is_some_and(char::is_alphabetic)
            };
            let continues_word = first.text.chars().next().is_some_and(char::is_lowercase);
            if hyphenated && continues_word {
                last.text.pop();
            } else {
                let same = last.style == first.style && last.link == first.link;
                if same {
                    last.text.push(' ');
                } else {
                    runs.push(Run {
                        text: " ".to_string(),
                        style: FontStyle::default(),
                        link: None,
                    });
                }
            }
        }
        for run in next {
            push_text(&mut runs, &run.text, run.style, run.link);
        }
    }
    runs
}

fn table_block(table: Table) -> Block {
    Block::Table {
        align: table.align,
        rows: table
            .rows
            .into_iter()
            .map(|row| row.into_iter().map(|cell| cell_runs(&cell)).collect())
            .collect(),
    }
}

/// A cell's runs, its lines separated by `"\n"` runs (`<br>` in GFM) except
/// where a hyphenated word continues on the next line.
fn cell_runs(cell: &Cell) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for line in cell {
        let next = line_runs(line);
        if let (Some(last), Some(first)) = (runs.last_mut(), next.first()) {
            let hyphenated = {
                let mut tail = last.text.chars().rev();
                tail.next() == Some('-') && tail.next().is_some_and(char::is_alphabetic)
            };
            let continues_word = first.text.chars().next().is_some_and(char::is_lowercase);
            if hyphenated && continues_word {
                last.text.pop();
            } else {
                runs.push(Run {
                    text: "\n".to_string(),
                    style: FontStyle::default(),
                    link: None,
                });
            }
        }
        for run in next {
            push_text(&mut runs, &run.text, run.style, run.link);
        }
    }
    runs
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "space counts are small non-negative numbers"
)]
fn spaces(distance: f32, char_width: f32) -> usize {
    (distance / char_width).round().max(0.0) as usize
}

/// Rebuilds a code block's text, recovering indentation and inner spacing
/// from glyph positions (monospace: one space per character advance).
fn code_text(lines: &[Line]) -> String {
    let mut widths: Vec<f32> = lines
        .iter()
        .flat_map(|l| l.glyphs.iter().map(|g| g.width))
        .filter(|w| *w > 0.0)
        .collect();
    widths.sort_by(f32::total_cmp);
    let char_width = widths.get(widths.len() / 2).copied().unwrap_or(6.0);
    let left = lines.iter().map(|l| l.x0).fold(f32::MAX, f32::min);
    let leading = lines
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] if a.page == b.page && a.y > b.y => Some(a.y - b.y),
            _ => None,
        })
        .fold(f32::MAX, f32::min);

    let mut out = String::new();
    let mut prev: Option<&Line> = None;
    for line in lines {
        if let Some(p) = prev {
            out.push('\n');
            if p.page == line.page && p.y > line.y && leading < f32::MAX {
                let blank = spaces(p.y - line.y, leading).saturating_sub(1);
                out.push_str(&"\n".repeat(blank.min(3)));
            }
        }
        out.push_str(&" ".repeat(spaces(line.x0 - left, char_width)));
        let mut cursor: Option<&Glyph> = None;
        for glyph in &line.glyphs {
            if let Some(c) = cursor {
                let gap = glyph.x - (c.x + c.width);
                out.push_str(&" ".repeat(spaces(gap, char_width)));
            }
            out.push_str(&glyph.text);
            cursor = Some(glyph);
        }
        prev = Some(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Column;

    fn glyphs(text: &str, x: f32, size: f32, style: FontStyle) -> Vec<Glyph> {
        let width = size * 0.5;
        let mut out = Vec::new();
        let mut cursor = x;
        for c in text.chars() {
            if c != ' ' {
                out.push(Glyph {
                    text: c.to_string(),
                    x: cursor,
                    y: 0.0,
                    width,
                    size,
                    style,
                    link: None,
                    synthetic: false,
                });
            }
            cursor += width;
        }
        out
    }

    fn line(text: &str, x: f32, y: f32, size: f32, style: FontStyle) -> Line {
        let mut glyphs = glyphs(text, x, size, style);
        for g in &mut glyphs {
            g.y = y;
        }
        let x0 = glyphs.first().map_or(x, |g| g.x);
        let x1 = glyphs.last().map_or(x, |g| g.x + g.width);
        Line {
            page: 1,
            column: Column::Full,
            region: 0,
            glyphs,
            x0,
            x1,
            y,
            size,
            column_x1: 540.0,
            column_width: 468.0,
        }
    }

    fn body(text: &str, y: f32) -> Line {
        line(text, 72.0, y, 10.0, FontStyle::default())
    }

    fn text(runs: &[Run]) -> String {
        runs.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn larger_text_becomes_a_heading_and_body_lines_merge() {
        let lines = vec![
            line("Title", 72.0, 700.0, 20.0, FontStyle::default()),
            body("first line of a paragraph that keeps going", 670.0),
            body("and wraps here.", 658.0),
            body("Second paragraph after a gap.", 630.0),
        ];
        let blocks = build(
            vec![PageItems::Items(
                lines.into_iter().map(Item::Line).collect(),
            )],
            &mut |_, _| Block::Rule,
        );
        assert!(matches!(&blocks[0], Block::Heading { level: 1, runs } if text(runs) == "Title"));
        assert!(
            matches!(&blocks[1], Block::Paragraph(r) if text(r) == "first line of a paragraph that keeps going and wraps here.")
        );
        assert!(
            matches!(&blocks[2], Block::Paragraph(r) if text(r) == "Second paragraph after a gap.")
        );
    }

    #[test]
    fn hyphenated_line_ends_are_rejoined() {
        let lines = vec![body("an exam-", 700.0), body("ple of it", 688.0)];
        let blocks = build(
            vec![PageItems::Items(
                lines.into_iter().map(Item::Line).collect(),
            )],
            &mut |_, _| Block::Rule,
        );
        assert!(matches!(&blocks[0], Block::Paragraph(r) if text(r) == "an example of it"));
    }

    #[test]
    fn bullets_and_numbers_start_list_items() {
        let lines = vec![
            body("• one", 700.0),
            line("continued", 82.0, 688.0, 10.0, FontStyle::default()),
            body("2. two", 676.0),
        ];
        let blocks = build(
            vec![PageItems::Items(
                lines.into_iter().map(Item::Line).collect(),
            )],
            &mut |_, _| Block::Rule,
        );
        assert!(
            matches!(&blocks[0], Block::ListItem { number: None, runs, .. } if text(runs) == "one continued")
        );
        assert!(
            matches!(&blocks[1], Block::ListItem { number: Some(2), runs, .. } if text(runs) == "two")
        );
    }

    #[test]
    fn numbered_paragraph_wrapping_to_the_margin_stays_one_item() {
        let full =
            "1. a numbered paragraph whose first line runs to the right margin of the column";
        let lines = vec![body(full, 700.0), body("and wraps flush left.", 688.0)];
        let blocks = build(
            vec![PageItems::Items(
                lines.into_iter().map(Item::Line).collect(),
            )],
            &mut |_, _| Block::Rule,
        );
        assert_eq!(blocks.len(), 1);
        assert!(
            matches!(&blocks[0], Block::ListItem { runs, .. } if text(runs).ends_with("flush left."))
        );
    }

    #[test]
    fn monospace_lines_form_a_code_block_with_indentation() {
        let mono = FontStyle {
            mono: true,
            ..FontStyle::default()
        };
        let lines = vec![
            body("Some text here to set the body size.", 720.0),
            line("fn main() {", 72.0, 700.0, 10.0, mono),
            line("    call();", 72.0, 688.0, 10.0, mono),
            line("}", 72.0, 676.0, 10.0, mono),
        ];
        let blocks = build(
            vec![PageItems::Items(
                lines.into_iter().map(Item::Line).collect(),
            )],
            &mut |_, _| Block::Rule,
        );
        assert_eq!(
            blocks[1],
            Block::Code("fn main() {\n    call();\n}".to_string())
        );
    }

    #[test]
    fn monospace_line_wrapped_inside_a_paragraph_stays_inline() {
        let mono = FontStyle {
            mono: true,
            ..FontStyle::default()
        };
        let long = "a paragraph line that runs all the way to the right margin of the page column";
        let lines = vec![
            line(long, 72.0, 700.0, 10.0, FontStyle::default()),
            line("example.com/path", 72.0, 688.0, 10.0, mono),
        ];
        let blocks = build(
            vec![PageItems::Items(
                lines.into_iter().map(Item::Line).collect(),
            )],
            &mut |_, _| Block::Rule,
        );
        assert_eq!(blocks.len(), 1);
        assert!(
            matches!(&blocks[0], Block::Paragraph(runs) if runs.last().is_some_and(|r| r.style.mono))
        );
    }

    #[test]
    fn failed_pages_leave_a_comment() {
        let blocks = build(vec![PageItems::Failed(2)], &mut |_, _| Block::Rule);
        assert_eq!(
            blocks,
            [Block::Comment("page 2: extraction failed".to_string())]
        );
    }
}
