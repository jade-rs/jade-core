//! SpreadsheetML worksheet lock flags. A true XML flag forbids the operation.
use crate::{Result, SheetProtection, XlsxError};
use quick_xml::events::BytesStart;

/// Independent content, object and scenario protection. `Sheet::protection`
/// describes protected cells only; object/scenario locks can exist without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProtectionState {
    pub sheet: bool,
    pub allowed: SheetProtection,
}

/// Read the selected owned record using the worksheet reader's namespace/MCE
/// policy. Foreign and nested lookalikes do not supply operation permissions.
pub fn read(xml: &str) -> Result<Option<ProtectionState>> {
    use quick_xml::{events::Event, Reader};
    let grid = crate::cell_content::worksheet(xml)?;
    let mut reader = Reader::from_str(&grid);
    let mut depth = 0;
    let mut state = None;
    loop {
        match reader.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                if depth == 1 && jade_opc::local_name(e.name().as_ref()) == b"sheetProtection" {
                    if state.is_some() {
                        return Err(XlsxError::BadSheetProtection("multiple records".into()));
                    }
                    let (sheet, allowed) = flags(&e)?;
                    state = Some(ProtectionState { sheet, allowed });
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth -= 1,
            Event::Eof => return Ok(state),
            _ => {}
        }
    }
}

/// Parse unqualified attributes of an already identified, owned sheetProtection.
/// Returns the sheet lock and the operations that remain allowed.
pub fn flags(e: &BytesStart<'_>) -> Result<(bool, SheetProtection)> {
    let mut values = std::collections::HashMap::new();
    for a in e.attributes() {
        let a = a?;
        if [
            b"sheet".as_slice(),
            b"objects",
            b"scenarios",
            b"formatCells",
            b"formatColumns",
            b"formatRows",
            b"insertColumns",
            b"insertRows",
            b"insertHyperlinks",
            b"deleteColumns",
            b"deleteRows",
            b"sort",
            b"autoFilter",
            b"pivotTables",
            b"selectLockedCells",
            b"selectUnlockedCells",
        ]
        .contains(&a.key.as_ref())
        {
            let value = a.unescape_value()?;
            let flag = match value.trim_matches([' ', '\t', '\r', '\n']) {
                "true" | "1" => true,
                "false" | "0" => false,
                _ => {
                    return Err(XlsxError::BadSheetProtection(format!(
                        "invalid {} boolean",
                        String::from_utf8_lossy(a.key.as_ref())
                    )))
                }
            };
            values.insert(a.key.as_ref().to_vec(), flag);
        }
    }
    let flag = |key: &[u8], default| values.get(key).copied().unwrap_or(default);
    Ok((
        flag(b"sheet", false),
        SheetProtection {
            format_cells: !flag(b"formatCells", true),
            format_columns: !flag(b"formatColumns", true),
            format_rows: !flag(b"formatRows", true),
            insert_columns: !flag(b"insertColumns", true),
            insert_rows: !flag(b"insertRows", true),
            insert_hyperlinks: !flag(b"insertHyperlinks", true),
            delete_columns: !flag(b"deleteColumns", true),
            delete_rows: !flag(b"deleteRows", true),
            sort: !flag(b"sort", true),
            auto_filter: !flag(b"autoFilter", true),
            objects: !flag(b"objects", false),
            scenarios: !flag(b"scenarios", false),
            pivot_tables: !flag(b"pivotTables", true),
            select_locked_cells: !flag(b"selectLockedCells", false),
            select_unlocked_cells: !flag(b"selectUnlockedCells", false),
        },
    ))
}

fn boolean(e: &BytesStart<'_>, key: &[u8], default: bool) -> Result<bool> {
    for a in e.attributes() {
        let a = a?;
        if a.key.as_ref() == key {
            return match a.unescape_value()?.trim_matches([' ', '\t', '\r', '\n']) {
                "1" | "true" => Ok(true),
                "0" | "false" => Ok(false),
                _ => Err(XlsxError::BadSheetProtection(
                    "invalid cell protection boolean".into(),
                )),
            };
        }
    }
    Ok(default)
}
/// Resolve only direct xf/protection children; foreign extensions cannot supply flags.
/// Office's cellXf applyProtection controls later named-style updates; it does
/// not erase an explicitly stored protection record on import.
pub(crate) fn style_flags(xml: &str, table: &mut crate::StyleTable) -> Result<()> {
    use quick_xml::{events::Event, name::ResolveResult, NsReader};
    #[derive(Default)]
    struct Entry {
        cell: bool,
        parent: Option<usize>,
        enabled: bool,
        own: Option<(bool, bool)>,
    }
    let mut reader = NsReader::from_str(xml);
    let mut stack: Vec<(Vec<u8>, bool)> = Vec::new();
    let mut namespace = None;
    let mut current: Option<Entry> = None;
    let mut entries: [Vec<Entry>; 2] = [Vec::new(), Vec::new()];
    loop {
        match reader.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let local = jade_opc::local_name(e.name().as_ref()).to_vec();
                let ns = match reader.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => Some(Vec::new()),
                    ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    _ => None,
                };
                if stack.is_empty() {
                    namespace = ns.clone().filter(|n| {
                        n.is_empty()
                            || matches!(
                                n.as_slice(),
                                b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                    | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                            )
                    });
                }
                let owned = namespace.is_some() && ns == namespace;
                if stack.len() == 2
                    && owned
                    && local == b"xf"
                    && stack[0] == (b"styleSheet".to_vec(), true)
                    && stack[1].1
                    && matches!(stack[1].0.as_slice(), b"cellXfs" | b"cellStyleXfs")
                {
                    let mut parent = None;
                    for a in e.attributes() {
                        let a = a?;
                        if a.key.as_ref() == b"xfId" {
                            parent = Some(a.unescape_value()?.parse().map_err(|_| {
                                XlsxError::BadSheetProtection("invalid xfId".into())
                            })?);
                        }
                    }
                    let item = Entry {
                        cell: stack[1].0 == b"cellXfs",
                        parent,
                        enabled: boolean(&e, b"applyProtection", true)?,
                        own: None,
                    };
                    if empty {
                        entries[item.cell as usize].push(item);
                    } else {
                        current = Some(item);
                    }
                } else if stack.len() == 3 && owned && local == b"protection" {
                    if let Some(entry) = &mut current {
                        if entry.own.is_some() {
                            return Err(XlsxError::BadSheetProtection(
                                "multiple cell protection records".into(),
                            ));
                        }
                        entry.own = Some((
                            !boolean(&e, b"locked", true)?,
                            boolean(&e, b"hidden", false)?,
                        ));
                    }
                }
                if !empty {
                    stack.push((local, owned));
                }
            }
            Event::End(_) => {
                if stack.len() == 3 {
                    if let Some(item) = current.take() {
                        entries[item.cell as usize].push(item);
                    }
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let named: Vec<_> = entries[0]
        .iter()
        .map(|entry| {
            if entry.enabled {
                entry.own.unwrap_or_default()
            } else {
                (false, false)
            }
        })
        .collect();
    for (format, flags) in table.style_formats.iter_mut().zip(&named) {
        (format.unlocked, format.formula_hidden) = *flags;
    }
    for (format, entry) in table.cell_formats.iter_mut().zip(&entries[1]) {
        let flags = entry
            .own
            .or_else(|| entry.parent.and_then(|i| named.get(i).copied()))
            .unwrap_or_default();
        (format.unlocked, format.formula_hidden) = flags;
    }
    Ok(())
}
