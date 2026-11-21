use std::collections::BTreeMap;
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
        Command::Convert { input, output } => convert(&input, output).map(drop),
    }
}

/// `scheme:` as in RFC 3986 (a letter, then letters, digits, `+`, `-`, `.`).
/// Single letters are not schemes, so Windows drive paths (`C:\x.png`) stay paths.
fn has_url_scheme(src: &str) -> bool {
    src.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Decodes `%XX` escapes (`my%20file.png` → `my file.png`), as a Markdown
/// link destination with spaces must be written. Invalid escapes stay as is.
fn percent_decode(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        if let (b'%', Some(decoded)) = (b, hex) {
            out.push(decoded);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

/// Reads the images the document embeds. Relative paths resolve against
/// `base_dir` (the Markdown file's directory); URLs are never fetched.
/// Problems become warnings: the image then renders as `[image: alt]`.
fn load_images(markdown: &str, base_dir: &Path) -> (BTreeMap<String, Vec<u8>>, Vec<String>) {
    let mut images = BTreeMap::new();
    let mut warnings = Vec::new();
    for src in md2pdf_core::image_sources(markdown) {
        if has_url_scheme(&src) {
            warnings.push(format!("image {src}: remote images are not fetched"));
            continue;
        }
        let path = base_dir.join(percent_decode(&src));
        match std::fs::read(&path) {
            Ok(bytes) => {
                images.insert(src, bytes);
            }
            Err(err) => warnings.push(format!(
                "image {src}: cannot read {}: {err}",
                path.display()
            )),
        }
    }
    (images, warnings)
}

/// Both warnings are about the same image (`image <src>: …`).
fn same_image(a: &str, b: &str) -> bool {
    let src = |w: &str| {
        w.strip_prefix("image ")
            .and_then(|w| w.split_once(": "))
            .map(|(s, _)| s.to_owned())
    };
    src(a).is_some() && src(a) == src(b)
}

/// Converts `input` and returns the warnings, already printed to stderr.
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "reporting the result is the CLI's job"
)]
fn convert(input: &Path, output: Option<PathBuf>) -> Result<Vec<String>> {
    let output = output.unwrap_or_else(|| md2pdf_core::default_pdf_output_path(input));

    let markdown = std::fs::read_to_string(input)
        .with_context(|| format!("failed to read {}", input.display()))?;

    let base_dir = input.parent().unwrap_or_else(|| Path::new(""));
    let (images, mut warnings) = load_images(&markdown, base_dir);
    let result = md2pdf_core::convert_with_options(&markdown, &md2pdf_core::Options { images })
        .with_context(|| format!("failed to convert {}", input.display()))?;
    // An image the CLI couldn't load is also "not found" for the core; keep
    // only the CLI's more specific reason.
    let core_warnings: Vec<String> = result
        .warnings
        .into_iter()
        .filter(|w| !warnings.iter().any(|ours| same_image(ours, w)))
        .collect();
    warnings.extend(core_warnings);

    std::fs::write(&output, result.pdf)
        .with_context(|| format!("failed to write {}", output.display()))?;

    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    let page_count = result.page_count;
    println!(
        "Wrote {} ({page_count} page{})",
        output.display(),
        if page_count == 1 { "" } else { "s" }
    );
    Ok(warnings)
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

    /// A valid 1 × 1 transparent PNG.
    const PNG_1PX: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn load_images_resolves_relative_paths_against_the_markdown_directory() {
        let dir = temp_dir("images");
        std::fs::create_dir_all(dir.join("img")).unwrap();
        std::fs::write(dir.join("img/a.png"), PNG_1PX).unwrap();
        let absolute = dir.join("abs.png");
        std::fs::write(&absolute, PNG_1PX).unwrap();
        let markdown = format!(
            "![a](img/a.png)\n\n![b]({})\n\n![c](missing.png)\n\n![d](https://example.com/d.png)\n",
            absolute.display()
        );
        let (images, warnings) = load_images(&markdown, &dir);
        let keys: Vec<&str> = images.keys().map(String::as_str).collect();
        let absolute = absolute.display().to_string();
        let mut expected = vec!["img/a.png", absolute.as_str()];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings[0].starts_with("image missing.png: cannot read "),
            "{warnings:?}"
        );
        assert_eq!(
            warnings[1],
            "image https://example.com/d.png: remote images are not fetched"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn percent_escapes_are_decoded_for_file_lookup() {
        assert_eq!(percent_decode("my%20file.png"), "my file.png");
        assert_eq!(percent_decode("caf%C3%A9.png"), "café.png");
        assert_eq!(percent_decode("100%.png"), "100%.png");
        assert_eq!(percent_decode("a%2.png"), "a%2.png");
    }

    #[test]
    fn url_schemes_are_told_apart_from_paths() {
        for url in [
            "http://x/a.png",
            "https://x/a.png",
            "data:image/png;base64,AA",
            "file:///a.png",
        ] {
            assert!(has_url_scheme(url), "{url}");
        }
        for path in [
            "a.png",
            "img/a:b.png",
            "/abs/a.png",
            "C:\\img\\a.png",
            "./a.png",
        ] {
            assert!(!has_url_scheme(path), "{path}");
        }
    }

    #[test]
    fn convert_reports_each_unavailable_image_once() {
        let dir = temp_dir("unavailable");
        let input = dir.join("doc.md");
        std::fs::write(dir.join("bad.png"), b"not a png").unwrap();
        std::fs::write(
            &input,
            "![a](gone.png)\n\n![b](http://x/b.png)\n\n![c](bad.png)\n",
        )
        .unwrap();
        let warnings = convert(&input, None).unwrap();
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].starts_with("image gone.png: cannot read "));
        assert!(warnings[1].starts_with("image http://x/b.png: remote"));
        assert!(warnings[2].starts_with("image bad.png: unsupported image format"));
        assert!(dir.join("doc.pdf").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn convert_embeds_a_local_image() {
        let dir = temp_dir("embed");
        let input = dir.join("doc.md");
        std::fs::write(dir.join("dot.png"), PNG_1PX).unwrap();
        std::fs::write(&input, "![dot](dot.png)\n").unwrap();
        let warnings = convert(&input, None).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let pdf = std::fs::read(dir.join("doc.pdf")).unwrap();
        assert!(
            pdf.windows(b"/Subtype /Image".len())
                .any(|w| w == b"/Subtype /Image")
                || pdf
                    .windows(b"/Subtype/Image".len())
                    .any(|w| w == b"/Subtype/Image")
        );
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
