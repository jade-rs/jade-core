mod common;

use jade_xlsx_reader::{
    read_xlsx, CellRef, CellValue, Color, HorizontalAlignment, SheetVisibility, VerticalAlignment,
};

fn cell_value(sheet: &jade_xlsx_reader::Sheet, reference: &str) -> CellValue {
    let target = CellRef::parse(reference).expect("reference");
    sheet
        .rows
        .iter()
        .find(|row| row.index == target.row)
        .and_then(|row| {
            row.cells
                .iter()
                .find(|cell| cell.reference.column == target.column)
        })
        .map(|cell| cell.value.clone())
        .unwrap_or(CellValue::Empty)
}

#[test]
fn reads_sheets_in_workbook_order() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let names: Vec<&str> = workbook
        .sheets
        .iter()
        .map(|sheet| sheet.name.as_str())
        .collect();
    assert_eq!(names, ["売上", "Sheet2"]);
    assert_eq!(workbook.sheets[0].part_name, "xl/worksheets/sheet1.xml");
    assert_eq!(workbook.sheets[0].visibility, SheetVisibility::Visible);
    assert_eq!(workbook.sheets[1].visibility, SheetVisibility::VeryHidden);
    assert_eq!(workbook.active_sheet_index, Some(0));
    assert!(workbook.full_calculation_on_load);
}

#[test]
fn reads_global_local_and_hidden_defined_names() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");

    assert_eq!(workbook.defined_names.len(), 4);
    let global = &workbook.defined_names[0];
    assert_eq!(global.name, "SalesData");
    assert_eq!(global.formula, "売上!$A$1:$C$8");
    assert_eq!(global.local_sheet_index, None);
    assert!(!global.hidden);

    let local = &workbook.defined_names[1];
    assert_eq!(local.name, "LocalTax");
    assert_eq!(local.local_sheet_index, Some(0));

    let internal = &workbook.defined_names[2];
    assert_eq!(internal.name, "_xlnm.Print_Area");
    assert!(internal.hidden);

    assert_eq!(workbook.defined_names[3].formula, "\"A & B\"");
}

#[test]
fn resolves_the_four_cell_value_kinds() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");

    assert_eq!(cell_value(sheet, "A1"), CellValue::Text("Item".into()));
    // Two `<r>` runs in one `<si>` are one value.
    assert_eq!(
        cell_value(sheet, "B1"),
        CellValue::Text("Total (JPY)".into())
    );
    assert_eq!(cell_value(sheet, "C1"), CellValue::Text("Note".into()));
    assert_eq!(cell_value(sheet, "B2"), CellValue::Number(1234.5));
    assert_eq!(cell_value(sheet, "C2"), CellValue::Bool(true));
    assert_eq!(cell_value(sheet, "C3"), CellValue::Error("#DIV/0!".into()));
}

#[test]
fn phonetic_guides_stay_out_of_the_value() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");
    // `<rPh>` holds the reading Excel shows above the text; joining it in would
    // read as "横浜ヨコハマ".
    assert_eq!(cell_value(sheet, "A2"), CellValue::Text("横浜".into()));
}

#[test]
fn preserves_significant_whitespace() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");
    assert_eq!(
        cell_value(sheet, "A3"),
        CellValue::Text("  padded  ".into())
    );
}

#[test]
fn keeps_the_formula_and_the_cached_value() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");
    let cell = sheet.rows[2]
        .cells
        .iter()
        .find(|cell| cell.reference.column == 1)
        .expect("B3");
    assert_eq!(cell.formula.as_deref(), Some("SUM(B2:B2)"));
    // Nothing recalculates yet, so the cached value is what renders.
    assert_eq!(cell.value, CellValue::Number(1234.5));
}

#[test]
fn expands_shared_formulas_for_the_cells_that_omit_the_expression() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");
    let follower = sheet.rows[7]
        .cells
        .iter()
        .find(|cell| cell.reference.column == 2)
        .expect("C8");

    // Relative rows move with the shared formula. Absolute axes and text that
    // merely looks like a reference stay untouched.
    assert_eq!(
        follower.formula.as_deref(),
        Some("SUM(A8:B8)+$A8+A$1+$A$1+\"A7\"+LOG10(A8)+'Sheet 1'!A8+Sheet2!A8")
    );
    assert_eq!(follower.value, CellValue::Number(11.0));
}

#[test]
fn reads_column_widths_row_heights_and_merges() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");

    let first = sheet.column_span(0).expect("column A");
    assert_eq!(first.width_chars, Some(20.5));
    assert!(first.custom_width);

    // A `<col>` that only hides a column carries a width but no `customWidth`.
    let hidden = sheet.column_span(3).expect("column D");
    assert!(hidden.hidden);
    assert!(!hidden.custom_width);

    assert_eq!(sheet.rows[0].height_points, Some(30.0));
    assert!(sheet.rows[0].custom_height);
    assert_eq!(sheet.rows[1].height_points, None);

    // Excel writes the exact binary fraction it holds, which is more digits
    // than f32 prints back; the fixture keeps it verbatim rather than a rounded
    // stand-in.
    #[allow(clippy::excessive_precision)]
    let default_width = 8.83203125_f32;
    assert_eq!(sheet.default_column_width_chars, Some(default_width));
    assert_eq!(sheet.default_row_height_points, Some(18.75));

    let merge = sheet
        .merge_at(CellRef::parse("B4").expect("ref"))
        .expect("merge");
    assert_eq!(merge.start, CellRef::new(0, 3));
    assert_eq!(merge.end, CellRef::new(2, 3));
}

#[test]
fn resolves_styles_inherited_from_columns_and_rows() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");

    // A cell style wins, then a row style explicitly marked customFormat,
    // then the column's style. This is how Excel avoids repeating `s` on
    // every cell in a uniformly formatted row or column.
    assert_eq!(
        sheet.effective_style_index(CellRef::new(1, 3), None),
        Some(2)
    );
    assert_eq!(
        sheet.effective_style_index(CellRef::new(2, 4), None),
        Some(2)
    );
    assert_eq!(sheet.effective_style_index(CellRef::new(2, 5), None), None);
    assert_eq!(
        sheet.effective_style_index(CellRef::new(1, 4), Some(1)),
        Some(1)
    );

    assert!(sheet.rows[4].custom_format);
    assert!(!sheet.rows[5].custom_format);
}

#[test]
fn reads_the_style_tables() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let styles = &workbook.styles;

    let heading = styles.cell_format(Some(1)).expect("xf 1");
    assert_eq!(heading.alignment.horizontal, HorizontalAlignment::Center);
    assert_eq!(heading.alignment.vertical, VerticalAlignment::Center);
    assert!(heading.alignment.wrap_text);
    assert!(heading.alignment.shrink_to_fit);

    let font = styles.font(heading).expect("font");
    assert!(font.bold);
    assert_eq!(font.size_points, Some(14.0));
    assert_eq!(font.name.as_deref(), Some("Meiryo"));
    assert_eq!(font.color, Some(Color::Argb([0xFF, 0xCC, 0x00, 0x00])));

    let fill = styles.fill(heading).expect("fill");
    assert_eq!(fill.pattern.as_deref(), Some("solid"));
    assert_eq!(fill.foreground, Some(Color::Argb([0xFF, 0xFF, 0xFF, 0x00])));

    let border = styles.border(heading).expect("border");
    assert_eq!(
        border.top.as_ref().map(|edge| edge.style.as_str()),
        Some("medium")
    );
    assert_eq!(
        border.left.as_ref().map(|edge| edge.style.as_str()),
        Some("thin")
    );

    // The default border entry has every side present but styleless, which is
    // "no border" rather than a thin one.
    let plain = styles.cell_format(Some(0)).expect("xf 0");
    let plain_border = styles.border(plain).expect("border 0");
    assert!(plain_border.top.is_none());
    assert!(plain_border.left.is_none());

    let currency = styles.cell_format(Some(2)).expect("xf 2");
    assert_eq!(styles.number_format_code(currency), Some("#,##0.00\"円\""));
    // Built-in ids are not written to the file but still resolve.
    assert_eq!(styles.number_format_code(plain), Some("General"));
}

#[test]
fn reads_cell_indent_levels() {
    let workbook = read_xlsx(include_bytes!(
        "../../../public/fixtures/number-formats-workbook.xlsx"
    ))
    .expect("fixture");
    let reference = CellRef::parse("C15").expect("C15");
    let cell = workbook.sheets[0]
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .find(|cell| cell.reference == reference)
        .expect("C15");
    let format = workbook
        .styles
        .cell_format(cell.style_index)
        .expect("C15 format");

    assert_eq!(format.alignment.horizontal, HorizontalAlignment::Left);
    assert_eq!(format.alignment.indent, 2);
}

#[test]
fn reads_cell_text_rotation() {
    let workbook = read_xlsx(include_bytes!(
        "../../../public/fixtures/number-formats-workbook.xlsx"
    ))
    .expect("fixture");
    let reference = CellRef::parse("C16").expect("C16");
    let cell = workbook.sheets[0]
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .find(|cell| cell.reference == reference)
        .expect("C16");
    let format = workbook
        .styles
        .cell_format(cell.style_index)
        .expect("C16 format");

    assert_eq!(format.alignment.text_rotation, 45);

    let stacked_reference = CellRef::parse("C17").expect("C17");
    let stacked = workbook.sheets[0]
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .find(|cell| cell.reference == stacked_reference)
        .expect("C17");
    let stacked_format = workbook
        .styles
        .cell_format(stacked.style_index)
        .expect("C17 format");
    assert_eq!(stacked_format.alignment.text_rotation, 255);
}

#[test]
fn grid_lines_can_be_turned_off_per_sheet() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    assert!(workbook.sheets[0].show_grid_lines);
    assert!(!workbook.sheets[1].show_grid_lines);
}

#[test]
fn reads_frozen_rows_columns_and_the_first_scrollable_cell() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");
    let pane = sheet.freeze_pane.as_ref().expect("frozen pane");

    assert_eq!(pane.frozen_columns, 1);
    assert_eq!(pane.frozen_rows, 2);
    assert_eq!(pane.top_left_cell, Some(CellRef::new(1, 2)));
    assert!(workbook.sheets[1].freeze_pane.is_none());
}

#[test]
fn reads_external_and_internal_hyperlinks() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = &workbook.sheets[0];

    let external = sheet
        .hyperlink_at(CellRef::parse("A2").expect("reference"))
        .expect("external hyperlink");
    assert_eq!(
        external.target.as_deref(),
        Some("https://example.com/a?x=1&y=2")
    );
    assert_eq!(external.location, None);
    assert_eq!(external.tooltip.as_deref(), Some("Open example"));

    let internal = sheet
        .hyperlink_at(CellRef::parse("C2").expect("reference"))
        .expect("range hyperlink");
    assert_eq!(internal.target, None);
    assert_eq!(internal.location.as_deref(), Some("'Sheet2'!A1"));
    assert_eq!(internal.display.as_deref(), Some("Jump"));
}

#[test]
fn reads_plain_and_rich_text_cell_comments_with_their_authors() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = &workbook.sheets[0];

    let plain = sheet
        .comment_at(CellRef::parse("A2").expect("reference"))
        .expect("plain comment");
    assert_eq!(plain.author.as_deref(), Some("Alice & Bob"));
    assert_eq!(plain.text, "External link note");

    let rich = sheet
        .comment_at(CellRef::parse("B2").expect("reference"))
        .expect("rich comment");
    assert_eq!(rich.author.as_deref(), Some("経理"));
    assert_eq!(rich.text, "売上 確認");
}

#[test]
fn reads_data_validations_with_formulas_messages_and_multiple_ranges() {
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = &workbook.sheets[0];

    let number = sheet
        .data_validation_at(CellRef::parse("B7").expect("reference"))
        .expect("number validation");
    assert_eq!(number.validation_type.as_deref(), Some("whole"));
    assert_eq!(number.operator.as_deref(), Some("between"));
    assert!(number.allow_blank);
    assert_eq!(number.formula1.as_deref(), Some("0"));
    assert_eq!(number.formula2.as_deref(), Some("10000"));
    assert_eq!(number.prompt.as_deref(), Some("Enter 0–10000"));
    assert_eq!(number.error.as_deref(), Some("Out of range"));

    let list = sheet
        .data_validation_at(CellRef::parse("A5").expect("reference"))
        .expect("list validation");
    assert_eq!(list.validation_type.as_deref(), Some("list"));
    assert_eq!(list.formula1.as_deref(), Some("\"横浜,東京\""));
    assert_eq!(list.ranges.len(), 2);
    assert!(sheet
        .data_validation_at(CellRef::parse("A3").expect("reference"))
        .is_none());

    let empty = sheet
        .data_validation_at(CellRef::parse("C2").expect("reference"))
        .expect("self-closing validation");
    assert_eq!(empty.validation_type.as_deref(), Some("none"));
    assert_eq!(empty.formula1, None);
}

#[test]
fn cell_references_parse_past_the_first_26_columns() {
    assert_eq!(CellRef::parse("A1").unwrap(), CellRef::new(0, 0));
    assert_eq!(CellRef::parse("Z1").unwrap(), CellRef::new(25, 0));
    assert_eq!(CellRef::parse("AA1").unwrap(), CellRef::new(26, 0));
    assert_eq!(CellRef::parse("$B$12").unwrap(), CellRef::new(1, 11));
    assert!(CellRef::parse("1A").is_err());
    assert!(CellRef::parse("A0").is_err());
    assert!(CellRef::parse("").is_err());

    // XLSX has fixed worksheet bounds. Accepting addresses beyond them lets a
    // headless write produce a package Excel must repair or reject.
    assert_eq!(
        CellRef::parse("XFD1048576").unwrap(),
        CellRef::new(16_383, 1_048_575)
    );
    assert!(CellRef::parse("XFE1").is_err());
    assert!(CellRef::parse("A1048577").is_err());
}

#[test]
fn the_date_epoch_defaults_to_1900_and_can_be_1904() {
    use jade_xlsx_reader::workbook_part::parse_workbook;

    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    assert_eq!(
        workbook.date_system,
        jade_xlsx_reader::DateSystem::Nineteen00
    );

    // Excel for Mac shipped with the other epoch and the setting survives in
    // files today; reading it wrong is a four-year error, not a rounding one.
    let mac = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><workbookPr date1904="1"/><sheets><sheet name="Sheet1" sheetId="1"/></sheets></workbook>"#;
    assert_eq!(
        parse_workbook(mac).expect("parse").date_system,
        jade_xlsx_reader::DateSystem::Nineteen04
    );

    let windows = mac.replace(r#"date1904="1""#, r#"date1904="0""#);
    assert_eq!(
        parse_workbook(&windows).expect("parse").date_system,
        jade_xlsx_reader::DateSystem::Nineteen00
    );
}

#[test]
fn entity_references_survive_the_parse() {
    // `quick-xml` reports `&amp;` as its own event rather than as text, so a
    // parser that only handles `Event::Text` drops every `&`, `<` and `>` a
    // cell contains — silently, and only for the cells that have them.
    let workbook = read_xlsx(&common::sample_workbook()).expect("read");
    let sheet = workbook.sheet_by_name("売上").expect("sheet");

    assert_eq!(
        cell_value(sheet, "A4"),
        CellValue::Text("a & b < c > d".into())
    );
    assert_eq!(cell_value(sheet, "B4"), CellValue::Text("x & y".into()));
}

#[test]
fn cdata_text_survives_shared_and_inline_strings() {
    // XML producers are allowed to use CDATA instead of escaped character
    // references. It is still cell text and must not disappear in headless
    // reads or rendering.
    let shared = jade_xlsx_reader::shared_strings::parse_shared_strings(
        r#"<sst><si><t><![CDATA[a & b < c]]></t></si></sst>"#,
    )
    .expect("shared strings");
    assert_eq!(shared, ["a & b < c"]);

    let xml = r#"<worksheet><sheetData><row r="1">
      <c r="A1" t="inlineStr"><is><t><![CDATA[x & y < z]]></t></is></c>
    </row></sheetData></worksheet>"#;
    let sheet =
        jade_xlsx_reader::sheet::parse_sheet("Sheet1", "sheet1.xml", xml, &[]).expect("parse");
    assert_eq!(
        cell_value(&sheet, "A1"),
        CellValue::Text("x & y < z".into())
    );
}

#[test]
fn spreadsheetml_xstring_escapes_become_the_characters_they_represent() {
    // Excel encodes control characters as `_xHHHH_`. `_x005F_` escapes the
    // leading underscore when that sequence is meant literally.
    let shared = jade_xlsx_reader::shared_strings::parse_shared_strings(
        r#"<sst><si><t>line1_x000A_line2</t></si><si><t>_x005F_x000A_</t></si></sst>"#,
    )
    .expect("shared strings");
    assert_eq!(shared, ["line1\nline2", "_x000A_"]);

    let xml = r#"<worksheet><sheetData><row r="1">
      <c r="A1" t="inlineStr"><is><t>tab_x0009_separated</t></is></c>
    </row></sheetData></worksheet>"#;
    let sheet =
        jade_xlsx_reader::sheet::parse_sheet("Sheet1", "sheet1.xml", xml, &[]).expect("parse");
    assert_eq!(
        cell_value(&sheet, "A1"),
        CellValue::Text("tab\tseparated".into())
    );
}

#[test]
fn spreadsheetml_utf16_surrogates_become_non_bmp_characters() {
    // Xstring uses UTF-16 code units, so characters above U+FFFF are written
    // as two adjacent `_xHHHH_` escapes.
    let shared = jade_xlsx_reader::shared_strings::parse_shared_strings(
        r#"<sst><si><t>status _xD83D__xDE00_</t></si></sst>"#,
    )
    .expect("shared strings");
    assert_eq!(shared, ["status 😀"]);
}

#[test]
fn inline_string_phonetic_guides_do_not_become_cell_text() {
    // Excel may store rich inline text together with rPh pronunciation hints.
    // The hints are metadata, just as they are in sharedStrings.xml, and must
    // not appear in the value a formula, search, or headless reader sees.
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
      <sheetData><row r="1"><c r="A1" t="inlineStr"><is>
        <r><t>東</t></r><r><t>京都</t></r>
        <rPh sb="0" eb="3"><t>とうきょうと</t></rPh>
      </is></c></row></sheetData>
    </worksheet>"#;
    let sheet =
        jade_xlsx_reader::sheet::parse_sheet("Sheet1", "sheet1.xml", xml, &[]).expect("parse");

    assert_eq!(cell_value(&sheet, "A1"), CellValue::Text("東京都".into()));
}

#[test]
fn boolean_cells_accept_xml_schema_lexical_values() {
    // SpreadsheetML's boolean value is an XML Schema boolean, whose lexical
    // space includes true/false as well as 1/0. Some non-Excel producers use
    // the word forms even though Excel itself usually writes digits.
    let xml = r#"<worksheet><sheetData><row r="1">
      <c r="A1" t="b"><v>true</v></c>
      <c r="B1" t="b"><v>false</v></c>
    </row></sheetData></worksheet>"#;
    let sheet =
        jade_xlsx_reader::sheet::parse_sheet("Sheet1", "sheet1.xml", xml, &[]).expect("parse");

    assert_eq!(cell_value(&sheet, "A1"), CellValue::Bool(true));
    assert_eq!(cell_value(&sheet, "B1"), CellValue::Bool(false));
}

#[test]
fn a_row_without_an_index_follows_the_previous_row() {
    // `row/@r` and `c/@r` are optional in SpreadsheetML. The implicit row is
    // relative to the preceding row, not to the number of row elements seen.
    let xml = r#"<worksheet><sheetData>
      <row r="5"><c r="A5" t="inlineStr"><is><t>five</t></is></c></row>
      <row><c t="inlineStr"><is><t>six</t></is></c></row>
    </sheetData></worksheet>"#;
    let sheet =
        jade_xlsx_reader::sheet::parse_sheet("Sheet1", "sheet1.xml", xml, &[]).expect("parse");

    assert_eq!(cell_value(&sheet, "A5"), CellValue::Text("five".into()));
    assert_eq!(cell_value(&sheet, "A6"), CellValue::Text("six".into()));
    assert_eq!(
        sheet.rows.iter().map(|row| row.index).collect::<Vec<_>>(),
        [4, 5]
    );
}

#[test]
fn comment_text_decodes_excels_x_escapes() {
    // Comments are ST_Xstring like shared strings, and Excel stores a CR in a
    // note as `_x000D_`.
    let xml = r#"<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
      <authors><author>A</author></authors>
      <commentList>
        <comment ref="A1" authorId="0"><text><t>line1_x000D_line2</t></text></comment>
      </commentList>
    </comments>"#;
    let comments = jade_xlsx_reader::comments::parse_comments(xml).expect("parse");
    assert_eq!(comments[0].text, "line1\rline2");
}

#[test]
fn a_formula_translates_for_a_copy_the_way_a_spreadsheet_pastes_one() {
    // The same movement rules shared formulas already follow, exposed for the
    // clipboard: relative axes shift with the paste offset, absolute axes and
    // string literals stay, and a reference pushed off the grid becomes #REF!.
    use jade_xlsx_reader::{translate_formula, CellRef};

    let from = CellRef::parse("D4").expect("D4");
    assert_eq!(
        translate_formula("SUM(B4:C4)", from, CellRef::parse("D5").expect("D5")),
        "SUM(B5:C5)"
    );
    assert_eq!(
        translate_formula("$B$4+C4", from, CellRef::parse("F5").expect("F5")),
        "$B$4+E5"
    );
    assert_eq!(
        translate_formula(r#""B4"&Notes!B4"#, from, CellRef::parse("E4").expect("E4")),
        r#""B4"&Notes!C4"#
    );
    assert_eq!(
        translate_formula(
            "A2+B2",
            CellRef::parse("B2").expect("B2"),
            CellRef::parse("A1").expect("A1")
        ),
        "#REF!+A1"
    );
}
