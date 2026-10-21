//! What a character in a math run is drawn as and how it spaces.
//!
//! Word draws the letters of a math run from the Mathematical Alphanumeric
//! Symbols block: an `x` typed in an equation is U+1D465, italic by default,
//! and `m:sty` / `m:scr` pick the other alphabets (bold, script, fraktur,
//! double-struck, sans-serif, monospace). Operators keep their code points and
//! are spaced by their class, as TeX spaces its atoms.

use super::MathVariant;

/// `m:scr`: the alphabet a run's letters are drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathScript {
    #[default]
    Roman,
    Script,
    Fraktur,
    DoubleStruck,
    SansSerif,
    Monospace,
}

impl MathScript {
    pub fn from_omml(value: &str) -> Self {
        match value {
            "script" => Self::Script,
            "fraktur" => Self::Fraktur,
            "double-struck" => Self::DoubleStruck,
            "sans-serif" => Self::SansSerif,
            "monospace" => Self::Monospace,
            _ => Self::Roman,
        }
    }
}

/// The character a run shows for `ch` in `variant` and `script`.
pub fn math_alphanumeric(ch: char, variant: MathVariant, script: MathScript) -> char {
    use MathScript::*;
    use MathVariant::*;
    let bold = matches!(variant, Bold | BoldItalic);
    let italic = matches!(variant, Italic | BoldItalic);
    let upper = ch.is_ascii_uppercase();
    let lower = ch.is_ascii_lowercase();
    let digit = ch.is_ascii_digit();
    let latin = |capital: u32, small: u32| -> Option<char> {
        let base = if upper { capital } else { small };
        let first = if upper { 'A' } else { 'a' };
        char::from_u32(base + ch as u32 - first as u32)
    };
    if upper || lower {
        let mapped = match (script, bold, italic) {
            (Roman, false, false) => None,
            (Roman, true, false) => latin(0x1D400, 0x1D41A),
            (Roman, false, true) => {
                if ch == 'h' {
                    return 'ℎ';
                }
                latin(0x1D434, 0x1D44E)
            }
            (Roman, true, true) => latin(0x1D468, 0x1D482),
            (Script, false, _) => {
                if let Some(hole) = script_hole(ch) {
                    return hole;
                }
                latin(0x1D49C, 0x1D4B6)
            }
            (Script, true, _) => latin(0x1D4D0, 0x1D4EA),
            (Fraktur, false, _) => {
                if let Some(hole) = fraktur_hole(ch) {
                    return hole;
                }
                latin(0x1D504, 0x1D51E)
            }
            (Fraktur, true, _) => latin(0x1D56C, 0x1D586),
            (DoubleStruck, _, _) => {
                if let Some(hole) = double_struck_hole(ch) {
                    return hole;
                }
                latin(0x1D538, 0x1D552)
            }
            (SansSerif, false, false) => latin(0x1D5A0, 0x1D5BA),
            (SansSerif, true, false) => latin(0x1D5D4, 0x1D5EE),
            (SansSerif, false, true) => latin(0x1D608, 0x1D622),
            (SansSerif, true, true) => latin(0x1D63C, 0x1D656),
            (Monospace, _, _) => latin(0x1D670, 0x1D68A),
        };
        return mapped.unwrap_or(ch);
    }
    if digit {
        let base = match (script, bold) {
            (DoubleStruck, _) => 0x1D7D8,
            (SansSerif, false) => 0x1D7E2,
            (SansSerif, true) => 0x1D7EC,
            (Monospace, _) => 0x1D7F6,
            (_, true) => 0x1D7CE,
            _ => return ch,
        };
        return char::from_u32(base + ch as u32 - '0' as u32).unwrap_or(ch);
    }
    if let Some(index) = greek_index(ch) {
        // Word keeps capital Greek upright and slants the small letters, as
        // mathematical typesetting does.
        let capital = index < 26;
        let block = match (script, bold, italic && !capital) {
            (SansSerif, true, false) => 0x1D756,
            (SansSerif, true, true) => 0x1D790,
            (_, true, false) => 0x1D6A8,
            (_, true, true) => 0x1D71C,
            (_, false, true) => 0x1D6E2,
            _ => return ch,
        };
        return char::from_u32(block + index).unwrap_or(ch);
    }
    ch
}

/// The position of a Greek letter in the math alphabets' Greek run: the 25
/// capitals with ϴ, ∇, then the 25 small letters with ς, ∂ and the variants.
fn greek_index(ch: char) -> Option<u32> {
    let code = ch as u32;
    match ch {
        'Α'..='Ω' if code != 0x3A2 => Some(code - 0x391),
        'ϴ' => Some(17),
        'α'..='ω' => Some(26 + code - 0x3B1),
        'ϵ' => Some(52),
        'ϑ' => Some(53),
        'ϰ' => Some(54),
        'ϕ' => Some(55),
        'ϱ' => Some(56),
        'ϖ' => Some(57),
        _ => None,
    }
}

fn script_hole(ch: char) -> Option<char> {
    Some(match ch {
        'B' => 'ℬ',
        'E' => 'ℰ',
        'F' => 'ℱ',
        'H' => 'ℋ',
        'I' => 'ℐ',
        'L' => 'ℒ',
        'M' => 'ℳ',
        'R' => 'ℛ',
        'e' => 'ℯ',
        'g' => 'ℊ',
        'o' => 'ℴ',
        _ => return None,
    })
}

fn fraktur_hole(ch: char) -> Option<char> {
    Some(match ch {
        'C' => 'ℭ',
        'H' => 'ℌ',
        'I' => 'ℑ',
        'R' => 'ℜ',
        'Z' => 'ℨ',
        _ => return None,
    })
}

fn double_struck_hole(ch: char) -> Option<char> {
    Some(match ch {
        'C' => 'ℂ',
        'H' => 'ℍ',
        'N' => 'ℕ',
        'P' => 'ℙ',
        'Q' => 'ℚ',
        'R' => 'ℝ',
        'Z' => 'ℤ',
        _ => return None,
    })
}

/// How an atom spaces against its neighbours (TeX's atom types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomClass {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// The class of one character of a math run.
pub fn char_class(ch: char) -> AtomClass {
    use AtomClass::*;
    match ch {
        '+' | '-' | '−' | '±' | '∓' | '×' | '÷' | '·' | '∙' | '∘' | '∗' | '*' | '⋅' | '∪' | '∩'
        | '∧' | '∨' | '⊕' | '⊖' | '⊗' | '⊘' | '⊙' | '∖' | '⋆' | '⋄' | '⊎' | '⊓' | '⊔' | '†'
        | '‡' | '≀' | '⊞' | '⊟' | '⊠' | '⊡' | '⋉' | '⋊' | '⋋' | '⋌' | '⋏' | '⋎' | '∔' | '⨿'
        | '⊼' | '⊻' | '⊽' | '⨁' | '⨂' => Bin,
        '=' | '<' | '>' | '≤' | '≥' | '≠' | '≈' | '≡' | '≅' | '∼' | '≃' | '≪' | '≫' | '∝' | '∈'
        | '∉' | '∋' | '∌' | '⊂' | '⊃' | '⊆' | '⊇' | '⊄' | '⊅' | '⊊' | '⊋' | '∣' | '∤' | '∥'
        | '∦' | '⊥' | '≺' | '≻' | '⪯' | '⪰' | '≼' | '≽' | '≐' | '≜' | '≝' | '≔' | '≕' | ':'
        | '⊢' | '⊣' | '⊨' | '⊩' | '∴' | '∵' | '≲' | '≳' | '≶' | '≷' | '≍' | '≑' | '≒' | '≓'
        | '≗' | '≙' | '≚' | '≞' | '≟' | '≢' | '≣' | '≦' | '≧' | '≨' | '≩' | '≮' | '≯' | '≰'
        | '≱' | '⊏' | '⊐' | '⊑' | '⊒' | '⋈' | '⊲' | '⊳' | '⊴' | '⊵' | '∷' | '∺' | '∻' | '∽'
        | '≁' | '≄' | '≆' | '≇' | '≉' | '⩽' | '⩾' | '≬' | '⋘' | '⋙' | '⋐' | '⋑' | '⊪' | '⊫'
        | '⟂' | '⫽' => Rel,
        '\u{2190}'..='\u{21FF}' | '\u{27F0}'..='\u{27FF}' | '\u{2900}'..='\u{297F}' => Rel,
        '(' | '[' | '{' | '⟨' | '⌈' | '⌊' | '⟦' | '⟮' | '⦃' | '〈' | '⟪' => Open,
        ')' | ']' | '}' | '⟩' | '⌉' | '⌋' | '⟧' | '⟯' | '⦄' | '〉' | '⟫' | '!' => {
            Close
        }
        ',' | ';' => Punct,
        '∑' | '∏' | '∐' | '∫' | '∬' | '∭' | '∮' | '∯' | '∰' | '⋀' | '⋁' | '⋂' | '⋃' | '⨀' | '⨄'
        | '⨅' | '⨆' | '∱' | '∲' | '∳' | '⨌' => Op,
        _ => Ord,
    }
}

/// The space TeX puts between a left and a right atom, in mu (1/18 em).
/// Medium and thick spaces vanish in script styles.
pub fn atom_space(left: AtomClass, right: AtomClass, script: bool) -> f64 {
    use AtomClass::*;
    const THIN: f64 = 3.0;
    const MEDIUM: f64 = 4.0;
    const THICK: f64 = 5.0;
    let (space, in_script) = match (left, right) {
        (Ord, Op) | (Op, Ord) | (Op, Op) | (Close, Op) | (Inner, Op) => (THIN, true),
        (Ord, Bin) | (Bin, _) | (_, Bin) => (MEDIUM, false),
        (Ord, Rel) | (Op, Rel) | (Close, Rel) | (Inner, Rel) => (THICK, false),
        (Rel, Ord) | (Rel, Op) | (Rel, Open) | (Rel, Inner) => (THICK, false),
        (Ord, Inner) | (Op, Inner) | (Close, Inner) | (Inner, Ord) | (Inner, Open) => (THIN, false),
        (Inner, Punct) | (Inner, Inner) => (THIN, false),
        (Punct, Rel) | (Punct, _) => (THIN, false),
        _ => (0.0, true),
    };
    if script && !in_script {
        0.0
    } else {
        space
    }
}

/// A binary operator that cannot be one where it stands (first in its
/// argument, after an operator, a relation or an opening) is spaced as an
/// ordinary atom: the unary minus of `−x`.
pub fn resolve_binary(previous: Option<AtomClass>, class: AtomClass) -> AtomClass {
    use AtomClass::*;
    if class == Bin
        && matches!(
            previous,
            None | Some(Bin) | Some(Op) | Some(Rel) | Some(Open) | Some(Punct)
        )
    {
        Ord
    } else {
        class
    }
}

/// Whether a run of this text is one of the stretchy characters a
/// delimiter, a group character or an accent draws.
pub fn is_vertical_delimiter(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '|'
            | '‖'
            | '⟨'
            | '⟩'
            | '⌈'
            | '⌉'
            | '⌊'
            | '⌋'
            | '⟦'
            | '⟧'
            | '〈'
            | '〉'
            | '│'
            | '∣'
            | '∥'
            | '/'
            | '\\'
    )
}

/// The spacing character Word shows for a combining accent when it has no
/// math face to draw the mark itself from.
pub fn spacing_accent(ch: char) -> char {
    match ch {
        '\u{0300}' => '`',
        '\u{0301}' => '´',
        '\u{0302}' => 'ˆ',
        '\u{0303}' => '˜',
        '\u{0304}' | '\u{0305}' => '¯',
        '\u{0306}' => '˘',
        '\u{0307}' => '˙',
        '\u{0308}' => '¨',
        '\u{030C}' => 'ˇ',
        '\u{030A}' => '˚',
        '\u{20D6}' => '←',
        '\u{20D7}' => '→',
        '\u{20E1}' => '↔',
        '\u{20D0}' => '↼',
        '\u{20D1}' => '⇀',
        '\u{20DB}' => '⋯',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_take_the_alphabet_their_run_names() {
        use MathScript::*;
        use MathVariant::*;
        assert_eq!(math_alphanumeric('x', Italic, Roman), '𝑥');
        assert_eq!(math_alphanumeric('h', Italic, Roman), 'ℎ');
        assert_eq!(math_alphanumeric('x', Plain, Roman), 'x');
        assert_eq!(math_alphanumeric('A', Bold, Roman), '𝐀');
        assert_eq!(math_alphanumeric('R', Plain, DoubleStruck), 'ℝ');
        assert_eq!(math_alphanumeric('A', Plain, DoubleStruck), '𝔸');
        assert_eq!(math_alphanumeric('L', Italic, Script), 'ℒ');
        assert_eq!(math_alphanumeric('A', Italic, Script), '𝒜');
        assert_eq!(math_alphanumeric('g', Plain, Fraktur), '𝔤');
        assert_eq!(math_alphanumeric('Z', Plain, Fraktur), 'ℨ');
        assert_eq!(math_alphanumeric('a', Plain, SansSerif), '𝖺');
        assert_eq!(math_alphanumeric('a', Plain, Monospace), '𝚊');
        assert_eq!(math_alphanumeric('1', Bold, Roman), '𝟏');
        assert_eq!(math_alphanumeric('1', Italic, Roman), '1');
        assert_eq!(math_alphanumeric('α', Italic, Roman), '𝛼');
        assert_eq!(math_alphanumeric('Ω', Italic, Roman), 'Ω');
        assert_eq!(math_alphanumeric('Σ', Bold, Roman), '𝚺');
        assert_eq!(math_alphanumeric('ω', BoldItalic, Roman), '𝝎');
        assert_eq!(math_alphanumeric('ϕ', Italic, Roman), '𝜙');
    }

    #[test]
    fn operators_space_as_tex_spaces_its_atoms() {
        use AtomClass::*;
        assert_eq!(char_class('='), Rel);
        assert_eq!(char_class('+'), Bin);
        assert_eq!(char_class('→'), Rel);
        assert_eq!(char_class('('), Open);
        assert_eq!(char_class(','), Punct);
        assert_eq!(atom_space(Ord, Rel, false), 5.0);
        assert_eq!(atom_space(Ord, Rel, true), 0.0);
        assert_eq!(atom_space(Ord, Bin, false), 4.0);
        assert_eq!(atom_space(Op, Ord, true), 3.0);
        assert_eq!(atom_space(Ord, Open, false), 0.0);
        assert_eq!(resolve_binary(None, Bin), Ord);
        assert_eq!(resolve_binary(Some(Rel), Bin), Ord);
        assert_eq!(resolve_binary(Some(Ord), Bin), Bin);
    }
}
