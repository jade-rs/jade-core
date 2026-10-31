use jade_xlsx_headless_editor::{BorderLine, BorderStyle, FormatChange, HeadlessWorkbook};
use jade_xlsx_reader::{HorizontalAlignment, OpcPackage, VerticalAlignment};
const SOURCE: &[u8] = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");

#[test]
fn source_format_query_resolves_axis_defaults_and_history_without_writing() {
    let mut book = HeadlessWorkbook::open(SOURCE).unwrap();
    let before = book.bytes().to_vec();
    let initial = book.format_at("Operations", "B100").unwrap();
    assert_eq!(initial.bold, Some(false));
    assert_eq!(initial.number_format.as_deref(), Some("General"));
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    let change = FormatChange {
        locked: Some(false),
        formula_hidden: Some(true),
        bold: Some(true),
        italic: Some(true),
        underline: Some(true),
        strike: Some(true),
        font_name: Some("Arial".into()),
        font_size: Some(18.5),
        font_color: Some("123456".into()),
        fill_color: Some("ABCDEF".into()),
        number_format: Some("0.00%".into()),
        horizontal: Some(HorizontalAlignment::Center),
        vertical: Some(VerticalAlignment::Top),
        wrap_text: Some(true),
        shrink_to_fit: Some(false),
        indent: Some(0),
        text_rotation: Some(45),
        border_left: Some(BorderLine {
            style: BorderStyle::Thin,
            color: Some("FF0000".into()),
        }),
        border_right: Some(BorderLine {
            style: BorderStyle::MediumDashed,
            color: None,
        }),
        border_top: Some(BorderLine::none()),
        border_bottom: Some(BorderLine {
            style: BorderStyle::Double,
            color: Some("0070C0".into()),
        }),
        border_diagonal: Some(BorderLine {
            style: BorderStyle::Hair,
            color: Some("00B050".into()),
        }),
        diagonal_up: Some(true),
        diagonal_down: Some(false),
    };
    book.set_format("Operations", "B:B", &change).unwrap();
    assert_eq!(book.format_at("Operations", "B100").unwrap(), change);
    let formatted = book.bytes().to_vec();
    let reopened = HeadlessWorkbook::open(&formatted).unwrap();
    assert_eq!(reopened.format_at("Operations", "B100").unwrap(), change);
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    assert_eq!(book.format_at("Operations", "B100").unwrap(), initial);
    book.redo().unwrap();
    assert_eq!(book.bytes(), formatted);
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("format-view-axis.xlsx"),
            formatted,
        )
        .unwrap();
    }
}

#[test]
fn merged_cells_use_the_anchor_and_invalid_queries_do_not_change_bytes() {
    let mut package = OpcPackage::open(SOURCE).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    let xml = xml.replace(
        "<mergeCells count=\"1\">",
        "<mergeCells count=\"2\"><mergeCell ref=\"H20:I21\"/>",
    );
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    book.set_format(
        "Operations",
        "H20",
        &FormatChange {
            bold: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        book.format_at("Operations", "I21").unwrap().bold,
        Some(true)
    );
    let before = book.bytes().to_vec();
    for reference in ["A0", "XFE1", "A1048577", "A1:B2"] {
        assert!(book.format_at("Operations", reference).is_err());
    }
    assert!(book.format_at("missing", "A1").is_err());
    assert_eq!(book.bytes(), before);
}

#[test]
fn conditional_painted_styles_do_not_replace_the_editable_base_format() {
    let mut base = HeadlessWorkbook::open(SOURCE).unwrap();
    base.set_format(
        "Operations",
        "B3",
        &FormatChange {
            bold: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    let mut package = OpcPackage::open(base.bytes()).unwrap();
    let styles = package.part_str("xl/styles.xml").unwrap().unwrap();
    let styles = if styles.contains("<dxfs") {
        styles.replace(
            "<dxfs count=\"0\"/>",
            "<dxfs count=\"1\"><dxf><font><b/><color rgb=\"FFEE0000\"/></font></dxf></dxfs>",
        )
    } else {
        styles.replace("</styleSheet>","<dxfs count=\"1\"><dxf><font><b/><color rgb=\"FFEE0000\"/></font></dxf></dxfs></styleSheet>")
    };
    package.replace("xl/styles.xml", styles.into_bytes());
    let xml=package.part_str("xl/worksheets/sheet1.xml").unwrap().unwrap().replace("</worksheet>","<conditionalFormatting sqref=\"B3\"><cfRule type=\"cellIs\" dxfId=\"0\" priority=\"1\" operator=\"greaterThan\"><formula>0</formula></cfRule></conditionalFormatting></worksheet>");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let layout = jade_xlsx_layout::layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &Default::default(),
    );
    assert!(
        layout
            .cell(jade_xlsx_reader::CellRef::parse("B3").unwrap())
            .unwrap()
            .font
            .bold
    );
    let base = book.format_at("Operations", "B3").unwrap();
    assert_eq!(base.bold, Some(false));
    assert_ne!(base.font_color.as_deref(), Some("EE0000"));
}

#[test]
fn invalid_font_sizes_fail_atomically_for_cells_and_complete_axes() {
    for range in ["B3", "B:B", "3:3", "A1:XFD1048576"] {
        for size in [0.0, -1.0, 410.0, f32::NAN, f32::INFINITY] {
            let mut book = HeadlessWorkbook::open(SOURCE).unwrap();
            let before = book.bytes().to_vec();
            let error = book
                .set_format(
                    "Operations",
                    range,
                    &FormatChange {
                        font_size: Some(size),
                        bold: Some(true),
                        ..Default::default()
                    },
                )
                .unwrap_err();
            assert!(error.is_invalid_input());
            assert_eq!(book.bytes(), before);
            assert!(!book.can_undo());
        }
    }
}

#[test]
fn theme_and_pattern_colors_are_read_without_rewriting_their_source() {
    let mut book = HeadlessWorkbook::open(SOURCE).unwrap();
    book.set_format(
        "Operations",
        "B3",
        &FormatChange {
            font_color: Some("123456".into()),
            fill_color: Some("ABCDEF".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    let styles = package
        .part_str("xl/styles.xml")
        .unwrap()
        .unwrap()
        .replace("rgb=\"FF123456\"", "theme=\"1\"")
        .replace("patternType=\"solid\"", "patternType=\"darkGrid\"");
    package.replace("xl/styles.xml", styles.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    let format = book.format_at("Operations", "B3").unwrap();
    assert_eq!(format.font_color.as_deref(), Some("000000"));
    assert_eq!(format.fill_color, None);
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    book.set_format(
        "Operations",
        "B3",
        &FormatChange {
            number_format: Some("0.00".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let package = OpcPackage::open(book.bytes()).unwrap();
    let xml = package.part_str("xl/styles.xml").unwrap().unwrap();
    assert!(xml.contains("theme=\"1\""));
    assert!(xml.contains("patternType=\"darkGrid\""));
    assert_eq!(book.format_at("Operations", "B3").unwrap().fill_color, None);
}
