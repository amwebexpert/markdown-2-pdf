//! PDF to Markdown conversion: extracts positioned text from born-digital PDFs
//! and rebuilds GitHub-flavored Markdown (headings, lists, emphasis, links,
//! code) with layout heuristics. Scanned PDFs (no text layer) are not supported.

#![warn(missing_docs)]

mod blocks;
mod emit;
mod error;
mod font;
mod geom;
mod interp;
mod layout;
mod page;

use std::path::{Path, PathBuf};

use lopdf::{Document, LoadOptions};

pub use error::Pdf2MdError;

use crate::blocks::PageLines;
use crate::interp::{Interpreter, MAX_DECOMPRESSED};

/// Conversion settings.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Inclusive 1-based page range to convert; `None` converts every page.
    pub pages: Option<(u32, u32)>,
    /// Keep running headers, footers and page numbers instead of stripping them.
    pub keep_headers: bool,
}

/// Result of a conversion.
#[derive(Debug, Clone)]
pub struct Output {
    /// The Markdown document.
    pub markdown: String,
    /// Number of pages converted.
    pub page_count: usize,
    /// Non-fatal problems, e.g. pages whose content could not be parsed.
    pub warnings: Vec<String>,
}

/// Default Markdown output path: the input path with its final extension
/// replaced by `md` ([`Path::with_extension`]), e.g. `paper.pdf` → `paper.md`.
pub fn default_md_output_path(input: impl AsRef<Path>) -> PathBuf {
    input.as_ref().with_extension("md")
}

/// Converts PDF bytes into Markdown.
///
/// Pages that fail to parse are replaced by an HTML comment and reported in
/// [`Output::warnings`] instead of aborting the conversion.
///
/// # Errors
///
/// Returns [`Pdf2MdError::Parse`] if the bytes are not a readable PDF,
/// [`Pdf2MdError::Encrypted`] if the document is password-protected, and
/// [`Pdf2MdError::PageRange`] if `options.pages` selects no existing page.
pub fn convert(pdf: &[u8], options: &Options) -> Result<Output, Pdf2MdError> {
    let load_options = LoadOptions {
        max_decompressed_size: Some(MAX_DECOMPRESSED),
        ..LoadOptions::default()
    };
    let doc = Document::load_mem_with_options(pdf, load_options).map_err(|err| match err {
        lopdf::Error::InvalidPassword | lopdf::Error::Decryption(_) => Pdf2MdError::Encrypted,
        other => Pdf2MdError::Parse(other.to_string()),
    })?;

    let all_pages = doc.get_pages();
    let selected: Vec<(u32, lopdf::ObjectId)> = all_pages
        .iter()
        .filter(|(number, _)| {
            options
                .pages
                .is_none_or(|(start, end)| (start..=end).contains(*number))
        })
        .map(|(number, id)| (*number, *id))
        .collect();
    if let Some((start, end)) = options.pages
        && selected.is_empty()
    {
        return Err(Pdf2MdError::PageRange {
            start,
            end,
            count: all_pages.len(),
        });
    }

    let mut interpreter = Interpreter::new(&doc);
    let mut warnings = Vec::new();
    let mut failed = Vec::new();
    let mut extracted = Vec::new();
    for (number, page_id) in &selected {
        match page::extract(&doc, &mut interpreter, *number, *page_id) {
            Ok(page) => {
                extracted.push(page);
                failed.push(false);
            }
            Err(err) => {
                warnings.push(format!("page {number}: {err}"));
                failed.push(true);
            }
        }
    }

    // Furniture detection needs every page at once; failed pages keep their slot.
    let mut lines = layout::lines(&extracted, options.keep_headers)
        .into_iter()
        .peekable();
    let page_lines: Vec<PageLines> = failed
        .into_iter()
        .zip(&selected)
        .map(|(failed, (number, _))| {
            if failed {
                return PageLines::Failed(*number);
            }
            let mut on_page = Vec::new();
            while let Some(line) = lines.next_if(|l| l.page == *number) {
                on_page.push(line);
            }
            PageLines::Lines(on_page)
        })
        .collect();

    let blocks = blocks::build(page_lines);
    Ok(Output {
        markdown: emit::markdown(&blocks),
        page_count: selected.len(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_md_output_path_replaces_the_extension() {
        assert_eq!(
            default_md_output_path("paper.pdf"),
            PathBuf::from("paper.md")
        );
    }

    #[test]
    fn garbage_is_a_parse_error() {
        assert!(matches!(
            convert(b"not a pdf", &Options::default()),
            Err(Pdf2MdError::Parse(_))
        ));
    }

    #[test]
    fn out_of_range_pages_are_rejected() {
        let (pdf, _) = md2pdf_core::convert("# One page\n").unwrap();
        let options = Options {
            pages: Some((5, 6)),
            ..Options::default()
        };
        assert!(matches!(
            convert(&pdf, &options),
            Err(Pdf2MdError::PageRange { count: 1, .. })
        ));
    }
}
