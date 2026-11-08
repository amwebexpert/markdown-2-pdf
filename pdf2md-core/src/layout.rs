//! Geometry pass: glyphs → line segments → reading-ordered lines.
//!
//! Glyphs are clustered into baselines, baselines split into segments at wide
//! horizontal gaps, repeated page furniture (running headers/footers, page
//! numbers) dropped, and two-column pages linearised column by column.

use std::collections::HashMap;

use crate::interp::Glyph;
use crate::page::Page;

/// Baseline clustering tolerance, as a fraction of the font size.
const BASELINE_TOLERANCE: f32 = 0.45;
/// Horizontal gap (in font sizes) that splits a baseline into segments.
const SEGMENT_GAP: f32 = 1.5;
/// Fraction of the page height treated as header/footer band at each edge.
const FURNITURE_BAND: f32 = 0.08;
/// Minimum gutter width (pt) for two-column detection.
const MIN_GUTTER: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Full,
    Left,
    Right,
}

/// A reading-ordered line of text: glyphs on one baseline within one column.
#[derive(Debug, Clone)]
pub struct Line {
    pub page: u32,
    pub column: Column,
    /// Region index on the page; changes when the reading order jumps
    /// (across a full-width line or from one column to the next).
    pub region: usize,
    pub glyphs: Vec<Glyph>,
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub size: f32,
    /// Right edge of the text column the line belongs to.
    pub column_x1: f32,
    /// Width of the text column the line belongs to.
    pub column_width: f32,
}

#[derive(Debug, Clone)]
struct Segment {
    baseline: usize,
    glyphs: Vec<Glyph>,
    x0: f32,
    x1: f32,
    y: f32,
    size: f32,
}

impl Segment {
    fn new(baseline: usize, glyphs: Vec<Glyph>) -> Option<Self> {
        let first = glyphs.first()?;
        let x0 = first.x;
        let x1 = glyphs
            .iter()
            .map(|g| g.x + g.width)
            .fold(f32::MIN, f32::max);
        let size = dominant_size(&glyphs);
        let y = glyphs
            .iter()
            .find(|g| (g.size - size).abs() < 0.5)
            .map_or(first.y, |g| g.y);
        Some(Self {
            baseline,
            glyphs,
            x0,
            x1,
            y,
            size,
        })
    }

    fn text(&self) -> String {
        self.glyphs.iter().map(|g| g.text.as_str()).collect()
    }
}

/// Character-weighted most common font size, rounded to 0.5pt.
pub fn dominant_size(glyphs: &[Glyph]) -> f32 {
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for glyph in glyphs {
        *counts.entry(half_points(glyph.size)).or_default() += glyph.text.chars().count();
    }
    counts
        .into_iter()
        .max_by_key(|&(size, count)| (count, size))
        .map_or(0.0, |(size, _)| size as f32 / 2.0)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "font sizes are small positive numbers"
)]
pub fn half_points(size: f32) -> i32 {
    (size * 2.0).round() as i32
}

/// Lays out all pages into one reading-ordered list of lines.
pub fn lines(pages: &[Page], keep_furniture: bool) -> Vec<Line> {
    let mut segmented: Vec<(&Page, Vec<Segment>)> =
        pages.iter().map(|page| (page, segments(page))).collect();
    if !keep_furniture {
        strip_furniture(&mut segmented);
    }
    segmented
        .into_iter()
        .flat_map(|(page, segments)| page_lines(page, segments))
        .collect()
}

fn segments(page: &Page) -> Vec<Segment> {
    let mut glyphs: Vec<&Glyph> = page
        .glyphs
        .iter()
        .filter(|g| !g.text.trim().is_empty() && g.size > 0.0)
        .collect();
    glyphs.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));

    let mut baselines: Vec<Vec<Glyph>> = Vec::new();
    let mut reference: Option<(f32, f32)> = None;
    for glyph in glyphs {
        match (reference, baselines.last_mut()) {
            (Some((y, size)), Some(current))
                if (y - glyph.y).abs() <= BASELINE_TOLERANCE * size.max(glyph.size) =>
            {
                current.push(glyph.clone());
            }
            _ => {
                baselines.push(vec![glyph.clone()]);
                reference = Some((glyph.y, glyph.size));
            }
        }
    }

    let mut out = Vec::new();
    for (baseline, mut glyphs) in baselines.into_iter().enumerate() {
        glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
        dedupe_overprint(&mut glyphs);
        let mut current: Vec<Glyph> = Vec::new();
        for glyph in glyphs {
            if let Some(prev) = current.last() {
                let gap = glyph.x - (prev.x + prev.width);
                if gap > SEGMENT_GAP * prev.size.max(glyph.size) {
                    out.extend(Segment::new(baseline, std::mem::take(&mut current)));
                }
            }
            current.push(glyph);
        }
        out.extend(Segment::new(baseline, current));
    }
    for segment in &mut out {
        keep_leading_bullet_only(&mut segment.glyphs);
    }
    out.retain(|s| !s.glyphs.is_empty());
    out.into_iter()
        .filter_map(|s| Segment::new(s.baseline, s.glyphs))
        .collect()
}

/// Shapes read as bullets only survive where a list marker can be: first in
/// their segment and followed by text of a matching size. Elsewhere (icons,
/// chart markers) they are dropped.
fn keep_leading_bullet_only(glyphs: &mut Vec<Glyph>) {
    let leading_ok = match glyphs.as_slice() {
        [first, next, ..] if first.synthetic => {
            !next.synthetic && (0.6..=1.6).contains(&(first.size / next.size.max(0.1)))
        }
        _ => false,
    };
    let mut index = 0;
    glyphs.retain(|g| {
        let keep = !g.synthetic || (index == 0 && leading_ok);
        index += 1;
        keep
    });
}

/// Drops glyphs drawn twice at (almost) the same spot — a common fake-bold trick.
fn dedupe_overprint(glyphs: &mut Vec<Glyph>) {
    glyphs.dedup_by(|next, prev| {
        next.text == prev.text
            && (next.x - prev.x).abs() < prev.width.max(0.5) * 0.3
            && (next.y - prev.y).abs() < 0.5
    });
}

fn band(page: &Page, segment: &Segment) -> bool {
    let relative = (segment.y - page.bbox[1]) / page.height().max(1.0);
    !(FURNITURE_BAND..=1.0 - FURNITURE_BAND).contains(&relative)
}

/// Normalised furniture key: digits collapsed so "Page 3" ≡ "Page 12".
fn furniture_key(text: &str) -> String {
    let mut key = String::new();
    for c in text.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        if c.is_ascii_digit() {
            if !key.ends_with('#') {
                key.push('#');
            }
        } else {
            key.push(c);
        }
    }
    key
}

/// "3", "Page 3", "3 / 12", "Page 3 of 12", "- 3 -", "iv".
pub fn is_page_number(text: &str) -> bool {
    let lower = text.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || matches!(c, '/' | '|' | '-' | '–' | '—'))
        .filter(|t| !t.is_empty())
        .collect();
    let numeric = |t: &str| t.chars().all(|c| c.is_ascii_digit());
    let roman = |t: &str| t.len() <= 6 && t.chars().all(|c| "ivxlcdm".contains(c));
    let has_number = tokens.iter().any(|t| numeric(t));
    let all_ok = tokens
        .iter()
        .all(|t| numeric(t) || matches!(*t, "page" | "p." | "of" | "pg."));
    (has_number && all_ok) || (tokens.len() == 1 && tokens.iter().all(|t| roman(t)))
}

fn strip_furniture(pages: &mut [(&Page, Vec<Segment>)]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (page, segments) in pages.iter() {
        let mut keys: Vec<String> = segments
            .iter()
            .filter(|s| band(page, s))
            .map(|s| furniture_key(&s.text()))
            .collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            *seen.entry(key).or_default() += 1;
        }
    }
    let threshold = (pages.len() / 2).max(2);
    let enough_pages = pages.len() >= 3;
    let repeated = |key: &str| enough_pages && seen.get(key).is_some_and(|n| *n >= threshold);

    for (page, segments) in pages.iter_mut() {
        segments.retain(|s| {
            !band(page, s) || {
                let text = s.text();
                !is_page_number(&text) && !repeated(&furniture_key(&text))
            }
        });
    }
}

/// Finds the x of a vertical gutter splitting the page into two text columns.
#[expect(
    clippy::cast_precision_loss,
    reason = "segment counts are tiny compared to f32 precision"
)]
fn find_gutter(segments: &[Segment]) -> Option<f32> {
    if segments.len() < 6 {
        return None;
    }
    let left_edge = segments.iter().map(|s| s.x0).fold(f32::MAX, f32::min);
    let right_edge = segments.iter().map(|s| s.x1).fold(f32::MIN, f32::max);
    let width = right_edge - left_edge;
    let tolerance = (segments.len() / 20).max(1);
    // Full-width blocks (title, abstract) legitimately cross the gutter; only
    // narrower segments crossing it argue against a column layout.
    let narrow: Vec<&Segment> = segments
        .iter()
        .filter(|s| s.x1 - s.x0 < width * 0.6)
        .collect();

    let mut best: Option<(f32, f32)> = None; // (start, end) of the widest gap run
    let mut run_start: Option<f32> = None;
    let mut x = left_edge + width * 0.3;
    let stop = left_edge + width * 0.7;
    while x <= stop {
        let crossing = narrow.iter().filter(|s| s.x0 < x && s.x1 > x).count();
        if crossing <= tolerance {
            run_start.get_or_insert(x);
        } else if let Some(start) = run_start.take()
            && best.is_none_or(|(s, e)| x - start > e - s)
        {
            best = Some((start, x));
        }
        x += 1.0;
    }
    if let Some(start) = run_start
        && best.is_none_or(|(s, e)| stop - start > e - s)
    {
        best = Some((start, stop));
    }

    let (start, end) = best.filter(|(s, e)| e - s >= MIN_GUTTER)?;
    let gutter = f32::midpoint(start, end);
    let median_width = |side: Vec<f32>| {
        let mut side = side;
        side.sort_by(f32::total_cmp);
        side.get(side.len() / 2).copied()
    };
    let left: Vec<f32> = segments
        .iter()
        .filter(|s| s.x1 <= gutter)
        .map(|s| s.x1 - s.x0)
        .collect();
    let right: Vec<f32> = segments
        .iter()
        .filter(|s| s.x0 >= gutter)
        .map(|s| s.x1 - s.x0)
        .collect();
    let full = segments.len() - left.len() - right.len();
    if left.len() < 3 || right.len() < 3 || full as f32 > segments.len() as f32 * 0.5 {
        return None;
    }
    let left_ok = median_width(left).is_some_and(|w| w >= 0.4 * (gutter - left_edge));
    let right_ok = median_width(right).is_some_and(|w| w >= 0.4 * (right_edge - gutter));
    (left_ok && right_ok).then_some(gutter)
}

type Group = (Column, Vec<Segment>);

/// Assigns each segment a column and merges segments sharing a baseline and
/// column into line groups (top-down order).
fn column_groups(segments: Vec<Segment>) -> Vec<Group> {
    let gutter = find_gutter(&segments);
    // Narrow text above or below the band where both columns have content
    // (e.g. a heading after the columns end) reads as full width.
    let band = |keep: fn(&Segment, f32) -> bool| {
        let g = gutter?;
        let ys = segments
            .iter()
            .filter(|s| keep(s, g))
            .map(|s| (s.y, s.size));
        ys.fold(None, |acc: Option<(f32, f32)>, (y, size)| {
            let (lo, hi) = acc.unwrap_or((y, y));
            Some((lo.min(y - 1.5 * size), hi.max(y + 1.5 * size)))
        })
    };
    let left_band = band(|s, g| s.x1 <= g);
    let right_band = band(|s, g| s.x0 >= g);
    let within = |band: Option<(f32, f32)>, y: f32| band.is_some_and(|(lo, hi)| y >= lo && y <= hi);
    // Outside the shared band, a column keeps text that follows its previous
    // line at normal leading (one column simply runs longer than the other).
    let mut last: [Option<(f32, f32)>; 2] = [None, None];
    let mut column_of = |s: &Segment| {
        let follows = |previous: Option<(f32, f32)>| {
            previous.is_some_and(|(y, size)| y - s.y <= 2.2 * size.min(s.size))
        };
        let column = match gutter {
            Some(g) if s.x1 <= g && (within(right_band, s.y) || follows(last[0])) => Column::Left,
            Some(g) if s.x0 >= g && (within(left_band, s.y) || follows(last[1])) => Column::Right,
            _ => Column::Full,
        };
        last = match column {
            Column::Left => [Some((s.y, s.size)), last[1]],
            Column::Right => [last[0], Some((s.y, s.size))],
            Column::Full => [None, None],
        };
        column
    };

    let mut merged: Vec<Group> = Vec::new();
    for segment in segments {
        let column = column_of(&segment);
        match merged.iter_mut().rev().find(|(c, group)| {
            *c == column
                && group
                    .first()
                    .is_some_and(|g| g.baseline == segment.baseline)
        }) {
            Some((_, group)) => group.push(segment),
            None => merged.push((column, vec![segment])),
        }
    }
    merged
}

/// Reading order: regions split by full-width lines; within a two-column
/// region the whole left column comes before the right one.
fn reading_order(groups: Vec<Group>) -> Vec<(usize, Group)> {
    let mut ordered: Vec<(usize, Group)> = Vec::new();
    let mut region = 0;
    let mut pending_right: Vec<Group> = Vec::new();
    let mut in_columns = false;
    for (column, group) in groups {
        if column == Column::Full && in_columns {
            region += 1;
            ordered.extend(pending_right.drain(..).map(|g| (region, g)));
            in_columns = false;
            region += 1;
        } else if column != Column::Full && !in_columns {
            region += 1;
            in_columns = true;
        }
        if column == Column::Right {
            pending_right.push((column, group));
        } else {
            ordered.push((region, (column, group)));
        }
    }
    if !pending_right.is_empty() {
        region += 1;
        ordered.extend(pending_right.into_iter().map(|g| (region, g)));
    }
    ordered
}

fn page_lines(page: &Page, segments: Vec<Segment>) -> Vec<Line> {
    let ordered = reading_order(column_groups(segments));
    let mut lines: Vec<Line> = ordered
        .into_iter()
        .filter_map(|(region, (column, group))| {
            let glyphs: Vec<Glyph> = group.into_iter().flat_map(|s| s.glyphs).collect();
            let segment = Segment::new(0, glyphs)?;
            Some(Line {
                page: page.number,
                column,
                region,
                x0: segment.x0,
                x1: segment.x1,
                y: segment.y,
                size: segment.size,
                glyphs: segment.glyphs,
                column_x1: 0.0,
                column_width: 0.0,
            })
        })
        .collect();

    for column in [Column::Full, Column::Left, Column::Right] {
        let members = lines.iter().filter(|l| l.column == column);
        let x0 = members.clone().map(|l| l.x0).fold(f32::MAX, f32::min);
        let x1 = members.map(|l| l.x1).fold(f32::MIN, f32::max);
        for line in lines.iter_mut().filter(|l| l.column == column) {
            line.column_x1 = x1;
            line.column_width = (x1 - x0).max(1.0);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontStyle;

    fn word(text: &str, x: f32, y: f32) -> Vec<Glyph> {
        text.chars()
            .enumerate()
            .map(|(i, c)| Glyph {
                text: c.to_string(),
                x: x + i as f32 * 5.0,
                y,
                width: 5.0,
                size: 10.0,
                style: FontStyle::default(),
                link: None,
                synthetic: false,
            })
            .collect()
    }

    fn page(number: u32, words: &[(&str, f32, f32)]) -> Page {
        Page {
            number,
            bbox: [0.0, 0.0, 612.0, 792.0],
            glyphs: words.iter().flat_map(|(t, x, y)| word(t, *x, *y)).collect(),
        }
    }

    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.glyphs.iter().map(|g| g.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn synthetic_bullets_survive_only_as_leading_markers() {
        let mut glyphs = word("ab", 100.0, 700.0);
        let bullet = Glyph {
            text: "•".to_string(),
            synthetic: true,
            ..glyphs[0].clone()
        };
        let mut leading = vec![bullet.clone()];
        leading.extend(glyphs.clone());
        keep_leading_bullet_only(&mut leading);
        assert_eq!(leading.len(), 3);

        glyphs.push(bullet);
        keep_leading_bullet_only(&mut glyphs);
        assert_eq!(glyphs.len(), 2);
    }

    #[test]
    fn page_number_detection() {
        for yes in ["3", "Page 3", "3 / 12", "Page 3 of 12", "- 3 -", "iv"] {
            assert!(is_page_number(yes), "{yes}");
        }
        for no in ["Chapter one", "3 apples", ""] {
            assert!(!is_page_number(no), "{no}");
        }
    }

    #[test]
    fn baselines_are_ordered_top_down_then_left_right() {
        let p = page(1, &[("second", 72.0, 680.0), ("first", 72.0, 700.0)]);
        assert_eq!(texts(&lines(&[p], true)), ["first", "second"]);
    }

    #[test]
    fn two_columns_are_read_left_column_first() {
        let mut words = vec![("Title spanning the whole page width ok", 100.0, 740.0)];
        for i in 0..6u8 {
            let y = 700.0 - f32::from(i) * 14.0;
            words.push(("leftcolumntextxxxxxxxxxxxxxxxxxxxxxxxxx", 72.0, y));
            words.push(("rightcolumntextxxxxxxxxxxxxxxxxxxxxxxxx", 330.0, y));
        }
        let lines = lines(&[page(1, &words)], true);
        let order = texts(&lines);
        assert!(order[0].starts_with("Title"));
        assert!(order[1..7].iter().all(|t| t.starts_with("left")));
        assert!(order[7..].iter().all(|t| t.starts_with("right")));
    }

    #[test]
    fn narrow_text_below_the_columns_is_full_width() {
        let mut words = Vec::new();
        for i in 0..6u8 {
            let y = 700.0 - f32::from(i) * 14.0;
            words.push(("leftcolumntextxxxxxxxxxxxxxxxxxxxxxxxxx", 72.0, y));
            words.push(("rightcolumntextxxxxxxxxxxxxxxxxxxxxxxxx", 330.0, y));
        }
        words.push(("After", 72.0, 500.0));
        let lines = lines(&[page(1, &words)], true);
        let after = lines.last().unwrap();
        assert_eq!(after.column, Column::Full);
        assert!(
            texts(&lines)
                .iter()
                .rev()
                .skip(1)
                .take(6)
                .all(|t| t.starts_with("right"))
        );
    }

    #[test]
    fn repeated_headers_and_page_numbers_are_dropped() {
        let pages: Vec<Page> = (1..=3)
            .map(|n| {
                page(
                    n,
                    &[
                        ("Running header", 72.0, 760.0),
                        ("body", 72.0, 400.0),
                        (if n == 2 { "2" } else { "9" }, 300.0, 30.0),
                    ],
                )
            })
            .collect();
        assert_eq!(texts(&lines(&pages, false)), ["body", "body", "body"]);
        assert_eq!(lines(&pages, true).len(), 9);
    }
}
