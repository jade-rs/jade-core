//! Persist verified row-hide provenance in a namespaced SpreadsheetML extension.
use crate::{
    auto_filter::children,
    patch::{apply, attribute, namespace_prefix, Splice},
    Result, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    filter_state::{signature, NAMESPACE, URI},
    Workbook,
};
use quick_xml::{
    events::Event,
    name::{QName, ResolveResult},
    NsReader,
};
use std::collections::{BTreeSet, HashSet};

pub fn write_filter_row_state(
    original: &[u8],
    workbook: &Workbook,
    sheet_name: &str,
    manual: &HashSet<u32>,
    filtered: &HashSet<u32>,
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
    let (root, nodes) = children(&xml, "worksheet")?;
    let list = nodes.iter().find(|node| node.local == "extLst");
    let mut splices = vec![];
    if let Some(list) = list {
        let mut reader = NsReader::from_str(&xml);
        let mut depth = 0;
        let mut root_ns = None;
        loop {
            let start = reader.buffer_position() as usize;
            match reader.read_event()? {
                Event::Start(ref element) | Event::Empty(ref element) => {
                    let empty = xml[..reader.buffer_position() as usize].ends_with("/>");
                    let (ns, local) = reader.resolver().resolve_element(element.name());
                    let ns = match ns {
                        ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                        ResolveResult::Unbound => Some(vec![]),
                        _ => None,
                    };
                    if start == root.start {
                        root_ns = ns.clone();
                    }
                    if depth == 2
                        && start >= list.head
                        && start < list.close
                        && ns == root_ns
                        && local.as_ref() == b"ext"
                        && attribute(element, b"uri")?.as_deref() == Some(URI)
                    {
                        let name = element.name().as_ref().to_vec();
                        if !empty {
                            reader.read_to_end(QName(&name))?;
                        }
                        splices.push(Splice {
                            start,
                            end: reader.buffer_position() as usize,
                            text: String::new(),
                        });
                        continue;
                    }
                    depth += usize::from(!empty);
                }
                Event::End(_) => depth = depth.saturating_sub(1),
                Event::Eof => break,
                _ => {}
            }
        }
    }
    let indices: BTreeSet<_> = manual.union(filtered).copied().collect();
    let ranges = jade_xlsx_reader::filter_state::body_ranges(sheet);
    let valid_indices: Vec<_> = indices
        .into_iter()
        .filter(|index| {
            ranges
                .iter()
                .any(|(first, end)| index >= first && index <= end)
        })
        .filter(|index| {
            sheet
                .rows
                .binary_search_by_key(index, |row| row.index)
                .ok()
                .map(|at| sheet.rows[at].hidden)
                .unwrap_or(sheet.default_rows_hidden)
        })
        .collect();
    let stored_manual = valid_indices
        .iter()
        .filter(|index| manual.contains(index))
        .copied()
        .collect();
    let stored_filtered = valid_indices
        .iter()
        .filter(|index| filtered.contains(index))
        .copied()
        .collect();
    let records = valid_indices
        .into_iter()
        .map(|index| {
            format!(
                "<jf:record index=\"{index}\" manual=\"{}\" filtered=\"{}\"/>",
                u8::from(manual.contains(&index)),
                u8::from(filtered.contains(&index))
            )
        })
        .collect::<String>();
    let content = if manual.is_empty() {
        String::new()
    } else {
        let prefix = namespace_prefix(
            list.map(|list| list.name.as_bytes())
                .unwrap_or(root.name.as_bytes()),
        );
        format!("<{prefix}ext uri=\"{URI}\"><jf:filterRows xmlns:jf=\"{NAMESPACE}\" version=\"1\" signature=\"{}\">{records}</jf:filterRows></{prefix}ext>",signature(sheet,&stored_manual,&stored_filtered))
    };
    if let Some(list) = list {
        if list.empty {
            if !content.is_empty() {
                splices.push(Splice {
                    start: list.start,
                    end: list.end,
                    text: format!(
                        "{}>{content}</{}>",
                        &xml[list.start..list.head - 2],
                        list.name
                    ),
                });
            }
        } else if !content.is_empty() {
            splices.push(Splice {
                start: list.close,
                end: list.close,
                text: content,
            });
        } else if !splices.is_empty() {
            let inner = &xml[list.head..list.close];
            let local_splices = splices
                .iter()
                .map(|splice| Splice {
                    start: splice.start - list.head,
                    end: splice.end - list.head,
                    text: String::new(),
                })
                .collect();
            if apply(inner, local_splices).trim().is_empty() {
                splices.clear();
                splices.push(Splice {
                    start: list.start,
                    end: list.end,
                    text: String::new(),
                });
            }
        }
    } else if !content.is_empty() {
        let prefix = namespace_prefix(root.name.as_bytes());
        splices.push(Splice {
            start: root.close,
            end: root.close,
            text: format!("<{prefix}extLst>{content}</{prefix}extLst>"),
        });
    }
    if splices.is_empty() {
        return Ok(original.into());
    }
    package.replace(&sheet.part_name, apply(&xml, splices).into_bytes());
    Ok(package.write()?)
}
