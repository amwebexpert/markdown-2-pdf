//! Parser stack frames — one variant per open pulldown-cmark construct.

use crate::ir::{Align, Block, ListItem, Span};

pub(super) enum Frame {
    /// Root document, list items, and blockquotes: a plain block container.
    Container(Vec<Block>),
    Paragraph(Vec<Span>),
    Heading(Vec<Span>),
    CodeBlock(String),
    /// Raw HTML block source, kept verbatim (rendered like a code block).
    HtmlBlock(String),
    List {
        ordered: bool,
        start: u64,
        items: Vec<ListItem>,
        /// Task marker of the item being parsed, set by `TaskListMarker`.
        pending_task: Option<bool>,
    },
    Emphasis(Vec<Span>),
    Strong(Vec<Span>),
    Strikethrough(Vec<Span>),
    Link {
        url: String,
        spans: Vec<Span>,
    },
    Image {
        src: String,
        title: String,
        alt: String,
    },
    Table {
        alignments: Vec<Align>,
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    TableRow(Vec<Vec<Span>>),
    TableCell(Vec<Span>),
}
