//! Manual page breaks written as Excel writes them: `rowBreaks` after the
//! header and footer and before `colBreaks`, each `brk` the zero-based row
//! the break sits above, the ones pagination placed kept.

use std::collections::BTreeSet;

use jade_xlsx_reader::Axis;
use jade_xlsx_writer::page_breaks::set_manual_breaks;

#[test]
fn row_breaks_go_in_order_keep_automatic_ones_and_go_away() {
    let xml = r#"<worksheet><sheetData/><pageMargins left="0.7"/><colBreaks count="1" manualBreakCount="1"><brk id="3" max="1048575" man="1"/></colBreaks></worksheet>"#;
    let breaks: BTreeSet<u32> = [5, 9].into();
    let written = set_manual_breaks(xml, Axis::Rows, &breaks).unwrap();
    assert!(written.contains(
        r#"<rowBreaks count="2" manualBreakCount="2"><brk id="5" max="16383" man="1"/><brk id="9" max="16383" man="1"/></rowBreaks><colBreaks"#
    ), "{written}");

    // One placed by pagination stays; a manual one at its row wins.
    let mixed = r#"<worksheet><sheetData/><rowBreaks count="2" manualBreakCount="1"><brk id="4" max="16383"/><brk id="5" max="16383" man="1"/></rowBreaks></worksheet>"#;
    let written = set_manual_breaks(mixed, Axis::Rows, &[7].into()).unwrap();
    assert!(written.contains(
        r#"<rowBreaks count="2" manualBreakCount="1"><brk id="4" max="16383"/><brk id="7" max="16383" man="1"/></rowBreaks>"#
    ), "{written}");

    let cleared = set_manual_breaks(&written, Axis::Rows, &BTreeSet::new()).unwrap();
    assert!(
        cleared.contains(
            r#"<rowBreaks count="1" manualBreakCount="0"><brk id="4" max="16383"/></rowBreaks>"#
        ),
        "{cleared}"
    );
    let none = set_manual_breaks(
        r#"<worksheet><sheetData/><rowBreaks count="1" manualBreakCount="1"><brk id="5" max="16383" man="1"/></rowBreaks></worksheet>"#,
        Axis::Rows,
        &BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(none, "<worksheet><sheetData/></worksheet>");

    // Read back as they were written.
    let written = set_manual_breaks(xml, Axis::Rows, &breaks).unwrap();
    assert!(written.find("<rowBreaks").unwrap() < written.find("<colBreaks").unwrap());
}
