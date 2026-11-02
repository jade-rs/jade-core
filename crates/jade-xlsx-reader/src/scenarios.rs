//! `<scenarios>`: the What-If scenarios the Scenario Manager keeps on a
//! sheet — each a name and a value for each of its changing cells — and
//! the result cells its last summary reported.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{CellRange, CellRef};
use jade_opc::local_name;

/// A sheet's scenarios.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenarios {
    /// `current`: the scenario last selected in the manager.
    pub current: Option<u32>,
    /// `show`: the scenario last shown.
    pub show: Option<u32>,
    /// `sqref`: the result cells the last summary reported.
    pub result_cells: Vec<CellRange>,
    pub scenarios: Vec<Scenario>,
}

/// One `<scenario>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// Prevent changes: the scenario cannot be edited while the sheet is
    /// protected.
    pub locked: bool,
    /// Hide: not listed while the sheet is protected.
    pub hidden: bool,
    /// Who made or last changed it.
    pub user: Option<String>,
    pub comment: Option<String>,
    /// Each changing cell and the value the scenario gives it, as typed.
    pub cells: Vec<(CellRef, String)>,
}

pub(crate) fn parse(xml: &str) -> Result<Option<Scenarios>> {
    if !xml.contains("scenarios") {
        return Ok(None);
    }
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut found: Option<Scenarios> = None;
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let name = e.name();
                let local = local_name(name.as_ref());
                match (depth, local) {
                    (1, b"scenarios") => {
                        let mut scenarios = Scenarios::default();
                        for a in e.attributes() {
                            let a = a?;
                            let value = a.unescape_value()?;
                            match a.key.as_ref() {
                                b"current" => scenarios.current = value.parse().ok(),
                                b"show" => scenarios.show = value.parse().ok(),
                                b"sqref" => {
                                    scenarios.result_cells = value
                                        .split_whitespace()
                                        .filter_map(|range| CellRange::parse(range).ok())
                                        .collect()
                                }
                                _ => {}
                            }
                        }
                        found = Some(scenarios);
                    }
                    (2, b"scenario") => {
                        let mut scenario = Scenario::default();
                        for a in e.attributes() {
                            let a = a?;
                            let value = a.unescape_value()?;
                            let on = matches!(value.as_ref(), "1" | "true");
                            match a.key.as_ref() {
                                b"name" => scenario.name = value.into_owned(),
                                b"locked" => scenario.locked = on,
                                b"hidden" => scenario.hidden = on,
                                b"user" => scenario.user = Some(value.into_owned()),
                                b"comment" => scenario.comment = Some(value.into_owned()),
                                _ => {}
                            }
                        }
                        if let Some(scenarios) = found.as_mut() {
                            scenarios.scenarios.push(scenario);
                        }
                    }
                    (3, b"inputCells") => {
                        let mut at = None;
                        let mut value = String::new();
                        for a in e.attributes() {
                            let a = a?;
                            let text = a.unescape_value()?;
                            match a.key.as_ref() {
                                b"r" => at = CellRef::parse(&text).ok(),
                                b"val" => value = text.into_owned(),
                                _ => {}
                            }
                        }
                        if let (Some(at), Some(scenario)) = (
                            at,
                            found
                                .as_mut()
                                .and_then(|scenarios| scenarios.scenarios.last_mut()),
                        ) {
                            scenario.cells.push((at, value));
                        }
                    }
                    _ => {}
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_scenario_and_its_cells() {
        let xml = r#"<worksheet xmlns="m"><sheetData/><scenarios current="1" show="0" sqref="B5 C5"><scenario name="Best" locked="1" count="2" user="Ann" comment="Made by Ann"><inputCells r="B1" val="10"/><inputCells r="B2" val="0.5"/></scenario><scenario name="Worst" count="1"><inputCells r="B1" val="1"/></scenario></scenarios></worksheet>"#;
        let scenarios = parse(xml).unwrap().unwrap();
        assert_eq!(scenarios.current, Some(1));
        assert_eq!(scenarios.result_cells.len(), 2);
        assert_eq!(scenarios.scenarios.len(), 2);
        let best = &scenarios.scenarios[0];
        assert!(best.locked && !best.hidden);
        assert_eq!(best.comment.as_deref(), Some("Made by Ann"));
        assert_eq!(
            best.cells,
            vec![
                (CellRef::new(1, 0), "10".to_string()),
                (CellRef::new(1, 1), "0.5".to_string())
            ]
        );
        assert!(parse("<worksheet><sheetData/></worksheet>")
            .unwrap()
            .is_none());
    }
}
