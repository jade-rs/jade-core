//! Synchronize explicitly bound classic PivotCharts with their visible reports.
mod projection;
mod xml;
use crate::{Result, WriteError};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    chart::{resolve_chart_reference, Chart},
    pivot::PivotTable,
    Sheet, Workbook,
};
use std::collections::{BTreeMap, BTreeSet};
fn local_evidence(
    chart: &Chart,
    host: &Sheet,
    book: &Workbook,
    t: &PivotTable,
    old: &projection::Projection,
) -> bool {
    let mut references = false;
    for series in chart
        .series
        .iter()
        .chain(chart.extra_plots.iter().flat_map(|p| &p.series))
    {
        for data in [&series.categories, &series.values] {
            if let Some(f) = &data.formula {
                let Some(r) = resolve_chart_reference(f, host, Some(book)) else {
                    return false;
                };
                if r.sheet.part_name != t.sheet_part
                    || r.points
                        .iter()
                        .any(|p| !t.location.reference.contains(p.reference))
                {
                    return false;
                }
                references = true;
            }
        }
    }
    if references {
        return true;
    }
    let series = chart
        .series
        .iter()
        .chain(chart.extra_plots.iter().flat_map(|p| &p.series))
        .collect::<Vec<_>>();
    let labels = old
        .categories
        .iter()
        .map(|c| Some(c.label.clone()))
        .collect::<Vec<_>>();
    series.len() == old.series.len()
        && series.iter().all(|a| {
            a.categories.points == labels && old.series.iter().any(|b| a.values.points == b.values)
        })
}
fn series_bindings(
    chart: &Chart,
    host: &Sheet,
    book: &Workbook,
    old: &projection::Projection,
) -> Result<BTreeMap<(usize, u32), usize>> {
    let refuse = |s: &str| WriteError::Pivot(format!("pivot chart: {s}"));
    let mut out = BTreeMap::new();
    let mut used = BTreeSet::new();
    let plots = std::iter::once(&chart.series).chain(chart.extra_plots.iter().map(|p| &p.series));
    for (plot, series) in plots.enumerate() {
        for s in series {
            let reference = s
                .values
                .formula
                .as_deref()
                .and_then(|f| resolve_chart_reference(f, host, Some(book)))
                .map(|r| {
                    (
                        r.sheet.part_name.clone(),
                        r.points
                            .iter()
                            .map(|p| p.reference)
                            .collect::<BTreeSet<_>>(),
                    )
                });
            let candidates = old
                .series
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    reference
                        .as_ref()
                        .is_none_or(|(_, cells)| p.references.iter().all(|r| cells.contains(r)))
                })
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            let names = candidates
                .iter()
                .copied()
                .filter(|i| {
                    s.name.points.first().and_then(|v| v.as_deref())
                        == Some(old.series[*i].name.as_str())
                })
                .collect::<Vec<_>>();
            let values = candidates
                .iter()
                .copied()
                .filter(|i| s.values.points == old.series[*i].values)
                .collect::<Vec<_>>();
            let choice = if candidates.len() == 1 {
                candidates[0]
            } else if names.len() == 1 {
                names[0]
            } else if values.len() == 1 {
                values[0]
            } else {
                return Err(refuse("ambiguous saved series identity"));
            };
            if !used.insert(choice) || out.insert((plot, s.order), choice).is_some() {
                return Err(refuse("duplicate saved series identity"));
            }
        }
    }
    if used.len() != old.series.len() {
        return Err(refuse("filtered series require explicit source semantics"));
    }
    Ok(out)
}

/// A report tab rename keeps its explicit local chart binding. Work from the
/// original model because the staged package's series formulas already name
/// the new sheet. Unresolved/external bindings remain opaque.
/// A PivotChart copied with sheet `source` as sheet `copy`: one showing a
/// pivot table on `source` shows the copy's (which has its name), as
/// Excel's Move or Copy does; one showing a pivot table elsewhere keeps
/// showing it. A chart without a pivot source comes back as it was.
pub(crate) fn copied_chart_source(xml: &str, source: &str, copy: &str) -> Result<String> {
    let Some(pivot) = jade_xlsx_reader::chart::parse_chart(xml)?.pivot_source else {
        return Ok(xml.to_string());
    };
    let Some(name) = pivot.qualified_name() else {
        return Err(WriteError::CopySheet(
            "a PivotChart's pivot table cannot be told".into(),
        ));
    };
    if !jade_xlsx_reader::formula::same_sheet_name(&name.sheet, source) {
        return Ok(xml.to_string());
    }
    let owner = format!(
        "{}{copy}",
        name.workbook
            .as_ref()
            .map(|book| format!("[{book}]"))
            .unwrap_or_default()
    );
    let renamed = format!("'{}'!{}", owner.replace('\'', "''"), name.pivot);
    xml::rename_source(xml, &renamed)
}

pub(crate) fn rename_sheet_sources(
    package: &mut OpcPackage,
    before: &Workbook,
    old_name: &str,
    new_name: &str,
) -> Result<()> {
    let mut prepared = BTreeMap::new();
    for host in &before.sheets {
        for chart in host.shapes.iter().filter_map(|s| s.chart.as_ref()) {
            if chart
                .unsupported_properties
                .iter()
                .any(|p| p == "external chart data")
            {
                continue;
            }
            let Some(source) = &chart.pivot_source else {
                continue;
            };
            let Some(name) = source.qualified_name() else {
                continue;
            };
            if !jade_xlsx_reader::formula::same_sheet_name(&name.sheet, old_name)
                || name
                    .workbook
                    .as_ref()
                    .is_some_and(|n| n.contains(['/', '\\', ':']))
            {
                continue;
            }
            let tables = before
                .pivots
                .tables
                .iter()
                .filter(|t| {
                    t.name.eq_ignore_ascii_case(&name.pivot)
                        && before.sheets.iter().any(|s| {
                            s.part_name == t.sheet_part && s.name.eq_ignore_ascii_case(&name.sheet)
                        })
                })
                .collect::<Vec<_>>();
            if tables.len() != 1 {
                continue;
            }
            let Ok(projection) = projection::project(before, tables[0]) else {
                continue;
            };
            if !local_evidence(chart, host, before, tables[0], &projection) {
                continue;
            }
            let Some(part) = &chart.part_name else {
                continue;
            };
            let owner = format!(
                "{}{new_name}",
                name.workbook
                    .as_ref()
                    .map(|n| format!("[{n}]"))
                    .unwrap_or_default()
            );
            let source_name = format!("'{}'!{}", owner.replace('\'', "''"), name.pivot);
            prepared.insert(
                part.clone(),
                xml::rename_source(&package.required_part_str(part)?, &source_name)?,
            );
        }
    }
    for (part, xml) in prepared {
        package.replace(&part, xml.into_bytes());
    }
    Ok(())
}
/// Runs against a staged package. No original source/history is changed until
/// every linked chart succeeds, just like the cache/report rebuild itself.
pub(crate) fn synchronize(
    package: &mut OpcPackage,
    before: &Workbook,
    cache_id: u32,
) -> Result<()> {
    synchronize_edited(package, before, cache_id, None)
}
pub(crate) fn synchronize_edited(
    package: &mut OpcPackage,
    before: &Workbook,
    cache_id: u32,
    edited_measures: Option<(&str, &[u32])>,
) -> Result<()> {
    let mut prepared = Vec::new();
    let mut seen = BTreeSet::new();
    let after = jade_xlsx_reader::read_package(package)?;
    for host in &before.sheets {
        for chart in host.shapes.iter().filter_map(|s| s.chart.as_ref()) {
            if chart
                .unsupported_properties
                .iter()
                .any(|p| p == "external chart data")
            {
                continue;
            }
            let Some(source) = &chart.pivot_source else {
                continue;
            };
            let Some(name) = source.qualified_name() else {
                continue;
            };
            if name
                .workbook
                .as_ref()
                .is_some_and(|n| n.contains(['/', '\\', ':']))
            {
                continue;
            }
            let matches = before
                .pivots
                .tables
                .iter()
                .filter(|t| {
                    t.name.eq_ignore_ascii_case(&name.pivot)
                        && before.sheets.iter().any(|s| {
                            s.part_name == t.sheet_part && s.name.eq_ignore_ascii_case(&name.sheet)
                        })
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 || matches[0].cache_id != cache_id {
                continue;
            }
            let t = matches[0];
            let old = projection::project(before, t)?;
            if !local_evidence(chart, host, before, t, &old) {
                continue;
            }
            let Some(part) = chart.part_name.as_ref() else {
                continue;
            };
            if !seen.insert(part.clone()) {
                continue;
            }
            let targets = after
                .pivots
                .tables
                .iter()
                .filter(|p| p.part_name == t.part_name && p.sheet_part == t.sheet_part)
                .collect::<Vec<_>>();
            if targets.len() != 1 {
                return Err(WriteError::Pivot(
                    "pivot chart report identity was lost".into(),
                ));
            }
            let mut fresh = projection::project(&after, targets[0])?;
            if let Some((_, keys)) = edited_measures.filter(|(part, _)| *part == t.part_name) {
                for series in &mut fresh.series {
                    series.key.measure =
                        *keys.get(series.key.measure as usize).ok_or_else(|| {
                            WriteError::Pivot("lost edited chart measure identity".into())
                        })?;
                }
            }
            let xml = package.required_part_str(part)?;
            let bindings = series_bindings(chart, host, before, &old)?;
            prepared.push((
                part.clone(),
                xml::update(
                    &xml,
                    &old,
                    &fresh,
                    &bindings,
                    &after
                        .sheets
                        .iter()
                        .find(|s| s.part_name == t.sheet_part)
                        .ok_or_else(|| {
                            WriteError::Pivot("missing pivot chart source sheet".into())
                        })?
                        .name,
                )?,
            ));
        }
    }
    for (part, xml) in prepared {
        package.replace(&part, xml.into_bytes());
    }
    Ok(())
}

/// Removing a report leaves corroborated local charts as static cached charts.
/// Unresolved local-looking bindings refuse; explicit external sources remain.
pub(crate) fn detach_report(
    package: &mut OpcPackage,
    before: &Workbook,
    table: &PivotTable,
) -> Result<usize> {
    let owner = before
        .sheets
        .iter()
        .find(|s| s.part_name == table.sheet_part)
        .ok_or_else(|| WriteError::Pivot("missing removed chart report owner".into()))?;
    let mut prepared = BTreeMap::new();
    for host in &before.sheets {
        for chart in host.shapes.iter().filter_map(|s| s.chart.as_ref()) {
            if chart
                .unsupported_properties
                .iter()
                .any(|p| p == "external chart data")
            {
                continue;
            }
            let Some(name) = chart.pivot_source.as_ref().and_then(|s| s.qualified_name()) else {
                continue;
            };
            if !name.pivot.eq_ignore_ascii_case(&table.name)
                || !jade_xlsx_reader::formula::same_sheet_name(&name.sheet, &owner.name)
                || name
                    .workbook
                    .as_ref()
                    .is_some_and(|n| n.contains(['/', '\\', ':']))
            {
                continue;
            }
            let references = chart
                .series
                .iter()
                .chain(chart.extra_plots.iter().flat_map(|p| &p.series))
                .flat_map(|s| [&s.categories, &s.values])
                .filter_map(|d| d.formula.as_deref())
                .collect::<Vec<_>>();
            let local = if references.is_empty() {
                projection::project(before, table)
                    .is_ok_and(|p| local_evidence(chart, host, before, table, &p))
            } else {
                references.iter().all(|f| {
                    resolve_chart_reference(f, host, Some(before)).is_some_and(|r| {
                        r.sheet.part_name == table.sheet_part
                            && r.points
                                .iter()
                                .all(|p| table.location.reference.contains(p.reference))
                    })
                })
            };
            if !local {
                return Err(WriteError::Pivot(
                    "cannot establish a local chart binding for report removal".into(),
                ));
            }
            let part = chart
                .part_name
                .as_ref()
                .ok_or_else(|| WriteError::Pivot("missing detached chart part".into()))?;
            if !prepared.contains_key(part) {
                let snapshot = crate::chart_cache::snapshot_chart_data(
                    &package.required_part_str(part)?,
                    host,
                    before,
                    chart.visible_only,
                )?;
                let fresh_before = jade_xlsx_reader::chart::parse_chart(&snapshot)?;
                let updated = xml::detach(&snapshot)?;
                let parsed = jade_xlsx_reader::chart::parse_chart(&updated)?;
                let chart = &fresh_before;
                let old_series = chart
                    .series
                    .iter()
                    .chain(chart.extra_plots.iter().flat_map(|p| &p.series))
                    .collect::<Vec<_>>();
                let fresh_series = parsed
                    .series
                    .iter()
                    .chain(parsed.extra_plots.iter().flat_map(|p| &p.series))
                    .collect::<Vec<_>>();
                let same_data =
                    |a: &jade_xlsx_reader::chart::ChartData,
                     b: &jade_xlsx_reader::chart::ChartData| {
                        a.points == b.points
                            && a.numeric == b.numeric
                            && a.format_code == b.format_code
                    };
                if parsed.pivot_source.is_some()
                    || chart.kind != parsed.kind
                    || chart.extra_plots.len() != parsed.extra_plots.len()
                    || old_series.len() != fresh_series.len()
                    || old_series.iter().zip(fresh_series).any(|(a, b)| {
                        a.order != b.order
                            || !same_data(&a.name, &b.name)
                            || !same_data(&a.categories, &b.categories)
                            || !same_data(&a.values, &b.values)
                    })
                    || (chart.title_source.formula.is_some()
                        && parsed.title
                            != chart
                                .title_source
                                .points
                                .first()
                                .and_then(|v| v.as_deref())
                                .unwrap_or(""))
                {
                    return Err(WriteError::Pivot(
                        "detached chart caches or identity changed during conversion".into(),
                    ));
                }
                prepared.insert(part.clone(), updated);
            }
        }
    }
    let count = prepared.len();
    for (part, body) in prepared {
        package.replace(&part, body.into_bytes());
    }
    Ok(count)
}
