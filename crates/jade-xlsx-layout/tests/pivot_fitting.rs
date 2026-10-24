use jade_xlsx_layout::{pivot_fit::fit_columns, units, LayoutOptions, ResolvedFont, TextMeasure};
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue, OpcPackage, Workbook};
use std::{collections::BTreeSet, rc::Rc};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-style-compat.xlsx");
struct Fixed;
impl TextMeasure for Fixed {
    fn measure(&self, text: &str, font: &ResolvedFont) -> f64 {
        text.chars().count() as f64 * if font.bold { 20. } else { 10. }
    }
}
fn options() -> LayoutOptions {
    LayoutOptions {
        measure: Rc::new(Fixed),
        ..Default::default()
    }
}
fn widths(b: &Workbook) -> std::collections::BTreeMap<u32, f32> {
    fit_columns(b, &BTreeSet::from([7]), &options())
        .unwrap()
        .remove("Report")
        .unwrap()
}
fn px(w: f32) -> f64 {
    units::column_width_chars_to_px(w, 7.)
}
fn edit(bytes: &[u8], part: &str, f: impl FnOnce(String) -> String) -> Vec<u8> {
    let mut p = OpcPackage::open(bytes).unwrap();
    let xml = p.required_part_str(part).unwrap();
    p.replace(part, f(xml).into_bytes());
    p.write().unwrap()
}
#[test]
fn full_values_use_own_fonts_and_ignore_the_corner_caption_and_unrelated_cells() {
    let bytes = edit(FIXTURE, "xl/worksheets/sheet2.xml", |s| {
        s.replace("Territory",&"Corner".repeat(100)).replace("</sheetData>", &format!("<row r=\"20\"><c r=\"E20\" t=\"inlineStr\"><is><t>{}</t></is></c></row></sheetData>","Outside".repeat(100)))
    });
    let mut b = read_xlsx(&bytes).unwrap();
    let w = widths(&b);
    assert_eq!(px(w[&4]), 116.); // Grand Total: 11 glyphs +6px padding.
    assert_eq!(px(w[&5]), 126.); // Total Amount: 12 glyphs, no automatic bold.
    b.styles.fonts[0].bold = true;
    assert_eq!(px(widths(&b)[&5]), 246.); // Explicit cell font is measured.
}
#[test]
fn full_number_format_is_measured_before_numeric_overflow_substitution() {
    let mut b = read_xlsx(FIXTURE).unwrap();
    let cell = b.sheets[1]
        .rows
        .iter_mut()
        .flat_map(|r| &mut r.cells)
        .find(|c| c.reference == CellRef::parse("F4").unwrap())
        .unwrap();
    cell.value = CellValue::Number(123456789012.);
    b.styles.cell_formats[1].number_format_id = Some(164);
    let w = widths(&b);
    assert_eq!(px(w[&5]), 156.);
}
#[test]
fn report_filters_reserve_dropdown_room_without_measuring_the_gap_row() {
    let bytes = edit(FIXTURE, "xl/worksheets/sheet2.xml", |s| {
        s.replace("<sheetData>","<sheetData><row r=\"1\"><c r=\"E1\" t=\"inlineStr\"><is><t>Filter Label Text</t></is></c><c r=\"F1\" t=\"inlineStr\"><is><t>Filter Value Text</t></is></c></row><row r=\"2\"><c r=\"F2\" t=\"inlineStr\"><is><t>A gap row that must never drive a pivot filter width</t></is></c></row>")
    });
    let b = read_xlsx(&bytes).unwrap();
    let w = widths(&b);
    assert_eq!(px(w[&4]), 176.); //17 characters + padding.
    assert_eq!(px(w[&5]), 196.); //17 characters +20px button + padding.
}
#[test]
fn hidden_and_zero_width_columns_stay_hidden_and_tabular_first_labels_keep_their_width() {
    let bytes = edit(FIXTURE, "xl/worksheets/sheet2.xml", |s| {
        s.replace("<sheetData>","<cols><col min=\"5\" max=\"5\" hidden=\"1\" width=\"20\"/><col min=\"6\" max=\"6\" width=\"0\"/></cols><sheetData>")
    });
    assert!(fit_columns(
        &read_xlsx(&bytes).unwrap(),
        &BTreeSet::from([7]),
        &options()
    )
    .unwrap()
    .is_empty());
    let mut b = read_xlsx(FIXTURE).unwrap();
    std::sync::Arc::make_mut(&mut b.pivots.tables[0])
        .attributes
        .insert("compact".into(), "0".into());
    assert!(!widths(&b).contains_key(&4));
    assert!(widths(&b).contains_key(&5));
}
#[test]
fn multiple_refreshed_reports_share_the_widest_requirement_and_invalid_metrics_refuse() {
    let mut b = read_xlsx(FIXTURE).unwrap();
    let mut other = b.pivots.tables[0].as_ref().clone();
    other.cache_id = 9;
    other.name = "Other".into();
    other.location.reference.start.row = 19;
    other.location.reference.end.row = 22;
    let bytes = edit(FIXTURE, "xl/worksheets/sheet2.xml", |s| {
        s.replace("</sheetData>","<row r=\"21\"><c r=\"F21\" t=\"inlineStr\"><is><t>A much wider second report cell</t></is></c></row></sheetData>")
    });
    b.sheets = read_xlsx(&bytes).unwrap().sheets;
    b.pivots.tables.push(std::sync::Arc::new(other));
    let all = fit_columns(&b, &BTreeSet::from([7, 9]), &options()).unwrap();
    assert_eq!(px(all["Report"][&5]), 316.);
    struct Bad;
    impl TextMeasure for Bad {
        fn measure(&self, _: &str, _: &ResolvedFont) -> f64 {
            f64::NAN
        }
    }
    let o = LayoutOptions {
        measure: Rc::new(Bad),
        ..Default::default()
    };
    assert!(fit_columns(&b, &BTreeSet::from([7, 9]), &o).is_err());
    let overflow = LayoutOptions {
        cell_padding_px: f64::MAX,
        ..options()
    };
    assert!(fit_columns(&b, &BTreeSet::from([7, 9]), &overflow).is_err());
}
