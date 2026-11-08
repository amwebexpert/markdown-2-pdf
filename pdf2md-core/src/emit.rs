//! GFM writer: blocks → Markdown text.

use std::fmt::Write as _;
use std::rc::Rc;

use crate::blocks::{Block, Run};
use crate::font::FontStyle;

/// Marker x positions closer than this (pt) share one list nesting level.
const LEVEL_TOLERANCE: f32 = 4.0;

pub fn markdown(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(block) = blocks.get(i) {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        match block {
            Block::Heading { level, runs } => {
                let all_bold = runs
                    .iter()
                    .all(|r| r.style.bold || r.text.trim().is_empty());
                let _ = write!(
                    out,
                    "{} {}",
                    "#".repeat(*level),
                    inline(runs, all_bold).trim()
                );
            }
            Block::Paragraph(runs) => out.push_str(&escape_line_start(inline(runs, false).trim())),
            Block::Code(text) => out.push_str(&fenced(text)),
            Block::Comment(text) => {
                let _ = write!(out, "<!-- {} -->", text.replace("--", "- -"));
            }
            Block::ListItem { .. } => {
                let end = blocks
                    .get(i..)
                    .unwrap_or_default()
                    .iter()
                    .position(|b| !matches!(b, Block::ListItem { .. }))
                    .map_or(blocks.len(), |n| i + n);
                out.push_str(&list(blocks.get(i..end).unwrap_or_default()));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn list(items: &[Block]) -> String {
    let mut levels: Vec<f32> = items
        .iter()
        .filter_map(|b| match b {
            Block::ListItem { marker_x, .. } => Some(*marker_x),
            _ => None,
        })
        .collect();
    levels.sort_by(f32::total_cmp);
    levels.dedup_by(|b, a| (*b - *a).abs() < LEVEL_TOLERANCE);

    // Content column of each open nesting level.
    let mut columns: Vec<usize> = Vec::new();
    let mut lines = Vec::new();
    for item in items {
        let Block::ListItem {
            number,
            marker_x,
            runs,
        } = item
        else {
            continue;
        };
        let level = levels
            .iter()
            .position(|x| (marker_x - x).abs() < LEVEL_TOLERANCE)
            .unwrap_or(0)
            .min(columns.len());
        columns.truncate(level);
        let indent = columns.last().copied().unwrap_or(0);
        let marker = number.map_or_else(|| "-".to_string(), |n| format!("{n}."));
        columns.push(indent + marker.len() + 1);
        lines.push(format!(
            "{}{marker} {}",
            " ".repeat(indent),
            inline(runs, false).trim()
        ));
    }
    lines.join("\n")
}

fn fenced(text: &str) -> String {
    let longest = longest_run(text, '`');
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{text}\n{fence}")
}

fn longest_run(text: &str, c: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in text.chars() {
        current = if ch == c { current + 1 } else { 0 };
        longest = longest.max(current);
    }
    longest
}

/// Backslash-escapes characters CommonMark would read as inline syntax.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escapes a paragraph opening that would otherwise parse as a block construct
/// (heading, quote, list item, thematic break).
fn escape_line_start(text: &str) -> String {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let after_digits = text.get(digits..).unwrap_or_default();
    if digits > 0
        && digits <= 9
        && (after_digits.starts_with(". ") || after_digits.starts_with(") "))
    {
        let (number, rest) = text.split_at(digits);
        return format!("{number}\\{rest}");
    }
    let block_start = text.starts_with('#')
        || text.starts_with('>')
        || text.starts_with("- ")
        || text.starts_with("+ ")
        || text.starts_with("---")
        || text.starts_with("===");
    if block_start {
        format!("\\{text}")
    } else {
        text.to_string()
    }
}

fn code_span(text: &str) -> String {
    let fence = "`".repeat(longest_run(text, '`') + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

/// Wraps `inner` in `marker`, keeping surrounding whitespace outside it
/// (`** bold**` would not parse as emphasis).
fn wrap(inner: &str, marker: &str) -> String {
    let core = inner.trim();
    if core.is_empty() {
        return inner.to_string();
    }
    let start = inner.len() - inner.trim_start().len();
    let lead = inner.get(..start).unwrap_or_default();
    let trail = inner.get(start + core.len()..).unwrap_or_default();
    format!("{lead}{marker}{core}{marker}{trail}")
}

fn link_target(url: &str) -> String {
    if url.contains([' ', '(', ')', '<', '>']) {
        format!(
            "<{}>",
            url.replace(' ', "%20")
                .replace('<', "%3C")
                .replace('>', "%3E")
        )
    } else {
        url.to_string()
    }
}

/// Renders runs as inline Markdown, nesting link ⊃ bold ⊃ italic ⊃ code.
fn inline(runs: &[Run], strip_bold: bool) -> String {
    let runs: Vec<Run> = runs
        .iter()
        .map(|r| Run {
            style: FontStyle {
                bold: r.style.bold && !strip_bold,
                ..r.style
            },
            ..r.clone()
        })
        .collect();

    group_by(&runs, |r| r.link.clone())
        .into_iter()
        .map(|(link, group)| {
            let inner = render_bold(group);
            match link {
                Some(url) => link_markdown(&inner, &url, group),
                None => inner,
            }
        })
        .collect()
}

fn link_markdown(inner: &str, url: &Rc<str>, group: &[Run]) -> String {
    let plain: String = group.iter().map(|r| r.text.as_str()).collect();
    if plain.trim() == url.as_ref() && !url.contains(' ') {
        return format!("<{url}>");
    }
    let core = inner.trim();
    let start = inner.len() - inner.trim_start().len();
    let lead = inner.get(..start).unwrap_or_default();
    let trail = inner.get(start + core.len()..).unwrap_or_default();
    format!("{lead}[{core}]({}){trail}", link_target(url))
}

fn render_bold(runs: &[Run]) -> String {
    group_by(runs, |r| r.style.bold && !r.text.trim().is_empty())
        .into_iter()
        .map(|(bold, group)| {
            let inner = render_italic(group);
            if bold { wrap(&inner, "**") } else { inner }
        })
        .collect()
}

fn render_italic(runs: &[Run]) -> String {
    group_by(runs, |r| r.style.italic && !r.text.trim().is_empty())
        .into_iter()
        .map(|(italic, group)| {
            let inner: String = group
                .iter()
                .map(|r| {
                    if r.style.mono && !r.text.trim().is_empty() {
                        let core = r.text.trim();
                        let start = r.text.len() - r.text.trim_start().len();
                        let lead = r.text.get(..start).unwrap_or_default();
                        let trail = r.text.get(start + core.len()..).unwrap_or_default();
                        format!("{lead}{}{trail}", code_span(core))
                    } else {
                        escape(&r.text)
                    }
                })
                .collect();
            if italic { wrap(&inner, "*") } else { inner }
        })
        .collect()
}

/// Groups consecutive runs by key. Whitespace-only runs join the current group
/// so a space between two bold words doesn't split the emphasis.
fn group_by<K: PartialEq + Clone>(runs: &[Run], key: impl Fn(&Run) -> K) -> Vec<(K, &[Run])> {
    let mut groups: Vec<(K, usize, usize)> = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let k = key(run);
        let blank = run.text.trim().is_empty();
        match groups.last_mut() {
            Some((current, _, end)) if *current == k || blank => *end = i + 1,
            _ => groups.push((k, i, i + 1)),
        }
    }
    groups
        .into_iter()
        .filter_map(|(k, start, end)| Some((k, runs.get(start..end)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, bold: bool, italic: bool, mono: bool) -> Run {
        Run {
            text: text.to_string(),
            style: FontStyle { bold, italic, mono },
            link: None,
        }
    }

    #[test]
    fn emphasis_keeps_spaces_outside_markers() {
        let runs = [
            run("a ", false, false, false),
            run("bold", true, false, false),
            run(" ", false, false, false),
            run("words", true, false, false),
            run(" and ", false, false, false),
            run("code", false, false, true),
        ];
        assert_eq!(inline(&runs, false), "a **bold words** and `code`");
    }

    #[test]
    fn nested_bold_italic() {
        let runs = [
            run("foo ", true, false, false),
            run("bar", true, true, false),
        ];
        assert_eq!(inline(&runs, false), "**foo *bar***");
    }

    #[test]
    fn links_and_autolinks() {
        let mut text = run("site", false, false, false);
        text.link = Some(Rc::from("https://example.com"));
        let mut bare = run("https://example.com", false, false, false);
        bare.link = Some(Rc::from("https://example.com"));
        assert_eq!(inline(&[text], false), "[site](https://example.com)");
        assert_eq!(inline(&[bare], false), "<https://example.com>");
    }

    #[test]
    fn special_characters_are_escaped() {
        assert_eq!(
            inline(&[run("a*b_c [d]", false, false, false)], false),
            r"a\*b\_c \[d\]"
        );
        assert_eq!(escape_line_start("# not a heading"), r"\# not a heading");
        assert_eq!(escape_line_start("1. not a list"), r"1\. not a list");
    }

    #[test]
    fn nested_lists_indent_under_their_parent_content() {
        let item = |number, marker_x, text: &str| Block::ListItem {
            number,
            marker_x,
            runs: vec![run(text, false, false, false)],
        };
        let blocks = [
            item(Some(1), 72.0, "one"),
            item(None, 90.0, "child"),
            item(Some(2), 72.0, "two"),
        ];
        assert_eq!(markdown(&blocks), "1. one\n   - child\n2. two\n");
    }

    #[test]
    fn code_fence_outgrows_inner_backticks() {
        assert_eq!(fenced("a ``` b"), "````\na ``` b\n````");
    }
}
