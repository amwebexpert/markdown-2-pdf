//! PDFs from other producers (see `fixtures/README.md` for provenance).

use pdf2md_core::{ImageMode, Options, convert};

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

#[test]
fn chrome_media() {
    let options = Options {
        images: ImageMode::Assets {
            dir: "media_assets".into(),
        },
        ..Options::default()
    };
    let output = convert(
        include_bytes!("fixtures/external/chrome-media.pdf"),
        &options,
    )
    .unwrap();
    // The logo is drawn twice but stored once.
    let mimes: Vec<&str> = output.assets.iter().map(|a| a.mime).collect();
    assert_eq!(mimes, ["image/png", "image/png", "image/jpeg"]);
    assert_eq!(
        output
            .markdown
            .matches("![](media_assets/img-001.png)")
            .count(),
        2
    );
    for asset in output.assets.iter().filter(|a| a.mime == "image/png") {
        let reader = png::Decoder::new(std::io::Cursor::new(&asset.bytes))
            .read_info()
            .unwrap();
        assert!(reader.info().width > 0);
    }
    let alpha = output.assets.get(1).unwrap();
    let info = png::Decoder::new(std::io::Cursor::new(&alpha.bytes))
        .read_info()
        .unwrap()
        .info()
        .color_type;
    assert_eq!(info, png::ColorType::Rgba);
    assert!(
        output
            .markdown
            .contains("| Checked weekly<br>by the field team |")
    );
    insta::assert_snapshot!(output.markdown);
}

#[test]
fn arxiv_booktabs_table() {
    let md = markdown(include_bytes!("fixtures/external/arxiv-2610.40745-p9.pdf"));
    // Table I (REVTeX rules, no vertical lines) becomes a four-column GFM table.
    assert!(md.contains(
        "| Activity | Number | Administrator | Researcher |\n| :-- | :-- | :-- | :-- |\n"
    ));
    insta::assert_snapshot!(md);
}
