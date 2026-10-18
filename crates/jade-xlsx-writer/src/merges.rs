//! A sheet's merged ranges.
//!
//! `<mergeCells>` has a fixed place among the worksheet's children — after
//! `customSheetViews`, before `phoneticPr` — and Excel refuses a sheet whose
//! children are out of order, so a list the sheet did not have is inserted
//! before the first child the schema puts after it.

use jade_xlsx_reader::CellRange;

use crate::header_footer::set_child;
use crate::patch::escape;
use crate::shift::range_text;
use crate::Result;

/// Worksheet children that come after `mergeCells`, in schema order.
const FOLLOWING: &[&[u8]] = &[
    b"phoneticPr",
    b"conditionalFormatting",
    b"dataValidations",
    b"hyperlinks",
    b"printOptions",
    b"pageMargins",
    b"pageSetup",
    b"headerFooter",
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"drawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];

/// The worksheet part with its merged ranges replaced by `merges`, in the
/// order given — the `<mergeCells>` list removed when `merges` is empty.
pub fn set_merges(xml: &str, merges: &[CellRange]) -> Result<String> {
    set_child(
        xml,
        b"mergeCells",
        |prefix| {
            if merges.is_empty() {
                return String::new();
            }
            let mut out = format!("<{prefix}mergeCells count=\"{}\">", merges.len());
            for range in merges {
                out.push_str(&format!(
                    "<{prefix}mergeCell ref=\"{}\"/>",
                    escape(&range_text(*range))
                ));
            }
            out.push_str(&format!("</{prefix}mergeCells>"));
            out
        },
        FOLLOWING,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(text: &str) -> CellRange {
        CellRange::parse(text).unwrap()
    }

    #[test]
    fn replaces_the_list_in_place_and_removes_it_when_empty() {
        let xml = r#"<x:worksheet xmlns:x="m"><x:sheetData/><x:mergeCells count="1"><x:mergeCell ref="A1:B1"/></x:mergeCells><x:pageMargins/></x:worksheet>"#;
        assert_eq!(
            set_merges(xml, &[range("A1:B1"), range("C3:D4")]).unwrap(),
            r#"<x:worksheet xmlns:x="m"><x:sheetData/><x:mergeCells count="2"><x:mergeCell ref="A1:B1"/><x:mergeCell ref="C3:D4"/></x:mergeCells><x:pageMargins/></x:worksheet>"#
        );
        assert_eq!(
            set_merges(xml, &[]).unwrap(),
            r#"<x:worksheet xmlns:x="m"><x:sheetData/><x:pageMargins/></x:worksheet>"#
        );
    }

    #[test]
    fn inserts_a_new_list_before_what_follows_it() {
        let xml = r#"<worksheet><sheetData/><autoFilter ref="A1:B2"/><hyperlinks/><pageMargins/></worksheet>"#;
        assert_eq!(
            set_merges(xml, &[range("C3:D4")]).unwrap(),
            r#"<worksheet><sheetData/><autoFilter ref="A1:B2"/><mergeCells count="1"><mergeCell ref="C3:D4"/></mergeCells><hyperlinks/><pageMargins/></worksheet>"#
        );
        let bare = r#"<worksheet><sheetData/></worksheet>"#;
        assert_eq!(
            set_merges(bare, &[range("C3:D4")]).unwrap(),
            r#"<worksheet><sheetData/><mergeCells count="1"><mergeCell ref="C3:D4"/></mergeCells></worksheet>"#
        );
    }
}
