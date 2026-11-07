//! Append dynamic-array metadata while keeping all existing indexes stable.
use crate::{
    patch::{apply, escape, Splice},
    Result,
};
use jade_opc::OpcPackage;
use quick_xml::{events::Event, name::ResolveResult, NsReader, Reader};
mod conditional;

#[derive(Default)]
struct Node {
    namespace: String,
    name: String,
    qualified: String,
    attributes: Vec<(String, String)>,
    children: Vec<usize>,
    start: usize,
    head_end: usize,
    close: usize,
    empty: bool,
    end: usize,
    parent: usize,
}
impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    fn index(&self, key: &str) -> Option<usize> {
        self.attr(key)?
            .trim_matches([' ', '\t', '\r', '\n'])
            .parse()
            .ok()
    }
}
fn nodes(xml: &str) -> Result<Vec<Node>> {
    let mut reader = NsReader::from_str(xml);
    let mut nodes = vec![Node::default()];
    let mut stack = vec![0];
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let head_end = reader.buffer_position() as usize;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let namespace = match reader.resolver().resolve_element(element.name()).0 {
                    ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).into_owned(),
                    ResolveResult::Unbound => String::new(),
                    ResolveResult::Unknown(_) => "!unresolved".into(),
                };
                let mut attributes = Vec::new();
                for attr in element.attributes() {
                    let attr = attr?;
                    if !attr.key.as_ref().contains(&b':') {
                        attributes.push((
                            String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                            attr.decode_and_unescape_value(reader.decoder())?
                                .into_owned(),
                        ));
                    }
                }
                let index = nodes.len();
                nodes.push(Node {
                    namespace,
                    name: String::from_utf8_lossy(element.local_name().as_ref()).into_owned(),
                    qualified: String::from_utf8_lossy(element.name().as_ref()).into_owned(),
                    attributes,
                    children: Vec::new(),
                    start,
                    head_end,
                    close: head_end,
                    empty,
                    end: head_end,
                    parent: *stack.last().unwrap(),
                });
                nodes[*stack.last().unwrap()].children.push(index);
                if !empty {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                if let Some(index) = stack.pop().filter(|&i| i != 0) {
                    nodes[index].close = start;
                    nodes[index].end = head_end;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(nodes)
}
fn owned(nodes: &[Node], parent: usize, name: &str) -> Vec<usize> {
    nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|&i| nodes[i].namespace == nodes[parent].namespace && nodes[i].name == name)
        .collect()
}
fn valid_namespace(ns: &str) -> bool {
    matches!(
        ns,
        "" | "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
            | "http://purl.oclc.org/ooxml/spreadsheetml/main"
    )
}

/// Detect index-bearing compatibility markup requiring a branch-aware append.
/// Extension payloads and foreign lookalikes do not participate in these indexes.
pub(crate) fn conditional_indexes(xml: &str) -> Result<bool> {
    let tree = nodes(xml)?;
    let mut pending = tree[0].children.clone();
    while let Some(parent) = pending.pop() {
        let node = &tree[parent];
        if !valid_namespace(&node.namespace) {
            continue;
        }
        for &child in &node.children {
            let next = &tree[child];
            if next.namespace == "http://schemas.openxmlformats.org/markup-compatibility/2006"
                && next.name == "AlternateContent"
            {
                return Ok(true);
            }
            if next.namespace == node.namespace
                && matches!(
                    (node.name.as_str(), next.name.as_str()),
                    (
                        "metadata",
                        "metadataTypes" | "cellMetadata" | "valueMetadata"
                    ) | ("metadataTypes", "metadataType")
                        | ("cellMetadata" | "valueMetadata", "bk")
                        | ("bk", "rc" | "extLst")
                        | ("futureMetadata", "bk")
                )
                || node.name == "metadata"
                    && next.namespace == node.namespace
                    && next.name == "futureMetadata"
                    && next.attr("name") == Some("XLDAPR")
                || node.name == "extLst"
                    && next.namespace == node.namespace
                    && next.name == "ext"
                    && next.attr("uri").is_some_and(|uri| {
                        uri.eq_ignore_ascii_case("{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}")
                    })
            {
                pending.push(child);
            }
        }
    }
    Ok(false)
}

/// Check whether a new metadata part can safely use its first cell record.
pub(crate) fn unused_cell_metadata_index(
    package: &OpcPackage,
    workbook: &str,
) -> Result<Option<u32>> {
    let mut used = std::collections::BTreeSet::new();
    if let Some(rels) = package.part_str(&jade_opc::rels_part_name(workbook))? {
        for rel in jade_opc::parse_relationships(&rels)?.by_type_suffix("/worksheet") {
            if rel.external {
                continue;
            }
            let part = jade_opc::resolve_target(workbook, &rel.target);
            let Some(xml) = package.part_str(&part)? else {
                return Ok(None);
            };
            let Some(links) = worksheet_metadata_links(&xml)? else {
                return Ok(None);
            };
            for link in links {
                if let Some(index) = link.and_then(|n| u32::try_from(n).ok()) {
                    used.insert(index);
                }
            }
        }
    }
    Ok((!used.contains(&1)).then_some(1))
}

// Metadata indexes are global to the package, so standard cell links in
// opaque alternatives must remain reserved even when their values cannot
// be projected. Only MCE wrappers are transparent to the standard grid path.
fn grid_children(tree: &[Node], parent: usize, namespace: &str, name: &str) -> Vec<usize> {
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    let mut result = Vec::new();
    let mut pending = tree[parent].children.clone();
    while let Some(index) = pending.pop() {
        let node = &tree[index];
        if node.namespace == namespace && node.name == name {
            result.push(index);
        } else if node.namespace == MC && node.name == "AlternateContent" {
            for &branch in &node.children {
                if tree[branch].namespace == MC
                    && matches!(tree[branch].name.as_str(), "Choice" | "Fallback")
                {
                    pending.extend_from_slice(&tree[branch].children);
                }
            }
        }
    }
    result
}

fn worksheet_metadata_links(xml: &str) -> Result<Option<Vec<Option<usize>>>> {
    let tree = nodes(xml)?;
    if tree[0].children.len() != 1 {
        return Ok(None);
    }
    let root = tree[0].children[0];
    if tree[root].name != "worksheet" || !valid_namespace(&tree[root].namespace) {
        return Ok(None);
    }
    let mut links = Vec::new();
    let namespace = &tree[root].namespace;
    for data in grid_children(&tree, root, namespace, "sheetData") {
        for row in grid_children(&tree, data, namespace, "row") {
            for cell in grid_children(&tree, row, namespace, "c") {
                if tree[cell].attr("cm").is_some() {
                    links.push(tree[cell].index("cm"));
                }
            }
        }
    }
    Ok(Some(links))
}
fn valid_count(node: &Node, actual: usize) -> bool {
    node.attr("count").is_none() || node.index("count") == Some(actual)
}
fn append(
    xml: &str,
    node: &Node,
    count: usize,
    content: &str,
    splices: &mut Vec<Splice>,
) -> Result<()> {
    let mut reader = Reader::from_str(&xml[node.start..node.head_end]);
    if let Event::Start(head) | Event::Empty(head) = reader.read_event()? {
        let head = crate::shift::rewrite(&head, &[("count", Some(count.to_string()))], false)?;
        splices.push(Splice {
            start: node.start,
            end: node.head_end,
            text: if node.empty {
                format!("{head}{content}</{}>", node.qualified)
            } else {
                head
            },
        });
        if !node.empty {
            splices.push(Splice {
                start: node.close,
                end: node.close,
                text: content.into(),
            });
        }
    }
    Ok(())
}

/// Unsupported ambiguous or dangling metadata is left unchanged. Adding a
/// record must not make an old invalid t/v/cm link acquire a new meaning.
pub(crate) fn append_dynamic(
    xml: &str,
    package: &OpcPackage,
    workbook: &str,
) -> Result<Option<(String, u32)>> {
    if conditional_indexes(xml)? {
        return conditional::append_dynamic(xml, package, workbook);
    }
    let tree = nodes(xml)?;
    if tree[0].children.len() != 1 {
        return Ok(None);
    }
    let root = tree[0].children[0];
    let ns = &tree[root].namespace;
    if tree[root].name != "metadata" || !valid_namespace(ns) {
        return Ok(None);
    }
    if tree[root].empty {
        let mut reader = Reader::from_str(&xml[tree[root].start..tree[root].head_end]);
        if let Event::Empty(head) = reader.read_event()? {
            let head = crate::shift::rewrite(&head, &[], false)?;
            let expanded = apply(
                xml,
                vec![Splice {
                    start: tree[root].start,
                    end: tree[root].head_end,
                    text: format!("{head}</{}>", tree[root].qualified),
                }],
            );
            return append_dynamic(&expanded, package, workbook);
        }
    }
    let types = owned(&tree, root, "metadataTypes");
    let cells = owned(&tree, root, "cellMetadata");
    let futures = owned(&tree, root, "futureMetadata")
        .into_iter()
        .filter(|&i| tree[i].attr("name") == Some("XLDAPR"))
        .collect::<Vec<_>>();
    if types.len() > 1 || cells.len() > 1 || futures.len() > 1 {
        return Ok(None);
    }
    let entries = types
        .first()
        .map(|&i| owned(&tree, i, "metadataType"))
        .unwrap_or_default();
    let blocks = cells
        .first()
        .map(|&i| owned(&tree, i, "bk"))
        .unwrap_or_default();
    let records = futures
        .first()
        .map(|&i| owned(&tree, i, "bk"))
        .unwrap_or_default();
    for (container, count) in [
        (types.first(), entries.len()),
        (cells.first(), blocks.len()),
        (futures.first(), records.len()),
    ] {
        if container.is_some_and(|&i| !valid_count(&tree[i], count)) {
            return Ok(None);
        }
    }
    let dynamic = entries
        .iter()
        .enumerate()
        .filter(|(_, i)| tree[**i].attr("name") == Some("XLDAPR"))
        .map(|(i, _)| i + 1)
        .collect::<Vec<_>>();
    if dynamic.len() > 1 {
        return Ok(None);
    }
    let type_index = dynamic.first().copied().unwrap_or(entries.len() + 1);
    // Both cell and value metadata may contain references to this type.
    for container in ["cellMetadata", "valueMetadata"] {
        for parent in owned(&tree, root, container) {
            for block in owned(&tree, parent, "bk") {
                for record in owned(&tree, block, "rc") {
                    let Some(t) = tree[record].index("t") else {
                        return Ok(None);
                    };
                    if t == 0
                        || t > entries.len()
                        || t == type_index
                            && tree[record].index("v").is_none_or(|v| v >= records.len())
                    {
                        return Ok(None);
                    }
                }
            }
        }
    }
    let Some(rels) = package.part_str(&jade_opc::rels_part_name(workbook))? else {
        return Ok(None);
    };
    for rel in jade_opc::parse_relationships(&rels)?.by_type_suffix("/worksheet") {
        if rel.external {
            continue;
        }
        let part = jade_opc::resolve_target(workbook, &rel.target);
        let Some(sheet) = package.part_str(&part)? else {
            return Ok(None);
        };
        let Some(links) = worksheet_metadata_links(&sheet)? else {
            return Ok(None);
        };
        if links
            .into_iter()
            .any(|index| index.is_none_or(|v| v == 0 || v > blocks.len()))
        {
            return Ok(None);
        }
    }
    let Some(cm) = blocks
        .len()
        .checked_add(1)
        .and_then(|i| u32::try_from(i).ok())
    else {
        return Ok(None);
    };
    let xmlns = format!(" xmlns=\"{}\"", escape(ns));
    let record = format!("<bk{xmlns}><extLst><ext uri=\"{{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}}\"><dynamicArrayProperties xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\" fDynamic=\"1\" fCollapsed=\"0\"/></ext></extLst></bk>");
    let cell = format!(
        "<bk{xmlns}><rc t=\"{type_index}\" v=\"{}\"/></bk>",
        records.len()
    );
    let metadata_type = format!("<metadataType{xmlns} name=\"XLDAPR\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\" cellMeta=\"1\"/>");
    let mut splices = Vec::new();
    let mut inserts = std::collections::BTreeMap::<usize, String>::new();
    if dynamic.is_empty() {
        if let Some(&i) = types.first() {
            append(
                xml,
                &tree[i],
                entries.len() + 1,
                &metadata_type,
                &mut splices,
            )?;
        } else {
            inserts
                .entry(tree[root].head_end)
                .or_default()
                .push_str(&format!(
                    "<metadataTypes{xmlns} count=\"1\">{metadata_type}</metadataTypes>"
                ));
        }
    }
    let before = |names: &[&str]| {
        tree[root]
            .children
            .iter()
            .copied()
            .find(|&i| tree[i].namespace == *ns && names.contains(&tree[i].name.as_str()))
            .map_or(tree[root].close, |i| tree[i].start)
    };
    if let Some(&i) = futures.first() {
        append(xml, &tree[i], records.len() + 1, &record, &mut splices)?;
    } else {
        inserts
            .entry(before(&["cellMetadata", "valueMetadata", "extLst"]))
            .or_default()
            .push_str(&format!(
                "<futureMetadata{xmlns} name=\"XLDAPR\" count=\"1\">{record}</futureMetadata>"
            ));
    }
    if let Some(&i) = cells.first() {
        append(xml, &tree[i], blocks.len() + 1, &cell, &mut splices)?;
    } else {
        inserts
            .entry(before(&["valueMetadata", "extLst"]))
            .or_default()
            .push_str(&format!(
                "<cellMetadata{xmlns} count=\"1\">{cell}</cellMetadata>"
            ));
    }
    splices.extend(inserts.into_iter().map(|(start, text)| Splice {
        start,
        end: start,
        text,
    }));
    let updated = apply(xml, splices);
    let old_flags = jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(xml)?;
    let new_flags = jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&updated)?;
    if new_flags.len() != blocks.len() + 1
        || new_flags.last() != Some(&Some(true))
        || (0..blocks.len()).any(|i| new_flags[i] != old_flags.get(i).copied().flatten())
    {
        return Ok(None);
    }
    Ok(Some((updated, cm)))
}
