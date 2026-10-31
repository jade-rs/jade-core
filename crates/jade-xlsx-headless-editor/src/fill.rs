//! The fill handle's arithmetic: what goes into the cells a drag uncovers.
//!
//! Excel reads the source run and continues it. A run of numbers goes on as a
//! linear series — the difference of two, the least-squares trend of more; a
//! lone date steps a day at a time; a day, month or quarter name from
//! Excel's built-in lists goes round its list; text ending in a number counts
//! on; and anything else — a lone number, formulas, booleans, a run with a gap or a
//! mix — repeats as a pattern, with formulas moved along the way a paste
//! moves them. Each column of a downward fill, and each row of a rightward
//! one, is its own run.
//!
//! Every filled cell also wears the format of the source cell it continues
//! from, with the source's period, as Excel's fill does.
//!
//! This lives beside the headless workbook rather than in the browser entry
//! point so the shells and the MCP server fill the same way.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_layout::number_format::is_date_format;
use jade_xlsx_reader::{translate_formula, Cell, CellRange, CellRef, CellValue, Sheet, StyleTable};

/// The direction a fill extends the source in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillAxis {
    /// The same columns, more rows.
    Rows,
    /// The same rows, more columns.
    Columns,
}

/// Which way `target` grows `source`, or `None` when it is not a fill: a fill
/// grows the source along one axis only, and never shrinks or moves it. A
/// target equal to the source is a fill of nothing.
pub fn fill_axis(source: CellRange, target: CellRange) -> Option<FillAxis> {
    let same_columns =
        target.start.column == source.start.column && target.end.column == source.end.column;
    let same_rows = target.start.row == source.start.row && target.end.row == source.end.row;
    let covers_rows = target.start.row <= source.start.row && target.end.row >= source.end.row;
    let covers_columns =
        target.start.column <= source.start.column && target.end.column >= source.end.column;
    if same_columns && covers_rows {
        Some(FillAxis::Rows)
    } else if same_rows && covers_columns {
        Some(FillAxis::Columns)
    } else {
        None
    }
}

/// One cell a fill writes.
#[derive(Debug, Clone, PartialEq)]
pub struct FillWrite {
    pub at: CellRef,
    pub value: CellInput,
    /// The style of the source cell this one continues from.
    pub style_index: Option<u32>,
}

/// What each cell of `target` outside `source` receives, in row order.
///
/// Both ranges are taken as normalised (start at the top-left).
pub fn plan_fill(
    sheet: &Sheet,
    styles: &StyleTable,
    source: CellRange,
    target: CellRange,
) -> Vec<FillWrite> {
    let Some(axis) = fill_axis(source, target) else {
        return Vec::new();
    };
    let mut writes = Vec::new();
    match axis {
        FillAxis::Rows => {
            for column in source.start.column..=source.end.column {
                let run = (source.start.row..=source.end.row)
                    .map(|row| cell_at(sheet, CellRef::new(column, row)))
                    .collect::<Vec<_>>();
                let series = Series::of(&run, sheet, styles);
                for row in target.start.row..=target.end.row {
                    if (source.start.row..=source.end.row).contains(&row) {
                        continue;
                    }
                    let position = i64::from(row) - i64::from(source.start.row);
                    let at = CellRef::new(column, row);
                    let source_reference = |index| CellRef::new(column, source.start.row + index);
                    let value = series.value_at(position, &run, at, source_reference);
                    let style_index = Some(crate::effective_style(
                        sheet,
                        source_reference(pattern_index(position, run.len())),
                    ));
                    writes.push(FillWrite {
                        at,
                        value,
                        style_index,
                    });
                }
            }
        }
        FillAxis::Columns => {
            for row in source.start.row..=source.end.row {
                let run = (source.start.column..=source.end.column)
                    .map(|column| cell_at(sheet, CellRef::new(column, row)))
                    .collect::<Vec<_>>();
                let series = Series::of(&run, sheet, styles);
                for column in target.start.column..=target.end.column {
                    if (source.start.column..=source.end.column).contains(&column) {
                        continue;
                    }
                    let position = i64::from(column) - i64::from(source.start.column);
                    let at = CellRef::new(column, row);
                    let source_reference = |index| CellRef::new(source.start.column + index, row);
                    let value = series.value_at(position, &run, at, source_reference);
                    let style_index = Some(crate::effective_style(
                        sheet,
                        source_reference(pattern_index(position, run.len())),
                    ));
                    writes.push(FillWrite {
                        at,
                        value,
                        style_index,
                    });
                }
            }
        }
    }
    writes.sort_by_key(|write| (write.at.row, write.at.column));
    writes
}

/// Which source cell a position continues from when the source repeats
/// with its own period: `position` counted from the first source cell,
/// negative above or left of it.
fn pattern_index(position: i64, period: usize) -> u32 {
    position.rem_euclid(period as i64) as u32
}

fn cell_at(sheet: &Sheet, reference: CellRef) -> Option<&Cell> {
    sheet.cell_at(reference)
}

/// How one run continues.
#[derive(Debug, Clone, PartialEq)]
enum Series {
    /// `intercept + slope × position`, position counted from the first
    /// source cell.
    Linear { intercept: f64, slope: f64 },
    /// Text whose number counts along a line, the words around it kept.
    Counting {
        prefix: String,
        suffix: String,
        /// Digits to pad to, when the source was zero-padded.
        width: usize,
        intercept: f64,
        slope: f64,
    },
    /// Names from one of Excel's built-in lists, going round it `step`
    /// entries at a time from `start`, in the source's capitals.
    List {
        list: &'static [&'static str],
        start: usize,
        step: i64,
        case: Case,
    },
    /// The source repeated with its own period.
    Repeat,
}

/// How a list name was capitalised: Excel continues `JAN` as `FEB` and
/// `jan` as `feb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    AsListed,
    Upper,
    Lower,
}

/// Excel's built-in custom lists (File ▸ Options ▸ Advanced ▸ Edit Custom
/// Lists) in an English and a Japanese Excel. `1月`…`12月` and the quarters
/// would otherwise count on past their last entry.
const LISTS: &[&[&str]] = &[
    &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    &[
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    &[
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    &[
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    &["日", "月", "火", "水", "木", "金", "土"],
    &[
        "日曜日",
        "月曜日",
        "火曜日",
        "水曜日",
        "木曜日",
        "金曜日",
        "土曜日",
    ],
    &[
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    &["第1四半期", "第2四半期", "第3四半期", "第4四半期"],
    &[
        "睦月",
        "如月",
        "弥生",
        "卯月",
        "皐月",
        "水無月",
        "文月",
        "葉月",
        "長月",
        "神無月",
        "霜月",
        "師走",
    ],
    &[
        "子", "丑", "寅", "卯", "辰", "巳", "午", "未", "申", "酉", "戌", "亥",
    ],
    &["甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸"],
    &["Q1", "Q2", "Q3", "Q4"],
    &["Qtr1", "Qtr2", "Qtr3", "Qtr4"],
    &["Quarter1", "Quarter2", "Quarter3", "Quarter4"],
];

/// Where `text` sits in `list`, and in what capitals.
fn list_entry(list: &[&str], text: &str) -> Option<(usize, Case)> {
    let index = list
        .iter()
        .position(|entry| entry.eq_ignore_ascii_case(text))?;
    let entry = list[index];
    let case = if entry == text {
        Case::AsListed
    } else if text == entry.to_ascii_uppercase() {
        Case::Upper
    } else if text == entry.to_ascii_lowercase() {
        Case::Lower
    } else {
        // `mAY`: not a way Excel recognises.
        return None;
    };
    Some((index, case))
}

/// The run going round `list`, when every text is an entry of it in the
/// same capitals and each step is the same counted round the list: `Sat,
/// Sun` steps one, `Mon, Wed, Fri` two. `May` alone is taken as the short
/// month it is first listed as; `May, June` as the long months.
fn list_series(list: &'static [&'static str], texts: &[&str]) -> Option<Series> {
    let entries = texts
        .iter()
        .map(|text| list_entry(list, text))
        .collect::<Option<Vec<_>>>()?;
    let (start, case) = *entries.first()?;
    if entries.iter().any(|&(_, other)| other != case) {
        return None;
    }
    let period = list.len() as i64;
    let steps = entries
        .windows(2)
        .map(|pair| (pair[1].0 as i64 - pair[0].0 as i64).rem_euclid(period))
        .collect::<Vec<_>>();
    if steps.windows(2).any(|pair| pair[0] != pair[1]) {
        return None;
    }
    Some(Series::List {
        list,
        start,
        step: steps.first().copied().unwrap_or(1),
        case,
    })
}

impl Series {
    fn of(run: &[Option<&Cell>], sheet: &Sheet, styles: &StyleTable) -> Series {
        if let Some(numbers) = run
            .iter()
            .map(|cell| match cell {
                Some(Cell {
                    value: CellValue::Number(number),
                    formula: None,
                    ..
                }) => Some(*number),
                _ => None,
            })
            .collect::<Option<Vec<f64>>>()
        {
            return match numbers.as_slice() {
                [only] => {
                    // A lone date is the start of a calendar; a lone number
                    // is just copied, as in Excel without Ctrl.
                    let cell = run[0].expect("a number has a cell");
                    if shows_a_date(cell, sheet, styles) {
                        Series::Linear {
                            intercept: *only,
                            slope: 1.0,
                        }
                    } else {
                        Series::Repeat
                    }
                }
                _ => {
                    let (intercept, slope) = least_squares(&numbers);
                    Series::Linear { intercept, slope }
                }
            };
        }

        if let Some(texts) = run
            .iter()
            .map(|cell| match cell {
                Some(Cell {
                    value: CellValue::Text(text),
                    formula: None,
                    ..
                }) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
        {
            if let Some(series) = LISTS.iter().find_map(|list| list_series(list, &texts)) {
                return series;
            }
        }

        if let Some(counted) = run
            .iter()
            .map(|cell| match cell {
                Some(Cell {
                    value: CellValue::Text(text),
                    formula: None,
                    ..
                }) => split_trailing_number(text),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
        {
            let (prefix, digits, suffix) = &counted[0];
            if counted.iter().all(|(other_prefix, _, other_suffix)| {
                other_prefix == prefix && other_suffix == suffix
            }) {
                let numbers = counted
                    .iter()
                    .map(|(_, digits, _)| digits.parse::<f64>().unwrap_or_default())
                    .collect::<Vec<_>>();
                let (intercept, slope) = match numbers.as_slice() {
                    [only] => (*only, 1.0),
                    _ => least_squares(&numbers),
                };
                return Series::Counting {
                    prefix: prefix.clone(),
                    suffix: suffix.clone(),
                    width: if digits.starts_with('0') && digits.len() > 1 {
                        digits.len()
                    } else {
                        0
                    },
                    intercept,
                    slope,
                };
            }
        }

        Series::Repeat
    }

    fn value_at(
        &self,
        position: i64,
        run: &[Option<&Cell>],
        at: CellRef,
        source_reference: impl Fn(u32) -> CellRef,
    ) -> CellInput {
        match self {
            Series::Linear { intercept, slope } => {
                CellInput::Number(round_to_15_digits(intercept + slope * position as f64))
            }
            Series::Counting {
                prefix,
                suffix,
                width,
                intercept,
                slope,
            } => {
                let number = (intercept + slope * position as f64).round() as i64;
                CellInput::Text(format!("{prefix}{number:0width$}{suffix}", width = *width))
            }
            Series::List {
                list,
                start,
                step,
                case,
            } => {
                let index = (*start as i64 + step * position).rem_euclid(list.len() as i64);
                let entry = list[index as usize];
                CellInput::Text(match case {
                    Case::AsListed => entry.to_string(),
                    Case::Upper => entry.to_ascii_uppercase(),
                    Case::Lower => entry.to_ascii_lowercase(),
                })
            }
            Series::Repeat => {
                let index = pattern_index(position, run.len());
                match run[index as usize] {
                    Some(cell) => match &cell.formula {
                        Some(formula) => CellInput::Formula(translate_formula(
                            formula,
                            source_reference(index),
                            at,
                        )),
                        None => match &cell.value {
                            CellValue::Number(number) => CellInput::Number(*number),
                            CellValue::Text(text) => CellInput::Text(text.clone()),
                            CellValue::Bool(flag) => CellInput::Bool(*flag),
                            CellValue::Error(error) => CellInput::Text(error.clone()),
                            CellValue::Empty => CellInput::Blank,
                        },
                    },
                    None => CellInput::Blank,
                }
            }
        }
    }
}

/// Whether the cell's number format shows it as a date or a time.
fn shows_a_date(cell: &Cell, sheet: &Sheet, styles: &StyleTable) -> bool {
    styles
        .cell_format(sheet.effective_style_index(cell.reference, cell.style_index))
        .and_then(|format| styles.number_format_code(format))
        .is_some_and(is_date_format)
}

/// The line through `(0, y₀), (1, y₁), …` that minimises the squared error —
/// exact for an arithmetic progression, the trend Excel extends otherwise.
fn least_squares(values: &[f64]) -> (f64, f64) {
    let count = values.len() as f64;
    let x_mean = (count - 1.0) / 2.0;
    let y_mean = values.iter().sum::<f64>() / count;
    let mut covariance = 0.0;
    let mut variance = 0.0;
    for (index, value) in values.iter().enumerate() {
        let dx = index as f64 - x_mean;
        covariance += dx * (value - y_mean);
        variance += dx * dx;
    }
    let slope = if variance == 0.0 {
        0.0
    } else {
        covariance / variance
    };
    (y_mean - slope * x_mean, slope)
}

/// Excel keeps fifteen significant digits; rounding to them turns the
/// `3.0000000000000004` a trend fit can produce back into the `3` a person
/// typed.
fn round_to_15_digits(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    format!("{value:.14e}").parse().unwrap_or(value)
}

/// `Item 12` → `("Item ", "12", "")`, splitting at the last run of ASCII
/// digits; `None` when there is no number to count.
fn split_trailing_number(text: &str) -> Option<(String, String, String)> {
    let end = text.rfind(|c: char| c.is_ascii_digit())? + 1;
    // The character before the digits may be several bytes wide (`第3回`).
    let start = text[..end]
        .rfind(|c: char| !c.is_ascii_digit())
        .map_or(0, |index| {
            index + text[index..].chars().next().map_or(1, char::len_utf8)
        });
    // A number too long to count is a serial, not a counter.
    if end - start > 15 {
        return None;
    }
    Some((
        text[..start].to_string(),
        text[start..end].to_string(),
        text[end..].to_string(),
    ))
}
