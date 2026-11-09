use jade_opc::OpcPackage;
use jade_xlsx_reader::CellRef;

const CHART: &str = "ppt/charts/chart1.xml";
const BOOK: &str = "ppt/embeddings/Microsoft_Excel_Worksheet.xlsx";
fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/poi/chart-slide-bg.pptx"
    ))
    .unwrap()
}
fn patch(bytes: &[u8], part: &str, update: impl FnOnce(String) -> String) -> Vec<u8> {
    let mut p = OpcPackage::open(bytes).unwrap();
    let s = update(p.required_part_str(part).unwrap());
    p.replace(part, s.into_bytes());
    p.write().unwrap()
}

#[test]
fn cacheless_value_labels_follow_cell_formats_unless_overridden() {
    use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure};
    use jade_xlsx_writer::{
        add_cell_format, styles_part, write_xlsx_with_styles, CellEdit, FormatChange, NewValue,
        SheetEdits,
    };

    let mut package = OpcPackage::open(&fixture()).unwrap();
    let book = patch(package.part(BOOK).unwrap(), "xl/workbook.xml", |xml| {
        xml.replace("<workbookPr ", "<workbookPr date1904=\"1\" ")
    });
    let (styles_part, mut styles) = styles_part(&book).unwrap();
    let mut cells = Vec::new();
    for (address, value, code) in [
        ("B2", 0.126, "0.0%"),
        ("B3", 12.5, "\"$\"0.00"),
        ("B4", 43830.0, "yyyy-mm-dd"),
    ] {
        let (updated, style_index) = add_cell_format(
            &styles,
            None,
            &FormatChange {
                number_format: Some(code.into()),
                ..Default::default()
            },
        )
        .unwrap();
        styles = updated;
        cells.push(CellEdit {
            reference: CellRef::parse(address).unwrap(),
            value: NewValue::Number(value),
            style_index: Some(style_index),
        });
    }
    let book = write_xlsx_with_styles(
        &book,
        Some((&styles_part, &styles)),
        &[SheetEdits {
            part_name: "xl/worksheets/sheet1.xml".into(),
            cells,
        }],
    )
    .unwrap();
    package.replace(BOOK, book);

    const LINKED: &str = r#"<c:numFmt formatCode="0.00" sourceLinked="1"/>"#;
    let bytes = patch(&package.write().unwrap(), CHART, |mut xml| {
        let start = xml.find("<c:numCache>").unwrap();
        let end = start + xml[start..].find("</c:numCache>").unwrap() + "</c:numCache>".len();
        xml.replace_range(start..end, "");
        xml.replace("<c:dLbls>", &format!("<c:dLbls>{LINKED}"))
            .replace("<c:showVal val=\"0\"/>", "<c:showVal val=\"1\"/>")
    });
    let assert_labels = |bytes: &[u8], expected: &[&str]| {
        let deck = jade_pptx_reader::read_deck(bytes).unwrap();
        let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
        for (point, expected) in expected.iter().enumerate() {
            let suffix = format!("Series 1 label {point}");
            let label = layout
                .shapes
                .iter()
                .find(|shape| shape.name.ends_with(&suffix))
                .unwrap();
            let text: String = label
                .lines
                .iter()
                .flat_map(|line| &line.runs)
                .map(|run| run.text.as_str())
                .collect();
            assert_eq!(text, *expected);
        }
        assert!(layout
            .shapes
            .iter()
            .any(|shape| shape.name.ends_with("value 0.0%")));
    };
    assert_labels(&bytes, &["12.6%", "$12.50", "2024-01-01"]);

    let implicit = patch(&bytes, CHART, |xml| {
        xml.replace(LINKED, r#"<c:numFmt formatCode="0.00"/>"#)
    });
    assert_labels(&implicit, &["12.6%", "$12.50", "2024-01-01"]);

    let unlinked = patch(&bytes, CHART, |xml| {
        xml.replace(LINKED, r#"<c:numFmt formatCode="0.00" sourceLinked="0"/>"#)
    });
    assert_labels(&unlinked, &["0.13", "12.50", "43830.00"]);
}
