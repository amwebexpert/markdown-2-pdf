//! Stack mutations shared by event handling (text, blocks, implicit paragraphs).

use crate::ir::{Block, Image, Span};

use super::frame::Frame;

pub(super) fn flush_dangling_paragraph(stack: &mut Vec<Frame>) {
    if matches!(stack.last(), Some(Frame::Paragraph(_)))
        && let Some(Frame::Paragraph(spans)) = stack.pop()
    {
        push_paragraph(stack, spans);
    }
}

/// Pushes a finished paragraph. One holding nothing but images (and
/// whitespace) becomes one [`Block::Image`] per image instead; an image mixed
/// with text keeps its `[image: alt]` fallback span.
pub(super) fn push_paragraph(stack: &mut [Frame], spans: Vec<Span>) {
    let only_images = spans.iter().any(|s| s.image.is_some())
        && spans
            .iter()
            .all(|s| s.image.is_some() || s.text.trim().is_empty());
    if !only_images {
        push_block(stack, Block::Paragraph { spans });
        return;
    }
    for span in spans {
        if let Some(image) = span.image {
            push_block(
                stack,
                Block::Image(Image {
                    link: span.link,
                    ..image
                }),
            );
        }
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
    if let Some(
        Frame::Paragraph(spans)
        | Frame::Heading(spans)
        | Frame::Emphasis(spans)
        | Frame::Strong(spans)
        | Frame::Strikethrough(spans)
        | Frame::Link { spans, .. }
        | Frame::TableCell(spans),
    ) = stack.last_mut()
    {
        spans.push(span);
    }
}

pub(super) fn push_block(stack: &mut [Frame], block: Block) {
    if let Some(Frame::Container(blocks)) = stack.last_mut() {
        blocks.push(block);
    }
}

pub(super) fn push_text(stack: &mut Vec<Frame>, text: &str) {
    match stack.last_mut() {
        Some(Frame::CodeBlock(buf) | Frame::HtmlBlock(buf)) => buf.push_str(text),
        Some(Frame::Image { alt, .. }) => alt.push_str(text),
        _ => push_span(stack, Span::plain(text.to_string())),
    }
}

pub(super) fn push_code(stack: &mut Vec<Frame>, text: &str) {
    match stack.last_mut() {
        Some(Frame::Image { alt, .. }) => alt.push_str(text),
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
