//! View ▸ Freeze Panes as Excel writes it: a frozen `<pane>` with the
//! scrolling part's first cell and the active part, a selection for each
//! part that scrolls, and the rest of the view left alone.

use jade_xlsx_reader::sheet::parse_sheet;
use jade_xlsx_reader::{CellRef, FreezePane};
use jade_xlsx_writer::freeze_panes::{set_freeze_pane, Frozen};

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

fn sheet(views: &str) -> String {
    format!(
        "<worksheet xmlns=\"{NS}\"><dimension ref=\"A1\"/>{views}<sheetFormatPr defaultRowHeight=\"15\"/><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row></sheetData></worksheet>"
    )
}

fn frozen(xml: &str) -> Option<FreezePane> {
    parse_sheet("Sheet1", "xl/worksheets/sheet1.xml", xml, &[])
        .expect("sheet")
        .freeze_pane
}

fn freeze(columns: u32, rows: u32) -> Option<Frozen> {
    Some(Frozen {
        columns,
        rows,
        top_left: None,
    })
}

#[test]
fn rows_columns_and_both_freeze_as_excel_writes_them() {
    let original = sheet(
        "<sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\"><selection activeCell=\"C5\" sqref=\"C5\"/></sheetView></sheetViews>",
    );

    let top_row = set_freeze_pane(&original, freeze(0, 1)).unwrap();
    assert!(top_row.contains(
        "<sheetView tabSelected=\"1\" workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/><selection pane=\"bottomLeft\" activeCell=\"C5\" sqref=\"C5\"/></sheetView>"
    ), "{top_row}");
    assert_eq!(
        frozen(&top_row),
        Some(FreezePane {
            frozen_columns: 0,
            frozen_rows: 1,
            top_left_cell: Some(CellRef::new(0, 1)),
        })
    );

    let first_column = set_freeze_pane(&original, freeze(1, 0)).unwrap();
    assert!(first_column.contains(
        "<pane xSplit=\"1\" topLeftCell=\"B1\" activePane=\"topRight\" state=\"frozen\"/><selection pane=\"topRight\" activeCell=\"C5\" sqref=\"C5\"/>"
    ), "{first_column}");

    let both = set_freeze_pane(&original, freeze(2, 3)).unwrap();
    assert!(both.contains(
        "<pane xSplit=\"2\" ySplit=\"3\" topLeftCell=\"C4\" activePane=\"bottomRight\" state=\"frozen\"/><selection pane=\"topRight\" activeCell=\"C1\" sqref=\"C1\"/><selection pane=\"bottomLeft\" activeCell=\"A4\" sqref=\"A4\"/><selection pane=\"bottomRight\" activeCell=\"C5\" sqref=\"C5\"/>"
    ), "{both}");
    assert_eq!(
        frozen(&both).map(|pane| (pane.frozen_columns, pane.frozen_rows)),
        Some((2, 3))
    );

    // Freezing again replaces the pane and its selections, keeping the
    // active cell; unfreezing leaves a plain selection.
    let again = set_freeze_pane(&both, freeze(0, 1)).unwrap();
    assert_eq!(again.matches("<pane").count(), 1);
    assert_eq!(again.matches("<selection").count(), 1);
    assert!(again.contains("activeCell=\"C5\""));
    let unfrozen = set_freeze_pane(&both, None).unwrap();
    assert_eq!(unfrozen, original);
    assert_eq!(frozen(&unfrozen), None);
}

#[test]
fn a_sheet_without_views_or_with_an_empty_one_gets_what_it_needs() {
    let bare = sheet("");
    let frozen_bare = set_freeze_pane(&bare, freeze(0, 1)).unwrap();
    assert!(
        frozen_bare.contains(
            "<dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\""
        ),
        "{frozen_bare}"
    );
    assert_eq!(frozen(&frozen_bare).map(|pane| pane.frozen_rows), Some(1));
    // Nothing to unfreeze: the part is untouched.
    assert_eq!(set_freeze_pane(&bare, None).unwrap(), bare);

    let empty = sheet("<sheetViews><sheetView workbookViewId=\"0\"/></sheetViews>");
    let frozen_empty = set_freeze_pane(&empty, freeze(1, 1)).unwrap();
    assert!(
        frozen_empty.contains(
            "<sheetView workbookViewId=\"0\"><pane xSplit=\"1\" ySplit=\"1\" topLeftCell=\"B2\""
        ),
        "{frozen_empty}"
    );
    assert!(frozen_empty.contains("</sheetView></sheetViews>"));
}

#[test]
fn a_prefixed_sheet_keeps_its_prefix_and_impossible_panes_are_refused() {
    let prefixed = format!(
        "<x:worksheet xmlns:x=\"{NS}\"><x:sheetViews><x:sheetView workbookViewId=\"0\"><x:selection activeCell=\"B2\" sqref=\"B2\"/></x:sheetView></x:sheetViews><x:sheetData/></x:worksheet>"
    );
    let frozen_prefixed = set_freeze_pane(&prefixed, freeze(0, 2)).unwrap();
    assert!(
        frozen_prefixed.contains("<x:pane ySplit=\"2\""),
        "{frozen_prefixed}"
    );
    assert!(frozen_prefixed.contains("<x:selection pane=\"bottomLeft\" activeCell=\"B2\""));
    assert!(set_freeze_pane(&prefixed, freeze(16_384, 0)).is_err());
    assert!(set_freeze_pane(&prefixed, freeze(0, 1_048_576)).is_err());
}
