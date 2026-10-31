//! A sheet's view settings on its first `<sheetView>`: Show Formulas.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::patch::{apply, local, Splice};
use crate::shift::rewrite;
use crate::Result;

/// The sheet part with `showFormulas` set on its first `<sheetView>`, or
/// taken away when `on` is false. A sheet with no `<sheetView>` is left as
/// it is.
pub fn set_show_formulas(xml: &str, on: bool) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e)
                if local(e.name().as_ref()) == b"sheetView" =>
            {
                let text = rewrite(
                    e,
                    &[("showFormulas", on.then(|| "1".to_string()))],
                    matches!(event, Event::Empty(_)),
                )?;
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
        assert_eq!(set_show_formulas(bare, true).unwrap(), bare);
    }
}
