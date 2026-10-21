//! What printing asks of the export: each page's section and number, for
//! ranges such as `p1s2-p3s3`, and the document with or without its markup.

mod common;

#[test]
fn every_page_has_its_section_and_number() {
    let document = common::open_fixture("public/fixtures/header-footer-sectionpages.docx");
    let labels = document.page_labels();
    assert_eq!(labels.len(), document.page_count());
    assert!(labels.len() >= 2);
    // Sections only go forward, and numbers count up within one.
    for pair in labels.windows(2) {
        assert!(pair[1].0 >= pair[0].0);
        if pair[1].0 == pair[0].0 {
            assert_eq!(pair[1].1, pair[0].1 + 1);
        }
    }
    assert_eq!(labels[0].1, 1);
}

#[test]
fn markup_prints_only_when_asked() {
    let document = common::open_fixture("public/fixtures/review.docx");
    let pdf = |markup| {
        document
            .export_pdf_with_options(jade_docx_native_render::PdfExportOptions {
                rendering: jade_docx_native_render::PdfRenderingMode::Vector,
                markup: Some(markup),
                ..Default::default()
            })
            .unwrap()
    };
    let with = pdf(true);
    let without = pdf(false);
    assert_ne!(
        with, without,
        "the final document prints differently from the marked-up one"
    );
}
