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
