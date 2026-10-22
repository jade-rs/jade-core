//! 検索と置換's オプション: everything Word's Find and Replace dialog does
//! beyond the find bar's match case and whole words (`word::find`).
//!
//! - ワイルドカードを使用する: Word's own syntax — `?` `*` `[a-z]` `[!a]`
//!   `{n}` `{n,}` `{n,m}` `@` `<` `>` `( )` `\` — and `\1`…`\9` in the
//!   replacement;
//! - 特殊文字: `^p` `^t` `^l` `^m` `^n` `^#` `^$` `^?` `^w` `^s` `^~` `^+`
//!   `^=` `^-` `^^` `^nnn`, and `^&` (what was found) in the replacement;
//! - 書式 and スタイル to find, and to give what replaces it;
//! - 検索方向 (下へ, 上へ, すべて) and 検索する場所 (メイン文書, 脚注,
//!   選択範囲);
//! - 接頭辞に一致する, 接尾辞に一致する, 句読点を無視する, 空白文字を無視
//!   する, 半角と全角を区別する, ひらがなとカタカナを区別する.
//!
//! The document is searched as one text with each paragraph mark a `\r`,
//! so a pattern can run across paragraphs as Word's can.

use crate::document::RunProperties;
use crate::word_document::WordDocument;
use jade_docx_headless_api::HeadlessCommand;

use super::{Position, WordSession};

/// Which way 次を検索 goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    Down,
    Up,
    All,
}

/// 検索する場所.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// メイン文書: the body and its text boxes.
    #[default]
    Main,
    /// 脚注と文末脚注.
    Notes,
    /// 選択範囲.
    Selection,
}

/// The formatting to find, or to give the replacement: each field set is a
/// condition (or a change); the rest are left alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormatFilter {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    /// `RRGGBB`.
    pub color: Option<String>,
    /// A highlight name, or `none` for text without one, `any` for any.
    pub highlight: Option<String>,
    /// A paragraph or character style's id.
    pub style: Option<String>,
}

impl FormatFilter {
    pub fn is_empty(&self) -> bool {
        *self == FormatFilter::default()
    }

    /// Whether a character with `run` properties in a paragraph of style
    /// `paragraph_style` meets every condition.
    pub fn matches(&self, run: Option<&RunProperties>, paragraph_style: Option<&str>) -> bool {
        let flag = |value: Option<bool>| value.unwrap_or(false);
        if let Some(bold) = self.bold {
            if flag(run.and_then(|run| run.bold)) != bold {
                return false;
            }
        }
        if let Some(italic) = self.italic {
            if flag(run.and_then(|run| run.italic)) != italic {
                return false;
            }
        }
        if let Some(underline) = self.underline {
            if flag(run.and_then(|run| run.underline)) != underline {
                return false;
            }
        }
        if let Some(family) = &self.font_family {
            let actual = run.and_then(|run| {
                run.font_family.as_deref().or_else(|| {
                    run.fonts
                        .as_ref()
                        .and_then(|fonts| fonts.east_asia.as_deref())
                })
            });
            if actual.is_none_or(|actual: &str| !actual.eq_ignore_ascii_case(family)) {
                return false;
            }
        }
        if let Some(size) = self.font_size {
            if run
                .and_then(|run| run.font_size)
                .is_none_or(|actual: f32| (actual - size).abs() > 0.01)
            {
                return false;
            }
        }
        if let Some(color) = &self.color {
            if run
                .and_then(|run| run.color.as_deref())
                .is_none_or(|actual: &str| {
                    !actual.trim_start_matches('#').eq_ignore_ascii_case(color)
                })
            {
                return false;
            }
        }
        if let Some(highlight) = &self.highlight {
            let actual = run
                .and_then(|run| run.highlight.as_deref())
                .filter(|value| *value != "none");
            let ok = match highlight.as_str() {
                "none" => actual.is_none(),
                "any" => actual.is_some(),
                wanted => actual.is_some_and(|actual: &str| actual.eq_ignore_ascii_case(wanted)),
            };
            if !ok {
                return false;
            }
        }
        if let Some(style) = &self.style {
            let character = run.and_then(|run| run.style.as_deref());
            if character != Some(style.as_str()) && paragraph_style != Some(style.as_str()) {
                return false;
            }
        }
        true
    }

    /// The commands that give `start..end` this formatting.
    pub fn commands(
        &self,
        start: Position,
        end: Position,
        paragraph_style: bool,
    ) -> Vec<HeadlessCommand> {
        let (sp, so, ep, eo) = (start.0, start.1, end.0, end.1);
        let mut commands = Vec::new();
        if let Some(enabled) = self.bold {
            commands.push(HeadlessCommand::ApplyBold {
                start_paragraph: sp,
                start_offset: so,
                end_paragraph: ep,
                end_offset: eo,
                enabled,
            });
        }
        if let Some(enabled) = self.italic {
            commands.push(HeadlessCommand::ApplyItalic {
                start_paragraph: sp,
                start_offset: so,
                end_paragraph: ep,
                end_offset: eo,
                enabled,
            });
        }
        if let Some(enabled) = self.underline {
            commands.push(HeadlessCommand::ApplyUnderline {
                start_paragraph: sp,
                start_offset: so,
                end_paragraph: ep,
                end_offset: eo,
                kind: None,
                enabled,
            });
        }
        if let Some(family) = &self.font_family {
            commands.push(HeadlessCommand::SetFontFamily {
                start_paragraph: sp,
                start_offset: so,
                end_paragraph: ep,
                end_offset: eo,
                font_family: family.clone(),
            });
        }
        if let Some(size) = self.font_size {
            commands.push(HeadlessCommand::SetFontSize {
                start_paragraph: sp,
                start_offset: so,
                end_paragraph: ep,
                end_offset: eo,
                font_size: size,
            });
        }
        if let Some(color) = &self.color {
            commands.push(HeadlessCommand::SetFontColor {
                start_paragraph: sp,
                start_offset: so,
                end_paragraph: ep,
                end_offset: eo,
                color: color.clone(),
            });
        }
        if let Some(highlight) = &self.highlight {
            if highlight != "any" {
                commands.push(HeadlessCommand::SetHighlightColor {
                    start_paragraph: sp,
                    start_offset: so,
                    end_paragraph: ep,
                    end_offset: eo,
                    color: highlight.clone(),
                });
            }
        }
        if let Some(style) = &self.style {
            commands.push(if paragraph_style {
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: sp,
                    end_paragraph: ep,
                    style_id: Some(style.clone()),
                }
            } else {
                HeadlessCommand::SetCharacterStyle {
                    start_paragraph: sp,
                    start_offset: so,
                    end_paragraph: ep,
                    end_offset: eo,
                    style_id: Some(style.clone()),
                }
            });
        }
        commands
    }
}

/// Everything 検索と置換's オプション can say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub wildcards: bool,
    /// 接頭辞に一致する: the match starts a word.
    pub match_prefix: bool,
    /// 接尾辞に一致する: the match ends a word.
    pub match_suffix: bool,
    /// 半角と全角を区別する: Ａ does not find A.
    pub match_width: bool,
    /// ひらがなとカタカナを区別する: カナ does not find かな.
    pub match_kana: bool,
    pub ignore_punctuation: bool,
    pub ignore_whitespace: bool,
    pub direction: Direction,
    pub scope: Scope,
    /// 書式: what the found text must look like.
    pub format: FormatFilter,
    /// 置換後の文字列's 書式.
    pub replace_format: FormatFilter,
}

// ---- the pattern ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Digit,
    Letter,
    White,
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Char(char),
    Any,
    Class(Class),
    Set(Vec<(char, char)>, bool),
    WordStart,
    WordEnd,
    Group(usize, Vec<Node>),
    Repeat(Box<Node>, usize, Option<usize>),
    /// `*`: any string, as short as will do.
    Lazy,
}

/// A pattern Word's syntax could not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternError(pub String);

/// The character a `^` code stands for in either field.
fn special_char(code: char) -> Option<char> {
    Some(match code {
        'p' => '\r',
        't' => '\t',
        'l' => '\n',
        'm' => '\u{000C}',
        'n' => '\u{000E}',
        's' => '\u{00A0}',
        '~' => '\u{2011}',
        '-' => '\u{00AD}',
        '+' => '—',
        '=' => '–',
        '^' => '^',
        _ => return None,
    })
}

/// `^nnn`: the character of that decimal code (13 is a paragraph mark).
fn numbered_char(chars: &[char], at: &mut usize) -> Option<char> {
    let start = *at;
    while *at < chars.len() && chars[*at].is_ascii_digit() && *at - start < 5 {
        *at += 1;
    }
    if *at == start {
        return None;
    }
    let code: u32 = chars[start..*at].iter().collect::<String>().parse().ok()?;
    Some(match code {
        13 => '\r',
        11 => '\n',
        12 => '\u{000C}',
        14 => '\u{000E}',
        code => char::from_u32(code)?,
    })
}

/// The `^` code at `chars[*at]` (after the caret), as a node.
fn caret_code(chars: &[char], at: &mut usize) -> Result<Node, PatternError> {
    let Some(&code) = chars.get(*at) else {
        return Err(PatternError("^ at the end".to_string()));
    };
    if code.is_ascii_digit() {
        return numbered_char(chars, at)
            .map(Node::Char)
            .ok_or_else(|| PatternError("^ code".to_string()));
    }
    *at += 1;
    Ok(match code {
        '#' => Node::Class(Class::Digit),
        '$' => Node::Class(Class::Letter),
        '?' => Node::Any,
        'w' => Node::Repeat(Box::new(Node::Class(Class::White)), 1, None),
        code => Node::Char(
            special_char(code).ok_or_else(|| PatternError(format!("^{code} is not a code")))?,
        ),
    })
}

/// A plain query (no wildcards): its characters and `^` codes.
fn compile_plain(query: &str) -> Result<Vec<Node>, PatternError> {
    let chars: Vec<char> = query.chars().collect();
    let mut nodes = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        at += 1;
        if ch == '^' {
            nodes.push(caret_code(&chars, &mut at)?);
        } else {
            nodes.push(Node::Char(ch));
        }
    }
    Ok(nodes)
}

/// A wildcard query, in Word's syntax.
fn compile_wildcards(query: &str) -> Result<(Vec<Node>, usize), PatternError> {
    let chars: Vec<char> = query.chars().collect();
    let mut at = 0;
    let mut groups = 0;
    let nodes = sequence(&chars, &mut at, &mut groups, false)?;
    if at < chars.len() {
        return Err(PatternError("unbalanced )".to_string()));
    }
    Ok((nodes, groups))
}

fn sequence(
    chars: &[char],
    at: &mut usize,
    groups: &mut usize,
    in_group: bool,
) -> Result<Vec<Node>, PatternError> {
    let mut nodes: Vec<Node> = Vec::new();
    while *at < chars.len() {
        let ch = chars[*at];
        *at += 1;
        let node = match ch {
            ')' if in_group => return Ok(nodes),
            ')' => return Err(PatternError("unbalanced )".to_string())),
            '(' => {
                *groups += 1;
                let index = *groups;
                let inner = sequence(chars, at, groups, true)?;
                if chars.get(*at - 1) != Some(&')') {
                    return Err(PatternError("unbalanced (".to_string()));
                }
                Node::Group(index, inner)
            }
            '?' => Node::Any,
            '*' => Node::Lazy,
            '<' => Node::WordStart,
            '>' => Node::WordEnd,
            '\\' => Node::Char(
                *chars
                    .get(*at)
                    .ok_or_else(|| PatternError("\\ at the end".to_string()))
                    .inspect(|_| *at += 1)?,
            ),
            '^' => caret_code(chars, at)?,
            '[' => set(chars, at)?,
            '@' => {
                let previous = nodes
                    .pop()
                    .ok_or_else(|| PatternError("@ with nothing before it".to_string()))?;
                Node::Repeat(Box::new(previous), 1, None)
            }
            '{' => {
                let close = chars[*at..]
                    .iter()
                    .position(|ch| *ch == '}')
                    .ok_or_else(|| PatternError("{ without }".to_string()))?;
                let body: String = chars[*at..*at + close].iter().collect();
                *at += close + 1;
                let (min, max) = match body.split_once(',') {
                    None => {
                        let n = body
                            .trim()
                            .parse()
                            .map_err(|_| PatternError(body.clone()))?;
                        (n, Some(n))
                    }
                    Some((min, max)) => (
                        min.trim().parse().map_err(|_| PatternError(body.clone()))?,
                        if max.trim().is_empty() {
                            None
                        } else {
                            Some(max.trim().parse().map_err(|_| PatternError(body.clone()))?)
                        },
                    ),
                };
                let previous = nodes
                    .pop()
                    .ok_or_else(|| PatternError("{} with nothing before it".to_string()))?;
                Node::Repeat(Box::new(previous), min, max)
            }
            ch => Node::Char(ch),
        };
        nodes.push(node);
    }
    if in_group {
        return Err(PatternError("unbalanced (".to_string()));
    }
    Ok(nodes)
}

/// `[…]` after its `[`.
fn set(chars: &[char], at: &mut usize) -> Result<Node, PatternError> {
    let negated = chars.get(*at) == Some(&'!');
    if negated {
        *at += 1;
    }
    let mut ranges = Vec::new();
    loop {
        let Some(&ch) = chars.get(*at) else {
            return Err(PatternError("[ without ]".to_string()));
        };
        *at += 1;
        if ch == ']' && !ranges.is_empty() {
            break;
        }
        let ch = if ch == '\\' {
            let escaped = *chars
                .get(*at)
                .ok_or_else(|| PatternError("\\ at the end".to_string()))?;
            *at += 1;
            escaped
        } else {
            ch
        };
        if chars.get(*at) == Some(&'-') && chars.get(*at + 1).is_some_and(|next| *next != ']') {
            let to = chars[*at + 1];
            *at += 2;
            if to < ch {
                return Err(PatternError(format!("{ch}-{to} goes backwards")));
            }
            ranges.push((ch, to));
        } else {
            ranges.push((ch, ch));
        }
    }
    Ok(Node::Set(ranges, negated))
}

// ---- matching ----------------------------------------------------------------

/// How two characters compare under the options.
#[derive(Clone, Copy)]
struct Fold {
    case: bool,
    width: bool,
    kana: bool,
}

impl Fold {
    fn of(options: &SearchOptions) -> Fold {
        if options.wildcards {
            // Word's wildcard search is exact.
            return Fold {
                case: true,
                width: true,
                kana: true,
            };
        }
        Fold {
            case: options.match_case,
            width: options.match_width,
            kana: options.match_kana,
        }
    }

    fn normal(self, ch: char) -> char {
        let mut ch = ch;
        if !self.width {
            ch = narrow(ch);
        }
        if !self.kana {
            ch = to_hiragana(ch);
        }
        if !self.case {
            ch = ch.to_lowercase().next().unwrap_or(ch);
        }
        ch
    }

    fn same(self, a: char, b: char) -> bool {
        a == b || self.normal(a) == self.normal(b)
    }
}

/// A full-width letter, digit or sign as its half-width one, and a
/// half-width katakana as its full-width one.
pub fn narrow(ch: char) -> char {
    let code = ch as u32;
    match code {
        0xFF01..=0xFF5E => char::from_u32(code - 0xFEE0).unwrap_or(ch),
        0x3000 => ' ',
        0xFF66..=0xFF9D => {
            const KANA: &str = "ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン";
            KANA.chars().nth((code - 0xFF66) as usize).unwrap_or(ch)
        }
        _ => ch,
    }
}

/// A katakana as the hiragana of the same sound.
pub fn to_hiragana(ch: char) -> char {
    let code = ch as u32;
    match code {
        0x30A1..=0x30F6 => char::from_u32(code - 0x60).unwrap_or(ch),
        _ => ch,
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

type Captures = Vec<Option<(usize, usize)>>;

struct Matcher<'a> {
    text: &'a [char],
    fold: Fold,
}

impl Matcher<'_> {
    fn at_word_start(&self, pos: usize) -> bool {
        pos < self.text.len()
            && is_word_char(self.text[pos])
            && (pos == 0 || !is_word_char(self.text[pos - 1]))
    }

    fn at_word_end(&self, pos: usize) -> bool {
        pos > 0
            && is_word_char(self.text[pos - 1])
            && (pos == self.text.len() || !is_word_char(self.text[pos]))
    }

    /// One character against a single-character node.
    fn one(&self, node: &Node, ch: char) -> bool {
        match node {
            Node::Char(want) => self.fold.same(*want, ch),
            Node::Any => ch != '\r',
            Node::Class(Class::Digit) => ch.is_numeric(),
            Node::Class(Class::Letter) => ch.is_alphabetic(),
            Node::Class(Class::White) => matches!(ch, ' ' | '\t' | '\u{3000}' | '\u{00A0}'),
            Node::Set(ranges, negated) => {
                let inside = ranges.iter().any(|(from, to)| *from <= ch && ch <= *to);
                inside != *negated
            }
            _ => false,
        }
    }

    /// Matches `nodes` from `pos`, then whatever `then` asks of the rest.
    fn seq(
        &self,
        nodes: &[Node],
        pos: usize,
        caps: &mut Captures,
        then: &mut dyn FnMut(usize, &mut Captures) -> bool,
    ) -> bool {
        let Some((first, rest)) = nodes.split_first() else {
            return then(pos, caps);
        };
        match first {
            Node::WordStart => self.at_word_start(pos) && self.seq(rest, pos, caps, then),
            Node::WordEnd => self.at_word_end(pos) && self.seq(rest, pos, caps, then),
            Node::Group(index, inner) => {
                let index = *index;
                let start = pos;
                self.seq(inner, pos, caps, &mut |end, caps: &mut Captures| {
                    let saved = caps[index];
                    caps[index] = Some((start, end));
                    if self.seq(rest, end, caps, then) {
                        return true;
                    }
                    caps[index] = saved;
                    false
                })
            }
            Node::Lazy => {
                let mut end = pos;
                loop {
                    if self.seq(rest, end, caps, then) {
                        return true;
                    }
                    if end >= self.text.len() {
                        return false;
                    }
                    end += 1;
                }
            }
            Node::Repeat(inner, min, max) => {
                self.repeat(inner, *min, *max, 0, pos, rest, caps, then)
            }
            single => {
                pos < self.text.len()
                    && self.one(single, self.text[pos])
                    && self.seq(rest, pos + 1, caps, then)
            }
        }
    }

    /// `inner` repeated, as many times as will do (greedy), then the rest.
    #[allow(clippy::too_many_arguments)]
    fn repeat(
        &self,
        inner: &Node,
        min: usize,
        max: Option<usize>,
        count: usize,
        pos: usize,
        rest: &[Node],
        caps: &mut Captures,
        then: &mut dyn FnMut(usize, &mut Captures) -> bool,
    ) -> bool {
        if max.is_none_or(|max| count < max) {
            let once = std::slice::from_ref(inner);
            let tried = self.seq(once, pos, caps, &mut |next, caps: &mut Captures| {
                next > pos && self.repeat(inner, min, max, count + 1, next, rest, caps, then)
            });
            if tried {
                return true;
            }
        }
        count >= min && self.seq(rest, pos, caps, then)
    }
}

/// A compiled query.
pub struct Pattern {
    nodes: Vec<Node>,
    groups: usize,
}

impl Pattern {
    pub fn compile(query: &str, options: &SearchOptions) -> Result<Pattern, PatternError> {
        let (mut nodes, groups) = if options.wildcards {
            compile_wildcards(query)?
        } else {
            (compile_plain(query)?, 0)
        };
        if options.ignore_punctuation || options.ignore_whitespace {
            nodes.retain(|node| match node {
                Node::Char(ch) => !skipped(*ch, options),
                _ => true,
            });
        }
        Ok(Pattern { nodes, groups })
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Whether 句読点を無視する or 空白文字を無視する leaves `ch` out of both
/// the text and the query.
fn skipped(ch: char, options: &SearchOptions) -> bool {
    (options.ignore_whitespace && matches!(ch, ' ' | '\t' | '\u{3000}' | '\u{00A0}'))
        || (options.ignore_punctuation
            && !matches!(ch, '\r' | '\n')
            && (ch.is_ascii_punctuation()
                || matches!(
                    ch,
                    '、' | '。'
                        | '，'
                        | '．'
                        | '・'
                        | '：'
                        | '；'
                        | '？'
                        | '！'
                        | '「'
                        | '」'
                        | '『'
                        | '』'
                        | '（'
                        | '）'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '…'
                        | '—'
                        | '–'
                )))
}

/// A match in a flat text: its character range and its groups'.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextMatch {
    pub start: usize,
    pub end: usize,
    pub groups: Vec<Option<(usize, usize)>>,
}

/// Every match of `pattern` in `text`, left to right, none overlapping.
pub fn find_in(text: &[char], pattern: &Pattern, options: &SearchOptions) -> Vec<TextMatch> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let matcher = Matcher {
        text,
        fold: Fold::of(options),
    };
    let mut found = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        let mut caps: Captures = vec![None; pattern.groups + 1];
        let mut end = None;
        let mut groups = Vec::new();
        matcher.seq(
            &pattern.nodes,
            pos,
            &mut caps,
            &mut |at, caps: &mut Captures| {
                if at == pos {
                    return false;
                }
                end = Some(at);
                groups = caps.clone();
                true
            },
        );
        match end {
            Some(end) if edges_ok(&matcher, pos, end, options) => {
                found.push(TextMatch {
                    start: pos,
                    end,
                    groups,
                });
                pos = end;
            }
            _ => pos += 1,
        }
    }
    found
}

fn edges_ok(matcher: &Matcher<'_>, start: usize, end: usize, options: &SearchOptions) -> bool {
    if options.wildcards {
        return true;
    }
    let text = matcher.text;
    let before = |pos: usize| pos > 0 && is_word_char(text[pos - 1]);
    let after = |pos: usize| pos < text.len() && is_word_char(text[pos]);
    let starts_word = !(is_word_char(text[start]) && before(start));
    let ends_word = !(is_word_char(text[end - 1]) && after(end));
    (!options.whole_word || (starts_word && ends_word))
        && (!options.match_prefix || starts_word)
        && (!options.match_suffix || ends_word)
}

/// What replaces a match: the template with `\1`…`\9` (wildcards), `^&`
/// and the `^` codes filled in.
pub fn expand_replacement(
    template: &str,
    found: &[char],
    groups: &[Option<(usize, usize)>],
    text: &[char],
    wildcards: bool,
) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        at += 1;
        match ch {
            '\\' if wildcards => match chars.get(at) {
                Some(digit) if digit.is_ascii_digit() => {
                    let index = digit.to_digit(10).unwrap_or(0) as usize;
                    at += 1;
                    if let Some(Some((start, end))) = groups.get(index) {
                        out.extend(&text[*start..*end]);
                    }
                }
                Some(other) => {
                    out.push(*other);
                    at += 1;
                }
                None => out.push('\\'),
            },
            '^' => match chars.get(at) {
                Some('&') => {
                    out.extend(found);
                    at += 1;
                }
                Some(digit) if digit.is_ascii_digit() => {
                    if let Some(ch) = numbered_char(&chars, &mut at) {
                        out.push(ch);
                    }
                }
                Some(code) => {
                    at += 1;
                    match special_char(*code) {
                        Some(ch) => out.push(ch),
                        None => {
                            out.push('^');
                            out.push(*code);
                        }
                    }
                }
                None => out.push('^'),
            },
            ch => out.push(ch),
        }
    }
    out
}

// ---- the document as one text --------------------------------------------------

/// The paragraphs a search reads, as one text: each character's place in
/// the document, and a `\r` for each paragraph's mark.
pub struct FlatText {
    pub chars: Vec<char>,
    /// Where each character is; one more entry, for the end.
    pub places: Vec<Position>,
}

impl FlatText {
    /// The paragraphs `paragraphs` of `document`, each as `text_of` gives it,
    /// leaving out what the options skip and what lies outside `within`.
    pub fn build(
        paragraphs: &[usize],
        text_of: impl Fn(usize) -> String,
        options: &SearchOptions,
        within: Option<(Position, Position)>,
    ) -> FlatText {
        let mut chars = Vec::new();
        let mut places = Vec::new();
        let inside =
            |place: Position| within.is_none_or(|(start, end)| place >= start && place < end);
        for &index in paragraphs {
            let text = text_of(index);
            for (offset, ch) in text.char_indices() {
                if skipped(ch, options) || !inside((index, offset)) {
                    continue;
                }
                chars.push(ch);
                places.push((index, offset));
            }
            if inside((index, text.len())) {
                chars.push('\r');
                places.push((index, text.len()));
            }
        }
        let end = places
            .last()
            .map(|(paragraph, offset)| (*paragraph + 1, 0).max((*paragraph, *offset)))
            .unwrap_or((0, 0));
        places.push(end);
        FlatText { chars, places }
    }

    /// The document position after flat character `index - 1`: the end of
    /// a range that ends there.
    pub fn end_place(&self, index: usize) -> Position {
        if index == 0 {
            return self.places[0];
        }
        let (paragraph, offset) = self.places[index - 1];
        if self.chars[index - 1] == '\r' {
            (paragraph + 1, 0)
        } else {
            (paragraph, offset + self.chars[index - 1].len_utf8())
        }
    }
}

/// A match in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub start: Position,
    pub end: Position,
    /// What was found and the groups' text, for the replacement.
    pub found: String,
    pub groups: Vec<Option<String>>,
}

/// The paragraphs of a document a scope searches.
pub fn scope_paragraphs(document: &WordDocument, scope: Scope) -> Vec<usize> {
    (0..document.paragraph_count())
        .filter(|&index| match scope {
            Scope::Main | Scope::Selection => {
                document.is_body_paragraph_surface(index)
                    || document.is_text_box_paragraph_surface(index)
            }
            Scope::Notes => document.is_note_paragraph_surface(index),
        })
        .collect()
}

fn paragraph_text(document: &WordDocument, index: usize) -> String {
    document
        .paragraph(index)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
        .unwrap_or_default()
}

/// The run properties of every character of a paragraph, by byte offset.
fn run_at(document: &WordDocument, index: usize, offset: usize) -> Option<RunProperties> {
    let paragraph = document.paragraph(index)?;
    let mut at = 0;
    for run in &paragraph.runs {
        let end = at + run.text.len();
        if offset >= at && offset < end {
            return run.properties.clone();
        }
        at = end;
    }
    paragraph.runs.last().and_then(|run| run.properties.clone())
}

fn paragraph_style(document: &WordDocument, index: usize) -> Option<String> {
    document
        .paragraph(index)?
        .properties
        .as_ref()?
        .style
        .as_deref()
        .map(str::to_string)
}

/// 検索と置換 with every option, for every session.
pub trait WordSearch: WordSession {
    /// Every match, in reading order.
    fn search(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchHit>, PatternError> {
        let document = self.word_document();
        let within = match options.scope {
            Scope::Selection => self.selection_range().and_then(|range| {
                let (start, end) = range.normalized();
                (start != end)
                    .then_some(((start.paragraph, start.offset), (end.paragraph, end.offset)))
            }),
            _ => None,
        };
        let paragraphs = scope_paragraphs(document, options.scope);
        // 書式 alone: the runs that have it.
        if query.is_empty() {
            if options.format.is_empty() {
                return Ok(Vec::new());
            }
            return Ok(format_runs(document, &paragraphs, &options.format, within));
        }
        let pattern = Pattern::compile(query, options)?;
        let flat = FlatText::build(
            &paragraphs,
            |index| paragraph_text(document, index),
            options,
            within,
        );
        let hits = find_in(&flat.chars, &pattern, options)
            .into_iter()
            .map(|found| SearchHit {
                start: flat.places[found.start],
                end: flat.end_place(found.end),
                found: flat.chars[found.start..found.end].iter().collect(),
                groups: found
                    .groups
                    .iter()
                    .map(|group| group.map(|(start, end)| flat.chars[start..end].iter().collect()))
                    .collect(),
            })
            // The document's last paragraph mark cannot go: a match that
            // takes it stops before it.
            .filter_map(|mut hit| {
                let count = document.paragraph_count();
                if hit.end.0 >= count && count > 0 {
                    let last = count - 1;
                    hit.end = (last, paragraph_text(document, last).len());
                    if hit.found.ends_with('\r') {
                        hit.found.pop();
                    }
                }
                (hit.start < hit.end).then_some(hit)
            })
            .filter(|hit| {
                options.format.is_empty() || {
                    let style = paragraph_style(document, hit.start.0);
                    hit_chars(&flat, hit).all(|(paragraph, offset)| {
                        options.format.matches(
                            run_at(document, paragraph, offset).as_ref(),
                            if paragraph == hit.start.0 {
                                style.as_deref()
                            } else {
                                None
                            },
                        )
                    })
                }
            })
            .collect();
        Ok(hits)
    }

    /// The match 次を検索 goes to from `from` (the caret, or the end of the
    /// match shown), the way the options' direction says; `None` when there
    /// is none that way.
    fn search_next(
        &self,
        query: &str,
        options: &SearchOptions,
        from: Position,
        current: Option<&SearchHit>,
    ) -> Result<Option<SearchHit>, PatternError> {
        let hits = self.search(query, options)?;
        let after = current.map(|hit| hit.start).unwrap_or(from);
        Ok(match options.direction {
            Direction::Down => hits.into_iter().find(|hit| {
                hit.start >= from && current.is_none_or(|current| hit.start > current.start)
            }),
            Direction::Up => hits.into_iter().rev().find(|hit| hit.start < after),
            Direction::All => {
                let next = hits
                    .iter()
                    .find(|hit| hit.start > after || (current.is_none() && hit.start >= after))
                    .cloned();
                next.or_else(|| hits.first().cloned())
            }
        })
    }

    /// Replaces one match with the template, its `\1` and `^` codes filled
    /// in and the replacement's formatting put on; returns where it ends.
    fn replace_hit(
        &mut self,
        hit: &SearchHit,
        template: &str,
        options: &SearchOptions,
    ) -> Result<Position, String> {
        let (commands, end) = replace_commands(hit, template, options);
        self.apply_commands(commands, false)?;
        Ok(end)
    }

    /// すべて置換: every match, last first, as one undo step. Answers how
    /// many.
    fn replace_every(
        &mut self,
        query: &str,
        template: &str,
        options: &SearchOptions,
    ) -> Result<usize, String> {
        let hits = self.search(query, options).map_err(|error| error.0)?;
        if hits.is_empty() {
            return Ok(0);
        }
        let mut commands = Vec::new();
        for hit in hits.iter().rev() {
            commands.extend(replace_commands(hit, template, options).0);
        }
        self.apply_commands(commands, false)?;
        Ok(hits.len())
    }
}

impl<T: WordSession + ?Sized> WordSearch for T {}

/// The document positions of a hit's characters.
fn hit_chars<'a>(flat: &'a FlatText, hit: &'a SearchHit) -> impl Iterator<Item = Position> + 'a {
    flat.places
        .iter()
        .zip(flat.chars.iter())
        .filter(move |(place, ch)| **place >= hit.start && **place < hit.end && **ch != '\r')
        .map(|(place, _)| *place)
}

/// The runs of `paragraphs` with formatting `format`, as hits.
fn format_runs(
    document: &WordDocument,
    paragraphs: &[usize],
    format: &FormatFilter,
    within: Option<(Position, Position)>,
) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for &index in paragraphs {
        let Some(paragraph) = document.paragraph(index) else {
            continue;
        };
        let style = paragraph_style(document, index);
        let mut at = 0;
        let mut open: Option<(usize, String)> = None;
        let close = |open: &mut Option<(usize, String)>, end: usize, hits: &mut Vec<SearchHit>| {
            if let Some((start, text)) = open.take() {
                if !text.is_empty() {
                    hits.push(SearchHit {
                        start: (index, start),
                        end: (index, end),
                        found: text,
                        groups: Vec::new(),
                    });
                }
            }
        };
        for run in &paragraph.runs {
            let end = at + run.text.len();
            let inside = within.is_none_or(|(from, to)| (index, at) >= from && (index, end) <= to);
            if inside
                && !run.text.is_empty()
                && format.matches(run.properties.as_ref(), style.as_deref())
            {
                match open.as_mut() {
                    Some((_, text)) => text.push_str(&run.text),
                    None => open = Some((at, run.text.clone())),
                }
            } else {
                close(&mut open, at, &mut hits);
            }
            at = end;
        }
        close(&mut open, at, &mut hits);
    }
    hits
}

/// The commands that replace `hit`, and where the replacement ends.
fn replace_commands(
    hit: &SearchHit,
    template: &str,
    options: &SearchOptions,
) -> (Vec<HeadlessCommand>, Position) {
    let found: Vec<char> = hit.found.chars().collect();
    // The groups as their own text, to be filled in.
    let mut joined: Vec<char> = Vec::new();
    let mut groups = vec![None];
    for group in hit.groups.iter().skip(1) {
        groups.push(group.as_ref().map(|text| {
            let start = joined.len();
            joined.extend(text.chars());
            (start, joined.len())
        }));
    }
    let replacement = if template.is_empty() && !options.replace_format.is_empty() {
        // 書式 alone: the text stays as it is.
        hit.found.clone()
    } else {
        expand_replacement(template, &found, &groups, &joined, options.wildcards)
    };
    let mut commands = Vec::new();
    let pieces: Vec<&str> = replacement.split('\r').collect();
    commands.push(HeadlessCommand::ReplaceRange {
        start_paragraph: hit.start.0,
        start_offset: hit.start.1,
        end_paragraph: hit.end.0,
        end_offset: hit.end.1,
        text: pieces[0].to_string(),
    });
    let mut at = (hit.start.0, hit.start.1 + pieces[0].len());
    for piece in &pieces[1..] {
        commands.push(HeadlessCommand::InsertParagraphBreak {
            paragraph: at.0,
            offset: at.1,
            count: 1,
        });
        at = (at.0 + 1, 0);
        if !piece.is_empty() {
            commands.push(HeadlessCommand::InsertText {
                paragraph: at.0,
                offset: 0,
                text: piece.to_string(),
            });
            at.1 = piece.len();
        }
    }
    if !options.replace_format.is_empty() {
        let paragraph_style = pieces.len() > 1 || (hit.start.1 == 0 && hit.found.ends_with('\r'));
        commands.extend(
            options
                .replace_format
                .commands(hit.start, at, paragraph_style),
        );
    }
    (commands, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(text: &str, query: &str, options: &SearchOptions) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let pattern = Pattern::compile(query, options).unwrap();
        find_in(&chars, &pattern, options)
            .into_iter()
            .map(|hit| chars[hit.start..hit.end].iter().collect())
            .collect()
    }

    fn wild() -> SearchOptions {
        SearchOptions {
            wildcards: true,
            ..SearchOptions::default()
        }
    }

    #[test]
    fn word_wildcards() {
        let w = wild();
        assert_eq!(find("cat cot cut", "c?t", &w), ["cat", "cot", "cut"]);
        assert_eq!(find("sad and sed", "s*d", &w), ["sad", "sed"]);
        assert_eq!(find("b1 c2 d3", "[b-c][0-9]", &w), ["b1", "c2"]);
        assert_eq!(find("bat cat rat", "[!bc]at", &w), ["rat"]);
        assert_eq!(find("10 100 1000", "10{2}", &w), ["100", "100"]);
        assert_eq!(find("1 10 100 1000", "<10{1,2}>", &w), ["10", "100"]);
        assert_eq!(find("lot loot looot", "lo@t", &w), ["lot", "loot", "looot"]);
        assert_eq!(find("interesting test", "<te*>", &w), ["test"]);
        assert_eq!(find("a.b a-b", "a\\.b", &w), ["a.b"]);
        // Exact: wildcards always match case.
        assert_eq!(find("Cat cat", "cat", &w), ["cat"]);
        assert!(Pattern::compile("(ab", &w).is_err());
        assert!(Pattern::compile("[z-a]", &w).is_err());
    }

    #[test]
    fn groups_fill_the_replacement() {
        let w = wild();
        let text: Vec<char> = "Smith John".chars().collect();
        let pattern = Pattern::compile("(<*>) (<*>)", &w).unwrap();
        let hit = &find_in(&text, &pattern, &w)[0];
        let out = expand_replacement(
            "\\2 \\1",
            &text[hit.start..hit.end],
            &hit.groups,
            &text,
            true,
        );
        assert_eq!(out, "John Smith");
        assert_eq!(
            expand_replacement("[^&]^p", &['x'], &[None], &[], false),
            "[x]\r"
        );
        assert_eq!(
            expand_replacement("^t^^^l^0169", &[], &[None], &[], false),
            "\t^\n©"
        );
    }

    #[test]
    fn special_characters() {
        let plain = SearchOptions::default();
        assert_eq!(find("a\tb", "^t", &plain), ["\t"]);
        assert_eq!(find("one\rtwo", "e^pt", &plain), ["e\rt"]);
        assert_eq!(find("a1b22", "^#", &plain), ["1", "2", "2"]);
        assert_eq!(find("1a2", "^$", &plain), ["a"]);
        assert_eq!(find("ab", "a^?", &plain), ["ab"]);
        assert_eq!(find("a  \t b", "a^wb", &plain), ["a  \t b"]);
        assert_eq!(find("x^y", "^^", &plain), ["^"]);
        assert_eq!(find("x\u{000C}y", "^m", &plain), ["\u{000C}"]);
    }

    #[test]
    fn options_for_words_width_kana_and_skipping() {
        let prefix = SearchOptions {
            match_prefix: true,
            ..SearchOptions::default()
        };
        assert_eq!(
            find("preview review", "view", &prefix),
            Vec::<String>::new()
        );
        assert_eq!(find("viewing preview", "view", &prefix), ["view"]);
        let suffix = SearchOptions {
            match_suffix: true,
            ..SearchOptions::default()
        };
        assert_eq!(find("viewing preview", "view", &suffix), ["view"]);
        // Width and kana fold unless told apart.
        let loose = SearchOptions::default();
        assert_eq!(find("ＡＢＣ abc", "abc", &loose).len(), 2);
        assert_eq!(find("カナ かな", "かな", &loose).len(), 2);
        assert_eq!(find("ｶﾅ", "カナ", &loose).len(), 1);
        let strict = SearchOptions {
            match_width: true,
            match_kana: true,
            match_case: true,
            ..SearchOptions::default()
        };
        assert_eq!(find("ＡＢＣ abc", "abc", &strict), ["abc"]);
        assert_eq!(find("カナ かな", "かな", &strict), ["かな"]);
        // Punctuation and white space left out of both.
        let ignoring = SearchOptions {
            ignore_punctuation: true,
            ignore_whitespace: true,
            ..SearchOptions::default()
        };
        let text: String = "e-mail, e mail"
            .chars()
            .filter(|ch| !skipped(*ch, &ignoring))
            .collect();
        assert_eq!(find(&text, "email", &ignoring).len(), 2);
    }

    #[test]
    fn flat_text_maps_back_to_paragraphs() {
        let texts = ["ab".to_string(), "cd".to_string()];
        let flat = FlatText::build(
            &[0, 1],
            |index| texts[index].clone(),
            &SearchOptions::default(),
            None,
        );
        assert_eq!(flat.chars.iter().collect::<String>(), "ab\rcd\r");
        let pattern = Pattern::compile("b^pc", &SearchOptions::default()).unwrap();
        let hit = &find_in(&flat.chars, &pattern, &SearchOptions::default())[0];
        assert_eq!(flat.places[hit.start], (0, 1));
        assert_eq!(flat.end_place(hit.end), (1, 1));
        // Within a selection.
        let within = FlatText::build(
            &[0, 1],
            |index| texts[index].clone(),
            &SearchOptions::default(),
            Some(((0, 1), (1, 1))),
        );
        assert_eq!(within.chars.iter().collect::<String>(), "b\rc");
    }

    #[test]
    fn a_replacement_with_paragraph_marks_splits() {
        let hit = SearchHit {
            start: (2, 3),
            end: (2, 5),
            found: "xy".to_string(),
            groups: vec![None],
        };
        let (commands, end) = replace_commands(&hit, "1^p2", &SearchOptions::default());
        assert_eq!(end, (3, 1));
        assert_eq!(commands.len(), 3);
    }
}
