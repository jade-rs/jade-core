//! Shared WordprocessingML counter formatting for display and text extraction.

mod more_formats;

/// Formats `value` with an `ST_NumberFormat` token (ECMA-376 Part 1
/// §17.18.59). List levels, page numbers (`w:pgNumType/@w:fmt`) and note
/// markers (`w:numFmt` in `w:footnotePr`/`w:endnotePr`) all name their counter
/// style with this one simple type, so they share this formatter. Tokens match
/// case-insensitively: section and note formats are kept lowercased by the
/// parser. Unknown tokens fall back to decimal.
///
/// Zero has no representation in most systems; it stays a decimal `0` here
/// and list levels clamp to one before calling.
pub fn format_number_value(format: &str, value: u32, language: Option<&str>) -> String {
    let format = canonical_number_format(format).unwrap_or(format);
    if value == 0
        && !matches!(
            format,
            "decimal"
                | "decimalHalfWidth"
                | "decimalFullWidth"
                | "decimalFullWidth2"
                | "decimalZero"
                | "hindiNumbers"
                | "thaiNumbers"
                | "numberInDash"
                | "hex"
                | "none"
                | "bullet"
        )
    {
        return value.to_string();
    }
    match format {
        "decimal" => value.to_string(),
        "upperRoman" => to_roman(value, true),
        "lowerRoman" => to_roman(value, false),
        "upperLetter" => to_letter(value, true),
        "lowerLetter" => to_letter(value, false),
        "ordinal" if language.is_some_and(is_english_language) => to_english_ordinal(value),
        "cardinalText" if language.is_some_and(is_english_language) => to_english_cardinal(value),
        "ordinalText" if language.is_some_and(is_english_language) => {
            to_english_ordinal_text(value)
        }
        "hex" => to_hex(value),
        // `target/contract-numbering-word-trace.json` records Word COM
        // `ListString` as `第１条　` for a `decimalFullWidth` heading.
        // Keeping this formatter explicit lets TRACE reports distinguish
        // true numbering interpretation bugs from later layout/metric drift.
        "decimalFullWidth" | "decimalFullWidth2" => to_full_width_decimal(value),
        // Keep this token explicit even though Rust's decimal output is
        // already single-byte ASCII. Otherwise a future generic fallback
        // could silently change a format whose width is layout-significant.
        "decimalHalfWidth" => value.to_string(),
        // `decimalZero` is a *minimum* width of two, not a fixed-width
        // counter: Word emits 01..09 and then continues with 10, 11, ... .
        // This distinction affects the numbering label advance and thus the
        // first-line text origin for single-digit list items.
        "decimalZero" => format!("{value:02}"),
        // Word follows the ST_NumberFormat examples: the unsuffixed token
        // uses half-width Katakana and the FullWidth token uses ordinary
        // full-width Katakana. This intentionally follows observed Word
        // interoperability rather than the contradictory "Hiragana" label
        // attached to aiueo in the schema prose.
        "aiueo" => to_kana_sequence(value, AIUEO_KATAKANA_HALF_WIDTH),
        "aiueoFullWidth" => to_kana_sequence(value, AIUEO_KATAKANA_FULL_WIDTH),
        "iroha" => to_kana_sequence(value, IROHA_KATAKANA_HALF_WIDTH),
        "irohaFullWidth" => to_kana_sequence(value, IROHA_KATAKANA_FULL_WIDTH),
        // Both formats are positional digit sequences (10 -> 一〇), unlike
        // japaneseCounting (10 -> 十). Microsoft notes that Word does not
        // apply the circle suggested by old ideographDigital prose.
        "japaneseDigitalTenThousand" | "ideographDigital" => to_japanese_positional_digits(value),
        "japaneseCounting" => to_japanese_counting(value, JapaneseKanjiStyle::Standard),
        "chineseCountingThousand" => to_japanese_counting(value, JapaneseKanjiStyle::Standard),
        "japaneseLegal" => to_japanese_counting(value, JapaneseKanjiStyle::Legal),
        // ST_NumberFormat defines dedicated Unicode enclosed glyphs only
        // through 20 (or 10 for the Chinese circle variant). Word permits
        // ordinary decimal output after that range, which is preferable to
        // synthesizing a visually different combining enclosure.
        "decimalEnclosedCircle" => to_enclosed_decimal(value, '①', 20),
        "decimalEnclosedFullstop" => to_enclosed_decimal(value, '⒈', 20),
        "decimalEnclosedParen" => to_enclosed_decimal(value, '⑴', 20),
        "decimalEnclosedCircleChinese" => to_enclosed_decimal(value, '①', 10),
        "ideographEnclosedCircle" => to_enclosed_ideograph(value),
        // These OOXML formats preserve the decimal place-value system and
        // replace each ASCII digit with the corresponding native digit.
        // Formatting the whole value first also handles embedded zeroes.
        "hindiNumbers" => to_decimal_digit_set(value, '०'),
        "thaiNumbers" => to_decimal_digit_set(value, '๐'),
        "numberInDash" => format!("- {value} -"),
        "chicago" => to_chicago_symbols(value),
        // Ten heavenly stems, twelve earthly branches and their sixty-term
        // sexagenary cycle. These are closed sets; Word wraps round them.
        "ideographTraditional" => cycle_char(value, HEAVENLY_STEMS),
        "ideographZodiac" => cycle_char(value, EARTHLY_BRANCHES),
        "ideographZodiacTraditional" => to_sexagenary(value),
        "taiwaneseDigital" => to_japanese_positional_digits(value),
        "chineseCounting" | "taiwaneseCounting" | "taiwaneseCountingThousand" => {
            to_chinese_counting(value, ChineseNumerals::COUNTING)
        }
        "chineseLegalSimplified" => to_chinese_counting(value, ChineseNumerals::LEGAL_SIMPLIFIED),
        "ideographLegalTraditional" => {
            to_chinese_counting(value, ChineseNumerals::LEGAL_TRADITIONAL)
        }
        "koreanDigital" => to_decimal_digits(value, KOREAN_DIGITS),
        "koreanCounting" => to_korean_counting(value),
        "koreanLegal" => to_korean_native(value),
        "ganada" => cycle_char(value, GANADA),
        "chosung" => cycle_char(value, CHOSUNG),
        "russianLower" => to_repeated_letter(value, RUSSIAN_LOWER),
        "russianUpper" => to_repeated_letter(value, RUSSIAN_UPPER),
        "hebrew1" => to_hebrew_numeral(value),
        "hebrew2" => to_repeated_letter(value, HEBREW_ALPHABET),
        "arabicAlpha" => to_repeated_letter(value, ARABIC_ALPHA),
        "arabicAbjad" => to_repeated_letter(value, ARABIC_ABJAD),
        "koreanDigital2" => more_formats::korean_digital2(value),
        "vietnameseCounting" => more_formats::vietnamese_counting(value),
        "hindiVowels" => to_repeated_letter(value, more_formats::HINDI_VOWELS),
        "hindiConsonants" => to_repeated_letter(value, more_formats::HINDI_CONSONANTS),
        "hindiCounting" => more_formats::hindi_counting(value),
        "thaiLetters" => to_repeated_letter(value, more_formats::THAI_LETTERS),
        "thaiCounting" => more_formats::thai_counting(value),
        "bahtText" => more_formats::baht_text(value),
        "dollarText" => more_formats::dollar_text(value),
        custom if custom.starts_with(CUSTOM_FORMAT_PREFIX) => {
            more_formats::custom(&custom[CUSTOM_FORMAT_PREFIX.len()..], value)
        }
        "none" => String::new(),
        "bullet" => "•".to_string(),
        _ => value.to_string(),
    }
}

/// Every `ST_NumberFormat` token this module formats, in schema spelling.
const NUMBER_FORMAT_TOKENS: &[&str] = &[
    "decimal",
    "upperRoman",
    "lowerRoman",
    "upperLetter",
    "lowerLetter",
    "ordinal",
    "cardinalText",
    "ordinalText",
    "hex",
    "chicago",
    "ideographDigital",
    "japaneseCounting",
    "aiueo",
    "iroha",
    "decimalFullWidth",
    "decimalHalfWidth",
    "japaneseLegal",
    "japaneseDigitalTenThousand",
    "decimalEnclosedCircle",
    "decimalFullWidth2",
    "aiueoFullWidth",
    "irohaFullWidth",
    "decimalZero",
    "bullet",
    "ganada",
    "chosung",
    "decimalEnclosedFullstop",
    "decimalEnclosedParen",
    "decimalEnclosedCircleChinese",
    "ideographEnclosedCircle",
    "ideographTraditional",
    "ideographZodiac",
    "ideographZodiacTraditional",
    "taiwaneseCounting",
    "ideographLegalTraditional",
    "taiwaneseCountingThousand",
    "taiwaneseDigital",
    "chineseCounting",
    "chineseLegalSimplified",
    "chineseCountingThousand",
    "koreanDigital",
    "koreanCounting",
    "koreanLegal",
    "russianLower",
    "russianUpper",
    "none",
    "numberInDash",
    "hebrew1",
    "hebrew2",
    "arabicAlpha",
    "arabicAbjad",
    "hindiNumbers",
    "thaiNumbers",
    "koreanDigital2",
    "vietnameseCounting",
    "hindiVowels",
    "hindiConsonants",
    "hindiCounting",
    "thaiLetters",
    "thaiCounting",
    "bahtText",
    "dollarText",
];

/// How a level's custom format is kept in `num_fmt`: Word writes
/// `w:numFmt w:val="custom" w:format="001, 002, 003, ..."` (with a decimal
/// fallback for older readers), and the sample is what to format by.
pub const CUSTOM_FORMAT_PREFIX: &str = "custom:";

/// The schema spelling of an `ST_NumberFormat` token this module formats,
/// matched case-insensitively (`"upperroman"` → `"upperRoman"`).
pub fn canonical_number_format(format: &str) -> Option<&'static str> {
    NUMBER_FORMAT_TOKENS
        .iter()
        .find(|token| token.eq_ignore_ascii_case(format))
        .copied()
}

/// Chicago Manual of Style note symbols: `*`, `†`, `‡`, `§`, then the same
/// four doubled, tripled, and so on.
fn to_chicago_symbols(value: u32) -> String {
    const SYMBOLS: [char; 4] = ['*', '†', '‡', '§'];
    to_repeated_symbol(value, &SYMBOLS)
}

const HEAVENLY_STEMS: &str = "甲乙丙丁戊己庚辛壬癸";
const EARTHLY_BRANCHES: &str = "子丑寅卯辰巳午未申酉戌亥";

/// Korean consonant-plus-a syllables, the order Hangul dictionaries use.
const GANADA: &str = "가나다라마바사아자차카타파하";
/// The fourteen basic initial consonants (jamo) in the same order.
const CHOSUNG: &str = "ㄱㄴㄷㄹㅁㅂㅅㅇㅈㅊㅋㅌㅍㅎ";

/// The Russian alphabet without ё, й, ъ, ы and ь, which are not used as
/// list labels.
const RUSSIAN_LOWER: &[char] = &[
    'а', 'б', 'в', 'г', 'д', 'е', 'ж', 'з', 'и', 'к', 'л', 'м', 'н', 'о', 'п', 'р', 'с', 'т', 'у',
    'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'э', 'ю', 'я',
];
const RUSSIAN_UPPER: &[char] = &[
    'А', 'Б', 'В', 'Г', 'Д', 'Е', 'Ж', 'З', 'И', 'К', 'Л', 'М', 'Н', 'О', 'П', 'Р', 'С', 'Т', 'У',
    'Ф', 'Х', 'Ц', 'Ч', 'Ш', 'Щ', 'Э', 'Ю', 'Я',
];
const HEBREW_ALPHABET: &[char] = &[
    'א', 'ב', 'ג', 'ד', 'ה', 'ו', 'ז', 'ח', 'ט', 'י', 'כ', 'ל', 'מ', 'נ', 'ס', 'ע', 'פ', 'צ', 'ק',
    'ר', 'ש', 'ת',
];
/// Hijāʾī order (letters grouped by shape).
const ARABIC_ALPHA: &[char] = &[
    'أ', 'ب', 'ت', 'ث', 'ج', 'ح', 'خ', 'د', 'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', 'ط', 'ظ', 'ع', 'غ',
    'ف', 'ق', 'ك', 'ل', 'م', 'ن', 'ه', 'و', 'ي',
];
/// Abjadī order (the letters' traditional numeric values 1, 2, 3, …).
const ARABIC_ABJAD: &[char] = &[
    'أ', 'ب', 'ج', 'د', 'ه', 'و', 'ز', 'ح', 'ط', 'ي', 'ك', 'ل', 'م', 'ن', 'س', 'ع', 'ف', 'ص', 'ق',
    'ر', 'ش', 'ت', 'ث', 'خ', 'ذ', 'ض', 'ظ', 'غ',
];
const KOREAN_DIGITS: [char; 10] = ['영', '일', '이', '삼', '사', '오', '육', '칠', '팔', '구'];

fn cycle_char(value: u32, alphabet: &str) -> String {
    let len = alphabet.chars().count() as u32;
    alphabet
        .chars()
        .nth(((value.max(1) - 1) % len) as usize)
        .map(String::from)
        .unwrap_or_default()
}

/// `甲子`, `乙丑`, … `癸亥`: the stem and branch advance together, so the
/// cycle repeats after sixty terms.
fn to_sexagenary(value: u32) -> String {
    let index = (value.max(1) - 1) % 60;
    let mut label = cycle_char(index % 10 + 1, HEAVENLY_STEMS);
    label.push_str(&cycle_char(index % 12 + 1, EARTHLY_BRANCHES));
    label
}

/// Longest label the repeating formats build before falling back to decimal;
/// Word's own lettered lists stop well short of this.
const MAX_REPEATED_LABEL: u32 = 40;

/// Word's alphabetic rollover: after the last letter the first comes back
/// doubled, then the second doubled (`z`, `aa`, `bb`, …), not the
/// spreadsheet-style `aa`, `ab`.
fn to_repeated_letter(value: u32, alphabet: &[char]) -> String {
    to_repeated_symbol(value, alphabet)
}

fn to_repeated_symbol(value: u32, alphabet: &[char]) -> String {
    let len = alphabet.len() as u32;
    let index = value.max(1) - 1;
    let times = index / len + 1;
    if times > MAX_REPEATED_LABEL {
        return value.to_string();
    }
    std::iter::repeat_n(alphabet[(index % len) as usize], times as usize).collect()
}

fn to_decimal_digits(value: u32, digits: [char; 10]) -> String {
    value
        .to_string()
        .bytes()
        .map(|digit| digits[(digit - b'0') as usize])
        .collect()
}

/// Digits, small units and large units of one Chinese counting style.
struct ChineseNumerals {
    /// Index 0 is the zero written between non-zero places (一百〇一).
    digits: [char; 10],
    /// 十, 百, 千 (or their financial forms).
    small_units: [char; 3],
    /// Units for the 10⁴ and 10⁸ groups.
    large_units: [&'static str; 2],
    /// Everyday counting writes 10–19 as 十, 十一, …; the legal (financial)
    /// styles always keep the digit: 壹拾.
    omit_leading_one_before_ten: bool,
}

impl ChineseNumerals {
    const COUNTING: Self = Self {
        digits: ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'],
        small_units: ['十', '百', '千'],
        large_units: ["万", "亿"],
        omit_leading_one_before_ten: true,
    };
    const LEGAL_SIMPLIFIED: Self = Self {
        digits: ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'],
        small_units: ['拾', '佰', '仟'],
        large_units: ["万", "亿"],
        omit_leading_one_before_ten: false,
    };
    const LEGAL_TRADITIONAL: Self = Self {
        digits: ['零', '壹', '貳', '參', '肆', '伍', '陸', '柒', '捌', '玖'],
        small_units: ['拾', '佰', '仟'],
        large_units: ["萬", "億"],
        omit_leading_one_before_ten: false,
    };
}

/// Chinese place-value counting. Unlike Japanese counting, the digit one is
/// kept before 百 and 千 (一百, 一千), and a run of empty places between
/// non-zero ones is written as a single zero (一百〇一, 一万〇一).
fn to_chinese_counting(value: u32, numerals: ChineseNumerals) -> String {
    let groups = [value / 100_000_000, value / 10_000 % 10_000, value % 10_000];
    let mut result = String::new();
    let mut gap = false;
    for (index, &group) in groups.iter().enumerate() {
        if group == 0 {
            gap |= !result.is_empty();
            continue;
        }
        if !result.is_empty() && (gap || group < 1_000) {
            result.push(numerals.digits[0]);
        }
        gap = false;
        push_chinese_group(&mut result, group, &numerals);
        result.push_str(match index {
            0 => numerals.large_units[1],
            1 => numerals.large_units[0],
            _ => "",
        });
    }
    result
}

fn push_chinese_group(result: &mut String, group: u32, numerals: &ChineseNumerals) {
    let leading = result.is_empty();
    let places = [group / 1_000, group / 100 % 10, group / 10 % 10, group % 10];
    let units = [
        Some(numerals.small_units[2]),
        Some(numerals.small_units[1]),
        Some(numerals.small_units[0]),
        None,
    ];
    let mut started = false;
    let mut zero_run = false;
    for (place, (&digit, unit)) in places.iter().zip(units).enumerate() {
        if digit == 0 {
            zero_run |= started;
            continue;
        }
        if zero_run {
            result.push(numerals.digits[0]);
            zero_run = false;
        }
        let bare_ten =
            numerals.omit_leading_one_before_ten && leading && !started && place == 2 && digit == 1;
        if !bare_ten {
            result.push(numerals.digits[digit as usize]);
        }
        if let Some(unit) = unit {
            result.push(unit);
        }
        started = true;
    }
}

/// Sino-Korean numerals in Hangul: 일, 이, … 십, 십일, … 백, 천, 만. As in
/// Japanese counting the digit one is dropped before a unit, and Korean
/// writes no zero for empty places.
fn to_korean_counting(value: u32) -> String {
    const UNITS: [&str; 4] = ["천", "백", "십", ""];
    const LARGE: [&str; 3] = ["억", "만", ""];
    let groups = [value / 100_000_000, value / 10_000 % 10_000, value % 10_000];
    let mut result = String::new();
    for (index, &group) in groups.iter().enumerate() {
        if group == 0 {
            continue;
        }
        if !(group == 1 && index == 1) {
            let places = [group / 1_000, group / 100 % 10, group / 10 % 10, group % 10];
            for (place, &digit) in places.iter().enumerate() {
                if digit == 0 {
                    continue;
                }
                if digit != 1 || place == 3 {
                    result.push(KOREAN_DIGITS[digit as usize]);
                }
                result.push_str(UNITS[place]);
            }
        }
        result.push_str(LARGE[index]);
    }
    result
}

/// Native Korean numerals (하나, 둘, … 아흔아홉). They stop at 99; larger
/// values are written in decimal.
fn to_korean_native(value: u32) -> String {
    const TENS: [&str; 10] = [
        "", "열", "스물", "서른", "마흔", "쉰", "예순", "일흔", "여든", "아흔",
    ];
    const ONES: [&str; 10] = [
        "", "하나", "둘", "셋", "넷", "다섯", "여섯", "일곱", "여덟", "아홉",
    ];
    if value > 99 {
        return value.to_string();
    }
    format!(
        "{}{}",
        TENS[(value / 10) as usize],
        ONES[(value % 10) as usize]
    )
}

/// Hebrew alphabetic numerals (gematria) for 1–999. 15 and 16 are written
/// טו and טז rather than spelling a divine name. Larger values stay decimal.
fn to_hebrew_numeral(value: u32) -> String {
    const ONES: [&str; 10] = ["", "א", "ב", "ג", "ד", "ה", "ו", "ז", "ח", "ט"];
    const TENS: [&str; 10] = ["", "י", "כ", "ל", "מ", "נ", "ס", "ע", "פ", "צ"];
    const HUNDREDS: [&str; 10] = ["", "ק", "ר", "ש", "ת", "תק", "תר", "תש", "תת", "תתק"];
    if value > 999 {
        return value.to_string();
    }
    let mut result = String::from(HUNDREDS[(value / 100) as usize]);
    match value % 100 {
        15 => result.push_str("טו"),
        16 => result.push_str("טז"),
        rest => {
            result.push_str(TENS[(rest / 10) as usize]);
            result.push_str(ONES[(rest % 10) as usize]);
        }
    }
    result
}

pub fn to_roman(num: u32, uppercase: bool) -> String {
    let values = [1000, 900, 500, 400, 100, 90, 50, 40, 10, 9, 5, 4, 1];
    let symbols = if uppercase {
        [
            "M", "CM", "D", "CD", "C", "XC", "L", "XL", "X", "IX", "V", "IV", "I",
        ]
    } else {
        [
            "m", "cm", "d", "cd", "c", "xc", "l", "xl", "x", "ix", "v", "iv", "i",
        ]
    };

    let mut result = String::new();
    let mut n = num;

    for (i, &value) in values.iter().enumerate() {
        while n >= value {
            result.push_str(symbols[i]);
            n -= value;
        }
    }

    result
}

pub fn to_letter(num: u32, uppercase: bool) -> String {
    const UPPER: [char; 26] = [
        'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R',
        'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
    ];
    let label = to_repeated_letter(num, &UPPER);
    if uppercase {
        label
    } else {
        label.to_ascii_lowercase()
    }
}

pub fn to_hex(num: u32) -> String {
    format!("{num:X}")
}

pub fn is_english_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("en")
        || language
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("en-"))
}

pub fn to_english_ordinal(value: u32) -> String {
    // 11, 12, and 13 are the exception even when the final digit would
    // otherwise select st/nd/rd (for example 112th).
    let suffix = if (11..=13).contains(&(value % 100)) {
        "th"
    } else {
        match value % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{value}{suffix}")
}

pub fn to_english_cardinal(mut value: u32) -> String {
    if value == 0 {
        return "zero".to_string();
    }
    let mut groups = Vec::new();

    // Word's English CardText output uses short-scale groups and omits the
    // British-style "and" (for example, "seven hundred ninety").
    for (scale, name) in [
        (1_000_000_000, "billion"),
        (1_000_000, "million"),
        (1_000, "thousand"),
    ] {
        let group = value / scale;
        if group > 0 {
            groups.push(format!("{} {name}", to_english_under_thousand(group)));
            value %= scale;
        }
    }

    if value > 0 {
        groups.push(to_english_under_thousand(value));
    }

    groups.join(" ")
}

fn to_english_under_thousand(value: u32) -> String {
    debug_assert!((1..1000).contains(&value));

    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];

    let hundreds = value / 100;
    let remainder = value % 100;
    let mut result = String::new();

    if hundreds > 0 {
        result.push_str(ONES[hundreds as usize]);
        result.push_str(" hundred");
        if remainder > 0 {
            result.push(' ');
        }
    }

    if remainder < 20 {
        if remainder > 0 {
            result.push_str(ONES[remainder as usize]);
        }
    } else {
        result.push_str(TENS[(remainder / 10) as usize]);
        if !remainder.is_multiple_of(10) {
            result.push('-');
            result.push_str(ONES[(remainder % 10) as usize]);
        }
    }

    result
}

pub fn to_english_ordinal_text(value: u32) -> String {
    let cardinal = to_english_cardinal(value);
    let last_separator = cardinal.rfind([' ', '-']);
    let (prefix, final_word) = match last_separator {
        Some(index) => cardinal.split_at(index + 1),
        None => ("", cardinal.as_str()),
    };

    let ordinal_word = match final_word {
        "one" => "first".to_string(),
        "two" => "second".to_string(),
        "three" => "third".to_string(),
        "five" => "fifth".to_string(),
        "eight" => "eighth".to_string(),
        "nine" => "ninth".to_string(),
        "twelve" => "twelfth".to_string(),
        word if word.ends_with('y') => format!("{}ieth", &word[..word.len() - 1]),
        word => format!("{word}th"),
    };

    format!("{prefix}{ordinal_word}")
}

fn to_full_width_decimal(value: u32) -> String {
    to_decimal_digit_set(value, '０')
}

fn to_decimal_digit_set(value: u32, zero: char) -> String {
    value
        .to_string()
        .chars()
        .map(|ch| {
            // All three digit sets used here are contiguous Unicode blocks.
            // Keeping the zero code point as the parameter makes the mapping
            // shared without hiding which script each numFmt requests.
            char::from_u32(zero as u32 + (ch as u32 - '0' as u32))
                .expect("decimal digit set is valid Unicode")
        })
        .collect()
}

/// Word's half-width `aiueo` alphabet. Small kana and voiced variants are
/// intentionally absent; they are not independent numbering positions.
const AIUEO_KATAKANA_HALF_WIDTH: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜｦﾝ";

const AIUEO_KATAKANA_FULL_WIDTH: &str =
    "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン";

/// Unicode has no half-width code points for the historic `ヰ` and `ヱ` used by
/// iroha. Word-compatible output therefore keeps those two glyphs full-width
/// while all representable positions use half-width Katakana.
const IROHA_KATAKANA_HALF_WIDTH: &str = "ｲﾛﾊﾆﾎﾍﾄﾁﾘﾇﾙｦﾜｶﾖﾀﾚｿﾂﾈﾅﾗﾑｳヰﾉｵｸﾔﾏｹﾌｺｴﾃｱｻｷﾕﾒﾐｼヱﾋﾓｾｽ";

const IROHA_KATAKANA_FULL_WIDTH: &str =
    "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス";

fn to_kana_sequence(value: u32, alphabet: &str) -> String {
    let alphabet: Vec<char> = alphabet.chars().collect();
    debug_assert!(!alphabet.is_empty());

    // Use the same bijective rollover model as Latin letter numbering: after
    // the last one-character label, continue with the first doubled label.
    // This avoids silently falling back to decimal in long generated lists and
    // keeps every counter value representable without a document-specific cap.
    let radix = alphabet.len() as u32;
    let mut remaining = value.max(1);
    let mut result = Vec::new();
    while remaining > 0 {
        remaining -= 1;
        result.push(alphabet[(remaining % radix) as usize]);
        remaining /= radix;
    }
    result.into_iter().rev().collect()
}

fn to_japanese_positional_digits(value: u32) -> String {
    value
        .max(1)
        .to_string()
        .chars()
        .map(|digit| match digit {
            '0' => '〇',
            '1' => '一',
            '2' => '二',
            '3' => '三',
            '4' => '四',
            '5' => '五',
            '6' => '六',
            '7' => '七',
            '8' => '八',
            '9' => '九',
            _ => unreachable!("u32 decimal rendering contains only ASCII digits"),
        })
        .collect()
}

fn to_enclosed_decimal(value: u32, first_glyph: char, limit: u32) -> String {
    if (1..=limit).contains(&value) {
        // Each supported Unicode block is contiguous in numeric order:
        // ①..⑳, ⒈..⒛, and ⑴..⒇. Keeping the start character explicit makes
        // the mapping auditable against the OOXML examples.
        char::from_u32(first_glyph as u32 + value - 1)
            .expect("enclosed decimal range is valid Unicode")
            .to_string()
    } else {
        value.to_string()
    }
}

fn to_enclosed_ideograph(value: u32) -> String {
    if (1..=10).contains(&value) {
        // Unicode assigns the OOXML example sequence contiguously as
        // CIRCLED IDEOGRAPH ONE (㊀) through TEN (㊉).
        char::from_u32('㊀' as u32 + value - 1)
            .expect("circled ideograph range is valid Unicode")
            .to_string()
    } else {
        // ST_NumberFormat allows non-enclosed equivalents after ten. Preserve
        // the ideographic counting system so the fallback remains script- and
        // width-compatible with the requested format.
        to_japanese_counting(value, JapaneseKanjiStyle::Standard)
    }
}

#[derive(Clone, Copy)]
enum JapaneseKanjiStyle {
    Standard,
    Legal,
}

impl JapaneseKanjiStyle {
    fn digits(self) -> &'static [&'static str; 10] {
        match self {
            Self::Standard => &["", "一", "二", "三", "四", "五", "六", "七", "八", "九"],
            // Word's Japanese legal numbering uses the modern financial forms
            // for the digits that need tamper resistance. Four, six, seven,
            // eight, and nine remain the ordinary common-use characters.
            Self::Legal => &["", "壱", "弐", "参", "四", "伍", "六", "七", "八", "九"],
        }
    }

    fn small_units(self) -> &'static [&'static str; 4] {
        match self {
            Self::Standard => &["千", "百", "十", ""],
            // Microsoft/CLDR examples use 拾 for ten while retaining 百 and 千
            // in this numbering style (as opposed to rarer accounting glyphs
            // such as 佰 and 阡).
            Self::Legal => &["千", "百", "拾", ""],
        }
    }

    fn large_units(self) -> &'static [&'static str; 3] {
        match self {
            Self::Standard => &["", "万", "億"],
            Self::Legal => &["", "萬", "億"],
        }
    }
}

fn to_japanese_counting(value: u32, style: JapaneseKanjiStyle) -> String {
    // Japanese groups large values by 10,000 rather than 1,000. Three groups
    // are sufficient for every u32 value: units, 万, and 億 (up to 42億...).
    let mut remaining = value.max(1);
    let mut groups = [0_u32; 3];
    for group in &mut groups {
        *group = remaining % 10_000;
        remaining /= 10_000;
    }

    let mut result = String::new();
    for group_index in (0..groups.len()).rev() {
        let group = groups[group_index];
        if group == 0 {
            continue;
        }
        result.push_str(&format_japanese_four_digit_group(group, style));
        result.push_str(style.large_units()[group_index]);
    }
    result
}

fn format_japanese_four_digit_group(group: u32, style: JapaneseKanjiStyle) -> String {
    debug_assert!(group < 10_000);
    let divisors = [1_000, 100, 10, 1];
    let digits = style.digits();
    let units = style.small_units();
    let mut result = String::new();

    for (index, divisor) in divisors.into_iter().enumerate() {
        let digit = (group / divisor) % 10;
        if digit == 0 {
            continue;
        }

        // Japanese omits the leading one before 千/百/十: 10 is 十, not 一十.
        // The ones position and large-unit groups keep it, yielding 一万 and
        // 壱萬 rather than a bare unit marker.
        if divisor == 1 || digit != 1 {
            result.push_str(digits[digit as usize]);
        }
        result.push_str(units[index]);
    }
    result
}
