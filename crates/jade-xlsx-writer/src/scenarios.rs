//! The Scenario Manager's scenarios saved as Excel saves them: the
//! sheet's `<scenarios>`, after `protectedRanges` and before `autoFilter`.

use jade_xlsx_reader::scenarios::Scenarios;

use crate::header_footer::set_child;
use crate::patch::escape;
use crate::shift::{range_text, reference_text};
use crate::Result;

/// Worksheet children that come after `scenarios`, in schema order.
const FOLLOWING: &[&[u8]] = &[
    b"autoFilter",
    b"sortState",
    b"dataConsolidate",
    b"customSheetViews",
    b"mergeCells",
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

/// The sheet part with its scenarios replaced by `scenarios`; none, or a
/// list without one, takes the element out.
pub fn set_scenarios(xml: &str, scenarios: Option<&Scenarios>) -> Result<String> {
    set_child(
        xml,
        b"scenarios",
        |prefix| match scenarios.filter(|list| !list.scenarios.is_empty()) {
            Some(list) => scenarios_xml(list, prefix),
            None => String::new(),
        },
        FOLLOWING,
    )
}

fn scenarios_xml(list: &Scenarios, prefix: &str) -> String {
    let mut out = format!("<{prefix}scenarios");
    if let Some(current) = list.current {
        out.push_str(&format!(" current=\"{current}\""));
    }
    if let Some(show) = list.show {
        out.push_str(&format!(" show=\"{show}\""));
    }
    if !list.result_cells.is_empty() {
        let sqref: Vec<String> = list.result_cells.iter().map(|r| range_text(*r)).collect();
        out.push_str(&format!(" sqref=\"{}\"", sqref.join(" ")));
    }
    out.push('>');
    for scenario in &list.scenarios {
        out.push_str(&format!(
            "<{prefix}scenario name=\"{}\"",
            escape(&scenario.name)
        ));
        if scenario.locked {
            out.push_str(" locked=\"1\"");
        }
        if scenario.hidden {
            out.push_str(" hidden=\"1\"");
        }
        out.push_str(&format!(" count=\"{}\"", scenario.cells.len()));
        if let Some(user) = &scenario.user {
            out.push_str(&format!(" user=\"{}\"", escape(user)));
        }
        if let Some(comment) = &scenario.comment {
            out.push_str(&format!(" comment=\"{}\"", escape(comment)));
        }
        out.push('>');
        for (at, value) in &scenario.cells {
            out.push_str(&format!(
                "<{prefix}inputCells r=\"{}\" val=\"{}\"/>",
                reference_text(*at),
                escape(value)
            ));
        }
        out.push_str(&format!("</{prefix}scenario>"));
    }
    out.push_str(&format!("</{prefix}scenarios>"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_xlsx_reader::scenarios::Scenario;
    use jade_xlsx_reader::CellRef;

    #[test]
    fn goes_before_the_auto_filter_and_reads_back() {
        let list = Scenarios {
            current: Some(0),
            show: Some(0),
            result_cells: vec![jade_xlsx_reader::CellRange::parse("B5").unwrap()],
            scenarios: vec![Scenario {
                name: "Best & <worst>".into(),
                locked: true,
                hidden: false,
                user: Some("Ann".into()),
                comment: Some("Made by Ann".into()),
                cells: vec![(CellRef::new(1, 0), "10".into())],
            }],
        };
        let xml = r#"<worksheet xmlns="m"><sheetData/><autoFilter ref="A1:B2"/><pageMargins left="1"/></worksheet>"#;
        let written = set_scenarios(xml, Some(&list)).unwrap();
        assert!(written.contains(r#"<sheetData/><scenarios current="0" show="0" sqref="B5"><scenario name="Best &amp; &lt;worst&gt;" locked="1" count="1" user="Ann" comment="Made by Ann"><inputCells r="B1" val="10"/></scenario></scenarios><autoFilter"#), "{written}");
        let sheet = jade_xlsx_reader::sheet::parse_sheet("S", "s.xml", &written, &[]).unwrap();
        assert_eq!(sheet.scenarios.as_ref(), Some(&list));
        // Again replaces it; none takes it out.
        let again = set_scenarios(&written, Some(&list)).unwrap();
        assert_eq!(again, written);
        assert_eq!(set_scenarios(&written, None).unwrap(), xml);
    }
}
