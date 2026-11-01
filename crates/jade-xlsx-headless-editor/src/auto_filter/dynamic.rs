//! Date Filters ▸ Today, This Month, Next Quarter… and Above/Below Average:
//! Excel's `<dynamicFilter>`, a range worked out from today (or from the
//! column's average) when the filter is applied or reapplied, kept on the
//! criterion as `val` and `maxVal`, and each cell matched against it.

use super::*;

/// The `type`s a dynamic filter may have.
const KINDS: &[&str] = &[
    "today",
    "yesterday",
    "tomorrow",
    "thisWeek",
    "lastWeek",
    "nextWeek",
    "thisMonth",
    "lastMonth",
    "nextMonth",
    "thisQuarter",
    "lastQuarter",
    "nextQuarter",
    "thisYear",
    "lastYear",
    "nextYear",
    "yearToDate",
    "aboveAverage",
    "belowAverage",
    "Q1",
    "Q2",
    "Q3",
    "Q4",
    "M1",
    "M2",
    "M3",
    "M4",
    "M5",
    "M6",
    "M7",
    "M8",
    "M9",
    "M10",
    "M11",
    "M12",
];

pub(super) fn validate(node: &FilterCriterion) -> Result<()> {
    if !node.children.is_empty() {
        return Err(error("dynamic filters cannot contain children"));
    }
    match node.attributes.get("type") {
        Some(kind) if KINDS.contains(&kind.as_str()) => Ok(()),
        _ => Err(error("unsupported dynamic filter")),
    }
}

/// Days from 1970-01-01 to `year`-`month`-`day` (proleptic Gregorian).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The serial of the first day of `year`-`month`, months past December
/// carried into the next year.
fn serial(year: i64, month: i64, epoch: DateSystem) -> f64 {
    let (year, month) = (
        year + (month - 1).div_euclid(12),
        (month - 1).rem_euclid(12) + 1,
    );
    let base = match epoch {
        DateSystem::Nineteen00 => days_from_civil(1899, 12, 30),
        DateSystem::Nineteen04 => days_from_civil(1904, 1, 1),
    };
    (days_from_civil(year, month as u32, 1) - base) as f64
}

/// `val` and `maxVal` for a date filter of `kind` on `today`: the dates
/// from `val` up to, not including, `maxVal`. `None` for the kinds that
/// need no range (a quarter or month of any year, the averages).
pub(super) fn range(kind: &str, today: f64, epoch: DateSystem) -> Option<(f64, f64)> {
    let today = today.floor();
    let fields = number_format::calendar_fields(today, epoch)?;
    let (year, month) = (i64::from(fields[0]), i64::from(fields[1]));
    // Excel's weeks start on Sunday; serial 1 (in 1900) is a Sunday.
    let weekday = (today + 6.0).rem_euclid(7.0);
    let quarter = (month - 1) / 3 * 3 + 1;
    let month_range = |offset: i64| {
        (
            serial(year, month + offset, epoch),
            serial(year, month + offset + 1, epoch),
        )
    };
    let quarter_range = |offset: i64| {
        (
            serial(year, quarter + 3 * offset, epoch),
            serial(year, quarter + 3 * offset + 3, epoch),
        )
    };
    let year_range = |offset: i64| {
        (
            serial(year + offset, 1, epoch),
            serial(year + offset + 1, 1, epoch),
        )
    };
    Some(match kind {
        "today" => (today, today + 1.0),
        "yesterday" => (today - 1.0, today),
        "tomorrow" => (today + 1.0, today + 2.0),
        "thisWeek" => (today - weekday, today - weekday + 7.0),
        "lastWeek" => (today - weekday - 7.0, today - weekday),
        "nextWeek" => (today - weekday + 7.0, today - weekday + 14.0),
        "thisMonth" => month_range(0),
        "lastMonth" => month_range(-1),
        "nextMonth" => month_range(1),
        "thisQuarter" => quarter_range(0),
        "lastQuarter" => quarter_range(-1),
        "nextQuarter" => quarter_range(1),
        "thisYear" => year_range(0),
        "lastYear" => year_range(-1),
        "nextYear" => year_range(1),
        "yearToDate" => (serial(year, 1, epoch), today + 1.0),
        _ => return None,
    })
}

/// The average of the numbers in `column` from row `first` to `end`.
pub(super) fn average(sheet: &Sheet, column: u32, first: u32, end: u32) -> f64 {
    let start = sheet.rows.partition_point(|row| row.index < first);
    let (sum, count) = sheet.rows[start..]
        .iter()
        .take_while(|row| row.index <= end)
        .filter_map(|row| find_cell(row, column))
        .filter_map(|cell| match cell.value {
            CellValue::Number(value) if value.is_finite() => Some(value),
            _ => None,
        })
        .fold((0.0, 0usize), |(sum, count), value| {
            (sum + value, count + 1)
        });
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

/// The criterion's attributes worked out afresh: the range from `today`,
/// or the column's `average`.
pub(super) fn refresh(
    node: &mut FilterCriterion,
    today: Option<f64>,
    average: f64,
    epoch: DateSystem,
) {
    let kind = node.attributes.get("type").cloned().unwrap_or_default();
    node.attributes.remove("val");
    node.attributes.remove("maxVal");
    if matches!(kind.as_str(), "aboveAverage" | "belowAverage") {
        node.attributes.insert("val".into(), average.to_string());
    } else if let Some((from, to)) = today.and_then(|today| range(&kind, today, epoch)) {
        node.attributes.insert("val".into(), from.to_string());
        node.attributes.insert("maxVal".into(), to.to_string());
    }
}

/// Whether a cell worth `value` passes a dynamic filter.
pub(super) fn matches(node: &FilterCriterion, value: &CellValue, epoch: DateSystem) -> bool {
    let CellValue::Number(number) = *value else {
        return false;
    };
    let attribute = |key: &str| {
        node.attributes
            .get(key)
            .and_then(|value| value.parse::<f64>().ok())
    };
    let kind = node
        .attributes
        .get("type")
        .map(String::as_str)
        .unwrap_or_default();
    match kind {
        "aboveAverage" => attribute("val").is_some_and(|average| number > average),
        "belowAverage" => attribute("val").is_some_and(|average| number < average),
        _ if kind.starts_with('Q') || kind.starts_with('M') => {
            let Some(fields) = number_format::calendar_fields(number, epoch) else {
                return false;
            };
            let Ok(n) = kind[1..].parse::<u32>() else {
                return false;
            };
            if kind.starts_with('Q') {
                (fields[1] - 1) / 3 + 1 == n
            } else {
                fields[1] == n
            }
        }
        _ => match (attribute("val"), attribute("maxVal")) {
            (Some(from), Some(to)) => number >= from && number < to,
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_run_from_today() {
        let epoch = DateSystem::Nineteen00;
        // 45306 is Monday 2024-01-15.
        let today = 45306.0;
        assert_eq!(range("today", today, epoch), Some((45306.0, 45307.0)));
        // The week from Sunday the 14th.
        assert_eq!(range("thisWeek", today, epoch), Some((45305.0, 45312.0)));
        // January 2024 is 45292 to 45323; December 2023 before it.
        assert_eq!(range("thisMonth", today, epoch), Some((45292.0, 45323.0)));
        assert_eq!(range("lastMonth", today, epoch), Some((45261.0, 45292.0)));
        assert_eq!(range("thisQuarter", today, epoch), Some((45292.0, 45383.0)));
        assert_eq!(range("lastYear", today, epoch), Some((44927.0, 45292.0)));
        assert_eq!(range("yearToDate", today, epoch), Some((45292.0, 45307.0)));
        assert_eq!(range("Q1", today, epoch), None);
    }
}
