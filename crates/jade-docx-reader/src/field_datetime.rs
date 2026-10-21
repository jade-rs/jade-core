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

const JAPANESE_WEEKDAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

/// A Japanese imperial era: its first day, its name, its one-kanji
/// abbreviation and the romanized initial Word writes for `g`.
struct JapaneseEra {
    start: (i32, u32, u32),
    name: &'static str,
    short: &'static str,
    initial: &'static str,
}

const JAPANESE_ERAS: [JapaneseEra; 5] = [
    JapaneseEra {
        start: (2019, 5, 1),
        name: "令和",
        short: "令",
        initial: "R",
    },
    JapaneseEra {
        start: (1989, 1, 8),
        name: "平成",
        short: "平",
        initial: "H",
    },
    JapaneseEra {
        start: (1926, 12, 25),
        name: "昭和",
        short: "昭",
        initial: "S",
    },
    JapaneseEra {
        start: (1912, 7, 30),
        name: "大正",
        short: "大",
        initial: "T",
    },
    JapaneseEra {
        start: (1868, 1, 25),
        name: "明治",
        short: "明",
        initial: "M",
    },
];

/// The era a date falls in and the year of that era (1 for its first year).
/// Dates before Meiji count in Meiji, as Word's Japanese calendar does.
fn japanese_era(dt: &FieldDateTime) -> (&'static JapaneseEra, i32) {
    let era = JAPANESE_ERAS
        .iter()
        .find(|era| (dt.year, dt.month, dt.day) >= era.start)
        .unwrap_or(&JAPANESE_ERAS[JAPANESE_ERAS.len() - 1]);
    (era, dt.year - era.start.0 + 1)
}

/// Formats a value using Word's `\@` date-time picture (ECMA-376 Part 1
/// §17.16.4.1). Longest tokens win, single-quoted text is literal, and
/// characters that are no token pass through. Besides the Gregorian tokens
/// it reads Word's Japanese ones: `ggg`/`gg`/`g` for the era name, its
/// abbreviation and its initial, `ee`/`e` for the year of the era — written
/// 元 in an era's first year when 年 follows, as Word does since 令和 — and
/// `aaaa`/`aaa` for the weekday (火曜日, 火).
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
    let starts = |at: usize, token: &str| {
        let mut index = at;
        for expected in token.chars() {
            if chars.get(index) != Some(&expected) {
                return false;
            }
            index += 1;
        }
        true
    };
    // The run of one picture letter starting at `at`, capped at `max`.
    let run_of = |at: usize, letter: char, max: usize| {
        chars[at..]
            .iter()
            .take(max)
            .take_while(|&&character| character == letter)
            .count()
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
        let matched: Option<(usize, String)> = if starts(i, "AM/PM") || starts(i, "am/pm") {
            Some((5, if dt.hour < 12 { "AM" } else { "PM" }.to_string()))
        } else if starts(i, "A/P") {
            Some((3, if dt.hour < 12 { "A" } else { "P" }.to_string()))
        } else if starts(i, "a/p") {
            Some((3, if dt.hour < 12 { "a" } else { "p" }.to_string()))
        } else {
            match chars[i] {
                'y' | 'Y' => {
                    let run = run_of(i, chars[i], 4);
                    Some(if run >= 3 {
                        (run, format!("{:04}", dt.year))
                    } else {
                        (run, format!("{:02}", dt.year.rem_euclid(100)))
                    })
                }
                'M' => {
                    let run = run_of(i, 'M', 4);
                    Some((
                        run,
                        match run {
                            4 => MONTH_NAMES[month_idx].to_string(),
                            3 => MONTH_NAMES[month_idx][..3].to_string(),
                            2 => format!("{:02}", dt.month),
                            _ => dt.month.to_string(),
                        },
                    ))
                }
                'd' | 'D' => {
                    let run = run_of(i, chars[i], 4);
                    Some((
                        run,
                        match run {
                            4 => WEEKDAY_NAMES[weekday_idx].to_string(),
                            3 => WEEKDAY_NAMES[weekday_idx][..3].to_string(),
                            2 => format!("{:02}", dt.day),
                            _ => dt.day.to_string(),
                        },
                    ))
                }
                'H' => {
                    let run = run_of(i, 'H', 2);
                    Some((
                        run,
                        if run == 2 {
                            format!("{:02}", dt.hour)
                        } else {
                            dt.hour.to_string()
                        },
                    ))
                }
                'h' => {
                    let run = run_of(i, 'h', 2);
                    Some((
                        run,
                        if run == 2 {
                            format!("{hour12:02}")
                        } else {
                            hour12.to_string()
                        },
                    ))
                }
                'm' => {
                    let run = run_of(i, 'm', 2);
                    Some((
                        run,
                        if run == 2 {
                            format!("{:02}", dt.minute)
                        } else {
                            dt.minute.to_string()
                        },
                    ))
                }
                's' | 'S' => {
                    let run = run_of(i, chars[i], 2);
                    Some((
                        run,
                        if run == 2 {
                            format!("{:02}", dt.second)
                        } else {
                            dt.second.to_string()
                        },
                    ))
                }
                'g' | 'G' => {
                    let run = run_of(i, chars[i], 3);
                    let (era, _) = japanese_era(dt);
                    Some((
                        run,
                        match run {
                            3 => era.name,
                            2 => era.short,
                            _ => era.initial,
                        }
                        .to_string(),
                    ))
                }
                'e' | 'E' => {
                    let run = run_of(i, chars[i], 2);
                    let (_, year) = japanese_era(dt);
                    Some((
                        run,
                        if run == 2 {
                            format!("{year:02}")
                        } else if year == 1 && chars.get(i + run) == Some(&'年') {
                            "元".to_string()
                        } else {
                            year.to_string()
                        },
                    ))
                }
                'a' if run_of(i, 'a', 4) >= 3 => {
                    let run = run_of(i, 'a', 4);
                    let short = JAPANESE_WEEKDAYS[weekday_idx];
                    Some((
                        run,
                        if run == 4 {
                            format!("{short}曜日")
                        } else {
                            short.to_string()
                        },
                    ))
                }
                _ => None,
            }
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

/// The weekday (0 = Sunday) of a proleptic-Gregorian date.
pub fn weekday_of(year: i32, month: u32, day: u32) -> u32 {
    (days_from_civil(year, month, day) + 4).rem_euclid(7) as u32
}

/// A date-time `seconds` after the Unix epoch, in the zone `offset_seconds`
/// east of UTC.
pub fn datetime_from_unix(seconds: i64, offset_seconds: i64) -> FieldDateTime {
    let local = seconds + offset_seconds;
    let days = local.div_euclid(86_400);
    let in_day = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    FieldDateTime {
        year,
        month,
        day,
        hour: (in_day / 3600) as u32,
        minute: ((in_day % 3600) / 60) as u32,
        second: (in_day % 60) as u32,
        weekday: (days + 4).rem_euclid(7) as u32,
    }
}

/// Seconds since the Unix epoch of a UTC date-time.
pub fn unix_seconds(dt: &FieldDateTime) -> i64 {
    days_from_civil(dt.year, dt.month, dt.day) * 86_400
        + i64::from(dt.hour) * 3600
        + i64::from(dt.minute) * 60
        + i64::from(dt.second)
}

/// The W3CDTF form core properties store (`2026-10-21T09:41:00Z`).
pub fn to_w3cdtf(dt: &FieldDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
    )
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
    fn japanese_era_and_weekday_tokens_follow_word() {
        let reiwa = FieldDateTime {
            year: 2026,
            month: 10,
            day: 21,
            hour: 9,
            minute: 5,
            second: 0,
            weekday: 3,
        };
        assert_eq!(format_word_date("ggge年M月d日", &reiwa), "令和8年10月21日");
        assert_eq!(
            format_word_date("gggee年MM月dd日(aaa)", &reiwa),
            "令和08年10月21日(水)"
        );
        assert_eq!(format_word_date("gge.M.d aaaa", &reiwa), "令8.10.21 水曜日");
        assert_eq!(format_word_date("ge.M.d", &reiwa), "R8.10.21");
        // 元年 in an era's first year, but only before 年.
        let first = FieldDateTime {
            year: 2019,
            month: 5,
            day: 1,
            weekday: 3,
            ..reiwa
        };
        assert_eq!(format_word_date("ggge年M月d日", &first), "令和元年5月1日");
        assert_eq!(format_word_date("ge", &first), "R1");
        // The last day of 平成 and the first of 昭和.
        let heisei = FieldDateTime {
            year: 2019,
            month: 4,
            day: 30,
            ..reiwa
        };
        assert_eq!(format_word_date("ggge年", &heisei), "平成31年");
        let showa = FieldDateTime {
            year: 1926,
            month: 12,
            day: 25,
            ..reiwa
        };
        assert_eq!(format_word_date("ggge年", &showa), "昭和元年");
        assert_eq!(format_word_date("H:mm A/P", &reiwa), "9:05 A");
    }

    #[test]
    fn unix_seconds_round_trip_through_local_offsets() {
        let dt = datetime_from_unix(1_792_573_500, 9 * 3600);
        assert_eq!((dt.year, dt.month, dt.day, dt.hour), (2026, 10, 21, 18));
        assert_eq!(dt.weekday, weekday_of(2026, 10, 21));
        assert_eq!(
            unix_seconds(&datetime_from_unix(1_792_573_500, 0)),
            1_792_573_500
        );
        assert_eq!(to_w3cdtf(&datetime_from_unix(0, 0)), "1970-01-01T00:00:00Z");
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
