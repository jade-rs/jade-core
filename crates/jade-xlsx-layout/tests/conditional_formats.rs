use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{read_xlsx, CellRef, Color};

fn layout() -> jade_xlsx_layout::SheetLayout {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/conditional-formats-workbook.xlsx"
    ))
    .unwrap();
    layout_sheet(
        &book.sheets[0],
        &book.styles,
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            ..Default::default()
        },
    )
}

#[test]
fn rules_compose_per_property_in_priority_order_and_stop_when_true() {
    let layout = layout();
    let a1 = layout.cell(CellRef::parse("A1").unwrap()).unwrap();
    assert!(!a1.font.bold);
    assert!(a1.font.italic);
    assert_eq!(a1.text_color, Some(Color::Argb([255, 0, 97, 0])));
    assert_eq!(
        a1.fill.as_ref().unwrap().foreground,
        Some(Color::Argb([255, 198, 239, 206]))
    );
    let a2 = layout.cell(CellRef::parse("A2").unwrap()).unwrap();
    assert_eq!(a2.text, "50.00");
    assert!(a2.font.bold);
    assert_eq!(
        a2.fill.as_ref().unwrap().foreground,
        Some(Color::Argb([255, 255, 199, 206]))
    );
    let a3 = layout.cell(CellRef::parse("A3").unwrap()).unwrap();
    assert_eq!(a3.text, "$120.00");
    assert!(a3.border.bottom.is_some());
    let b1 = layout.cell(CellRef::parse("B1").unwrap()).unwrap();
    assert_eq!(b1.text, "80.00");
    assert!(b1.border.bottom.is_none());
    let b2 = layout.cell(CellRef::parse("B2").unwrap()).unwrap();
    assert!(!b2.font.bold);
    assert!(b2.font.italic);
    assert_eq!(b2.text_color, Some(Color::Argb([255, 156, 0, 6])));
}

#[test]
fn blank_rules_paint_unstored_cells_without_materializing_a_million_rows() {
    let layout = layout();
    assert_eq!(layout.rows.len(), 30);
    assert!(layout.cells.len() < 20);
    let cells = layout.conditional_blank_cells_in(Rect {
        x: 0.0,
        y: 0.0,
        width: 600.0,
        height: 300.0,
    });
    assert!(cells
        .iter()
        .any(|cell| cell.reference == CellRef::parse("D2").unwrap()));
    assert!(cells
        .iter()
        .any(|cell| cell.reference == CellRef::parse("D10").unwrap()));
    assert!(cells.len() < 15);
    // The blank merge is one placed cell, never painted again as its follower.
    let merge = layout.cell(CellRef::parse("D4").unwrap()).unwrap();
    assert!(merge.fill.is_some());
    assert!(!cells
        .iter()
        .any(|cell| cell.reference == CellRef::parse("E4").unwrap()));
}

#[test]
fn comparison_operators_and_text_wildcards_obey_rule_semantics() {
    use jade_xlsx_layout::conditional_format::matches_rule;
    use jade_xlsx_reader::{conditional_format::ConditionalRule, CellValue};
    let mut rule = ConditionalRule {
        rule_type: "cellIs".into(),
        priority: 1,
        differential_style_index: None,
        stop_if_true: false,
        operator: Some("between".into()),
        text: None,
        formulas: vec!["10".into(), "20".into()],
        visual: None,
        issues: Vec::new(),
    };
    assert_eq!(matches_rule(&rule, &CellValue::Number(10.0)), Some(true));
    assert_eq!(matches_rule(&rule, &CellValue::Number(20.0)), Some(true));
    assert_eq!(matches_rule(&rule, &CellValue::Number(21.0)), Some(false));
    rule.operator = Some("notBetween".into());
    assert_eq!(matches_rule(&rule, &CellValue::Number(21.0)), Some(true));
    rule.formulas = vec!["$B$1".into()];
    assert_eq!(matches_rule(&rule, &CellValue::Number(21.0)), None);
    rule.rule_type = "containsText".into();
    rule.text = Some("w?rn*".into());
    assert_eq!(
        matches_rule(&rule, &CellValue::Text("new WARNING".into())),
        Some(true)
    );
    rule.text = Some("~*".into());
    assert_eq!(
        matches_rule(&rule, &CellValue::Text("literal * star".into())),
        Some(true)
    );
    assert_eq!(
        matches_rule(&rule, &CellValue::Text("plain".into())),
        Some(false)
    );
}

#[test]
fn an_unstored_font_only_rule_is_available_to_the_cell_editor() {
    use jade_xlsx_reader::{
        conditional_format::{ConditionalFormatting, ConditionalRule},
        CellRange,
    };
    let mut book = read_xlsx(include_bytes!(
        "../../../public/fixtures/conditional-formats-workbook.xlsx"
    ))
    .unwrap();
    book.sheets[0].conditional_formats.insert(
        0,
        ConditionalFormatting {
            ranges: vec![CellRange::parse("D2").unwrap()],
            rules: vec![ConditionalRule {
                rule_type: "containsBlanks".into(),
                priority: 0,
                differential_style_index: Some(2),
                stop_if_true: true,
                operator: None,
                text: None,
                formulas: Vec::new(),
                visual: None,
                issues: Vec::new(),
            }],
        },
    );
    let layout = layout_sheet(
        &book.sheets[0],
        &book.styles,
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            ..Default::default()
        },
    );
    let cell = layout.styled_cell(CellRef::parse("D2").unwrap()).unwrap();
    assert!(cell.font.italic);
    assert!(!cell.font.bold);
    assert!(cell.fill.is_none());
}
