//! `<ignoredErrors>`: the cells whose background error checks Excel was told
//! to leave alone (Ignore Error on the error button), so they show no green
//! triangle.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{CellRange, CellRef};
use jade_opc::local_name;

/// One `<ignoredError>`: the ranges it covers and the checks it turns off.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredError {
    pub ranges: Vec<CellRange>,
    /// `evalError`: a formula giving an error.
    pub eval_error: bool,
    /// `numberStoredAsText`: text that reads as a number.
    pub number_stored_as_text: bool,
}

impl IgnoredError {
    pub fn covers(&self, cell: CellRef) -> bool {
        self.ranges.iter().any(|range| range.contains(cell))
    }
}

pub(crate) fn parse(xml: &str) -> Result<Vec<IgnoredError>> {
    if !xml.contains("ignoredError") {
        return Ok(Vec::new());
    }
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut inside = false;
    let mut ignored = Vec::new();
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let name = e.name();
                let local = local_name(name.as_ref());
                if depth == 1 && local == b"ignoredErrors" {
                    inside = !empty;
                }
                if depth == 2 && inside && local == b"ignoredError" {
                    let mut entry = IgnoredError::default();
                    for a in e.attributes() {
                        let a = a?;
                        let value = a.unescape_value()?;
                        let on = matches!(value.as_ref(), "1" | "true");
                        match a.key.as_ref() {
                            b"sqref" => {
                                entry.ranges = value
                                    .split_whitespace()
                                    .filter_map(|range| CellRange::parse(range).ok())
                                    .collect();
                            }
                            b"evalError" => entry.eval_error = on,
                            b"numberStoredAsText" => entry.number_stored_as_text = on,
                            _ => {}
                        }
                    }
                    if !entry.ranges.is_empty() {
                        ignored.push(entry);
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 1 {
                    inside = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(ignored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_ranges_and_the_checks_each_entry_turns_off() {
        let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/><ignoredErrors><ignoredError sqref="A1:A3 C5" numberStoredAsText="1"/><ignoredError sqref="D2" evalError="1"/></ignoredErrors></worksheet>"#;
        let ignored = parse(xml).unwrap();
        assert_eq!(ignored.len(), 2);
        assert!(ignored[0].number_stored_as_text && !ignored[0].eval_error);
        assert!(ignored[0].covers(CellRef::new(2, 4)));
        assert!(ignored[0].covers(CellRef::new(0, 1)));
        assert!(!ignored[0].covers(CellRef::new(1, 1)));
        assert!(ignored[1].eval_error && ignored[1].covers(CellRef::new(3, 1)));
    }
}
