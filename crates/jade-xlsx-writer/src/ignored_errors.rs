//! Ignore Error: a cell's background error check turned off, saved as an
//! `<ignoredError>` in the sheet's `<ignoredErrors>`.
//!
//! `<ignoredErrors>` sits after `cellWatches` and before `smartTags`; Excel
//! refuses a sheet whose children are out of order, so a new list is
//! inserted before the first child that follows it.

use quick_xml::events::Event;
use quick_xml::Reader;

use jade_xlsx_reader::CellRange;

use crate::header_footer::set_child;
use crate::patch::{apply, local, Splice};
use crate::Result;

/// Worksheet children that come after `ignoredErrors`, in schema order.
const FOLLOWING: &[&[u8]] = &[
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

/// The background error checks Ignore Error can turn off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCheck {
    /// `numberStoredAsText`: text that reads as a number.
    NumberStoredAsText,
    /// `evalError`: a formula giving an error.
    EvalError,
}

impl ErrorCheck {
    fn attribute(self) -> &'static str {
        match self {
            Self::NumberStoredAsText => "numberStoredAsText",
            Self::EvalError => "evalError",
        }
    }
}

/// The sheet part with `check` ignored over `range`: an `<ignoredError>`
/// added to its `<ignoredErrors>`, which is made when it has none.
pub fn ignore_error(xml: &str, range: CellRange, check: ErrorCheck) -> Result<String> {
    let sqref = crate::shift::range_text(range);
    let entry = |prefix: &str| {
        format!(
            "<{prefix}ignoredError sqref=\"{sqref}\" {}=\"1\"/>",
            check.attribute()
        )
    };
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = e.name().as_ref().to_vec();
                if depth == 1 && local(&qualified) == b"ignoredErrors" {
                    let prefix = crate::patch::namespace_prefix(&qualified);
                    let qualified = String::from_utf8_lossy(&qualified).into_owned();
                    let end = reader.buffer_position() as usize;
                    let splice = if empty {
                        let head = xml[start..end].trim_end_matches("/>");
                        Splice {
                            start,
                            end,
                            text: format!("{head}>{}</{qualified}>", entry(&prefix)),
                        }
                    } else {
                        // The new entry goes last, before the list's end tag.
                        reader.read_to_end_into(e.name(), &mut Vec::new())?;
                        let close = reader.buffer_position() as usize;
                        let at = xml[..close].rfind("</").unwrap_or(close);
                        Splice {
                            start: at,
                            end: at,
                            text: entry(&prefix),
                        }
                    };
                    return Ok(apply(xml, vec![splice]));
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    set_child(
        xml,
        b"ignoredErrors",
        |prefix| {
            format!(
                "<{prefix}ignoredErrors>{}</{prefix}ignoredErrors>",
                entry(prefix)
            )
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
    fn makes_the_list_in_its_place_and_adds_to_one_already_there() {
        let xml = r#"<worksheet xmlns="m"><sheetData/><pageMargins left="1"/><drawing r:id="rId1"/></worksheet>"#;
        let once = ignore_error(xml, range("B3"), ErrorCheck::NumberStoredAsText).unwrap();
        assert_eq!(
            once,
            r#"<worksheet xmlns="m"><sheetData/><pageMargins left="1"/><ignoredErrors><ignoredError sqref="B3" numberStoredAsText="1"/></ignoredErrors><drawing r:id="rId1"/></worksheet>"#
        );
        let twice = ignore_error(&once, range("C4:D5"), ErrorCheck::EvalError).unwrap();
        assert!(twice.contains(r#"numberStoredAsText="1"/><ignoredError sqref="C4:D5" evalError="1"/></ignoredErrors><drawing"#));
        let sheet = jade_xlsx_reader::sheet::parse_sheet("S", "s.xml", &twice, &[]).unwrap();
        assert_eq!(sheet.ignored_errors.len(), 2);
    }

    #[test]
    fn keeps_the_root_prefix() {
        let xml = r#"<x:worksheet xmlns:x="m"><x:sheetData/></x:worksheet>"#;
        let out = ignore_error(xml, range("A1"), ErrorCheck::EvalError).unwrap();
        assert!(out.contains(r#"<x:ignoredErrors><x:ignoredError sqref="A1" evalError="1"/></x:ignoredErrors></x:worksheet>"#));
    }
}
