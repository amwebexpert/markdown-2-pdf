//! Parses Markdown into the [`crate::ir`] block tree using pulldown-cmark.
//!
//! Only `Options::ENABLE_TABLES` is turned on. Everything else CommonMark can
//! emit without an extension flag (blockquotes, images, thematic breaks, raw
//! HTML) is handled below; anything gated behind an extension we don't enable
//! (strikethrough, task lists, footnotes, ...) simply falls through pulldown-cmark
//! as literal text, which is itself the "best-effort, never fail" fallback.

mod align;
mod close;
mod event;
mod frame;
mod open;
mod stack;

use pulldown_cmark::{Options, Parser};

use crate::ir::Block;

use frame::Frame;

pub fn parse(input: &str) -> Vec<Block> {
    let parser = Parser::new_ext(input, Options::ENABLE_TABLES);
    let mut stack: Vec<Frame> = vec![Frame::Container(Vec::new())];

    for event in parser {
        event::apply_event(&mut stack, event);
    }

    match stack.pop() {
        Some(Frame::Container(blocks)) => blocks,
        _ => Vec::new(),
    }
}
