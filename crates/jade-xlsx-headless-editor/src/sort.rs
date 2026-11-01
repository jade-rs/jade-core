use super::*;
use icu_collator::{
    options::{CaseLevel, CollatorOptions, Strength},
    preferences::{CollationCaseFirst, CollationNumericOrdering},
    CollatorBorrowed, CollatorPreferences,
};
use icu_locale::Locale;
use jade_xlsx_headless_api::{SortRequest, SortSummary};
use jade_xlsx_writer::sort::{intersects, SORT_CELL_LIMIT, SORT_ROW_LIMIT};
use std::cmp::Ordering;
use std::collections::BTreeSet;

#[derive(Clone)]
struct AsciiKey {
    weights: Vec<u16>,
    hyphen: bool,
}
fn ascii_key(text: &str, case_sensitive: bool) -> Option<AsciiKey> {
    const ORDER: &[u8] = b"0123456789 !\"#$%&()*,./:;?@[\\]^_`{|}~+<=>ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut weights = vec![];
    let mut hyphen = false;
    for byte in text.bytes() {
        if byte == b'\'' {
            continue;
        }
        if byte == b'-' {
            hyphen = true;
            continue;
        }
        let rank = ORDER
            .iter()
            .position(|item| *item == byte.to_ascii_uppercase())? as u16;
        weights.push(rank * 2 + u16::from(case_sensitive && byte.is_ascii_uppercase()));
    }
    Some(AsciiKey { weights, hyphen })
}

fn error(message: impl Into<String>) -> HeadlessError {
    jade_xlsx_writer::WriteError::Sort(message.into()).into()
}
#[derive(Clone)]
enum SortValue {
    Number(f64),
    Text {
        value: String,
        ascii: Option<AsciiKey>,
    },
    Bool(bool),
    Error,
    Blank,
}
impl SortValue {
    fn new(value: Option<&CellValue>, reading: Option<&str>, case_sensitive: bool) -> Self {
        match value {
            Some(CellValue::Number(value)) => Self::Number(*value),
            Some(CellValue::Text(value)) if !value.is_empty() => {
                let value = reading.unwrap_or(value).to_string();
                let ascii = ascii_key(&value, case_sensitive);
                Self::Text { value, ascii }
            }
            Some(CellValue::Bool(value)) => Self::Bool(*value),
            Some(CellValue::Error(_)) => Self::Error,
            _ => Self::Blank,
        }
    }
    fn rank(&self) -> u8 {
        match self {
            Self::Number(_) => 0,
            Self::Text { .. } => 1,
            Self::Bool(_) => 2,
            Self::Error => 3,
            Self::Blank => 4,
        }
    }
    fn compare(
        &self,
        other: &Self,
        descending: bool,
        ascii: bool,
        collator: &CollatorBorrowed<'_>,
    ) -> Ordering {
        if matches!(self, Self::Blank) || matches!(other, Self::Blank) {
            return self.rank().cmp(&other.rank());
        }
        let order = match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (
                Self::Text {
                    value: a,
                    ascii: ka,
                },
                Self::Text {
                    value: b,
                    ascii: kb,
                },
            ) => {
                if ascii {
                    let a = ka.as_ref().expect("ASCII column was validated");
                    let b = kb.as_ref().expect("ASCII column was validated");
                    a.weights
                        .cmp(&b.weights)
                        .then_with(|| a.hyphen.cmp(&b.hyphen))
                } else {
                    collator.compare(a, b)
                }
            }
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        };
        if descending {
            order.reverse()
        } else {
            order
        }
    }
}
impl HeadlessWorkbook {
    pub fn sort_columns(
        &self,
        sheet_name: &str,
        reference: &str,
        header: bool,
    ) -> Result<Vec<jade_xlsx_headless_api::AutoFilterColumnOption>> {
        self.sheet_index(sheet_name)?;
        let range = CellRange::parse(reference)
            .map_err(|_| HeadlessError::BadReference(reference.into()))?;
        if range.start.column > range.end.column || range.start.row > range.end.row {
            return Err(error("invalid sort range"));
        }
        (range.start.column..=range.end.column)
            .map(|column| {
                let name = column_label(column);
                let label = if header {
                    self.cell(
                        sheet_name,
                        &format_reference(CellRef::new(column, range.start.row)),
                    )?
                    .display
                } else {
                    String::new()
                };
                Ok(jade_xlsx_headless_api::AutoFilterColumnOption {
                    column: column - range.start.column,
                    worksheet_column: column,
                    label: if label.is_empty() {
                        name
                    } else {
                        format!("{label} ({name})")
                    },
                })
            })
            .collect()
    }
    /// Sort complete records by one or more value columns in one history step.
    pub fn sort_range(&mut self, sheet_name: &str, request: &SortRequest) -> Result<SortSummary> {
        let index = self.sheet_index(sheet_name)?;
        let canonical_name = self.workbook.sheets[index].name.clone();
        let sheet_name = canonical_name.as_str();
        let sheet = &self.workbook.sheets[index];
        let full = CellRange::parse(&request.reference)
            .map_err(|_| HeadlessError::BadReference(request.reference.clone()))?;
        if full.start.row > full.end.row || full.start.column > full.end.column {
            return Err(error("invalid sort range"));
        }
        if request.by_columns {
            return self.sort_left_to_right(index, request, full);
        }
        let width = full.end.column - full.start.column + 1;
        if request.keys.is_empty()
            || request.keys.len() > 64
            || request.keys.iter().any(|key| key.column >= width)
            || request
                .keys
                .iter()
                .map(|key| (key.column, key.color.clone()))
                .collect::<HashSet<_>>()
                .len()
                != request.keys.len()
        {
            // A column may come back once per colour, as Excel's levels do.
            return Err(error(
                "sort requires 1–64 distinct columns within the range",
            ));
        }
        let mut body = full;
        body.start.row += u32::from(request.header);
        let mut table = None;
        for candidate in &sheet.tables {
            if !intersects(candidate.reference, full) {
                continue;
            }
            if candidate.reference != full
                || candidate.header_row_count > 1
                || candidate.totals_row_count > 1
                || (candidate.header_row_count == 1) != request.header
            {
                return Err(error(
                    "sort the complete table with its declared header setting",
                ));
            }
            body.end.row = body.end.row.saturating_sub(candidate.totals_row_count);
            table = Some(candidate.name.as_str());
        }
        if body.start.row > body.end.row {
            return Err(error("sort range contains no data rows"));
        }
        let height = body.end.row - body.start.row + 1;
        if height > SORT_ROW_LIMIT || u64::from(width) * u64::from(height) > SORT_CELL_LIMIT {
            return Err(error("sort row/cell limit exceeded"));
        }
        if self
            .filter_row_state
            .get(sheet_name)
            .is_some_and(|state| state.invalid)
            || sheet.filtered_row_ranges().1
        {
            return Err(error("saved filter visibility cannot be established"));
        }
        // Sorting only part of filtered records would invalidate row visibility.
        let mut filtered_bodies = vec![];
        if let Some(filter) = &sheet.auto_filter {
            if let Some(mut range) = filter.reference {
                range.start.row += 1;
                if range.start.row <= range.end.row {
                    filtered_bodies.push((
                        range,
                        filter.has_criteria() && sheet.filter_mode != Some(false),
                    ));
                }
            }
        }
        for table in &sheet.tables {
            if let Some(filter) = &table.filter {
                if let Some(mut range) = filter.reference.or(Some(table.reference)) {
                    range.start.row += table.header_row_count;
                    range.end.row = range.end.row.min(
                        table
                            .reference
                            .end
                            .row
                            .saturating_sub(table.totals_row_count),
                    );
                    if range.start.row <= range.end.row {
                        filtered_bodies.push((
                            range,
                            filter.has_criteria() && sheet.filter_mode != Some(false),
                        ));
                    }
                }
            }
        }
        let move_filtered_visibility = filtered_bodies
            .iter()
            .any(|(range, active)| *active && intersects(*range, body));
        for (range, active) in filtered_bodies {
            if (intersects(range, body)
                || move_filtered_visibility
                    && active
                    && range.start.row <= body.end.row
                    && body.start.row <= range.end.row)
                && (body.start.row != range.start.row
                    || body.end.row != range.end.row
                    || body.start.column > range.start.column
                    || body.end.column < range.end.column)
            {
                return Err(error("sort the complete filtered records"));
            }
        }
        let (locale, phonetic, collator) = collation(request)?;
        let columns = request
            .keys
            .iter()
            .map(|key| body.start.column + key.column)
            .collect::<BTreeSet<_>>();
        let readings = if phonetic {
            jade_xlsx_reader::phonetic::sort_readings(
                &self.source,
                &sheet.part_name,
                body,
                &columns,
            )?
        } else {
            BTreeMap::new()
        };
        let cells: BTreeMap<_, _> = sheet
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .map(|cell| (cell.reference, cell))
            .collect();
        let mut keyed = Vec::with_capacity(height as usize);
        for row in body.start.row..=body.end.row {
            let mut values = vec![];
            for key in &request.keys {
                let cell = cells.get(&CellRef::new(body.start.column + key.column, row));
                if cell.is_some_and(|cell| {
                    cell.formula.is_some() && matches!(cell.value, CellValue::Empty)
                }) {
                    return Err(error(
                        "calculate missing sort-key formula results before sorting",
                    ));
                }
                if cell.is_some_and(
                    |cell| matches!(cell.value,CellValue::Number(number) if !number.is_finite()),
                ) {
                    return Err(error("nonfinite sort key"));
                }
                let reading = readings.get(&CellRef::new(body.start.column + key.column, row));
                if let Some(reading) = reading
                    .filter(|_| cell.is_some_and(|cell| matches!(cell.value, CellValue::Text(_))))
                {
                    if let Some(issue) = &reading.issue {
                        return Err(error(issue.clone()));
                    }
                    if cell.is_some_and(
                        |cell| matches!(&cell.value,CellValue::Text(value) if value!=&reading.base),
                    ) {
                        return Err(error(
                            "phonetic guide base text differs from the cell value",
                        ));
                    }
                }
                if let Some(color) = &key.color {
                    // Of the colour or not: those that are first (last when
                    // descending), each lot keeping its order.
                    let at = CellRef::new(body.start.column + key.column, row);
                    let of = cell_color(sheet, &self.workbook.styles, at, color.font) == color.rgb;
                    values.push(SortValue::Number(if of { 0.0 } else { 1.0 }));
                    continue;
                }
                values.push(SortValue::new(
                    cell.map(|cell| &cell.value),
                    reading.and_then(|reading| reading.key.as_deref()),
                    request.case_sensitive,
                ));
            }
            keyed.push((row, values));
        }
        // Choose one comparison per column; pair-dependent fallback can create cycles.
        let ascii_columns: Vec<_> = (0..request.keys.len())
            .map(|index| {
                locale.id.language.as_str() == "en"
                    && keyed.iter().all(|(_, values)| {
                        !matches!(&values[index], SortValue::Text { ascii: None, .. })
                    })
            })
            .collect();
        keyed.sort_by(|(_, a), (_, b)| {
            a.iter()
                .zip(b)
                .zip(&request.keys)
                .enumerate()
                .map(|(index, ((a, b), key))| {
                    a.compare(b, key.descending, ascii_columns[index], &collator)
                })
                .find(|order| *order != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        });
        let order: Vec<_> = keyed.into_iter().map(|(row, _)| row).collect();
        let moved = order
            .iter()
            .enumerate()
            .filter(|(offset, row)| body.start.row + *offset as u32 != **row)
            .count() as u32;
        let mut written = jade_xlsx_writer::sort::permute_rows(&self.source, sheet, body, &order)?;
        // The sort state says how to sort again by value; one by colour
        // would need a differential style for each colour, so it says none.
        if request.keys.iter().all(|key| key.color.is_none()) {
            let keys: Vec<_> = request
                .keys
                .iter()
                .map(|key| (body.start.column + key.column, key.descending))
                .collect();
            written = jade_xlsx_writer::sort::write_sort_state(
                &written,
                sheet,
                table,
                body,
                &keys,
                request.case_sensitive,
            )?;
        } else {
            written = jade_xlsx_writer::sort::clear_sort_state(&written, sheet, table, body)?;
        }
        let mut state = self
            .filter_row_state
            .get(sheet_name)
            .cloned()
            .unwrap_or_default();
        let filtered = state.filtered.clone();
        let mut hidden = BTreeMap::new();
        for (offset, source) in order.iter().enumerate() {
            let destination = body.start.row + offset as u32;
            let physically_hidden = sheet
                .rows
                .binary_search_by_key(&destination, |row| row.index)
                .ok()
                .map(|i| sheet.rows[i].hidden)
                .unwrap_or(sheet.default_rows_hidden);
            let manual = state.manual.contains(&destination)
                || physically_hidden && !filtered.contains(&destination);
            if manual {
                state.manual.insert(destination);
            }
            let is_filtered = filtered.contains(if move_filtered_visibility {
                source
            } else {
                &destination
            });
            if is_filtered {
                state.filtered.insert(destination);
            } else {
                state.filtered.remove(&destination);
            }
            if physically_hidden != (manual || is_filtered) {
                hidden.insert(destination, manual || is_filtered);
            }
        }
        let workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let active = !state.filtered.is_empty();
        if !hidden.is_empty() {
            written = jade_xlsx_writer::auto_filter::write_filter_visibility(
                &written, &workbook, sheet_name, &hidden, active,
            )?;
        }
        let workbook = jade_xlsx_reader::read_xlsx(&written)?;
        written = jade_xlsx_writer::filter_state::write_filter_row_state(
            &written,
            &workbook,
            sheet_name,
            &state.manual,
            &state.filtered,
        )?;
        let summary = SortSummary {
            reference: format!(
                "{}:{}",
                format_reference(body.start),
                format_reference(body.end)
            ),
            rows: height,
            moved_rows: moved,
            locale: locale.to_string(),
            phonetic,
        };
        if written == self.source {
            return Ok(summary);
        }
        let workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let before = std::mem::replace(&mut self.source, written);
        self.workbook = workbook;
        self.remember(before);
        self.filter_row_state.insert(sheet_name.into(), state);
        self.recalculate_after_edit();
        Ok(summary)
    }
}

/// The colour of the cell at `at` as Sort On Cell Color (or, `font`, Font
/// Color) sees it: `RRGGBB` of a solid fill or the font, empty for no fill
/// or the automatic font colour; a pattern or gradient is neither.
fn cell_color(
    sheet: &Sheet,
    styles: &jade_xlsx_reader::StyleTable,
    at: CellRef,
    font: bool,
) -> String {
    let format = styles.cell_format(Some(effective_style(sheet, at)));
    let hex = |color| {
        jade_xlsx_layout::color::rgba(color)
            .filter(|rgba| rgba[3] == 255)
            .map(|rgba| format!("{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2]))
    };
    if font {
        return format
            .and_then(|format| styles.font(format))
            .and_then(|font| font.color)
            .and_then(hex)
            .unwrap_or_default();
    }
    let fill = format.and_then(|format| styles.fill(format));
    match fill.and_then(|fill| fill.pattern.as_deref()) {
        _ if fill.is_some_and(|fill| fill.gradient.is_some()) => "gradient".into(),
        None | Some("none") => String::new(),
        Some("solid") => fill
            .and_then(|fill| fill.foreground)
            .and_then(hex)
            .unwrap_or_default(),
        Some(_) => "pattern".into(),
    }
}

/// The language, whether saved phonetic guides count, and the collator a
/// sort request asks for.
fn collation(request: &SortRequest) -> Result<(Locale, bool, CollatorBorrowed<'static>)> {
    let locale: Locale = request
        .locale
        .as_deref()
        .unwrap_or("en-US")
        .parse()
        .map_err(|_| error("invalid sort language tag"))?;
    let phonetic = request
        .phonetic
        .unwrap_or(locale.id.language.as_str() == "ja");
    let mut preferences: CollatorPreferences = locale.clone().into();
    preferences.case_first = Some(CollationCaseFirst::Lower);
    preferences.numeric_ordering = Some(CollationNumericOrdering::False);
    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Secondary);
    options.case_level = Some(if request.case_sensitive {
        CaseLevel::On
    } else {
        CaseLevel::Off
    });
    let collator = CollatorBorrowed::try_new(preferences, options)
        .map_err(|cause| error(format!("sort language data is unavailable: {cause}")))?;
    Ok((locale, phonetic, collator))
}

impl HeadlessWorkbook {
    /// Sort Options ▸ Sort left to right: the range's columns put in the
    /// order of the key rows' values (or colours), each column whole, in one
    /// history step. Tables and filtered lists sort top to bottom only, as
    /// in Excel; saved phonetic guides do not count here.
    fn sort_left_to_right(
        &mut self,
        index: usize,
        request: &SortRequest,
        full: CellRange,
    ) -> Result<SortSummary> {
        let sheet = &self.workbook.sheets[index];
        let height = full.end.row - full.start.row + 1;
        if request.keys.is_empty()
            || request.keys.len() > 64
            || request.keys.iter().any(|key| key.column >= height)
            || request
                .keys
                .iter()
                .map(|key| (key.column, key.color.clone()))
                .collect::<HashSet<_>>()
                .len()
                != request.keys.len()
        {
            return Err(error("sort requires 1–64 distinct rows within the range"));
        }
        let mut body = full;
        body.start.column += u32::from(request.header);
        if body.start.column > body.end.column {
            return Err(error("sort range contains no data columns"));
        }
        let width = body.end.column - body.start.column + 1;
        if width > SORT_ROW_LIMIT || u64::from(width) * u64::from(height) > SORT_CELL_LIMIT {
            return Err(error("sort row/cell limit exceeded"));
        }
        let filtered = sheet
            .auto_filter
            .as_ref()
            .and_then(|filter| filter.reference)
            .is_some_and(|range| intersects(range, full));
        if filtered
            || sheet
                .tables
                .iter()
                .any(|table| intersects(table.reference, full))
        {
            return Err(error("tables and filtered lists sort top to bottom only"));
        }
        let (locale, _, collator) = collation(request)?;
        let cells: BTreeMap<_, _> = sheet
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .map(|cell| (cell.reference, cell))
            .collect();
        let mut keyed = Vec::with_capacity(width as usize);
        for column in body.start.column..=body.end.column {
            let mut values = vec![];
            for key in &request.keys {
                let at = CellRef::new(column, full.start.row + key.column);
                if let Some(color) = &key.color {
                    let of = cell_color(sheet, &self.workbook.styles, at, color.font) == color.rgb;
                    values.push(SortValue::Number(if of { 0.0 } else { 1.0 }));
                    continue;
                }
                let cell = cells.get(&at);
                if cell.is_some_and(|cell| {
                    cell.formula.is_some() && matches!(cell.value, CellValue::Empty)
                }) {
                    return Err(error(
                        "calculate missing sort-key formula results before sorting",
                    ));
                }
                if cell.is_some_and(
                    |cell| matches!(cell.value, CellValue::Number(number) if !number.is_finite()),
                ) {
                    return Err(error("nonfinite sort key"));
                }
                values.push(SortValue::new(
                    cell.map(|cell| &cell.value),
                    None,
                    request.case_sensitive,
                ));
            }
            keyed.push((column, values));
        }
        let ascii_keys: Vec<_> = (0..request.keys.len())
            .map(|index| {
                locale.id.language.as_str() == "en"
                    && keyed.iter().all(|(_, values)| {
                        !matches!(&values[index], SortValue::Text { ascii: None, .. })
                    })
            })
            .collect();
        keyed.sort_by(|(_, a), (_, b)| {
            a.iter()
                .zip(b)
                .zip(&request.keys)
                .enumerate()
                .map(|(index, ((a, b), key))| {
                    a.compare(b, key.descending, ascii_keys[index], &collator)
                })
                .find(|order| *order != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        });
        let order: Vec<_> = keyed.into_iter().map(|(column, _)| column).collect();
        let moved = order
            .iter()
            .enumerate()
            .filter(|(offset, column)| body.start.column + *offset as u32 != **column)
            .count() as u32;
        let written = jade_xlsx_writer::sort::permute(
            &self.source,
            sheet,
            body,
            jade_xlsx_reader::Axis::Columns,
            &order,
        )?;
        let summary = SortSummary {
            reference: format!(
                "{}:{}",
                format_reference(body.start),
                format_reference(body.end)
            ),
            rows: width,
            moved_rows: moved,
            locale: locale.to_string(),
            phonetic: false,
        };
        if written == self.source {
            return Ok(summary);
        }
        let workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let before = std::mem::replace(&mut self.source, written);
        self.workbook = workbook;
        self.remember(before);
        self.recalculate_after_edit();
        Ok(summary)
    }
}
