use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, Axis, StructureChange};
use jade_xlsx_writer::{restructure, shift_table};

const SHEET: &str = "xl/worksheets/sheet1.xml";
const TABLE: &str = "xl/tables/table1.xml";
const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

fn fixture() -> Vec<u8> {
    let source = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let mut package = OpcPackage::open(&source).unwrap();
    let sheet = package.part_str(SHEET).unwrap().unwrap();
    let filter = r#"<autoFilter ref="E1:G4"><filterColumn colId="1" hiddenButton="1"><customFilters and="1"><customFilter operator="greaterThan" val="10"/><customFilter operator="lessThan" val="40"/></customFilters></filterColumn><sortState ref="E2:G4"><sortCondition descending="1" ref="F2:F4"/></sortState></autoFilter><sortState ref="I2:K4"><sortCondition ref="J2:J4"/></sortState>"#;
    package.replace(
        SHEET,
        sheet
            .replace("<mergeCells", &format!("{filter}<mergeCells"))
            .into_bytes(),
    );
    let table = package.part_str(TABLE).unwrap().unwrap();
    let filter = r#"<autoFilter ref="A1:C4"><filterColumn colId="0"><filters blank="1"><filter val="Alpha"/></filters></filterColumn><filterColumn colId="1"><top10 top="1" percent="0" val="10"/></filterColumn><filterColumn colId="2"><customFilters><customFilter operator="greaterThan" val="20"/></customFilters></filterColumn><sortState ref="A2:C4"><sortCondition descending="1" ref="B2:B4"/><sortCondition ref="C2:C4"/></sortState></autoFilter>"#;
    package.replace(
        TABLE,
        table
            .replace(r#"<autoFilter ref="A1:C4"/>"#, filter)
            .into_bytes(),
    );
    package.write().unwrap()
}

fn apply(source: &[u8], axis: Axis, at: u32, count: u32, inserted: bool) -> Vec<u8> {
    let book = read_xlsx(source).unwrap();
    restructure(
        source,
        &book,
        &StructureChange {
            sheet: "Operations".into(),
            axis,
            at,
            count,
            inserted,
        },
    )
    .unwrap()
}

fn part(bytes: &[u8], name: &str) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .part_str(name)
        .unwrap()
        .unwrap()
}

fn export(name: &str, bytes: &[u8]) {
    if let Ok(directory) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&directory).join(format!("filters-{name}.xlsx")),
            bytes,
        )
        .unwrap();
    }
}

#[test]
fn inserted_columns_keep_criteria_on_the_original_cells_in_both_filter_owners() {
    let source = fixture();
    let written = apply(&source, Axis::Columns, 1, 1, true);
    let table = part(&written, TABLE);
    assert!(table.contains(r#"autoFilter ref="A1:D4""#), "{table}");
    assert!(
        table.contains(r#"filterColumn colId="2"><top10"#),
        "{table}"
    );
    assert!(
        table.contains(r#"filterColumn colId="3"><customFilters"#),
        "{table}"
    );
    assert!(table.contains(r#"sortState ref="A2:D4""#), "{table}");
    assert!(table.contains(r#"descending="1" ref="C2:C4""#), "{table}");
    assert!(table.contains(r#"sortCondition ref="D2:D4""#), "{table}");
    let sheet = part(&written, SHEET);
    assert!(sheet.contains(r#"autoFilter ref="F1:H4""#), "{sheet}");
    assert!(sheet.contains(r#"colId="1" hiddenButton="1""#), "{sheet}");
    assert!(sheet.contains(r#"sortState ref="F2:H4""#), "{sheet}");
    assert!(sheet.contains(r#"descending="1" ref="G2:G4""#), "{sheet}");
    assert!(sheet.contains(r#"sortState ref="J2:L4""#), "{sheet}");
    assert!(sheet.contains(r#"sortCondition ref="K2:K4""#), "{sheet}");
    export("inserted", &written);
}

#[test]
fn deleting_filtered_columns_removes_their_criteria_and_sort_keys() {
    let written = apply(&fixture(), Axis::Columns, 1, 1, false);
    let table = part(&written, TABLE);
    assert!(!table.contains("<top10"), "{table}");
    assert!(
        table.contains(r#"filterColumn colId="1"><customFilters"#),
        "{table}"
    );
    assert!(!table.contains(r#"descending="1""#), "{table}");
    assert!(table.contains(r#"sortCondition ref="B2:B4""#), "{table}");
    let sheet = part(&written, SHEET);
    assert!(sheet.contains(r#"autoFilter ref="D1:F4""#), "{sheet}");
    assert!(sheet.contains(r#"descending="1" ref="E2:E4""#), "{sheet}");
    export("deleted", &written);
}

#[test]
fn deleting_the_leading_columns_rebases_surviving_relative_ids() {
    let written = apply(&fixture(), Axis::Columns, 0, 2, false);
    let table = part(&written, TABLE);
    assert!(table.contains(r#"autoFilter ref="A1:A4""#), "{table}");
    assert!(
        table.contains(r#"filterColumn colId="0"><customFilters"#),
        "{table}"
    );
    assert!(
        !table.contains("<filters ") && !table.contains("<top10"),
        "{table}"
    );
    assert!(table.contains(r#"sortCondition ref="A2:A4""#), "{table}");
    export("leading", &written);
}

#[test]
fn removing_all_sort_keys_and_filter_ranges_drops_their_containers() {
    let source = fixture();
    let written = apply(&source, Axis::Columns, 5, 1, false);
    let sheet = part(&written, SHEET);
    assert!(sheet.contains(r#"autoFilter ref="E1:F4""#), "{sheet}");
    assert!(
        !sheet.contains("<filterColumn") && !sheet.contains(r#"descending="1""#),
        "{sheet}"
    );
    assert!(!sheet.contains(r#"sortState ref="E2:F4""#), "{sheet}");
    assert!(sheet.contains(r#"sortState ref="H2:J4""#), "{sheet}");
    let written = apply(&source, Axis::Rows, 0, 4, false);
    assert!(!part(&written, SHEET).contains("<autoFilter"));
    assert!(!part(&written, SHEET).contains("<sortState"));
    export("removed", &written);
}

#[test]
fn row_changes_move_sort_ranges_without_changing_filter_criteria() {
    let source = fixture();
    let written = apply(&source, Axis::Rows, 2, 2, true);
    let sheet = part(&written, SHEET);
    assert!(sheet.contains(r#"autoFilter ref="E1:G6""#), "{sheet}");
    assert!(sheet.contains(r#"sortState ref="E2:G6""#), "{sheet}");
    assert!(sheet.contains(r#"descending="1" ref="F2:F6""#), "{sheet}");
    assert!(sheet.contains(r#"colId="1" hiddenButton="1""#), "{sheet}");
    let table = part(&written, TABLE);
    assert!(table.contains(r#"autoFilter ref="A1:C6""#), "{table}");
    assert!(
        table.contains(r#"filterColumn colId="1"><top10"#),
        "{table}"
    );
    assert!(table.contains(r#"descending="1" ref="B2:B6""#), "{table}");
    export("rows", &written);
}

#[test]
fn prefixed_filters_preserve_unknown_nested_criteria_bytes() {
    let xml = format!(
        r#"<x:table xmlns:x="{NS}" xmlns:q="urn:future" ref="B3:D7"><x:autoFilter q:ref="Z1:Z9" ref="B3:D7"><x:filterColumn q:colId="99" colId="1"><x:filters><x:filter val="&amp;"/></x:filters><x:extLst><x:ext uri="future"><q:filterColumn colId="99"><q:sortState ref="Z1:Z9"/></q:filterColumn></x:ext></x:extLst></x:filterColumn></x:autoFilter><x:tableColumns count="3"><x:tableColumn id="1" name="A"/><x:tableColumn id="2" name="B"/><x:tableColumn id="3" name="C"/></x:tableColumns></x:table>"#
    );
    let shifted = shift_table(
        &xml,
        "Operations",
        &StructureChange {
            sheet: "Operations".into(),
            axis: Axis::Columns,
            at: 2,
            count: 1,
            inserted: true,
        },
    )
    .unwrap()
    .xml
    .unwrap();
    assert!(
        shifted.contains(r#"<x:filterColumn q:colId="99" colId="2">"#),
        "{shifted}"
    );
    assert!(
        shifted
            .contains(r#"<q:filterColumn colId="99"><q:sortState ref="Z1:Z9"/></q:filterColumn>"#),
        "{shifted}"
    );
    assert!(shifted.contains(r#"<x:filter val="&amp;"/>"#), "{shifted}");
    assert!(
        shifted.contains(r#"q:ref="Z1:Z9" ref="B3:E7""#),
        "{shifted}"
    );
}
