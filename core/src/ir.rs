//! Intermediate representation produced by the Markdown parser and consumed by the PDF layout engine.

#[derive(Debug, Clone, Default, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent inline styles that combine freely, as in Markdown"
)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    /// Raised, smaller text (footnote reference numbers).
    pub superscript: bool,
    /// Set on the `[image: alt]` fallback span of an image, so a paragraph
    /// holding only images can become [`Block::Image`]s.
    pub image: Option<ImageRef>,
    pub link: Option<String>,
}

const IMAGE_FALLBACK_PREFIX: &str = "[image: ";

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Span {
            text: text.into(),
            ..Default::default()
        }
    }

    /// The text shown for an image that can't be embedded: `[image: alt]`.
    pub fn image_fallback(alt: &str, image: ImageRef) -> Self {
        Span {
            text: format!("{IMAGE_FALLBACK_PREFIX}{alt}]"),
            image: Some(image),
            ..Default::default()
        }
    }

    /// The alt text of an [`Span::image_fallback`] span.
    pub fn alt_text(&self) -> String {
        self.text
            .strip_prefix(IMAGE_FALLBACK_PREFIX)
            .and_then(|t| t.strip_suffix(']'))
            .unwrap_or(&self.text)
            .to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageRef {
    pub src: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    None,
}

#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::enum_variant_names,
    reason = "variant names mirror CommonMark element names"
)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph {
        spans: Vec<Span>,
    },
    CodeBlock {
        text: String,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<ListItem>,
    },
    BlockQuote(Vec<Block>),
    Table {
        alignments: Vec<Align>,
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    ThematicBreak,
    /// An image that stood alone in its paragraph.
    Image {
        src: String,
        alt: String,
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    /// `Some(checked)` for a GFM task list item (`- [x]` / `- [ ]`).
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}
