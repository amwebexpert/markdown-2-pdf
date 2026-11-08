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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("md2pdf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn convert_writes_next_to_the_input_by_default() {
        let dir = temp_dir("default");
        let input = dir.join("doc.md");
        std::fs::write(&input, "# Title\n\nBody.\n").unwrap();
        convert(&input, None).unwrap();
        let pdf = std::fs::read(dir.join("doc.pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn convert_honors_an_explicit_output_path() {
        let dir = temp_dir("explicit");
        let input = dir.join("doc.md");
        let output = dir.join("other.pdf");
        std::fs::write(&input, "text\n").unwrap();
        convert(&input, Some(output.clone())).unwrap();
        assert!(output.is_file());
        assert!(!dir.join("doc.pdf").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn convert_reports_a_missing_input() {
        let dir = temp_dir("missing");
        let input = dir.join("absent.md");
        let err = convert(&input, None).unwrap_err();
        assert!(err.to_string().starts_with("failed to read"), "{err}");
        assert!(!dir.join("absent.pdf").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn convert_reports_an_unwritable_output() {
        let dir = temp_dir("unwritable");
        let input = dir.join("doc.md");
        std::fs::write(&input, "text\n").unwrap();
        let err = convert(&input, Some(dir.join("no/such/dir/out.pdf"))).unwrap_err();
        assert!(err.to_string().starts_with("failed to write"), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cli_parses_convert_with_output_flag() {
        let cli = Cli::try_parse_from(["md2pdf", "convert", "in.md", "-o", "out.pdf"]).unwrap();
        let Command::Convert { input, output } = cli.command;
        assert_eq!(input, PathBuf::from("in.md"));
        assert_eq!(output, Some(PathBuf::from("out.pdf")));
    }
}
