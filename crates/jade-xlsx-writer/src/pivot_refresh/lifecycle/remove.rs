use super::{
    super::{bad, xml, Result},
    package,
};
use crate::{xml_edit_ns, CellEdit, NewValue};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{pivot::PivotTable, CellRange, Workbook};
use std::collections::BTreeSet;
#[derive(Debug, Clone)]
pub struct PivotDeleteResult {
    pub bytes: Vec<u8>,
    pub removed_cache: bool,
    pub detached_charts: usize,
}
fn slicer_dependencies(package: &OpcPackage, table: &PivotTable) -> Result<bool> {
    let types = package.required_part_str("[Content_Types].xml")?;
    package::root(&types, "Types", package::TYPES)?;
    let mut uses_cache = false;
    for (_, _, entry) in xml_edit_ns::children(&types, b"Override")? {
        let Some(kind) = xml::attr(&entry.text, "ContentType")? else {
            continue;
        };
        if !kind.contains(".slicerCache+") && !kind.contains(".timelineCache+") {
            continue;
        }
        let part = xml::attr(&entry.text, "PartName")?
            .ok_or_else(|| bad("missing slicer part identity"))?;
        let body = package.required_part_str(part.trim_start_matches('/'))?;
        let mut r = quick_xml::NsReader::from_str(&body);
        loop {
            match r.read_event()? {
                quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                    let own = matches!(r.resolver().resolve_element(e.name()).0,quick_xml::name::ResolveResult::Bound(n) if n.as_ref()==b"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"||n.as_ref()==b"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main");
                    if !own {
                        continue;
                    }
                    for attr in e.attributes() {
                        let a = attr?;
                        let value = a.unescape_value()?;
                        if a.key.as_ref() == b"name"
                            && e.local_name().as_ref() == b"pivotTable"
                            && value.eq_ignore_ascii_case(&table.name)
                        {
                            return Err(bad("removing a pivot bound to slicers/timelines requires their own reconciliation"));
                        }
                        if a.key.as_ref() == b"pivotCacheId"
                            && value.trim().parse::<u32>().ok() == Some(table.cache_id)
                        {
                            uses_cache = true;
                        }
                    }
                }
                quick_xml::events::Event::Eof => break,
                _ => {}
            }
        }
    }
    Ok(uses_cache)
}
fn report_footprint(book: &Workbook, table: &PivotTable) -> Result<Vec<CellRange>> {
    let mut regions = vec![table.location.reference];
    regions.extend(
        table
            .page_cells()
            .ok_or_else(|| bad("ambiguous removed page geometry"))?
            .into_iter()
            .flat_map(|(a, b)| [a, b])
            .map(|at| CellRange { start: at, end: at }),
    );
    let owner = book
        .sheets
        .iter()
        .find(|s| s.part_name == table.sheet_part)
        .ok_or_else(|| bad("missing report owner"))?;
    if owner.protection.as_ref().is_some_and(|p| !p.pivot_tables) {
        return Err(bad("removing pivots is not allowed on the protected sheet"));
    }
    for other in book
        .pivots
        .tables
        .iter()
        .filter(|p| p.part_name != table.part_name && p.sheet_part == table.sheet_part)
    {
        let mut footprint = vec![other.location.reference];
        footprint.extend(
            other
                .page_cells()
                .ok_or_else(|| bad("ambiguous neighboring report geometry"))?
                .into_iter()
                .flat_map(|(a, b)| [a, b])
                .map(|at| CellRange { start: at, end: at }),
        );
        if footprint
            .iter()
            .any(|r| regions.iter().any(|p| crate::tables::overlaps(*r, *p)))
        {
            return Err(bad("removed report overlaps another pivot"));
        }
    }
    if owner.tables.iter().any(|t| {
        regions
            .iter()
            .any(|r| crate::tables::overlaps(t.reference, *r))
    }) || owner
        .merges
        .iter()
        .any(|m| regions.iter().any(|r| crate::tables::overlaps(*m, *r)))
        || owner
            .array_formulas
            .iter()
            .any(|a| regions.iter().any(|r| crate::tables::overlaps(a.range, *r)))
    {
        return Err(bad(
            "removed report intersects a table, merge or array formula",
        ));
    }
    Ok(regions)
}
/// Remove a saved report and its cells. Shared/referenced caches survive;
/// unshared cache/records parts are collected only from a proven owned graph.
/// Corroborated PivotCharts retain their actual saved data as static charts.
pub fn delete_pivot(original: &[u8], sheet: &str, name: &str) -> Result<PivotDeleteResult> {
    let book = jade_xlsx_reader::read_xlsx(original)?;
    if !book.pivots.issues.is_empty() {
        return Err(bad("removing a pivot requires an unambiguous catalog"));
    }
    let owner = book
        .sheets
        .iter()
        .find(|s| jade_xlsx_reader::formula::same_sheet_name(&s.name, sheet))
        .ok_or_else(|| crate::WriteError::NoSuchSheet(sheet.into()))?;
    let matches = book
        .pivots
        .tables
        .iter()
        .filter(|t| t.sheet_part == owner.part_name && t.name.eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(bad("no unique pivot to remove"));
    }
    let table = matches[0];
    let footprint = report_footprint(&book, table)?;
    let mut package = OpcPackage::open(original)?;
    let opaque_cache_use = slicer_dependencies(&package, table)?;
    let (_, _, links) = package::rels(&package, &owner.part_name)?;
    let links = links
        .iter()
        .filter(|r| {
            jade_xlsx_reader::object_protection::office_relationship(r, "pivotTable")
                && jade_opc::resolve_target(&owner.part_name, &r.target) == table.part_name
        })
        .collect::<Vec<_>>();
    if links.len() != 1 {
        return Err(bad("no unique owned report relationship to remove"));
    }
    let id = links[0].id.clone();
    if package::has_other_incoming(
        &package,
        &[table.part_name.clone()].into_iter().collect(),
        &[(owner.part_name.clone(), id.clone())]
            .into_iter()
            .collect(),
    )? {
        return Err(bad("removed pivot part has other package owners"));
    }
    let detached_charts = crate::pivot_chart::detach_report(&mut package, &book, table)?;
    let body = package.required_part_str(&owner.part_name)?;
    let mut references = owner
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .filter(|c| footprint.iter().any(|r| r.contains(c.reference)))
        .map(|c| c.reference)
        .collect::<BTreeSet<_>>();
    references.extend(footprint.iter().skip(1).map(|r| r.start));
    if references.len() > 1_000_000 {
        return Err(bad("removed report exceeds the edit budget"));
    }
    let edits = references
        .into_iter()
        .map(|reference| CellEdit {
            reference,
            value: NewValue::Blank,
            style_index: None,
        })
        .collect::<Vec<_>>();
    let mut body = crate::patch_sheet(&body, &edits)?;
    // Only legacy producers declare this nonstandard worksheet list. Standard
    // worksheets own their reports directly through the relationship above.
    if let Some(list) = xml::child(&body, "pivotTableParts")? {
        let rows = xml_edit_ns::children(&list.text, b"pivotTablePart")?;
        let mut keep = Vec::new();
        for (_, _, n) in rows {
            if package::relationship_id(&n.text)?.as_deref() != Some(&id) {
                keep.push(n.finish(&n.text)?);
            }
        }
        body = xml::list(&body, "pivotTableParts", "pivotTablePart", |_| Ok(keep))?;
    }
    package.replace(&owner.part_name, body.into_bytes());
    package::remove_relationship(&mut package, &owner.part_name, &id)?;
    package::drop_part(&mut package, &table.part_name)?;
    let workbook = crate::workbook_part_name(&package)?;
    let mut removed_cache = false;
    if !opaque_cache_use
        && !book
            .pivots
            .tables
            .iter()
            .any(|t| t.part_name != table.part_name && t.cache_id == table.cache_id)
    {
        let cache = book
            .pivots
            .caches
            .iter()
            .find(|c| c.id == table.cache_id)
            .ok_or_else(|| bad("missing removed pivot cache"))?;
        let body = package.required_part_str(&workbook)?;
        let list = xml::child(&body, "pivotCaches")?
            .ok_or_else(|| bad("missing pivot cache declarations"))?;
        let declarations = xml_edit_ns::children(&list.text, b"pivotCache")?;
        let mut relationship = None;
        let mut keep = Vec::new();
        for (_, _, n) in declarations {
            if xml::attr(&n.text, "cacheId")?.and_then(|s| s.trim().parse::<u32>().ok())
                == Some(table.cache_id)
            {
                if relationship.is_some() {
                    return Err(bad("ambiguous cache declaration to remove"));
                }
                relationship = package::relationship_id(&n.text)?;
            } else {
                keep.push(n.finish(&n.text)?);
            }
        }
        let relationship =
            relationship.ok_or_else(|| bad("missing removed cache relationship identity"))?;
        let targets = std::iter::once(cache.part_name.clone())
            .chain(cache.records_part.clone())
            .collect::<BTreeSet<_>>();
        let (_, _, cache_links) = package::rels(&package, &cache.part_name)?;
        let mut allowed = [(workbook.clone(), relationship.clone())]
            .into_iter()
            .collect::<BTreeSet<_>>();
        allowed.extend(
            cache_links
                .iter()
                .filter(|r| {
                    jade_xlsx_reader::object_protection::office_relationship(r, "pivotCacheRecords")
                        && cache.records_part.as_ref().is_some_and(|p| {
                            jade_opc::resolve_target(&cache.part_name, &r.target) == *p
                        })
                })
                .map(|r| (cache.part_name.clone(), r.id.clone())),
        );
        if !package::has_other_incoming(&package, &targets, &allowed)? {
            let mut updated = xml::list(&body, "pivotCaches", "pivotCache", |_| Ok(keep))?;
            // CT_PivotCaches has no count attribute (unlike report lists).
            if let Some(list) = xml::child(&updated, "pivotCaches")? {
                let clean = crate::xml_edit::root_attributes(&list.text, &[("count", None)])?;
                let clean = list.finish(&clean)?;
                updated = xml_edit_ns::set_child(&updated, b"pivotCaches", |_| clean, &[])?;
            }
            package.replace(&workbook, updated.into_bytes());
            package::remove_relationship(&mut package, &workbook, &relationship)?;
            for part in targets {
                package::drop_part(&mut package, &part)?;
            }
            removed_cache = true;
        }
    }
    let body = package.required_part_str(&workbook)?;
    package.replace(
        &workbook,
        crate::request_full_recalculation(&body).into_bytes(),
    );
    crate::shift::drop_calc_chain(&mut package, &workbook)?;
    crate::chart_cache::refresh_package(&mut package, &crate::VerifiedChartCells::new())?;
    let bytes = package.write()?;
    let after = jade_xlsx_reader::read_xlsx(&bytes)?;
    if !after.pivots.issues.is_empty()
        || after
            .pivots
            .tables
            .iter()
            .any(|t| t.part_name == table.part_name)
        || book
            .pivots
            .tables
            .iter()
            .filter(|t| t.part_name != table.part_name)
            .any(|t| {
                !after
                    .pivots
                    .tables
                    .iter()
                    .any(|p| p.part_name == t.part_name && p.cache_id == t.cache_id)
            })
    {
        return Err(bad("removed pivot graph is inconsistent"));
    }
    Ok(PivotDeleteResult {
        bytes,
        removed_cache,
        detached_charts,
    })
}
