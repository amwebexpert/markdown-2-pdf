//! md → PDF (md2pdf) → md (pdf2md) round trips.

use pdf2md_core::{Options, convert};

#[expect(
    clippy::expect_used,
    reason = "test helper; a failure should abort the test"
)]
fn round_trip(markdown: &str) -> String {
    let (pdf, _) = md2pdf_core::convert(markdown).expect("md2pdf conversion");
    convert(&pdf, &Options::default())
        .expect("pdf2md conversion")
        .markdown
}

#[test]
fn supported_constructs_round_trip_exactly() {
    let source = include_str!("fixtures/roundtrip.md");
    assert_eq!(round_trip(source), source);
}

#[test]
fn kitchen_sink_snapshot() {
    insta::assert_snapshot!(round_trip(include_str!(
        "../../cli/examples/kitchen-sink.md"
    )));
}

#[test]
fn multi_page_snapshot() {
    insta::assert_snapshot!(round_trip(include_str!(
        "../../cli/examples/nested-and-long.md"
    )));
}

#[test]
fn strikethrough_task_lists_and_footnotes_round_trip() {
    let markdown = round_trip(
        "This is ~~deprecated~~ text.[^source]\n\n\
         - [x] Completed item\n\
         - [ ] Pending item\n\
         \x20 - [x] Nested completed item\n\n\
         [^source]: Supporting information.\n",
    );
    for expected in [
        "This is deprecated text.1",
        "- [x] Completed item\n- [ ] Pending item\n  - [x] Nested completed item\n",
        "1. Supporting information.",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected:?} missing in:\n{markdown}"
        );
    }
}

#[expect(
    clippy::expect_used,
    reason = "test helper; a failure should abort the test"
)]
fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().expect("PNG header");
    let pixels = vec![200; usize::try_from(width * height * 3).expect("pixel count")];
    writer.write_image_data(&pixels).expect("PNG data");
    writer.finish().expect("PNG end");
    out
}

#[expect(
    clippy::expect_used,
    reason = "test helper; a failure should abort the test"
)]
fn embed(markdown: &str, images: &[(&str, Vec<u8>)]) -> pdf2md_core::Output {
    let options = md2pdf_core::Options {
        images: images
            .iter()
            .map(|(src, bytes)| ((*src).to_string(), bytes.clone()))
            .collect(),
    };
    let output = md2pdf_core::convert_with_options(markdown, &options).expect("md2pdf conversion");
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
    convert(&output.pdf, &Options::default()).expect("pdf2md conversion")
}

#[test]
fn embedded_png_comes_back_as_an_image_of_the_same_size() {
    let output = embed(
        "Before.\n\n![chart](chart.png)\n\nAfter.\n",
        &[("chart.png", png_bytes(40, 20))],
    );
    assert_eq!(output.assets.len(), 1);
    let asset = &output.assets[0];
    assert_eq!(asset.mime, "image/png");
    let reader = png::Decoder::new(std::io::Cursor::new(&asset.bytes))
        .read_info()
        .unwrap();
    assert_eq!((reader.info().width, reader.info().height), (40, 20));
    assert!(output.markdown.contains(&format!("![]({})", asset.path)));
    let before = output.markdown.find("Before.").unwrap();
    let image = output.markdown.find("![]").unwrap();
    let after = output.markdown.find("After.").unwrap();
    assert!(before < image && image < after, "{}", output.markdown);
}

#[test]
fn embedded_jpeg_is_passed_through_byte_for_byte() {
    // Frame header only: enough for md2pdf, which never decodes the scan.
    let mut jpeg = vec![
        0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x10, 0x00, 0x20, 0x03,
    ];
    jpeg.extend([1, 0x11, 0, 0xFF, 0xD9]);
    let output = embed("![photo](photo.jpg)\n", &[("photo.jpg", jpeg.clone())]);
    assert_eq!(output.assets.len(), 1);
    assert_eq!(output.assets[0].mime, "image/jpeg");
    assert_eq!(output.assets[0].bytes, jpeg);
}

#[test]
fn features_example_snapshot() {
    let options = md2pdf_core::Options {
        images: [(
            "images/diagram.png".to_string(),
            include_bytes!("../../cli/examples/images/diagram.png").to_vec(),
        )]
        .into(),
    };
    let rendered =
        md2pdf_core::convert_with_options(include_str!("../../cli/examples/features.md"), &options)
            .unwrap();
    assert_eq!(
        rendered.warnings,
        ["image images/missing.png: image not found"]
    );
    let output = convert(&rendered.pdf, &Options::default()).unwrap();
    assert_eq!(output.assets.len(), 1);
    insta::assert_snapshot!(output.markdown);
}

fn long_table(rows: usize) -> String {
    let body: Vec<String> = (1..=rows).map(|i| format!("| row{i} | {i} |")).collect();
    let body = body.join("\n");
    format!("Intro.\n\n| Name | Value |\n| :-- | --: |\n{body}\n\nAfter.\n")
}

#[test]
fn table_split_across_pages_comes_back_as_one_table() {
    let markdown = round_trip(&long_table(120));
    assert_eq!(
        markdown.matches("| Name | Value |").count(),
        1,
        "{markdown}"
    );
    assert_eq!(markdown.matches("| :-- | --: |").count(), 1);
    let rows: Vec<&str> = markdown
        .lines()
        .filter(|l| l.starts_with("| row"))
        .collect();
    let expected: Vec<String> = (1..=120).map(|i| format!("| row{i} | {i} |")).collect();
    assert_eq!(rows, expected);
    assert!(markdown.starts_with("Intro.\n\n| Name"));
    assert!(markdown.trim_end().ends_with("After."));
}

fn table_md(header: [&str; 3], prefix: &str, rows: usize) -> String {
    let body: Vec<String> = (1..=rows)
        .map(|i| format!("| {prefix}{i} | value {i} | note {i} |"))
        .collect();
    format!(
        "| {} | {} | {} |\n| --- | --- | --- |\n{}\n",
        header[0],
        header[1],
        header[2],
        body.join("\n")
    )
}

#[test]
fn back_to_back_tables_with_the_same_columns_stay_separate() {
    for rows in [26, 28, 29] {
        let md = format!(
            "{}\n{}",
            table_md(["Name", "Value", "Note"], "a", rows),
            table_md(["Nome", "Velue", "Nota"], "z", 20)
        );
        let markdown = round_trip(&md);
        assert_eq!(
            markdown.matches("| Name | Value | Note |").count(),
            1,
            "{rows}: {markdown}"
        );
        assert_eq!(
            markdown.matches("| Nome | Velue | Nota |").count(),
            1,
            "{rows}: {markdown}"
        );
        assert!(!markdown.contains("**Nome**"), "{rows}: {markdown}");
        let first_header = markdown.find("| Name |").unwrap();
        let second_header = markdown.find("| Nome |").unwrap();
        assert!(first_header < second_header, "{rows}: tables out of order");
        let a_rows: Vec<&str> = markdown.lines().filter(|l| l.starts_with("| a")).collect();
        assert_eq!(a_rows.len(), rows, "{rows}");
        let second_table = &markdown[second_header..];
        assert!(
            !second_table.contains("| a"),
            "{rows}: a-rows after the second header"
        );
    }
}
