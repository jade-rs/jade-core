use crate::tz::Tz;
use chrono::DateTime;
use chrono::Datelike;
use chrono::Months;
use chrono::NaiveDateTime;
use chrono::NaiveTime;
use chrono::Timelike;

const SECONDS_PER_DAY: i32 = 86_400;
const SECONDS_PER_DAY_F64: f64 = SECONDS_PER_DAY as f64;

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0) && (year % 100 != 0 || year % 400 == 0)
}

fn last_day_of_feb(year: i32) -> u32 {
    if is_leap_year(year) {
        29
    } else {
        28
    }
}

// Maps a date serial number (date part only) to the Excel (year, month, day) it
// represents, replicating Excel's fictional 29 February 1900 (the "1900
// leap-year bug") so DAYS360 matches Excel for early-1900 serials. Returns
// `None` when the serial is outside `0..=MAXIMUM_DATE_SERIAL_NUMBER`, which Excel
// reports as `#NUM!`.
fn excel_serial_to_ymd(serial: i64) -> Option<(i32, u32, u32)> {
    if crate::date_system::is_1904() {
        let date = from_excel_date(serial).ok()?;
        return Some((date.year(), date.month(), date.day()));
    }
    if !(0..=crate::date_system::maximum_serial() as i64).contains(&serial) {
        return None;
    }
    // Excel renders serial 0 as "0 January 1900"; DAYS360 reads it as day 0 of
    // January 1900 (not 31 December 1899), which is what makes it antisymmetric.
    if serial == 0 {
        return Some((1900, 1, 0));
    }
    // Excel's non-existent 29 February 1900 (serial 60).
    if serial == 60 {
        return Some((1900, 2, 29));
    }
    let date = if serial < 60 {
        // Below the fictional leap day Excel counts serial 1 as 1900-01-01, so
        // serial N is N days after 1899-12-31 (serial 0).
        let base = chrono::NaiveDate::from_ymd_opt(1899, 12, 31)?;
        base + chrono::Duration::days(serial)
    } else {
        // From 1900-03-01 (serial 61) onwards IronCalc's serials match Excel.
        from_excel_date(serial).ok()?
    };
    Some((date.year(), date.month(), date.day()))
}

// Converts a raw numeric argument to a date serial (date part) for DAYS360,
// rounding the time-of-day to the nearest second before discarding it. Excel
// rolls e.g. 46000.999999 up to 46001 when extracting the date part.
fn days360_serial(raw: f64) -> i64 {
    ((raw * SECONDS_PER_DAY_F64).round() / SECONDS_PER_DAY_F64).floor() as i64
}

// Days d1→d2 via the US (NASD) 30/360 convention, as used by the worksheet
// DAYS360 function. Note this differs from the bond 30/360 (`days_30_360_us`):
// DAYS360 only promotes the *start* date when it is the last day of February,
// and never adjusts the end date for February (it touches the end date only
// when its day-of-month is literally 31).
fn days360_us(start: (i32, u32, u32), end: (i32, u32, u32)) -> f64 {
    let (y1, m1, day1) = start;
    let (y2, m2, day2) = end;
    let mut day1 = day1 as i32;
    let mut day2 = day2 as i32;

    if day1 == 31 || (m1 == 2 && day1 == last_day_of_feb(y1) as i32) {
        day1 = 30;
    }
    if day2 == 31 && day1 == 30 {
        day2 = 30;
    }
    (360 * (y2 - y1) + 30 * (m2 as i32 - m1 as i32) + (day2 - day1)) as f64
}

// Days d1→d2 via the European 30/360 convention used by DAYS360 (method = TRUE):
// any day-of-month equal to 31 becomes 30, with no February handling.
fn days360_eu(start: (i32, u32, u32), end: (i32, u32, u32)) -> f64 {
    let (y1, m1, day1) = start;
    let (y2, m2, day2) = end;
    let day1 = (day1 as i32).min(30);
    let day2 = (day2 as i32).min(30);
    (360 * (y2 - y1) + 30 * (m2 as i32 - m1 as i32) + (day2 - day1)) as f64
}

/// Maps a date to the WEEKDAY number for the given `return_type`.
/// Returns `Error::NUM` for an invalid `return_type`.
fn weekday_number(date: chrono::NaiveDate, return_type: i32) -> Result<f64, Error> {
    let weekday = date.weekday();
    let num = match return_type {
        1 => weekday.num_days_from_sunday() + 1,
        2 => weekday.number_from_monday(),
        3 => (weekday.number_from_monday() - 1) % 7, // 0-based Monday start
        11..=17 => {
            let start = (return_type - 11) as u32; // 0 = Monday, 6 = Sunday
            let zero_based = weekday.number_from_monday() - 1; // 0..6, Monday = 0
            ((zero_based + 7 - start) % 7) + 1
        }
        _ => return Err(Error::NUM),
    };
    Ok(num as f64)
}

// Port of `isFeb29BetweenConsecutiveYears` from ExcelFinancialFunctions. Only
// called for dates at most one year apart (`y1 == y2` or `y2 == y1 + 1`).
fn is_feb29_between_consecutive_years(start: chrono::NaiveDate, end: chrono::NaiveDate) -> bool {
    let (y1, m1) = (start.year(), start.month());
    let (y2, m2) = (end.year(), end.month());
    if y1 == y2 {
        is_leap_year(y1) && m1 <= 2 && m2 > 2
    } else if y2 == y1 + 1 {
        if is_leap_year(y1) {
            m1 <= 2
        } else if is_leap_year(y2) {
            m2 > 2
        } else {
            false
        }
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Helper macros to eliminate boilerplate in date/time component extraction
// functions (DAY, MONTH, YEAR, HOUR, MINUTE, SECOND).
// ---------------------------------------------------------------------------

// Generate DAY / MONTH / YEAR helpers – simply convert the serial number to a
// NaiveDate and return the requested component as a number.
macro_rules! date_part_fn {
    ($name:ident, $part:tt) => {
        pub(crate) fn $name(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
            if args.len() != 1 {
                return CalcResult::new_args_number_error(cell);
            }
            // Read the serial as Excel does — 0 is 0 January 1900 and 60 the
            // fictional 29 February — and return the requested component.
            // Broadcasts element-wise over ranges/arrays.
            self.apply_number_unary(&args[0], cell, |f| {
                match excel_serial_to_ymd(f.floor() as i64) {
                    Some(ymd) => Ok(ymd.$part as f64),
                    None => Err(Error::NUM),
                }
            })
        }
    };
}

// Generate HOUR / MINUTE / SECOND helpers – extract the desired component from
// a day-fraction value. We pass an extraction closure so each helper can keep
// its own formula while sharing the surrounding boilerplate.
macro_rules! time_part_fn {
    ($name:ident, $extract:expr) => {
        pub(crate) fn $name(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
            if args.len() != 1 {
                return CalcResult::new_args_number_error(cell);
            }
            self.map_scalar_values(&args[0], cell, |model, value| {
                let value = match model.cast_to_number(value, cell) {
                    Ok(value) => value,
                    Err(error) => return error,
                };
                if !value.is_finite() || value < 0.0 {
                    return CalcResult::new_error(Error::NUM, cell, "Invalid time".into());
                }
                CalcResult::Number(($extract)(value))
            })
        }
    };
}

use crate::expressions::types::CellReferenceIndex;
use crate::formatter::dates::date_to_serial_number;
use crate::formatter::dates::permissive_date_to_serial_number;
use crate::formatter::dates::DATE_OUT_OF_RANGE_MESSAGE;
use crate::number_format::to_precision;
use crate::{
    calc_result::CalcResult,
    expressions::parser::{ArrayNode, Node},
    expressions::token::Error,
    formatter::dates::from_excel_date,
    model::Model,
};

#[derive(Debug, Clone, Copy)]
enum WeekendPattern {
    SatSun,
    SunMon,
    MonTue,
    TueWed,
    WedThu,
    ThuFri,
    FriSat,
    SunOnly,
    MonOnly,
    TueOnly,
    WedOnly,
    ThuOnly,
    FriOnly,
    SatOnly,
}

impl std::convert::TryFrom<i32> for WeekendPattern {
    type Error = ();
    fn try_from(code: i32) -> Result<Self, Self::Error> {
        Ok(match code {
            1 => Self::SatSun,
            2 => Self::SunMon,
            3 => Self::MonTue,
            4 => Self::TueWed,
            5 => Self::WedThu,
            6 => Self::ThuFri,
            7 => Self::FriSat,
            11 => Self::SunOnly,
            12 => Self::MonOnly,
            13 => Self::TueOnly,
            14 => Self::WedOnly,
            15 => Self::ThuOnly,
            16 => Self::FriOnly,
            17 => Self::SatOnly,
            _ => return Err(()),
        })
    }
}

impl WeekendPattern {
    fn to_mask(self) -> [bool; 7] {
        match self {
            Self::SatSun => [false, false, false, false, false, true, true],
            Self::SunMon => [true, false, false, false, false, false, true],
            Self::MonTue => [true, true, false, false, false, false, false],
            Self::TueWed => [false, true, true, false, false, false, false],
            Self::WedThu => [false, false, true, true, false, false, false],
            Self::ThuFri => [false, false, false, true, true, false, false],
            Self::FriSat => [false, false, false, false, true, true, false],
            Self::SunOnly => [false, false, false, false, false, false, true],
            Self::MonOnly => [true, false, false, false, false, false, false],
            Self::TueOnly => [false, true, false, false, false, false, false],
            Self::WedOnly => [false, false, true, false, false, false, false],
            Self::ThuOnly => [false, false, false, true, false, false, false],
            Self::FriOnly => [false, false, false, false, true, false, false],
            Self::SatOnly => [false, false, false, false, false, true, false],
        }
    }
}

fn parse_time_string(text: &str) -> Option<f64> {
    let text = text.trim();

    // First, try custom parsing for edge cases like "24:00:00", "23:60:00", "23:59:60"
    // that need normalization to match Excel behavior
    if let Some(time_fraction) = parse_time_with_normalization(text) {
        return Some(time_fraction);
    }

    // First, try manual parsing for simple "N PM" / "N AM" format (case-insensitive)
    if let Some((hour_str, is_pm)) = parse_simple_am_pm(text) {
        if let Ok(hour) = hour_str.parse::<u32>() {
            if (1..=12).contains(&hour) {
                let hour_24 = if is_pm {
                    if hour == 12 {
                        12
                    } else {
                        hour + 12
                    }
                } else if hour == 12 {
                    0
                } else {
                    hour
                };
                let time = NaiveTime::from_hms_opt(hour_24, 0, 0)?;
                return Some(time.num_seconds_from_midnight() as f64 / SECONDS_PER_DAY_F64);
            }
        }
    }

    // Standard patterns
    let patterns_time = ["%H:%M:%S", "%H:%M", "%I:%M %p", "%I %p", "%I:%M:%S %p"];
    for p in patterns_time {
        if let Ok(t) = NaiveTime::parse_from_str(text, p) {
            return Some(t.num_seconds_from_midnight() as f64 / SECONDS_PER_DAY_F64);
        }
    }

    let patterns_dt = [
        // ISO formats
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        // Excel-style date formats with AM/PM
        "%d-%b-%Y %I:%M:%S %p", // "22-Aug-2011 6:35:00 AM"
        "%d-%b-%Y %I:%M %p",    // "22-Aug-2011 6:35 AM"
        "%d-%b-%Y %H:%M:%S",    // "22-Aug-2011 06:35:00"
        "%d-%b-%Y %H:%M",       // "22-Aug-2011 06:35"
        // US date formats with AM/PM
        "%m/%d/%Y %I:%M:%S %p", // "8/22/2011 6:35:00 AM"
        "%m/%d/%Y %I:%M %p",    // "8/22/2011 6:35 AM"
        "%m/%d/%Y %H:%M:%S",    // "8/22/2011 06:35:00"
        "%m/%d/%Y %H:%M",       // "8/22/2011 06:35"
        // European date formats with AM/PM
        "%d/%m/%Y %I:%M:%S %p", // "22/8/2011 6:35:00 AM"
        "%d/%m/%Y %I:%M %p",    // "22/8/2011 6:35 AM"
        "%d/%m/%Y %H:%M:%S",    // "22/8/2011 06:35:00"
        "%d/%m/%Y %H:%M",       // "22/8/2011 06:35"
    ];
    for p in patterns_dt {
        if let Ok(dt) = NaiveDateTime::parse_from_str(text, p) {
            return Some(dt.time().num_seconds_from_midnight() as f64 / SECONDS_PER_DAY_F64);
        }
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.time().num_seconds_from_midnight() as f64 / SECONDS_PER_DAY_F64);
    }
    None
}

fn parse_localized_date_time_string(text: &str, locale: &crate::locale::Locale) -> Option<f64> {
    let text = text.trim();
    let colon = text.find(':')?;
    // The whitespace immediately before the clock separates it from a date
    // whose month name may itself be surrounded by spaces.
    let boundary = text[..colon].rfind(char::is_whitespace)?;
    let (date, clock) = text.split_at(boundary);
    parse_datevalue_text(date, locale).ok()?;
    parse_time_string(clock.trim_start())
}

// Custom parser that handles time normalization like Excel does
fn parse_time_with_normalization(text: &str) -> Option<f64> {
    // Try to parse H:M:S format with potential overflow values
    let parts: Vec<&str> = text.split(':').collect();

    if parts.len() == 3 {
        // H:M:S format
        if let (Ok(h), Ok(m), Ok(s)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<i32>(),
            parts[2].parse::<i32>(),
        ) {
            // Only normalize specific edge cases that Excel handles
            // Don't normalize arbitrary large values like 25:00:00
            if should_normalize_time_components(h, m, s) {
                return Some(normalize_time_components(h, m, s));
            }
        }
    } else if parts.len() == 2 {
        // H:M format (assume seconds = 0)
        if let (Ok(h), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
            // Only normalize specific edge cases
            if should_normalize_time_components(h, m, 0) {
                return Some(normalize_time_components(h, m, 0));
            }
        }
    }

    None
}

// Normalize time components with overflow handling (like Excel)
fn normalize_time_components(hour: i32, minute: i32, second: i32) -> f64 {
    // Convert everything to total seconds
    let mut total_seconds = hour * 3600 + minute * 60 + second;

    // Handle negative values by wrapping around
    if total_seconds < 0 {
        total_seconds = total_seconds.rem_euclid(SECONDS_PER_DAY);
    }

    // Normalize to within a day (0-86399 seconds)
    total_seconds %= SECONDS_PER_DAY;

    // Convert to fraction of a day
    total_seconds as f64 / SECONDS_PER_DAY_F64
}

// Check if time components should be normalized (only specific Excel edge cases)
fn should_normalize_time_components(hour: i32, minute: i32, second: i32) -> bool {
    // Only normalize these specific cases that Excel handles:
    // 1. Hour 24 with valid minutes/seconds
    // 2. Hour 23 with minute 60 (becomes 24:00)
    // 3. Any time with second 60 that normalizes to exactly 24:00

    if hour == 24 && (0..=59).contains(&minute) && (0..=59).contains(&second) {
        return true; // 24:MM:SS -> normalize to next day
    }

    if hour == 23 && minute == 60 && (0..=59).contains(&second) {
        return true; // 23:60:SS -> normalize to 24:00:SS
    }

    if (0..=23).contains(&hour) && (0..=59).contains(&minute) && second == 60 {
        // Check if this normalizes to exactly 24:00:00
        let total_seconds = hour * 3600 + minute * 60 + second;
        return total_seconds == SECONDS_PER_DAY; // Exactly 24:00:00
    }

    false
}

// Helper function to parse simple "N PM" / "N AM" formats (case-insensitive)
fn parse_simple_am_pm(text: &str) -> Option<(&str, bool)> {
    let (hour_part, suffix) = text.rsplit_once(char::is_whitespace)?;

    let suffix = suffix.strip_suffix('.').unwrap_or(suffix);

    let is_pm = if suffix.eq_ignore_ascii_case("pm") {
        true
    } else if suffix.eq_ignore_ascii_case("am") {
        false
    } else {
        return None;
    };

    let hour_part = hour_part.trim();

    if !hour_part.is_empty() && hour_part.chars().all(|c| c.is_ascii_digit()) {
        return Some((hour_part, is_pm));
    }
    None
}

fn parse_day_simple(day_str: &str) -> Result<u32, String> {
    let bytes_len = day_str.len();
    if bytes_len == 0 || bytes_len > 2 {
        return Err("Not a valid day".to_string());
    }
    match day_str.parse::<u32>() {
        Ok(y) => Ok(y),
        Err(_) => Err("Not a valid day".to_string()),
    }
}

fn parse_month_simple(month_str: &str) -> Result<u32, String> {
    let bytes_len = month_str.len();
    if bytes_len == 0 {
        return Err("Not a valid month".to_string());
    }
    if bytes_len <= 2 {
        // Numeric month representation. Ensure it is within the valid range 1-12.
        return match month_str.parse::<u32>() {
            Ok(m) if (1..=12).contains(&m) => Ok(m),
            _ => Err("Not a valid month".to_string()),
        };
    }

    // Textual month representations.
    // Use standard 3-letter abbreviations (e.g. "Sep") but also accept the legacy "Sept".
    let month_names_short = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month_names_long = [
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
    ];
    if let Some(m) = month_names_short
        .iter()
        .position(|&r| r.eq_ignore_ascii_case(month_str))
    {
        return Ok(m as u32 + 1);
    }
    // Special-case the non-standard abbreviation "Sept" so older inputs still work.
    if month_str.eq_ignore_ascii_case("Sept") {
        return Ok(9);
    }

    if let Some(m) = month_names_long
        .iter()
        .position(|&r| r.eq_ignore_ascii_case(month_str))
    {
        return Ok(m as u32 + 1);
    }
    Err("Not a valid month".to_string())
}

fn parse_year_simple(year_str: &str) -> Result<i32, String> {
    let bytes_len = year_str.len();
    if bytes_len != 2 && bytes_len != 4 {
        return Err("Not a valid year".to_string());
    }
    let y = year_str
        .parse::<i32>()
        .map_err(|_| "Not a valid year".to_string())?;
    if y < 30 && bytes_len == 2 {
        Ok(2000 + y)
    } else if y < 100 && bytes_len == 2 {
        Ok(1900 + y)
    } else {
        Ok(y)
    }
}

pub(crate) fn parse_datevalue_text(
    value: &str,
    locale: &crate::locale::Locale,
) -> Result<i32, String> {
    // A month name can contain T and dates can contain spaces. Only remove
    // an ISO time separator or the whitespace preceding a clock value.
    let mut date_str = value.trim();
    if let Some(idx) = date_str
        .as_bytes()
        .windows(3)
        .position(|part| part[0].is_ascii_digit() && part[1] == b'T' && part[2].is_ascii_digit())
        .map(|idx| idx + 1)
    {
        date_str = &date_str[..idx];
    }
    if let Some(idx) = date_str.find(':') {
        if let Some(start) = date_str[..idx].rfind(char::is_whitespace) {
            date_str = date_str[..start].trim_end();
        }
    }

    let separator = if date_str.contains('/') {
        '/'
    } else if date_str.contains('-') {
        '-'
    } else if date_str.contains('.') && !date_str.chars().any(char::is_whitespace) {
        '.'
    } else {
        ' '
    };

    let mut parts: Vec<&str> = if separator == ' ' {
        date_str
            .split_whitespace()
            .map(|s| s.trim_end_matches(','))
            .collect()
    } else {
        date_str.split(separator).map(|s| s.trim()).collect()
    };
    if parts.len() != 3 {
        return Err("Not a valid date".to_string());
    }

    // Identify the year: prefer the one that is four-digit numeric, otherwise assume the third part.
    let mut year_idx: usize = 2;
    for (idx, p) in parts.iter().enumerate() {
        if p.len() == 4 && p.chars().all(char::is_numeric) {
            year_idx = idx;
            break;
        }
    }

    let year_str = parts[year_idx];
    // Remove the year from the remaining vector to process day / month.
    parts.remove(year_idx);
    let part1 = parts[0];
    let part2 = parts[1];

    // Helper closures
    let is_numeric = |s: &str| s.chars().all(char::is_numeric);

    // Determine month and day.
    let (month_str, day_str) = if !is_numeric(part1) {
        // textual month in first
        (part1, part2)
    } else if !is_numeric(part2) {
        // textual month in second
        (part2, part1)
    } else {
        // ISO dates are year/month/day; other numeric dates follow the
        // configured regional date order rather than guessing from values.
        if year_idx != 0 && locale.dates.date_formats.short.starts_with('d') {
            (part2, part1)
        } else {
            (part1, part2)
        }
    };

    let day = parse_day_simple(day_str)?;
    let month = locale
        .dates
        .months
        .iter()
        .position(|name| name.to_lowercase() == month_str.to_lowercase())
        .or_else(|| {
            locale
                .dates
                .months_short
                .iter()
                .position(|name| name.to_lowercase() == month_str.to_lowercase())
        })
        .map(|index| index as u32 + 1)
        .map(Ok)
        .unwrap_or_else(|| parse_month_simple(month_str))?;
    let year = parse_year_simple(year_str)?;

    // Excel 1900 leap-year bug: 29-Feb-1900 is treated as serial 60
    if !crate::date_system::is_1904() && year == 1900 && month == 2 && day == 29 {
        return Ok(60);
    }

    match date_to_serial_number(day, month, year) {
        Ok(n)
            if (crate::date_system::minimum_serial()..=crate::date_system::maximum_serial())
                .contains(&n) =>
        {
            Ok(n)
        }
        _ => Err("Not a valid date".to_string()),
    }
}

impl<'a> Model<'a> {
    fn cast_date_serial(
        &mut self,
        result: CalcResult,
        cell: CellReferenceIndex,
    ) -> Result<i64, CalcResult> {
        match result {
            CalcResult::Number(f) if f.is_finite() => Ok(f.floor() as i64),
            CalcResult::Number(_) => Err(CalcResult::new_error(
                Error::NUM,
                cell,
                DATE_OUT_OF_RANGE_MESSAGE.into(),
            )),
            CalcResult::String(s) => match parse_datevalue_text(&s, self.locale) {
                Ok(n) => Ok(n as i64),
                Err(_) => Err(CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid date".to_string(),
                }),
            },
            CalcResult::Boolean(b) => {
                if b {
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
            error @ CalcResult::Error { .. } => Err(error),
            CalcResult::Range { .. } => Err(CalcResult::Error {
                error: Error::NIMPL,
                origin: cell,
                message: "Arrays not supported yet".to_string(),
            }),
            CalcResult::EmptyCell | CalcResult::EmptyArg => Ok(0),
            CalcResult::Array(_) | CalcResult::Lambda(_) => Err(CalcResult::Error {
                error: Error::NIMPL,
                origin: cell,
                message: "Arrays not supported yet".to_string(),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Auto-generated DATE part helpers (DAY / MONTH / YEAR)
    // -----------------------------------------------------------------------
    date_part_fn!(fn_day, 2);
    date_part_fn!(fn_month, 1);
    date_part_fn!(fn_year, 0);

    pub(crate) fn fn_eomonth(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.month_shift(args, cell, true)
    }

    fn month_shift(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        end_of_month: bool,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let start = match model.cast_to_number(values[0].clone(), cell) {
                Ok(number) => number,
                Err(error) => return error,
            };
            if !start.is_finite() {
                return CalcResult::new_error(Error::NUM, cell, DATE_OUT_OF_RANGE_MESSAGE.into());
            }
            let date = match model.excel_date(start.floor() as i64, cell) {
                Ok(date) => date,
                Err(error) => return error,
            };
            // Retain EOMONTH's existing Boolean-month coercion policy.
            if end_of_month && matches!(values[1], CalcResult::Boolean(_)) {
                return CalcResult::new_error(Error::VALUE, cell, "Expected numeric months".into());
            }
            let months = match model.cast_to_number(values[1].clone(), cell) {
                Ok(number) => number.trunc(),
                Err(error) => return error,
            };
            if !months.is_finite() || months.abs() > u32::MAX as f64 {
                return CalcResult::new_error(Error::NUM, cell, DATE_OUT_OF_RANGE_MESSAGE.into());
            }
            let offset = Months::new(months.abs() as u32);
            let shifted = if months >= 0.0 {
                date.checked_add_months(offset)
            } else {
                date.checked_sub_months(offset)
            };
            let Some(shifted) = shifted else {
                return CalcResult::new_error(Error::NUM, cell, DATE_OUT_OF_RANGE_MESSAGE.into());
            };
            let serial = if end_of_month {
                let (year, month) = if shifted.month() == 12 {
                    (shifted.year() + 1, 1)
                } else {
                    (shifted.year(), shifted.month() + 1)
                };
                match date_to_serial_number(1, month, year) {
                    Ok(serial) => serial - 1,
                    Err(message) => return CalcResult::new_error(Error::NUM, cell, message),
                }
            } else {
                crate::formatter::dates::convert_to_serial_number(shifted)
            };
            if serial < crate::date_system::minimum_serial()
                || serial > crate::date_system::maximum_serial()
            {
                return CalcResult::new_error(Error::NUM, cell, DATE_OUT_OF_RANGE_MESSAGE.into());
            }
            CalcResult::Number(serial as f64)
        })
    }

    // year, month, day
    pub(crate) fn fn_date(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 3 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let mut components = [0.0; 3];
            for (index, value) in values.iter().enumerate() {
                components[index] = match model.cast_to_number(value.clone(), cell) {
                    Ok(number) => number,
                    Err(error) => return error,
                };
            }
            if components.iter().any(|number| !number.is_finite()) {
                return CalcResult::new_error(Error::NUM, cell, DATE_OUT_OF_RANGE_MESSAGE.into());
            }
            let year = components[0].floor();
            if !(0.0..=9999.0).contains(&year) {
                return CalcResult::new_error(Error::NUM, cell, DATE_OUT_OF_RANGE_MESSAGE.into());
            }
            match permissive_date_to_serial_number(
                components[2].floor() as i32,
                components[1].floor() as i32,
                year as i32,
            ) {
                Ok(serial) => CalcResult::Number(serial as f64),
                Err(_) => CalcResult::new_error(Error::NUM, cell, DATE_OUT_OF_RANGE_MESSAGE.into()),
            }
        })
    }

    // date, months
    pub(crate) fn fn_edate(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.month_shift(args, cell, false)
    }

    /// Walk a scalar / range / array node and invoke the provided closure with every
    /// numeric date serial that is encountered.
    fn holiday_serial(&self, number: f64, cell: CellReferenceIndex) -> Result<i64, CalcResult> {
        if !number.is_finite() {
            return Err(CalcResult::new_error(
                Error::NUM,
                cell,
                "Invalid holiday date".into(),
            ));
        }
        let serial = number.floor() as i64;
        self.excel_date(serial, cell)?;
        Ok(serial)
    }

    fn collect_serial_numbers<F>(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
        mut handle: F,
    ) -> Result<(), CalcResult>
    where
        F: FnMut(i64) -> Result<(), CalcResult>,
    {
        match self.evaluate_node_in_context(node, cell) {
            CalcResult::Number(v) => {
                let serial = self.holiday_serial(v, cell)?;
                handle(serial)?;
            }
            CalcResult::Range { left, right } => {
                if left.sheet != right.sheet {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "Ranges are in different sheets".to_string(),
                    ));
                }
                for row in left.row..=right.row {
                    for column in left.column..=right.column {
                        match self.evaluate_cell(CellReferenceIndex {
                            sheet: left.sheet,
                            row,
                            column,
                        }) {
                            CalcResult::Number(v) => {
                                let serial = self.holiday_serial(v, cell)?;
                                handle(serial)?;
                            }
                            CalcResult::EmptyArg | CalcResult::EmptyCell => {}
                            e @ CalcResult::Error { .. } => return Err(e),
                            _ => {
                                return Err(CalcResult::new_error(
                                    Error::VALUE,
                                    cell,
                                    "Invalid holiday date".to_string(),
                                ))
                            }
                        }
                    }
                }
            }
            CalcResult::Array(array) => {
                for row in array {
                    for value in row {
                        match value {
                            ArrayNode::Number(num) => {
                                let serial = self.holiday_serial(num, cell)?;
                                handle(serial)?;
                            }
                            ArrayNode::Empty => {}
                            ArrayNode::Error(error) => {
                                return Err(CalcResult::Error {
                                    error,
                                    origin: cell,
                                    message: "Error in array".to_string(),
                                });
                            }
                            _ => {
                                return Err(CalcResult::new_error(
                                    Error::VALUE,
                                    cell,
                                    "Invalid holiday date".to_string(),
                                ))
                            }
                        }
                    }
                }
            }
            CalcResult::EmptyArg | CalcResult::EmptyCell => {}
            e @ CalcResult::Error { .. } => return Err(e),
            _ => {
                return Err(CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "Invalid holiday date".to_string(),
                ))
            }
        }
        Ok(())
    }

    fn get_array_of_dates(
        &mut self,
        arg: &Node,
        cell: CellReferenceIndex,
    ) -> Result<Vec<i64>, CalcResult> {
        let mut values = Vec::new();
        self.collect_serial_numbers(arg, cell, |serial| {
            values.push(serial);
            Ok(())
        })?;
        Ok(values)
    }

    pub(crate) fn fn_networkdays(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.networkdays_arguments(args, cell, false)
    }

    fn networkdays_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        international: bool,
    ) -> CalcResult {
        let scalar_count = if international { 3 } else { 2 };
        if args.len() < 2 || args.len() > scalar_count + 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let holidays: Result<std::collections::HashSet<i64>, CalcResult> =
            match args.get(scalar_count) {
                Some(arg) => self
                    .get_array_of_dates(arg, cell)
                    .map(|dates| dates.into_iter().collect()),
                None => Ok(std::collections::HashSet::new()),
            };
        if let Err(
            error @ CalcResult::Error {
                error: Error::CIRC, ..
            },
        ) = &holidays
        {
            return error.clone();
        }
        self.map_scalar_arguments(
            &args[..args.len().min(scalar_count)],
            cell,
            |model, values| {
                let numeric = |model: &mut Self, value: &CalcResult| match model
                    .cast_to_number(value.clone(), cell)
                {
                    Ok(number) if number.is_finite() => Ok(number.floor() as i64),
                    Ok(_) => Err(CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )),
                    Err(error) => Err(error),
                };
                let start = match numeric(model, &values[0]) {
                    Ok(value) => value,
                    Err(error) => return error,
                };
                let end = match numeric(model, &values[1]) {
                    Ok(value) => value,
                    Err(error) => return error,
                };
                let weekend = match Self::weekend_mask_value(
                    values.get(2).cloned().unwrap_or(CalcResult::EmptyArg),
                    cell,
                ) {
                    Ok(mask) => mask,
                    Err(error) => return error,
                };
                let holidays = match &holidays {
                    Ok(holidays) => holidays,
                    Err(error) => return error.clone(),
                };
                model.networkdays_result(start, end, &weekend, holidays, cell)
            },
        )
    }

    fn networkdays_result(
        &self,
        start: i64,
        end: i64,
        weekend: &[bool; 7],
        holidays: &std::collections::HashSet<i64>,
        cell: CellReferenceIndex,
    ) -> CalcResult {
        for serial in [start, end] {
            if let Err(error) = self.excel_date(serial, cell) {
                return error;
            }
        }
        let (mut from, to, sign) = if start <= end {
            (start, end, 1.0)
        } else {
            (end, start, -1.0)
        };
        let original_from = from;
        let mut count = 0_i64;
        // Keep the existing early-1900 serial interpretation, including its
        // duplicated calendar mapping at serial 60. Later dates are regular.
        if !crate::date_system::is_1904() {
            while from <= to.min(60) {
                let date = match self.excel_date(from, cell) {
                    Ok(date) => date,
                    Err(error) => return error,
                };
                if !Self::is_weekend(date.weekday(), weekend) {
                    count += 1;
                }
                from += 1;
            }
        }
        if from <= to {
            let date = match self.excel_date(from, cell) {
                Ok(date) => date,
                Err(error) => return error,
            };
            let weekday = date.weekday().num_days_from_monday() as usize;
            let length = to - from + 1;
            let per_week = weekend.iter().filter(|day| !**day).count() as i64;
            count += (length / 7) * per_week;
            for offset in 0..length % 7 {
                if !weekend[(weekday + offset as usize) % 7] {
                    count += 1;
                }
            }
        }
        for holiday in holidays
            .iter()
            .filter(|serial| **serial >= original_from && **serial <= to)
        {
            let date = match self.excel_date(*holiday, cell) {
                Ok(date) => date,
                Err(error) => return error,
            };
            if !Self::is_weekend(date.weekday(), weekend) {
                count -= 1;
            }
        }
        CalcResult::Number(count as f64 * sign)
    }

    fn excel_date(
        &self,
        serial: i64,
        cell: CellReferenceIndex,
    ) -> Result<chrono::NaiveDate, CalcResult> {
        match from_excel_date(serial) {
            Ok(date) => Ok(date),
            Err(_) => Err(CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            }),
        }
    }

    fn weekend_mask_value(
        value: CalcResult,
        cell: CellReferenceIndex,
    ) -> Result<[bool; 7], CalcResult> {
        let mut weekend = [false, false, false, false, false, true, true];
        match value {
            CalcResult::Number(n) => {
                let code = n.trunc() as i32;
                if (n - n.trunc()).abs() > f64::EPSILON {
                    return Err(CalcResult::new_error(
                        Error::NUM,
                        cell,
                        "Invalid weekend".to_string(),
                    ));
                }
                weekend = match WeekendPattern::try_from(code) {
                    Ok(pattern) => pattern.to_mask(),
                    Err(_) => {
                        return Err(CalcResult::new_error(
                            Error::NUM,
                            cell,
                            "Invalid weekend".to_string(),
                        ))
                    }
                };
                Ok(weekend)
            }
            CalcResult::String(s) => {
                if s.len() != 7 {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "Invalid weekend".to_string(),
                    ));
                }
                if !s.chars().all(|c| c == '0' || c == '1') {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "Invalid weekend".to_string(),
                    ));
                }
                weekend = [false; 7];
                for (i, ch) in s.chars().enumerate() {
                    weekend[i] = ch == '1';
                }
                Ok(weekend)
            }
            CalcResult::Boolean(_) => Err(CalcResult::new_error(
                Error::VALUE,
                cell,
                "Invalid weekend".to_string(),
            )),
            e @ CalcResult::Error { .. } => Err(e),
            CalcResult::Range { .. } => Err(CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "Invalid weekend".to_string(),
            }),
            CalcResult::EmptyCell | CalcResult::EmptyArg => Ok(weekend),
            CalcResult::Array(_) | CalcResult::Lambda(_) => Err(CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "Invalid weekend".to_string(),
            }),
        }
    }

    pub(crate) fn fn_networkdays_intl(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.networkdays_arguments(args, cell, true)
    }

    // Extends the standard by adding an optional timezone parameter
    pub(crate) fn fn_today(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() > 1 {
            return CalcResult::Error {
                error: Error::ERROR,
                origin: cell,
                message: "Wrong number of arguments".to_string(),
            };
        }
        let tz_owned;
        let tz: &Tz = match args.first() {
            Some(arg0) => {
                let tz_str = match self.get_string(arg0, cell) {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                tz_owned = match Tz::parse(&tz_str) {
                    Ok(tz) => tz,
                    Err(_) => {
                        return CalcResult::Error {
                            error: Error::VALUE,
                            origin: cell,
                            message: format!("Invalid timezone: {}", tz_str),
                        }
                    }
                };
                &tz_owned
            }
            None => &self.tz,
        };
        match crate::tz::excel_serial_for_now(tz) {
            Some(serial) => CalcResult::Number(serial.floor()),
            None => CalcResult::Error {
                error: Error::ERROR,
                origin: cell,
                message: "Invalid date".to_string(),
            },
        }
    }

    // Extends the standard by adding an optional timezone parameter
    pub(crate) fn fn_now(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() > 1 {
            return CalcResult::Error {
                error: Error::ERROR,
                origin: cell,
                message: "Wrong number of arguments".to_string(),
            };
        }
        let tz_owned;
        let tz: &Tz = match args.first() {
            Some(arg0) => {
                let tz_str = match self.get_string(arg0, cell) {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                tz_owned = match Tz::parse(&tz_str) {
                    Ok(tz) => tz,
                    Err(_) => {
                        return CalcResult::Error {
                            error: Error::VALUE,
                            origin: cell,
                            message: format!("Invalid timezone: {}", tz_str),
                        }
                    }
                };
                &tz_owned
            }
            None => &self.tz,
        };
        match crate::tz::excel_serial_for_now(tz) {
            Some(serial) => CalcResult::Number(serial),
            None => CalcResult::Error {
                error: Error::ERROR,
                origin: cell,
                message: "Invalid date".to_string(),
            },
        }
    }

    pub(crate) fn fn_time(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 3 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let hour = match model.cast_to_number(values[0].clone(), cell) {
                Ok(f) => f,
                Err(e) => return e,
            };
            let minute = match model.cast_to_number(values[1].clone(), cell) {
                Ok(f) => f,
                Err(e) => return e,
            };
            let second = match model.cast_to_number(values[2].clone(), cell) {
                Ok(f) => f,
                Err(e) => return e,
            };
            if [hour, minute, second]
                .iter()
                .any(|value| !value.is_finite() || value.floor() > 32767.0)
            {
                return CalcResult::new_error(Error::NUM, cell, "Invalid time component".into());
            }
            let total_seconds = hour.floor() * 3600.0 + minute.floor() * 60.0 + second.floor();
            if total_seconds < 0.0 {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Invalid time".to_string(),
                };
            }
            let secs = total_seconds.rem_euclid(SECONDS_PER_DAY_F64);
            CalcResult::Number(secs / SECONDS_PER_DAY_F64)
        })
    }

    // -----------------------------------------------------------------------
    // Auto-generated TIME part helpers (HOUR / MINUTE / SECOND)
    // -----------------------------------------------------------------------

    time_part_fn!(fn_hour, |v: f64| (v.rem_euclid(1.0) * 24.0).floor());
    time_part_fn!(fn_minute, |v: f64| {
        let total_seconds = (v.rem_euclid(1.0) * SECONDS_PER_DAY_F64).floor();
        ((total_seconds / 60.0) as i64 % 60) as f64
    });
    time_part_fn!(fn_second, |v: f64| {
        let total_seconds = to_precision(v.rem_euclid(1.0) * SECONDS_PER_DAY_F64, 15).floor();
        (total_seconds as i64 % 60) as f64
    });

    pub(crate) fn fn_timevalue(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |model, value| {
            let text = match model.cast_to_string(value, cell) {
                Ok(text) => text,
                Err(error) => return error,
            };
            match parse_time_string(&text)
                .or_else(|| parse_localized_date_time_string(&text, model.locale))
            {
                Some(value) => CalcResult::Number(value),
                None => CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid time".to_string(),
                },
            }
        })
    }

    pub(crate) fn fn_datevalue(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |model, value| {
            let text = match model.cast_to_string(value, cell) {
                Ok(text) => text,
                Err(error) => return error,
            };
            match parse_datevalue_text(&text, model.locale) {
                Ok(n) => CalcResult::Number(n as f64),
                Err(_) => CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid date".to_string(),
                },
            }
        })
    }

    pub(crate) fn fn_datedif(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 3 {
            return CalcResult::new_args_number_error(cell);
        }

        self.map_scalar_arguments(args, cell, |model, values| {
            let start_serial = match model.cast_date_serial(values[0].clone(), cell) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let end_serial = match model.cast_date_serial(values[1].clone(), cell) {
                Ok(v) => v,
                Err(e) => return e,
            };
            if end_serial < start_serial {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Start date greater than end date".to_string(),
                };
            }
            let start = match model.excel_date(start_serial, cell) {
                Ok(d) => d,
                Err(e) => return e,
            };
            let end = match model.excel_date(end_serial, cell) {
                Ok(d) => d,
                Err(e) => return e,
            };
            let unit = match model.cast_to_string(values[2].clone(), cell) {
                Ok(s) => s.to_uppercase(),
                Err(e) => return e,
            };

            let result = match unit.as_str() {
                "Y" => {
                    let mut years = end.year() - start.year();
                    if (end.month(), end.day()) < (start.month(), start.day()) {
                        years -= 1;
                    }
                    years as f64
                }
                "M" => {
                    let mut months = (end.year() - start.year()) * 12
                        + (end.month() as i32 - start.month() as i32);
                    if end.day() < start.day() {
                        months -= 1;
                    }
                    months as f64
                }
                "D" => (end_serial - start_serial) as f64,
                "YM" => {
                    let mut months = (end.year() - start.year()) * 12
                        + (end.month() as i32 - start.month() as i32);
                    if end.day() < start.day() {
                        months -= 1;
                    }
                    (months % 12).abs() as f64
                }
                "YD" => {
                    // Helper to create a date or early-return with #NUM! if impossible
                    let make_date =
                        |y: i32, m: u32, d: u32| -> Result<chrono::NaiveDate, CalcResult> {
                            match chrono::NaiveDate::from_ymd_opt(y, m, d) {
                                Some(dt) => Ok(dt),
                                None => Err(CalcResult::Error {
                                    error: Error::NUM,
                                    origin: cell,
                                    message: "Invalid date".to_string(),
                                }),
                            }
                        };

                    // Compute the last valid day of a given month/year.
                    let make_last_day_of_month =
                        |y: i32, m: u32| -> Result<chrono::NaiveDate, CalcResult> {
                            let (next_y, next_m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
                            let first_next = make_date(next_y, next_m, 1)?;
                            let last_day = first_next - chrono::Duration::days(1);
                            make_date(y, m, last_day.day())
                        };

                    // Attempt to build the anniversary date in the end year.
                    let mut start_adj = match chrono::NaiveDate::from_ymd_opt(
                        end.year(),
                        start.month(),
                        start.day(),
                    ) {
                        Some(d) => d,
                        None => match make_last_day_of_month(end.year(), start.month()) {
                            Ok(d) => d,
                            Err(e) => return e,
                        },
                    };

                    // If the adjusted date is after the end date, shift one year back.
                    if start_adj > end {
                        let shift_year = end.year() - 1;
                        start_adj = match chrono::NaiveDate::from_ymd_opt(
                            shift_year,
                            start.month(),
                            start.day(),
                        ) {
                            Some(d) => d,
                            None => match make_last_day_of_month(shift_year, start.month()) {
                                Ok(d) => d,
                                Err(e) => return e,
                            },
                        };
                    }

                    (end - start_adj).num_days() as f64
                }
                "MD" => {
                    let mut months = (end.year() - start.year()) * 12
                        + (end.month() as i32 - start.month() as i32);
                    if end.day() < start.day() {
                        months -= 1;
                    }
                    let start_shifted = if months >= 0 {
                        start + Months::new(months as u32)
                    } else {
                        start - Months::new((-months) as u32)
                    };
                    (end - start_shifted).num_days() as f64
                }
                _ => {
                    return CalcResult::Error {
                        error: Error::VALUE,
                        origin: cell,
                        message: "Invalid unit".to_string(),
                    };
                }
            };
            CalcResult::Number(result)
        })
    }

    pub(crate) fn fn_days(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let date_arg = |model: &mut Self, value: &CalcResult| -> Result<f64, CalcResult> {
                // Text follows DATEVALUE and discards its time component;
                // numeric serials retain their fractional time for subtraction.
                let number = match value {
                    CalcResult::String(text) => parse_datevalue_text(text, model.locale)
                        .map(f64::from)
                        .map_err(|message| CalcResult::new_error(Error::VALUE, cell, message))?,
                    value => model.cast_to_number(value.clone(), cell)?,
                };
                if !number.is_finite()
                    || !(0.0..=crate::date_system::maximum_serial() as f64)
                        .contains(&number.floor())
                {
                    return Err(CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    ));
                }
                Ok(number)
            };
            let end = match date_arg(model, &values[0]) {
                Ok(number) => number,
                Err(error) => return error,
            };
            let start = match date_arg(model, &values[1]) {
                Ok(number) => number,
                Err(error) => return error,
            };
            CalcResult::Number(end - start)
        })
    }

    pub(crate) fn fn_days360(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let start_serial = match model.cast_to_number(values[0].clone(), cell) {
                Ok(c) if c.is_finite() => days360_serial(c),
                Ok(_) => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )
                }
                Err(s) => return s,
            };
            let end_serial = match model.cast_to_number(values[1].clone(), cell) {
                Ok(c) if c.is_finite() => days360_serial(c),
                Ok(_) => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )
                }
                Err(s) => return s,
            };
            // The European-method flag follows Excel's logical coercion: numbers are
            // truthy when non-zero, and only the text "TRUE"/"FALSE" is accepted.
            let method = if args.len() == 3 {
                match model.cast_to_bool(values[2].clone(), cell) {
                    Ok(b) => b,
                    Err(s) => return s,
                }
            } else {
                false
            };
            let out_of_range = || {
                CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Out of range parameters for date".to_string(),
                )
            };
            let start = match excel_serial_to_ymd(start_serial) {
                Some(ymd) => ymd,
                None => return out_of_range(),
            };
            let end = match excel_serial_to_ymd(end_serial) {
                Some(ymd) => ymd,
                None => return out_of_range(),
            };
            let result = if !method {
                days360_us(start, end)
            } else {
                days360_eu(start, end)
            };
            CalcResult::Number(result)
        })
    }

    pub(crate) fn fn_weekday(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(1..=2).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let serial = match model.cast_to_number(values[0].clone(), cell) {
                Ok(number) if number.is_finite() => number,
                Ok(_) => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )
                }
                Err(error) => return error,
            };
            let date = match from_excel_date(serial.floor() as i64) {
                Ok(date) => date,
                Err(_) => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )
                }
            };
            let return_type = if values.len() == 2 {
                match model.cast_to_number(values[1].clone(), cell) {
                    Ok(number) if number.is_finite() => number as i32,
                    Ok(_) => {
                        return CalcResult::new_error(
                            Error::NUM,
                            cell,
                            "Invalid return_type".into(),
                        )
                    }
                    Err(error) => return error,
                }
            } else {
                1
            };
            match weekday_number(date, return_type) {
                Ok(number) => CalcResult::Number(number),
                Err(error) => CalcResult::new_error(error, cell, "Invalid return_type".into()),
            }
        })
    }

    pub(crate) fn fn_weeknum(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(1..=2).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let serial = match model.cast_to_number(values[0].clone(), cell) {
                Ok(c) if c.is_finite() => c.floor() as i64,
                Ok(_) => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )
                }
                Err(s) => return s,
            };
            let date = match model.excel_date(serial, cell) {
                Ok(d) => d,
                Err(e) => return e,
            };
            let return_type = if args.len() == 2 {
                match model.cast_to_number(values[1].clone(), cell) {
                    Ok(f) if f.is_finite() => f as i32,
                    Ok(_) => {
                        return CalcResult::new_error(
                            Error::NUM,
                            cell,
                            "Invalid return_type".into(),
                        )
                    }
                    Err(s) => return s,
                }
            } else {
                1
            };
            if return_type == 21 {
                let w = date.iso_week().week();
                return CalcResult::Number(w as f64);
            }
            let start_offset = match return_type {
                1 => chrono::Weekday::Sun,
                2 | 11 => chrono::Weekday::Mon,
                12 => chrono::Weekday::Tue,
                13 => chrono::Weekday::Wed,
                14 => chrono::Weekday::Thu,
                15 => chrono::Weekday::Fri,
                16 => chrono::Weekday::Sat,
                17 => chrono::Weekday::Sun,
                _ => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        "Invalid return_type".to_string(),
                    )
                }
            };
            let mut first = match chrono::NaiveDate::from_ymd_opt(date.year(), 1, 1) {
                Some(d) => d,
                None => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.to_string(),
                    );
                }
            };
            while first.weekday() != start_offset {
                first -= chrono::Duration::days(1);
            }
            let week = (date - first).num_days() / 7 + 1;
            CalcResult::Number(week as f64)
        })
    }

    pub(crate) fn fn_isoweeknum(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        // Broadcasts element-wise over a range/array date argument.
        self.apply_number_unary(&args[0], cell, |serial| {
            let date = from_excel_date(serial.floor() as i64).map_err(|_| Error::NUM)?;
            Ok(date.iso_week().week() as f64)
        })
    }

    fn is_weekend(day: chrono::Weekday, weekend_mask: &[bool; 7]) -> bool {
        match day {
            chrono::Weekday::Mon => weekend_mask[0],
            chrono::Weekday::Tue => weekend_mask[1],
            chrono::Weekday::Wed => weekend_mask[2],
            chrono::Weekday::Thu => weekend_mask[3],
            chrono::Weekday::Fri => weekend_mask[4],
            chrono::Weekday::Sat => weekend_mask[5],
            chrono::Weekday::Sun => weekend_mask[6],
        }
    }

    pub(crate) fn fn_workday(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.workday_arguments(args, cell, false)
    }

    fn workday_arguments(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        international: bool,
    ) -> CalcResult {
        let scalar_count = if international { 3 } else { 2 };
        if args.len() < 2 || args.len() > scalar_count + 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let holidays = self.get_holiday_set(args.get(scalar_count), cell);
        if let Err(
            error @ CalcResult::Error {
                error: Error::CIRC, ..
            },
        ) = &holidays
        {
            return error.clone();
        }
        let operands = &args[..args.len().min(scalar_count)];
        self.map_scalar_arguments(operands, cell, |model, values| {
            let start_serial = match model.cast_to_number(values[0].clone(), cell) {
                Ok(number) if number.is_finite() => number.floor() as i64,
                Ok(_) => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )
                }
                Err(error) => return error,
            };
            let date = match model.excel_date(start_serial, cell) {
                Ok(date) => date,
                Err(error) => return error,
            };
            let days = match model.cast_to_number(values[1].clone(), cell) {
                Ok(number) => number.trunc(),
                Err(error) => return error,
            };
            let weekend = match Self::weekend_mask_value(
                values.get(2).cloned().unwrap_or(CalcResult::EmptyArg),
                cell,
            ) {
                Ok(mask) => mask,
                Err(error) => return error,
            };
            let holidays = match &holidays {
                Ok(holidays) => holidays,
                Err(error) => return error.clone(),
            };
            if weekend == [true; 7] {
                return CalcResult::new_error(Error::VALUE, cell, "Invalid weekend mask".into());
            }
            model.workday_result(date, days, &weekend, holidays, cell)
        })
    }

    fn workday_result(
        &self,
        start: chrono::NaiveDate,
        days: f64,
        weekend: &[bool; 7],
        holidays: &std::collections::HashSet<chrono::NaiveDate>,
        cell: CellReferenceIndex,
    ) -> CalcResult {
        let invalid = || CalcResult::new_error(Error::NUM, cell, DATE_OUT_OF_RANGE_MESSAGE.into());
        if !days.is_finite() {
            return invalid();
        }
        let direction = if days < 0.0 { -1 } else { 1 };
        let boundary_serial = if direction < 0 {
            crate::date_system::minimum_serial()
        } else {
            crate::date_system::maximum_serial()
        };
        let boundary = match from_excel_date(boundary_serial as i64) {
            Ok(date) => date,
            Err(_) => return invalid(),
        };
        let available = (boundary - start).num_days().abs();
        if days.abs() > available as f64 {
            return invalid();
        }
        let wanted = days.abs() as i64;
        let workdays_per_week = weekend.iter().filter(|day| !**day).count() as i64;
        let start_weekday = start.weekday().num_days_from_monday() as i64;
        let holiday_offsets: Vec<i64> = holidays
            .iter()
            .filter(|date| !Self::is_weekend(date.weekday(), weekend))
            .map(|date| (*date - start).num_days() * direction)
            .filter(|offset| *offset > 0 && *offset <= available)
            .collect();
        // Count complete weeks, then at most six residual weekdays. The start
        // date is excluded in either direction; duplicate holidays are already
        // removed by the shared set and weekend holidays do not subtract twice.
        let count = |distance: i64| {
            let mut total = (distance / 7) * workdays_per_week;
            for offset in 1..=distance % 7 {
                let weekday = (start_weekday + direction * offset).rem_euclid(7) as usize;
                if !weekend[weekday] {
                    total += 1;
                }
            }
            total
                - holiday_offsets
                    .iter()
                    .filter(|offset| **offset <= distance)
                    .count() as i64
        };
        if count(available) < wanted {
            return invalid();
        }
        let (mut low, mut high) = (0, available);
        while low < high {
            let middle = low + (high - low) / 2;
            if count(middle) >= wanted {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        let result = start + chrono::Duration::days(direction * low);
        CalcResult::Number(crate::formatter::dates::convert_to_serial_number(result) as f64)
    }

    fn get_holiday_set(
        &mut self,
        arg_option: Option<&Node>,
        cell: CellReferenceIndex,
    ) -> Result<std::collections::HashSet<chrono::NaiveDate>, CalcResult> {
        let mut holiday_set = std::collections::HashSet::new();

        if let Some(arg) = arg_option {
            self.collect_serial_numbers(arg, cell, |serial| match from_excel_date(serial) {
                Ok(date) => {
                    holiday_set.insert(date);
                    Ok(())
                }
                Err(_) => Err(CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Invalid holiday date".to_string(),
                }),
            })?;
        }

        Ok(holiday_set)
    }

    pub(crate) fn fn_workday_intl(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.workday_arguments(args, cell, true)
    }

    pub(crate) fn fn_yearfrac(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_arguments(args, cell, |model, values| {
            let start_serial = match model.cast_to_number(values[0].clone(), cell) {
                Ok(c) if c.is_finite() => c.floor() as i64,
                Ok(_) => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )
                }
                Err(s) => return s,
            };
            let end_serial = match model.cast_to_number(values[1].clone(), cell) {
                Ok(c) if c.is_finite() => c.floor() as i64,
                Ok(_) => {
                    return CalcResult::new_error(
                        Error::NUM,
                        cell,
                        DATE_OUT_OF_RANGE_MESSAGE.into(),
                    )
                }
                Err(s) => return s,
            };
            let basis = if args.len() == 3 {
                match model.cast_to_number(values[2].clone(), cell) {
                    Ok(f) if f.is_finite() => f as i32,
                    Ok(_) => {
                        return CalcResult::new_error(Error::NUM, cell, "Invalid basis".into())
                    }
                    Err(s) => return s,
                }
            } else {
                0
            };
            // Day-count adjustments and Actual/Actual year spans operate on
            // chronological endpoints, even when callers provide reversed dates.
            let (start_serial, end_serial) = if start_serial > end_serial {
                (end_serial, start_serial)
            } else {
                (start_serial, end_serial)
            };
            let start_date = match model.excel_date(start_serial, cell) {
                Ok(d) => d,
                Err(e) => return e,
            };
            let end_date = match model.excel_date(end_serial, cell) {
                Ok(d) => d,
                Err(e) => return e,
            };
            let days = (end_date - start_date).num_days() as f64;
            let result = match basis {
                0 => {
                    // YEARFRAC basis 0 uses the financial US 30/360 convention
                    // (`dateDiff360Us`, ModifyStartDate), which differs from the
                    // DAYS360 worksheet function for February month-ends: a
                    // February-end *end* date only collapses to 30 when the *start*
                    // is also a February month-end, and the `end == 31` rule tests
                    // the original (un-normalised) start day.
                    let last_day_feb = |year: i32| if is_leap_year(year) { 29 } else { 28 };
                    let sd_is_feb_last = start_date.month() == 2
                        && start_date.day() == last_day_feb(start_date.year());
                    let ed_is_feb_last =
                        end_date.month() == 2 && end_date.day() == last_day_feb(end_date.year());
                    let mut sd_day = start_date.day() as i32;
                    let mut ed_day = end_date.day() as i32;

                    if ed_is_feb_last && sd_is_feb_last {
                        ed_day = 30;
                    }
                    if ed_day == 31 && sd_day >= 30 {
                        ed_day = 30;
                    }
                    if sd_day == 31 {
                        sd_day = 30;
                    }
                    if sd_is_feb_last {
                        sd_day = 30;
                    }
                    let d360 = (end_date.year() - start_date.year()) * 360
                        + (end_date.month() as i32 - start_date.month() as i32) * 30
                        + (ed_day - sd_day);
                    d360 as f64 / 360.0
                }
                1 => {
                    // Actual/Actual. Port of ExcelFinancialFunctions
                    // `ActualActual.DaysInYear`: yearfrac = actual_days / DaysInYear.
                    let (y1, m1, d1) = (start_date.year(), start_date.month(), start_date.day());
                    let (y2, m2, d2) = (end_date.year(), end_date.month(), end_date.day());

                    // lessOrEqualToAYearApart
                    let within_a_year =
                        y1 == y2 || (y2 == y1 + 1 && (m1 > m2 || (m1 == m2 && d1 >= d2)));

                    let days_in_year = if !within_a_year {
                        // Dates more than a year apart: average the calendar-year
                        // lengths spanned, inclusive of both end years.
                        let tot_days: i64 = (y1..=y2)
                            .map(|y| if is_leap_year(y) { 366 } else { 365 })
                            .sum();
                        tot_days as f64 / (y2 - y1 + 1) as f64
                    } else {
                        // considerAsBisestile
                        let consider = (y1 == y2 && is_leap_year(y1))
                            || (m2 == 2 && d2 == 29)
                            || is_feb29_between_consecutive_years(start_date, end_date);
                        if consider {
                            366.0
                        } else {
                            365.0
                        }
                    };
                    days / days_in_year
                }
                2 => days / 360.0,
                3 => days / 365.0,
                4 => {
                    let d360 = model.fn_days360(
                        &[
                            Node::NumberKind(start_serial as f64),
                            Node::NumberKind(end_serial as f64),
                            Node::NumberKind(1.0),
                        ],
                        cell,
                    );
                    if let CalcResult::Number(n) = d360 {
                        n / 360.0
                    } else {
                        return d360;
                    }
                }
                _ => return CalcResult::new_error(Error::NUM, cell, "Invalid basis".to_string()),
            };
            CalcResult::Number(result.abs())
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn holiday_numbers_reject_nonfinite_values_in_both_date_systems() {
        let mut model = crate::test::util::new_empty_model();
        let cell = CellReferenceIndex {
            sheet: 0,
            row: 1,
            column: 1,
        };
        for system in [
            crate::date_system::DateSystem::Excel1900,
            crate::date_system::DateSystem::Excel1904,
        ] {
            crate::date_system::with_date_system(system, || {
                for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                    assert!(matches!(
                        model.holiday_serial(number, cell),
                        Err(CalcResult::Error {
                            error: Error::NUM,
                            ..
                        })
                    ));
                }
                let mut collected = Vec::new();
                assert!(model
                    .collect_serial_numbers(
                        &Node::ArrayKind(vec![vec![ArrayNode::Empty, ArrayNode::Number(45000.0)]]),
                        cell,
                        |serial| {
                            collected.push(serial);
                            Ok(())
                        },
                    )
                    .is_ok());
                assert_eq!(collected, vec![45000]);
            });
        }
    }

    #[test]
    fn networkdays_week_counts_match_daily_serial_iteration() {
        let model = crate::test::util::new_empty_model();
        let cell = CellReferenceIndex {
            sheet: 0,
            row: 1,
            column: 1,
        };
        for system in [
            crate::date_system::DateSystem::Excel1900,
            crate::date_system::DateSystem::Excel1904,
        ] {
            crate::date_system::with_date_system(system, || {
                let holidays: std::collections::HashSet<i64> =
                    [58, 59, 60, 61, 45001, 45004, 45005, 45006, 45010]
                        .into_iter()
                        .collect();
                for bits in 0..128 {
                    let weekend = std::array::from_fn(|index| bits & (1 << index) != 0);
                    for start in [58, 59, 60, 45000, 45001, 45002, 45003, 45004, 45005, 45006] {
                        for delta in -30..=30 {
                            let end = start + delta;
                            let mut expected = 0;
                            for serial in start.min(end)..=start.max(end) {
                                let date = from_excel_date(serial).unwrap();
                                if !Model::is_weekend(date.weekday(), &weekend)
                                    && !holidays.contains(&serial)
                                {
                                    expected += 1;
                                }
                            }
                            if start > end {
                                expected = -expected;
                            }
                            let actual =
                                model.networkdays_result(start, end, &weekend, &holidays, cell);
                            assert!(
                                matches!(actual, CalcResult::Number(value) if value == expected as f64),
                                "mask={bits}, start={start}, end={end}"
                            );
                        }
                    }
                }
            });
        }
    }

    #[test]
    fn bounded_workday_search_matches_daily_count_for_all_weekend_patterns() {
        let model = crate::test::util::new_empty_model();
        let cell = CellReferenceIndex {
            sheet: 0,
            row: 1,
            column: 1,
        };
        let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let holidays: std::collections::HashSet<_> = [1, 4, 5, 6, 10, 28]
            .into_iter()
            .map(|offset| base + chrono::Duration::days(offset))
            .collect();
        for bits in 0..127 {
            let weekend = std::array::from_fn(|index| bits & (1 << index) != 0);
            for start_offset in 0..7 {
                let start = base + chrono::Duration::days(start_offset);
                for days in -30_i64..=30 {
                    let mut expected = start;
                    let mut remaining = days.abs();
                    while remaining > 0 {
                        expected += chrono::Duration::days(days.signum());
                        if !Model::is_weekend(expected.weekday(), &weekend)
                            && !holidays.contains(&expected)
                        {
                            remaining -= 1;
                        }
                    }
                    let actual =
                        model.workday_result(start, days as f64, &weekend, &holidays, cell);
                    assert!(
                        matches!(actual, CalcResult::Number(number) if number == crate::formatter::dates::convert_to_serial_number(expected) as f64),
                        "mask={bits}, start={start}, days={days}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_is_leap_year() {
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2004));
        assert!(!is_leap_year(2001));
    }

    #[test]
    fn test_is_feb29_between_consecutive_years() {
        // Same leap year, spanning Feb 29.
        let d1 = chrono::NaiveDate::from_ymd_opt(2020, 2, 28).unwrap();
        let d2 = chrono::NaiveDate::from_ymd_opt(2020, 3, 1).unwrap();
        assert!(is_feb29_between_consecutive_years(d1, d2));
    }

    #[test]
    fn test_is_feb29_between_consecutive_years_false() {
        // Non-leap year, no Feb 29 in range.
        let d1 = chrono::NaiveDate::from_ymd_opt(2021, 2, 28).unwrap();
        let d2 = chrono::NaiveDate::from_ymd_opt(2021, 3, 1).unwrap();
        assert!(!is_feb29_between_consecutive_years(d1, d2));

        // Leap year, but the period starts after Feb 29 (March).
        let d3 = chrono::NaiveDate::from_ymd_opt(1992, 3, 4).unwrap();
        let d4 = chrono::NaiveDate::from_ymd_opt(1993, 3, 1).unwrap();
        assert!(!is_feb29_between_consecutive_years(d3, d4));
    }
}
