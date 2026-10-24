use crate::{Result, WriteError};
use jade_xlsx_reader::{
    pivot::{AxisItem, PivotCache, PivotScalar, PivotTable},
    Cell, CellRef, CellValue, Sheet, Workbook,
};
use std::collections::{BTreeMap, BTreeSet};
type Key = Vec<(u32, u32)>;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SeriesKey {
    pub members: Key,
    pub measure: u32,
}
pub(super) struct Category {
    pub key: Key,
    pub label: String,
    pub reference: Option<CellRef>,
}
pub(super) struct Series {
    pub key: SeriesKey,
    pub name: String,
    pub values: Vec<Option<String>>,
    pub format: String,
    pub references: Vec<CellRef>,
    pub name_reference: Option<CellRef>,
}
pub(super) struct Projection {
    pub categories: Vec<Category>,
    pub series: Vec<Series>,
}
struct Line {
    members: Vec<(usize, u32)>,
    measure: Option<u32>,
    kind: String,
}
fn bad(s: &str) -> WriteError {
    WriteError::Pivot(format!("pivot chart: {s}"))
}
fn axes(
    items: &[AxisItem],
    axis: &[i32],
    table: &PivotTable,
    expected: usize,
) -> Result<Vec<Line>> {
    if items.is_empty() {
        return if expected == 1 && axis.iter().all(|f| *f < 0) {
            Ok(vec![Line {
                members: vec![],
                measure: None,
                kind: "grand".into(),
            }])
        } else if expected == 0 {
            Ok(vec![])
        } else {
            Err(bad("missing report axis"))
        };
    }
    if items.len() != expected {
        return Err(bad("report axis and data bounds disagree"));
    }
    let real = axis
        .iter()
        .filter(|f| **f >= 0)
        .copied()
        .collect::<Vec<_>>();
    let values = axis.contains(&-2);
    let mut previous = Vec::new();
    let mut out = Vec::new();
    for item in items {
        let mut raw = previous
            .get(..item.repeat as usize)
            .ok_or_else(|| bad("invalid axis prefix"))?
            .to_vec();
        raw.extend(&item.items);
        let measure = values.then_some(item.data_index);
        if measure.is_some_and(|m| m as usize >= table.data_fields.len()) {
            return Err(bad("invalid measure index"));
        }
        let positions = if values && raw.len() == real.len() {
            &real[..]
        } else {
            axis
        };
        if raw.len() > positions.len() {
            return Err(bad("excess axis coordinates"));
        }
        let mut members = Vec::new();
        if !matches!(item.kind.as_str(), "grand" | "blank") {
            for (field, index) in positions.iter().zip(&raw).filter(|(f, _)| **f >= 0) {
                let field = *field as usize;
                let member = table
                    .fields
                    .get(field)
                    .and_then(|f| f.items.get(*index as usize))
                    .ok_or_else(|| bad("axis item outside field"))?;
                if member.hidden || member.kind != "data" || member.shared_index.is_none() {
                    return Err(bad("uninterpreted visible member"));
                }
                members.push((field, *index));
            }
        }
        out.push(Line {
            members,
            measure,
            kind: item.kind.clone(),
        });
        previous = raw;
    }
    Ok(out)
}
fn key(line: &Line, t: &PivotTable) -> Result<Key> {
    let mut key = line
        .members
        .iter()
        .map(|(f, i)| {
            Ok((
                *f as u32,
                t.fields[*f].items[*i as usize]
                    .shared_index
                    .ok_or_else(|| bad("member without cache identity"))?,
            ))
        })
        .collect::<Result<Key>>()?;
    key.sort_unstable();
    Ok(key)
}
fn leaf(line: &Line, dimensions: usize) -> bool {
    if dimensions == 0 {
        line.kind != "blank" && line.members.is_empty()
    } else {
        line.kind == "data" && line.members.len() == dimensions
    }
}
fn format_cell(book: &Workbook, sheet: &Sheet, cell: &Cell) -> String {
    let code = book
        .styles
        .cell_format(sheet.effective_style_index(cell.reference, cell.style_index))
        .and_then(|f| book.styles.number_format_code(f));
    jade_xlsx_layout::number_format::format_value(&cell.value, code, book.date_system).text
}
fn cache_label(cache: &PivotCache, t: &PivotTable, field: usize, item: u32) -> Result<String> {
    let item = &t.fields[field].items[item as usize];
    if let Some(c) = &item.caption {
        return Ok(c.clone());
    }
    let value = cache
        .fields
        .get(field)
        .and_then(|f| f.shared_items.get(item.shared_index? as usize));
    let value = value.ok_or_else(|| bad("member outside cache"))?;
    if let Some(c) = &value.caption {
        return Ok(c.clone());
    }
    Ok(match &value.value {
        PivotScalar::Text(s) | PivotScalar::Error(s) | PivotScalar::Date(s) => s.clone(),
        PivotScalar::Missing => "(blank)".into(),
        PivotScalar::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        PivotScalar::Number(n) if n.is_finite() => n.to_string(),
        _ => return Err(bad("uninterpreted member label")),
    })
}
pub(super) fn project(book: &Workbook, t: &PivotTable) -> Result<Projection> {
    let sheet = book
        .sheets
        .iter()
        .find(|s| s.part_name == t.sheet_part)
        .ok_or_else(|| bad("missing report sheet"))?;
    let caches = book
        .pivots
        .caches
        .iter()
        .filter(|c| c.id == t.cache_id)
        .collect::<Vec<_>>();
    if caches.len() != 1 {
        return Err(bad("ambiguous source cache"));
    }
    let cache = caches[0];
    if t.data_fields.is_empty() || (t.row_fields.contains(&-2) && t.column_fields.contains(&-2)) {
        return Err(bad("invalid values axis"));
    }
    let mut used = BTreeSet::new();
    for f in t
        .row_fields
        .iter()
        .chain(&t.column_fields)
        .filter(|f| **f >= 0)
    {
        if !used.insert(*f) || *f as usize >= t.fields.len() {
            return Err(bad("duplicate or invalid dimension"));
        }
    }
    let r = t.location.reference;
    let height = r
        .end
        .row
        .checked_sub(r.start.row)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| bad("inverted report bounds"))?;
    let width = r
        .end
        .column
        .checked_sub(r.start.column)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| bad("inverted report bounds"))?;
    if u64::from(height) * u64::from(width) > 1_000_000 {
        return Err(bad("projection exceeds report budget"));
    }
    let data_row = r
        .start
        .row
        .checked_add(t.location.first_data_row)
        .ok_or_else(|| bad("row overflow"))?;
    let data_col = r
        .start
        .column
        .checked_add(t.location.first_data_column)
        .ok_or_else(|| bad("column overflow"))?;
    let rows = axes(
        &t.row_items,
        &t.row_fields,
        t,
        height
            .checked_sub(t.location.first_data_row)
            .ok_or_else(|| bad("invalid row offset"))? as usize,
    )?;
    let cols = axes(
        &t.column_items,
        &t.column_fields,
        t,
        width
            .checked_sub(t.location.first_data_column)
            .ok_or_else(|| bad("invalid column offset"))? as usize,
    )?;
    let row_dims = t.row_fields.iter().filter(|f| **f >= 0).count();
    let col_dims = t.column_fields.iter().filter(|f| **f >= 0).count();
    if t.data_fields.len() > 1 && !t.row_fields.contains(&-2) && !t.column_fields.contains(&-2) {
        return Err(bad("multiple measures without a values axis"));
    }
    let cells = sheet
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .map(|c| (c.reference, c))
        .collect::<BTreeMap<_, _>>();
    let compact = t.location.first_data_column == 1 && row_dims > 1;
    let mut categories = Vec::<Category>::new();
    let mut category_index = BTreeMap::new();
    let mut row_slots = Vec::new();
    for (i, line) in rows.iter().enumerate().filter(|(_, l)| leaf(l, row_dims)) {
        let k = key(line, t)?;
        let index = if let Some(index) = category_index.get(&k) {
            *index
        } else {
            let mut labels = Vec::new();
            let mut reference = None;
            for (depth, (field, item)) in line.members.iter().enumerate() {
                let pos = t
                    .row_fields
                    .iter()
                    .position(|f| *f == *field as i32)
                    .ok_or_else(|| bad("missing row dimension"))?;
                let at = CellRef::new(
                    if compact {
                        r.start.column
                    } else {
                        r.start.column + pos as u32
                    },
                    data_row + i as u32,
                );
                let actual = (!compact || depth + 1 == line.members.len())
                    .then(|| cells.get(&at))
                    .flatten()
                    .filter(|c| !c.value.is_empty());
                if line.members.len() == 1 && actual.is_some() {
                    reference = Some(at);
                }
                labels.push(if let Some(cell) = actual {
                    format_cell(book, sheet, cell)
                } else {
                    cache_label(cache, t, *field, *item)?
                });
            }
            if labels.is_empty() {
                labels.push(
                    t.attributes
                        .get("grandTotalCaption")
                        .cloned()
                        .unwrap_or("Grand Total".into()),
                );
            }
            let index = categories.len();
            category_index.insert(k.clone(), index);
            categories.push(Category {
                key: k,
                label: labels.join(" / "),
                reference,
            });
            index
        };
        row_slots.push((i, line, index));
    }
    if categories.len() > jade_xlsx_reader::chart::MAX_CHART_POINTS {
        return Err(bad("category point limit"));
    }
    // Charge the full matrix before allocating any per-series value vectors,
    // including malformed saved reports that omit some measure rows.
    let series_count = cols
        .iter()
        .filter(|l| leaf(l, col_dims))
        .count()
        .checked_mul(if t.row_fields.contains(&-2) {
            t.data_fields.len()
        } else {
            1
        })
        .ok_or_else(|| bad("projected chart value budget"))?;
    if series_count
        .checked_mul(categories.len().max(1))
        .is_none_or(|n| n > 1_000_000)
    {
        return Err(bad("projected chart value budget"));
    }
    // Values-on-rows can have many measures. Index once instead of scanning
    // every physical report row again for every measure.
    let mut slots_by_measure = BTreeMap::<_, Vec<_>>::new();
    for slot in &row_slots {
        slots_by_measure
            .entry(slot.1.measure)
            .or_default()
            .push(slot);
    }
    let mut series = Vec::<Series>::new();
    let mut series_index = BTreeMap::new();
    for (j, line) in cols.iter().enumerate().filter(|(_, l)| leaf(l, col_dims)) {
        let measures = if t.row_fields.contains(&-2) {
            (0..t.data_fields.len() as u32).collect::<Vec<_>>()
        } else {
            vec![line.measure.unwrap_or(0)]
        };
        for measure in measures {
            let k = SeriesKey {
                members: key(line, t)?,
                measure,
            };
            if series_index.contains_key(&k) {
                return Err(bad("duplicate series coordinates"));
            }
            let mut names = line
                .members
                .iter()
                .map(|(f, i)| cache_label(cache, t, *f, *i))
                .collect::<Result<Vec<_>>>()?;
            if names.is_empty() || t.data_fields.len() > 1 {
                names.push(
                    t.data_fields[measure as usize]
                        .name
                        .clone()
                        .or_else(|| {
                            cache
                                .fields
                                .get(t.data_fields[measure as usize].field as usize)
                                .map(|f| f.name.clone())
                        })
                        .ok_or_else(|| bad("missing measure caption"))?,
                );
            }
            let mut values = vec![None; categories.len()];
            let mut written = BTreeSet::new();
            let mut references = Vec::new();
            let mut format = "General".to_string();
            for (i, _, slot) in slots_by_measure
                .get(&Some(measure))
                .into_iter()
                .flatten()
                .chain(slots_by_measure.get(&None).into_iter().flatten())
                .copied()
            {
                if !written.insert(*slot) {
                    return Err(bad("duplicate category/measure cell"));
                }
                let at = CellRef::new(data_col + j as u32, data_row + *i as u32);
                references.push(at);
                if let Some(cell) = cells.get(&at) {
                    if format == "General" {
                        format = book
                            .styles
                            .cell_format(sheet.effective_style_index(at, cell.style_index))
                            .and_then(|f| book.styles.number_format_code(f))
                            .unwrap_or("General")
                            .into();
                    }
                    values[*slot] = match &cell.value {
                        CellValue::Empty => None,
                        CellValue::Number(n) if n.is_finite() => Some(n.to_string()),
                        CellValue::Error(e) => Some(e.clone()),
                        CellValue::Bool(b) => Some(if *b { "1" } else { "0" }.into()),
                        _ => return Err(bad("uninterpreted report data value")),
                    };
                }
            }
            let name = names.join(" / ");
            let header = data_row
                .checked_sub(1)
                .map(|row| CellRef::new(data_col + j as u32, row));
            let name_reference = header.filter(|at| {
                cells
                    .get(at)
                    .is_some_and(|c| format_cell(book, sheet, c) == name)
            });
            series_index.insert(k.clone(), series.len());
            series.push(Series {
                key: k,
                name,
                values,
                format,
                references,
                name_reference,
            });
        }
    }
    if series.len().saturating_mul(categories.len()) > 1_000_000 {
        return Err(bad("projected chart value budget"));
    }
    Ok(Projection { categories, series })
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &[u8] = include_bytes!("../../../../public/fixtures/pivot-chart-compat.xlsx");
    fn rebuilt(edit: impl FnOnce(String) -> String) -> Workbook {
        let mut p = jade_opc::OpcPackage::open(FIXTURE).unwrap();
        let part = "xl/pivotTables/report.xml";
        let xml = edit(p.required_part_str(part).unwrap());
        p.replace(part, xml.into_bytes());
        let part = "xl/charts/pivot.xml";
        let xml = p.required_part_str(part).unwrap();
        let start = xml.find("<c:pivotSource>").unwrap();
        let end = xml.find("</c:pivotSource>").unwrap() + "</c:pivotSource>".len();
        let xml = format!("{}{}", &xml[..start], &xml[end..]);
        p.replace(part, xml.into_bytes());
        let result = crate::pivot_refresh::refresh_pivot(
            &p.write().unwrap(),
            "Report",
            "RegionalSales",
            &crate::pivot_refresh::VerifiedPivotCells::new(),
        )
        .unwrap();
        jade_xlsx_reader::read_xlsx(&result.bytes).unwrap()
    }
    fn count(s: String) -> String {
        s.replace("<dataFields count=\"1\">", "<dataFields count=\"2\">")
            .replace(
                "</dataFields>",
                "<dataField name=\"Count Amount\" fld=\"1\" subtotal=\"count\"/></dataFields>",
            )
    }
    #[test]
    fn compact_groups_project_leaf_values_without_repeating_subtotals_or_the_grand_total() {
        let b = rebuilt(|s| {
            s.replace(
                "<rowFields count=\"1\"><field x=\"0\"/></rowFields>",
                "<rowFields count=\"2\"><field x=\"0\"/><field x=\"2\"/></rowFields>",
            )
            .replace(
                "<pageFields count=\"1\"><pageField fld=\"2\"/></pageFields>",
                "<pageFields count=\"0\"/>",
            )
        });
        let p = project(&b, &b.pivots.tables[0]).unwrap();
        assert_eq!(
            p.categories
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            ["East / 2026-10-24 12:30:00", "West / (blank)"]
        );
        assert_eq!(p.series.len(), 1);
        assert_eq!(p.series[0].values, [Some("15".into()), Some("27".into())]);
    }
    #[test]
    fn values_on_rows_produce_separate_series_over_the_same_real_categories() {
        let b = rebuilt(|s| {
            count(s)
                .replace(
                    "<rowFields count=\"1\"><field x=\"0\"/></rowFields>",
                    "<rowFields count=\"2\"><field x=\"0\"/><field x=\"-2\"/></rowFields>",
                )
                .replace(
                    "<colFields count=\"1\"><field x=\"-2\"/></colFields>",
                    "<colFields count=\"0\"/>",
                )
                .replace("firstDataCol=\"1\"", "firstDataCol=\"2\"")
                .replace("E3:F6", "E3:G6")
        });
        let p = project(&b, &b.pivots.tables[0]).unwrap();
        assert_eq!(p.categories.len(), 2);
        assert_eq!(p.series.len(), 2);
        assert_eq!(p.series[0].values, [Some("15".into()), Some("27".into())]);
        assert_eq!(p.series[1].values, [Some("2".into()), Some("2".into())]);
    }
    #[test]
    fn column_members_and_measures_keep_sparse_intersections_and_exclude_total_columns() {
        let b = rebuilt(|s| {
            count(s)
                .replace(
                    "<colFields count=\"1\"><field x=\"-2\"/></colFields>",
                    "<colFields count=\"2\"><field x=\"2\"/><field x=\"-2\"/></colFields>",
                )
                .replace(
                    "<pageFields count=\"1\"><pageField fld=\"2\"/></pageFields>",
                    "<pageFields count=\"0\"/>",
                )
        });
        let p = project(&b, &b.pivots.tables[0]).unwrap();
        assert_eq!(p.categories.len(), 2);
        assert_eq!(p.series.len(), 4);
        assert_eq!(p.series[0].values, [Some("15".into()), None]);
        assert_eq!(p.series[1].values, [Some("2".into()), None]);
        assert_eq!(p.series[2].values, [None, Some("27".into())]);
        assert_eq!(p.series[3].values, [None, Some("2".into())]);
    }
}
