use chrono::Datelike;
use chrono::Days;
use chrono::Duration;
use chrono::Months;
use chrono::NaiveDate;

#[cfg(test)]
use crate::constants::MAXIMUM_DATE_SERIAL_NUMBER;
use crate::constants::MINIMUM_DATE_SERIAL_NUMBER;

pub const DATE_OUT_OF_RANGE_MESSAGE: &str = "Out of range parameters for date";

/// Excel counts a 29 February 1900 that never was (the "1900 leap-year bug",
/// kept for Lotus 1-2-3), so in the 1900 system every date before 1 March 1900
/// is one serial lower than the calendar says: 1900-01-01 is 1, 1900-02-28 is
/// 59, 60 is the fictional leap day and 1900-03-01 is 61.
fn before_leap_day_1900(date: NaiveDate) -> bool {
    !crate::date_system::is_1904() && (date.year(), date.month()) < (1900, 3)
}

/// The date's serial number, as Excel counts it.
#[inline]
pub(crate) fn convert_to_serial_number(date: NaiveDate) -> i32 {
    date.num_days_from_ce() - crate::date_system::base() - i32::from(before_leap_day_1900(date))
}

fn is_date_within_range(date: NaiveDate) -> bool {
    convert_to_serial_number(date) >= crate::date_system::minimum_serial()
        && convert_to_serial_number(date) <= crate::date_system::maximum_serial()
}

pub fn from_excel_date(days: i64) -> Result<NaiveDate, String> {
    let minimum = crate::date_system::minimum_serial();
    let maximum = crate::date_system::maximum_serial();
    if days < minimum as i64 {
        return Err(format!("Excel date must be greater than {minimum}"));
    };
    if days > maximum as i64 {
        return Err(format!("Excel date must be less than {maximum}"));
    };
    #[allow(clippy::expect_used)]
    let dt = NaiveDate::from_ymd_opt(1900, 1, 1).expect("problem with chrono::NaiveDate");
    if !crate::date_system::is_1904() && days < 61 {
        // Before Excel's fictional leap day serial N is 1 January plus N - 1
        // days. Serial 60 itself has no calendar date; it reads as the 28th,
        // and DAY/MONTH/YEAR answer 29 February through `excel_serial_to_ymd`.
        return Ok(dt + Duration::days(days.min(59) - 1));
    }
    Ok(dt
        + Duration::days(
            days - 2
                + if crate::date_system::is_1904() {
                    1462
                } else {
                    0
                },
        ))
}

pub fn date_to_serial_number(day: u32, month: u32, year: i32) -> Result<i32, String> {
    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(native_date) => Ok(convert_to_serial_number(native_date)),
        None => Err(DATE_OUT_OF_RANGE_MESSAGE.to_string()),
    }
}

pub fn permissive_date_to_serial_number(day: i32, month: i32, year: i32) -> Result<i32, String> {
    // Excel parses `DATE` very permissively. It allows not just for valid date values, but it
    // allows for invalid dates as well. If you for example enter `DATE(1900, 1, 32)` it will
    // return the date `1900-02-01`. Despite giving a day that is out of range it will just
    // wrap the month and year around.
    //
    // This function applies that same logic to dates. And does it in the most compatible way as
    // possible.

    // Special case for the minimum date
    if !crate::date_system::is_1904() && year == 1899 && month == 12 && day == 31 {
        return Ok(MINIMUM_DATE_SERIAL_NUMBER);
    }
    // January and February 1900 are counted on Excel's calendar, which has a
    // 29 February: DATE(1900,2,29) is 60 and DATE(1900,1,61) is 1 March (61),
    // so a day past the month's end counts the fictional day too.
    let months = i64::from(year) * 12 + i64::from(month) - 1;
    if !crate::date_system::is_1904() && months.div_euclid(12) == 1900 && months.rem_euclid(12) < 2
    {
        let serial = if months.rem_euclid(12) == 0 { 0 } else { 31 } + i64::from(day);
        if serial < 0 || serial > i64::from(crate::date_system::maximum_serial()) {
            return Err(DATE_OUT_OF_RANGE_MESSAGE.to_string());
        }
        return Ok(serial as i32);
    }
    let Some(mut date) = NaiveDate::from_ymd_opt(year, 1, 1) else {
        return Err(DATE_OUT_OF_RANGE_MESSAGE.to_string());
    };

    // One thing to note for example is that even if you started with a year out of range
    // but tried to increment the months so that it wraps around into within range, excel
    // would still return an error.
    //
    // I.E. DATE(0,13,-1) will return an error, despite it being equivalent to DATE(1,1,0) which
    // is within range.
    //
    // As a result, we have to run range checks as we parse the date from the biggest unit to the
    // smallest unit.
    if !is_date_within_range(date) {
        return Err(DATE_OUT_OF_RANGE_MESSAGE.to_string());
    }

    date = {
        let month_diff = month - 1;
        let abs_month = month_diff.unsigned_abs();
        if month_diff <= 0 {
            date = date - Months::new(abs_month);
        } else {
            date = date + Months::new(abs_month);
        }
        if !is_date_within_range(date) {
            return Err(DATE_OUT_OF_RANGE_MESSAGE.to_string());
        }
        date
    };

    date = {
        let day_diff = day - 1;
        let abs_day = day_diff.unsigned_abs() as u64;
        if day_diff <= 0 {
            date = date - Days::new(abs_day);
        } else {
            date = date + Days::new(abs_day);
        }
        if !is_date_within_range(date) {
            return Err(DATE_OUT_OF_RANGE_MESSAGE.to_string());
        }
        date
    };

    Ok(convert_to_serial_number(date))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissive_date_to_serial_number() {
        assert_eq!(
            permissive_date_to_serial_number(42, 42, 2002),
            date_to_serial_number(12, 7, 2005)
        );
        assert_eq!(
            permissive_date_to_serial_number(1, 42, 2002),
            date_to_serial_number(1, 6, 2005)
        );
        assert_eq!(
            permissive_date_to_serial_number(1, 15, 2000),
            date_to_serial_number(1, 3, 2001)
        );
        assert_eq!(
            permissive_date_to_serial_number(1, 49, 2000),
            date_to_serial_number(1, 1, 2004)
        );
        assert_eq!(
            permissive_date_to_serial_number(1, 49, 2000),
            date_to_serial_number(1, 1, 2004)
        );
        assert_eq!(
            permissive_date_to_serial_number(31, 49, 2000),
            date_to_serial_number(31, 1, 2004)
        );
        assert_eq!(
            permissive_date_to_serial_number(256, 49, 2000),
            date_to_serial_number(12, 9, 2004)
        );
        assert_eq!(
            permissive_date_to_serial_number(256, 1, 2004),
            date_to_serial_number(12, 9, 2004)
        );
    }

    #[test]
    fn test_max_and_min_dates() {
        assert_eq!(
            permissive_date_to_serial_number(31, 12, 9999),
            Ok(MAXIMUM_DATE_SERIAL_NUMBER),
        );
        assert_eq!(
            permissive_date_to_serial_number(31, 12, 1899),
            Ok(MINIMUM_DATE_SERIAL_NUMBER),
        );
    }
}
