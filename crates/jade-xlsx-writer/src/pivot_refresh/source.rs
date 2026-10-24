use super::{bad, Result, VerifiedPivotCells};
use jade_xlsx_reader::{pivot::PivotCache, CellRange, CellRef, CellValue, Sheet, Workbook};
use std::collections::BTreeMap;
pub(super) struct Source {
    pub sheet_name: String,
    pub sheet_part: String,
    pub range: CellRange,
    pub rows: Vec<Vec<CellValue>>,
    pub dates: Vec<Vec<bool>>,
    pub formats: Vec<Option<String>>,
    pub date_system: jade_xlsx_reader::DateSystem,
}
fn sheet<'a>(book: &'a Workbook, name: &str) -> Result<&'a Sheet> {
    let found = book
        .sheets
        .iter()
        .filter(|s| jade_xlsx_reader::formula::same_sheet_name(&s.name, name))
        .collect::<Vec<_>>();
    if found.len() != 1 {
        return Err(bad("pivot source worksheet is missing or ambiguous"));
    }
    Ok(found[0])
}
fn address(
    book: &Workbook,
    text: &str,
    fallback: Option<&str>,
    visited: &mut Vec<String>,
) -> Result<(String, CellRange)> {
    let text = text.trim().strip_prefix('=').unwrap_or(text.trim());
    if visited.len() >= 64 || visited.iter().any(|s| s.eq_ignore_ascii_case(text)) {
        return Err(bad("circular or excessively nested pivot source name"));
    }
    if let Some((name, at)) = text.rsplit_once('!') {
        let name = jade_xlsx_reader::formula::unquote_sheet_name(name)
            .ok_or_else(|| bad("invalid quoted pivot source worksheet"))?;
        let s = sheet(book, &name)?;
        let range = CellRange::parse(at)
            .map_err(|_| bad("pivot source name is not a single worksheet range"))?;
        return Ok((s.name.clone(), range));
    }
    let tables = book
        .sheets
        .iter()
        .flat_map(|s| {
            s.tables
                .iter()
                .filter(move |t| {
                    t.name.eq_ignore_ascii_case(text) || t.display_name.eq_ignore_ascii_case(text)
                })
                .map(move |t| (s, t))
        })
        .collect::<Vec<_>>();
    if tables.len() == 1 {
        let (s, t) = tables[0];
        if t.header_row_count != 1 {
            return Err(bad("pivot source table needs one header row"));
        }
        let mut r = t.reference;
        r.end.row = r
            .end
            .row
            .checked_sub(t.totals_row_count)
            .ok_or_else(|| bad("invalid pivot source table totals"))?;
        return Ok((s.name.clone(), r));
    } else if tables.len() > 1 {
        return Err(bad("ambiguous pivot source table"));
    }
    let local = fallback.and_then(|name| {
        book.sheets
            .iter()
            .position(|s| jade_xlsx_reader::formula::same_sheet_name(&s.name, name))
    });
    let named = |scope| {
        book.defined_names
            .iter()
            .filter(move |n| n.name.eq_ignore_ascii_case(text) && n.local_sheet_index == scope)
            .collect::<Vec<_>>()
    };
    let mut names = if local.is_some() {
        named(local)
    } else {
        Vec::new()
    };
    if names.is_empty() {
        names = named(None);
    }
    if names.len() == 1 {
        visited.push(text.into());
        let result = address(book, &names[0].formula, fallback, visited);
        visited.pop();
        return result;
    }
    if names.len() > 1 {
        return Err(bad("ambiguous pivot source defined name"));
    }
    if let (Some(name), Ok(range)) = (fallback, CellRange::parse(text)) {
        return Ok((sheet(book, name)?.name.clone(), range));
    }
    Err(bad(
        "pivot source name is not an interpreted table or worksheet range",
    ))
}
pub(super) fn location<'a>(
    book: &'a Workbook,
    cache: &PivotCache,
) -> Result<(&'a Sheet, CellRange)> {
    if cache.source.kind != "worksheet" || cache.source.relationship_id.is_some() {
        return Err(bad("pivot refresh requires an internal worksheet source"));
    }
    let (name, range) = if let Some(name) = &cache.source.defined_name {
        address(book, name, cache.source.sheet.as_deref(), &mut Vec::new())?
    } else {
        (
            cache
                .source
                .sheet
                .clone()
                .ok_or_else(|| bad("pivot source has no worksheet"))?,
            cache
                .source
                .reference
                .ok_or_else(|| bad("pivot source has no range"))?,
        )
    };
    let sheet = sheet(book, &name)?;
    Ok((sheet, range))
}
pub(super) fn read(
    book: &Workbook,
    cache: &PivotCache,
    verified: &VerifiedPivotCells,
) -> Result<Source> {
    if cache.source.kind != "worksheet"
        || cache.source.relationship_id.is_some()
        || cache.fields.iter().any(|f| {
            f.grouping.is_some()
                || f.attributes.contains_key("formula")
                || f.attributes
                    .get("databaseField")
                    .is_some_and(|v| matches!(v.as_str(), "0" | "false"))
        })
        || cache.unsupported.iter().any(|p| !p.ends_with("/extLst"))
    {
        return Err(bad(
            "refresh of grouped, calculated, external or uninterpreted caches is not implemented",
        ));
    }
    let (sheet, range) = location(book, cache)?;
    if range.start.row > range.end.row || range.start.column > range.end.column {
        return Err(bad("inverted pivot source"));
    }
    let width = (range.end.column - range.start.column + 1) as usize;
    let height = (range.end.row - range.start.row) as usize;
    if width != cache.fields.len() {
        return Err(bad(
            "pivot source columns changed; field remapping is required",
        ));
    }
    if height.saturating_add(1).saturating_mul(width) > 1_000_000 {
        return Err(bad("pivot source exceeds the refresh cell budget"));
    }
    let mut cells = BTreeMap::new();
    let mut source_rows = BTreeMap::new();
    for row in &sheet.rows {
        if row.index >= range.start.row
            && row.index <= range.end.row
            && source_rows.insert(row.index, row).is_some()
        {
            return Err(bad("ambiguous pivot source row metadata"));
        }
        for cell in row.cells.iter().filter(|c| range.contains(c.reference)) {
            if cells.insert(cell.reference, cell).is_some() {
                return Err(bad("duplicate pivot source cell"));
            }
        }
    }
    let value = |at| -> Result<CellValue> {
        if let Some(c) = cells.get(&at) {
            if c.formula.is_some()
                && !verified
                    .get(&sheet.part_name)
                    .is_some_and(|v| v.contains(&at))
            {
                return Err(bad("pivot source formula has no verified current result"));
            }
            if matches!(c.value, CellValue::Number(n) if !n.is_finite()) {
                return Err(bad("non-finite pivot source value"));
            }
            Ok(c.value.clone())
        } else {
            Ok(CellValue::Empty)
        }
    };
    let mut field_names = std::collections::BTreeSet::new();
    for (i, field) in cache.fields.iter().enumerate() {
        let header = value(CellRef::new(range.start.column + i as u32, range.start.row))?;
        if !matches!(header, CellValue::Text(ref name) if name.eq_ignore_ascii_case(&field.name)) {
            return Err(bad(
                "pivot source headers no longer match their cache fields",
            ));
        }
        if !field_names.insert(field.name.to_uppercase()) {
            return Err(bad("duplicate pivot source field names"));
        }
    }
    let mut rows = Vec::with_capacity(height);
    let mut formats = vec![None; width];
    let mut dates = Vec::with_capacity(height);
    let date_defaults = cache
        .fields
        .iter()
        .map(|f| {
            f.shared_items
                .iter()
                .any(|v| matches!(v.value, jade_xlsx_reader::pivot::PivotScalar::Date(_)))
                && !f
                    .shared_items
                    .iter()
                    .any(|v| matches!(v.value, jade_xlsx_reader::pivot::PivotScalar::Number(_)))
        })
        .collect::<Vec<_>>();
    for row in range.start.row + 1..=range.end.row {
        let row_metadata = source_rows.get(&row).copied();
        let mut values = Vec::with_capacity(width);
        let mut date_row = Vec::with_capacity(width);
        for (i, format) in formats.iter_mut().enumerate() {
            let at = CellRef::new(range.start.column + i as u32, row);
            let style = sheet
                .effective_style_index_in_row(
                    at,
                    cells.get(&at).and_then(|c| c.style_index),
                    row_metadata,
                )
                .unwrap_or(0) as usize;
            let code = book
                .styles
                .cell_formats
                .get(style)
                .and_then(|f| book.styles.number_format_code(f));
            if format.is_none() {
                *format = code.map(str::to_string);
            }
            date_row.push(
                code.is_some_and(jade_xlsx_layout::number_format::is_date_format)
                    || date_defaults[i],
            );
            values.push(value(at)?);
        }
        rows.push(values);
        dates.push(date_row);
    }
    Ok(Source {
        sheet_name: sheet.name.clone(),
        sheet_part: sheet.part_name.clone(),
        range,
        rows,
        dates,
        formats,
        date_system: book.date_system,
    })
}
