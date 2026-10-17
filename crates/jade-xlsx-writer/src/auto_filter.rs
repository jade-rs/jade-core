//! Edit filter predicates and row visibility without changing dimensions or cells.
use crate::{
    patch::{apply, attribute, escape, namespace_prefix, Splice},
    Result, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{auto_filter::FilterCriterion, CellRange, Workbook};
use quick_xml::{
    events::Event,
    name::{QName, ResolveResult},
    NsReader,
};
use std::collections::BTreeMap;

pub struct AutoFilterWrite<'a> {
    pub table: Option<&'a str>,
    pub reference: CellRange,
    pub column: Option<u32>,
    pub criterion: Option<&'a FilterCriterion>,
    pub hidden: &'a BTreeMap<u32, bool>,
    pub filter_mode: bool,
}

#[derive(Clone)]
pub(super) struct Node {
    pub(super) start: usize,
    pub(super) head: usize,
    pub(super) close: usize,
    pub(super) end: usize,
    pub(super) name: String,
    pub(super) local: String,
    pub(super) empty: bool,
}
pub(super) fn children(xml: &str, root: &str) -> Result<(Node, Vec<Node>)> {
    let mut reader = NsReader::from_str(xml);
    let mut parent: Option<Node> = None;
    let mut namespace: Option<Vec<u8>> = None;
    let mut nodes = vec![];
    let mut depth = 0;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let head = reader.buffer_position() as usize;
                let empty = xml[..head].ends_with("/>");
                let name = element.name();
                let (resolved, local) = reader.resolver().resolve_element(name);
                let ns = match resolved {
                    ResolveResult::Unbound => Some(vec![]),
                    ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    _ => None,
                };
                if depth == 0
                    && local.as_ref() == root.as_bytes()
                    && ns.as_ref().is_some_and(|ns| {
                        ns.is_empty()
                            || matches!(
                                ns.as_slice(),
                                b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                    | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                            )
                    })
                {
                    namespace = ns;
                    parent = Some(Node {
                        start,
                        head,
                        close: head,
                        end: head,
                        name: String::from_utf8_lossy(name.as_ref()).into_owned(),
                        local: root.into(),
                        empty,
                    });
                } else if depth == 1 && parent.is_some() && ns == namespace {
                    let name = name.as_ref().to_vec();
                    let local = String::from_utf8_lossy(local.as_ref()).into_owned();
                    if !empty {
                        reader.read_to_end(QName(&name))?;
                    }
                    let end = reader.buffer_position() as usize;
                    let close = if empty {
                        head
                    } else {
                        xml[..end].rfind("</").unwrap_or(head)
                    };
                    nodes.push(Node {
                        start,
                        head,
                        close,
                        end,
                        name: String::from_utf8_lossy(&name).into_owned(),
                        local,
                        empty,
                    });
                    continue;
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    if let Some(parent) = &mut parent {
                        parent.close = start;
                        parent.end = reader.buffer_position() as usize;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((
        parent.ok_or_else(|| WriteError::AutoFilter(format!("missing {root} root")))?,
        nodes,
    ))
}
pub(crate) fn head_attribute(head: &str, key: &str, value: &str) -> Result<String> {
    let mut reader = NsReader::from_str(head);
    let element = match reader.read_event()? {
        Event::Start(ref element) | Event::Empty(ref element) => element.clone().into_owned(),
        _ => return Err(WriteError::AutoFilter("invalid XML head".into())),
    };
    let raw: &[u8] = element.as_ref();
    for attr in element.attributes() {
        let attr = attr?;
        if attr.key.as_ref() == key.as_bytes() {
            let at = 1 + attr.value.as_ptr() as usize - raw.as_ptr() as usize;
            return Ok(apply(
                head,
                vec![Splice {
                    start: at,
                    end: at + attr.value.len(),
                    text: escape(value),
                }],
            ));
        }
    }
    let at = head.len() - if head.ends_with("/>") { 2 } else { 1 };
    Ok(apply(
        head,
        vec![Splice {
            start: at,
            end: at,
            text: format!(" {key}=\"{}\"", escape(value)),
        }],
    ))
}
fn node_attribute(xml: &str, node: &Node, key: &[u8]) -> Result<Option<String>> {
    let mut reader = NsReader::from_str(&xml[node.start..node.head]);
    match reader.read_event()? {
        Event::Start(ref element) | Event::Empty(ref element) => {
            for attr in element.attributes() {
                let attr = attr?;
                if attr.key.as_ref() == key {
                    return Ok(Some(attr.unescape_value()?.into_owned()));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}
fn criterion_xml(node: &FilterCriterion, prefix: &str) -> String {
    let attributes = node
        .attributes
        .iter()
        .map(|(key, value)| {
            let value = if key == "val" {
                jade_xlsx_reader::xstring::encode_xstring(value)
            } else {
                value.clone()
            };
            format!(" {key}=\"{}\"", escape(&value))
        })
        .collect::<String>();
    if node.children.is_empty() {
        format!("<{prefix}{}{attributes}/>", node.kind)
    } else {
        format!(
            "<{prefix}{}{attributes}>{}</{prefix}{}>",
            node.kind,
            node.children
                .iter()
                .map(|child| criterion_xml(child, prefix))
                .collect::<String>(),
            node.kind
        )
    }
}
fn replace_children(xml: &str, parent: &Node, splices: Vec<Splice>, insertion: &str) -> String {
    if parent.empty {
        let head = &xml[parent.start..parent.head - 2];
        return apply(
            xml,
            vec![Splice {
                start: parent.start,
                end: parent.end,
                text: format!("{head}>{insertion}</{}>", parent.name),
            }],
        );
    }
    let mut splices = splices;
    if !insertion.is_empty() {
        splices.push(Splice {
            start: parent.close,
            end: parent.close,
            text: insertion.into(),
        });
    }
    apply(xml, splices)
}
fn edit_column(xml: &str, node: &Node, criterion: Option<&FilterCriterion>) -> Result<String> {
    // Bind the inherited namespace while inspecting a standalone column fragment.
    let fragment = &xml[node.start..node.end];
    let mut reader = NsReader::from_str(xml);
    let mut depth = 0;
    let mut splices = vec![];
    let mut insert = node.close - node.start;
    let mut namespace: Option<Vec<u8>> = None;
    // Namespace resolution uses the complete filter XML, including inherited aliases.
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(element.name());
                let ns = match ns {
                    ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    ResolveResult::Unbound => Some(vec![]),
                    _ => None,
                };
                if start == node.start {
                    namespace = ns;
                    depth = 1;
                } else if depth > 0 {
                    if depth == 1 && ns == namespace {
                        if local.as_ref() == b"extLst" {
                            insert = start - node.start;
                        } else {
                            let name = element.name().as_ref().to_vec();
                            if !empty {
                                reader.read_to_end(QName(&name))?;
                            }
                            splices.push(Splice {
                                start: start - node.start,
                                end: reader.buffer_position() as usize - node.start,
                                text: String::new(),
                            });
                            continue;
                        }
                    }
                    depth += usize::from(!empty);
                }
            }
            Event::End(_) if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        if node.empty && start == node.start {
            break;
        }
    }
    let added = criterion
        .map(|criterion| criterion_xml(criterion, &namespace_prefix(node.name.as_bytes())))
        .unwrap_or_default();
    if node.empty {
        if added.is_empty() {
            return Ok(fragment.into());
        }
        return Ok(format!(
            "{}>{added}</{}>",
            &fragment[..fragment.len() - 2],
            node.name
        ));
    }
    if !added.is_empty() {
        splices.push(Splice {
            start: insert,
            end: insert,
            text: added,
        });
    }
    Ok(apply(fragment, splices))
}
fn edit_filter(xml: &str, update: &AutoFilterWrite<'_>) -> Result<String> {
    let (root, nodes) = children(xml, "autoFilter")?;
    let mut splices = vec![];
    let mut found = false;
    for node in nodes.iter().filter(|node| node.local == "filterColumn") {
        let id = node_attribute(xml, node, b"colId")?.and_then(|id| id.parse::<u32>().ok());
        if update.column.is_none() || id == update.column {
            found = true;
            splices.push(Splice {
                start: node.start,
                end: node.end,
                text: edit_column(xml, node, update.criterion)?,
            });
        }
    }
    let mut added = String::new();
    if !found {
        if let (Some(column), Some(criterion)) = (update.column, update.criterion) {
            let prefix = namespace_prefix(root.name.as_bytes());
            added = format!(
                "<{prefix}filterColumn colId=\"{column}\">{}</{prefix}filterColumn>",
                criterion_xml(criterion, &prefix)
            );
            // filterColumn must precede sortState and extLst.
            if !root.empty {
                let at = nodes
                    .iter()
                    .find(|node| matches!(node.local.as_str(), "sortState" | "extLst"))
                    .map(|node| node.start)
                    .unwrap_or(root.close);
                splices.push(Splice {
                    start: at,
                    end: at,
                    text: added.clone(),
                });
                added.clear();
            }
        }
    }
    let result = replace_children(xml, &root, splices, &added);
    let (root, _) = children(&result, "autoFilter")?;
    Ok(apply(
        &result,
        vec![Splice {
            start: root.start,
            end: root.head,
            text: head_attribute(
                &result[root.start..root.head],
                "ref",
                &crate::shift::range_text(update.reference),
            )?,
        }],
    ))
}
fn edit_parent(xml: &str, root: &str, update: &AutoFilterWrite<'_>) -> Result<String> {
    let (parent, nodes) = children(xml, root)?;
    if let Some(filter) = nodes.iter().find(|node| node.local == "autoFilter") {
        let original = &xml[filter.start..filter.end];
        let mut reader = NsReader::from_str(&xml[parent.start..parent.head]);
        let inherited = match reader.read_event()? {
            Event::Start(ref element) | Event::Empty(ref element) => element
                .attributes()
                .map(|attr| {
                    let attr = attr?;
                    Ok((
                        String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                        attr.unescape_value()?.into_owned(),
                    ))
                })
                .collect::<Result<Vec<(String, String)>>>()?,
            _ => vec![],
        };
        let mut reader = NsReader::from_str(&xml[filter.start..filter.head]);
        let existing = match reader.read_event()? {
            Event::Start(ref element) | Event::Empty(ref element) => element
                .attributes()
                .map(|attr| {
                    attr.map(|attr| String::from_utf8_lossy(attr.key.as_ref()).into_owned())
                })
                .collect::<std::result::Result<Vec<_>, _>>()?,
            _ => vec![],
        };
        let added = inherited
            .into_iter()
            .filter(|(key, _)| {
                (key == "xmlns" || key.starts_with("xmlns:")) && !existing.contains(key)
            })
            .map(|(key, value)| format!(" {key}=\"{}\"", escape(&value)))
            .collect::<String>();
        let at = filter.head - filter.start - if filter.empty { 2 } else { 1 };
        let bound = format!("{}{}{}", &original[..at], added, &original[at..]);
        let updated = edit_filter(&bound, update)?;
        let updated = if added.is_empty() {
            updated
        } else {
            updated.replacen(&added, "", 1)
        };
        return Ok(apply(
            xml,
            vec![Splice {
                start: filter.start,
                end: filter.end,
                text: updated,
            }],
        ));
    }
    let prefix = namespace_prefix(parent.name.as_bytes());
    let criterion = update
        .criterion
        .map(|c| {
            format!(
                "<{prefix}filterColumn colId=\"{}\">{}</{prefix}filterColumn>",
                update.column.unwrap_or(0),
                criterion_xml(c, &prefix)
            )
        })
        .unwrap_or_default();
    let filter = format!(
        "<{prefix}autoFilter ref=\"{}\">{criterion}</{prefix}autoFilter>",
        crate::shift::range_text(update.reference)
    );
    let before = if root == "worksheet" {
        &[
            "sortState",
            "dataConsolidate",
            "customSheetViews",
            "mergeCells",
            "phoneticPr",
            "conditionalFormatting",
            "dataValidations",
            "hyperlinks",
            "printOptions",
            "pageMargins",
            "pageSetup",
            "headerFooter",
            "rowBreaks",
            "colBreaks",
            "customProperties",
            "cellWatches",
            "ignoredErrors",
            "smartTags",
            "drawing",
            "legacyDrawing",
            "legacyDrawingHF",
            "picture",
            "oleObjects",
            "controls",
            "webPublishItems",
            "tableParts",
            "extLst",
        ][..]
    } else {
        &["sortState", "tableColumns", "tableStyleInfo", "extLst"][..]
    };
    let at = nodes
        .iter()
        .find(|node| before.contains(&node.local.as_str()))
        .map(|node| node.start)
        .unwrap_or(parent.close);
    Ok(replace_children(
        xml,
        &parent,
        vec![Splice {
            start: at,
            end: at,
            text: filter.clone(),
        }],
        if parent.empty { &filter } else { "" },
    ))
}
fn patch_visibility(xml: &str, hidden: &BTreeMap<u32, bool>, active: bool) -> Result<String> {
    let (root, nodes) = children(xml, "worksheet")?;
    let mut splices = vec![];
    let prefix = namespace_prefix(root.name.as_bytes());
    if let Some(properties) = nodes.iter().find(|node| node.local == "sheetPr") {
        splices.push(Splice {
            start: properties.start,
            end: properties.head,
            text: head_attribute(
                &xml[properties.start..properties.head],
                "filterMode",
                if active { "1" } else { "0" },
            )?,
        });
    } else {
        splices.push(Splice {
            start: root.head,
            end: root.head,
            text: format!("<{prefix}sheetPr filterMode=\"{}\"/>", u8::from(active)),
        });
    }
    let data = nodes
        .iter()
        .find(|node| node.local == "sheetData")
        .ok_or_else(|| WriteError::NoSheetData("worksheet".into()))?;
    // Rows inherit the worksheet namespace. Inspect the full XML so aliases resolve.
    let mut reader = NsReader::from_str(xml);
    let mut remaining = hidden.clone();
    let mut in_data = false;
    let mut depth = 0;
    let mut namespace: Option<Vec<u8>> = None;
    let mut insertion = data.close;
    let mut next_row = 0;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let end = reader.buffer_position() as usize;
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(element.name());
                let ns = match ns {
                    ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    ResolveResult::Unbound => Some(vec![]),
                    _ => None,
                };
                if start == data.start {
                    in_data = !empty;
                    namespace = ns;
                    depth = 1;
                } else if in_data {
                    if depth == 1 && ns == namespace && local.as_ref() == b"row" {
                        let row = if let Some(value) = attribute(element, b"r")? {
                            value
                                .parse::<u32>()
                                .ok()
                                .and_then(|row| row.checked_sub(1))
                                .ok_or_else(|| {
                                    WriteError::AutoFilter("row index is unavailable".into())
                                })?
                        } else {
                            next_row
                        };
                        next_row = row + 1;
                        let mut inserted = String::new();
                        while remaining
                            .first_key_value()
                            .is_some_and(|(index, _)| *index < row)
                        {
                            let (index, value) = remaining.pop_first().unwrap();
                            inserted.push_str(&format!(
                                "<{prefix}row r=\"{}\" hidden=\"{}\"/>",
                                index + 1,
                                u8::from(value)
                            ));
                        }
                        if !inserted.is_empty() {
                            splices.push(Splice {
                                start,
                                end: start,
                                text: inserted,
                            });
                        }
                        if let Some(value) = remaining.remove(&row) {
                            splices.push(Splice {
                                start,
                                end,
                                text: head_attribute(
                                    &xml[start..end],
                                    "hidden",
                                    if value { "1" } else { "0" },
                                )?,
                            });
                        }
                    }
                    depth += usize::from(!empty);
                }
            }
            Event::End(_) if in_data => {
                depth -= 1;
                if depth == 0 {
                    in_data = false;
                    insertion = start;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let inserted = remaining
        .into_iter()
        .map(|(row, value)| {
            format!(
                "<{prefix}row r=\"{}\" hidden=\"{}\"/>",
                row + 1,
                u8::from(value)
            )
        })
        .collect::<String>();
    if data.empty && !inserted.is_empty() {
        splices.push(Splice {
            start: data.start,
            end: data.end,
            text: format!(
                "{}>{inserted}</{}>",
                &xml[data.start..data.head - 2],
                data.name
            ),
        });
    } else if !inserted.is_empty() {
        splices.push(Splice {
            start: insertion,
            end: insertion,
            text: inserted,
        });
    }
    Ok(apply(xml, splices))
}

pub fn write_auto_filter(
    original: &[u8],
    workbook: &Workbook,
    sheet_name: &str,
    update: &AutoFilterWrite<'_>,
) -> Result<Vec<u8>> {
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    let mut package = OpcPackage::open(original)?;
    let sheet_xml = package
        .part_str(&sheet.part_name)?
        .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
    let sheet_xml = if let Some(name) = update.table {
        let table = sheet
            .tables
            .iter()
            .find(|table| table.name == name)
            .ok_or_else(|| WriteError::AutoFilter(format!("table {name} is unavailable")))?;
        let xml = package
            .part_str(&table.part_name)?
            .ok_or_else(|| WriteError::MissingPart(table.part_name.clone()))?;
        package.replace(
            &table.part_name,
            edit_parent(&xml, "table", update)?.into_bytes(),
        );
        sheet_xml
    } else {
        edit_parent(&sheet_xml, "worksheet", update)?
    };
    package.replace(
        &sheet.part_name,
        patch_visibility(&sheet_xml, update.hidden, update.filter_mode)?.into_bytes(),
    );
    Ok(package.write()?)
}

pub fn write_filter_visibility(
    original: &[u8],
    workbook: &Workbook,
    sheet_name: &str,
    hidden: &BTreeMap<u32, bool>,
    active: bool,
) -> Result<Vec<u8>> {
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    let mut package = OpcPackage::open(original)?;
    let xml = package
        .part_str(&sheet.part_name)?
        .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
    package.replace(
        &sheet.part_name,
        patch_visibility(&xml, hidden, active)?.into_bytes(),
    );
    Ok(package.write()?)
}

/// Protection flags describe prohibited operations, not granted permissions.
pub fn validate_filter_edit(
    original: &[u8],
    workbook: &Workbook,
    sheet_name: &str,
    toggling: bool,
) -> Result<()> {
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    let package = OpcPackage::open(original)?;
    let xml = package
        .part_str(&sheet.part_name)?
        .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
    let (_, nodes) = children(&xml, "worksheet")?;
    let flag = |node: &Node, name: &[u8], default: bool| -> Result<bool> {
        match node_attribute(&xml, node, name)?.as_deref() {
            None => Ok(default),
            Some("1" | "true") => Ok(true),
            Some("0" | "false") => Ok(false),
            _ => Err(WriteError::AutoFilter(
                "invalid sheet protection flag".into(),
            )),
        }
    };
    for node in nodes.iter().filter(|node| node.local == "sheetProtection") {
        if flag(node, b"sheet", false)? && (toggling || flag(node, b"autoFilter", true)?) {
            return Err(WriteError::AutoFilter(
                "worksheet protection prohibits this filter operation".into(),
            ));
        }
    }
    Ok(())
}

/// Remove one filter owner while preserving its table and unrelated XML.
pub fn remove_auto_filter(
    original: &[u8],
    workbook: &Workbook,
    sheet_name: &str,
    table: Option<&str>,
    hidden: &BTreeMap<u32, bool>,
    active: bool,
) -> Result<Vec<u8>> {
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    let mut package = OpcPackage::open(original)?;
    let sheet_xml = package
        .part_str(&sheet.part_name)?
        .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
    let remove = |xml: &str, root: &str| -> Result<String> {
        let (_, nodes) = children(xml, root)?;
        Ok(apply(
            xml,
            nodes
                .iter()
                .filter(|node| node.local == "autoFilter")
                .map(|node| Splice {
                    start: node.start,
                    end: node.end,
                    text: String::new(),
                })
                .collect(),
        ))
    };
    let sheet_xml = if let Some(name) = table {
        let table = sheet
            .tables
            .iter()
            .find(|table| table.name == name)
            .ok_or_else(|| WriteError::AutoFilter(format!("table {name} is unavailable")))?;
        let xml = package
            .part_str(&table.part_name)?
            .ok_or_else(|| WriteError::MissingPart(table.part_name.clone()))?;
        package.replace(&table.part_name, remove(&xml, "table")?.into_bytes());
        sheet_xml
    } else {
        remove(&sheet_xml, "worksheet")?
    };
    package.replace(
        &sheet.part_name,
        patch_visibility(&sheet_xml, hidden, active)?.into_bytes(),
    );
    Ok(package.write()?)
}

/// Refresh only owned top10 cache attributes, retaining all other XML bytes.
pub fn write_rank_thresholds(
    original: &[u8],
    workbook: &Workbook,
    sheet_name: &str,
    table: Option<&str>,
    thresholds: &BTreeMap<u32, f64>,
) -> Result<Vec<u8>> {
    if thresholds.is_empty() {
        return Ok(original.to_vec());
    }
    if thresholds.values().any(|value| !value.is_finite()) {
        return Err(WriteError::AutoFilter(
            "rank threshold must be finite".into(),
        ));
    }
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    let (part, root) = if let Some(name) = table {
        let table = sheet
            .tables
            .iter()
            .find(|table| table.name == name)
            .ok_or_else(|| WriteError::AutoFilter("table is unavailable".into()))?;
        (table.part_name.as_str(), "table")
    } else {
        (sheet.part_name.as_str(), "worksheet")
    };
    let mut package = OpcPackage::open(original)?;
    let xml = package
        .part_str(part)?
        .ok_or_else(|| WriteError::MissingPart(part.into()))?;
    let (_, nodes) = children(&xml, root)?;
    let filters: Vec<_> = nodes
        .iter()
        .filter(|node| node.local == "autoFilter")
        .collect();
    if filters.len() != 1 {
        return Err(WriteError::AutoFilter(
            "rank filter owner is unavailable".into(),
        ));
    }
    let filter = filters[0];
    let mut reader = NsReader::from_str(&xml);
    let mut namespace = None;
    let mut depth = 0usize;
    let mut column = None;
    let mut found = std::collections::BTreeSet::new();
    let mut splices = vec![];
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let head = reader.buffer_position() as usize;
                let empty = xml[..head].ends_with("/>");
                let (resolved, local) = reader.resolver().resolve_element(element.name());
                let ns = match resolved {
                    ResolveResult::Unbound => Some(vec![]),
                    ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    _ => None,
                };
                if depth == 0 {
                    namespace = ns.clone();
                }
                if ns == namespace && start >= filter.head && start < filter.close {
                    if depth == 2 && local.as_ref() == b"filterColumn" {
                        column = None;
                        for attr in element.attributes() {
                            let attr = attr?;
                            if attr.key.as_ref() == b"colId" {
                                column = attr.unescape_value()?.parse::<u32>().ok();
                            }
                        }
                    } else if depth == 3 && local.as_ref() == b"top10" {
                        if let Some((id, value)) =
                            column.and_then(|id| thresholds.get(&id).map(|value| (id, value)))
                        {
                            if !found.insert(id) {
                                return Err(WriteError::AutoFilter(
                                    "duplicate rank threshold target".into(),
                                ));
                            }
                            let before = &xml[start..head];
                            let after = head_attribute(before, "filterVal", &value.to_string())?;
                            if before != after {
                                splices.push(Splice {
                                    start,
                                    end: head,
                                    text: after,
                                });
                            }
                        }
                    }
                }
                if empty && depth == 2 {
                    column = None;
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 2 {
                    column = None;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if found.len() != thresholds.len() {
        return Err(WriteError::AutoFilter(
            "rank threshold target is unavailable".into(),
        ));
    }
    if splices.is_empty() {
        return Ok(original.to_vec());
    }
    package.replace(part, apply(&xml, splices).into_bytes());
    Ok(package.write()?)
}
