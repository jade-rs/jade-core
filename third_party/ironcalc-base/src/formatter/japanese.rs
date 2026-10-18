//! Japanese date codes (Jade addition): the era `g` and its year `e`, and
//! the weekday `aaa`. Excel prints them in Japanese whatever the locale.

use chrono::{Datelike, NaiveDate};

use super::parser::TextToken;

/// An era: its first day, Latin initial, short and full names.
type Era = ((i32, u32, u32), &'static str, &'static str, &'static str);

/// The eras a date serial can reach.
const ERAS: [Era; 5] = [
    ((1868, 1, 1), "M", "明", "明治"),
    ((1912, 7, 30), "T", "大", "大正"),
    ((1926, 12, 25), "S", "昭", "昭和"),
    ((1989, 1, 8), "H", "平", "平成"),
    ((2019, 5, 1), "R", "令", "令和"),
];

const DAY_NAMES: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

fn era(date: NaiveDate) -> (usize, i32) {
    let key = (date.year(), date.month(), date.day());
    let index = ERAS
        .iter()
        .rposition(|(start, ..)| key >= *start)
        .unwrap_or(0);
    (index, date.year() - ERAS[index].0 .0 + 1)
}

/// The text one of the Japanese tokens stands for on `date`.
pub(crate) fn render(token: &TextToken, date: NaiveDate) -> String {
    match token {
        TextToken::Era(width) => {
            let (_, latin, short, full) = ERAS[era(date).0];
            match width {
                1 => latin,
                2 => short,
                _ => full,
            }
            .to_string()
        }
        TextToken::EraYear(width) => {
            let year = era(date).1;
            if *width >= 2 {
                format!("{year:02}")
            } else {
                year.to_string()
            }
        }
        TextToken::JapaneseWeekday(width) => {
            let day = DAY_NAMES[date.weekday().num_days_from_sunday() as usize];
            if *width >= 4 {
                format!("{day}曜日")
            } else {
                day.to_string()
            }
        }
        _ => String::new(),
    }
}
