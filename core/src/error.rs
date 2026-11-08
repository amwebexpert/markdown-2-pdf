use thiserror::Error;

/// Errors returned by [`crate::convert`].
#[derive(Debug, Error)]
pub enum Md2PdfError {
    /// An embedded font could not be parsed.
    #[error("failed to parse embedded font: {0}")]
    FontParse(String),
    /// The PDF document could not be serialized.
    #[error("failed to write PDF output: {0}")]
    PdfWrite(String),
}
