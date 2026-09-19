mod error;
mod fonts;
mod ir;
mod markdown;
mod render;

use std::path::{Path, PathBuf};

pub use error::Md2PdfError;

/// Default PDF output path: the input path with its final extension replaced by
/// `pdf` ([`Path::with_extension`]). For example, `notes.tar.md` →
/// `notes.tar.pdf`; a path with no extension becomes `{name}.pdf`.
pub fn default_pdf_output_path(input: impl AsRef<Path>) -> PathBuf {
    input.as_ref().with_extension("pdf")
}

/// Converts a Markdown document into PDF bytes, along with the page count.
pub fn convert(markdown: &str) -> Result<(Vec<u8>, usize), Md2PdfError> {
    let blocks = markdown::parse(markdown);
    render::render(&blocks)
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
    fn convert_turns_markdown_into_pdf_bytes() {
        let (bytes, page_count) = convert("# Title\n\nSome *text*.\n").unwrap();
        assert_eq!(page_count, 1);
        assert!(bytes.starts_with(b"%PDF"));
    }
}
