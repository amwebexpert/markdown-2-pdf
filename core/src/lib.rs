//! Markdown to PDF conversion: parses CommonMark plus GFM tables,
//! strikethrough, task lists and footnotes, and lays it out as a paginated US
//! Letter PDF with embedded fonts and PNG/JPEG images.
//!
//! The crate does no I/O: image bytes are supplied by the caller through
//! [`Options::images`] (see [`image_sources`] for which ones are needed).

#![warn(missing_docs)]

mod error;
mod fonts;
mod image;
mod ir;
mod markdown;
mod render;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use error::Md2PdfError;

/// Default PDF output path: the input path with its final extension replaced by
/// `pdf` ([`Path::with_extension`]). For example, `notes.tar.md` →
/// `notes.tar.pdf`; a path with no extension becomes `{name}.pdf`.
pub fn default_pdf_output_path(input: impl AsRef<Path>) -> PathBuf {
    input.as_ref().with_extension("pdf")
}

/// Conversion options.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Image bytes keyed by the image URL exactly as written in the Markdown
    /// (`![alt](this/part.png)`). PNG and JPEG are embedded; anything missing
    /// or unsupported is rendered as `[image: alt]` text with a warning.
    pub images: BTreeMap<String, Vec<u8>>,
}

/// The result of [`convert_with_options`].
#[derive(Debug, Clone)]
pub struct Output {
    /// The PDF file bytes.
    pub pdf: Vec<u8>,
    /// Number of pages produced.
    pub page_count: usize,
    /// Content that could not be rendered faithfully (e.g. a missing image)
    /// and fell back to text. The conversion itself still succeeded.
    pub warnings: Vec<String>,
}

/// Converts a Markdown document into PDF bytes, along with the page count.
/// Images are rendered as `[image: alt]`; use [`convert_with_options`] to
/// embed them.
///
/// # Errors
///
/// Returns [`Md2PdfError::FontParse`] if an embedded font cannot be parsed.
pub fn convert(markdown: &str) -> Result<(Vec<u8>, usize), Md2PdfError> {
    let output = convert_with_options(markdown, &Options::default())?;
    Ok((output.pdf, output.page_count))
}

/// Converts a Markdown document into a PDF, embedding the images supplied in
/// `options`.
///
/// # Errors
///
/// Returns [`Md2PdfError::FontParse`] if an embedded font cannot be parsed.
pub fn convert_with_options(markdown: &str, options: &Options) -> Result<Output, Md2PdfError> {
    let blocks = markdown::parse(markdown);
    let (pdf, page_count, warnings) = render::render(&blocks, &options.images)?;
    Ok(Output {
        pdf,
        page_count,
        warnings,
    })
}

/// The image URLs the document would embed (images standing alone in their
/// paragraph), deduplicated, in document order. Callers use it to know which
/// bytes to put in [`Options::images`]; images inside running text are shown
/// as `[image: alt]` and not listed.
pub fn image_sources(markdown: &str) -> Vec<String> {
    markdown::image_sources(&markdown::parse(markdown))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pdf_output_path_replaces_the_extension() {
        assert_eq!(
            default_pdf_output_path("notes.tar.md"),
            PathBuf::from("notes.tar.pdf")
        );
    }

    #[test]
    fn default_pdf_output_path_appends_when_there_is_none() {
        assert_eq!(default_pdf_output_path("notes"), PathBuf::from("notes.pdf"));
    }

    #[test]
    fn image_sources_lists_what_convert_would_embed() {
        let md = "![a](img/a.png)\n\ntext ![b](b.png)\n\n![a](img/a.png)\n";
        assert_eq!(image_sources(md), ["img/a.png"]);
    }

    #[test]
    fn missing_image_is_a_warning_not_an_error() {
        let output = convert_with_options("![alt](gone.png)\n", &Options::default()).unwrap();
        assert_eq!(output.page_count, 1);
        assert!(output.pdf.starts_with(b"%PDF"));
        assert_eq!(output.warnings, ["image gone.png: image not found"]);
    }

    #[test]
    fn convert_turns_markdown_into_pdf_bytes() {
        let (bytes, page_count) = convert("# Title\n\nSome *text*.\n").unwrap();
        assert_eq!(page_count, 1);
        assert!(bytes.starts_with(b"%PDF"));
    }
}
