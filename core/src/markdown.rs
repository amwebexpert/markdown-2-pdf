//! Parses Markdown into the [`crate::ir`] block tree using pulldown-cmark.
//!
//! Only `Options::ENABLE_TABLES` is turned on. Everything else CommonMark can
//! emit without an extension flag (blockquotes, images, thematic breaks, raw
//! HTML) is handled below; anything gated behind an extension we don't enable
//! (strikethrough, task lists, footnotes, ...) simply falls through pulldown-cmark
//! as literal text, which is itself the "best-effort, never fail" fallback.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::ir::{Align, Block, Span};

enum Frame {
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

pub fn parse(input: &str) -> Vec<Block> {
    let parser = Parser::new_ext(input, Options::ENABLE_TABLES);
    let mut stack: Vec<Frame> = vec![Frame::Container(Vec::new())];

    for event in parser {
        match event {
            Event::Start(tag) => {
                // Tight list items skip Paragraph events entirely ("tight
                // paragraphs emit nothing" — pulldown-cmark parse.rs), so
                // inline content arrives directly inside `Tag::Item`'s
                // Container frame. `push_span` can't route text into a
                // Container, so a Paragraph opened implicitly by `push_text`
                // must be closed before any other block-level tag can nest
                // — except another inline container, which belongs inside it.
                if !matches!(tag, Tag::Emphasis | Tag::Strong | Tag::Link { .. } | Tag::Image { .. }) {
                    flush_dangling_paragraph(&mut stack);
                }
                start_tag(&mut stack, tag)
            }
            Event::End(tag_end) => {
                // Only `Item`'s Container can have a dangling implicit
                // paragraph sitting above it — every other End pops a frame
                // that manages its own closing (including `Paragraph`
                // itself, which must NOT be pre-flushed here or its own
                // handler below would pop the wrong frame).
                if matches!(tag_end, TagEnd::Item) {
                    flush_dangling_paragraph(&mut stack);
                }
                end_tag(&mut stack, tag_end)
            }
            Event::Text(text) => push_text(&mut stack, &text),
            Event::Code(text) => push_code(&mut stack, &text),
            Event::Html(text) | Event::InlineHtml(text) => push_text(&mut stack, &text),
            Event::SoftBreak => push_text(&mut stack, " "),
            Event::HardBreak => push_text(&mut stack, "\n"),
            Event::Rule => push_block(&mut stack, Block::ThematicBreak),
            _ => {}
        }
    }

    match stack.pop() {
        Some(Frame::Container(blocks)) => blocks,
        _ => Vec::new(),
    }
}

fn flush_dangling_paragraph(stack: &mut Vec<Frame>) {
    if matches!(stack.last(), Some(Frame::Paragraph(_))) {
        if let Some(Frame::Paragraph(spans)) = stack.pop() {
            push_block(stack, Block::Paragraph { spans });
        }
    }
}

fn start_tag(stack: &mut Vec<Frame>, tag: Tag) {
    match tag {
        Tag::Paragraph => stack.push(Frame::Paragraph(Vec::new())),
        Tag::Heading { .. } => stack.push(Frame::Heading(Vec::new())),
        Tag::BlockQuote(_) => stack.push(Frame::Container(Vec::new())),
        Tag::CodeBlock(_) => stack.push(Frame::CodeBlock(String::new())),
        Tag::List(start) => stack.push(Frame::List {
            ordered: start.is_some(),
            start: start.unwrap_or(1),
            items: Vec::new(),
        }),
        Tag::Item => stack.push(Frame::Container(Vec::new())),
        Tag::Table(alignments) => stack.push(Frame::Table {
            alignments: alignments.into_iter().map(map_alignment).collect(),
            header: Vec::new(),
            rows: Vec::new(),
        }),
        // TableHead has no distinct frame: its cells are direct children of
        // Table (pulldown-cmark doesn't wrap the header in a TableRow the
        // way it does for body rows), so TagEnd::TableCell routes them into
        // `Table.header` by checking what's actually on top of the stack.
        Tag::TableRow => stack.push(Frame::TableRow(Vec::new())),
        Tag::TableCell => stack.push(Frame::TableCell(Vec::new())),
        Tag::Emphasis => stack.push(Frame::Emphasis(Vec::new())),
        Tag::Strong => stack.push(Frame::Strong(Vec::new())),
        Tag::Link { dest_url, .. } => stack.push(Frame::Link {
            url: dest_url.into_string(),
            spans: Vec::new(),
        }),
        Tag::Image { .. } => stack.push(Frame::Image { alt: String::new() }),
        // HtmlBlock content arrives as Event::Html while this frame is on top;
        // out of the requested feature scope, so it's dropped rather than rendered.
        Tag::HtmlBlock => stack.push(Frame::Container(Vec::new())),
        _ => {}
    }
}

fn end_tag(stack: &mut Vec<Frame>, tag_end: TagEnd) {
    match tag_end {
        TagEnd::Paragraph => {
            if let Some(Frame::Paragraph(spans)) = stack.pop() {
                push_block(stack, Block::Paragraph { spans });
            }
        }
        TagEnd::Heading(level) => {
            if let Some(Frame::Heading(spans)) = stack.pop() {
                push_block(stack, Block::Heading { level: heading_level(level), spans });
            }
        }
        TagEnd::BlockQuote(_) => {
            if let Some(Frame::Container(blocks)) = stack.pop() {
                push_block(stack, Block::BlockQuote(blocks));
            }
        }
        TagEnd::CodeBlock => {
            if let Some(Frame::CodeBlock(text)) = stack.pop() {
                push_block(stack, Block::CodeBlock { text });
            }
        }
        TagEnd::List(_) => {
            if let Some(Frame::List { ordered, start, items }) = stack.pop() {
                push_block(stack, Block::List { ordered, start, items });
            }
        }
        TagEnd::Item => {
            if let Some(Frame::Container(blocks)) = stack.pop() {
                if let Some(Frame::List { items, .. }) = stack.last_mut() {
                    items.push(blocks);
                }
            }
        }
        TagEnd::Table => {
            if let Some(Frame::Table { alignments, header, rows }) = stack.pop() {
                push_block(stack, Block::Table { alignments, header, rows });
            }
        }
        TagEnd::TableRow => {
            if let Some(Frame::TableRow(cells)) = stack.pop() {
                if let Some(Frame::Table { rows, .. }) = stack.last_mut() {
                    rows.push(cells);
                }
            }
        }
        TagEnd::TableCell => {
            if let Some(Frame::TableCell(spans)) = stack.pop() {
                match stack.last_mut() {
                    Some(Frame::TableRow(cells)) => cells.push(spans),
                    Some(Frame::Table { header, .. }) => header.push(spans),
                    _ => {}
                }
            }
        }
        TagEnd::Emphasis => {
            if let Some(Frame::Emphasis(spans)) = stack.pop() {
                extend_spans(stack, spans, |s| s.italic = true);
            }
        }
        TagEnd::Strong => {
            if let Some(Frame::Strong(spans)) = stack.pop() {
                extend_spans(stack, spans, |s| s.bold = true);
            }
        }
        TagEnd::Link => {
            if let Some(Frame::Link { url, spans }) = stack.pop() {
                extend_spans(stack, spans, |s| s.link = Some(url.clone()));
            }
        }
        TagEnd::Image => {
            if let Some(Frame::Image { alt }) = stack.pop() {
                push_span(stack, Span::plain(format!("[image: {alt}]")));
            }
        }
        TagEnd::HtmlBlock => {
            stack.pop();
        }
        _ => {}
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    level as u8
}

fn map_alignment(a: pulldown_cmark::Alignment) -> Align {
    match a {
        pulldown_cmark::Alignment::None => Align::None,
        pulldown_cmark::Alignment::Left => Align::Left,
        pulldown_cmark::Alignment::Center => Align::Center,
        pulldown_cmark::Alignment::Right => Align::Right,
    }
}

/// Applies `set_flag` to every span in `spans`, then hands them to whatever
/// span-accumulating frame is now on top of the stack (closing an inline
/// container folds its styling into its parent's accumulator).
fn extend_spans(stack: &mut Vec<Frame>, mut spans: Vec<Span>, set_flag: impl Fn(&mut Span)) {
    for span in &mut spans {
        set_flag(span);
    }
    for span in spans {
        push_span(stack, span);
    }
}

fn push_span(stack: &mut Vec<Frame>, span: Span) {
    // Tight list items have no `Tag::Paragraph` wrapper at all (pulldown-cmark
    // emits neither Start nor End for one — see parse.rs's `TightParagraph`
    // handling), so the first bit of inline content seen directly under a
    // `Container` implicitly opens one — closed again by
    // `flush_dangling_paragraph` at the next block boundary.
    if matches!(stack.last(), Some(Frame::Container(_))) {
        stack.push(Frame::Paragraph(Vec::new()));
    }
    match stack.last_mut() {
        Some(Frame::Paragraph(spans))
        | Some(Frame::Heading(spans))
        | Some(Frame::Emphasis(spans))
        | Some(Frame::Strong(spans))
        | Some(Frame::Link { spans, .. })
        | Some(Frame::TableCell(spans)) => spans.push(span),
        _ => {}
    }
}

fn push_block(stack: &mut Vec<Frame>, block: Block) {
    if let Some(Frame::Container(blocks)) = stack.last_mut() {
        blocks.push(block);
    }
}

fn push_text(stack: &mut Vec<Frame>, text: &str) {
    match stack.last_mut() {
        Some(Frame::CodeBlock(buf)) => buf.push_str(text),
        Some(Frame::Image { alt }) => alt.push_str(text),
        _ => push_span(stack, Span::plain(text.to_string())),
    }
}

fn push_code(stack: &mut Vec<Frame>, text: &str) {
    match stack.last_mut() {
        Some(Frame::Image { alt }) => alt.push_str(text),
        _ => push_span(
            stack,
            Span {
                text: text.to_string(),
                code: true,
                ..Default::default()
            },
        ),
    }
}
