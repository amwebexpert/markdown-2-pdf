//! Closes stack frames for pulldown-cmark `TagEnd` events.

use pulldown_cmark::{HeadingLevel, TagEnd};

use crate::ir::{Block, Span};

use super::align::heading_level;
use super::frame::Frame;
use super::stack::{extend_spans, push_block, push_span};

pub(super) fn close_tag(stack: &mut Vec<Frame>, tag_end: TagEnd) {
    match tag_end {
        TagEnd::Paragraph => close_paragraph(stack),
        TagEnd::Heading(level) => close_heading(stack, level),
        TagEnd::BlockQuote(_) => close_block_quote(stack),
        TagEnd::CodeBlock => close_code_block(stack),
        TagEnd::List(_) => close_list(stack),
        TagEnd::Item => close_item(stack),
        TagEnd::Table => close_table(stack),
        TagEnd::TableRow => close_table_row(stack),
        TagEnd::TableCell => close_table_cell(stack),
        TagEnd::Emphasis => close_emphasis(stack),
        TagEnd::Strong => close_strong(stack),
        TagEnd::Link => close_link(stack),
        TagEnd::Image => close_image(stack),
        TagEnd::HtmlBlock => close_html_block(stack),
        _ => {}
    }
}

fn close_paragraph(stack: &mut Vec<Frame>) {
    if let Some(Frame::Paragraph(spans)) = stack.pop() {
        push_block(stack, Block::Paragraph { spans });
    }
}

fn close_heading(stack: &mut Vec<Frame>, level: HeadingLevel) {
    if let Some(Frame::Heading(spans)) = stack.pop() {
        push_block(
            stack,
            Block::Heading {
                level: heading_level(level),
                spans,
            },
        );
    }
}

fn close_block_quote(stack: &mut Vec<Frame>) {
    if let Some(Frame::Container(blocks)) = stack.pop() {
        push_block(stack, Block::BlockQuote(blocks));
    }
}

fn close_code_block(stack: &mut Vec<Frame>) {
    if let Some(Frame::CodeBlock(text)) = stack.pop() {
        push_block(stack, Block::CodeBlock { text });
    }
}

fn close_list(stack: &mut Vec<Frame>) {
    if let Some(Frame::List {
        ordered,
        start,
        items,
    }) = stack.pop()
    {
        push_block(
            stack,
            Block::List {
                ordered,
                start,
                items,
            },
        );
    }
}

fn close_item(stack: &mut Vec<Frame>) {
    if let Some(Frame::Container(blocks)) = stack.pop()
        && let Some(Frame::List { items, .. }) = stack.last_mut()
    {
        items.push(blocks);
    }
}

fn close_table(stack: &mut Vec<Frame>) {
    if let Some(Frame::Table {
        alignments,
        header,
        rows,
    }) = stack.pop()
    {
        push_block(
            stack,
            Block::Table {
                alignments,
                header,
                rows,
            },
        );
    }
}

fn close_table_row(stack: &mut Vec<Frame>) {
    if let Some(Frame::TableRow(cells)) = stack.pop()
        && let Some(Frame::Table { rows, .. }) = stack.last_mut()
    {
        rows.push(cells);
    }
}

fn close_table_cell(stack: &mut Vec<Frame>) {
    if let Some(Frame::TableCell(spans)) = stack.pop() {
        attach_table_cell(stack, spans);
    }
}

fn attach_table_cell(stack: &mut [Frame], spans: Vec<Span>) {
    match stack.last_mut() {
        Some(Frame::TableRow(cells)) => cells.push(spans),
        Some(Frame::Table { header, .. }) => header.push(spans),
        _ => {}
    }
}

fn close_emphasis(stack: &mut Vec<Frame>) {
    if let Some(Frame::Emphasis(spans)) = stack.pop() {
        extend_spans(stack, spans, |s| s.italic = true);
    }
}

fn close_strong(stack: &mut Vec<Frame>) {
    if let Some(Frame::Strong(spans)) = stack.pop() {
        extend_spans(stack, spans, |s| s.bold = true);
    }
}

fn close_link(stack: &mut Vec<Frame>) {
    if let Some(Frame::Link { url, spans }) = stack.pop() {
        extend_spans(stack, spans, |s| s.link = Some(url.clone()));
    }
}

fn close_image(stack: &mut Vec<Frame>) {
    if let Some(Frame::Image { alt }) = stack.pop() {
        push_span(stack, Span::plain(format!("[image: {alt}]")));
    }
}

fn close_html_block(stack: &mut Vec<Frame>) {
    stack.pop();
}
