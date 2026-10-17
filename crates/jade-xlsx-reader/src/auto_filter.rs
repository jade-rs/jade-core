//! Saved worksheet/table filters, without reapplying their predicates.
use crate::{CellRange, Result};
use quick_xml::{
    events::{BytesStart, Event},
    name::ResolveResult,
    NsReader,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterCriterion {
    pub kind: String,
    pub attributes: BTreeMap<String, String>,
    pub children: Vec<FilterCriterion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterColumn {
    pub column_id: u32,
    pub hidden_button: bool,
    pub show_button: bool,
    pub criterion: Option<FilterCriterion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFilter {
    pub reference: Option<CellRange>,
    pub columns: Vec<FilterColumn>,
    pub issues: Vec<String>,
}

impl AutoFilter {
    pub fn has_criteria(&self) -> bool {
        self.columns.iter().any(|column| column.criterion.is_some())
    }
}

fn attribute(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
fn enabled(value: Option<String>, default: bool) -> bool {
    value
        .map(|value| value != "0" && value != "false")
        .unwrap_or(default)
}
fn root_namespace(namespace: ResolveResult<'_>) -> Option<Vec<u8>> {
    match namespace {
        ResolveResult::Unbound => Some(vec![]),
        ResolveResult::Bound(namespace)
            if matches!(
                namespace.as_ref(),
                b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                    | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
            ) =>
        {
            Some(namespace.as_ref().to_vec())
        }
        _ => None,
    }
}
fn criterion(element: &BytesStart<'_>, kind: &[u8]) -> Result<FilterCriterion> {
    let mut attributes = BTreeMap::new();
    for attribute in element.attributes() {
        let attribute = attribute?;
        if !attribute.key.as_ref().contains(&b':') {
            let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
            let value = attribute.unescape_value()?;
            let value = if key == "val" {
                crate::xstring::decode_xstring(&value)
            } else {
                value.into_owned()
            };
            attributes.insert(key, value);
        }
    }
    Ok(FilterCriterion {
        kind: String::from_utf8_lossy(kind).into_owned(),
        attributes,
        children: vec![],
    })
}

/// Only direct children of a recognized worksheet/table root belong to this view.
pub fn parse(xml: &str, root: &[u8]) -> Result<(Option<AutoFilter>, Option<bool>)> {
    let mut reader = NsReader::from_str(xml);
    let mut depth = 0usize;
    let mut namespace = None;
    let mut filter: Option<AutoFilter> = None;
    let mut mode = None;
    let mut inside = false;
    let mut column = None;
    let mut stack: Vec<(usize, FilterCriterion)> = vec![];
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name();
                let (resolved, local) = reader.resolver().resolve_element(name);
                if depth == 0 && local.as_ref() == root {
                    namespace = root_namespace(resolved);
                } else if namespace.as_ref().is_some_and(|root| match resolved {
                    ResolveResult::Unbound => root.is_empty(),
                    ResolveResult::Bound(namespace) => namespace.as_ref() == root.as_slice(),
                    _ => false,
                }) {
                    if depth == 1 && root == b"worksheet" && local.as_ref() == b"sheetPr" {
                        if let Some(value) = attribute(element, b"filterMode")? {
                            mode = Some(enabled(Some(value), false));
                        }
                    }
                    if depth == 1 && local.as_ref() == b"autoFilter" {
                        if let Some(filter) = &mut filter {
                            filter.issues.push("multiple autoFilter elements".into());
                        } else {
                            let raw = attribute(element, b"ref")?;
                            let reference = raw
                                .as_deref()
                                .and_then(|value| CellRange::parse(value).ok())
                                .filter(|range| {
                                    range.start.row <= range.end.row
                                        && range.start.column <= range.end.column
                                });
                            let issues = if reference.is_none() {
                                vec![if raw.is_none() {
                                    "missing filter range"
                                } else {
                                    "invalid filter range"
                                }
                                .into()]
                            } else {
                                vec![]
                            };
                            filter = Some(AutoFilter {
                                reference,
                                columns: vec![],
                                issues,
                            });
                        }
                        inside = !empty;
                    } else if inside && depth == 2 && local.as_ref() == b"filterColumn" {
                        let Some(id) =
                            attribute(element, b"colId")?.and_then(|value| value.parse().ok())
                        else {
                            filter
                                .as_mut()
                                .unwrap()
                                .issues
                                .push("invalid filter column ID".into());
                            depth += usize::from(!empty);
                            continue;
                        };
                        let node = FilterColumn {
                            column_id: id,
                            hidden_button: enabled(attribute(element, b"hiddenButton")?, false),
                            show_button: enabled(attribute(element, b"showButton")?, true),
                            criterion: None,
                        };
                        if filter
                            .as_ref()
                            .unwrap()
                            .columns
                            .iter()
                            .any(|column| column.column_id == id)
                        {
                            filter
                                .as_mut()
                                .unwrap()
                                .issues
                                .push("duplicate filter column ID".into());
                        }
                        filter.as_mut().unwrap().columns.push(node);
                        column = (!empty).then_some(filter.as_ref().unwrap().columns.len() - 1);
                    } else if inside && column.is_some() && !stack.is_empty() && depth > 64 {
                        let filter = filter.as_mut().unwrap();
                        if !filter
                            .issues
                            .iter()
                            .any(|issue| issue == "filter criterion nesting exceeds limit")
                        {
                            filter
                                .issues
                                .push("filter criterion nesting exceeds limit".into());
                        }
                    } else if let Some(column) = column.filter(|_| {
                        inside
                            && depth >= 3
                            && local.as_ref() != b"extLst"
                            && (depth == 3
                                || stack
                                    .last()
                                    .is_some_and(|(parent_depth, _)| depth == parent_depth + 1))
                    }) {
                        if depth == 3
                            && filter.as_ref().unwrap().columns[column].criterion.is_some()
                        {
                            filter
                                .as_mut()
                                .unwrap()
                                .issues
                                .push("multiple filter criteria".into());
                        }
                        let node = criterion(element, local.as_ref())?;
                        if depth == 3
                            && !matches!(
                                local.as_ref(),
                                b"filters"
                                    | b"customFilters"
                                    | b"dynamicFilter"
                                    | b"top10"
                                    | b"colorFilter"
                                    | b"iconFilter"
                            )
                        {
                            filter
                                .as_mut()
                                .unwrap()
                                .issues
                                .push("unrecognized filter criterion".into());
                        }
                        if empty {
                            if let Some((_, parent)) = stack.last_mut() {
                                parent.children.push(node);
                            } else {
                                filter.as_mut().unwrap().columns[column].criterion = Some(node);
                            }
                        } else {
                            stack.push((depth, node));
                        }
                    }
                }
                depth += usize::from(!empty);
            }
            Event::End(ref element) => {
                depth = depth.saturating_sub(1);
                let (resolved, local) = reader.resolver().resolve_element(element.name());
                let main = namespace.as_ref().is_some_and(|root| match resolved {
                    ResolveResult::Unbound => root.is_empty(),
                    ResolveResult::Bound(namespace) => namespace.as_ref() == root.as_slice(),
                    _ => false,
                });
                if main && inside {
                    if depth >= 3
                        && stack.last().is_some_and(|(node_depth, node)| {
                            *node_depth == depth && node.kind.as_bytes() == local.as_ref()
                        })
                    {
                        let (_, node) = stack.pop().unwrap();
                        if let Some((_, parent)) = stack.last_mut() {
                            parent.children.push(node);
                        } else if let Some(column) = column {
                            filter.as_mut().unwrap().columns[column].criterion = Some(node);
                        }
                    } else if depth == 2 && local.as_ref() == b"filterColumn" {
                        column = None;
                        stack.clear();
                    } else if depth == 1 && local.as_ref() == b"autoFilter" {
                        inside = false;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if let Some(filter) = &mut filter {
        if let Some(range) = filter.reference {
            for column in &filter.columns {
                if column.column_id > range.end.column.saturating_sub(range.start.column) {
                    filter
                        .issues
                        .push("filter column is outside its range".into());
                }
            }
        }
    }
    Ok((filter, mode))
}
