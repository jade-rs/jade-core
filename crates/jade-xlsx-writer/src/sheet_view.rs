//! A sheet's view settings on its first `<sheetView>`: Show Formulas,
//! showing zeros, gridlines, headings, the zoom and Page Break Preview.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::freeze_panes::missing_views;
use crate::patch::{apply, attribute, escape, local, Splice};
use crate::shift::rewrite;
use crate::Result;

/// The sheet part with `showFormulas` set on its first `<sheetView>`, or
/// taken away when `on` is false. A sheet with no `<sheetView>` is left as
/// it is.
pub fn set_show_formulas(xml: &str, on: bool) -> Result<String> {
    set_view_attribute(xml, "showFormulas", on.then(|| "1".to_string()))
}

/// The sheet part with zeros shown (`showZeros` taken away, Excel's
/// default) or hidden (`showZeros="0"`) on its first `<sheetView>`.
pub fn set_show_zeros(xml: &str, on: bool) -> Result<String> {
    set_view_attribute(xml, "showZeros", (!on).then(|| "0".to_string()))
}

/// The sheet part opening in Page Break Preview (`view="pageBreakPreview"`
/// on its first `<sheetView>`) or in the normal view (`view` taken away).
pub fn set_page_break_preview(xml: &str, on: bool) -> Result<String> {
    set_view_attribute(xml, "view", on.then(|| "pageBreakPreview".to_string()))
}

/// The sheet part with gridlines shown (`showGridLines` taken away, Excel's
/// default) or hidden (`showGridLines="0"`) on its first `<sheetView>`.
pub fn set_show_grid_lines(xml: &str, on: bool) -> Result<String> {
    set_view_attribute(xml, "showGridLines", (!on).then(|| "0".to_string()))
}

/// The sheet part with its row numbers and column letters shown
/// (`showRowColHeaders` taken away) or hidden (`showRowColHeaders="0"`).
pub fn set_show_headers(xml: &str, on: bool) -> Result<String> {
    set_view_attribute(xml, "showRowColHeaders", (!on).then(|| "0".to_string()))
}

/// The sheet part opening at `percent` zoom, as Excel writes it: the
/// normal view's `zoomScale` and `zoomScaleNormal`, or in Page Break
/// Preview `zoomScale` and `zoomScaleSheetLayoutView`. 100% takes them
/// away. Held to Excel's 10–400%.
pub fn set_zoom(xml: &str, percent: u32) -> Result<String> {
    let percent = percent.clamp(10, 400);
    let value = (percent != 100).then(|| percent.to_string());
    let xml = set_view_attribute(xml, "zoomScale", value.clone())?;
    let other = if first_view_attribute(&xml, "view")?.as_deref() == Some("pageBreakPreview") {
        "zoomScaleSheetLayoutView"
    } else {
        "zoomScaleNormal"
    };
    set_view_attribute(&xml, other, value)
}

/// The sheet's tab selected or not (`tabSelected`), as a grouped sheet's
/// is; a tab not selected loses the attribute, as Excel writes it.
pub fn set_tab_selected(xml: &str, on: bool) -> Result<String> {
    set_view_attribute(xml, "tabSelected", on.then(|| "1".to_string()))
}

/// `name`'s value on the first `<sheetView>`, if it has one.
fn first_view_attribute(xml: &str, name: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref e) | Event::Empty(ref e)
                if local(e.name().as_ref()) == b"sheetView" =>
            {
                return attribute(e, name.as_bytes());
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
        buf.clear();
    }
}

/// `name` set to `value`, or taken away, on the first `<sheetView>`. A
/// sheet with no view gets one to hold a value; taking one away from it
/// leaves it as it is.
fn set_view_attribute(xml: &str, name: &str, value: Option<String>) -> Result<String> {
    if let Some((at, prefix)) = missing_views(xml)? {
        let Some(value) = value else {
            return Ok(xml.to_string());
        };
        let text = format!(
            "<{prefix}sheetViews><{prefix}sheetView {name}=\"{}\" workbookViewId=\"0\"/></{prefix}sheetViews>",
            escape(&value)
        );
        return Ok(apply(
            xml,
            vec![Splice {
                start: at,
                end: at,
                text,
            }],
        ));
    }
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e)
                if local(e.name().as_ref()) == b"sheetView" =>
            {
                let text = rewrite(e, &[(name, value)], matches!(event, Event::Empty(_)))?;
                let splice = Splice {
                    start,
                    end: reader.buffer_position() as usize,
                    text,
                };
                return Ok(apply(xml, vec![splice]));
            }
            Event::Eof => return Ok(xml.to_string()),
            _ => {}
        }
        buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_formulas_is_set_and_taken_away_on_the_first_view() {
        let xml = r#"<worksheet><sheetViews><sheetView tabSelected="1" workbookViewId="0"><pane ySplit="1"/></sheetView></sheetViews><sheetData/></worksheet>"#;
        let on = set_show_formulas(xml, true).unwrap();
        assert!(on.contains(r#"showFormulas="1""#));
        assert!(on.contains("<pane"));
        let off = set_show_formulas(&on, false).unwrap();
        assert!(!off.contains("showFormulas"));
        let bare = "<worksheet><sheetData/></worksheet>";
        assert_eq!(set_show_formulas(bare, false).unwrap(), bare);
    }

    #[test]
    fn a_sheet_with_no_view_gets_one_to_hold_a_setting() {
        let bare = r#"<worksheet xmlns="m"><sheetPr/><dimension ref="A1"/><sheetFormatPr defaultRowHeight="15"/><sheetData/></worksheet>"#;
        let hidden = set_show_grid_lines(bare, false).unwrap();
        assert!(hidden.contains(
            r#"<dimension ref="A1"/><sheetViews><sheetView showGridLines="0" workbookViewId="0"/></sheetViews><sheetFormatPr"#
        ));
        assert_eq!(set_show_grid_lines(bare, true).unwrap(), bare);
    }

    #[test]
    fn gridlines_and_headings_are_hidden_with_0_and_shown_by_taking_it_away() {
        let xml = r#"<worksheet><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetData/></worksheet>"#;
        let hidden = set_show_headers(&set_show_grid_lines(xml, false).unwrap(), false).unwrap();
        assert!(hidden.contains(r#"showGridLines="0""#));
        assert!(hidden.contains(r#"showRowColHeaders="0""#));
        let shown = set_show_headers(&set_show_grid_lines(&hidden, true).unwrap(), true).unwrap();
        assert_eq!(shown, xml);
    }

    #[test]
    fn the_zoom_is_written_for_the_view_the_sheet_opens_in() {
        let xml = r#"<worksheet><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetData/></worksheet>"#;
        let zoomed = set_zoom(xml, 85).unwrap();
        assert!(zoomed.contains(r#"zoomScale="85" zoomScaleNormal="85""#));
        assert_eq!(set_zoom(&zoomed, 100).unwrap(), xml);
        let preview = set_page_break_preview(xml, true).unwrap();
        let zoomed = set_zoom(&preview, 1000).unwrap();
        assert!(zoomed.contains(r#"zoomScale="400" zoomScaleSheetLayoutView="400""#));
    }

    #[test]
    fn zeros_are_hidden_with_show_zeros_0_and_shown_by_taking_it_away() {
        let xml = r#"<worksheet><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetData/></worksheet>"#;
        let hidden = set_show_zeros(xml, false).unwrap();
        assert!(hidden.contains(r#"<sheetView workbookViewId="0" showZeros="0"/>"#));
        assert_eq!(set_show_zeros(&hidden, true).unwrap(), xml);
    }
}
