use jade_xlsx_writer::freeze_panes::{set_freeze_pane, Frozen};
use quick_xml::{events::Event, name::ResolveResult, NsReader};

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

fn assert_sheet_namespace(xml: &str, expected: (usize, usize)) {
    let mut counts = (0, 0);
    let mut reader = NsReader::from_str(xml);
    loop {
        match reader
            .read_resolved_event()
            .expect("well-formed worksheet XML")
        {
            (namespace, Event::Start(element) | Event::Empty(element)) => {
                let local = element.local_name();
                if matches!(local.as_ref(), b"pane" | b"selection") {
                    if local.as_ref() == b"pane" {
                        counts.0 += 1;
                    } else {
                        counts.1 += 1;
                    }
                    assert!(
                        matches!(namespace, ResolveResult::Bound(value) if value.as_ref() == NS.as_bytes()),
                        "wrong namespace for {:?} in {xml}",
                        local
                    );
                }
            }
            (_, Event::Eof) => break,
            _ => {}
        }
    }
    assert_eq!(counts, expected);
}

#[test]
fn differently_prefixed_empty_view_stays_well_formed() {
    let xml = format!("<s:worksheet xmlns:s=\"{NS}\"><s:sheetViews><v:sheetView xmlns:v=\"{NS}\" workbookViewId=\"0\"/></s:sheetViews><s:sheetData/></s:worksheet>");
    assert_sheet_namespace(&xml, (0, 0));
    let changed = set_freeze_pane(
        &xml,
        Some(Frozen {
            columns: 1,
            rows: 1,
            top_left: None,
        }),
    )
    .unwrap();
    assert_sheet_namespace(&changed, (1, 3));
    let unfrozen = set_freeze_pane(&changed, None).unwrap();
    assert_sheet_namespace(&unfrozen, (0, 1));
}

#[test]
fn local_default_namespace_does_not_capture_inserted_children() {
    let xml = format!("<worksheet xmlns=\"{NS}\"><sheetViews><v:sheetView xmlns:v=\"{NS}\" xmlns=\"urn:extension\" workbookViewId=\"0\"></v:sheetView></sheetViews><sheetData/></worksheet>");
    assert_sheet_namespace(&xml, (0, 0));
    let changed = set_freeze_pane(
        &xml,
        Some(Frozen {
            columns: 0,
            rows: 1,
            top_left: None,
        }),
    )
    .unwrap();
    assert_sheet_namespace(&changed, (1, 1));
    let unfrozen = set_freeze_pane(&changed, None).unwrap();
    assert_sheet_namespace(&unfrozen, (0, 1));
}
