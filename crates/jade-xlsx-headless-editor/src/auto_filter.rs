use super::*;
mod rank;
use jade_xlsx_headless_api::{
    AutoFilterChange, AutoFilterOption, AutoFilterOptions, AutoFilterSummary, AutoFilterTarget,
};
use jade_xlsx_reader::auto_filter::{AutoFilter, FilterColumn, FilterCriterion};

const ROW_LIMIT: u32 = 100_000;
const OPTION_LIMIT: usize = 10_000;
const CUSTOM_COMPARISON_LIMIT: u64 = 32_000_000;

pub(super) type FilterRowState = jade_xlsx_reader::filter_state::FilterRowState;

fn error(message: impl Into<String>) -> HeadlessError {
    HeadlessError::AutoFilter(message.into())
}
struct Target<'a> {
    exists: bool,
    filter: AutoFilter,
    table: Option<&'a jade_xlsx_reader::Table>,
    range: CellRange,
    first: u32,
    end: u32,
}
fn target<'a>(
    sheet: &'a Sheet,
    target: &AutoFilterTarget,
    reference: Option<&str>,
) -> Result<Target<'a>> {
    let table = match target {
        AutoFilterTarget::Worksheet => None,
        AutoFilterTarget::Table { name } => Some(
            sheet
                .tables
                .iter()
                .find(|table| &table.name == name)
                .ok_or_else(|| error("table is unavailable"))?,
        ),
    };
    let existing = table.and_then(|table| table.filter.as_ref()).or_else(|| {
        if table.is_none() {
            sheet.auto_filter.as_ref()
        } else {
            None
        }
    });
    let mut range = if let Some(reference) = reference {
        if table.is_some() || existing.is_some() {
            return Err(error(
                "filter ranges can only be specified when creating a worksheet filter",
            ));
        }
        CellRange::parse(reference).map_err(|_| error("invalid filter range"))?
    } else {
        existing
            .and_then(|filter| filter.reference)
            .or_else(|| table.map(|table| table.reference))
            .ok_or_else(|| error("filter range is unavailable"))?
    };
    if let Some(total) = table.and_then(|table| table.totals_row()) {
        let end = total
            .checked_sub(1)
            .ok_or_else(|| error("filter contains no data rows"))?;
        range.end.row = range.end.row.min(end);
    }
    let headers = table.map(|table| table.header_row_count).unwrap_or(1);
    if headers > 1 || range.start.row > range.end.row || range.start.column > range.end.column {
        return Err(error("invalid filter range/header count"));
    }
    let first = range.start.row + headers;
    if first > range.end.row.saturating_add(1) {
        return Err(error("invalid filter data rows"));
    }
    if first <= range.end.row && range.end.row - first + 1 > ROW_LIMIT {
        return Err(error(format!("filter row limit is {ROW_LIMIT}")));
    }
    if existing.is_some_and(|filter| {
        filter
            .issues
            .iter()
            .any(|issue| issue != "missing filter range" || table.is_none())
    }) {
        return Err(error("filter metadata is invalid"));
    }
    let mut filter = existing.cloned().unwrap_or(AutoFilter {
        reference: Some(range),
        columns: vec![],
        issues: vec![],
    });
    filter.reference = Some(range);
    filter.issues.clear();
    if filter
        .columns
        .iter()
        .any(|column| column.column_id > range.end.column - range.start.column)
    {
        return Err(error("filter column is outside its range"));
    }
    Ok(Target {
        exists: existing.is_some(),
        filter,
        table,
        range,
        first,
        end: range.end.row,
    })
}
fn bool_attribute(node: &FilterCriterion, key: &str) -> Result<bool> {
    match node.attributes.get(key).map(String::as_str) {
        None | Some("0" | "false") => Ok(false),
        Some("1" | "true") => Ok(true),
        _ => Err(error(format!("invalid {key} filter flag"))),
    }
}
const DATE_FIELDS: [&str; 6] = ["year", "month", "day", "hour", "minute", "second"];
fn date_prefix(node: &FilterCriterion) -> Result<Vec<u32>> {
    let count = match node.attributes.get("dateTimeGrouping").map(String::as_str) {
        Some("year") => 1,
        Some("month") => 2,
        Some("day") => 3,
        Some("hour") => 4,
        Some("minute") => 5,
        Some("second") => 6,
        _ => return Err(error("invalid date grouping")),
    };
    if !node.children.is_empty() {
        return Err(error("date groups cannot contain child conditions"));
    }
    let mut fields = [None; 6];
    for (index, key) in DATE_FIELDS.iter().enumerate() {
        if let Some(value) = node.attributes.get(*key) {
            let parsed = value
                .parse::<u32>()
                .map_err(|_| error(format!("invalid date group {key}")))?;
            let valid = match index {
                0 => (1..=9999).contains(&parsed),
                1 => (1..=12).contains(&parsed),
                2 => (1..=31).contains(&parsed),
                3 => parsed < 24,
                _ => parsed < 60,
            };
            if !valid {
                return Err(error(format!("invalid date group {key}")));
            }
            fields[index] = Some(parsed);
        } else if index < count {
            return Err(error(format!("date group requires {key}")));
        }
    }
    if let (Some(year), Some(month), Some(day)) = (fields[0], fields[1], fields[2]) {
        if !number_format::is_calendar_date_valid(year, month, day, DateSystem::Nineteen00) {
            return Err(error("invalid date group calendar day"));
        }
    }
    Ok(fields[..count]
        .iter()
        .map(|field| field.expect("required date group field validated"))
        .collect())
}
fn validate(node: &FilterCriterion) -> Result<()> {
    match node.kind.as_str() {
        "filters" => {
            bool_attribute(node, "blank")?;
            for child in &node.children {
                match child.kind.as_str() {
                    "filter" if child.attributes.contains_key("val") => {}
                    "dateGroupItem" => {
                        date_prefix(child)?;
                    }
                    _ => return Err(error("extension filters cannot yet be reapplied")),
                }
            }
            if node
                .children
                .iter()
                .any(|child| child.kind == "dateGroupItem")
                && !matches!(
                    node.attributes.get("calendarType").map(String::as_str),
                    None | Some("none" | "gregorian")
                )
            {
                return Err(error("unsupported date filter calendar"));
            }
        }
        "top10" => {
            rank::parameters(node)?;
        }
        "customFilters" => {
            bool_attribute(node, "and")?;
            if node.children.is_empty() || node.children.len() > 2 {
                return Err(error("custom filters require one or two conditions"));
            }
            for child in &node.children {
                if child.kind != "customFilter"
                    || !child.attributes.contains_key("val")
                    || child
                        .attributes
                        .get("val")
                        .is_some_and(|value| value.len() > 131_068)
                    || !child.children.is_empty()
                    || !matches!(
                        child
                            .attributes
                            .get("operator")
                            .map(String::as_str)
                            .unwrap_or("equal"),
                        "equal"
                            | "notEqual"
                            | "lessThan"
                            | "lessThanOrEqual"
                            | "greaterThan"
                            | "greaterThanOrEqual"
                    )
                {
                    return Err(error("unsupported custom filter condition"));
                }
            }
        }
        _ => {
            return Err(error(format!(
                "{} filters cannot yet be reapplied",
                node.kind
            )))
        }
    }
    Ok(())
}
fn comparison_work(budget: &mut u64, amount: u64) -> Result<()> {
    *budget = budget
        .checked_sub(amount)
        .ok_or_else(|| error("custom filter comparison work limit exceeded"))?;
    Ok(())
}
fn wildcard(pattern: &str, text: &str, budget: &mut u64) -> Result<bool> {
    #[derive(Clone, Copy)]
    enum Token {
        Literal(char),
        Any,
        Star,
    }
    let mut tokens = vec![];
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        tokens.push(match c {
            '~' if chars
                .peek()
                .is_some_and(|next| matches!(next, '?' | '*' | '~')) =>
            {
                Token::Literal(chars.next().unwrap())
            }
            '~' => Token::Literal('~'),
            '?' => Token::Any,
            '*' => Token::Star,
            _ => Token::Literal(c),
        });
    }
    let text: Vec<_> = text.chars().collect();
    let (mut p, mut t, mut star, mut retry) = (0, 0, None, 0);
    while t < text.len() {
        comparison_work(budget, 1)?;
        if p < tokens.len()
            && (matches!(tokens[p], Token::Any)
                || matches!(tokens[p],Token::Literal(c) if c==text[t]))
        {
            p += 1;
            t += 1;
        } else if p < tokens.len() && matches!(tokens[p], Token::Star) {
            star = Some(p);
            p += 1;
            retry = t;
        } else if let Some(at) = star {
            retry += 1;
            t = retry;
            p = at + 1;
        } else {
            return Ok(false);
        }
    }
    while p < tokens.len() && matches!(tokens[p], Token::Star) {
        p += 1;
    }
    Ok(p == tokens.len())
}
fn scalar_key(value: &CellValue) -> String {
    match value {
        CellValue::Number(value) => {
            if *value == 0.0 {
                "0".into()
            } else {
                value.to_string()
            }
        }
        CellValue::Text(value) | CellValue::Error(value) => value.clone(),
        CellValue::Bool(value) => {
            if *value {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }
        CellValue::Empty => String::new(),
    }
}

enum PreparedCriterion<'a> {
    Values {
        blank: bool,
        strings: HashSet<String>,
        numbers: HashSet<String>,
        dates: Option<Box<[HashSet<Vec<u32>>; 6]>>,
    },
    Custom(&'a FilterCriterion),
    Rank {
        top: bool,
        threshold: f64,
    },
}
fn prepare(node: &FilterCriterion) -> Result<PreparedCriterion<'_>> {
    validate(node)?;
    if node.kind == "top10" {
        let (top, _, _, threshold) = rank::parameters(node)?;
        return Ok(PreparedCriterion::Rank { top, threshold });
    }
    if node.kind == "filters" {
        let strings = node
            .children
            .iter()
            .filter(|child| child.kind == "filter")
            .map(|child| child.attributes["val"].to_lowercase())
            .collect();
        let numbers = node
            .children
            .iter()
            .filter(|child| child.kind == "filter")
            .filter_map(|child| {
                child.attributes["val"]
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
            })
            .map(|number| scalar_key(&CellValue::Number(number)))
            .collect();
        let mut dates: [HashSet<Vec<u32>>; 6] = std::array::from_fn(|_| HashSet::new());
        for group in node
            .children
            .iter()
            .filter(|child| child.kind == "dateGroupItem")
        {
            let prefix = date_prefix(group)?;
            dates[prefix.len() - 1].insert(prefix);
        }
        Ok(PreparedCriterion::Values {
            blank: bool_attribute(node, "blank")?,
            strings,
            numbers,
            dates: dates
                .iter()
                .any(|groups| !groups.is_empty())
                .then(|| Box::new(dates)),
        })
    } else {
        Ok(PreparedCriterion::Custom(node))
    }
}
impl PreparedCriterion<'_> {
    fn matches(
        &self,
        value: &CellValue,
        display: &str,
        epoch: DateSystem,
        comparison_budget: &mut u64,
    ) -> Result<bool> {
        match self {
            Self::Custom(node) => matches_custom(node, value, comparison_budget),
            Self::Rank { top, threshold } => match value {
                CellValue::Number(value) if value.is_finite() => Ok(if *top {
                    value >= threshold
                } else {
                    value <= threshold
                }),
                CellValue::Number(_) => Err(error("non-finite numeric cells cannot be filtered")),
                _ => Ok(false),
            },
            Self::Values {
                blank,
                strings,
                numbers,
                dates,
            } => {
                let empty = matches!(value, CellValue::Empty)
                    || matches!(value,CellValue::Text(text) if text.is_empty());
                if empty {
                    return Ok(*blank);
                }
                if matches!(value,CellValue::Number(number) if !number.is_finite()) {
                    return Err(error("non-finite numeric cells cannot be filtered"));
                }
                let date_match = if let (CellValue::Number(number), Some(dates)) = (value, dates) {
                    number_format::calendar_fields(*number, epoch).is_some_and(|fields| {
                        dates
                            .iter()
                            .enumerate()
                            .any(|(index, groups)| groups.contains(&fields[..=index]))
                    })
                } else {
                    false
                };
                Ok(date_match
                    || strings.contains(&scalar_key(value).to_lowercase())
                    || strings.contains(&display.to_lowercase())
                    || (matches!(value, CellValue::Number(_))
                        && numbers.contains(&scalar_key(value))))
            }
        }
    }
}

fn matches_custom(node: &FilterCriterion, value: &CellValue, budget: &mut u64) -> Result<bool> {
    let mut results = vec![];
    for child in &node.children {
        let expected = &child.attributes["val"];
        comparison_work(
            budget,
            expected.len() as u64
                + match value {
                    CellValue::Text(text) => text.len() as u64,
                    _ => 1,
                },
        )?;
        if matches!(value, CellValue::Number(number) if !number.is_finite()) {
            return Err(error("non-finite numeric cells cannot be filtered"));
        }
        let operator = child
            .attributes
            .get("operator")
            .map(String::as_str)
            .unwrap_or("equal");
        let ordering = match value {
            CellValue::Number(number) => expected
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .and_then(|n| number.partial_cmp(&n)),
            CellValue::Text(text) => Some(text.to_lowercase().cmp(&expected.to_lowercase())),
            CellValue::Empty => Some("".cmp(expected)),
            _ => None,
        };
        let equal = if matches!(value, CellValue::Text(_) | CellValue::Empty) {
            let text = match value {
                CellValue::Text(text) => text.as_str(),
                _ => "",
            };
            wildcard(&expected.to_lowercase(), &text.to_lowercase(), budget)?
        } else {
            ordering == Some(std::cmp::Ordering::Equal)
        };
        results.push(match operator {
            "equal" => equal,
            "notEqual" => !equal,
            "lessThan" => ordering == Some(std::cmp::Ordering::Less),
            "lessThanOrEqual" => matches!(
                ordering,
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ),
            "greaterThan" => ordering == Some(std::cmp::Ordering::Greater),
            "greaterThanOrEqual" => matches!(
                ordering,
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ),
            _ => false,
        });
    }
    Ok(if bool_attribute(node, "and")? {
        results.iter().all(|result| *result)
    } else {
        results.iter().any(|result| *result)
    })
}
impl HeadlessWorkbook {
    pub(super) fn initialize_filter_row_state(&mut self) {
        let mut states = BTreeMap::new();
        for sheet in &self.workbook.sheets {
            if let Some(state) = &sheet.filter_row_state {
                states.insert(sheet.name.clone(), state.clone());
                continue;
            }
            let (mut ranges, unknown) = sheet.filtered_row_ranges();
            if unknown {
                continue;
            }
            if sheet.filter_mode == Some(false) {
                if let Ok(target) = target(sheet, &AutoFilterTarget::Worksheet, None) {
                    if target.filter.has_criteria() {
                        ranges.push((target.first, target.end));
                    }
                }
                for table in &sheet.tables {
                    if let Ok(target) = target(
                        sheet,
                        &AutoFilterTarget::Table {
                            name: table.name.clone(),
                        },
                        None,
                    ) {
                        if target.filter.has_criteria() {
                            ranges.push((target.first, target.end));
                        }
                    }
                }
            }
            if ranges.is_empty() {
                continue;
            }
            let mut indices = HashSet::new();
            for row in &sheet.rows {
                if row.hidden
                    && ranges
                        .iter()
                        .any(|(first, end)| row.index >= *first && row.index <= *end)
                {
                    indices.insert(row.index);
                }
            }
            if sheet.default_rows_hidden {
                for (first, end) in ranges {
                    if first <= end && end - first < ROW_LIMIT {
                        for row in first..=end {
                            if sheet
                                .rows
                                .binary_search_by_key(&row, |row| row.index)
                                .ok()
                                .map(|at| sheet.rows[at].hidden)
                                .unwrap_or(true)
                            {
                                indices.insert(row);
                            }
                        }
                    }
                }
            }
            let state = if sheet.filter_mode == Some(false) {
                FilterRowState {
                    manual: indices,
                    ..Default::default()
                }
            } else {
                FilterRowState {
                    filtered: indices,
                    ..Default::default()
                }
            };
            states.insert(sheet.name.clone(), state);
        }
        self.filter_row_state = states;
    }

    pub(super) fn persist_filter_row_state(&mut self, sheet: &str) -> Result<()> {
        let Some(state) = self.filter_row_state.get(sheet) else {
            return Ok(());
        };
        if state.invalid {
            return Ok(());
        }
        let source = jade_xlsx_writer::filter_state::write_filter_row_state(
            &self.source,
            &self.workbook,
            sheet,
            &state.manual,
            &state.filtered,
        )?;
        if source != self.source {
            self.workbook = jade_xlsx_reader::read_xlsx(&source)?;
            self.source = source;
        }
        Ok(())
    }

    pub(super) fn filter_state_for_row_sizes(
        &self,
        sheet_name: &str,
        sizes: &BTreeMap<u32, f32>,
    ) -> BTreeMap<String, FilterRowState> {
        let mut states = self.filter_row_state.clone();
        if let Ok(sheet) = self.sheet(sheet_name) {
            let (mut ranges, unknown) = sheet.filtered_row_ranges();
            if !unknown {
                if let Some(filter) = &sheet.auto_filter {
                    if let Some(range) = filter.reference {
                        ranges.push((range.start.row.saturating_add(1), range.end.row));
                    }
                }
                for table in &sheet.tables {
                    if table.filter.is_some() {
                        ranges.push((
                            table.reference.start.row + table.header_row_count,
                            table
                                .reference
                                .end
                                .row
                                .saturating_sub(table.totals_row_count),
                        ));
                    }
                }
            }
            if !unknown && !ranges.is_empty() {
                let state = states.entry(sheet_name.into()).or_default();
                for (row, size) in sizes {
                    if ranges.iter().any(|(first, end)| row >= first && row <= end) {
                        if *size == 0.0 {
                            state.manual.insert(*row);
                        } else {
                            state.manual.remove(row);
                            state.filtered.remove(row);
                        }
                    }
                }
            }
        }
        states
    }

    pub(super) fn filter_state_for_all_row_sizes(
        &self,
        sheet_name: &str,
        size: f32,
    ) -> BTreeMap<String, FilterRowState> {
        let mut sizes = BTreeMap::new();
        if let Ok(sheet) = self.sheet(sheet_name) {
            let (ranges, unknown) = sheet.filtered_row_ranges();
            if !unknown {
                for (first, end) in ranges {
                    if end - first < ROW_LIMIT {
                        for row in first..=end {
                            sizes.insert(row, size);
                        }
                    }
                }
            }
        }
        self.filter_state_for_row_sizes(sheet_name, &sizes)
    }
    pub fn auto_filter_options(
        &self,
        sheet: &str,
        target_kind: &AutoFilterTarget,
        column: u32,
        reference: Option<&str>,
    ) -> Result<AutoFilterOptions> {
        let sheet = self.sheet(sheet)?;
        let target = target(sheet, target_kind, reference)?;
        if column > target.range.end.column - target.range.start.column {
            return Err(error("filter column is outside its range"));
        }
        let mut choices = BTreeMap::<(bool, String), AutoFilterOption>::new();
        let current = target
            .filter
            .columns
            .iter()
            .find(|entry| entry.column_id == column)
            .and_then(|entry| entry.criterion.as_ref());
        let mut unsupported = target
            .filter
            .columns
            .iter()
            .filter_map(|column| column.criterion.as_ref())
            .find_map(|node| validate(node).err().map(|error| error.to_string()))
            .or_else(|| {
                sheet
                    .filter_row_state
                    .as_ref()
                    .filter(|state| state.invalid)
                    .map(|_| "saved filter visibility snapshot is invalid".into())
            });
        let current = current.and_then(|node| prepare(node).ok());
        let other_columns = if unsupported.is_none() {
            target
                .filter
                .columns
                .iter()
                .filter(|entry| entry.column_id != column)
                .filter_map(|entry| {
                    entry
                        .criterion
                        .as_ref()
                        .map(|node| (target.range.start.column + entry.column_id, node))
                })
                .map(|(column, node)| Ok((column, prepare(node)?)))
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![]
        };
        let mut identity_labels = std::collections::HashMap::<String, (bool, String)>::new();
        let mut group_tokens = std::collections::HashMap::<(bool, String), HashSet<String>>::new();
        let mut truncated = false;
        let mut comparison_budget = CUSTOM_COMPARISON_LIMIT;
        for row in target.first..=target.end {
            let row = sheet
                .rows
                .binary_search_by_key(&row, |row| row.index)
                .ok()
                .map(|index| &sheet.rows[index]);
            let mut eligible = true;
            for (column, predicate) in &other_columns {
                let cell = row.and_then(|row| find_cell(row, *column));
                let value = cell.map(|cell| &cell.value).unwrap_or(&CellValue::Empty);
                let display = cell
                    .and_then(|cell| row.map(|row| self.display_of(sheet, row, cell)))
                    .unwrap_or_default();
                eligible &= predicate.matches(
                    value,
                    &display,
                    self.workbook.date_system,
                    &mut comparison_budget,
                )?;
            }
            if !eligible {
                continue;
            }
            let reference = CellRef {
                row: row.map(|row| row.index).unwrap_or(0),
                column: target.range.start.column + column,
            };
            let cell = row.and_then(|row| find_cell(row, reference.column));
            let blank = cell.is_none_or(|cell| {
                matches!(cell.value, CellValue::Empty)
                    || matches!(&cell.value,CellValue::Text(text) if text.is_empty())
            });
            let text = cell
                .and_then(|cell| row.map(|row| self.display_of(sheet, row, cell)))
                .unwrap_or_default();
            let key = (blank, text.to_lowercase());
            let value = cell.map(|cell| scalar_key(&cell.value)).unwrap_or_default();
            let identity = value.to_lowercase();
            if let Some(previous) = identity_labels.get(&identity) {
                if previous != &key {
                    unsupported = Some("formatted values share an ambiguous filter key".into());
                }
            } else if identity_labels.len() >= OPTION_LIMIT {
                truncated = true;
                continue;
            }
            identity_labels.insert(identity.clone(), key.clone());
            let selected = if unsupported.is_some() {
                None
            } else {
                Some(
                    current
                        .as_ref()
                        .map(|node| {
                            node.matches(
                                cell.map(|cell| &cell.value).unwrap_or(&CellValue::Empty),
                                &text,
                                self.workbook.date_system,
                                &mut comparison_budget,
                            )
                            .unwrap_or(false)
                        })
                        .unwrap_or(true),
                )
            };
            if choices.len() >= OPTION_LIMIT && !choices.contains_key(&key) {
                truncated = true;
                continue;
            }
            let option = choices.entry(key.clone()).or_insert(AutoFilterOption {
                value: value.clone(),
                values: vec![],
                label: text,
                blank,
                count: 0,
                selected,
            });
            if group_tokens.entry(key).or_default().insert(identity) {
                option.values.push(value);
            }
            if option.selected != selected {
                option.selected = None;
            }
            option.count += 1;
        }
        Ok(AutoFilterOptions {
            columns: (target.range.start.column..=target.range.end.column)
                .map(|column| {
                    let letter = column_label(column);
                    let header = if target.table.is_none_or(|table| table.header_row_count > 0) {
                        let row = sheet
                            .rows
                            .binary_search_by_key(&target.range.start.row, |row| row.index)
                            .ok()
                            .map(|index| &sheet.rows[index]);
                        self.display_in_row(
                            sheet,
                            row,
                            CellRef {
                                column,
                                row: target.range.start.row,
                            },
                        )
                    } else {
                        String::new()
                    };
                    jade_xlsx_headless_api::AutoFilterColumnOption {
                        column: column - target.range.start.column,
                        worksheet_column: column,
                        label: if header.is_empty() {
                            letter
                        } else {
                            format!("{header} ({letter})")
                        },
                    }
                })
                .collect(),
            options: choices.into_values().collect(),
            truncated,
            unsupported,
        })
    }

    pub fn change_auto_filter(
        &mut self,
        sheet_name: &str,
        change: &AutoFilterChange,
    ) -> Result<AutoFilterSummary> {
        let index = self.sheet_index(sheet_name)?;
        let canonical_name = self.workbook.sheets[index].name.clone();
        let sheet_name = canonical_name.as_str();
        let sheet = &self.workbook.sheets[index];
        if sheet
            .filter_row_state
            .as_ref()
            .is_some_and(|state| state.invalid)
        {
            return Err(error("saved filter visibility snapshot is invalid"));
        }
        if sheet.filtered_row_ranges().1 {
            return Err(error("existing filtered row range is unavailable"));
        }
        let disable = matches!(change, AutoFilterChange::Disable { .. });
        let rank_change = matches!(change, AutoFilterChange::Rank { .. });
        let (target_kind, reference, column, mut criterion, reapply, enable) = match change {
            AutoFilterChange::Enable { target, reference } => {
                (target, reference.as_deref(), None, None, false, true)
            }
            AutoFilterChange::Values {
                target,
                reference,
                column,
                values,
                include_blank,
            } => {
                if values.len() > OPTION_LIMIT || values.iter().any(|value| value.len() > 131_068) {
                    return Err(error("filter value limit exceeded"));
                }
                let node = FilterCriterion {
                    kind: "filters".into(),
                    attributes: BTreeMap::from([(
                        "blank".into(),
                        u8::from(*include_blank).to_string(),
                    )]),
                    children: values
                        .iter()
                        .map(|value| FilterCriterion {
                            kind: "filter".into(),
                            attributes: BTreeMap::from([("val".into(), value.clone())]),
                            children: vec![],
                        })
                        .collect(),
                };
                (
                    target,
                    reference.as_deref(),
                    Some(*column),
                    Some(node),
                    false,
                    false,
                )
            }
            AutoFilterChange::Dates {
                target,
                reference,
                column,
                groups,
                include_blank,
            } => {
                if groups.len() > OPTION_LIMIT {
                    return Err(error("date group limit exceeded"));
                }
                let children = groups
                    .iter()
                    .map(|group| {
                        let mut attributes = BTreeMap::from([
                            ("year".into(), group.year.to_string()),
                            ("dateTimeGrouping".into(), group.grouping.as_str().into()),
                        ]);
                        for (key, value) in [
                            ("month", group.month),
                            ("day", group.day),
                            ("hour", group.hour),
                            ("minute", group.minute),
                            ("second", group.second),
                        ] {
                            if let Some(value) = value {
                                attributes.insert(key.into(), value.to_string());
                            }
                        }
                        FilterCriterion {
                            kind: "dateGroupItem".into(),
                            attributes,
                            children: vec![],
                        }
                    })
                    .collect();
                let node = FilterCriterion {
                    kind: "filters".into(),
                    attributes: BTreeMap::from([
                        ("blank".into(), u8::from(*include_blank).to_string()),
                        ("calendarType".into(), "gregorian".into()),
                    ]),
                    children,
                };
                validate(&node)?;
                (
                    target,
                    reference.as_deref(),
                    Some(*column),
                    Some(node),
                    false,
                    false,
                )
            }
            AutoFilterChange::Rank {
                target,
                reference,
                column,
                value,
                top,
                percent,
            } => {
                if *value == 0 || *value > if *percent { 100 } else { 500 } {
                    return Err(error("rank value is outside its allowed range"));
                }
                let node = FilterCriterion {
                    kind: "top10".into(),
                    attributes: BTreeMap::from([
                        ("val".into(), value.to_string()),
                        ("top".into(), u8::from(*top).to_string()),
                        ("percent".into(), u8::from(*percent).to_string()),
                    ]),
                    children: vec![],
                };
                (
                    target,
                    reference.as_deref(),
                    Some(*column),
                    Some(node),
                    false,
                    false,
                )
            }
            AutoFilterChange::Custom {
                target,
                reference,
                column,
                conditions,
                and,
            } => {
                if conditions.is_empty() || conditions.len() > 2 {
                    return Err(error("custom filters require one or two conditions"));
                }
                if conditions
                    .iter()
                    .any(|condition| condition.value.len() > 131_068)
                {
                    return Err(error("filter value limit exceeded"));
                }
                let node = FilterCriterion {
                    kind: "customFilters".into(),
                    attributes: BTreeMap::from([("and".into(), u8::from(*and).to_string())]),
                    children: conditions
                        .iter()
                        .map(|condition| FilterCriterion {
                            kind: "customFilter".into(),
                            attributes: BTreeMap::from([
                                ("operator".into(), condition.operator.as_str().into()),
                                ("val".into(), condition.value.clone()),
                            ]),
                            children: vec![],
                        })
                        .collect(),
                };
                validate(&node)?;
                (
                    target,
                    reference.as_deref(),
                    Some(*column),
                    Some(node),
                    false,
                    false,
                )
            }
            AutoFilterChange::Clear { target, column } => {
                (target, None, *column, None, false, false)
            }
            AutoFilterChange::Reapply { target } => (target, None, None, None, true, false),
            AutoFilterChange::Disable { target } => (target, None, None, None, false, false),
        };
        if disable {
            let exists = match target_kind {
                AutoFilterTarget::Worksheet => sheet.auto_filter.is_some(),
                AutoFilterTarget::Table { name } => sheet
                    .tables
                    .iter()
                    .find(|table| &table.name == name)
                    .ok_or_else(|| error("table is unavailable"))?
                    .filter
                    .is_some(),
            };
            if !exists {
                return Ok(AutoFilterSummary {
                    changed_rows: 0,
                    active_columns: 0,
                });
            }
        }
        let mut selected = target(sheet, target_kind, reference)?;
        if enable && selected.exists {
            return Ok(AutoFilterSummary {
                changed_rows: 0,
                active_columns: selected
                    .filter
                    .columns
                    .iter()
                    .filter(|column| column.criterion.is_some())
                    .count() as u32,
            });
        }
        jade_xlsx_writer::auto_filter::validate_filter_edit(
            &self.source,
            &self.workbook,
            sheet_name,
            enable || disable || !selected.exists,
        )?;
        if !selected.exists
            && selected
                .table
                .is_some_and(|table| table.header_row_count == 0)
        {
            return Err(error("table filters require a header row"));
        }
        // Empty dropdowns do not reapply saved predicates from other owners.
        if enable {
            let state = self
                .filter_row_state
                .get(sheet_name)
                .cloned()
                .unwrap_or_default();
            let written = jade_xlsx_writer::auto_filter::write_auto_filter(
                &self.source,
                &self.workbook,
                sheet_name,
                &jade_xlsx_writer::auto_filter::AutoFilterWrite {
                    table: selected.table.map(|table| table.name.as_str()),
                    reference: selected.range,
                    column: None,
                    criterion: None,
                    hidden: &BTreeMap::new(),
                    filter_mode: sheet.filter_mode.unwrap_or(!state.filtered.is_empty()),
                },
            )?;
            self.commit_filter_bytes(written, sheet_name, state)?;
            return Ok(AutoFilterSummary {
                changed_rows: 0,
                active_columns: 0,
            });
        }
        if disable && !selected.filter.has_criteria() {
            let state = self
                .filter_row_state
                .get(sheet_name)
                .cloned()
                .unwrap_or_default();
            let written = jade_xlsx_writer::auto_filter::remove_auto_filter(
                &self.source,
                &self.workbook,
                sheet_name,
                selected.table.map(|table| table.name.as_str()),
                &BTreeMap::new(),
                sheet.filter_mode.unwrap_or(!state.filtered.is_empty()),
            )?;
            self.commit_filter_bytes(written, sheet_name, state)?;
            return Ok(AutoFilterSummary {
                changed_rows: 0,
                active_columns: 0,
            });
        }
        if column
            .is_some_and(|column| column > selected.range.end.column - selected.range.start.column)
        {
            return Err(error("filter column is outside its range"));
        }
        let before = selected.filter.clone();
        if !disable {
            for column in &before.columns {
                if let Some(criterion) = &column.criterion {
                    validate(criterion)?;
                }
            }
        }
        if !reapply {
            if let Some(column) = column {
                if let Some(existing) = selected
                    .filter
                    .columns
                    .iter_mut()
                    .find(|node| node.column_id == column)
                {
                    existing.criterion = criterion.clone();
                } else if criterion.is_some() {
                    selected.filter.columns.push(FilterColumn {
                        column_id: column,
                        hidden_button: false,
                        show_button: true,
                        criterion: criterion.clone(),
                    });
                }
            } else {
                for column in &mut selected.filter.columns {
                    column.criterion = None;
                }
            }
        }
        let mut rank_updates = BTreeMap::new();
        if reapply || rank_change {
            let rank_columns = selected
                .filter
                .columns
                .iter()
                .filter(|entry| {
                    (reapply || column == Some(entry.column_id))
                        && entry
                            .criterion
                            .as_ref()
                            .is_some_and(|node| node.kind == "top10")
                })
                .count() as u64;
            let data_rows = if selected.first <= selected.end {
                u64::from(selected.end - selected.first + 1)
            } else {
                0
            };
            if data_rows * rank_columns > RANGE_CELL_LIMIT {
                return Err(error("rank sample evaluation limit exceeded"));
            }
            for entry in &mut selected.filter.columns {
                if let Some(node) = entry.criterion.as_mut().filter(|node| node.kind == "top10") {
                    if reapply || column == Some(entry.column_id) {
                        let threshold = rank::threshold(
                            sheet,
                            selected.range.start.column + entry.column_id,
                            selected.first,
                            selected.end,
                            node,
                        )?;
                        node.attributes
                            .insert("filterVal".into(), threshold.to_string());
                        rank_updates.insert(entry.column_id, threshold);
                    }
                }
            }
            if rank_change {
                criterion = selected
                    .filter
                    .columns
                    .iter()
                    .find(|entry| Some(entry.column_id) == column)
                    .and_then(|entry| entry.criterion.clone());
            }
        }
        let mut filters = vec![(
            selected.range,
            selected.first,
            selected.end,
            &selected.filter,
        )];
        if !matches!(target_kind, AutoFilterTarget::Worksheet) {
            if let Some(filter) = &sheet.auto_filter {
                if filter.has_criteria() {
                    let other = target(sheet, &AutoFilterTarget::Worksheet, None)?;
                    filters.push((other.range, other.first, other.end, filter));
                }
            }
        }
        for table in &sheet.tables {
            if selected
                .table
                .is_some_and(|chosen| chosen.name == table.name)
            {
                continue;
            }
            if let Some(filter) = &table.filter {
                if filter.has_criteria() {
                    let other = target(
                        sheet,
                        &AutoFilterTarget::Table {
                            name: table.name.clone(),
                        },
                        None,
                    )?;
                    filters.push((other.range, other.first, other.end, filter));
                }
            }
        }
        let compiled = filters
            .iter()
            .map(|(range, first, end, filter)| {
                let columns = if (disable && sheet.filter_mode == Some(false))
                    || (sheet.filter_mode != Some(false)
                        && (*first > selected.end || *end < selected.first))
                {
                    vec![]
                } else {
                    filter
                        .columns
                        .iter()
                        .filter_map(|column| {
                            column
                                .criterion
                                .as_ref()
                                .map(|node| (column.column_id, node))
                        })
                        .map(|(column, node)| Ok((range.start.column + column, prepare(node)?)))
                        .collect::<Result<Vec<_>>>()?
                };
                Ok((*first, *end, columns))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut hidden = BTreeMap::new();
        let mut row_state = self
            .filter_row_state
            .get(sheet_name)
            .cloned()
            .unwrap_or_default();
        row_state.manual.retain(|index| {
            sheet
                .rows
                .binary_search_by_key(index, |row| row.index)
                .ok()
                .map(|at| sheet.rows[at].hidden)
                .unwrap_or(sheet.default_rows_hidden)
        });
        row_state.filtered.retain(|index| {
            sheet
                .rows
                .binary_search_by_key(index, |row| row.index)
                .ok()
                .map(|at| sheet.rows[at].hidden)
                .unwrap_or(sheet.default_rows_hidden)
        });
        let mut indices = std::collections::BTreeSet::new();
        if sheet.filter_mode == Some(false) && !disable {
            for (_, first, end, _) in &filters {
                for index in *first..=*end {
                    indices.insert(index);
                    if indices.len() > ROW_LIMIT as usize {
                        return Err(error("filter row limit exceeded"));
                    }
                }
            }
        } else {
            indices.extend(selected.first..=selected.end);
        }
        if indices.len() as u64
            * compiled
                .iter()
                .map(|(_, _, columns)| columns.len() as u64)
                .sum::<u64>()
            > RANGE_CELL_LIMIT
        {
            return Err(error("filter condition evaluation limit exceeded"));
        }
        let mut comparison_budget = CUSTOM_COMPARISON_LIMIT;
        for index in indices {
            let row = sheet
                .rows
                .binary_search_by_key(&index, |row| row.index)
                .ok()
                .map(|index| &sheet.rows[index]);
            let mut filtered = false;
            for (first, end, columns) in &compiled {
                if index < *first || index > *end {
                    continue;
                }
                for (column, predicate) in columns {
                    let cell = row.and_then(|row| find_cell(row, *column));
                    let value = cell.map(|cell| &cell.value).unwrap_or(&CellValue::Empty);
                    let display = cell
                        .and_then(|cell| row.map(|row| self.display_of(sheet, row, cell)))
                        .unwrap_or_default();
                    filtered |= !predicate.matches(
                        value,
                        &display,
                        self.workbook.date_system,
                        &mut comparison_budget,
                    )?;
                }
            }
            // Before filtering a new/unfiltered region, retain its manual hides.
            let was_hidden = row
                .map(|row| row.hidden)
                .unwrap_or(sheet.default_rows_hidden);
            if was_hidden
                && !row_state.filtered.contains(&index)
                && (!before.has_criteria() || sheet.filter_mode == Some(false))
            {
                row_state.manual.insert(index);
            }
            let manual = row_state.manual.contains(&index);
            if filtered {
                row_state.filtered.insert(index);
            } else {
                row_state.filtered.remove(&index);
            }
            let desired = filtered || manual;
            if desired != was_hidden {
                hidden.insert(index, desired);
            }
        }
        let active = row_state.filtered.iter().any(|row| {
            filters
                .iter()
                .any(|(_, first, end, filter)| filter.has_criteria() && row >= first && row <= end)
        });
        let summary = AutoFilterSummary {
            changed_rows: hidden.len() as u32,
            active_columns: selected
                .filter
                .columns
                .iter()
                .filter(|column| column.criterion.is_some())
                .count() as u32,
        };
        let state_changed = self
            .filter_row_state
            .get(sheet_name)
            .cloned()
            .unwrap_or_default()
            != row_state;
        if !disable
            && selected.exists
            && !state_changed
            && hidden.is_empty()
            && selected.filter == before
            && (sheet.filter_mode == Some(true)) == active
        {
            return Ok(summary);
        }
        let mut written = if disable {
            jade_xlsx_writer::auto_filter::remove_auto_filter(
                &self.source,
                &self.workbook,
                sheet_name,
                selected.table.map(|table| table.name.as_str()),
                &hidden,
                active,
            )?
        } else if reapply {
            jade_xlsx_writer::auto_filter::write_filter_visibility(
                &self.source,
                &self.workbook,
                sheet_name,
                &hidden,
                active,
            )?
        } else {
            jade_xlsx_writer::auto_filter::write_auto_filter(
                &self.source,
                &self.workbook,
                sheet_name,
                &jade_xlsx_writer::auto_filter::AutoFilterWrite {
                    table: selected.table.map(|table| table.name.as_str()),
                    reference: selected.range,
                    column,
                    criterion: criterion.as_ref(),
                    hidden: &hidden,
                    filter_mode: active,
                },
            )?
        };
        if reapply && !rank_updates.is_empty() {
            written = jade_xlsx_writer::auto_filter::write_rank_thresholds(
                &written,
                &self.workbook,
                sheet_name,
                selected.table.map(|table| table.name.as_str()),
                &rank_updates,
            )?;
        }
        self.commit_filter_bytes(written, sheet_name, row_state)?;
        Ok(summary)
    }
    fn commit_filter_bytes(
        &mut self,
        mut written: Vec<u8>,
        sheet: &str,
        state: FilterRowState,
    ) -> Result<()> {
        written = jade_xlsx_writer::mark_for_recalculation(&written)?;
        let workbook = jade_xlsx_reader::read_xlsx(&written)?;
        written = jade_xlsx_writer::filter_state::write_filter_row_state(
            &written,
            &workbook,
            sheet,
            &state.manual,
            &state.filtered,
        )?;
        let workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let before = std::mem::replace(&mut self.source, written);
        self.workbook = workbook;
        self.remember(before);
        self.filter_row_state.insert(sheet.into(), state);
        self.recalculate_after_edit();
        Ok(())
    }
}
