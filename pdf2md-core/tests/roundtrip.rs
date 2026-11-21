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
fn strikethrough_task_lists_and_footnotes_keep_their_text() {
    let markdown = round_trip(
        "This is ~~deprecated~~ text.[^source]\n\n\
         - [x] Completed item\n\
         - [ ] Pending item\n\
         \x20 - [x] Nested completed item\n\n\
         [^source]: Supporting information.\n",
    );
    for expected in [
        "This is deprecated text.1",
        "Completed item",
        "Pending item",
        "Nested completed item",
        "1. Supporting information.",
    ] {
        assert!(
            markdown.contains(expected),
            "{expected:?} missing in:\n{markdown}"
        );
    }
    // The checkboxes are vector boxes, not text.
    assert!(!markdown.contains("[x]") && !markdown.contains("[ ]"));
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
