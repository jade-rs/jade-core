//! Explicit refresh of a worksheet-backed pivot cache and all its reports.
//! Source cells, cache indices, table metadata and report cells form one edit.
mod aggregate;
mod cache;
mod edit;
mod lifecycle;
pub use lifecycle::{
    create_pivot, delete_pivot, PivotCreate, PivotCreateResult, PivotDeleteResult, PivotDestination,
};
mod relationships;
mod report;
mod source;
mod xml;
use crate::{Result, WriteError};
pub use edit::{
    pivot_configuration, PivotFieldOptions, PivotFilterSelection, PivotLayout, PivotMeasure,
    PivotMemberFilter, PivotPage, PivotReportLayout, PivotSource, PivotStyle, PivotUpdate,
};
use jade_xlsx_reader::{CellRange, CellRef};
use std::collections::{BTreeMap, BTreeSet};
/// The caller has evaluated these formula cells successfully in this exact
/// workbook state. An unverified saved formula cache is never a refresh source.
pub type VerifiedPivotCells = BTreeMap<String, BTreeSet<CellRef>>;
#[derive(Debug, Clone)]
pub struct PivotRefreshResult {
    pub bytes: Vec<u8>,
    pub cache_id: u32,
    pub source_sheet: String,
    pub source_range: CellRange,
    pub source_rows: usize,
    pub tables: Vec<String>,
}
/// Resolve the same source rectangle used by refresh, without trusting saved
/// formula values. The sheet identity is its package part, not a display name.
pub fn source_range(
    book: &jade_xlsx_reader::Workbook,
    cache_id: u32,
) -> Result<(String, CellRange)> {
    let caches = book
        .pivots
        .caches
        .iter()
        .filter(|c| c.id == cache_id)
        .collect::<Vec<_>>();
    if caches.len() != 1 {
        return Err(bad("no unique source cache for this pivot"));
    }
    let (sheet, range) = source::location(book, caches[0])?;
    Ok((sheet.part_name.clone(), range))
}
fn bad(message: impl Into<String>) -> WriteError {
    WriteError::Pivot(message.into())
}
/// Refresh a named pivot and every other report sharing its cache. All plans
/// and collision checks complete before any returned package is constructed.
pub fn refresh_pivot(
    original: &[u8],
    sheet: &str,
    name: &str,
    verified: &VerifiedPivotCells,
) -> Result<PivotRefreshResult> {
    rebuild_pivot(original, sheet, name, verified, None)
}
/// Edit a saved worksheet pivot, then rebuild its cache and every report that
/// shares that cache. The returned package is the complete atomic result.
pub fn update_pivot(
    original: &[u8],
    sheet: &str,
    name: &str,
    update: &PivotUpdate,
    verified: &VerifiedPivotCells,
) -> Result<PivotRefreshResult> {
    rebuild_pivot(original, sheet, name, verified, Some(update))
}
fn rebuild_pivot(
    original: &[u8],
    sheet: &str,
    name: &str,
    verified: &VerifiedPivotCells,
    update: Option<&PivotUpdate>,
) -> Result<PivotRefreshResult> {
    let book = jade_xlsx_reader::read_xlsx(original)?;
    let mut package = jade_opc::OpcPackage::open(original)?;
    crate::pivots::require_source_context(&package, &book)?;
    let owner = book
        .sheets
        .iter()
        .find(|s| jade_xlsx_reader::formula::same_sheet_name(&s.name, sheet))
        .ok_or_else(|| WriteError::NoSuchSheet(sheet.into()))?;
    let found = book
        .pivots
        .tables
        .iter()
        .filter(|t| t.sheet_part == owner.part_name && t.name.eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    if found.len() != 1 {
        return Err(bad("no unique pivot with that name on the worksheet"));
    }
    let id = found[0].cache_id;
    let caches = book
        .pivots
        .caches
        .iter()
        .filter(|c| c.id == id)
        .collect::<Vec<_>>();
    if caches.len() != 1 {
        return Err(bad("no unique source cache for this pivot"));
    }
    let original_cache = caches[0];
    let edited = update
        .map(|update| edit::prepare(&mut package, &book, found[0], original_cache, update))
        .transpose()?;
    let cache = edited
        .as_ref()
        .map_or(original_cache.as_ref(), |e| &e.cache);
    let source = source::read(&book, cache, verified)?;
    let refreshed = cache::rebuild(cache, &source)?;
    let shared = book
        .pivots
        .tables
        .iter()
        .filter(|t| t.cache_id == id)
        .collect::<Vec<_>>();
    let mut plans = Vec::new();
    let mut total_edits = 0usize;
    for t in shared {
        let t = edited
            .as_ref()
            .filter(|e| e.table.part_name == t.part_name)
            .map_or(t.as_ref(), |e| &e.table);
        let plan = report::build(
            &book,
            t,
            cache,
            &refreshed,
            &source,
            &package.required_part_str(&t.part_name)?,
        )?;
        total_edits = total_edits.saturating_add(plan.edits.len());
        if total_edits > 1_000_000 {
            return Err(bad("shared pivot reports exceed the refresh edit budget"));
        }
        plans.push(plan);
    }
    report::check_destinations(&book, &plans, &source)?;
    relationships::ensure_cache_links(&mut package, &book, id, &cache.part_name)?;
    cache::write(&mut package, cache, &refreshed)?;
    for plan in &plans {
        report::write(&mut package, plan)?;
    }
    let measures = edited
        .as_ref()
        .map(|e| (e.table.part_name.as_str(), e.measure_keys.as_slice()));
    if measures.is_some() {
        crate::pivot_chart::synchronize_edited(&mut package, &book, id, measures)?;
    } else {
        crate::pivot_chart::synchronize(&mut package, &book, id)?;
    }
    let workbook_part = crate::workbook_part_name(&package)?;
    let workbook_xml = package.required_part_str(&workbook_part)?;
    package.replace(
        &workbook_part,
        crate::request_full_recalculation(&workbook_xml).into_bytes(),
    );
    crate::shift::drop_calc_chain(&mut package, &workbook_part)?;
    crate::chart_cache::refresh_package(&mut package, &crate::VerifiedChartCells::new())?;
    let bytes = package.write()?;
    // Do not return a package whose interpreted catalog already contradicts
    // what was just written. Office/schema/oracle validation remains separate.
    let after = jade_xlsx_reader::read_xlsx(&bytes)?;
    if after
        .pivots
        .issues
        .iter()
        .any(|issue| !book.pivots.issues.contains(issue))
    {
        return Err(bad("refreshed pivot metadata is inconsistent"));
    }
    if !after.pivots.caches.iter().any(|c| {
        c.id == id
            && c.records
                .as_ref()
                .is_some_and(|r| r.rows.len() == source.rows.len())
    }) || after
        .pivots
        .tables
        .iter()
        .filter(|t| t.cache_id == id)
        .count()
        != plans.len()
    {
        return Err(bad("refreshed cache/report identities are incomplete"));
    }
    Ok(PivotRefreshResult {
        bytes,
        cache_id: id,
        source_sheet: source.sheet_name.clone(),
        source_range: source.range,
        source_rows: source.rows.len(),
        tables: plans.iter().map(|p| p.table.name.clone()).collect(),
    })
}
