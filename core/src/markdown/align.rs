use pulldown_cmark::HeadingLevel;

use crate::ir::Align;

pub(super) fn heading_level(level: HeadingLevel) -> u8 {
    level as u8
}

pub(super) fn map_alignment(a: pulldown_cmark::Alignment) -> Align {
    match a {
        pulldown_cmark::Alignment::None => Align::None,
        pulldown_cmark::Alignment::Left => Align::Left,
        pulldown_cmark::Alignment::Center => Align::Center,
        pulldown_cmark::Alignment::Right => Align::Right,
    }
}
