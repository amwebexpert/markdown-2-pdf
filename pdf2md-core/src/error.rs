use thiserror::Error;

/// Errors returned by [`crate::convert`].
#[derive(Debug, Error)]
pub enum Pdf2MdError {
    /// The input could not be parsed as a PDF document.
    #[error("failed to parse PDF: {0}")]
    Parse(String),
    /// The document is password-protected.
    #[error("PDF is encrypted; password-protected files are not supported yet")]
    Encrypted,
    /// The requested page range does not intersect the document.
    #[error("page range {start}-{end} is outside the document ({count} pages)")]
    PageRange {
        /// First requested page (1-based).
        start: u32,
        /// Last requested page (1-based, inclusive).
        end: u32,
        /// Number of pages in the document.
        count: usize,
    },
}
