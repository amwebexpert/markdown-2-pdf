//! Raw HTML policy: never interpreted, never silently lost.
//!
//! Inline HTML stays literal text, except `<br>` (a line break) and comments
//! (dropped, since the author meant them to be hidden).

use super::frame::Frame;
use super::stack::push_text;

/// Exactly one comment: anything after its first `-->` is other content.
pub(super) fn is_html_comment(html: &str) -> bool {
    html.trim()
        .strip_prefix("<!--")
        .and_then(|rest| rest.split_once("-->"))
        .is_some_and(|(_, after)| after.is_empty())
}

fn is_br_tag(html: &str) -> bool {
    let inner = html
        .trim()
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or_default()
        .trim_end_matches('/')
        .trim();
    inner.eq_ignore_ascii_case("br")
}

pub(super) fn push_inline_html(stack: &mut Vec<Frame>, html: &str) {
    if is_html_comment(html) {
        return;
    }
    if is_br_tag(html) {
        push_text(stack, "\n");
    } else {
        push_text(stack, html);
    }
}
