use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;

/// Convert a PDF file to Markdown.
#[derive(Parser)]
#[command(name = "pdf2md")]
struct Cli {
    input: PathBuf,
    /// Output path (defaults to the input path with a .md extension).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Pages to convert, 1-based and inclusive: `3` or `2-5`.
    #[arg(long, value_parser = parse_pages)]
    pages: Option<(u32, u32)>,
    /// Keep running headers, footers and page numbers.
    #[arg(long)]
    keep_headers: bool,
}

fn parse_pages(value: &str) -> Result<(u32, u32)> {
    let (start, end) = value.split_once('-').unwrap_or((value, value));
    let start: u32 = start.trim().parse().context("invalid start page")?;
    let end: u32 = end.trim().parse().context("invalid end page")?;
    if start == 0 || end < start {
        bail!("expected `N` or `N-M` with 1 <= N <= M");
    }
    Ok((start, end))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let options = pdf2md_core::Options {
        pages: cli.pages,
        keep_headers: cli.keep_headers,
    };
    convert(&cli.input, cli.output, &options)
}

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "reporting the result and warnings is the CLI's job"
)]
fn convert(input: &Path, output: Option<PathBuf>, options: &pdf2md_core::Options) -> Result<()> {
    let output = output.unwrap_or_else(|| pdf2md_core::default_md_output_path(input));

    let pdf =
        std::fs::read(input).with_context(|| format!("failed to read {}", input.display()))?;

    let result = pdf2md_core::convert(&pdf, options)
        .with_context(|| format!("failed to convert {}", input.display()))?;
    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }

    std::fs::write(&output, result.markdown)
        .with_context(|| format!("failed to write {}", output.display()))?;

    println!(
        "Wrote {} ({} page{})",
        output.display(),
        result.page_count,
        if result.page_count == 1 { "" } else { "s" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pages_accepts_single_pages_and_ranges() {
        assert_eq!(parse_pages("3").unwrap(), (3, 3));
        assert_eq!(parse_pages("2-5").unwrap(), (2, 5));
        assert!(parse_pages("0").is_err());
        assert!(parse_pages("5-2").is_err());
        assert!(parse_pages("x").is_err());
    }
}
