//! Parser stack frames — one variant per open pulldown-cmark construct.

use crate::ir::{Align, Block, Span};

pub(super) enum Frame {
    /// Root document, list items, and blockquotes: a plain block container.
    Container(Vec<Block>),
    Paragraph(Vec<Span>),
    Heading(Vec<Span>),
    CodeBlock(String),
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<Block>>,
    },
    Emphasis(Vec<Span>),
    Strong(Vec<Span>),
    Link {
        url: String,
        spans: Vec<Span>,
    },
    Image {
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
