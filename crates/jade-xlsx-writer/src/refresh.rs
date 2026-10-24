//! Refreshing the cached `<v>` of formula cells in place.
//!
//! Recalculation changes what a formula's cached result is, not what the cell
//! is — so unlike [`crate::patch_sheet`], which replaces whole cells, this
//! touches at most the cell's `t` attribute and its `<v>` element. The
//! formula itself — a shared master's `ref`, a follower's `si` — and any
//! other child the cell carries survive byte for byte.

use std::collections::{HashMap, HashSet};

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::patch::{
    apply, cell_attribute as attribute, escape as xml_escape, number_text, qualified_name, Splice,
};
use crate::{Result, WriteError};
use jade_xlsx_reader::CellRef;

/// A freshly computed result for a formula cell.
#[derive(Debug, Clone, PartialEq)]
pub enum CachedValue {
    Number(f64),
    Text(String),
    Bool(bool),
    /// `#DIV/0!` and friends, verbatim.
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CachedValueEdit {
    pub reference: CellRef,
    pub value: CachedValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SheetCachedValues {
    /// The part name inside the archive, e.g. `xl/worksheets/sheet1.xml`.
    pub part_name: String,
    pub cells: Vec<CachedValueEdit>,
}

/// The `t` attribute a cached value needs on its `<c>`, `None` for numeric.
fn type_attribute(value: &CachedValue) -> Option<&'static str> {
    match value {
        CachedValue::Number(_) => None,
        CachedValue::Text(_) => Some("str"),
        CachedValue::Bool(_) => Some("b"),
        CachedValue::Error(_) => Some("e"),
    }
}

fn value_text(value: &CachedValue) -> String {
    match value {
        CachedValue::Number(number) => number_text(*number),
        CachedValue::Text(text) => xml_escape(&jade_xlsx_reader::xstring::encode_xstring(text)),
        CachedValue::Bool(flag) => u8::from(*flag).to_string(),
        CachedValue::Error(error) => xml_escape(error),
    }
}

/// Refreshes cached values in one worksheet part.
///
/// A reference that names a cell without a formula (or a cell the part does
/// not write) is ignored: there is no cached result there to refresh, and
/// replacing a stored value is [`crate::patch_sheet`]'s job.
pub fn refresh_sheet(xml: &str, edits: &[CachedValueEdit]) -> Result<String> {
    for edit in edits {
        if matches!(edit.value, CachedValue::Number(number) if !number.is_finite()) {
            return Err(WriteError::NonFiniteNumber {
                column: edit.reference.column,
                row: edit.reference.row,
            });
        }
    }
    let mut pending: HashMap<CellRef, &CachedValue> = HashMap::new();
    for edit in edits {
        pending.insert(edit.reference, &edit.value);
    }

    let references = pending.keys().copied().collect();
    let cells = jade_xlsx_reader::cell_content::compatible_cells(xml, &references)?;
    let eligible: HashSet<_> = cells
        .iter()
        .filter(|cell| cell.selected && cell.selected_formula)
        .map(|cell| cell.reference)
        .collect();
    let mut splices = Vec::new();
    for cell in cells {
        if !eligible.contains(&cell.reference) {
            continue;
        }
        let Some(value) = pending.get(&cell.reference) else {
            continue;
        };
        if !cell.has_formula {
            continue;
        }
        let mut reader = Reader::from_str(&xml[cell.range.start..cell.head_end]);
        let event = reader.read_event()?;
        let Event::Start(ref element) = event else {
            continue;
        };
        let wanted = type_attribute(value);
        if attribute(element, b"t")?.as_deref() != wanted {
            let mut head = format!("<{}", qualified_name(element.name().as_ref()));
            for a in element.attributes() {
                let a = a?;
                if a.key.as_ref() != b"t" {
                    head.push(' ');
                    head.push_str(&String::from_utf8_lossy(a.key.as_ref()));
                    head.push_str("=\"");
                    head.push_str(&xml_escape(&a.unescape_value()?));
                    head.push('"');
                }
            }
            if let Some(kind) = wanted {
                head.push_str(&format!(" t=\"{kind}\""));
            }
            head.push('>');
            splices.push(Splice {
                start: cell.range.start,
                end: cell.head_end,
                text: head,
            });
        }
        let encoded = value_text(value);
        let insertion = cell.missing_cache_insertion();
        for span in cell.values {
            let mut value_reader = Reader::from_str(&xml[span.clone()]);
            if let Event::Start(value_element) | Event::Empty(value_element) =
                value_reader.read_event()?
            {
                let head = crate::patch::element_head(&value_element)?;
                let name = qualified_name(value_element.name().as_ref());
                splices.push(Splice {
                    start: span.start,
                    end: span.end,
                    text: format!("{head}{encoded}</{name}>"),
                });
            }
        }
        if let Some(insertion) = insertion {
            match insertion {
                jade_xlsx_reader::cell_content::CacheInsertion::AfterFormula(span) => {
                    let mut formula_reader = Reader::from_str(&xml[span.clone()]);
                    if let Event::Start(formula) | Event::Empty(formula) =
                        formula_reader.read_event()?
                    {
                        let prefix = crate::patch::namespace_prefix(formula.name().as_ref());
                        let value = format!("<{prefix}v>{encoded}</{prefix}v>");
                        let value = crate::patch::preserve_namespaces(value, &formula)?;
                        splices.push(Splice {
                            start: span.end,
                            end: span.end,
                            text: value,
                        });
                    }
                }
                jade_xlsx_reader::cell_content::CacheInsertion::InBranch(at) => {
                    let value =
                        format!("<v xmlns=\"{}\">{encoded}</v>", xml_escape(&cell.namespace));
                    splices.push(Splice {
                        start: at,
                        end: at,
                        text: value,
                    });
                }
                jade_xlsx_reader::cell_content::CacheInsertion::OpenEmpty(span) => {
                    let mut branch_reader = Reader::from_str(&xml[span.clone()]);
                    if let Event::Empty(branch) = branch_reader.read_event()? {
                        let head = crate::patch::element_head(&branch)?;
                        let close = qualified_name(branch.name().as_ref());
                        let value =
                            format!("<v xmlns=\"{}\">{encoded}</v>", xml_escape(&cell.namespace));
                        splices.push(Splice {
                            start: span.start,
                            end: span.end,
                            text: format!("{head}{value}</{close}>"),
                        });
                    }
                }
            }
        }
    }
    Ok(apply(xml, splices))
}
