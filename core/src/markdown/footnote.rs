//! GFM footnotes, rendered endnote-style: references become superscript
//! numbers (first-reference order) and every definition is moved to the end
//! of the document, so none is lost whatever its position.

use crate::ir::{Block, ListItem, Span};

#[derive(Default)]
pub(super) struct Footnotes {
    /// Normalized labels in first-reference order; index + 1 = number.
    referenced: Vec<String>,
    /// Definitions in source order.
    definitions: Vec<(String, Vec<Block>)>,
    /// Labels of the definitions currently open (normally at most one).
    open: Vec<String>,
}

/// Footnote labels match case-insensitively (`[^A]` refers to `[^a]`).
fn normalize(label: &str) -> String {
    label.to_lowercase()
}

impl Footnotes {
    pub(super) fn reference(&mut self, label: &str) -> Span {
        let label = normalize(label);
        let index = if let Some(index) = self.referenced.iter().position(|l| *l == label) {
            index
        } else {
            self.referenced.push(label);
            self.referenced.len() - 1
        };
        Span {
            text: (index + 1).to_string(),
            superscript: true,
            ..Default::default()
        }
    }

    pub(super) fn open_definition(&mut self, label: &str) {
        self.open.push(normalize(label));
    }

    pub(super) fn close_definition(&mut self, blocks: Vec<Block>) {
        if let Some(label) = self.open.pop() {
            self.definitions.push((label, blocks));
        }
    }

    /// A rule followed by an ordered list: referenced notes by number, then
    /// definitions nobody referenced, numbered on in source order.
    pub(super) fn into_endnotes(mut self) -> Vec<Block> {
        if self.definitions.is_empty() {
            return Vec::new();
        }
        let rank = |label: &str| {
            self.referenced
                .iter()
                .position(|l| l == label)
                .unwrap_or(usize::MAX)
        };
        // Stable sort: unreferenced (MAX) keep their source order.
        let mut definitions = std::mem::take(&mut self.definitions);
        definitions.sort_by_key(|(label, _)| rank(label));
        let items = definitions
            .into_iter()
            .map(|(_, blocks)| ListItem { task: None, blocks })
            .collect();
        vec![
            Block::ThematicBreak,
            Block::List {
                ordered: true,
                start: 1,
                items,
            },
        ]
    }
}
