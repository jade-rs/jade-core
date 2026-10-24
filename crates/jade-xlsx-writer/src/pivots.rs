//! Follow owned pivot report/source geometry while preserving cache snapshots.
use crate::{
    patch::{apply, Splice},
    shift::{range_text, rewrite},
    Result, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{formula::same_sheet_name, CellRange, CellShift, StructureChange, Workbook};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
fn bad(reason: &str) -> WriteError {
    WriteError::Pivot(reason.into())
}
/// Edit exactly one direct owned path. Same-named foreign descendants and
/// inherited namespace aliases retain their original bytes.
fn attributes(xml: &str, path: &[&str], updates: &[(&str, Option<String>)]) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    let mut stack: Vec<(String, bool)> = Vec::new();
    let mut root_ns = None;
    let mut splices = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let ns = match reader.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => Some(Vec::new()),
                    ResolveResult::Bound(n) => Some(n.as_ref().to_vec()),
                    _ => None,
                };
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if stack.is_empty() {
                    if name != path[0]
                        || !ns.as_ref().is_some_and(|n| {
                            matches!(
                                n.as_slice(),
                                b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                    | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                            )
                        })
                    {
                        return Err(bad("unrecognized pivot part root"));
                    }
                    root_ns = ns.clone();
                }
                let own = ns.is_some() && ns == root_ns && stack.last().is_none_or(|p| p.1);
                if own
                    && stack.len() + 1 == path.len()
                    && name == path[path.len() - 1]
                    && stack
                        .iter()
                        .zip(path)
                        .all(|((name, owned), expected)| *owned && name == expected)
                {
                    splices.push(Splice {
                        start,
                        end: reader.buffer_position() as usize,
                        text: rewrite(&e, updates, empty)?,
                    });
                }
                if !empty {
                    if stack.len() > 128 {
                        return Err(bad("pivot metadata depth limit"));
                    }
                    stack.push((name, own));
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if splices.len() != 1 {
        return Err(bad("pivot metadata has no unique owned element"));
    }
    Ok(apply(xml, splices))
}
pub(crate) fn require_source_context(package: &OpcPackage, workbook: &Workbook) -> Result<()> {
    let owner = crate::workbook_part_name(package)?;
    if workbook.pivots.issues.iter().any(|i| i.part_name == owner) {
        // An omitted cache may point at any sheet. Moving/renaming only known
        // sources would silently leave that cache's original source stale.
        return Err(bad("cannot reconcile uninterpreted pivot cache sources"));
    }
    Ok(())
}
fn same_size(a: CellRange, b: CellRange) -> bool {
    a.end.row - a.start.row == b.end.row - b.start.row
        && a.end.column - a.start.column == b.end.column - b.start.column
}
fn follow(
    package: &mut OpcPackage,
    workbook: &Workbook,
    sheet: &str,
    affects: impl Fn(CellRange) -> bool,
    covers: impl Fn(CellRange) -> bool,
    map: impl Fn(CellRange) -> Option<CellRange>,
) -> Result<()> {
    require_source_context(package, workbook)?;
    let owner = workbook
        .sheets
        .iter()
        .find(|s| same_sheet_name(&s.name, sheet))
        .ok_or_else(|| WriteError::NoSuchSheet(sheet.into()))?;
    if workbook
        .pivots
        .issues
        .iter()
        .any(|i| i.part_name == owner.part_name)
    {
        return Err(bad(
            "cannot move a sheet with uninterpreted pivot declarations",
        ));
    }
    for table in workbook
        .pivots
        .tables
        .iter()
        .filter(|t| t.sheet_part == owner.part_name)
    {
        let before = table.location.reference;
        if !affects(before) {
            continue;
        }
        if !covers(before) {
            return Err(bad("the change would split a pivot report"));
        }
        let after = map(before)
            .filter(|after| same_size(before, *after))
            .ok_or_else(|| bad("the change would resize or remove a saved pivot report"))?;
        if before != after {
            let xml = package.required_part_str(&table.part_name)?;
            let updated = attributes(
                &xml,
                &["pivotTableDefinition", "location"],
                &[("ref", Some(range_text(after)))],
            )?;
            package.replace(&table.part_name, updated.into_bytes());
        }
    }
    for cache in &workbook.pivots.caches {
        let source = &cache.source;
        if source.kind != "worksheet"
            || source.relationship_id.is_some()
            || !source
                .sheet
                .as_deref()
                .is_some_and(|s| same_sheet_name(s, sheet))
        {
            continue;
        }
        let Some(before) = source.reference else {
            continue;
        };
        if !affects(before) {
            continue;
        }
        if !covers(before) {
            return Err(bad("the change would split a pivot source range"));
        }
        let after =
            map(before).ok_or_else(|| bad("the change would remove the entire pivot source"))?;
        if before != after {
            let xml = package.required_part_str(&cache.part_name)?;
            let updated = attributes(
                &xml,
                &["pivotCacheDefinition", "cacheSource", "worksheetSource"],
                &[("ref", Some(range_text(after)))],
            )?;
            let updated =
                crate::xml_edit::root_attributes(&updated, &[("refreshOnLoad", Some("1".into()))])?;
            package.replace(&cache.part_name, updated.into_bytes());
        }
    }
    Ok(())
}
pub(crate) fn after_structure(
    package: &mut OpcPackage,
    workbook: &Workbook,
    change: &StructureChange,
) -> Result<()> {
    follow(
        package,
        workbook,
        &change.sheet,
        |_| true,
        |_| true,
        |r| change.map_range(r),
    )
}
pub(crate) fn after_cells(
    package: &mut OpcPackage,
    workbook: &Workbook,
    change: &CellShift,
) -> Result<()> {
    follow(
        package,
        workbook,
        &change.sheet,
        |r| change.affects(r),
        |r| change.covers(r),
        |r| change.map_reference(r),
    )
}
pub(crate) fn rename_sources(
    package: &mut OpcPackage,
    workbook: &Workbook,
    old: &str,
    new: &str,
) -> Result<()> {
    require_source_context(package, workbook)?;
    for cache in &workbook.pivots.caches {
        if cache.source.kind == "worksheet"
            && cache.source.relationship_id.is_none()
            && cache
                .source
                .sheet
                .as_deref()
                .is_some_and(|s| same_sheet_name(s, old))
        {
            let xml = package.required_part_str(&cache.part_name)?;
            let updated = attributes(
                &xml,
                &["pivotCacheDefinition", "cacheSource", "worksheetSource"],
                &[(
                    "sheet",
                    Some(jade_xlsx_reader::xstring::encode_xstring(new)),
                )],
            )?;
            package.replace(&cache.part_name, updated.into_bytes());
        }
    }
    Ok(())
}

fn intersects(a: CellRange, b: CellRange) -> bool {
    a.start.row <= b.end.row
        && a.end.row >= b.start.row
        && a.start.column <= b.end.column
        && a.end.column >= b.start.column
}
/// User value edits cannot replace part of a saved pivot report. Formatting
/// and calculation-cache updates use their separate paths. Label/filter
/// authoring requires a corresponding pivot metadata edit.
pub fn guard_values(workbook: &Workbook, sheet_part: &str, range: CellRange) -> Result<()> {
    if workbook
        .pivots
        .tables
        .iter()
        .any(|t| t.sheet_part == sheet_part && intersects(t.location.reference, range))
    {
        return Err(bad("cannot change cells inside a saved pivot report"));
    }
    Ok(())
}
pub(crate) fn guard_cut_source(
    package: &OpcPackage,
    workbook: &Workbook,
    sheet: &jade_xlsx_reader::Sheet,
    range: CellRange,
) -> Result<()> {
    require_source_context(package, workbook)?;
    guard_values(workbook, &sheet.part_name, range)?;
    if workbook.pivots.caches.iter().any(|c| {
        c.source.relationship_id.is_none()
            && c.source
                .sheet
                .as_deref()
                .is_some_and(|s| same_sheet_name(s, &sheet.name))
            && c.source.reference.is_some_and(|r| intersects(r, range))
    }) {
        return Err(bad(
            "moving a pivot source by cut requires source-range reconciliation",
        ));
    }
    Ok(())
}
