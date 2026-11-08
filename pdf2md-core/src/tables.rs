//! Table detection: grids of drawn rulings, rule-only (booktabs) tables, and
//! borderless tables recovered from text alignment, plus per-column alignment.

use crate::interp::{Glyph, Ruling};

/// Rulings this close (pt) across their axis are collinear.
const COLLINEAR: f32 = 1.0;
/// Rulings this close (pt) along their axis, or across each other, touch.
const TOUCH: f32 = 2.0;
/// Rule-only tables: rules whose ends differ by at most this (pt) align.
const SAME_EXTENT: f32 = 3.0;
/// Smallest share of non-empty cells for a ruled grid to be a table.
const MIN_GRID_FILL: f32 = 0.3;
/// Smallest share of non-empty cells for an alignment table.
const MIN_ALIGNED_FILL: f32 = 0.6;
/// Gap between glyphs (in font sizes) that separates words.
const WORD_GAP: f32 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// One table cell: its text lines, glyphs sorted left to right.
pub type Cell = Vec<Vec<Glyph>>;

#[derive(Debug, Clone)]
pub struct Table {
    /// `[x0, y0, x1, y1]` in user space.
    pub bbox: [f32; 4],
    pub align: Vec<Align>,
    /// `rows[0]` is the header row; every row has `align.len()` cells.
    pub rows: Vec<Vec<Cell>>,
}

/// Horizontal extent of a piece of text on one line.
#[derive(Debug, Clone)]
pub struct Piece {
    pub x0: f32,
    pub x1: f32,
    pub glyphs: Vec<Glyph>,
}

fn centre(glyph: &Glyph) -> (f32, f32) {
    (glyph.x + glyph.width / 2.0, glyph.y + glyph.size * 0.3)
}

fn inside([x0, y0, x1, y1]: [f32; 4], (x, y): (f32, f32)) -> bool {
    x >= x0 && x <= x1 && y >= y0 && y <= y1
}

/// Groups glyphs into baselines (top-down), each sorted left to right.
pub fn text_lines(glyphs: Vec<Glyph>) -> Vec<Vec<Glyph>> {
    let mut glyphs: Vec<Glyph> = glyphs
        .into_iter()
        .filter(|g| !g.text.trim().is_empty() && g.size > 0.0)
        .collect();
    glyphs.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
    let mut lines: Vec<Vec<Glyph>> = Vec::new();
    for glyph in glyphs {
        match lines.last_mut() {
            Some(line)
                if line
                    .first()
                    .is_some_and(|f| (f.y - glyph.y).abs() <= 0.45 * f.size.max(glyph.size)) =>
            {
                line.push(glyph);
            }
            _ => lines.push(vec![glyph]),
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
    lines
}

/// Merges overlapping or touching collinear rulings (md2pdf strokes every
/// cell rectangle, so shared edges are drawn twice).
fn merge(mut rulings: Vec<Ruling>) -> Vec<Ruling> {
    rulings.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    let mut groups: Vec<Vec<Ruling>> = Vec::new();
    for ruling in rulings {
        match groups.last_mut() {
            Some(group)
                if group
                    .first()
                    .is_some_and(|f| ruling.pos - f.pos <= COLLINEAR) =>
            {
                group.push(ruling);
            }
            _ => groups.push(vec![ruling]),
        }
    }
    let mut merged = Vec::new();
    for mut group in groups {
        group.sort_by(|a, b| a.from.total_cmp(&b.from));
        let mut current: Option<Ruling> = None;
        for ruling in group {
            match current.as_mut() {
                Some(c) if ruling.from <= c.to + TOUCH => c.to = c.to.max(ruling.to),
                _ => merged.extend(current.replace(ruling)),
            }
        }
        merged.extend(current);
    }
    merged
}

fn touches(h: &Ruling, v: &Ruling) -> bool {
    v.pos >= h.from - TOUCH
        && v.pos <= h.to + TOUCH
        && h.pos >= v.from - TOUCH
        && h.pos <= v.to + TOUCH
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        let grand = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = grand;
        }
        i = p;
    }
    i
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if let Some(slot) = parent.get_mut(ra.max(rb)) {
        *slot = ra.min(rb);
    }
}

/// Distinct values, sorted ascending, merging those within `tolerance`.
fn distinct(mut values: Vec<f32>, tolerance: f32) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    values.dedup_by(|b, a| *b - *a <= tolerance);
    values
}

/// Result of the ruling-based pass over one page.
#[derive(Debug, Default)]
pub struct Ruled {
    pub tables: Vec<Table>,
    /// Glyphs not inside any table.
    pub glyphs: Vec<Glyph>,
    /// Horizontal rulings not used by any table (candidate `---` rules).
    pub loose: Vec<Ruling>,
}

/// Detects grid and rule-only tables, taking their glyphs out of the page.
pub fn ruled(rulings: &[Ruling], glyphs: Vec<Glyph>) -> Ruled {
    let (horizontal, vertical): (Vec<Ruling>, Vec<Ruling>) =
        rulings.iter().partition(|r| r.horizontal);
    let horizontal = merge(horizontal);
    let vertical = merge(vertical);

    let all: Vec<&Ruling> = horizontal.iter().chain(&vertical).collect();
    let mut parent: Vec<usize> = (0..all.len()).collect();
    for (i, h) in horizontal.iter().enumerate() {
        for (j, v) in vertical.iter().enumerate() {
            if touches(h, v) {
                union(&mut parent, i, horizontal.len() + j);
            }
        }
    }
    let mut clusters: Vec<Vec<&Ruling>> = Vec::new();
    let mut root_slot: Vec<Option<usize>> = vec![None; all.len()];
    for (i, ruling) in all.iter().enumerate() {
        let root = find(&mut parent, i);
        if let Some(slot) = root_slot.get_mut(root) {
            let index = *slot.get_or_insert_with(|| {
                clusters.push(Vec::new());
                clusters.len() - 1
            });
            if let Some(cluster) = clusters.get_mut(index) {
                cluster.push(ruling);
            }
        }
    }

    let mut out = Ruled {
        glyphs,
        ..Ruled::default()
    };
    let mut lone_rules: Vec<Ruling> = Vec::new();
    for cluster in clusters {
        let hs: Vec<Ruling> = cluster
            .iter()
            .filter(|r| r.horizontal)
            .map(|r| **r)
            .collect();
        let vs: Vec<Ruling> = cluster
            .iter()
            .filter(|r| !r.horizontal)
            .map(|r| **r)
            .collect();
        if hs.len() >= 2 && vs.len() >= 2 {
            let remaining = std::mem::take(&mut out.glyphs);
            match grid(&hs, &vs, remaining) {
                Ok((table, rest)) => {
                    out.tables.push(table);
                    out.glyphs = rest;
                }
                Err(rest) => out.glyphs = rest,
            }
        } else if vs.is_empty() {
            lone_rules.extend(hs);
        }
    }

    let (tables, loose) = booktabs(lone_rules, &mut out.glyphs);
    out.tables.extend(tables);
    out.loose = loose;
    out
}

/// Builds a table from a connected cluster of rulings, or hands the glyphs
/// back when the grid has no table shape.
fn grid(
    hs: &[Ruling],
    vs: &[Ruling],
    glyphs: Vec<Glyph>,
) -> Result<(Table, Vec<Glyph>), Vec<Glyph>> {
    let xs = distinct(vs.iter().map(|r| r.pos).collect(), COLLINEAR);
    let mut ys = distinct(hs.iter().map(|r| r.pos).collect(), COLLINEAR);
    ys.reverse(); // top-down
    let (Some(&left), Some(&right), Some(&top), Some(&bottom)) =
        (xs.first(), xs.last(), ys.first(), ys.last())
    else {
        return Err(glyphs);
    };
    let cols = xs.len().saturating_sub(1);
    let rows = ys.len().saturating_sub(1);
    // A single framed box (code block, callout) is not a table.
    if cols * rows < 2 {
        return Err(glyphs);
    }
    let bbox = [left, bottom, right, top];
    let x_at = |i: usize| xs.get(i).copied().unwrap_or(right);
    let y_at = |i: usize| ys.get(i).copied().unwrap_or(bottom);

    // Cells joined where the boundary between them is not drawn (spans).
    let index = |r: usize, c: usize| r * cols + c;
    let mut parent: Vec<usize> = (0..rows * cols).collect();
    for r in 0..rows {
        let mid_y = f32::midpoint(y_at(r), y_at(r + 1));
        for c in 0..cols {
            let mid_x = f32::midpoint(x_at(c), x_at(c + 1));
            let has_v = |x: f32| {
                vs.iter()
                    .any(|v| (v.pos - x).abs() <= COLLINEAR && v.from <= mid_y && v.to >= mid_y)
            };
            let has_h = |y: f32| {
                hs.iter()
                    .any(|h| (h.pos - y).abs() <= COLLINEAR && h.from <= mid_x && h.to >= mid_x)
            };
            if c + 1 < cols && !has_v(x_at(c + 1)) {
                union(&mut parent, index(r, c), index(r, c + 1));
            }
            if r + 1 < rows && !has_h(y_at(r + 1)) {
                union(&mut parent, index(r, c), index(r + 1, c));
            }
        }
    }

    let mut cell_glyphs: Vec<Vec<Glyph>> = vec![Vec::new(); rows * cols];
    let mut rest = Vec::new();
    for glyph in glyphs {
        let point = centre(&glyph);
        // Shapes read as bullets inside a grid are chart markers, not text.
        if glyph.synthetic || !inside(bbox, point) {
            rest.push(glyph);
            continue;
        }
        let c = (0..cols)
            .find(|&c| point.0 <= x_at(c + 1))
            .unwrap_or(cols - 1);
        let r = (0..rows)
            .find(|&r| point.1 >= y_at(r + 1))
            .unwrap_or(rows - 1);
        let anchor = find(&mut parent, index(r, c));
        if let Some(cell) = cell_glyphs.get_mut(anchor) {
            cell.push(glyph);
        }
    }

    let anchors: Vec<usize> = (0..rows * cols)
        .filter(|&i| find(&mut parent, i) == i)
        .collect();
    let filled = anchors
        .iter()
        .filter(|&&i| cell_glyphs.get(i).is_some_and(|g| !g.is_empty()))
        .count();
    if (filled as f32) < MIN_GRID_FILL * anchors.len() as f32 {
        rest.extend(cell_glyphs.into_iter().flatten());
        return Err(rest);
    }

    let spanning: Vec<bool> = (0..rows * cols)
        .map(|i| {
            let root = find(&mut parent, i);
            (0..rows * cols)
                .filter(|&j| find(&mut parent, j) == root)
                .count()
                > 1
        })
        .collect();
    let mut cells = cell_glyphs.into_iter().map(text_lines);
    let table_rows: Vec<Vec<Cell>> = (0..rows)
        .map(|_| cells.by_ref().take(cols).collect())
        .collect();
    let align = (0..cols)
        .map(|c| {
            let boxes = (0..rows).filter_map(|r| {
                let cell = table_rows.get(r)?.get(c)?;
                let spans = spanning.get(index(r, c)).copied().unwrap_or(true);
                (!spans).then(|| extent(cell)).flatten().map(|e| (r, e))
            });
            vote(boxes, x_at(c), x_at(c + 1))
        })
        .collect();
    Ok((
        Table {
            bbox,
            align,
            rows: table_rows,
        },
        rest,
    ))
}

/// Horizontal extent of a cell's text.
fn extent(cell: &Cell) -> Option<(f32, f32)> {
    let glyphs = cell.iter().flatten();
    let x0 = glyphs.clone().map(|g| g.x).reduce(f32::min)?;
    let x1 = glyphs.map(|g| g.x + g.width).reduce(f32::max)?;
    Some((x0, x1))
}

/// Column alignment by majority vote over body rows (header row 0 only
/// counts when no body cell decides). Gaps are measured from the column
/// box minus the smallest gap seen (the cell padding).
fn vote(cells: impl Iterator<Item = (usize, (f32, f32))>, left: f32, right: f32) -> Align {
    let gaps: Vec<(usize, f32, f32)> = cells
        .map(|(row, (x0, x1))| (row, x0 - left, right - x1))
        .collect();
    let pad_left = gaps.iter().map(|g| g.1).fold(f32::MAX, f32::min);
    let pad_right = gaps.iter().map(|g| g.2).fold(f32::MAX, f32::min);
    let tolerance = 1.0;
    let decide = |l: f32, r: f32| {
        let (l, r) = (l - pad_left, r - pad_right);
        if l < tolerance && r > tolerance {
            Some(Align::Left)
        } else if r < tolerance && l > tolerance {
            Some(Align::Right)
        } else if (l - r).abs() < tolerance && l > tolerance {
            Some(Align::Center)
        } else {
            None
        }
    };
    let tally = |header: bool| {
        let mut counts = [0usize; 3];
        for &(row, l, r) in &gaps {
            if (row == 0) != header {
                continue;
            }
            match decide(l, r) {
                Some(Align::Left) => counts[0] += 1,
                Some(Align::Center) => counts[1] += 1,
                Some(Align::Right) => counts[2] += 1,
                _ => {}
            }
        }
        counts
    };
    let mut counts = tally(false);
    if counts.iter().all(|&n| n == 0) {
        counts = tally(true);
    }
    let [l, c, r] = counts;
    if l > c && l > r {
        Align::Left
    } else if c > l && c > r {
        Align::Center
    } else if r > l && r > c {
        Align::Right
    } else {
        Align::None
    }
}

/// Rule-only tables: ≥ 2 horizontal rules of the same extent with aligned
/// text columns between the first and the last. Returns the tables and the
/// rules left unused.
fn booktabs(rules: Vec<Ruling>, glyphs: &mut Vec<Glyph>) -> (Vec<Table>, Vec<Ruling>) {
    let mut groups: Vec<Vec<Ruling>> = Vec::new();
    for rule in rules {
        match groups.iter_mut().find(|g| {
            g.first().is_some_and(|f| {
                (f.from - rule.from).abs() <= SAME_EXTENT && (f.to - rule.to).abs() <= SAME_EXTENT
            })
        }) {
            Some(group) => group.push(rule),
            None => groups.push(vec![rule]),
        }
    }
    let mut tables = Vec::new();
    let mut loose = Vec::new();
    for group in groups {
        let top = group.iter().map(|r| r.pos).fold(f32::MIN, f32::max);
        let bottom = group.iter().map(|r| r.pos).fold(f32::MAX, f32::min);
        let from = group.iter().map(|r| r.from).fold(f32::MAX, f32::min);
        let to = group.iter().map(|r| r.to).fold(f32::MIN, f32::max);
        let bbox = [from - TOUCH, bottom, to + TOUCH, top];
        if group.len() < 2 {
            loose.extend(group);
            continue;
        }
        let (inner, outer): (Vec<Glyph>, Vec<Glyph>) = std::mem::take(glyphs)
            .into_iter()
            .partition(|g| inside(bbox, centre(g)));
        let lines = text_lines(inner.clone());
        let rows: Vec<Vec<Piece>> = lines.into_iter().map(|line| split(line, 0.6)).collect();
        let misses = usize::from(rows.len() >= 4);
        *glyphs = outer;
        if let Some(mut table) = aligned(&rows, 0.8, misses, false) {
            table.bbox = [from, bottom, to, top];
            tables.push(table);
        } else {
            glyphs.extend(inner);
            loose.extend(group);
        }
    }
    (tables, loose)
}

/// Splits a line into pieces at gaps wider than `gap` font sizes.
pub fn split(line: Vec<Glyph>, gap: f32) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    for glyph in line {
        let start_new = pieces.last().is_none_or(|p| {
            p.glyphs.last().is_none_or(|prev| {
                glyph.x - (prev.x + prev.width) > gap * prev.size.max(glyph.size)
            })
        });
        if start_new {
            pieces.push(Piece {
                x0: glyph.x,
                x1: glyph.x + glyph.width,
                glyphs: vec![glyph],
            });
        } else if let Some(piece) = pieces.last_mut() {
            piece.x1 = piece.x1.max(glyph.x + glyph.width);
            piece.glyphs.push(glyph);
        }
    }
    pieces
}

fn is_marker(piece: &Piece) -> bool {
    let text: String = piece.glyphs.iter().map(|g| g.text.as_str()).collect();
    let text = text.trim();
    let mut chars = text.chars();
    let single_symbol =
        matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_alphanumeric());
    let numbered = text.strip_suffix(['.', ')']).is_some_and(|n| {
        !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_alphanumeric())
    });
    single_symbol || numbered
}

/// x ranges covered by at most `misses` rows, at least `min_gap` wide, inside
/// `[left, right]`, swept in half points.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "page coordinates are small positive numbers"
)]
fn shared_gaps(
    rows: &[Vec<Piece>],
    left: f32,
    right: f32,
    min_gap: f32,
    misses: usize,
) -> Vec<(f32, f32)> {
    let steps = ((right - left) * 2.0).max(0.0) as usize;
    let mut gaps: Vec<(f32, f32)> = Vec::new();
    let mut run: Option<f32> = None;
    for step in 0..=steps {
        let x = left + step as f32 / 2.0;
        let covered = rows
            .iter()
            .filter(|row| row.iter().any(|p| x >= p.x0 && x <= p.x1))
            .count();
        if covered <= misses {
            run.get_or_insert(x);
        } else if let Some(start) = run.take()
            && x - start >= min_gap
        {
            gaps.push((start, x));
        }
    }
    gaps
}

type Extents = Vec<Vec<Option<(f32, f32)>>>;

/// Distributes each row's words over the columns between `gaps`. Words, not
/// whole pieces, pick their column: a cell overflowing into a gap keeps its
/// words left of it, and text closer than the split gap still lands on both
/// sides. A word belongs right of every gap it starts after.
fn fill(rows: &[Vec<Piece>], gaps: &[(f32, f32)]) -> (Vec<Vec<Cell>>, Extents) {
    let cols = gaps.len() + 1;
    let column_of = |x0: f32| gaps.iter().filter(|(_, end)| x0 + 0.5 >= *end).count();
    let mut table_rows: Vec<Vec<Cell>> = Vec::new();
    let mut boxes: Extents = Vec::new();
    for row in rows {
        let mut cells: Vec<Vec<Glyph>> = vec![Vec::new(); cols];
        let mut extents: Vec<Option<(f32, f32)>> = vec![None; cols];
        for word in row.iter().flat_map(|p| split(p.glyphs.clone(), WORD_GAP)) {
            let c = column_of(word.x0);
            if let (Some(cell), Some(e)) = (cells.get_mut(c), extents.get_mut(c)) {
                cell.extend(word.glyphs);
                *e = Some(e.map_or((word.x0, word.x1), |(a, b)| {
                    (a.min(word.x0), b.max(word.x1))
                }));
            }
        }
        table_rows.push(cells.into_iter().map(|g| vec![g]).collect());
        boxes.push(extents);
    }
    (table_rows, boxes)
}

/// Aligned text that is something else: a list with a wide gap after each
/// marker, or mostly monospace (code).
fn other_aligned_text(rows: &[Vec<Piece>]) -> bool {
    let list = rows.iter().all(|row| row.first().is_some_and(is_marker));
    let glyphs = rows.iter().flatten().flat_map(|p| &p.glyphs);
    let mono = glyphs.clone().filter(|g| g.style.mono).count();
    list || mono * 2 > glyphs.count()
}

/// Builds a table from rows of text pieces when x ranges empty in all rows
/// but `misses` of them split them into ≥ 2 columns. `merge` folds wrapped
/// rows together and then requires ≥ 3 rows.
pub fn aligned(rows: &[Vec<Piece>], min_gap: f32, misses: usize, merge: bool) -> Option<Table> {
    if rows.len() < 2 || other_aligned_text(rows) {
        return None;
    }
    let pieces = rows.iter().flatten();
    let left = pieces.clone().map(|p| p.x0).reduce(f32::min)?;
    let right = pieces.clone().map(|p| p.x1).reduce(f32::max)?;
    let glyphs = pieces.flat_map(|p| &p.glyphs);
    let top = glyphs
        .clone()
        .map(|g| g.y + g.size)
        .fold(f32::MIN, f32::max);
    let bottom = glyphs.clone().map(|g| g.y).fold(f32::MAX, f32::min);
    let size = {
        let mut sizes: Vec<f32> = glyphs.map(|g| g.size).collect();
        sizes.sort_by(f32::total_cmp);
        sizes.get(sizes.len() / 2).copied()?
    };

    let gaps = shared_gaps(rows, left, right, min_gap * size, misses);
    if gaps.is_empty() {
        return None;
    }
    let cols = gaps.len() + 1;
    let (mut table_rows, mut boxes) = fill(rows, &gaps);
    if merge {
        merge_continuations(&mut table_rows, &mut boxes);
        if table_rows.len() < 3 {
            return None;
        }
    }
    let total = table_rows.len() * cols;
    let filled = boxes.iter().flatten().filter(|e| e.is_some()).count();
    if (filled as f32) < MIN_ALIGNED_FILL * total as f32 {
        return None;
    }

    let align = (0..cols)
        .map(|c| {
            let column: Vec<(usize, (f32, f32))> = boxes
                .iter()
                .enumerate()
                .filter_map(|(r, row)| Some((r, (*row.get(c)?)?)))
                .collect();
            let x0 = column.iter().map(|(_, e)| e.0).fold(f32::MAX, f32::min);
            let x1 = column.iter().map(|(_, e)| e.1).fold(f32::MIN, f32::max);
            vote(column.into_iter(), x0, x1)
        })
        .collect();
    for row in &mut table_rows {
        for cell in row.iter_mut() {
            cell.retain(|line| !line.is_empty());
        }
    }
    Some(Table {
        bbox: [left, bottom, right, top],
        align,
        rows: table_rows,
    })
}

/// Folds a row into the one above when every non-empty cell continues a
/// non-empty cell above in lowercase (cells wrapped over two lines).
fn merge_continuations(rows: &mut Vec<Vec<Cell>>, boxes: &mut Extents) {
    let starts_lower = |cell: &Cell| {
        cell.first()
            .and_then(|line| line.first())
            .and_then(|g| g.text.chars().next())
            .is_some_and(char::is_lowercase)
    };
    let mut r = 1;
    while r < rows.len() {
        let continues = match (rows.get(r - 1), rows.get(r)) {
            (Some(above), Some(row)) => {
                row.iter().any(|c| !c.is_empty())
                    && row
                        .iter()
                        .zip(above)
                        .all(|(cell, up)| cell.is_empty() || (starts_lower(cell) && !up.is_empty()))
            }
            _ => false,
        };
        if !continues {
            r += 1;
            continue;
        }
        let row = rows.remove(r);
        let extents = boxes.remove(r);
        if let (Some(above), Some(above_box)) = (rows.get_mut(r - 1), boxes.get_mut(r - 1)) {
            for ((cell, up), (e, up_e)) in row
                .into_iter()
                .zip(above.iter_mut())
                .zip(extents.into_iter().zip(above_box.iter_mut()))
            {
                up.extend(cell.into_iter().filter(|line| !line.is_empty()));
                if let (Some((a, b)), Some((c, d))) = (e, up_e.as_mut()) {
                    *c = c.min(a);
                    *d = d.max(b);
                }
            }
        }
    }
}

/// Column widths of a candidate alignment table look like a two-column page
/// (each column wider than 35% of the text width).
pub fn looks_like_columns(table: &Table) -> bool {
    let [left, _, right, _] = table.bbox;
    let width = (right - left).max(1.0);
    let cols = table.align.len();
    if cols != 2 {
        return false;
    }
    (0..cols).all(|c| {
        let cells = table
            .rows
            .iter()
            .filter_map(|row| row.get(c))
            .filter_map(extent);
        let x0 = cells.clone().map(|e| e.0).fold(f32::MAX, f32::min);
        let x1 = cells.map(|e| e.1).fold(f32::MIN, f32::max);
        x1 - x0 > 0.35 * width
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontStyle;

    fn h(y: f32, from: f32, to: f32) -> Ruling {
        Ruling {
            horizontal: true,
            pos: y,
            from,
            to,
        }
    }

    fn v(x: f32, from: f32, to: f32) -> Ruling {
        Ruling {
            horizontal: false,
            pos: x,
            from,
            to,
        }
    }

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

    fn cell_text(cell: &Cell) -> String {
        cell.iter()
            .map(|line| line.iter().map(|g| g.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn texts(table: &Table) -> Vec<Vec<String>> {
        table
            .rows
            .iter()
            .map(|row| row.iter().map(cell_text).collect())
            .collect()
    }

    /// md2pdf style: every cell stroked as its own rectangle.
    fn cell_rects(xs: &[f32], ys: &[f32]) -> Vec<Ruling> {
        let mut out = Vec::new();
        for pair_y in ys.windows(2) {
            for pair_x in xs.windows(2) {
                let ([x0, x1], [y1, y0]) = (pair_x, pair_y) else {
                    continue;
                };
                let (x0, x1, y0, y1) = (*x0, *x1, *y0, *y1);
                out.extend([h(y0, x0, x1), h(y1, x0, x1), v(x0, y0, y1), v(x1, y0, y1)]);
            }
        }
        out
    }

    #[test]
    fn per_cell_rectangles_merge_into_one_grid() {
        let rulings = cell_rects(&[100.0, 200.0, 300.0], &[700.0, 680.0, 660.0]);
        let merged = merge(rulings.iter().filter(|r| r.horizontal).copied().collect());
        assert_eq!(merged.len(), 3);
        assert!(
            merged
                .iter()
                .all(|r| (r.from - 100.0).abs() < 0.01 && (r.to - 300.0).abs() < 0.01)
        );

        let mut glyphs = word("Name", 104.0, 686.0);
        glyphs.extend(word("Role", 204.0, 686.0));
        glyphs.extend(word("Ada", 104.0, 666.0));
        glyphs.extend(word("Eng", 204.0, 666.0));
        glyphs.extend(word("outside", 104.0, 600.0));
        let ruled = ruled(&rulings, glyphs);
        assert_eq!(ruled.tables.len(), 1);
        assert_eq!(texts(&ruled.tables[0]), [["Name", "Role"], ["Ada", "Eng"]]);
        assert_eq!(ruled.glyphs.len(), "outside".len());
        assert!(ruled.loose.is_empty());
    }

    #[test]
    fn missing_boundary_makes_a_spanning_cell() {
        // Two columns, but the first row has no middle divider.
        let rulings = vec![
            h(700.0, 100.0, 300.0),
            h(680.0, 100.0, 300.0),
            h(660.0, 100.0, 300.0),
            v(100.0, 660.0, 700.0),
            v(300.0, 660.0, 700.0),
            v(200.0, 660.0, 680.0),
        ];
        let mut glyphs = word("Wide title", 150.0, 686.0);
        glyphs.extend(word("a", 104.0, 666.0));
        glyphs.extend(word("b", 204.0, 666.0));
        let table = ruled(&rulings, glyphs).tables.remove(0);
        assert_eq!(texts(&table), [["Widetitle", ""], ["a", "b"]]);
    }

    #[test]
    fn a_single_box_is_not_a_table() {
        let rulings = cell_rects(&[100.0, 300.0], &[700.0, 600.0]);
        let ruled = ruled(&rulings, word("code", 104.0, 650.0));
        assert!(ruled.tables.is_empty());
        assert_eq!(ruled.glyphs.len(), 4);
    }

    #[test]
    fn booktabs_three_rules() {
        let rulings = vec![
            h(700.0, 100.0, 400.0),
            h(684.0, 100.0, 400.0),
            h(640.0, 100.0, 400.0),
        ];
        let mut glyphs = Vec::new();
        for (y, a, b) in [
            (688.0, "Model", "Score"),
            (670.0, "alpha", "0.91"),
            (656.0, "beta", "0.87"),
        ] {
            glyphs.extend(word(a, 110.0, y));
            glyphs.extend(word(b, 300.0, y));
        }
        let ruled = ruled(&rulings, glyphs);
        assert_eq!(ruled.tables.len(), 1);
        assert_eq!(
            texts(&ruled.tables[0]),
            [["Model", "Score"], ["alpha", "0.91"], ["beta", "0.87"]]
        );
        assert!(ruled.glyphs.is_empty());
    }

    #[test]
    fn a_lone_rule_stays_loose() {
        let ruled = ruled(&[h(500.0, 72.0, 540.0)], Vec::new());
        assert!(ruled.tables.is_empty());
        assert_eq!(ruled.loose.len(), 1);
    }

    fn rows(lines: &[&[(&str, f32)]]) -> Vec<Vec<Piece>> {
        lines
            .iter()
            .enumerate()
            .map(|(i, words)| {
                let y = 700.0 - i as f32 * 14.0;
                let glyphs: Vec<Glyph> = words.iter().flat_map(|(t, x)| word(t, *x, y)).collect();
                split(glyphs, 1.5)
            })
            .collect()
    }

    #[test]
    fn aligned_three_by_three() {
        let table = aligned(
            &rows(&[
                &[("Name", 72.0), ("Role", 200.0), ("Age", 320.0)],
                &[("Ada", 72.0), ("Engineer", 200.0), ("36", 330.0)],
                &[("Grace", 72.0), ("Admiral", 200.0), ("85", 330.0)],
            ]),
            1.0,
            0,
            true,
        )
        .unwrap();
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.align, [Align::Left, Align::Left, Align::Right]);
        assert_eq!(cell_text(&table.rows[1][1]), "Engineer");
    }

    #[test]
    fn two_column_prose_is_not_a_table() {
        let left = "Leftcolumntextxxxxxxxxxxxxxxxxxxxxx";
        let right = "Rightcolumntextxxxxxxxxxxxxxxxxxxxx";
        let lines: Vec<[(&str, f32); 2]> = (0..5).map(|_| [(left, 72.0), (right, 330.0)]).collect();
        let refs: Vec<&[(&str, f32)]> = lines.iter().map(<[(&str, f32); 2]>::as_slice).collect();
        let table = aligned(&rows(&refs), 1.0, 1, true).unwrap();
        assert!(looks_like_columns(&table));
    }

    #[test]
    fn bullet_list_with_wide_gaps_is_not_a_table() {
        let table = aligned(
            &rows(&[
                &[("•", 72.0), ("first item", 100.0)],
                &[("•", 72.0), ("second", 100.0)],
                &[("•", 72.0), ("third one", 100.0)],
            ]),
            1.0,
            0,
            true,
        );
        assert!(table.is_none());
    }

    #[test]
    fn alignment_vote() {
        // Column box 100..200, padding 4.
        let cells = [
            (0, (104.0, 196.0)),
            (1, (130.0, 170.0)),
            (2, (140.0, 160.0)),
        ];
        assert_eq!(vote(cells.into_iter(), 100.0, 200.0), Align::Center);
        let cells = [
            (0, (104.0, 196.0)),
            (1, (150.0, 196.0)),
            (2, (170.0, 196.0)),
        ];
        assert_eq!(vote(cells.into_iter(), 100.0, 200.0), Align::Right);
        let cells = [
            (0, (104.0, 150.0)),
            (1, (104.0, 120.0)),
            (2, (104.0, 196.0)),
        ];
        assert_eq!(vote(cells.into_iter(), 100.0, 200.0), Align::Left);
        let cells = [(0, (104.0, 196.0)), (1, (104.0, 196.0))];
        assert_eq!(vote(cells.into_iter(), 100.0, 200.0), Align::None);
    }
}
