//! Opens stack frames for pulldown-cmark `Tag::Start` events.

use pulldown_cmark::Tag;

use super::align::map_alignment;
use super::frame::Frame;

pub(super) fn open_tag(stack: &mut Vec<Frame>, tag: Tag) {
    match tag {
        Tag::Paragraph => open_paragraph(stack),
        Tag::Heading { .. } => open_heading(stack),
        Tag::BlockQuote(_) => open_block_quote(stack),
        Tag::CodeBlock(_) => open_code_block(stack),
        Tag::List(start) => open_list(stack, start),
        Tag::Item => open_item(stack),
        Tag::Table(alignments) => open_table(stack, alignments),
        Tag::TableRow => open_table_row(stack),
        Tag::TableCell => open_table_cell(stack),
        Tag::Emphasis => open_emphasis(stack),
        Tag::Strong => open_strong(stack),
        Tag::Link { dest_url, .. } => open_link(stack, dest_url.into_string()),
        Tag::Image { .. } => open_image(stack),
        Tag::HtmlBlock => open_html_block(stack),
        _ => {}
    }
}

fn open_paragraph(stack: &mut Vec<Frame>) {
    stack.push(Frame::Paragraph(Vec::new()));
}

fn open_heading(stack: &mut Vec<Frame>) {
    stack.push(Frame::Heading(Vec::new()));
}

fn open_block_quote(stack: &mut Vec<Frame>) {
    stack.push(Frame::Container(Vec::new()));
}

fn open_code_block(stack: &mut Vec<Frame>) {
    stack.push(Frame::CodeBlock(String::new()));
}

fn open_list(stack: &mut Vec<Frame>, start: Option<u64>) {
    stack.push(Frame::List {
        ordered: start.is_some(),
        start: start.unwrap_or(1),
        items: Vec::new(),
    });
}

fn open_item(stack: &mut Vec<Frame>) {
    stack.push(Frame::Container(Vec::new()));
}

fn open_table(stack: &mut Vec<Frame>, alignments: Vec<pulldown_cmark::Alignment>) {
    // TableHead has no distinct frame: its cells are direct children of
    // Table (pulldown-cmark doesn't wrap the header in a TableRow the
    // way it does for body rows), so TagEnd::TableCell routes them into
    // `Table.header` by checking what's actually on top of the stack.
    stack.push(Frame::Table {
        alignments: alignments.into_iter().map(map_alignment).collect(),
        header: Vec::new(),
        rows: Vec::new(),
    });
}

fn open_table_row(stack: &mut Vec<Frame>) {
    stack.push(Frame::TableRow(Vec::new()));
}

fn open_table_cell(stack: &mut Vec<Frame>) {
    stack.push(Frame::TableCell(Vec::new()));
}

fn open_emphasis(stack: &mut Vec<Frame>) {
    stack.push(Frame::Emphasis(Vec::new()));
}

fn open_strong(stack: &mut Vec<Frame>) {
    stack.push(Frame::Strong(Vec::new()));
}

fn open_link(stack: &mut Vec<Frame>, url: String) {
    stack.push(Frame::Link {
        url,
        spans: Vec::new(),
    });
}

fn open_image(stack: &mut Vec<Frame>) {
    stack.push(Frame::Image { alt: String::new() });
}

fn open_html_block(stack: &mut Vec<Frame>) {
    // HtmlBlock content arrives as Event::Html while this frame is on top;
    // out of the requested feature scope, so it's dropped rather than rendered.
    stack.push(Frame::Container(Vec::new()));
}
