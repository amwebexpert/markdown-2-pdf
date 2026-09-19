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

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::ir::{Align, Block, Span};

    fn span_text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn paragraph_is_a_single_plain_span() {
        let blocks = parse("hello world");
        match blocks.as_slice() {
            [Block::Paragraph { spans }] => assert_eq!(span_text(spans), "hello world"),
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }

    #[test]
    fn heading_levels_are_mapped_1_through_6() {
        let blocks = parse("# a\n\n###### b");
        let levels: Vec<u8> = blocks
            .iter()
            .map(|b| match b {
                Block::Heading { level, .. } => *level,
                other => panic!("expected heading, got {other:?}"),
            })
            .collect();
        assert_eq!(levels, vec![1, 6]);
    }

    #[test]
    fn nested_strong_and_emphasis_set_both_flags() {
        let blocks = parse("**_bi_**");
        match blocks.as_slice() {
            [Block::Paragraph { spans }] => match spans.as_slice() {
                [span] => {
                    assert_eq!(span.text, "bi");
                    assert!(span.bold && span.italic);
                }
                other => panic!("expected one span, got {other:?}"),
            },
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }

    #[test]
    fn code_span_sets_code_flag() {
        let blocks = parse("`code`");
        match blocks.as_slice() {
            [Block::Paragraph { spans }] => match spans.as_slice() {
                [span] => assert!(span.code && span.text == "code"),
                other => panic!("expected one span, got {other:?}"),
            },
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }

    #[test]
    fn link_sets_url_on_its_spans() {
        let blocks = parse("[text](http://example.com)");
        match blocks.as_slice() {
            [Block::Paragraph { spans }] => match spans.as_slice() {
                [span] => assert_eq!(span.link.as_deref(), Some("http://example.com")),
                other => panic!("expected one span, got {other:?}"),
            },
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }

    #[test]
    fn image_becomes_bracketed_alt_text() {
        let blocks = parse("![alt text](http://example.com/x.png)");
        match blocks.as_slice() {
            [Block::Paragraph { spans }] => assert_eq!(span_text(spans), "[image: alt text]"),
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }

    #[test]
    fn tight_list_items_become_paragraph_blocks() {
        let blocks = parse("- one\n- two");
        match blocks.as_slice() {
            [
                Block::List {
                    ordered,
                    start,
                    items,
                },
            ] => {
                assert!(!ordered);
                assert_eq!(*start, 1);
                assert_eq!(items.len(), 2);
                for (item, text) in items.iter().zip(["one", "two"]) {
                    match item.as_slice() {
                        [Block::Paragraph { spans }] => assert_eq!(span_text(spans), text),
                        other => panic!("expected one paragraph, got {other:?}"),
                    }
                }
            }
            other => panic!("expected one list, got {other:?}"),
        }
    }

    #[test]
    fn ordered_list_keeps_its_start_number() {
        let blocks = parse("3. a\n4. b");
        match blocks.as_slice() {
            [Block::List { ordered, start, .. }] => {
                assert!(ordered);
                assert_eq!(*start, 3);
            }
            other => panic!("expected one list, got {other:?}"),
        }
    }

    #[test]
    fn blockquote_wraps_its_blocks() {
        let blocks = parse("> quoted");
        match blocks.as_slice() {
            [Block::BlockQuote(inner)] => match inner.as_slice() {
                [Block::Paragraph { spans }] => assert_eq!(span_text(spans), "quoted"),
                other => panic!("expected one paragraph, got {other:?}"),
            },
            other => panic!("expected one blockquote, got {other:?}"),
        }
    }

    #[test]
    fn table_collects_alignments_header_and_rows() {
        let blocks = parse("| a | b |\n|:--|--:|\n| 1 | 2 |\n");
        match blocks.as_slice() {
            [
                Block::Table {
                    alignments,
                    header,
                    rows,
                },
            ] => {
                assert_eq!(alignments, &[Align::Left, Align::Right]);
                assert_eq!(
                    header.iter().map(|c| span_text(c)).collect::<Vec<_>>(),
                    ["a", "b"]
                );
                assert_eq!(rows.len(), 1);
                assert_eq!(
                    rows[0].iter().map(|c| span_text(c)).collect::<Vec<_>>(),
                    ["1", "2"]
                );
            }
            other => panic!("expected one table, got {other:?}"),
        }
    }

    #[test]
    fn thematic_break_produces_its_own_block() {
        let blocks = parse("***");
        assert_eq!(blocks, vec![Block::ThematicBreak]);
    }

    #[test]
    fn soft_and_hard_breaks_become_space_and_newline() {
        let soft = parse("line one\nline two");
        match soft.as_slice() {
            [Block::Paragraph { spans }] => assert_eq!(span_text(spans), "line one line two"),
            other => panic!("expected one paragraph, got {other:?}"),
        }

        let hard = parse("line one  \nline two");
        match hard.as_slice() {
            [Block::Paragraph { spans }] => assert_eq!(span_text(spans), "line one\nline two"),
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }
}
