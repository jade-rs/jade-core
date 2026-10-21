//! Word field instructions as a whole: the field name, its operands and its
//! switches, and the general switches every field result can carry
//! (ECMA-376 Part 1 §17.16.4.1–3):
//!
//! - `\*` formats — number systems (`Arabic`, `ROMAN`, `alphabetic`,
//!   `CardText`, `OrdText`, `DollarText`, `Hex`, Japanese `KANJINUM1`…,
//!   `DBNUM1`…, `AIUEO`, `IROHA`, `CIRCLENUM`…), capitalization (`Upper`,
//!   `Lower`, `FirstCap`, `Caps`), character width (`DBCHAR`, `SBCHAR`) and
//!   the formatting keepers `MERGEFORMAT` and `CHARFORMAT`;
//! - `\#` numeric pictures (`0`, `#`, `x`, `.`, `,`, `-`, `+`, sections
//!   split by `;`, quoted literals);
//! - `\@` date-time pictures, formatted by [`crate::field_datetime`].
//!
//! The parser keeps what each switch applies to, so an evaluator reads the
//! operands it needs and hands the raw result to [`apply_general_format`].

use crate::document::{tokenize_field_instruction_with_quotes, FieldInstructionToken};
use crate::field_datetime::{format_word_date, FieldDateTime};
use crate::numbering::{format_number_value, to_english_cardinal, to_english_ordinal_text};

/// A field operand: the text, and whether it was written in double quotes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldOperand {
    pub text: String,
    pub quoted: bool,
}

/// A field-specific switch (`\h`, `\r 3`, `\d "Default"`), its letters kept
/// as written, without the backslash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSwitch {
    pub name: String,
    pub value: Option<String>,
}

/// The general switches of a field instruction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeneralFormat {
    /// Every `\*` value in instruction order, as written.
    pub formats: Vec<String>,
    /// `\#`'s picture.
    pub numeric_picture: Option<String>,
    /// `\@`'s picture.
    pub date_picture: Option<String>,
}

impl GeneralFormat {
    fn has(&self, name: &str) -> bool {
        self.formats
            .iter()
            .any(|format| format.eq_ignore_ascii_case(name))
    }

    /// `\* MERGEFORMAT`: an updated result keeps the formatting the previous
    /// result had, run by run.
    pub fn merge_format(&self) -> bool {
        self.has("MERGEFORMAT")
    }

    /// `\* CHARFORMAT`: the result takes the formatting of the field name's
    /// first character in the code.
    pub fn char_format(&self) -> bool {
        self.has("CHARFORMAT")
    }

    pub fn is_empty(&self) -> bool {
        self.formats.is_empty() && self.numeric_picture.is_none() && self.date_picture.is_none()
    }
}

/// A parsed field instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldCode {
    /// The field name, uppercased; `=` for a formula.
    pub name: String,
    /// The operands, in order, switch values excluded.
    pub operands: Vec<FieldOperand>,
    /// The field-specific switches, in order.
    pub switches: Vec<FieldSwitch>,
    pub format: GeneralFormat,
    /// A formula field's expression, as written (`SUM(ABOVE)`).
    pub expression: Option<String>,
}

impl FieldCode {
    /// Parses an instruction (`DATE \@ "yyyy/MM/dd" \* MERGEFORMAT`).
    /// Returns `None` for an empty one.
    pub fn parse(instruction: &str) -> Option<Self> {
        let trimmed = instruction.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix('=') {
            return Some(Self::parse_formula(rest));
        }
        let tokens = tokenize_field_instruction_with_quotes(trimmed);
        let mut tokens = tokens.into_iter().peekable();
        let name = loop {
            let token = tokens.next()?;
            if token.text.is_empty() && !token.quoted {
                continue;
            }
            break token.text.to_ascii_uppercase();
        };
        let mut code = Self {
            name,
            operands: Vec::new(),
            switches: Vec::new(),
            format: GeneralFormat::default(),
            expression: None,
        };
        while let Some(token) = tokens.next() {
            if token.quoted || !token.text.starts_with('\\') {
                code.operands.push(FieldOperand {
                    text: token.text,
                    quoted: token.quoted,
                });
                continue;
            }
            let switch = &token.text[1..];
            // Word accepts a general switch's value glued to it (`\*Upper`).
            let (letters, glued) = match switch.chars().next() {
                Some(first @ ('*' | '#' | '@')) => {
                    let rest = &switch[first.len_utf8()..];
                    (
                        first.to_string(),
                        (!rest.is_empty()).then(|| rest.to_string()),
                    )
                }
                _ => (switch.to_string(), None),
            };
            match letters.as_str() {
                "*" | "#" | "@" => {
                    let value = glued.or_else(|| tokens.next().map(|token| token.text));
                    let Some(value) = value else { continue };
                    match letters.as_str() {
                        "*" => code.format.formats.push(value),
                        "#" => code.format.numeric_picture = Some(value),
                        _ => code.format.date_picture = Some(value),
                    }
                }
                _ => {
                    let takes = switch_takes_value(&code.name, &letters);
                    let next_is_value =
                        tokens.peek().is_some_and(|next: &FieldInstructionToken| {
                            next.quoted || !next.text.starts_with('\\')
                        });
                    let value = match takes {
                        SwitchValue::Always if next_is_value => tokens.next().map(|t| t.text),
                        SwitchValue::IfQuoted if tokens.peek().is_some_and(|next| next.quoted) => {
                            tokens.next().map(|t| t.text)
                        }
                        _ => None,
                    };
                    code.switches.push(FieldSwitch {
                        name: letters,
                        value,
                    });
                }
            }
        }
        Some(code)
    }

    fn parse_formula(rest: &str) -> Self {
        // The expression runs to the first general switch outside quotes.
        let mut end = rest.len();
        let mut in_quotes = false;
        let bytes = rest.as_bytes();
        for (index, &byte) in bytes.iter().enumerate() {
            match byte {
                b'"' => in_quotes = !in_quotes,
                b'\\' if !in_quotes && matches!(bytes.get(index + 1), Some(b'#' | b'*' | b'@')) => {
                    end = index;
                    break;
                }
                _ => {}
            }
        }
        let mut code = Self::parse(&format!("FORMULA {}", &rest[end..])).unwrap_or(Self {
            name: String::new(),
            operands: Vec::new(),
            switches: Vec::new(),
            format: GeneralFormat::default(),
            expression: None,
        });
        code.name = "=".to_string();
        code.operands.clear();
        code.expression = Some(rest[..end].trim().to_string());
        code
    }

    /// The `index`th operand's text.
    pub fn operand(&self, index: usize) -> Option<&str> {
        self.operands
            .get(index)
            .map(|operand| operand.text.as_str())
    }

    /// Whether switch `\name` is present (letters compared case-insensitively).
    pub fn has_switch(&self, name: &str) -> bool {
        self.switches
            .iter()
            .any(|switch| switch.name.eq_ignore_ascii_case(name))
    }

    /// The value of switch `\name`, when it has one.
    pub fn switch_value(&self, name: &str) -> Option<&str> {
        self.switches
            .iter()
            .rev()
            .find(|switch| switch.name.eq_ignore_ascii_case(name))
            .and_then(|switch| switch.value.as_deref())
    }
}

enum SwitchValue {
    Never,
    Always,
    IfQuoted,
}

/// Whether field `field`'s switch `\letters` takes a value, per the field
/// reference (ECMA-376 Part 1 §17.16.5). A switch the table does not know
/// takes the next operand only when it is quoted.
fn switch_takes_value(field: &str, letters: &str) -> SwitchValue {
    let letter = letters.to_ascii_lowercase();
    let with: &[&str] = match field {
        "SEQ" => &["r", "s"],
        "LISTNUM" => &["l", "s"],
        "AUTONUM" | "AUTONUMLGL" | "AUTONUMOUT" => &["s"],
        "REF" => &["d"],
        "FILLIN" | "ASK" => &["d"],
        "HYPERLINK" => &["l", "o", "t"],
        "INCLUDETEXT" => &["c"],
        "INCLUDEPICTURE" => &["c"],
        "SYMBOL" => &["f", "s"],
        "TOC" => &["a", "b", "c", "d", "f", "l", "n", "o", "p", "s", "t"],
        "INDEX" => &["b", "c", "d", "e", "f", "g", "h", "k", "l", "p", "s"],
        "TOA" => &["b", "c", "d", "e", "g", "l", "s"],
        "TA" => &["c", "l", "s"],
        "TC" => &["f", "l"],
        "XE" => &["f", "r", "t"],
        "MERGEFIELD" => &["b", "f"],
        "ADVANCE" => &["d", "l", "r", "u", "x", "y"],
        "BIBLIOGRAPHY" => &["l", "f", "m"],
        "CITATION" => &["l", "f", "s", "v", "p", "m"],
        "AUTOTEXTLIST" => &["s", "t"],
        "DATE" | "TIME" | "CREATEDATE" | "SAVEDATE" | "PRINTDATE" | "PAGE" | "NUMPAGES"
        | "SECTIONPAGES" | "SECTION" | "NOTEREF" | "PAGEREF" | "STYLEREF" | "FILENAME"
        | "FILESIZE" | "TEMPLATE" | "NUMWORDS" | "NUMCHARS" | "EDITTIME" | "REVNUM" => {
            return SwitchValue::Never
        }
        _ => return SwitchValue::IfQuoted,
    };
    if with.contains(&letter.as_str()) {
        SwitchValue::Always
    } else {
        SwitchValue::Never
    }
}

/// A field's raw result, before the general switches.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Text(String),
    Number(f64),
    Date(FieldDateTime),
}

impl FieldValue {
    /// The number a text result reads as (`1,234.5`, `-3`, `¥1,000`), when
    /// it is one.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            FieldValue::Number(value) => Some(*value),
            FieldValue::Text(text) => parse_field_number(text),
            FieldValue::Date(_) => None,
        }
    }
}

/// The number a field result's text holds: grouping commas and a leading
/// currency sign are ignored, `(5)` is negative, and a trailing `%` stays out.
pub fn parse_field_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let (negative, body) = match trimmed
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(inner) => (true, inner.trim()),
        None => (false, trimmed),
    };
    let body = body.trim_start_matches(['$', '¥', '￥', '€', '£']).trim();
    let body = body.trim_end_matches('%').trim();
    let cleaned: String = body.chars().filter(|&c| c != ',').collect();
    if cleaned.is_empty() || !cleaned.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: f64 = cleaned.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// Formats `value` the way an unformatted numeric field result reads: whole
/// numbers without decimals, others to Word's default precision of up to
/// two places, trailing zeros dropped.
pub fn format_plain_number(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let rounded = (value * 100.0).round() / 100.0;
    if rounded == rounded.trunc() {
        return format!("{}", rounded as i64);
    }
    let text = format!("{rounded:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The `ST_NumberFormat` a `\*` number format names, with the case of the
/// switch deciding the case of letters and numerals (`ROMAN` against
/// `roman`).
fn number_system(name: &str) -> Option<&'static str> {
    let upper_initial = name.chars().next().is_some_and(char::is_uppercase);
    Some(match name.to_ascii_uppercase().as_str() {
        "ARABIC" => "decimal",
        "ARABICDASH" => "numberInDash",
        "ALPHABETIC" if upper_initial => "upperLetter",
        "ALPHABETIC" => "lowerLetter",
        "ROMAN" if upper_initial => "upperRoman",
        "ROMAN" => "lowerRoman",
        "ORDINAL" => "ordinal",
        "CARDTEXT" => "cardinalText",
        "ORDTEXT" => "ordinalText",
        "HEX" => "hex",
        "CIRCLENUM" => "decimalEnclosedCircle",
        "KANJINUM1" | "DBNUM1" => "japaneseCounting",
        "KANJINUM2" | "DBNUM2" => "japaneseLegal",
        "KANJINUM3" | "DBNUM4" => "japaneseDigitalTenThousand",
        "DBNUM3" => "decimalFullWidth",
        "AIUEO" => "aiueoFullWidth",
        "IROHA" => "irohaFullWidth",
        "ZODIAC1" => "ideographTraditional",
        "ZODIAC2" => "ideographZodiac",
        "ZODIAC3" => "ideographZodiacTraditional",
        "GB1" => "decimalEnclosedFullstop",
        "GB2" => "decimalEnclosedParen",
        "GB3" => "decimalEnclosedCircleChinese",
        "GB4" => "ideographEnclosedCircle",
        "CHOSUNG" => "chosung",
        "GANADA" => "ganada",
        "HEBREW1" => "hebrew1",
        "HEBREW2" => "hebrew2",
        "ARABICABJAD" => "arabicAbjad",
        "ARABICALPHA" => "arabicAlpha",
        "HINDIARABIC" => "hindiNumbers",
        "THAIARABIC" => "thaiNumbers",
        _ => return None,
    })
}

/// Formats `value` in the number system a `\*` switch names (`roman`,
/// `CardText`, `KANJINUM1`…), or `None` when `name` names none.
pub fn format_number_switch(value: f64, name: &str) -> Option<String> {
    if name.eq_ignore_ascii_case("DOLLARTEXT") {
        return Some(dollar_text(value));
    }
    let system = number_system(name)?;
    let rounded = value.round();
    if rounded < 0.0 || rounded > u32::MAX as f64 {
        return Some(format_plain_number(value));
    }
    // Word writes ordinal and cardinal text in English whatever the
    // result's language, as Word for Mac does for a Japanese document.
    Some(format_number_value(system, rounded as u32, Some("en-US")))
}

/// `DollarText`: the whole part in words, the cents as a fraction
/// (`one hundred twenty-three and 45/100`).
fn dollar_text(value: f64) -> String {
    let cents_total = (value.abs() * 100.0).round() as u64;
    let whole = (cents_total / 100) as u32;
    let cents = cents_total % 100;
    let words = if whole == 0 {
        "zero".to_string()
    } else {
        to_english_cardinal(whole)
    };
    format!("{words} and {cents:02}/100")
}

/// The capitalization a `\*` switch asks for.
fn apply_case(text: &str, name: &str) -> Option<String> {
    Some(match name.to_ascii_uppercase().as_str() {
        "UPPER" => text.to_uppercase(),
        "LOWER" => text.to_lowercase(),
        "FIRSTCAP" => {
            let mut done = false;
            text.chars()
                .flat_map(|character| {
                    if !done && character.is_alphabetic() {
                        done = true;
                        character.to_uppercase().collect::<Vec<_>>()
                    } else {
                        vec![character]
                    }
                })
                .collect()
        }
        "CAPS" => {
            let mut starts_word = true;
            text.chars()
                .flat_map(|character| {
                    let mapped = if character.is_alphanumeric() && starts_word {
                        character.to_uppercase().collect::<Vec<_>>()
                    } else {
                        vec![character]
                    };
                    starts_word = !character.is_alphanumeric();
                    mapped
                })
                .collect()
        }
        "DBCHAR" => to_full_width(text),
        "SBCHAR" => to_half_width(text),
        _ => return None,
    })
}

/// Full-width forms of ASCII and half-width katakana (`\* DBCHAR`).
pub fn to_full_width(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        let next = chars.get(index + 1).copied();
        if character == ' ' {
            out.push('\u{3000}');
        } else if ('!'..='~').contains(&character) {
            out.push(char::from_u32(character as u32 - 0x21 + 0xFF01).unwrap_or(character));
        } else if let Some(full) = half_width_katakana_to_full(character, next) {
            out.push(full.0);
            if full.1 {
                index += 1;
            }
        } else {
            out.push(character);
        }
        index += 1;
    }
    out
}

/// Half-width forms of full-width ASCII and katakana (`\* SBCHAR`).
pub fn to_half_width(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\u{3000}' {
            out.push(' ');
        } else if ('\u{FF01}'..='\u{FF5E}').contains(&character) {
            out.push(char::from_u32(character as u32 - 0xFF01 + 0x21).unwrap_or(character));
        } else if let Some(half) = full_width_katakana_to_half(character) {
            out.push_str(half);
        } else {
            out.push(character);
        }
    }
    out
}

const HALF_KATAKANA: &str = "｡｢｣､･ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝﾞﾟ";
const FULL_KATAKANA: &str = "。「」、・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン゛゜";

/// The full-width katakana a half-width one (and a following voicing mark)
/// stands for; the flag says whether the mark was consumed.
fn half_width_katakana_to_full(character: char, next: Option<char>) -> Option<(char, bool)> {
    let index = HALF_KATAKANA.chars().position(|c| c == character)?;
    let full = FULL_KATAKANA.chars().nth(index)?;
    match next {
        Some('ﾞ') => {
            if let Some(voiced) = char::from_u32(full as u32 + 1)
                .filter(|_| "カキクケコサシスセソタチツテトハヒフヘホ".contains(full))
            {
                return Some((voiced, true));
            }
            if full == 'ウ' {
                return Some(('ヴ', true));
            }
        }
        Some('ﾟ') if "ハヒフヘホ".contains(full) => {
            return char::from_u32(full as u32 + 2).map(|semi| (semi, true));
        }
        _ => {}
    }
    Some((full, false))
}

fn full_width_katakana_to_half(character: char) -> Option<&'static str> {
    const PAIRS: &[(char, &str)] = &[
        ('ガ', "ｶﾞ"),
        ('ギ', "ｷﾞ"),
        ('グ', "ｸﾞ"),
        ('ゲ', "ｹﾞ"),
        ('ゴ', "ｺﾞ"),
        ('ザ', "ｻﾞ"),
        ('ジ', "ｼﾞ"),
        ('ズ', "ｽﾞ"),
        ('ゼ', "ｾﾞ"),
        ('ゾ', "ｿﾞ"),
        ('ダ', "ﾀﾞ"),
        ('ヂ', "ﾁﾞ"),
        ('ヅ', "ﾂﾞ"),
        ('デ', "ﾃﾞ"),
        ('ド', "ﾄﾞ"),
        ('バ', "ﾊﾞ"),
        ('ビ', "ﾋﾞ"),
        ('ブ', "ﾌﾞ"),
        ('ベ', "ﾍﾞ"),
        ('ボ', "ﾎﾞ"),
        ('パ', "ﾊﾟ"),
        ('ピ', "ﾋﾟ"),
        ('プ', "ﾌﾟ"),
        ('ペ', "ﾍﾟ"),
        ('ポ', "ﾎﾟ"),
        ('ヴ', "ｳﾞ"),
    ];
    if let Some((_, half)) = PAIRS.iter().find(|(full, _)| *full == character) {
        return Some(half);
    }
    let index = FULL_KATAKANA.chars().position(|c| c == character)?;
    let start = HALF_KATAKANA.char_indices().nth(index)?.0;
    let end = HALF_KATAKANA
        .char_indices()
        .nth(index + 1)
        .map(|(offset, _)| offset)
        .unwrap_or(HALF_KATAKANA.len());
    Some(&HALF_KATAKANA[start..end])
}

/// Applies a field's general switches to its raw result: the `\@` picture to
/// a date, the `\#` picture or a `\*` number system to a number, then the
/// `\*` capitalization and width switches to the text.
pub fn apply_general_format(value: &FieldValue, format: &GeneralFormat) -> String {
    let mut text = match value {
        FieldValue::Date(date) => {
            format_word_date(format.date_picture.as_deref().unwrap_or("yyyy/MM/dd"), date)
        }
        FieldValue::Number(number) => format_number_result(*number, format),
        FieldValue::Text(text) => match (text.trim().is_empty(), value.as_number()) {
            (false, Some(number))
                if format.numeric_picture.is_some()
                    || format.formats.iter().any(|name| {
                        number_system(name).is_some() || name.eq_ignore_ascii_case("DOLLARTEXT")
                    }) =>
            {
                format_number_result(number, format)
            }
            _ => text.clone(),
        },
    };
    for name in &format.formats {
        if let Some(cased) = apply_case(&text, name) {
            text = cased;
        }
    }
    text
}

fn format_number_result(number: f64, format: &GeneralFormat) -> String {
    if let Some(picture) = format.numeric_picture.as_deref() {
        return format_numeric_picture(number, picture);
    }
    format
        .formats
        .iter()
        .rev()
        .find_map(|name| format_number_switch(number, name))
        .unwrap_or_else(|| format_plain_number(number))
}

#[derive(Clone, Debug, PartialEq)]
enum PictureToken {
    /// `0`, `#` or `x`.
    Digit(char),
    Decimal,
    Group,
    Minus,
    Plus,
    Literal(String),
}

fn tokenize_picture(picture: &str) -> Vec<PictureToken> {
    let mut tokens = Vec::new();
    let mut chars = picture.chars().peekable();
    while let Some(character) = chars.next() {
        tokens.push(match character {
            '0' | '#' | 'x' => PictureToken::Digit(character),
            '.' => PictureToken::Decimal,
            ',' => PictureToken::Group,
            '-' => PictureToken::Minus,
            '+' => PictureToken::Plus,
            '\'' => {
                let mut literal = String::new();
                for next in chars.by_ref() {
                    if next == '\'' {
                        break;
                    }
                    literal.push(next);
                }
                PictureToken::Literal(literal)
            }
            other => PictureToken::Literal(other.to_string()),
        });
    }
    tokens
}

/// Splits a picture into its `;` sections, leaving quoted text alone.
fn picture_sections(picture: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut in_quote = false;
    for character in picture.chars() {
        if character == '\'' {
            in_quote = !in_quote;
        }
        if character == ';' && !in_quote {
            sections.push(String::new());
        } else {
            sections.last_mut().expect("one section").push(character);
        }
    }
    sections
}

/// Formats `value` with a `\#` numeric picture (ECMA-376 Part 1
/// §17.16.4.2): `0` shows a digit or zero, `#` a digit or a space, `x`
/// drops the digits to its left (or rounds at its place after the decimal
/// point), `,` groups thousands, `-`/`+` place the sign, `'text'` and other
/// characters are literal, and `;` separates the positive, negative and zero
/// sections.
pub fn format_numeric_picture(value: f64, picture: &str) -> String {
    let sections = picture_sections(picture);
    let negative = value < 0.0;
    let (section, explicit_negative) = if negative && sections.len() >= 2 {
        (&sections[1], true)
    } else {
        (&sections[0], false)
    };
    let tokens = tokenize_picture(section);
    let decimal_at = tokens
        .iter()
        .position(|token| *token == PictureToken::Decimal);
    let fraction_places = decimal_at.map_or(0, |at| {
        tokens[at + 1..]
            .iter()
            .filter(|token| matches!(token, PictureToken::Digit(_)))
            .count()
    });
    let magnitude = value.abs();
    let scale = 10f64.powi(fraction_places as i32);
    let scaled = (magnitude * scale + 1e-9).round();
    if scaled == 0.0 && sections.len() >= 3 && !sections[2].is_empty() {
        return format_numeric_picture_section(0.0, &tokenize_picture(&sections[2]), false, 0);
    }
    let rounded = scaled / scale;
    format_numeric_picture_section(
        rounded,
        &tokens,
        negative && !explicit_negative && scaled != 0.0,
        fraction_places,
    )
}

fn format_numeric_picture_section(
    magnitude: f64,
    tokens: &[PictureToken],
    negative: bool,
    fraction_places: usize,
) -> String {
    let decimal_at = tokens
        .iter()
        .position(|token| *token == PictureToken::Decimal);
    let integer_tokens = &tokens[..decimal_at.unwrap_or(tokens.len())];
    let fraction_tokens = decimal_at.map_or(&[][..], |at| &tokens[at + 1..]);
    let has_sign_placeholder = tokens
        .iter()
        .any(|token| matches!(token, PictureToken::Minus | PictureToken::Plus));

    let whole = magnitude.trunc();
    let mut digits = format!("{whole:.0}");
    let integer_slots: Vec<char> = integer_tokens
        .iter()
        .filter_map(|token| match token {
            PictureToken::Digit(slot) => Some(*slot),
            _ => None,
        })
        .collect();
    if digits == "0" && !integer_slots.contains(&'0') {
        digits.clear();
    }
    // `x` drops every digit to the left of it.
    if let Some(x_at) = integer_slots.iter().position(|&slot| slot == 'x') {
        let keep = integer_slots.len() - x_at;
        if digits.len() > keep {
            digits = digits[digits.len() - keep..].to_string();
        }
    }
    let grouping = integer_tokens.contains(&PictureToken::Group);
    let mut integer = String::new();
    {
        // Fill the slots right to left; extra digits go before the first.
        let mut remaining: Vec<char> = digits.chars().collect();
        let mut filled: Vec<char> = Vec::new();
        for &slot in integer_slots.iter().rev() {
            match remaining.pop() {
                Some(digit) => filled.push(digit),
                None => filled.push(match slot {
                    '0' => '0',
                    _ => ' ',
                }),
            }
        }
        while let Some(digit) = remaining.pop() {
            filled.push(digit);
        }
        filled.reverse();
        if grouping {
            let len = filled.len();
            for (index, character) in filled.iter().enumerate() {
                let from_right = len - index;
                if index > 0 && from_right.is_multiple_of(3) {
                    let previous = filled[index - 1];
                    integer.push(if previous == ' ' { ' ' } else { ',' });
                }
                integer.push(*character);
            }
        } else {
            integer = filled.into_iter().collect();
        }
    }

    let mut fraction = String::new();
    if fraction_places > 0 {
        let scale = 10f64.powi(fraction_places as i32);
        let fraction_value = ((magnitude - whole) * scale).round() as u64;
        let fraction_digits = format!("{fraction_value:0width$}", width = fraction_places);
        let slots: Vec<char> = fraction_tokens
            .iter()
            .filter_map(|token| match token {
                PictureToken::Digit(slot) => Some(*slot),
                _ => None,
            })
            .collect();
        // Trailing zeros under `#` read as spaces.
        let mut trailing = true;
        let mut chars: Vec<char> = Vec::new();
        let fraction_chars: Vec<char> = fraction_digits.chars().collect();
        for (slot, digit) in slots.iter().copied().zip(fraction_chars).rev() {
            if trailing && digit == '0' && slot == '#' {
                chars.push(' ');
            } else {
                trailing = false;
                chars.push(digit);
            }
        }
        chars.reverse();
        fraction = chars.into_iter().collect();
    }

    let mut out = String::new();
    if negative && !has_sign_placeholder {
        out.push('-');
    }
    let mut integer_written = false;
    let mut fraction_written = false;
    let mut seen_decimal = false;
    for token in tokens {
        match token {
            PictureToken::Digit(_) | PictureToken::Group => {
                if seen_decimal {
                    if !fraction_written {
                        out.push_str(&fraction);
                        fraction_written = true;
                    }
                } else if !integer_written {
                    out.push_str(&integer);
                    integer_written = true;
                }
            }
            PictureToken::Decimal => {
                if !integer_written {
                    out.push_str(&integer);
                    integer_written = true;
                }
                seen_decimal = true;
                out.push('.');
            }
            PictureToken::Minus => out.push(if negative { '-' } else { ' ' }),
            PictureToken::Plus => out.push(if negative {
                '-'
            } else if magnitude == 0.0 {
                ' '
            } else {
                '+'
            }),
            PictureToken::Literal(text) => out.push_str(text),
        }
    }
    if !integer_written && !integer.is_empty() {
        out.push_str(&integer);
    }
    out
}

/// English ordinal text, for the `\* OrdText` of a formula result.
pub fn ordinal_text(value: u32) -> String {
    to_english_ordinal_text(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(value: f64, picture: &str) -> String {
        format_numeric_picture(value, picture)
    }

    #[test]
    fn parses_operands_switches_and_general_formats() {
        let code = FieldCode::parse(r#" SEQ Figure \r 3 \* ROMAN \* MERGEFORMAT "#).unwrap();
        assert_eq!(code.name, "SEQ");
        assert_eq!(code.operand(0), Some("Figure"));
        assert_eq!(code.switch_value("r"), Some("3"));
        assert_eq!(code.format.formats, vec!["ROMAN", "MERGEFORMAT"]);
        assert!(code.format.merge_format());

        let code = FieldCode::parse(r#"DATE \@ "ggge年M月d日" \*Upper"#).unwrap();
        assert_eq!(code.format.date_picture.as_deref(), Some("ggge年M月d日"));
        assert_eq!(code.format.formats, vec!["Upper"]);

        let code = FieldCode::parse(r#"FILLIN "Your name?" \d "Anon" \o"#).unwrap();
        assert_eq!(code.operand(0), Some("Your name?"));
        assert_eq!(code.switch_value("d"), Some("Anon"));
        assert!(code.has_switch("o"));

        let code = FieldCode::parse(r##"= SUM(ABOVE) * 1.1 \# "#,##0.00""##).unwrap();
        assert_eq!(code.name, "=");
        assert_eq!(code.expression.as_deref(), Some("SUM(ABOVE) * 1.1"));
        assert_eq!(code.format.numeric_picture.as_deref(), Some("#,##0.00"));
        assert!(FieldCode::parse("   ").is_none());
    }

    #[test]
    fn numeric_pictures_follow_words_placeholders() {
        assert_eq!(picture(15.0, "$###"), "$ 15");
        assert_eq!(picture(1234567.891, "#,##0.00"), "1,234,567.89");
        assert_eq!(picture(5.0, "0.00"), "5.00");
        assert_eq!(picture(0.125, "0.00x"), "0.125");
        assert_eq!(picture(0.75, ".x"), ".8");
        assert_eq!(picture(222492.0, "x##"), "492");
        assert_eq!(picture(-5.0, "0.00"), "-5.00");
        assert_eq!(picture(-5.0, "0.00;(0.00)"), "(5.00)");
        assert_eq!(picture(0.0, "0.00;(0.00);'zero'"), "zero");
        assert_eq!(picture(5.0, "+0"), "+5");
        assert_eq!(picture(-5.0, "+0"), "-5");
        assert_eq!(picture(5.0, "-0"), " 5");
        assert_eq!(picture(1500.0, "¥#,##0"), "¥1,500");
        assert_eq!(picture(2.5, "0.##"), "2.5 ");
        assert_eq!(picture(0.5, "0"), "1");
        assert_eq!(picture(42.0, "00000"), "00042");
        assert_eq!(picture(3.0, "0' 個'"), "3 個");
    }

    #[test]
    fn number_systems_follow_the_switch_spelling() {
        assert_eq!(format_number_switch(14.0, "ROMAN").unwrap(), "XIV");
        assert_eq!(format_number_switch(14.0, "roman").unwrap(), "xiv");
        assert_eq!(format_number_switch(28.0, "ALPHABETIC").unwrap(), "BB");
        assert_eq!(format_number_switch(28.0, "alphabetic").unwrap(), "bb");
        assert_eq!(
            format_number_switch(21.0, "CardText").unwrap(),
            "twenty-one"
        );
        assert_eq!(
            format_number_switch(21.0, "OrdText").unwrap(),
            "twenty-first"
        );
        assert_eq!(format_number_switch(21.0, "Ordinal").unwrap(), "21st");
        assert_eq!(format_number_switch(255.0, "Hex").unwrap(), "FF");
        assert_eq!(
            format_number_switch(123.45, "DollarText").unwrap(),
            "one hundred twenty-three and 45/100"
        );
        assert_eq!(
            format_number_switch(125.0, "KANJINUM1").unwrap(),
            "百二十五"
        );
        assert_eq!(
            format_number_switch(125.0, "KANJINUM2").unwrap(),
            "百弐拾伍"
        );
        assert_eq!(format_number_switch(105.0, "KANJINUM3").unwrap(), "一〇五");
        assert_eq!(format_number_switch(12.0, "DBNUM3").unwrap(), "１２");
        assert_eq!(format_number_switch(3.0, "CIRCLENUM").unwrap(), "③");
        assert_eq!(format_number_switch(2.0, "AIUEO").unwrap(), "イ");
        assert_eq!(format_number_switch(2.0, "IROHA").unwrap(), "ロ");
        assert_eq!(format_number_switch(1.0, "ZODIAC1").unwrap(), "甲");
        assert_eq!(format_number_switch(7.0, "ArabicDash").unwrap(), "- 7 -");
        assert!(format_number_switch(1.0, "MERGEFORMAT").is_none());
    }

    #[test]
    fn general_format_applies_pictures_then_case_and_width() {
        let format = |code: &str| FieldCode::parse(code).unwrap().format;
        assert_eq!(
            apply_general_format(
                &FieldValue::Text("hello world".into()),
                &format(r"X \* Caps")
            ),
            "Hello World"
        );
        assert_eq!(
            apply_general_format(
                &FieldValue::Text("hello world".into()),
                &format(r"X \* FirstCap")
            ),
            "Hello world"
        );
        assert_eq!(
            apply_general_format(&FieldValue::Text("Mixed".into()), &format(r"X \* Upper")),
            "MIXED"
        );
        assert_eq!(
            apply_general_format(&FieldValue::Number(9.0), &format(r"X \* roman \* Upper")),
            "IX"
        );
        assert_eq!(
            apply_general_format(&FieldValue::Text("1,234".into()), &format(r#"X \# "0.0""#)),
            "1234.0"
        );
        assert_eq!(
            apply_general_format(&FieldValue::Text("ABC 123".into()), &format(r"X \* DBCHAR")),
            "ＡＢＣ　１２３"
        );
        assert_eq!(
            apply_general_format(
                &FieldValue::Text("ＡＢＣ　ガギ".into()),
                &format(r"X \* SBCHAR")
            ),
            "ABC ｶﾞｷﾞ"
        );
        assert_eq!(
            apply_general_format(&FieldValue::Text("ｶﾞｷﾞﾊﾟ".into()), &format(r"X \* DBCHAR")),
            "ガギパ"
        );
        assert_eq!(
            apply_general_format(&FieldValue::Number(1.0 / 3.0), &GeneralFormat::default()),
            "0.33"
        );
        assert_eq!(
            apply_general_format(&FieldValue::Number(12.0), &GeneralFormat::default()),
            "12"
        );
        let date = FieldDateTime {
            year: 2026,
            month: 10,
            day: 21,
            hour: 9,
            minute: 41,
            second: 0,
            weekday: 3,
        };
        assert_eq!(
            apply_general_format(
                &FieldValue::Date(date),
                &format(r#"X \@ "MMMM d" \* Upper"#)
            ),
            "OCTOBER 21"
        );
    }

    #[test]
    fn reads_numbers_out_of_text_results() {
        assert_eq!(parse_field_number("1,234.5"), Some(1234.5));
        assert_eq!(parse_field_number("(12)"), Some(-12.0));
        assert_eq!(parse_field_number("$5"), Some(5.0));
        assert_eq!(parse_field_number("abc"), None);
        assert_eq!(format_plain_number(2.5), "2.5");
        assert_eq!(format_plain_number(-4.0), "-4");
    }
}
