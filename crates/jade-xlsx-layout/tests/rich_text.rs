//! Rich text runs, raised and lowered text, and `*` repeat fills.
mod common;

use common::{number, styles, workbook, SheetPart};
use jade_xlsx_layout::{
    layout_workbook_sheet, number_format::format_value, LayoutOptions, SheetLayout,
};
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue, Color, DateSystem};

const ACCOUNTING: &str = r#"_("$"* #,##0.00_);_("$"* \(#,##0.00\);_("$"* "-"??_);_(@_)"#;

#[test]
fn a_repeat_fill_is_lifted_out_of_the_text() {
    let formatted = format_value(
        &CellValue::Number(1234.5),
        Some(ACCOUNTING),
        DateSystem::Nineteen00,
    );
    assert_eq!(formatted.text, " $1,234.50 ");
    assert_eq!(formatted.fill, Some((2, ' ')));
    assert_eq!(formatted.filled(3), " $   1,234.50 ");
    let negative = format_value(
        &CellValue::Number(-50.0),
        Some(ACCOUNTING),
        DateSystem::Nineteen00,
    );
    assert_eq!(negative.text, " $(50.00)");
    let text = format_value(
        &CellValue::Text("Item".into()),
        Some("@*."),
        DateSystem::Nineteen00,
    );
    assert_eq!((text.text.as_str(), text.fill), ("Item", Some((4, '.'))));
}

fn lay_out() -> SheetLayout {
    let shared = r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><r><rPr><b/><sz val="11"/><color rgb="FFC00000"/><rFont val="Calibri"/></rPr><t>Bold</t></r><r><t xml:space="preserve"> and </t></r><r><rPr><i/><sz val="16"/><rFont val="Calibri"/></rPr><t>big</t></r></si><si><r><t>x</t></r><r><rPr><vertAlign val="superscript"/></rPr><t>2</t></r></si></sst>"#;
    let numbers = format!(
        r#"<numFmts count="2"><numFmt numFmtId="164" formatCode="{}"/><numFmt numFmtId="165" formatCode="@*."/></numFmts>"#,
        ACCOUNTING.replace('&', "&amp;").replace('"', "&quot;")
    );
    let styles_xml = styles(
        "",
        r#"<xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>"#,
        r#"<font><vertAlign val="subscript"/><sz val="11"/><name val="Calibri"/></font>"#,
    )
    .replacen("<fonts", &format!("{numbers}<fonts"), 1);
    let sheet = SheetPart::new(
        "Sheet1",
        format!(
            r#"<cols><col min="1" max="2" width="20" customWidth="1"/></cols><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" s="1">{}</c></row><row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2" s="2" t="inlineStr"><is><t>Item</t></is></c></row><row r="3"><c r="A3" s="3" t="inlineStr"><is><t>low</t></is></c><c r="B3" t="inlineStr"><is><r><rPr><u/></rPr><t>under</t></r><r><t>line</t></r></is></c></row></sheetData>"#,
            number("B1", 1234.5)
                .replace(r#"<c r="B1">"#, "")
                .replace("</c>", "")
        ),
    );
    let book = read_xlsx(&workbook(
        vec![sheet],
        &styles_xml,
        &[("xl/sharedStrings.xml", shared)],
    ))
    .unwrap();
    layout_workbook_sheet(&book, 0, &LayoutOptions::default()).unwrap()
}

fn at(layout: &SheetLayout, name: &str) -> jade_xlsx_layout::PlacedCell {
    layout.cell(CellRef::parse(name).unwrap()).unwrap().clone()
}

#[test]
fn shared_and_inline_runs_keep_their_fonts() {
    let layout = lay_out();
    let cell = at(&layout, "A1");
    assert_eq!(cell.text, "Bold and big");
    assert_eq!(cell.runs.len(), 3);
    assert_eq!(&cell.text[cell.runs[0].start..cell.runs[0].end], "Bold");
    assert!(cell.runs[0].font.bold);
    assert_eq!(cell.runs[0].color, Some(Color::Argb([255, 192, 0, 0])));
    // A run without its own font takes the cell's.
    assert!(!cell.runs[1].font.bold);
    assert_eq!(cell.runs[1].color, None);
    assert!(cell.runs[2].font.italic);
    assert_eq!(cell.runs[2].font.size_points, 16.0);
    let inline = at(&layout, "B3");
    assert!(inline.runs[0].font.underline && !inline.runs[1].font.underline);
}

#[test]
fn raised_and_lowered_text() {
    let layout = lay_out();
    let power = at(&layout, "A2");
    assert_eq!(
        power.runs[1].font.vert_align.as_deref(),
        Some("superscript")
    );
    assert_eq!(power.runs[0].font.vert_align, None);
    // A cell whose own font is lowered draws as one lowered run.
    let low = at(&layout, "A3");
    assert_eq!(low.runs.len(), 1);
    assert_eq!(low.runs[0].font.vert_align.as_deref(), Some("subscript"));
}

#[test]
fn repeat_fills_span_the_cell() {
    let layout = lay_out();
    let money = at(&layout, "B1");
    assert!(money.lines[0].starts_with(" $  "), "{:?}", money.lines);
    assert!(money.lines[0].ends_with("1,234.50 "));
    let dots = at(&layout, "B2");
    assert!(dots.lines[0].starts_with("Item...."), "{:?}", dots.lines);
    assert_eq!(dots.text, "Item");
}
