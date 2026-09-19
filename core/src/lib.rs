mod error;
mod fonts;
mod ir;
mod markdown;
mod render;

pub use error::Md2PdfError;

/// Converts a Markdown document into PDF bytes, along with the page count.
pub fn convert(markdown: &str) -> Result<(Vec<u8>, usize), Md2PdfError> {
    let blocks = markdown::parse(markdown);
    render::render(&blocks)
}
