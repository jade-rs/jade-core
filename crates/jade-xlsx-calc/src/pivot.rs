//! Project saved visible pivot coordinates without recomputing source records.
use ironcalc_base::pivot::{Field, FieldRole, ItemValue, Line, Measure, Member, Region};
use jade_xlsx_reader::{
    pivot::{AxisItem, PivotCache, PivotScalar, PivotTable},
    Workbook,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn value(v: &PivotScalar) -> ItemValue {
    match v {
        PivotScalar::Missing => ItemValue::Missing,
        PivotScalar::Number(n) => ItemValue::Number(*n),
        PivotScalar::Text(s) => ItemValue::Text(s.clone()),
        PivotScalar::Bool(b) => ItemValue::Boolean(*b),
        PivotScalar::Error(e) => ItemValue::Error(e.clone()),
        PivotScalar::Date(s) => ItemValue::Date(s.clone()),
        _ => ItemValue::Unknown,
    }
}
fn aliases(primary: Option<&str>, root: &str) -> Vec<String> {
    let mut names = vec![primary.unwrap_or(root).into()];
    if names[0] != root {
        names.push(root.into());
    }
    names
}
fn expanded(
    items: &[AxisItem],
    axis: &[i32],
    fields: &[Field],
    measure_count: usize,
    expected: usize,
) -> Result<Vec<Line>, String> {
    if items.is_empty() {
        if axis.iter().any(|i| *i >= 0) || expected != 1 {
            return Err("Missing visible axis items".into());
        }
        return Ok(vec![Line {
            members: BTreeMap::new(),
            measure: None,
            kind: "grand".into(),
        }]);
    }
    if items.len() != expected {
        return Err("Pivot item count does not match its saved data area".into());
    }
    let values = axis.contains(&-2);
    let mut previous = Vec::new();
    let mut result = Vec::with_capacity(items.len());
    for item in items {
        let repeat = item.repeat as usize;
        if repeat > previous.len() {
            return Err("Invalid repeated pivot item prefix".into());
        }
        let mut raw = previous[..repeat].to_vec();
        raw.extend(&item.items);
        let measure = values.then_some(item.data_index);
        if measure.is_some_and(|i| i as usize >= measure_count) {
            return Err("Pivot data field index outside definitions".into());
        }
        let mut members = BTreeMap::new();
        if item.kind != "grand" && item.kind != "blank" {
            let actual = axis
                .iter()
                .filter(|i| **i >= 0)
                .copied()
                .collect::<Vec<_>>();
            let positions = if values && raw.len() == actual.len() {
                actual.as_slice()
            } else {
                axis
            };
            if raw.len() > positions.len() {
                return Err("Too many pivot item coordinates".into());
            }
            for (field, index) in positions.iter().zip(&raw) {
                if *field < 0 {
                    continue;
                }
                let field = *field as usize;
                let member = fields
                    .get(field)
                    .and_then(|f| f.members.get(*index as usize))
                    .ok_or("Pivot item index outside its field")?;
                if member.hidden || member.aliases.is_empty() {
                    return Err(
                        "Visible pivot line references a hidden or uninterpreted item".into(),
                    );
                }
                members.insert(field, *index);
            }
        }
        result.push(Line {
            members,
            measure,
            kind: item.kind.clone(),
        });
        previous = raw;
    }
    Ok(result)
}
fn populate(region: &mut Region, t: &PivotTable, c: &PivotCache) -> Result<(), String> {
    // An uninterpreted alternative may contain dimensions/report filters. Cube
    // hierarchies also use member identities beyond the flat cache aliases.
    // Neither may be mistaken for a genuinely absent/invisible field. Opaque
    // extensions alone do not prevent reading a complete saved flat layout.
    if t.unsupported.iter().chain(&c.unsupported).any(|path| {
        matches!(
            path.rsplit('/').next(),
            Some(
                "AlternateContent"
                    | "pivotHierarchies"
                    | "cacheHierarchies"
                    | "rowHierarchiesUsage"
                    | "colHierarchiesUsage"
                    | "tupleCache"
                    | "dimensions"
                    | "measureGroups"
                    | "calculatedMembers"
                    | "kpis"
                    | "maps"
            )
        )
    }) || t
        .page_fields
        .iter()
        .any(|p| p.hierarchy.is_some_and(|h| h >= 0))
    {
        return Err("Uninterpreted pivot alternatives or cube hierarchy identities".into());
    }
    if t.fields.len() != c.fields.len() {
        return Err("Pivot and cache field counts disagree".into());
    }
    let mut rows = BTreeSet::new();
    let mut columns = BTreeSet::new();
    for (axis, set) in [(&t.row_fields, &mut rows), (&t.column_fields, &mut columns)] {
        for i in axis.iter().filter(|i| **i >= 0) {
            if *i as usize >= c.fields.len() || !set.insert(*i as usize) {
                return Err("Invalid or duplicate pivot axis field".into());
            }
        }
    }
    if !rows.is_disjoint(&columns) {
        return Err("Pivot field appears on both axes".into());
    }
    let mut pages = BTreeMap::new();
    for p in &t.page_fields {
        if p.field as usize >= c.fields.len()
            || pages.insert(p.field as usize, p.item).is_some()
            || rows.contains(&(p.field as usize))
            || columns.contains(&(p.field as usize))
        {
            return Err("Invalid or duplicate report filter field".into());
        }
    }
    for (index, (field, cache)) in t.fields.iter().zip(&c.fields).enumerate() {
        let source = cache
            .grouping
            .as_ref()
            .filter(|g| !g.items.is_empty())
            .map_or(cache.shared_items.as_slice(), |g| g.items.as_slice());
        let mut members = Vec::new();
        for item in &field.items {
            let mut names = Vec::new();
            if item.kind == "data" {
                if let Some(shared) = item.shared_index.and_then(|i| source.get(i as usize)) {
                    names.push(value(&shared.value));
                    if let Some(caption) = &shared.caption {
                        names.push(ItemValue::Text(caption.clone()));
                    }
                } else {
                    names.push(ItemValue::Unknown);
                }
            }
            if let Some(caption) = &item.caption {
                names.push(ItemValue::Text(caption.clone()));
            }
            members.push(Member {
                aliases: names,
                hidden: item.hidden,
            });
        }
        let role = if rows.contains(&index) {
            FieldRole::Row
        } else if columns.contains(&index) {
            FieldRole::Column
        } else if pages.contains_key(&index) {
            FieldRole::Page
        } else {
            FieldRole::Unused
        };
        // PageField.item indexes the cache, while axis x coordinates index
        // PivotField.items. Resolve those two index spaces explicitly.
        let page_item = if let Some(shared) = pages.get(&index).copied().flatten() {
            let matches = field
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    (item.kind == "data" && item.shared_index == Some(shared)).then_some(i as u32)
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err("Report filter cache item has no unique field item".into());
            }
            Some(matches[0])
        } else {
            None
        };
        region.fields.push(Field {
            names: aliases(field.caption.as_deref(), &cache.name),
            members,
            role,
            page_item,
        });
    }
    for measure in &t.data_fields {
        let field = c
            .fields
            .get(measure.field as usize)
            .ok_or("Measure refers to an absent cache field")?;
        region.measures.push(Measure {
            names: aliases(measure.name.as_deref(), &field.name),
            aggregation: measure.aggregation.clone(),
        });
    }
    if region.measures.is_empty() {
        return Err("No pivot data fields".into());
    }
    if region.measures.len() > 1 && !t.row_fields.contains(&-2) && !t.column_fields.contains(&-2) {
        return Err("Multiple measures have no values axis".into());
    }
    let height = usize::try_from(region.last_row - region.data_row + 1)
        .map_err(|_| "Invalid pivot data rows")?;
    let width = usize::try_from(region.last_column - region.data_column + 1)
        .map_err(|_| "Invalid pivot data columns")?;
    region.rows = expanded(
        &t.row_items,
        &t.row_fields,
        &region.fields,
        region.measures.len(),
        height,
    )?;
    region.columns = expanded(
        &t.column_items,
        &t.column_fields,
        &region.fields,
        region.measures.len(),
        width,
    )?;
    // Compact group headers can occupy physical rows without a displayed
    // subtotal. They are not queryable summary values. Keep those rows in the
    // coordinate map while excluding them from the summary lookup index.
    let actual = t
        .row_fields
        .iter()
        .filter(|i| **i >= 0)
        .map(|i| *i as usize)
        .collect::<Vec<_>>();
    for line in &mut region.rows {
        if line.kind == "data" && !line.members.is_empty() && line.members.len() < actual.len() {
            let field = actual.iter().rev().find(|i| line.members.contains_key(i));
            if field.is_some_and(|i| {
                let attrs = &t.fields[*i].attributes;
                attrs
                    .get("defaultSubtotal")
                    .is_some_and(|v| matches!(v.as_str(), "0" | "false"))
                    || attrs
                        .get("subtotalTop")
                        .is_some_and(|v| matches!(v.as_str(), "0" | "false"))
            }) {
                line.kind = "header".into();
            }
        }
    }
    Ok(())
}
pub(crate) fn regions(workbook: &Workbook) -> Vec<Arc<Region>> {
    let mut regions = Vec::new();
    // An unreadable declaration has no reliable bounds. Keep that uncertainty
    // separate from a genuine no-pivot REF result, then prefer known regions.
    for (sheet, s) in workbook.sheets.iter().enumerate() {
        if workbook
            .pivots
            .issues
            .iter()
            .any(|i| i.part_name == s.part_name)
        {
            regions.push(Arc::new(Region {
                sheet: sheet as u32,
                first_row: 1,
                first_column: 1,
                last_row: 1_048_576,
                last_column: 16_384,
                data_row: 1,
                data_column: 1,
                fields: Vec::new(),
                measures: Vec::new(),
                rows: Vec::new(),
                columns: Vec::new(),
                unsupported: Some("Uninterpreted pivot declarations on this worksheet".into()),
            }));
        }
    }
    for table in &workbook.pivots.tables {
        let Some(sheet) = workbook
            .sheets
            .iter()
            .position(|s| s.part_name == table.sheet_part)
        else {
            continue;
        };
        let loc = &table.location;
        let r = loc.reference;
        let mut region = Region {
            sheet: sheet as u32,
            first_row: r.start.row as i32 + 1,
            first_column: r.start.column as i32 + 1,
            last_row: r.end.row as i32 + 1,
            last_column: r.end.column as i32 + 1,
            data_row: (r.start.row + loc.first_data_row) as i32 + 1,
            data_column: (r.start.column + loc.first_data_column) as i32 + 1,
            fields: Vec::new(),
            measures: Vec::new(),
            rows: Vec::new(),
            columns: Vec::new(),
            unsupported: None,
        };
        let cache = workbook
            .pivots
            .caches
            .iter()
            .filter(|c| c.id == table.cache_id)
            .collect::<Vec<_>>();
        let ready = if cache.len() == 1 {
            populate(&mut region, table, cache[0])
        } else {
            Err("Unresolved pivot cache identity".into())
        };
        if let Err(reason) = ready {
            region.unsupported = Some(reason);
        }
        regions.push(Arc::new(region));
    }
    regions
}
