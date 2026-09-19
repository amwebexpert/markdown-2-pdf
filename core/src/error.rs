use thiserror::Error;

#[derive(Debug, Error)]
pub enum Md2PdfError {
    #[error("failed to parse embedded font: {0}")]
    FontParse(String),
    #[error("failed to write PDF output: {0}")]
    PdfWrite(String),
}
