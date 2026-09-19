//! Maps pulldown-cmark events to stack operations.

use pulldown_cmark::{Event, Tag, TagEnd};

use super::close::close_tag;
use super::frame::Frame;
use super::open::open_tag;
use super::stack::{flush_dangling_paragraph, push_block, push_code, push_text};
use crate::ir::Block;

fn is_inline_open(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Emphasis | Tag::Strong | Tag::Link { .. } | Tag::Image { .. }
    )
}

pub(super) fn apply_event(stack: &mut Vec<Frame>, event: Event<'_>) {
    match event {
        Event::Start(tag) => {
            // Tight list items skip Paragraph events entirely ("tight
            // paragraphs emit nothing" — pulldown-cmark parse.rs), so
            // inline content arrives directly inside `Tag::Item`'s
            // Container frame. `push_span` can't route text into a
            // Container, so a Paragraph opened implicitly by `push_text`
            // must be closed before any other block-level tag can nest
            // — except another inline container, which belongs inside it.
            if !is_inline_open(&tag) {
                flush_dangling_paragraph(stack);
            }
            open_tag(stack, tag);
        }
        Event::End(tag_end) => {
            // Only `Item`'s Container can have a dangling implicit
            // paragraph sitting above it — every other End pops a frame
            // that manages its own closing (including `Paragraph`
            // itself, which must NOT be pre-flushed here or its own
            // handler below would pop the wrong frame).
            if matches!(tag_end, TagEnd::Item) {
                flush_dangling_paragraph(stack);
            }
            close_tag(stack, tag_end);
        }
        Event::Text(text) => push_text(stack, &text),
        Event::Code(text) => push_code(stack, &text),
        Event::Html(text) | Event::InlineHtml(text) => push_text(stack, &text),
        Event::SoftBreak => push_text(stack, " "),
        Event::HardBreak => push_text(stack, "\n"),
        Event::Rule => push_block(stack, Block::ThematicBreak),
        _ => {}
    }
}
