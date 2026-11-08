use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};

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
    /// Where extracted images go: files in an assets directory, or base64
    /// data URIs inside the Markdown.
    #[arg(long, value_enum, default_value_t = Images::Dir)]
    images: Images,
    /// Assets directory (default `<output stem>_assets`); a relative path is
    /// relative to the output file's folder.
    #[arg(long)]
    assets_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Images {
    Dir,
    Inline,
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
    let output = cli
        .output
        .unwrap_or_else(|| pdf2md_core::default_md_output_path(&cli.input));
    let images = match cli.images {
        Images::Inline => pdf2md_core::ImageMode::Inline,
        Images::Dir => pdf2md_core::ImageMode::Assets {
            dir: assets_link(&output, cli.assets_dir.as_deref()),
        },
    };
    let options = pdf2md_core::Options {
        pages: cli.pages,
        keep_headers: cli.keep_headers,
        images,
    };
    convert(&cli.input, &output, &options)
}

/// Assets directory as written in image links: relative to the Markdown
/// file's folder when possible, `/`-separated.
fn assets_link(output: &Path, assets_dir: Option<&Path>) -> String {
    let dir = assets_dir.map_or_else(
        || {
            let stem = output.file_stem().unwrap_or_default().to_string_lossy();
            PathBuf::from(format!("{stem}_assets"))
        },
        Path::to_path_buf,
    );
    let folder = output.parent().unwrap_or_else(|| Path::new(""));
    let relative = dir.strip_prefix(folder).unwrap_or(&dir);
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "reporting the result and warnings is the CLI's job"
)]
fn convert(input: &Path, output: &Path, options: &pdf2md_core::Options) -> Result<()> {
    let pdf =
        std::fs::read(input).with_context(|| format!("failed to read {}", input.display()))?;

    let result = pdf2md_core::convert(&pdf, options)
        .with_context(|| format!("failed to convert {}", input.display()))?;
    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }

    std::fs::write(output, result.markdown)
        .with_context(|| format!("failed to write {}", output.display()))?;
    // Links are relative to the Markdown file, so assets are written there.
    let folder = output.parent().unwrap_or_else(|| Path::new(""));
    for asset in &result.assets {
        let path = folder.join(&asset.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        std::fs::write(&path, &asset.bytes)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }

    let images = match result.assets.len() {
        0 => String::new(),
        1 => ", 1 image".to_string(),
        n => format!(", {n} images"),
    };
    println!(
        "Wrote {} ({} page{}{images})",
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

    #[test]
    fn assets_link_is_relative_to_the_markdown_folder() {
        assert_eq!(assets_link(Path::new("out/doc.md"), None), "doc_assets");
        assert_eq!(
            assets_link(Path::new("out/doc.md"), Some(Path::new("out/img/x"))),
            "img/x"
        );
        assert_eq!(
            assets_link(Path::new("doc.md"), Some(Path::new("pics"))),
            "pics"
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdf2md-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MEDIA_PDF: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pdf2md-core/tests/fixtures/external/chrome-media.pdf"
    );

    #[test]
    fn default_mode_writes_assets_next_to_the_markdown() {
        let dir = temp_dir("dir");
        let output = dir.join("out.md");
        let options = pdf2md_core::Options {
            images: pdf2md_core::ImageMode::Assets {
                dir: assets_link(&output, None),
            },
            ..pdf2md_core::Options::default()
        };
        convert(Path::new(MEDIA_PDF), &output, &options).unwrap();
        let markdown = std::fs::read_to_string(&output).unwrap();
        let links: Vec<&str> = markdown
            .split("![](")
            .skip(1)
            .filter_map(|rest| rest.split(')').next())
            .collect();
        assert!(!links.is_empty());
        assert!(links.iter().all(|l| l.starts_with("out_assets/img-00")));
        assert!(links.iter().all(|l| dir.join(l).is_file()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn inline_mode_writes_no_assets() {
        let dir = temp_dir("inline");
        let output = dir.join("out.md");
        let options = pdf2md_core::Options {
            images: pdf2md_core::ImageMode::Inline,
            ..pdf2md_core::Options::default()
        };
        convert(Path::new(MEDIA_PDF), &output, &options).unwrap();
        let markdown = std::fs::read_to_string(&output).unwrap();
        assert!(markdown.contains("![](data:image/png;base64,"));
        let entries = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(entries, 1, "only out.md");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
