use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "md2pdf")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert a Markdown file to PDF.
    Convert {
        input: PathBuf,
        /// Output path (defaults to the input path with a .pdf extension).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Convert { input, output } => convert(&input, output),
    }
}

#[expect(clippy::print_stdout, reason = "reporting the result is the CLI's job")]
fn convert(input: &Path, output: Option<PathBuf>) -> Result<()> {
    let output = output.unwrap_or_else(|| md2pdf_core::default_pdf_output_path(input));

    let markdown = std::fs::read_to_string(input)
        .with_context(|| format!("failed to read {}", input.display()))?;

    let (pdf_bytes, page_count) = md2pdf_core::convert(&markdown)
        .with_context(|| format!("failed to convert {}", input.display()))?;

    std::fs::write(&output, pdf_bytes)
        .with_context(|| format!("failed to write {}", output.display()))?;

    println!(
        "Wrote {} ({page_count} page{})",
        output.display(),
        if page_count == 1 { "" } else { "s" }
    );
    Ok(())
}
