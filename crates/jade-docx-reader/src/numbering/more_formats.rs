//! The `ST_NumberFormat` values (ECMA-376 Part 1 §17.18.59) beyond the
//! East Asian, Latin and Middle Eastern sets: Korean Hanja digits,
//! Vietnamese, Hindi and Thai counting, Thai letters, the baht and dollar
//! amount texts, and Word's custom formats (`w:numFmt w:val="custom"
//! w:format="001, 002, 003, ..."`).

/// `koreanDigital2`: Hanja digits, one per decimal digit (一〇).
pub(super) fn korean_digital2(value: u32) -> String {
    const DIGITS: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
    value
        .to_string()
        .bytes()
        .map(|digit| DIGITS[(digit - b'0') as usize])
        .collect()
}

/// `vietnameseCounting`: một, hai, … mười, mười một, hai mươi mốt, một trăm
/// lẻ năm, một nghìn.
pub(super) fn vietnamese_counting(value: u32) -> String {
    const UNITS: [&str; 10] = [
        "không", "một", "hai", "ba", "bốn", "năm", "sáu", "bảy", "tám", "chín",
    ];
    fn under_thousand(value: u32, leading: bool) -> String {
        let hundreds = value / 100;
        let tens = value / 10 % 10;
        let units = value % 10;
        let mut words: Vec<&str> = Vec::new();
        if hundreds > 0 || !leading {
            words.push(UNITS[hundreds as usize]);
            words.push("trăm");
        }
        match tens {
            0 if units > 0 && (hundreds > 0 || !leading) => words.push("lẻ"),
            0 => {}
            1 => words.push("mười"),
            tens => {
                words.push(UNITS[tens as usize]);
                words.push("mươi");
            }
        }
        if units > 0 {
            words.push(match units {
                1 if tens >= 2 => "mốt",
                4 if tens >= 2 => "tư",
                5 if tens >= 1 => "lăm",
                units => UNITS[units as usize],
            });
        }
        words.join(" ")
    }
    if value == 0 {
        return UNITS[0].to_string();
    }
    let mut groups = Vec::new();
    let mut rest = value;
    let mut leading = true;
    for (scale, name) in [
        (1_000_000_000, "tỷ"),
        (1_000_000, "triệu"),
        (1_000, "nghìn"),
    ] {
        let group = rest / scale;
        if group > 0 {
            groups.push(format!("{} {name}", under_thousand(group, leading)));
            leading = false;
        }
        rest %= scale;
    }
    if rest > 0 {
        groups.push(under_thousand(rest, leading));
    }
    groups.join(" ")
}

/// Devanagari vowels for `hindiVowels`.
pub(super) const HINDI_VOWELS: &[char] = &['अ', 'आ', 'इ', 'ई', 'उ', 'ऊ', 'ऋ', 'ए', 'ऐ', 'ओ', 'औ'];
/// Devanagari consonants for `hindiConsonants`.
pub(super) const HINDI_CONSONANTS: &[char] = &[
    'क', 'ख', 'ग', 'घ', 'ङ', 'च', 'छ', 'ज', 'झ', 'ञ', 'ट', 'ठ', 'ड', 'ढ', 'ण', 'त', 'थ', 'द', 'ध',
    'न', 'प', 'फ', 'ब', 'भ', 'म', 'य', 'र', 'ल', 'व', 'श', 'ष', 'स', 'ह',
];

/// `hindiCounting`: Hindi number words; the numbers below a hundred are
/// each a word of their own.
pub(super) fn hindi_counting(value: u32) -> String {
    const UNDER_HUNDRED: [&str; 100] = [
        "शून्य",
        "एक",
        "दो",
        "तीन",
        "चार",
        "पाँच",
        "छह",
        "सात",
        "आठ",
        "नौ",
        "दस",
        "ग्यारह",
        "बारह",
        "तेरह",
        "चौदह",
        "पंद्रह",
        "सोलह",
        "सत्रह",
        "अठारह",
        "उन्नीस",
        "बीस",
        "इक्कीस",
        "बाईस",
        "तेईस",
        "चौबीस",
        "पच्चीस",
        "छब्बीस",
        "सत्ताईस",
        "अट्ठाईस",
        "उनतीस",
        "तीस",
        "इकतीस",
        "बत्तीस",
        "तैंतीस",
        "चौंतीस",
        "पैंतीस",
        "छत्तीस",
        "सैंतीस",
        "अड़तीस",
        "उनतालीस",
        "चालीस",
        "इकतालीस",
        "बयालीस",
        "तैंतालीस",
        "चौवालीस",
        "पैंतालीस",
        "छियालीस",
        "सैंतालीस",
        "अड़तालीस",
        "उनचास",
        "पचास",
        "इक्यावन",
        "बावन",
        "तिरेपन",
        "चौवन",
        "पचपन",
        "छप्पन",
        "सत्तावन",
        "अट्ठावन",
        "उनसठ",
        "साठ",
        "इकसठ",
        "बासठ",
        "तिरसठ",
        "चौंसठ",
        "पैंसठ",
        "छियासठ",
        "सड़सठ",
        "अड़सठ",
        "उनहत्तर",
        "सत्तर",
        "इकहत्तर",
        "बहत्तर",
        "तिहत्तर",
        "चौहत्तर",
        "पचहत्तर",
        "छिहत्तर",
        "सतहत्तर",
        "अठहत्तर",
        "उन्यासी",
        "अस्सी",
        "इक्यासी",
        "बयासी",
        "तिरासी",
        "चौरासी",
        "पचासी",
        "छियासी",
        "सत्तासी",
        "अट्ठासी",
        "नवासी",
        "नब्बे",
        "इक्यानवे",
        "बानवे",
        "तिरानवे",
        "चौरानवे",
        "पचानवे",
        "छियानवे",
        "सत्तानवे",
        "अट्ठानवे",
        "निन्यानवे",
    ];
    if value < 100 {
        return UNDER_HUNDRED[value as usize].to_string();
    }
    // Indian grouping: करोड़ (10⁷), लाख (10⁵), हज़ार (10³), सौ (10²).
    let mut words = Vec::new();
    let mut rest = value;
    for (scale, name) in [
        (10_000_000, "करोड़"),
        (100_000, "लाख"),
        (1_000, "हज़ार"),
        (100, "सौ"),
    ] {
        let count = rest / scale;
        if count > 0 {
            words.push(format!("{} {name}", hindi_counting(count)));
        }
        rest %= scale;
    }
    if rest > 0 {
        words.push(UNDER_HUNDRED[rest as usize].to_string());
    }
    words.join(" ")
}

/// Thai consonants for `thaiLetters`, without the obsolete ฃ and ฅ.
pub(super) const THAI_LETTERS: &[char] = &[
    'ก', 'ข', 'ค', 'ฆ', 'ง', 'จ', 'ฉ', 'ช', 'ซ', 'ฌ', 'ญ', 'ฎ', 'ฏ', 'ฐ', 'ฑ', 'ฒ', 'ณ', 'ด', 'ต',
    'ถ', 'ท', 'ธ', 'น', 'บ', 'ป', 'ผ', 'ฝ', 'พ', 'ฟ', 'ภ', 'ม', 'ย', 'ร', 'ล', 'ว', 'ศ', 'ษ', 'ส',
    'ห', 'ฬ', 'อ', 'ฮ',
];

/// `thaiCounting`: หนึ่ง, สอง, … สิบ, สิบเอ็ด, ยี่สิบ, หนึ่งร้อย, หนึ่งพัน.
pub(super) fn thai_counting(value: u32) -> String {
    const DIGITS: [&str; 10] = [
        "ศูนย์",
        "หนึ่ง",
        "สอง",
        "สาม",
        "สี่",
        "ห้า",
        "หก",
        "เจ็ด",
        "แปด",
        "เก้า",
    ];
    const PLACES: [&str; 6] = ["", "สิบ", "ร้อย", "พัน", "หมื่น", "แสน"];
    fn under_million(value: u32) -> String {
        let digits: Vec<u32> = value
            .to_string()
            .bytes()
            .map(|digit| u32::from(digit - b'0'))
            .collect();
        let mut text = String::new();
        let count = digits.len();
        for (index, digit) in digits.iter().copied().enumerate() {
            let place = count - 1 - index;
            if digit == 0 {
                continue;
            }
            match (place, digit) {
                (1, 1) => {}
                (1, 2) => text.push_str("ยี่"),
                (0, 1) if count > 1 => {
                    text.push_str("เอ็ด");
                    continue;
                }
                _ => text.push_str(DIGITS[digit as usize]),
            }
            text.push_str(PLACES[place]);
        }
        text
    }
    if value == 0 {
        return DIGITS[0].to_string();
    }
    let millions = value / 1_000_000;
    let rest = value % 1_000_000;
    let mut text = String::new();
    if millions > 0 {
        text.push_str(&thai_counting(millions));
        text.push_str("ล้าน");
    }
    if rest > 0 {
        text.push_str(&under_million(rest));
    }
    text
}

/// `bahtText`: the amount in Thai words and baht, with no satang.
pub(super) fn baht_text(value: u32) -> String {
    format!("{}บาทถ้วน", thai_counting(value))
}

/// `dollarText`: the amount in English words with no cents, as Word's
/// DollarText writes it.
pub(super) fn dollar_text(value: u32) -> String {
    format!("{} and 00/100", super::to_english_cardinal(value))
}

/// A custom format (`w:format="001, 002, 003, ..."`): the first sample's
/// digits set the width and the digit set (ASCII or full-width); anything
/// else is decimal.
pub(super) fn custom(sample: &str, value: u32) -> String {
    let first = sample.split(',').next().unwrap_or("").trim();
    let width = first.chars().filter(char::is_ascii_digit).count();
    let full_width = first.chars().filter(|c| ('０'..='９').contains(c)).count();
    let decimal = value.to_string();
    if width > 0 && full_width == 0 {
        let digits = format!("{decimal:0>width$}");
        return replace_digits(first, &digits, false);
    }
    if full_width > 0 {
        let digits = format!("{decimal:0>full_width$}");
        return replace_digits(first, &digits, true);
    }
    decimal
}

/// `pattern` with its run of digits replaced by `digits` (as full-width
/// digits when `full`), keeping any text around them (`(001)`).
fn replace_digits(pattern: &str, digits: &str, full: bool) -> String {
    let is_digit = |c: char| c.is_ascii_digit() || ('０'..='９').contains(&c);
    let start = pattern.find(is_digit).unwrap_or(0);
    let end = pattern
        .char_indices()
        .rfind(|(_, c)| is_digit(*c))
        .map_or(start, |(index, c)| index + c.len_utf8());
    let digits: String = if full {
        digits
            .chars()
            .map(|c| char::from_u32(u32::from(c) - u32::from('0') + u32::from('０')).unwrap_or(c))
            .collect()
    } else {
        digits.to_string()
    };
    format!("{}{digits}{}", &pattern[..start], &pattern[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vietnamese_counting_follows_the_spoken_forms() {
        assert_eq!(vietnamese_counting(1), "một");
        assert_eq!(vietnamese_counting(10), "mười");
        assert_eq!(vietnamese_counting(15), "mười lăm");
        assert_eq!(vietnamese_counting(21), "hai mươi mốt");
        assert_eq!(vietnamese_counting(24), "hai mươi tư");
        assert_eq!(vietnamese_counting(105), "một trăm lẻ năm");
        assert_eq!(vietnamese_counting(1_005), "một nghìn không trăm lẻ năm");
    }

    #[test]
    fn hindi_and_thai_counting() {
        assert_eq!(hindi_counting(1), "एक");
        assert_eq!(hindi_counting(21), "इक्कीस");
        assert_eq!(hindi_counting(105), "एक सौ पाँच");
        assert_eq!(hindi_counting(2_000), "दो हज़ार");
        assert_eq!(thai_counting(1), "หนึ่ง");
        assert_eq!(thai_counting(11), "สิบเอ็ด");
        assert_eq!(thai_counting(21), "ยี่สิบเอ็ด");
        assert_eq!(thai_counting(101), "หนึ่งร้อยเอ็ด");
        assert_eq!(thai_counting(1_000_000), "หนึ่งล้าน");
        assert_eq!(baht_text(25), "ยี่สิบห้าบาทถ้วน");
    }

    #[test]
    fn custom_formats_pad_to_their_sample() {
        assert_eq!(custom("001, 002, 003, ...", 7), "007");
        assert_eq!(custom("001, 002, 003, ...", 1234), "1234");
        assert_eq!(custom("０１, ０２, ０３, ...", 5), "０５");
        assert_eq!(custom("(01), (02), ...", 3), "(03)");
        assert_eq!(custom("a, b, c", 3), "3");
    }

    #[test]
    fn korean_digital2_and_dollar_text() {
        assert_eq!(korean_digital2(105), "一零五");
        assert_eq!(dollar_text(123), "one hundred twenty-three and 00/100");
    }
}
