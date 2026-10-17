/// Date/time value used by Word field picture formatting. `weekday` is
/// 0 = Sunday .. 6 = Saturday.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldDateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub weekday: u32,
}

const MONTH_NAMES: [&str; 12] = [
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
const WEEKDAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Parses an OPC core-property W3CDTF timestamp and normalizes an explicit
/// timezone offset to UTC. Fractional seconds are accepted but Word field
/// pictures do not expose them.
pub fn parse_core_property_datetime(value: &str) -> Option<FieldDateTime> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() < 19
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't'))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let year = value.get(0..4)?.parse::<i32>().ok()?;
    let month = value.get(5..7)?.parse::<u32>().ok()?;
    let day = value.get(8..10)?.parse::<u32>().ok()?;
    let hour = value.get(11..13)?.parse::<u32>().ok()?;
    let minute = value.get(14..16)?.parse::<u32>().ok()?;
    let second = value.get(17..19)?.parse::<u32>().ok()?;
    if !valid_date(year, month, day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let mut zone_index = 19;
    if bytes.get(zone_index) == Some(&b'.') {
        zone_index += 1;
        let fraction_start = zone_index;
        while matches!(bytes.get(zone_index), Some(b'0'..=b'9')) {
            zone_index += 1;
        }
        if zone_index == fraction_start {
            return None;
        }
    }
    let offset_seconds = match bytes.get(zone_index) {
        Some(b'Z' | b'z') if zone_index + 1 == bytes.len() => 0,
        Some(sign @ (b'+' | b'-')) if zone_index + 6 == bytes.len() => {
            if bytes.get(zone_index + 3) != Some(&b':') {
                return None;
            }
            let zone_hour = value
                .get(zone_index + 1..zone_index + 3)?
                .parse::<i64>()
                .ok()?;
            let zone_minute = value
                .get(zone_index + 4..zone_index + 6)?
                .parse::<i64>()
                .ok()?;
            // XML Schema/W3CDTF timezone offsets are bounded by ±14:00.
            if zone_hour > 14 || zone_minute > 59 || (zone_hour == 14 && zone_minute != 0) {
                return None;
            }
            let seconds = zone_hour * 3600 + zone_minute * 60;
            if *sign == b'-' {
                -seconds
            } else {
                seconds
            }
        }
        _ => return None,
    };

    let local_seconds = days_from_civil(year, month, day) * 86_400
        + i64::from(hour) * 3600
        + i64::from(minute) * 60
        + i64::from(second);
    let utc_seconds = local_seconds - offset_seconds;
    let utc_days = utc_seconds.div_euclid(86_400);
    let seconds_in_day = utc_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(utc_days);
    Some(FieldDateTime {
        year,
        month,
        day,
        hour: (seconds_in_day / 3600) as u32,
        minute: ((seconds_in_day % 3600) / 60) as u32,
        second: (seconds_in_day % 60) as u32,
        weekday: (utc_days + 4).rem_euclid(7) as u32,
    })
}

/// Formats a value using the common Word `\@` picture tokens. Longest tokens
/// win, single-quoted text is literal, and unknown characters pass through.
pub fn format_word_date(picture: &str, dt: &FieldDateTime) -> String {
    let chars: Vec<char> = picture.chars().collect();
    let mut out = String::with_capacity(picture.len());
    let mut i = 0;
    let month_idx = (dt.month.clamp(1, 12) - 1) as usize;
    let weekday_idx = (dt.weekday % 7) as usize;
    let hour12 = match dt.hour % 12 {
        0 => 12,
        hour => hour,
    };
    while i < chars.len() {
        if chars[i] == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        let rest: String = chars[i..].iter().collect();
        let matched: Option<(usize, String)> =
            if rest.starts_with("AM/PM") || rest.starts_with("am/pm") {
                Some((5, if dt.hour < 12 { "AM" } else { "PM" }.to_string()))
            } else if rest.starts_with("yyyy") || rest.starts_with("YYYY") {
                Some((4, format!("{:04}", dt.year)))
            } else if rest.starts_with("yy") || rest.starts_with("YY") {
                Some((2, format!("{:02}", dt.year.rem_euclid(100))))
            } else if rest.starts_with("MMMM") {
                Some((4, MONTH_NAMES[month_idx].to_string()))
            } else if rest.starts_with("MMM") {
                Some((3, MONTH_NAMES[month_idx][..3].to_string()))
            } else if rest.starts_with("MM") {
                Some((2, format!("{:02}", dt.month)))
            } else if rest.starts_with('M') {
                Some((1, dt.month.to_string()))
            } else if rest.starts_with("dddd") || rest.starts_with("DDDD") {
                Some((4, WEEKDAY_NAMES[weekday_idx].to_string()))
            } else if rest.starts_with("ddd") || rest.starts_with("DDD") {
                Some((3, WEEKDAY_NAMES[weekday_idx][..3].to_string()))
            } else if rest.starts_with("dd") || rest.starts_with("DD") {
                Some((2, format!("{:02}", dt.day)))
            } else if rest.starts_with('d') || rest.starts_with('D') {
                Some((1, dt.day.to_string()))
            } else if rest.starts_with("HH") {
                Some((2, format!("{:02}", dt.hour)))
            } else if rest.starts_with('H') {
                Some((1, dt.hour.to_string()))
            } else if rest.starts_with("hh") {
                Some((2, format!("{:02}", hour12)))
            } else if rest.starts_with('h') {
                Some((1, hour12.to_string()))
            } else if rest.starts_with("mm") {
                Some((2, format!("{:02}", dt.minute)))
            } else if rest.starts_with('m') {
                Some((1, dt.minute.to_string()))
            } else if rest.starts_with("ss") {
                Some((2, format!("{:02}", dt.second)))
            } else if rest.starts_with('s') {
                Some((1, dt.second.to_string()))
            } else {
                None
            };
        if let Some((len, text)) = matched {
            out.push_str(&text);
            i += len;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn valid_date(year: i32, month: u32, day: u32) -> bool {
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.rem_euclid(400) == 0
            || (year.rem_euclid(4) == 0 && year.rem_euclid(100) != 0) =>
        {
            29
        }
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day)
}

// Dependency-free proleptic-Gregorian conversions, offset so day zero is
// 1970-01-01. These are Howard Hinnant's civil date algorithms.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_word_picture_tokens_and_literals() {
        let datetime = FieldDateTime {
            year: 2026,
            month: 5,
            day: 30,
            hour: 15,
            minute: 7,
            second: 9,
            weekday: 6,
        };
        assert_eq!(
            format_word_date("dddd, MMMM d, yyyy HH:mm:ss", &datetime),
            "Saturday, May 30, 2026 15:07:09"
        );
        assert_eq!(
            format_word_date("yyyy'年'M'月'd'日' h:mm AM/PM", &datetime),
            "2026年5月30日 3:07 PM"
        );
        // Word treats day/year picture letters as case-insensitive and emits
        // uppercase meridiem text for both documented spellings.
        assert_eq!(
            format_word_date("DDDD, MMMM D, YYYY hh:mm am/pm", &datetime),
            "Saturday, May 30, 2026 03:07 PM"
        );
    }

    #[test]
    fn parses_w3cdtf_offsets_and_fractional_seconds_as_utc() {
        // Core properties normally use Z, but W3CDTF also permits explicit
        // offsets. Normalize the instant before applying a field picture so a
        // positive offset that crosses midnight does not display the wrong day.
        assert_eq!(
            parse_core_property_datetime("2024-01-02T03:04:05.987+09:00"),
            Some(FieldDateTime {
                year: 2024,
                month: 1,
                day: 1,
                hour: 18,
                minute: 4,
                second: 5,
                weekday: 1,
            })
        );
        assert_eq!(
            parse_core_property_datetime("2024-01-01T22:30:00-03:30"),
            Some(FieldDateTime {
                year: 2024,
                month: 1,
                day: 2,
                hour: 2,
                minute: 0,
                second: 0,
                weekday: 2,
            })
        );
        assert_eq!(
            parse_core_property_datetime("2024-01-01T00:00:00+14:01"),
            None
        );
        assert_eq!(
            parse_core_property_datetime("2024-01-01T00:00:00+15:00"),
            None
        );
    }
}
