use super::{aggregate::Aggregate, bad, cache::Rebuilt, source::Source, xml, Result};
use crate::{
    patch::{apply, Splice},
    xml_edit, xml_edit_ns, CellEdit, NewValue,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    pivot::{PivotCache, PivotField, PivotItem, PivotTable},
    CellRange, CellRef, CellValue, Workbook,
};
use std::collections::{BTreeMap, BTreeSet};
pub(super) struct Plan {
    pub table: PivotTable,
    old: Option<CellRange>,
    pub range: CellRange,
    fields: Vec<PivotField>,
    rows: Vec<Line>,
    columns: Vec<Line>,
    row_axis: Vec<i32>,
    column_axis: Vec<i32>,
    data_row: u32,
    data_column: u32,
    pub(super) edits: Vec<CellEdit>,
    formats: BTreeMap<CellRef, (u32, Option<String>, Option<u32>)>,
    compact: bool,
    old_pages: Vec<CellRef>,
    pages: Vec<CellRef>,
}
#[derive(Clone)]
struct Line {
    members: Vec<u32>,
    kind: String,
    measure: Option<usize>,
    top: bool,
}
pub(super) fn flag(attrs: &BTreeMap<String, String>, name: &str, default: bool) -> Result<bool> {
    match attrs.get(name).map(|s| s.trim()) {
        None => Ok(default),
        Some("1" | "true") => Ok(true),
        Some("0" | "false") => Ok(false),
        _ => Err(bad(format!("invalid pivot {name}"))),
    }
}
fn dimensions(axis: &[i32], count: usize) -> Result<Vec<usize>> {
    let mut fields = Vec::new();
    let mut values = false;
    for index in axis {
        if *index == -2 {
            if values {
                return Err(bad("duplicate values axis"));
            }
            values = true;
        } else if *index < 0 || *index as usize >= count || fields.contains(&(*index as usize)) {
            return Err(bad("invalid pivot axis field"));
        } else {
            fields.push(*index as usize);
        }
    }
    Ok(fields)
}
fn axis_lines(
    paths: &BTreeSet<Vec<u32>>,
    axis: &[i32],
    fields: &[PivotField],
    measures: usize,
    compact: bool,
    ranks: &[BTreeMap<u32, u64>],
    grand: bool,
) -> Result<Vec<Line>> {
    let dims = dimensions(axis, fields.len())?;
    let mut coordinate_cost = dims.len();
    for (depth, field) in dims.iter().enumerate().take(dims.len().saturating_sub(1)) {
        if compact || flag(&fields[*field].attributes, "defaultSubtotal", true)? {
            coordinate_cost = coordinate_cost.saturating_add(depth + 1);
        }
    }
    if paths.len().saturating_mul(coordinate_cost) > 1_000_000 {
        return Err(bad("pivot axis descriptors exceed the coordinate budget"));
    }
    let mut base = Vec::new();
    for path in paths {
        base.push(Line {
            members: path.clone(),
            kind: "data".into(),
            measure: None,
            top: false,
        });
    }
    for depth in 1..dims.len() {
        let f = &fields[dims[depth - 1]];
        let custom = [
            "sumSubtotal",
            "countASubtotal",
            "avgSubtotal",
            "maxSubtotal",
            "minSubtotal",
            "productSubtotal",
            "countSubtotal",
            "stdDevSubtotal",
            "stdDevPSubtotal",
            "varSubtotal",
            "varPSubtotal",
        ]
        .iter()
        .any(|a| {
            f.attributes
                .get(*a)
                .is_some_and(|v| matches!(v.as_str(), "1" | "true"))
        });
        if custom {
            return Err(bad(
                "custom subtotal refresh needs explicit subtotal semantics",
            ));
        }
        let subtotal = flag(&f.attributes, "defaultSubtotal", true)?;
        let top = flag(&f.attributes, "subtotalTop", true)?;
        for prefix in paths
            .iter()
            .map(|p| p[..depth].to_vec())
            .collect::<BTreeSet<_>>()
        {
            if subtotal {
                base.push(Line {
                    members: prefix.clone(),
                    kind: "default".into(),
                    measure: None,
                    top,
                });
            }
            if compact && (!subtotal || !top) {
                base.push(Line {
                    members: prefix,
                    kind: "header".into(),
                    measure: None,
                    top: true,
                });
            }
        }
    }
    if grand || dims.is_empty() {
        base.push(Line {
            members: vec![],
            kind: "grand".into(),
            measure: None,
            top: false,
        });
    }
    if dims.is_empty() {
        base.retain(|l| l.kind == "grand");
    }
    if base.is_empty() {
        base.push(Line {
            members: vec![],
            kind: "blank".into(),
            measure: None,
            top: false,
        });
    }
    let values = axis.contains(&-2);
    if base.len().saturating_mul(if values { measures } else { 1 }) > 1_000_000 {
        return Err(bad("pivot axis expansion exceeds the line budget"));
    }
    let mut lines = base
        .into_iter()
        .flat_map(|l| {
            if values {
                (0..measures)
                    .map(|m| Line {
                        measure: Some(m),
                        ..l.clone()
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![l]
            }
        })
        .collect::<Vec<_>>();
    let key = |line: &Line| {
        let mut real = 0;
        let mut out = Vec::new();
        for field in axis {
            if *field == -2 {
                out.push(line.measure.unwrap_or(0) as u64 + 1);
            } else {
                out.push(
                    line.members
                        .get(real)
                        .map_or(if line.top { 0 } else { u64::MAX }, |i| {
                            ranks[*field as usize]
                                .get(i)
                                .copied()
                                .unwrap_or(u64::from(*i))
                                + 1
                        }),
                );
                real += 1;
            }
        }
        out
    };
    lines.sort_by_cached_key(key);
    Ok(lines)
}
fn member_order(a: &CellValue, b: &CellValue) -> std::cmp::Ordering {
    let category = |v: &CellValue| match v {
        CellValue::Number(_) => 0,
        CellValue::Text(_) => 1,
        CellValue::Bool(_) => 2,
        CellValue::Error(_) => 3,
        CellValue::Empty => 4,
    };
    category(a).cmp(&category(b)).then_with(|| match (a, b) {
        (CellValue::Number(a), CellValue::Number(b)) => {
            a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
        }
        (CellValue::Text(a), CellValue::Text(b)) | (CellValue::Error(a), CellValue::Error(b)) => {
            a.to_uppercase().cmp(&b.to_uppercase())
        }
        (CellValue::Bool(a), CellValue::Bool(b)) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    })
}
fn member_ranks(fields: &[PivotField], new: &Rebuilt) -> Result<Vec<BTreeMap<u32, u64>>> {
    fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let sort = f
                .attributes
                .get("sortType")
                .map(String::as_str)
                .unwrap_or("manual");
            if !matches!(sort, "manual" | "ascending" | "descending") {
                return Err(bad("unknown pivot sort mode"));
            }
            let mut items = f
                .items
                .iter()
                .enumerate()
                .filter_map(|(at, v)| v.shared_index.map(|s| (at as u32, s as usize)))
                .collect::<Vec<_>>();
            if sort != "manual" {
                items.sort_by(|(ai, a), (bi, b)| {
                    let order = member_order(&new.native[i][*a], &new.native[i][*b]);
                    (if sort == "descending" {
                        order.reverse()
                    } else {
                        order
                    })
                    .then(ai.cmp(bi))
                });
            }
            Ok(items
                .into_iter()
                .enumerate()
                .map(|(rank, (item, _))| (item, rank as u64))
                .collect())
        })
        .collect()
}
fn measure_name(t: &PivotTable, c: &PivotCache, m: usize) -> String {
    t.data_fields[m].name.clone().unwrap_or_else(|| {
        format!(
            "{} of {}",
            t.data_fields[m].aggregation, c.fields[t.data_fields[m].field as usize].name
        )
    })
}
pub(super) fn named_measures(
    owner: &jade_xlsx_reader::Sheet,
    t: &PivotTable,
    c: &PivotCache,
) -> Result<PivotTable> {
    let mut out = t.clone();
    let mut used = BTreeSet::new();
    for d in &t.data_fields {
        if let Some(name) = d.name.as_ref().filter(|s| !s.trim().is_empty()) {
            if !used.insert(name.to_uppercase()) {
                return Err(bad("duplicate displayed pivot measure captions"));
            }
        }
    }
    let mut cells = BTreeMap::new();
    for cell in owner.rows.iter().flat_map(|r| &r.cells) {
        cells.entry(cell.reference).or_insert(cell);
    }
    let mut captions: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    let mut candidate = |m: usize, row: u32, column: u32| {
        if let Some(cell) = cells.get(&CellRef::new(column, row)) {
            if let CellValue::Text(text) = &cell.value {
                if !text.trim().is_empty() {
                    captions.entry(m).or_default().insert(text.clone());
                }
            }
        }
    };
    let origin = t.location.reference.start;
    if let Some(level) = t.column_fields.iter().position(|i| *i == -2) {
        for (column, item) in t.column_items.iter().enumerate() {
            candidate(
                item.data_index as usize,
                origin.row + t.location.first_header_row + level as u32,
                origin.column + t.location.first_data_column + column as u32,
            );
        }
    } else if let Some(level) = t
        .row_fields
        .iter()
        .position(|i| *i == -2)
        .filter(|_| t.location.first_data_column >= t.row_fields.len() as u32)
    {
        for (row, item) in t.row_items.iter().enumerate() {
            candidate(
                item.data_index as usize,
                origin.row + t.location.first_data_row + row as u32,
                origin.column + level as u32,
            );
        }
    } else if t.data_fields.len() == 1 && t.location.first_data_row > 0 {
        candidate(
            0,
            origin.row + t.location.first_data_row - 1,
            origin.column + t.location.first_data_column,
        );
    }
    for (m, d) in out.data_fields.iter_mut().enumerate() {
        if d.name.as_ref().is_some_and(|s| !s.trim().is_empty()) {
            continue;
        }
        let known = captions
            .get(&m)
            .filter(|s| s.len() == 1)
            .and_then(|s| s.first())
            .filter(|name| !used.contains(&name.to_uppercase()))
            .cloned();
        let mut name = known.unwrap_or_else(|| measure_name(t, c, m));
        let base = name.clone();
        let mut suffix = 2;
        while used.contains(&name.to_uppercase()) {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        used.insert(name.to_uppercase());
        d.name = Some(name);
    }
    Ok(out)
}
fn label(fields: &[PivotField], cache: &Rebuilt, field: usize, item: u32) -> String {
    fields[field]
        .items
        .get(item as usize)
        .and_then(|v| v.caption.clone())
        .unwrap_or_else(|| {
            fields[field]
                .items
                .get(item as usize)
                .and_then(|v| v.shared_index)
                .and_then(|i| cache.labels[field].get(i as usize))
                .cloned()
                .unwrap_or_default()
        })
}
fn label_value(fields: &[PivotField], cache: &Rebuilt, field: usize, item: u32) -> CellValue {
    let Some(member) = fields[field].items.get(item as usize) else {
        return CellValue::Empty;
    };
    if let Some(caption) = &member.caption {
        return CellValue::Text(caption.clone());
    }
    member
        .shared_index
        .and_then(|i| cache.native[field].get(i as usize))
        .cloned()
        .unwrap_or(CellValue::Empty)
}
fn newvalue(v: CellValue) -> NewValue {
    match v {
        CellValue::Empty => NewValue::Blank,
        CellValue::Number(n) => NewValue::Number(n),
        CellValue::Text(t) => NewValue::Text(t),
        CellValue::Bool(b) => NewValue::Bool(b),
        CellValue::Error(e) => NewValue::Error(e),
    }
}
pub(super) fn build(
    book: &Workbook,
    t: &PivotTable,
    c: &PivotCache,
    new: &Rebuilt,
    source: &Source,
    definition: &str,
) -> Result<Plan> {
    let original = book
        .pivots
        .tables
        .iter()
        .find(|p| p.part_name == t.part_name && p.sheet_part == t.sheet_part)
        .ok_or_else(|| bad("missing original pivot report identity"))?;
    build_report(book, t, c, new, source, definition, Some(original))
}
pub(super) fn build_new(
    book: &Workbook,
    t: &PivotTable,
    c: &PivotCache,
    new: &Rebuilt,
    source: &Source,
    definition: &str,
) -> Result<Plan> {
    build_report(book, t, c, new, source, definition, None)
}
fn build_report(
    book: &Workbook,
    t: &PivotTable,
    c: &PivotCache,
    new: &Rebuilt,
    source: &Source,
    definition: &str,
    original: Option<&PivotTable>,
) -> Result<Plan> {
    let old = original.map(|t| t.location.reference);
    let old_pages = original
        .map(|t| {
            t.page_cells()
                .ok_or_else(|| bad("ambiguous original pivot page geometry"))
        })
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .flat_map(|(a, b)| [a, b])
        .collect::<Vec<_>>();
    if !t.filters.is_empty()
        || t.unsupported.iter().any(|p| !p.ends_with("/extLst"))
        || t.data_fields.is_empty()
        || t.fields.len() != c.fields.len()
    {
        return Err(bad(
            "pivot layout/filter metadata cannot yet be rebuilt completely",
        ));
    }
    let owner = book
        .sheets
        .iter()
        .find(|s| s.part_name == t.sheet_part)
        .ok_or_else(|| bad("missing pivot owner"))?;
    if owner.protection.as_ref().is_some_and(|p| !p.pivot_tables) {
        return Err(bad(
            "pivot refresh is not allowed on the protected report sheet",
        ));
    }
    for d in &t.data_fields {
        if d.field as usize >= c.fields.len() || d.show_as != "normal" {
            return Err(bad(
                "pivot show-as/calculated measure refresh is not implemented",
            ));
        }
        Aggregate::default().value(&d.aggregation)?;
    }
    let named_table = named_measures(owner, t, c)?;
    let t = &named_table;
    for name in ["rowItems", "colItems"] {
        if let Some(c) = xml::child(definition, name)? {
            xml::flat_entries(&c.text, "i", &["r", "i", "t"], &["x"], &["v"])?;
        }
    }
    let row_axis = t.row_fields.clone();
    let mut column_axis = t.column_fields.clone();
    if !row_axis.contains(&-2) && !column_axis.contains(&-2) && t.data_fields.len() > 1 {
        column_axis.push(-2);
    }
    if row_axis.contains(&-2) && column_axis.contains(&-2) {
        return Err(bad("values field appears on both axes"));
    }
    let row_dims = dimensions(&row_axis, t.fields.len())?;
    let col_dims = dimensions(&column_axis, t.fields.len())?;
    if row_dims.iter().any(|i| col_dims.contains(i)) {
        return Err(bad("field appears on both pivot axes"));
    }
    let compact = row_dims.len() > 1 && t.location.first_data_column == 1;
    if compact && row_axis.contains(&-2) {
        return Err(bad(
            "compact values-on-rows refresh needs additional layout semantics",
        ));
    }
    if !compact
        && t.location.first_data_column
            < (row_dims.len() + usize::from(row_axis.contains(&-2))) as u32
    {
        return Err(bad("mixed compact/tabular pivot layout is not implemented"));
    }
    if let Some(raw) = xml::child(definition, "pivotFields")? {
        for (_, _, field) in xml_edit_ns::children(&raw.text, b"pivotField")? {
            if xml::child(&field.text, "autoSortScope")?.is_some() {
                return Err(bad("value-based pivot sorting is not implemented"));
            }
        }
    }
    let mut fields = t.fields.clone();
    let mut maps = Vec::new();
    for (i, field) in fields.iter_mut().enumerate() {
        let mut map = BTreeMap::new();
        for (index, item) in field
            .items
            .iter()
            .enumerate()
            .filter(|(_, v)| v.kind == "data")
        {
            let shared = item
                .shared_index
                .ok_or_else(|| bad("data item has no cache index"))?;
            if shared as usize >= new.fields[i].len() || map.insert(shared, index as u32).is_some()
            {
                return Err(bad("ambiguous pivot field item mapping"));
            }
            if item
                .attributes
                .get("sd")
                .is_some_and(|v| matches!(v.as_str(), "1" | "true"))
            {
                return Err(bad("collapsed pivot member refresh is not implemented"));
            }
        }
        for shared in &new.active[i] {
            if !map.contains_key(shared) {
                let index = field.items.len() as u32;
                field.items.push(PivotItem {
                    shared_index: Some(*shared),
                    kind: "data".into(),
                    hidden: flag(&field.attributes, "hideNewItems", false)?,
                    caption: None,
                    attributes: Default::default(),
                });
                map.insert(*shared, index);
            }
        }
        maps.push(map);
    }
    let mut pages = BTreeMap::new();
    for p in &t.page_fields {
        let i = p.field as usize;
        if i >= fields.len()
            || row_dims.contains(&i)
            || col_dims.contains(&i)
            || pages.insert(i, p.item).is_some()
            || p.hierarchy.is_some_and(|h| h >= 0)
        {
            return Err(bad("invalid or hierarchical pivot page field"));
        }
    }
    for (i, field) in fields.iter_mut().enumerate() {
        field.axis = if row_dims.contains(&i) {
            Some("axisRow".into())
        } else if col_dims.contains(&i) {
            Some("axisCol".into())
        } else if pages.contains_key(&i) {
            Some("axisPage".into())
        } else {
            None
        };
    }
    let visible_fields = row_dims
        .iter()
        .chain(&col_dims)
        .copied()
        .chain(pages.keys().copied())
        .collect::<BTreeSet<_>>();
    let mut row_paths = BTreeSet::new();
    let mut col_paths = BTreeSet::new();
    let mut accepted = Vec::new();
    for (index, record) in new.rows.iter().enumerate() {
        let hidden = visible_fields
            .iter()
            .any(|f| fields[*f].items[maps[*f][&record[*f]] as usize].hidden);
        if hidden
            || pages
                .iter()
                .any(|(f, item)| item.is_some_and(|i| i != record[*f]))
        {
            continue;
        }
        let r = row_dims
            .iter()
            .map(|f| maps[*f][&record[*f]])
            .collect::<Vec<_>>();
        let c = col_dims
            .iter()
            .map(|f| maps[*f][&record[*f]])
            .collect::<Vec<_>>();
        row_paths.insert(r.clone());
        col_paths.insert(c.clone());
        accepted.push((index, r, c));
    }
    // Row grand totals form the rightmost column; column grand totals form
    // the bottom row. An empty axis still has its implicit one-line summary.
    let ranks = member_ranks(&fields, new)?;
    let rows = axis_lines(
        &row_paths,
        &row_axis,
        &fields,
        t.data_fields.len(),
        compact,
        &ranks,
        flag(&t.attributes, "colGrandTotals", true)?,
    )?;
    let columns = axis_lines(
        &col_paths,
        &column_axis,
        &fields,
        t.data_fields.len(),
        false,
        &ranks,
        flag(&t.attributes, "rowGrandTotals", true)?,
    )?;
    let data_row = t
        .location
        .first_data_row
        .max(column_axis.len().max(1) as u32);
    let data_column = if compact {
        1
    } else {
        t.location
            .first_data_column
            .max(row_axis.len().max(1) as u32)
    };
    let height = data_row as usize + rows.len();
    let width = data_column as usize + columns.len();
    if height.saturating_mul(width) > 1_000_000 {
        return Err(bad("refreshed report exceeds cell budget"));
    }
    let origin = t.location.reference.start;
    let end = CellRef::new(
        origin
            .column
            .checked_add(width as u32 - 1)
            .ok_or_else(|| bad("pivot column overflow"))?,
        origin
            .row
            .checked_add(height as u32 - 1)
            .ok_or_else(|| bad("pivot row overflow"))?,
    );
    if end.column >= 16384 || end.row >= 1048576 {
        return Err(bad("refreshed report exceeds worksheet bounds"));
    }
    let range = CellRange { start: origin, end };
    let row_lengths = rows
        .iter()
        .filter(|l| l.kind != "blank")
        .map(|l| l.members.len())
        .collect::<BTreeSet<_>>();
    let col_lengths = columns
        .iter()
        .filter(|l| l.kind != "blank")
        .map(|l| l.members.len())
        .collect::<BTreeSet<_>>();
    if accepted
        .len()
        .saturating_mul(row_lengths.len())
        .saturating_mul(col_lengths.len())
        .saturating_mul(t.data_fields.len())
        > 5_000_000
    {
        return Err(bad("pivot aggregation exceeds work budget"));
    }
    let coordinate_work = row_lengths
        .iter()
        .sum::<usize>()
        .saturating_mul(col_lengths.len())
        .saturating_add(
            col_lengths
                .iter()
                .sum::<usize>()
                .saturating_mul(row_lengths.len()),
        );
    if accepted
        .len()
        .saturating_mul(coordinate_work)
        .saturating_mul(t.data_fields.len())
        > 5_000_000
    {
        return Err(bad(
            "pivot aggregation keys exceed the coordinate work budget",
        ));
    }
    let mut bins: BTreeMap<(Vec<u32>, Vec<u32>, usize), Aggregate> = BTreeMap::new();
    for (index, r, c) in &accepted {
        for rn in &row_lengths {
            for cn in &col_lengths {
                for (m, measure) in t.data_fields.iter().enumerate() {
                    bins.entry((r[..*rn].to_vec(), c[..*cn].to_vec(), m))
                        .or_default()
                        .add(&source.rows[*index][measure.field as usize]);
                }
            }
        }
    }
    let mut values = BTreeMap::new();
    let mut put = |row: u32, col: u32, value: CellValue| {
        values.insert(CellRef::new(origin.column + col, origin.row + row), value);
    };
    // Tabular labels preserve member captions; values-axis labels occupy
    // their actual position on either axis, independent of cache field order.
    for (i, field) in row_axis.iter().enumerate().filter(|_| !compact) {
        put(
            data_row - 1,
            i as u32,
            CellValue::Text(if *field == -2 {
                t.attributes
                    .get("dataCaption")
                    .cloned()
                    .unwrap_or("Values".into())
            } else {
                fields[*field as usize]
                    .caption
                    .clone()
                    .unwrap_or_else(|| c.fields[*field as usize].name.clone())
            }),
        );
    }
    if row_axis.is_empty() {
        put(
            data_row - 1,
            0,
            CellValue::Text(
                t.attributes
                    .get("rowHeaderCaption")
                    .cloned()
                    .unwrap_or("Grand Total".into()),
            ),
        );
    }
    let mut requested_formats = BTreeMap::new();
    let mut requested_indents = BTreeMap::new();
    let axis_format = |field: usize, item: u32| -> Option<String> {
        let shared = fields[field].items.get(item as usize)?.shared_index? as usize;
        if matches!(
            new.fields[field].get(shared)?.value,
            jade_xlsx_reader::pivot::PivotScalar::Date(_)
        ) {
            Some(
                source.formats[field]
                    .clone()
                    .filter(|c| !c.eq_ignore_ascii_case("General"))
                    .unwrap_or("yyyy-mm-dd hh:mm:ss".into()),
            )
        } else {
            source.formats[field]
                .clone()
                .filter(|c| !c.eq_ignore_ascii_case("General"))
        }
    };
    for (row, line) in rows.iter().enumerate() {
        if compact {
            let depth = line.members.len();
            if depth > 0 {
                let field = row_dims[depth - 1];
                let item = line.members[depth - 1];
                let value = if line.kind == "default" {
                    CellValue::Text(format!("{} Total", label(&fields, new, field, item)))
                } else {
                    label_value(&fields, new, field, item)
                };
                let at = CellRef::new(origin.column, origin.row + data_row + row as u32);
                if let Some(format) = axis_format(field, item) {
                    requested_formats.insert(at, format);
                }
                requested_indents.insert(at, (depth - 1) as u32);
                put(data_row + row as u32, 0, value);
            } else {
                put(
                    data_row + row as u32,
                    0,
                    CellValue::Text(
                        t.attributes
                            .get("grandTotalCaption")
                            .cloned()
                            .unwrap_or("Grand Total".into()),
                    ),
                );
                requested_indents.insert(
                    CellRef::new(origin.column, origin.row + data_row + row as u32),
                    0,
                );
            }
            continue;
        }
        let mut real = 0;
        for (column, field) in row_axis.iter().enumerate() {
            let value = if *field == -2 {
                CellValue::Text(measure_name(t, c, line.measure.unwrap_or(0)))
            } else {
                let item = line.members.get(real).copied();
                real += 1;
                match item {
                    None => CellValue::Empty,
                    Some(item) => {
                        if line.kind == "default" && real == line.members.len() {
                            CellValue::Text(format!(
                                "{} Total",
                                label(&fields, new, *field as usize, item)
                            ))
                        } else {
                            if let Some(format) = axis_format(*field as usize, item) {
                                requested_formats.insert(
                                    CellRef::new(
                                        origin.column + column as u32,
                                        origin.row + data_row + row as u32,
                                    ),
                                    format,
                                );
                            }
                            label_value(&fields, new, *field as usize, item)
                        }
                    }
                }
            };
            put(data_row + row as u32, column as u32, value);
        }
        if line.kind == "grand" {
            if let Some(column) = row_axis
                .iter()
                .position(|i| *i >= 0)
                .or_else(|| row_axis.is_empty().then_some(0))
            {
                put(
                    data_row + row as u32,
                    column as u32,
                    CellValue::Text(
                        t.attributes
                            .get("grandTotalCaption")
                            .cloned()
                            .unwrap_or("Grand Total".into()),
                    ),
                );
            }
        }
    }
    for (column, line) in columns.iter().enumerate() {
        let mut real = 0;
        for (row, field) in column_axis.iter().enumerate() {
            let value = if *field == -2 {
                CellValue::Text(measure_name(t, c, line.measure.unwrap_or(0)))
            } else {
                let item = line.members.get(real).copied();
                real += 1;
                match item {
                    None => CellValue::Empty,
                    Some(item) => {
                        if line.kind == "default" && real == line.members.len() {
                            CellValue::Text(format!(
                                "{} Total",
                                label(&fields, new, *field as usize, item)
                            ))
                        } else {
                            if let Some(format) = axis_format(*field as usize, item) {
                                requested_formats.insert(
                                    CellRef::new(
                                        origin.column + data_column + column as u32,
                                        origin.row + row as u32,
                                    ),
                                    format,
                                );
                            }
                            label_value(&fields, new, *field as usize, item)
                        }
                    }
                }
            };
            put(row as u32, data_column + column as u32, value);
        }
        if line.kind == "grand" {
            if let Some(row) = column_axis.iter().position(|i| *i >= 0) {
                put(
                    row as u32,
                    data_column + column as u32,
                    CellValue::Text(
                        t.attributes
                            .get("grandTotalCaption")
                            .cloned()
                            .unwrap_or("Grand Total".into()),
                    ),
                );
            }
        }
        if column_axis.is_empty() {
            put(
                data_row - 1,
                data_column + column as u32,
                CellValue::Text(if row_axis.contains(&-2) {
                    t.attributes
                        .get("dataCaption")
                        .cloned()
                        .unwrap_or("Values".into())
                } else {
                    measure_name(t, c, 0)
                }),
            );
        }
    }
    for (row, r) in rows.iter().enumerate() {
        for (column, c_line) in columns.iter().enumerate() {
            if matches!(r.kind.as_str(), "blank" | "header")
                || matches!(c_line.kind.as_str(), "blank" | "header")
            {
                continue;
            }
            let m = r.measure.or(c_line.measure).unwrap_or(0);
            let value = if let Some(bin) = bins.get(&(r.members.clone(), c_line.members.clone(), m))
            {
                bin.value(&t.data_fields[m].aggregation)?
            } else {
                CellValue::Empty
            };
            if let Some(id) = t.data_fields[m].number_format_id {
                if let Some(code) = book
                    .styles
                    .number_formats
                    .get(&id)
                    .map(String::as_str)
                    .or_else(|| jade_xlsx_reader::builtin_number_format(id))
                {
                    requested_formats.insert(
                        CellRef::new(
                            origin.column + data_column + column as u32,
                            origin.row + data_row + row as u32,
                        ),
                        code.into(),
                    );
                }
            }
            put(data_row + row as u32, data_column + column as u32, value);
        }
    }
    if compact {
        put(
            data_row - 1,
            0,
            CellValue::Text(
                t.attributes
                    .get("rowHeaderCaption")
                    .cloned()
                    .unwrap_or("Row Labels".into()),
            ),
        );
    }
    let positions = requested_formats
        .keys()
        .chain(requested_indents.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let formats = positions
        .into_iter()
        .filter_map(|at| {
            let row = owner
                .rows
                .binary_search_by_key(&at.row, |r| r.index)
                .ok()
                .map(|i| &owner.rows[i]);
            let style = row.and_then(|r| {
                r.cells
                    .binary_search_by_key(&at.column, |c| c.reference.column)
                    .ok()
                    .and_then(|i| r.cells[i].style_index)
            });
            let base = owner
                .effective_style_index_in_row(at, style, row)
                .unwrap_or(0);
            let current = book.styles.cell_formats.get(base as usize);
            let code = requested_formats
                .get(&at)
                .filter(|code| {
                    current.and_then(|f| book.styles.number_format_code(f)) != Some(code.as_str())
                })
                .cloned();
            let indent = requested_indents
                .get(&at)
                .copied()
                .filter(|v| current.is_none_or(|f| f.alignment.indent != *v));
            (code.is_some() || indent.is_some()).then_some((at, (base, code, indent)))
        })
        .collect();
    let page_cells = t
        .page_cells()
        .ok_or_else(|| bad("ambiguous pivot page geometry"))?;
    let mut pages = Vec::new();
    for (page, (label, value)) in t.page_fields.iter().zip(page_cells) {
        let field = page.field as usize;
        let caption = page
            .caption
            .clone()
            .or_else(|| fields[field].caption.clone())
            .unwrap_or_else(|| c.fields[field].name.clone());
        values.insert(label, CellValue::Text(caption));
        let hidden = |f: &PivotField| {
            f.items
                .iter()
                .filter(|i| i.kind == "data" && i.hidden)
                .filter_map(|i| i.shared_index)
                .collect::<BTreeSet<_>>()
        };
        let multiple = !hidden(&fields[field]).is_empty();
        let selected = if let Some(item) = page.item {
            CellValue::Text(
                fields[field]
                    .items
                    .iter()
                    .find(|i| i.shared_index == Some(item))
                    .and_then(|i| i.caption.clone())
                    .or_else(|| new.labels[field].get(item as usize).cloned())
                    .ok_or_else(|| bad("page member is outside the rebuilt cache"))?,
            )
        } else {
            let previous = original.and_then(|original| {
                original
                    .page_fields
                    .iter()
                    .position(|p| p.field == page.field && p.item.is_none())
                    .filter(|_| hidden(&original.fields[field]) == hidden(&fields[field]))
            });
            let retained = previous
                .and_then(|i| original?.page_cells()?.get(i).copied())
                .and_then(|(_, at)| {
                    owner
                        .rows
                        .iter()
                        .flat_map(|r| &r.cells)
                        .find(|c| c.reference == at)
                })
                .and_then(|c| match &c.value {
                    CellValue::Text(t) if !t.is_empty() => Some(t.clone()),
                    _ => None,
                });
            CellValue::Text(retained.unwrap_or_else(|| {
                if multiple {
                    "(Multiple Items)"
                } else {
                    "(All)"
                }
                .into()
            }))
        };
        values.insert(value, selected);
        pages.extend([label, value]);
    }
    for at in &old_pages {
        values.entry(*at).or_insert(CellValue::Empty);
    }
    // Clear only actual old cells. A sparse report can declare a very large
    // old location; materializing every empty old coordinate is unnecessary.
    for cell in owner
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .filter(|c| old.is_some_and(|old| old.contains(c.reference)))
    {
        values.entry(cell.reference).or_insert(CellValue::Empty);
    }
    if values.len() > 1_000_000 {
        return Err(bad("report edits exceed the cell budget"));
    }
    for row in range.start.row..=range.end.row {
        for column in range.start.column..=range.end.column {
            values
                .entry(CellRef::new(column, row))
                .or_insert(CellValue::Empty);
        }
    }
    if values.len() > 1_000_000 {
        return Err(bad("report edits exceed the cell budget"));
    }
    let edits = values
        .into_iter()
        .map(|(reference, value)| CellEdit {
            reference,
            value: newvalue(value),
            style_index: None,
        })
        .collect();
    Ok(Plan {
        table: t.clone(),
        old,
        range,
        fields,
        rows,
        columns,
        row_axis,
        column_axis,
        data_row,
        data_column,
        edits,
        formats,
        compact,
        old_pages,
        pages,
    })
}
pub(super) fn check_destinations(book: &Workbook, plans: &[Plan], source: &Source) -> Result<()> {
    let regions = |p: &Plan, old: bool| {
        let mut ranges = vec![p.range];
        ranges.extend(p.pages.iter().map(|at| CellRange {
            start: *at,
            end: *at,
        }));
        if old {
            ranges.extend(p.old);
            ranges.extend(p.old_pages.iter().map(|at| CellRange {
                start: *at,
                end: *at,
            }));
        }
        ranges
    };
    for p in plans {
        let destinations = regions(p, false);
        let touched = regions(p, true);
        let sheet = book
            .sheets
            .iter()
            .find(|s| s.part_name == p.table.sheet_part)
            .ok_or_else(|| bad("missing report owner"))?;
        if source.sheet_part == p.table.sheet_part
            && touched
                .iter()
                .any(|r| crate::tables::overlaps(source.range, *r))
        {
            return Err(bad("pivot report overlaps its own source"));
        }
        if sheet.rows.iter().flat_map(|r| &r.cells).any(|c| {
            destinations.iter().any(|r| r.contains(c.reference))
                && !p.old.is_some_and(|old| old.contains(c.reference))
                && !p.old_pages.contains(&c.reference)
                && (!c.value.is_empty() || c.formula.is_some())
        }) {
            return Err(bad("refreshed pivot report would overwrite existing cells"));
        }
        if sheet
            .merges
            .iter()
            .any(|r| touched.iter().any(|t| crate::tables::overlaps(*r, *t)))
            || sheet.tables.iter().any(|t| {
                touched
                    .iter()
                    .any(|r| crate::tables::overlaps(t.reference, *r))
            })
            || sheet
                .array_formulas
                .iter()
                .any(|a| touched.iter().any(|r| crate::tables::overlaps(a.range, *r)))
        {
            return Err(bad(
                "pivot report intersects a merge, Excel table or array formula",
            ));
        }
        for t in book
            .pivots
            .tables
            .iter()
            .filter(|t| t.sheet_part == p.table.sheet_part && t.part_name != p.table.part_name)
        {
            let mut other = vec![t.location.reference];
            other.extend(
                t.page_cells()
                    .ok_or_else(|| bad("ambiguous neighboring pivot page geometry"))?
                    .into_iter()
                    .flat_map(|(a, b)| [a, b])
                    .map(|at| CellRange { start: at, end: at }),
            );
            if other
                .iter()
                .any(|r| touched.iter().any(|p| crate::tables::overlaps(*r, *p)))
            {
                return Err(bad("refreshed report would overlap another pivot"));
            }
        }
        if plans.iter().any(|other| {
            other.table.sheet_part == p.table.sheet_part
                && other.table.part_name != p.table.part_name
                && regions(other, false)
                    .iter()
                    .any(|r| destinations.iter().any(|p| crate::tables::overlaps(*r, *p)))
        }) {
            return Err(bad("refreshed pivot reports would overlap"));
        }
    }
    Ok(())
}
fn axis_xml(lines: &[Line], p: &str) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            format!(
                "<{p}i t=\"{}\" i=\"{}\">{}</{p}i>",
                if l.kind == "header" { "data" } else { &l.kind },
                l.measure.unwrap_or(0),
                l.members
                    .iter()
                    .map(|v| format!("<{p}x v=\"{v}\"/>"))
                    .collect::<String>()
            )
        })
        .collect()
}
pub(super) fn write(package: &mut OpcPackage, plan: &Plan) -> Result<()> {
    let mut definition = package.required_part_str(&plan.table.part_name)?;
    let strict = xml::strict(&definition)?;
    definition = xml_edit::root_attributes(
        &definition,
        &[
            (
                "dataOnRows",
                Some(u8::from(plan.row_axis.contains(&-2)).to_string()),
            ),
            (
                "dataPosition",
                plan.row_axis
                    .iter()
                    .position(|f| *f == -2)
                    .or_else(|| plan.column_axis.iter().position(|f| *f == -2))
                    .map(|n| n.to_string()),
            ),
        ],
    )?;
    if plan.compact {
        definition = xml_edit::root_attributes(&definition, &[("compact", Some("1".into()))])?;
    }
    let loc = xml::child(&definition, "location")?.ok_or_else(|| bad("missing pivot location"))?;
    let updated = xml_edit::root_attributes(
        &loc.text,
        &[
            ("ref", Some(crate::shift::range_text(plan.range))),
            ("firstHeaderRow", Some("0".into())),
            ("firstDataRow", Some(plan.data_row.to_string())),
            ("firstDataCol", Some(plan.data_column.to_string())),
            (
                "rowPageCount",
                plan.table.location.row_page_count.map(|n| n.to_string()),
            ),
            (
                "colPageCount",
                plan.table.location.column_page_count.map(|n| n.to_string()),
            ),
        ],
    )?;
    let updated = loc.finish(&updated)?;
    definition = xml_edit_ns::set_child(&definition, b"location", |_| updated, &[])?;
    let fields =
        xml::child(&definition, "pivotFields")?.ok_or_else(|| bad("missing pivot fields"))?;
    let raw = xml_edit_ns::children(&fields.text, b"pivotField")?;
    if raw.len() != plan.fields.len() {
        return Err(bad("pivot field ownership mismatch"));
    }
    let mut splices = Vec::new();
    for (i, (start, end, c)) in raw.into_iter().enumerate() {
        let f = &plan.fields[i];
        let old_items = xml::child(&c.text, "items")?;
        let old_items = old_items
            .as_ref()
            .map(|c| xml_edit_ns::children(&c.text, b"item"))
            .transpose()?
            .unwrap_or_default();
        let head = xml_edit::root_attributes(
            &c.text,
            &[
                ("axis", f.axis.clone()),
                (
                    "dataField",
                    plan.table
                        .data_fields
                        .iter()
                        .any(|d| d.field as usize == i)
                        .then(|| "1".into()),
                ),
            ],
        )?;
        let updated = xml::list(&head, "items", "item", |prefix| {
            f.items
                .iter()
                .enumerate()
                .map(|(index, v)| {
                    if let Some((_, _, old)) = old_items.get(index) {
                        let updated = xml_edit::root_attributes(
                            &old.text,
                            &[
                                ("x", v.shared_index.map(|i| i.to_string())),
                                ("h", v.hidden.then(|| "1".into())),
                            ],
                        )?;
                        old.finish(&updated)
                    } else {
                        Ok(format!(
                            "<{prefix}item t=\"{}\"{}{} />",
                            v.kind,
                            v.shared_index
                                .map_or(String::new(), |x| format!(" x=\"{x}\"")),
                            if v.hidden { " h=\"1\"" } else { "" }
                        ))
                    }
                })
                .collect()
        })?;
        splices.push(Splice {
            start,
            end,
            text: c.finish(&updated)?,
        });
    }
    let updated = fields.finish(&apply(&fields.text, splices))?;
    definition = xml_edit_ns::set_child(&definition, b"pivotFields", |_| updated, &[])?;
    for (container, axis) in [
        ("rowFields", &plan.row_axis),
        ("colFields", &plan.column_axis),
    ] {
        definition = xml::list(&definition, container, "field", |p| {
            Ok(axis
                .iter()
                .map(|i| format!("<{p}field x=\"{i}\"/>"))
                .collect())
        })?;
    }
    definition = xml::list(&definition, "rowItems", "i", |p| {
        Ok(axis_xml(&plan.rows, p))
    })?;
    definition = xml::list(&definition, "colItems", "i", |p| {
        Ok(axis_xml(&plan.columns, p))
    })?;
    if let Some(data) = xml::child(&definition, "dataFields")? {
        let entries = xml_edit_ns::children(&data.text, b"dataField")?;
        if entries.len() != plan.table.data_fields.len() {
            return Err(bad("pivot measure ownership mismatch"));
        }
        let mut updates = Vec::new();
        for (i, (start, end, c)) in entries.into_iter().enumerate() {
            let text = xml_edit::root_attributes(
                &c.text,
                &[(
                    "name",
                    plan.table.data_fields[i]
                        .name
                        .as_deref()
                        .map(xml::xstring_attribute),
                )],
            )?;
            updates.push(Splice {
                start,
                end,
                text: c.finish(&text)?,
            });
        }
        let updated = data.finish(&apply(&data.text, updates))?;
        definition = xml_edit_ns::set_child(&definition, b"dataFields", |_| updated, &[])?;
    }

    package.replace(&plan.table.part_name, definition.into_bytes());
    let mut edits = plan.edits.clone();
    if !plan.formats.is_empty() {
        let part = crate::styles_part_name(package)?;
        let mut styles = package
            .part_str(&part)?
            .unwrap_or_else(|| crate::styles::minimal_styles(strict));
        let mut mapped = BTreeMap::new();
        for edit in &mut edits {
            if let Some((base, code, indent)) = plan.formats.get(&edit.reference) {
                let key = (*base, code.clone(), *indent);
                let index = if let Some(index) = mapped.get(&key) {
                    *index
                } else {
                    let (updated, index) = crate::add_cell_format(
                        &styles,
                        Some(*base),
                        &crate::FormatChange {
                            number_format: code.clone(),
                            horizontal: indent.map(|_| jade_xlsx_reader::HorizontalAlignment::Left),
                            ..Default::default()
                        },
                    )?;
                    styles = if let Some(indent) = indent {
                        format_indent(&updated, index, *indent)?
                    } else {
                        updated
                    };
                    mapped.insert(key, index);
                    index
                };
                edit.style_index = Some(index);
            }
        }
        crate::styles::install_styles_relationship(package, &part)?;
        package.replace(&part, styles.into_bytes());
    }
    let sheet = package.required_part_str(&plan.table.sheet_part)?;
    package.replace(
        &plan.table.sheet_part,
        crate::patch_sheet(&sheet, &edits)?.into_bytes(),
    );
    Ok(())
}

fn format_indent(styles: &str, index: u32, indent: u32) -> Result<String> {
    let formats = xml::child(styles, "cellXfs")?.ok_or(crate::WriteError::NoStyleTables)?;
    let all = xml_edit_ns::children(&formats.text, b"xf")?;
    let (start, end, c) = all
        .get(index as usize)
        .ok_or(crate::WriteError::NoSuchStyle { index })?;
    let xf = xml_edit_ns::edit_child(&c.text, b"alignment", &[b"protection", b"extLst"], |a| {
        xml_edit::root_attributes(a, &[("indent", Some(indent.to_string()))])
    })?;
    let child = c.finish(&xf)?;
    let updated = formats.finish(&apply(
        &formats.text,
        vec![Splice {
            start: *start,
            end: *end,
            text: child,
        }],
    ))?;
    xml_edit_ns::set_child(styles, b"cellXfs", |_| updated, &[])
}
