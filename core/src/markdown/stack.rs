//! Stack mutations shared by event handling (text, blocks, implicit paragraphs).

use crate::ir::{Block, Span};

use super::frame::Frame;

pub(super) fn flush_dangling_paragraph(stack: &mut Vec<Frame>) {
    if matches!(stack.last(), Some(Frame::Paragraph(_)))
        && let Some(Frame::Paragraph(spans)) = stack.pop()
    {
        push_block(stack, Block::Paragraph { spans });
    }
}

pub(super) fn extend_spans(
    stack: &mut Vec<Frame>,
    mut spans: Vec<Span>,
    set_flag: impl Fn(&mut Span),
) {
    for span in &mut spans {
        set_flag(span);
    }
    for span in spans {
        push_span(stack, span);
    }
}

pub(super) fn push_span(stack: &mut Vec<Frame>, span: Span) {
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

pub(super) fn push_block(stack: &mut [Frame], block: Block) {
    if let Some(Frame::Container(blocks)) = stack.last_mut() {
        blocks.push(block);
    }
}

pub(super) fn push_text(stack: &mut Vec<Frame>, text: &str) {
    match stack.last_mut() {
        Some(Frame::CodeBlock(buf)) => buf.push_str(text),
        Some(Frame::Image { alt }) => alt.push_str(text),
        _ => push_span(stack, Span::plain(text.to_string())),
    }
}

pub(super) fn push_code(stack: &mut Vec<Frame>, text: &str) {
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
