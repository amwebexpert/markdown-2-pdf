//! Tables split across pages: a table ending near the bottom of one page and
//! a table with the same columns starting near the top of the next are one
//! logical table. Merged conservatively; a repeated header row is dropped.

use crate::blocks::PageItems;
use crate::layout::Item;
use crate::tables::{Cell, Span, Table};

/// A table's bottom (top) must lie within this share of the page height from
/// the page bottom (top) for it to continue (be continued).
const EDGE_SHARE: f32 = 0.2;
/// Column boundaries match within this many pt, or this share of the width.
const BOUNDARY_PT: f32 = 3.0;
const BOUNDARY_SHARE: f32 = 0.02;
/// Only tables of at least this many columns are merged.
const MIN_COLUMNS: usize = 2;

/// Merges continued tables in place and returns a warning per merge.
/// `boxes` holds each page's bounding box, `None` for failed pages.
pub fn merge_across_pages(pages: &mut [PageItems], boxes: &[Option<[f32; 4]>]) -> Vec<String> {
    let mut warnings = Vec::new();
    // The table that may continue onto the next page: (page, item) indices.
    let mut open: Option<(usize, usize)> = None;
    for index in 0..pages.len() {
        let Some(page_box) = boxes.get(index).copied().flatten() else {
            open = None;
            continue;
        };
        let continues = open.is_some_and(|(page, item)| {
            match (table_at(pages, page, item), table_at(pages, index, 0)) {
                (Some(open), Some(next)) => near_top(next, page_box) && same_columns(open, next),
                _ => false,
            }
        });
        if continues
            && let Some((page, item)) = open
            && let Some(PageItems::Items(items)) = pages.get_mut(index)
            && let Item::Table { table: next, .. } = items.remove(0)
        {
            let next_ends_low = near_bottom(&next, page_box);
            let page_done = items.is_empty();
            if let Some(PageItems::Items(items)) = pages.get_mut(page)
                && let Some(Item::Table { table, .. }) = items.get_mut(item)
            {
                append(table, next);
            }
            warnings.push(format!(
                "page {}: table continued from the previous page (merged)",
                index + 1
            ));
            // The table filled this page: it may go on over the next one.
            if page_done {
                open = open.filter(|_| next_ends_low);
                continue;
            }
        }
        open = match pages.get(index) {
            Some(PageItems::Items(items)) => match items.last() {
                Some(Item::Table { table, .. }) if near_bottom(table, page_box) => {
                    Some((index, items.len() - 1))
                }
                _ => None,
            },
            _ => None,
        };
    }
    warnings
}

fn table_at(pages: &[PageItems], page: usize, item: usize) -> Option<&Table> {
    match pages.get(page)? {
        PageItems::Items(items) => match items.get(item)? {
            Item::Table { table, .. } => Some(table),
            _ => None,
        },
        PageItems::Failed(_) => None,
    }
}

fn near_bottom(table: &Table, [_, y0, _, y1]: [f32; 4]) -> bool {
    table.bbox[1] - y0 <= EDGE_SHARE * (y1 - y0)
}

fn near_top(table: &Table, [_, y0, _, y1]: [f32; 4]) -> bool {
    y1 - table.bbox[3] <= EDGE_SHARE * (y1 - y0)
}

fn same_columns(a: &Table, b: &Table) -> bool {
    let width = a.bbox[2] - a.bbox[0];
    let tolerance = BOUNDARY_PT.max(BOUNDARY_SHARE * width);
    a.columns.len() == b.columns.len()
        && a.columns.len() > MIN_COLUMNS // one boundary more than columns
        && a.columns
            .iter()
            .zip(&b.columns)
            .all(|(x, y)| (x - y).abs() <= tolerance)
}

/// Text of a row, whitespace and case ignored, to spot a repeated header.
fn row_key(row: &[Cell]) -> String {
    row.iter()
        .flatten()
        .flatten()
        .flat_map(|g| g.text.chars())
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn append(open: &mut Table, mut next: Table) {
    let repeats_header = match (open.rows.first(), next.rows.first()) {
        (Some(header), Some(first)) => row_key(header) == row_key(first),
        _ => false,
    };
    let dropped = usize::from(repeats_header);
    if repeats_header {
        next.rows.remove(0);
    }
    let offset = open.rows.len();
    open.spans.extend(
        next.spans
            .into_iter()
            .filter(|span| span.row >= dropped)
            .map(|span| Span {
                row: span.row - dropped + offset,
                ..span
            }),
    );
    open.rows.extend(next.rows);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontStyle;
    use crate::interp::Glyph;
    use crate::tables::{Align, Evidence};

    const PAGE: [f32; 4] = [0.0, 0.0, 612.0, 792.0];

    fn cell(text: &str) -> Cell {
        vec![
            text.chars()
                .enumerate()
                .map(|(i, c)| Glyph {
                    text: c.to_string(),
                    x: i as f32 * 5.0,
                    y: 0.0,
                    width: 5.0,
                    size: 10.0,
                    style: FontStyle::default(),
                    link: None,
                    synthetic: false,
                })
                .collect(),
        ]
    }

    /// A table spanning `top..bottom` (PDF y) with the given column bounds.
    fn table(columns: &[f32], top: f32, bottom: f32, rows: &[&[&str]]) -> Item {
        Item::Table {
            page: 0,
            table: Table {
                bbox: [columns[0], bottom, columns[columns.len() - 1], top],
                columns: columns.to_vec(),
                align: vec![Align::None; columns.len() - 1],
                rows: rows
                    .iter()
                    .map(|r| r.iter().map(|t| cell(t)).collect())
                    .collect(),
                spans: Vec::new(),
                evidence: Evidence::Ruled,
            },
        }
    }

    fn texts(item: &Item) -> Vec<String> {
        match item {
            Item::Table { table, .. } => table
                .rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| {
                            c.iter()
                                .flatten()
                                .map(|g| g.text.as_str())
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect(),
            _ => panic!("not a table"),
        }
    }

    fn pages(items: Vec<Vec<Item>>) -> Vec<PageItems> {
        items.into_iter().map(PageItems::Items).collect()
    }

    fn boxes(n: usize) -> Vec<Option<[f32; 4]>> {
        vec![Some(PAGE); n]
    }

    const COLS: &[f32] = &[72.0, 200.0, 300.0];

    #[test]
    fn continued_table_is_merged_and_its_repeated_header_dropped() {
        let mut p = pages(vec![
            vec![table(COLS, 600.0, 80.0, &[&["Name", "Value"], &["a", "1"]])],
            vec![table(COLS, 720.0, 80.0, &[&["Name", "Value"], &["b", "2"]])],
            vec![table(
                COLS,
                720.0,
                500.0,
                &[&["Name", "Value"], &["c", "3"]],
            )],
        ]);
        let warnings = merge_across_pages(&mut p, &boxes(3));
        let PageItems::Items(first) = &p[0] else {
            panic!()
        };
        assert_eq!(texts(&first[0]), ["Name|Value", "a|1", "b|2", "c|3"]);
        assert!(matches!(&p[1], PageItems::Items(i) if i.is_empty()));
        assert!(matches!(&p[2], PageItems::Items(i) if i.is_empty()));
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn continuation_without_a_repeated_header_keeps_all_rows() {
        let mut p = pages(vec![
            vec![table(COLS, 600.0, 80.0, &[&["Name", "Value"], &["a", "1"]])],
            vec![table(COLS, 720.0, 600.0, &[&["b", "2"]])],
        ]);
        merge_across_pages(&mut p, &boxes(2));
        let PageItems::Items(first) = &p[0] else {
            panic!()
        };
        assert_eq!(texts(&first[0]), ["Name|Value", "a|1", "b|2"]);
    }

    #[test]
    fn same_column_count_but_different_bounds_stays_separate() {
        let mut p = pages(vec![
            vec![table(COLS, 600.0, 80.0, &[&["A", "B"]])],
            vec![table(&[72.0, 120.0, 300.0], 720.0, 600.0, &[&["C", "D"]])],
        ]);
        assert!(merge_across_pages(&mut p, &boxes(2)).is_empty());
        assert!(matches!(&p[1], PageItems::Items(i) if i.len() == 1));
    }

    #[test]
    fn table_ending_mid_page_or_starting_low_stays_separate() {
        let mid = pages(vec![
            vec![table(COLS, 600.0, 400.0, &[&["A", "B"]])],
            vec![table(COLS, 720.0, 600.0, &[&["C", "D"]])],
        ]);
        let low = pages(vec![
            vec![table(COLS, 600.0, 80.0, &[&["A", "B"]])],
            vec![table(COLS, 400.0, 300.0, &[&["C", "D"]])],
        ]);
        for mut p in [mid, low] {
            assert!(merge_across_pages(&mut p, &boxes(2)).is_empty());
        }
    }

    #[test]
    fn text_between_the_tables_prevents_a_merge() {
        let mut p = pages(vec![
            vec![table(COLS, 600.0, 80.0, &[&["A", "B"]])],
            vec![
                Item::Rule { page: 1 },
                table(COLS, 700.0, 600.0, &[&["C", "D"]]),
            ],
        ]);
        assert!(merge_across_pages(&mut p, &boxes(2)).is_empty());
    }

    #[test]
    fn a_failed_page_breaks_the_chain() {
        let mut p = pages(vec![vec![table(COLS, 600.0, 80.0, &[&["A", "B"]])]]);
        p.push(PageItems::Failed(2));
        p.extend(pages(vec![vec![table(COLS, 720.0, 600.0, &[&["C", "D"]])]]));
        let mut b = boxes(3);
        b[1] = None;
        assert!(merge_across_pages(&mut p, &b).is_empty());
    }
}
