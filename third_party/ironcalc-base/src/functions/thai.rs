//! Jade addition: BAHTTEXT, an amount in Thai words as Excel writes it.

use crate::calc_result::CalcResult;
use crate::expressions::parser::Node;
use crate::expressions::types::CellReferenceIndex;
use crate::model::Model;

const DIGITS: [&str; 10] = [
    "ศูนย์", "หนึ่ง", "สอง", "สาม", "สี่", "ห้า", "หก", "เจ็ด", "แปด", "เก้า",
];
/// Hundreds to hundred-thousands; tens are written apart.
const PLACES: [&str; 4] = ["ร้อย", "พัน", "หมื่น", "แสน"];

/// `value` (below a million) in words: ten is สิบ, twenty ยี่สิบ, and a final
/// one reads เอ็ด after any higher digit — of this group, or of the millions
/// before it (`after_higher`).
fn group(value: u64, after_higher: bool, text: &mut String) {
    for (place, name) in PLACES.iter().enumerate().rev() {
        let digit = (value / 10u64.pow(place as u32 + 2)) % 10;
        if digit > 0 {
            text.push_str(DIGITS[digit as usize]);
            text.push_str(name);
        }
    }
    let (tens, ones) = ((value / 10) % 10, value % 10);
    match tens {
        0 => {}
        1 => text.push_str("สิบ"),
        2 => text.push_str("ยี่สิบ"),
        _ => {
            text.push_str(DIGITS[tens as usize]);
            text.push_str("สิบ");
        }
    }
    if ones == 1 && (after_higher || value > 1) {
        text.push_str("เอ็ด");
    } else if ones > 0 {
        text.push_str(DIGITS[ones as usize]);
    }
}

/// A whole number in words, the millions read as a number of ล้าน.
fn whole(value: u64, text: &mut String) {
    if value >= 1_000_000 {
        whole(value / 1_000_000, text);
        text.push_str("ล้าน");
        group(value % 1_000_000, true, text);
    } else {
        group(value, false, text);
    }
}

/// `value` rounded to the satang, in words: บาท then ถ้วน (exact) or the
/// satang and สตางค์; ลบ before a negative amount.
pub(crate) fn baht_text(value: f64) -> String {
    let satang_total = (value.abs() * 100.0).round() as u64;
    let (baht, satang) = (satang_total / 100, satang_total % 100);
    if satang_total == 0 {
        return "ศูนย์บาทถ้วน".to_string();
    }
    let mut text = String::new();
    if value < 0.0 {
        text.push_str("ลบ");
    }
    if baht > 0 {
        whole(baht, &mut text);
        text.push_str("บาท");
    }
    if satang == 0 {
        text.push_str("ถ้วน");
    } else {
        group(satang, false, &mut text);
        text.push_str("สตางค์");
    }
    text
}

impl<'a> Model<'a> {
    pub(crate) fn fn_bahttext(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        match self.get_number(&args[0], cell) {
            Ok(value) => CalcResult::String(baht_text(value)),
            Err(error) => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::baht_text;

    #[test]
    fn a_final_one_reads_et_after_any_higher_digit() {
        assert_eq!(baht_text(1.0), "หนึ่งบาทถ้วน");
        assert_eq!(baht_text(11.0), "สิบเอ็ดบาทถ้วน");
        assert_eq!(baht_text(1001.0), "หนึ่งพันเอ็ดบาทถ้วน");
        assert_eq!(baht_text(1_000_001.0), "หนึ่งล้านเอ็ดบาทถ้วน");
        assert_eq!(baht_text(0.01), "หนึ่งสตางค์");
        assert_eq!(baht_text(0.21), "ยี่สิบเอ็ดสตางค์");
    }

    #[test]
    fn millions_of_millions_read_as_a_number_of_lan() {
        assert_eq!(baht_text(1e12), "หนึ่งล้านล้านบาทถ้วน");
        assert_eq!(baht_text(2_500_000.0), "สองล้านห้าแสนบาทถ้วน");
    }
}
