//! PDFs from other producers (see `fixtures/README.md` for provenance).

use pdf2md_core::{Options, convert};

#[expect(
    clippy::expect_used,
    reason = "test helper; a failure should abort the test"
)]
fn markdown(pdf: &[u8]) -> String {
    convert(pdf, &Options::default())
        .expect("pdf2md conversion")
        .markdown
}

#[test]
fn chrome_handbook() {
    let md = markdown(include_bytes!("fixtures/external/chrome-handbook.pdf"));
    // Vector-drawn bullets and nesting.
    assert!(md.contains("- Pack the notebook and two pencils\n  - graphite, not ink\n"));
    // The heading after the two-column section comes after all column text.
    let returning = md.find("## Returning").unwrap();
    assert!(md.find("8. Field notes").unwrap() < returning);
    insta::assert_snapshot!(md);
}

#[test]
fn arxiv_two_column_paper() {
    let md = markdown(include_bytes!(
        "fixtures/external/arxiv-2610.40745-p1-3.pdf"
    ));
    // Left column text is read before the right column, not interleaved.
    assert!(md.contains(
        "**Keywords:** author disambiguation; research information management; CRIS; ORCID; OpenAlex; Dimensions"
    ));
    insta::assert_snapshot!(md);
}
