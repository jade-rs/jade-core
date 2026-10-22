//! Rich Text Format (`.rtf`), in and out.
//!
//! The reader follows the RTF 1.9.1 specification's model: groups save and
//! restore character and paragraph state; destinations (`\fonttbl`,
//! `\colortbl`, `\stylesheet`, `\info`, `\pict`, `\header`, `\field`, the
//! list tables) are read for what they define and the rest skipped. Text is
//! decoded in its font's character set, or the document's `\ansicpg` —
//! Shift_JIS for a Japanese Word's RTF — and `\uN` takes Unicode as it is.
//!
//! The writer writes what Word's RTF writer does for the same document: a
//! font and colour table, heading styles, list tables, then sections of
//! paragraphs, tables row by row, and pictures as hex-encoded blips.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use crate::flow::*;
use crate::text::TextEncoding;

// ---- reading ---------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Open,
    Close,
    Word(String, Option<i32>),
    /// `\'hh`
    Hex(u8),
    /// A control symbol: `\~`, `\-`, `\_`, `\*`, `\\`, `\{`, `\}`, `\|`, `\:`.
    Symbol(u8),
    Text(Vec<u8>),
    Binary(Vec<u8>),
}

fn tokenize(bytes: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut text = Vec::new();
    let flush = |text: &mut Vec<u8>, tokens: &mut Vec<Token>| {
        if !text.is_empty() {
            tokens.push(Token::Text(std::mem::take(text)));
        }
    };
    while at < bytes.len() {
        let byte = bytes[at];
        match byte {
            b'{' => {
                flush(&mut text, &mut tokens);
                tokens.push(Token::Open);
                at += 1;
            }
            b'}' => {
                flush(&mut text, &mut tokens);
                tokens.push(Token::Close);
                at += 1;
            }
            b'\r' | b'\n' => at += 1,
            b'\\' => {
                flush(&mut text, &mut tokens);
                at += 1;
                let Some(&next) = bytes.get(at) else {
                    break;
                };
                if next.is_ascii_alphabetic() {
                    let start = at;
                    while at < bytes.len() && bytes[at].is_ascii_alphabetic() && at - start < 32 {
                        at += 1;
                    }
                    let word = String::from_utf8_lossy(&bytes[start..at]).into_owned();
                    let mut parameter = None;
                    let negative = bytes.get(at) == Some(&b'-')
                        && bytes.get(at + 1).is_some_and(u8::is_ascii_digit);
                    if negative {
                        at += 1;
                    }
                    if bytes.get(at).is_some_and(u8::is_ascii_digit) {
                        let start = at;
                        while at < bytes.len() && bytes[at].is_ascii_digit() && at - start < 10 {
                            at += 1;
                        }
                        let value: i64 = std::str::from_utf8(&bytes[start..at])
                            .ok()
                            .and_then(|digits| digits.parse().ok())
                            .unwrap_or(0);
                        let value = if negative { -value } else { value };
                        parameter = Some(value.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
                    }
                    if bytes.get(at) == Some(&b' ') {
                        at += 1;
                    }
                    if word == "bin" {
                        let length = parameter.unwrap_or(0).max(0) as usize;
                        let end = (at + length).min(bytes.len());
                        tokens.push(Token::Binary(bytes[at..end].to_vec()));
                        at = end;
                        continue;
                    }
                    tokens.push(Token::Word(word, parameter));
                } else if next == b'\'' {
                    let hex = bytes.get(at + 1..at + 3).and_then(|digits| {
                        u8::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
                    });
                    match hex {
                        Some(value) => {
                            tokens.push(Token::Hex(value));
                            at += 3;
                        }
                        None => at += 1,
                    }
                } else if next == b'\r' || next == b'\n' {
                    // A backslash before a line end is a paragraph mark.
                    tokens.push(Token::Word("par".into(), None));
                    at += 1;
                } else {
                    tokens.push(Token::Symbol(next));
                    at += 1;
                }
            }
            _ => {
                text.push(byte);
                at += 1;
            }
        }
    }
    flush(&mut text, &mut tokens);
    tokens
}

/// Where text in a group goes.
#[derive(Clone, Debug, PartialEq)]
enum Destination {
    /// The document, a header or a footer.
    Text,
    FontTable,
    ColorTable,
    StyleSheet,
    Info(Option<&'static str>),
    Picture,
    FieldInstruction,
    ListTable,
    ListOverrideTable,
    /// Text that is not content, inside which a `\pict` is still read
    /// (a shape's properties).
    ShapeProperties,
    Skip,
}

#[derive(Clone, Debug, Default)]
struct ParagraphState {
    kind: ParagraphKind,
    alignment: Option<Alignment>,
    left: i32,
    right: i32,
    first: i32,
    before: Option<u32>,
    after: Option<u32>,
    line: Option<i32>,
    line_multiple: bool,
    in_table: bool,
    page_break_before: bool,
    keep_with_next: bool,
    list_override: Option<i32>,
    list_level: u8,
    legacy_list: Option<bool>,
    shading: Option<usize>,
}

#[derive(Clone, Debug, Default)]
struct CharState {
    format: CharFormat,
    font: Option<i32>,
    color: Option<usize>,
    highlight: Option<usize>,
    background: Option<usize>,
}

#[derive(Clone, Debug)]
struct GroupState {
    destination: Destination,
    character: CharState,
    paragraph: ParagraphState,
    unicode_skip: usize,
    /// The story the group's text goes to.
    story: usize,
}

#[derive(Clone, Debug, Default)]
struct CellDefinition {
    right: i32,
    horizontal_merge: Option<bool>,
    vertical_merge: VerticalMerge,
    shading: Option<usize>,
}

#[derive(Default)]
struct TableBuilder {
    rows: Vec<(Vec<CellDefinition>, Vec<Vec<Block>>, bool)>,
    definitions: Vec<CellDefinition>,
    cells: Vec<Vec<Block>>,
    cell: Vec<Block>,
    header: bool,
    borders: bool,
}

#[derive(Default)]
struct Story {
    blocks: Vec<Block>,
    paragraph: Paragraph,
    table: Option<TableBuilder>,
}

#[derive(Default)]
struct Picture {
    hex: Vec<u8>,
    binary: Vec<u8>,
    kind: Option<&'static str>,
    goal_width: Option<i32>,
    goal_height: Option<i32>,
    pixel_width: Option<i32>,
    pixel_height: Option<i32>,
    scale_x: Option<i32>,
    scale_y: Option<i32>,
}

struct RtfReader {
    fonts: HashMap<i32, (String, Option<TextEncoding>)>,
    font_being_read: Option<i32>,
    font_name: String,
    font_charset: Option<TextEncoding>,
    colors: Vec<Option<String>>,
    color_being_read: (u8, u8, u8, bool),
    styles: HashMap<i32, String>,
    style_being_read: Option<i32>,
    style_name: String,
    default_encoding: TextEncoding,
    default_font: Option<i32>,
    lists: HashMap<i32, Vec<bool>>,
    list_being_read: Option<(Option<i32>, Vec<bool>)>,
    overrides: HashMap<i32, i32>,
    override_being_read: (Option<i32>, Option<i32>),
    info: BTreeMap<&'static str, String>,
    stories: Vec<Story>,
    sections: Vec<FlowSection>,
    section_page: PageSetup,
    document_page: PageSetup,
    pending_bytes: Vec<u8>,
    pending_encoding: TextEncoding,
    picture: Option<Picture>,
    field_instruction: String,
    /// The story each open header or footer group writes to, and which.
    header_story: Option<(usize, bool)>,
    section_header: Vec<Block>,
    section_footer: Vec<Block>,
    skip_after_unicode: usize,
    pictures_in_shape: bool,
    pending_cell: CellDefinition,
    high_surrogate: u32,
}

/// Reads an RTF file.
pub fn read_rtf(bytes: &[u8]) -> Result<FlowDocument, String> {
    if !bytes.trim_ascii_start().starts_with(b"{\\rtf") {
        return Err("not an RTF file".to_string());
    }
    let tokens = tokenize(bytes);
    let mut reader = RtfReader {
        fonts: HashMap::new(),
        font_being_read: None,
        font_name: String::new(),
        font_charset: None,
        colors: Vec::new(),
        color_being_read: (0, 0, 0, false),
        styles: HashMap::new(),
        style_being_read: None,
        style_name: String::new(),
        default_encoding: TextEncoding::Windows1252,
        default_font: None,
        lists: HashMap::new(),
        list_being_read: None,
        overrides: HashMap::new(),
        override_being_read: (None, None),
        info: BTreeMap::new(),
        stories: vec![Story::default()],
        sections: Vec::new(),
        section_page: PageSetup::default(),
        document_page: PageSetup::default(),
        pending_bytes: Vec::new(),
        pending_encoding: TextEncoding::Windows1252,
        picture: None,
        field_instruction: String::new(),
        header_story: None,
        section_header: Vec::new(),
        section_footer: Vec::new(),
        skip_after_unicode: 0,
        pictures_in_shape: false,
        pending_cell: CellDefinition::default(),
        high_surrogate: 0,
    };
    reader.run(&tokens);
    Ok(reader.finish())
}

impl RtfReader {
    fn run(&mut self, tokens: &[Token]) {
        let mut stack: Vec<GroupState> = Vec::new();
        let mut state = GroupState {
            destination: Destination::Text,
            character: CharState::default(),
            paragraph: ParagraphState::default(),
            unicode_skip: 1,
            story: 0,
        };
        let mut ignorable_next = false;
        for token in tokens {
            // The characters after `\uN` that stand in for it are skipped.
            if self.skip_after_unicode > 0 {
                match token {
                    Token::Hex(_) | Token::Symbol(_) | Token::Word(..) => {
                        self.skip_after_unicode -= 1;
                        continue;
                    }
                    Token::Text(text) => {
                        let skip = self.skip_after_unicode.min(text.len());
                        self.skip_after_unicode -= skip;
                        if skip < text.len() {
                            self.text(&state, &text[skip..]);
                        }
                        continue;
                    }
                    _ => self.skip_after_unicode = 0,
                }
            }
            match token {
                Token::Open => {
                    self.flush_text(&state);
                    stack.push(state.clone());
                }
                Token::Close => {
                    self.flush_text(&state);
                    self.end_group(&state, stack.last());
                    if let Some(previous) = stack.pop() {
                        state = previous;
                    }
                    if stack.is_empty() {
                        // The document's group is closed: anything after is
                        // not RTF.
                        state.destination = Destination::Text;
                    }
                    ignorable_next = false;
                }
                Token::Symbol(b'*') => ignorable_next = true,
                Token::Word(word, parameter) => {
                    let ignorable = std::mem::take(&mut ignorable_next);
                    self.word(word, *parameter, ignorable, &mut state);
                }
                Token::Symbol(symbol) => {
                    let text = match symbol {
                        b'~' => Some("\u{a0}"),
                        b'_' => Some("\u{2011}"),
                        b'\\' => Some("\\"),
                        b'{' => Some("{"),
                        b'}' => Some("}"),
                        _ => None,
                    };
                    if let Some(text) = text {
                        self.flush_text(&state);
                        self.unicode_text(&state, text);
                    }
                }
                Token::Hex(value) => {
                    if state.destination == Destination::Picture {
                        // A picture's data is hex digits, never escapes.
                    } else {
                        self.bytes(&state, &[*value]);
                    }
                }
                Token::Text(text) => self.text(&state, text),
                Token::Binary(data) => {
                    if let (Destination::Picture, Some(picture)) =
                        (&state.destination, self.picture.as_mut())
                    {
                        picture.binary.extend_from_slice(data);
                    }
                }
            }
        }
        self.flush_text(&state);
    }

    fn encoding_for(&self, state: &GroupState) -> TextEncoding {
        state
            .character
            .font
            .or(self.default_font)
            .and_then(|font| self.fonts.get(&font))
            .and_then(|(_, encoding)| *encoding)
            .unwrap_or(self.default_encoding)
    }

    fn text(&mut self, state: &GroupState, text: &[u8]) {
        match state.destination {
            Destination::Picture => {
                if let Some(picture) = self.picture.as_mut() {
                    picture
                        .hex
                        .extend(text.iter().filter(|byte| byte.is_ascii_hexdigit()));
                }
            }
            Destination::Text => self.bytes(state, text),
            Destination::FontTable
            | Destination::StyleSheet
            | Destination::Info(_)
            | Destination::FieldInstruction => self.bytes(state, text),
            Destination::ColorTable => {
                for byte in text {
                    if *byte == b';' {
                        let (r, g, b, set) = self.color_being_read;
                        self.colors
                            .push(set.then(|| format!("{r:02X}{g:02X}{b:02X}")));
                        self.color_being_read = (0, 0, 0, false);
                    }
                }
            }
            _ => {}
        }
    }

    fn bytes(&mut self, state: &GroupState, bytes: &[u8]) {
        let encoding = self.encoding_for(state);
        if !self.pending_bytes.is_empty() && encoding != self.pending_encoding {
            self.flush_text(state);
        }
        self.pending_encoding = encoding;
        self.pending_bytes.extend_from_slice(bytes);
    }

    fn flush_text(&mut self, state: &GroupState) {
        if self.pending_bytes.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.pending_bytes);
        let text = self.pending_encoding.decode(&bytes);
        self.unicode_text(state, &text);
    }

    fn unicode_text(&mut self, state: &GroupState, text: &str) {
        match state.destination {
            Destination::Text => {
                let format = self.char_format(&state.character);
                let story = &mut self.stories[state.story];
                story.paragraph.push_text(text, &format);
            }
            Destination::FontTable => {
                for ch in text.chars() {
                    if ch == ';' {
                        if let Some(id) = self.font_being_read {
                            let name = self.font_name.trim().to_string();
                            self.fonts.insert(id, (name, self.font_charset));
                        }
                        self.font_name.clear();
                    } else {
                        self.font_name.push(ch);
                    }
                }
            }
            Destination::StyleSheet => {
                for ch in text.chars() {
                    if ch == ';' {
                        let id = self.style_being_read.unwrap_or(0);
                        self.styles.insert(id, self.style_name.trim().to_string());
                        self.style_name.clear();
                    } else {
                        self.style_name.push(ch);
                    }
                }
            }
            Destination::Info(Some(key)) => {
                self.info.entry(key).or_default().push_str(text);
            }
            Destination::FieldInstruction => self.field_instruction.push_str(text),
            _ => {}
        }
    }

    fn char_format(&self, state: &CharState) -> CharFormat {
        let mut format = state.format.clone();
        format.font = state
            .font
            .filter(|font| Some(*font) != self.default_font)
            .and_then(|font| self.fonts.get(&font))
            .map(|(name, _)| name.clone())
            .filter(|name| !name.is_empty());
        let color = |index: Option<usize>| {
            index.and_then(|index| self.colors.get(index).cloned().flatten())
        };
        format.color = color(state.color);
        format.highlight = color(state.highlight)
            .map(|color| highlight_named(&color).unwrap_or("yellow").to_string());
        format.shading = color(state.background);
        format
    }

    fn word(
        &mut self,
        word: &str,
        parameter: Option<i32>,
        ignorable: bool,
        state: &mut GroupState,
    ) {
        let value = parameter.unwrap_or(1);
        let on = parameter != Some(0);
        if word == "u" {
            self.flush_text(state);
            let code = if value < 0 { value + 65536 } else { value } as u32;
            let text = if (0xd800..0xdc00).contains(&code) {
                // The high half of a pair: held until the low half comes.
                self.high_surrogate = code;
                String::new()
            } else if (0xdc00..0xe000).contains(&code) {
                let high = std::mem::take(&mut self.high_surrogate);
                let combined = 0x10000 + (high.saturating_sub(0xd800) << 10) + (code - 0xdc00);
                char::from_u32(combined)
                    .map(String::from)
                    .unwrap_or_default()
            } else {
                char::from_u32(code).map(String::from).unwrap_or_default()
            };
            if !text.is_empty() {
                self.unicode_text(state, &text);
            }
            self.skip_after_unicode = state.unicode_skip;
            return;
        }
        // Destinations first.
        let destination = match word {
            "fonttbl" => Some(Destination::FontTable),
            "colortbl" => Some(Destination::ColorTable),
            "stylesheet" => Some(Destination::StyleSheet),
            "info" => Some(Destination::Info(None)),
            "title" if matches!(state.destination, Destination::Info(_)) => {
                Some(Destination::Info(Some("title")))
            }
            "author" if matches!(state.destination, Destination::Info(_)) => {
                Some(Destination::Info(Some("author")))
            }
            "subject" | "operator" | "keywords" | "comment" | "doccomm" | "company" | "manager"
            | "category" | "creatim" | "revtim" | "printim" | "buptim" | "hlinkbase"
                if matches!(state.destination, Destination::Info(_)) =>
            {
                Some(Destination::Info(Some("other")))
            }
            "pict" => {
                if matches!(state.destination, Destination::Skip) {
                    None
                } else {
                    self.picture = Some(Picture::default());
                    Some(Destination::Picture)
                }
            }
            "fldinst" => {
                self.field_instruction.clear();
                Some(Destination::FieldInstruction)
            }
            "fldrslt" => {
                let target = crate::docx_read_hyperlink(&self.field_instruction);
                state.character.format.link = target;
                Some(Destination::Text)
            }
            "listtable" => Some(Destination::ListTable),
            "listoverridetable" => Some(Destination::ListOverrideTable),
            "header" | "headerr" | "headerl" | "headerf" | "footer" | "footerr" | "footerl"
            | "footerf" => {
                // The first-page and left-page stories are not the default:
                // only `\header`/`\headerr` (and footers) are kept.
                if matches!(word, "headerl" | "headerf" | "footerl" | "footerf") {
                    Some(Destination::Skip)
                } else {
                    self.flush_text(state);
                    self.stories.push(Story::default());
                    let index = self.stories.len() - 1;
                    state.story = index;
                    state.paragraph = ParagraphState::default();
                    state.character = CharState::default();
                    self.header_story = Some((index, word.starts_with("header")));
                    Some(Destination::Text)
                }
            }
            "shpinst" | "sp" | "sn" | "sv" | "shp"
                if !matches!(state.destination, Destination::Skip) =>
            {
                self.pictures_in_shape = true;
                Some(Destination::ShapeProperties)
            }
            "nonshppict" | "shprslt" | "pntext" | "listtext" | "footnote" | "annotation"
            | "atnid" | "atnauthor" | "bkmkstart" | "bkmkend" | "xe" | "tc" | "txe" | "rxe"
            | "template" | "filetbl" | "revtbl" | "rsidtbl" | "generator" | "xmlnstbl"
            | "themedata" | "colorschememapping" | "datastore" | "latentstyles" | "pgdsctbl"
            | "mmathPr" | "listpicture" | "objdata" | "fontemb" | "fontfile" | "panose"
            | "falt" | "do" | "datafield" | "ffdeftext" | "private" | "userprops"
            | "wgrffmtfilter" | "docvar" | "pnseclvl" | "background" | "objclass" | "objname"
            | "bliptag" | "blipuid" | "picprop" | "passwordhash" => Some(Destination::Skip),
            "shpgrp" | "shptxt" => None,
            _ => None,
        };
        if let Some(destination) = destination {
            self.flush_text(state);
            state.destination = destination;
            return;
        }
        if ignorable && !self.is_known(word) {
            self.flush_text(state);
            state.destination = Destination::Skip;
            return;
        }
        match state.destination.clone() {
            Destination::FontTable => match word {
                "f" => {
                    self.flush_text(state);
                    self.font_being_read = parameter;
                    self.font_charset = None;
                    self.font_name.clear();
                }
                "fcharset" => self.font_charset = charset_encoding(value),
                "cpg" => {
                    if let Some(encoding) = codepage_encoding(value) {
                        self.font_charset = Some(encoding);
                    }
                }
                _ => {}
            },
            Destination::ColorTable => match word {
                "red" => {
                    self.color_being_read.0 = value.clamp(0, 255) as u8;
                    self.color_being_read.3 = true;
                }
                "green" => {
                    self.color_being_read.1 = value.clamp(0, 255) as u8;
                    self.color_being_read.3 = true;
                }
                "blue" => {
                    self.color_being_read.2 = value.clamp(0, 255) as u8;
                    self.color_being_read.3 = true;
                }
                _ => {}
            },
            Destination::StyleSheet => match word {
                "s" | "cs" | "ds" | "ts" => {
                    self.flush_text(state);
                    self.style_being_read = Some(if word == "s" { value } else { -1 - value });
                    self.style_name.clear();
                }
                _ => {}
            },
            Destination::Picture => {
                if let Some(picture) = self.picture.as_mut() {
                    match word {
                        "pngblip" => picture.kind = Some("png"),
                        "jpegblip" => picture.kind = Some("jpeg"),
                        "emfblip" => picture.kind = Some("emf"),
                        "wmetafile" => picture.kind = Some("wmf"),
                        "dibitmap" | "wbitmap" => picture.kind = Some("dib"),
                        "macpict" | "pmmetafile" => picture.kind = Some("unsupported"),
                        "picwgoal" => picture.goal_width = parameter,
                        "pichgoal" => picture.goal_height = parameter,
                        "picw" => picture.pixel_width = parameter,
                        "pich" => picture.pixel_height = parameter,
                        "picscalex" => picture.scale_x = parameter,
                        "picscaley" => picture.scale_y = parameter,
                        _ => {}
                    }
                }
            }
            Destination::ListTable => match word {
                "list" => self.list_being_read = Some((None, Vec::new())),
                "listid" => {
                    if let Some((id, _)) = self.list_being_read.as_mut() {
                        *id = parameter;
                    }
                }
                "levelnfc" | "levelnfcn" => {
                    if let Some((_, levels)) = self.list_being_read.as_mut() {
                        // 23 is the bullet; 255 no number.
                        levels.push(value != 23 && value != 255);
                    }
                }
                _ => {}
            },
            Destination::ListOverrideTable => match word {
                "listoverride" => self.override_being_read = (None, None),
                "listid" => self.override_being_read.0 = parameter,
                "ls" => {
                    self.override_being_read.1 = parameter;
                    if let (Some(list), Some(ls)) = self.override_being_read {
                        self.overrides.insert(ls, list);
                    }
                }
                _ => {}
            },
            Destination::Text => self.text_word(word, parameter, value, on, state),
            Destination::Info(_)
            | Destination::FieldInstruction
            | Destination::ShapeProperties
            | Destination::Skip => {
                // A legacy list's level says whether it is bulleted.
                if word == "pnlvlblt" {
                    state.paragraph.legacy_list = Some(false);
                } else if matches!(word, "pnlvlbody" | "pndec" | "pnlvl")
                    && state.paragraph.legacy_list.is_none()
                {
                    state.paragraph.legacy_list = Some(true);
                }
            }
        }
        // Document-level words that can appear before the body.
        match word {
            "ansicpg" => {
                if let Some(encoding) = codepage_encoding(value) {
                    self.default_encoding = encoding;
                }
            }
            "mac" => self.default_encoding = TextEncoding::MacRoman,
            "deff" => self.default_font = parameter,
            "uc" => state.unicode_skip = value.max(0) as usize,
            "paperw" => self.document_page.width = value.max(0) as u32,
            "paperh" => self.document_page.height = value.max(0) as u32,
            "margl" => self.document_page.left = value.max(0) as u32,
            "margr" => self.document_page.right = value.max(0) as u32,
            "margt" => self.document_page.top = value.max(0) as u32,
            "margb" => self.document_page.bottom = value.max(0) as u32,
            "landscape" => {
                let page = &mut self.document_page;
                if page.width < page.height {
                    std::mem::swap(&mut page.width, &mut page.height);
                }
            }
            _ => {}
        }
    }

    fn is_known(&self, word: &str) -> bool {
        matches!(
            word,
            "shppict" | "pn" | "fldinst" | "listtable" | "listoverridetable" | "pict" | "shpinst"
        )
    }

    fn text_word(
        &mut self,
        word: &str,
        parameter: Option<i32>,
        value: i32,
        on: bool,
        state: &mut GroupState,
    ) {
        // Words that put something in the text.
        let special = match word {
            "tab" => Some("\t"),
            "emdash" => Some("\u{2014}"),
            "endash" => Some("\u{2013}"),
            "bullet" => Some("\u{2022}"),
            "lquote" => Some("\u{2018}"),
            "rquote" => Some("\u{2019}"),
            "ldblquote" => Some("\u{201c}"),
            "rdblquote" => Some("\u{201d}"),
            "enspace" => Some("\u{2002}"),
            "emspace" => Some("\u{2003}"),
            "qmspace" => Some("\u{2005}"),
            "zwj" => Some("\u{200d}"),
            "zwnj" => Some("\u{200c}"),
            _ => None,
        };
        if let Some(text) = special {
            self.flush_text(state);
            if word == "tab" {
                self.stories[state.story]
                    .paragraph
                    .inlines
                    .push(Inline::Tab);
            } else {
                self.unicode_text(state, text);
            }
            return;
        }
        match word {
            "par" => {
                self.flush_text(state);
                self.end_paragraph(state);
            }
            "line" | "softline" => {
                self.flush_text(state);
                self.stories[state.story]
                    .paragraph
                    .inlines
                    .push(Inline::LineBreak);
            }
            "page" => {
                self.flush_text(state);
                self.stories[state.story]
                    .paragraph
                    .inlines
                    .push(Inline::PageBreak);
            }
            "sect" => {
                self.flush_text(state);
                self.end_paragraph_if_open(state);
                self.end_section();
            }
            "sectd" => {
                self.section_page = self.document_page;
            }
            "pgwsxn" => self.section_page.width = value.max(0) as u32,
            "pghsxn" => self.section_page.height = value.max(0) as u32,
            "marglsxn" => self.section_page.left = value.max(0) as u32,
            "margrsxn" => self.section_page.right = value.max(0) as u32,
            "margtsxn" => self.section_page.top = value.max(0) as u32,
            "margbsxn" => self.section_page.bottom = value.max(0) as u32,
            "headery" => self.section_page.header = value.max(0) as u32,
            "footery" => self.section_page.footer = value.max(0) as u32,
            "lndscpsxn" => {
                let page = &mut self.section_page;
                if page.width < page.height {
                    std::mem::swap(&mut page.width, &mut page.height);
                }
            }
            // Paragraph formatting.
            "pard" => {
                self.flush_text(state);
                state.paragraph = ParagraphState::default();
            }
            "s" => {
                state.paragraph.kind = self
                    .styles
                    .get(&value)
                    .map(|name| kind_of_style(name))
                    .unwrap_or_default();
            }
            "ql" => state.paragraph.alignment = Some(Alignment::Left),
            "qc" => state.paragraph.alignment = Some(Alignment::Center),
            "qr" => state.paragraph.alignment = Some(Alignment::Right),
            "qj" | "qd" => state.paragraph.alignment = Some(Alignment::Justify),
            "li" | "lin" => state.paragraph.left = value,
            "ri" | "rin" => state.paragraph.right = value,
            "fi" => state.paragraph.first = value,
            "sb" => state.paragraph.before = Some(value.max(0) as u32),
            "sa" => state.paragraph.after = Some(value.max(0) as u32),
            "sl" => state.paragraph.line = (value != 0 && value != 1000).then_some(value),
            "slmult" => state.paragraph.line_multiple = on,
            "intbl" => state.paragraph.in_table = true,
            "pagebb" => state.paragraph.page_break_before = on,
            "keepn" => state.paragraph.keep_with_next = on,
            "ls" => state.paragraph.list_override = parameter,
            "ilvl" => state.paragraph.list_level = value.clamp(0, 8) as u8,
            "cbpat" => state.paragraph.shading = Some(value.max(0) as usize),
            // Character formatting.
            "plain" => {
                self.flush_text(state);
                let link = state.character.format.link.take();
                state.character = CharState::default();
                state.character.format.link = link;
            }
            "b" => self.set(state, |c| c.format.bold = on),
            "i" => self.set(state, |c| c.format.italic = on),
            "ul" | "uld" | "uldash" | "uldashd" | "uldashdd" | "uldb" | "ulth" | "ulw"
            | "ulwave" | "ulhwave" | "ulldash" | "ulthd" | "ulthdash" => {
                self.set(state, |c| c.format.underline = on)
            }
            "ulnone" => self.set(state, |c| c.format.underline = false),
            "strike" | "striked" => self.set(state, |c| c.format.strike = on),
            "super" => self.set(state, |c| {
                c.format.superscript = on;
                c.format.subscript = false;
            }),
            "sub" => self.set(state, |c| {
                c.format.subscript = on;
                c.format.superscript = false;
            }),
            "nosupersub" => self.set(state, |c| {
                c.format.subscript = false;
                c.format.superscript = false;
            }),
            "caps" => self.set(state, |c| c.format.all_caps = on),
            "scaps" => self.set(state, |c| c.format.small_caps = on),
            "v" => self.set(state, |c| c.format.hidden = on),
            "fs" => self.set(state, |c| c.format.size = Some(value.max(1) as u32)),
            "f" => self.set(state, |c| c.font = parameter),
            "cf" => self.set(state, |c| c.color = (value > 0).then_some(value as usize)),
            "highlight" => self.set(state, |c| {
                c.highlight = (value > 0).then_some(value as usize)
            }),
            "cb" | "chcbpat" => self.set(state, |c| {
                c.background = (value > 0).then_some(value as usize)
            }),
            // Tables.
            "trowd" => {
                self.flush_text(state);
                let story = &mut self.stories[state.story];
                let table = story.table.get_or_insert_with(TableBuilder::default);
                table.definitions.clear();
                table.header = false;
            }
            "trhdr" => {
                if let Some(table) = self.stories[state.story].table.as_mut() {
                    table.header = true;
                }
            }
            "cellx" => {
                let story = &mut self.stories[state.story];
                let table = story.table.get_or_insert_with(TableBuilder::default);
                let mut definition = std::mem::take(&mut self.pending_cell);
                definition.right = value;
                table.definitions.push(definition);
            }
            "clmgf" => self.pending_cell.horizontal_merge = Some(true),
            "clmrg" => self.pending_cell.horizontal_merge = Some(false),
            "clvmgf" => self.pending_cell.vertical_merge = VerticalMerge::Restart,
            "clvmrg" => self.pending_cell.vertical_merge = VerticalMerge::Continue,
            "clcbpat" => self.pending_cell.shading = Some(value.max(0) as usize),
            "clbrdrt" | "clbrdrb" | "clbrdrl" | "clbrdrr" | "trbrdrt" | "trbrdrb" | "trbrdrl"
            | "trbrdrr" | "trbrdrh" | "trbrdrv" => {
                if let Some(table) = self.stories[state.story].table.as_mut() {
                    table.borders = true;
                }
            }
            "cell" | "nestcell" => {
                self.flush_text(state);
                let paragraph = self.take_paragraph(state);
                let story = &mut self.stories[state.story];
                let table = story.table.get_or_insert_with(TableBuilder::default);
                if paragraph.is_some() || table.cell.is_empty() {
                    table
                        .cell
                        .push(Block::Paragraph(paragraph.unwrap_or_default()));
                }
                let cell = std::mem::take(&mut table.cell);
                table.cells.push(cell);
            }
            "row" | "nestrow" => {
                self.flush_text(state);
                let story = &mut self.stories[state.story];
                if let Some(table) = story.table.as_mut() {
                    let cells = std::mem::take(&mut table.cells);
                    let definitions = table.definitions.clone();
                    let header = table.header;
                    table.rows.push((definitions, cells, header));
                }
            }
            _ => {}
        }
    }

    fn set(&mut self, state: &mut GroupState, change: impl FnOnce(&mut CharState)) {
        self.flush_text(state);
        change(&mut state.character);
    }

    /// The paragraph being written, finished with the formatting in force,
    /// or `None` when there is nothing in it to keep.
    fn take_paragraph(&mut self, state: &GroupState) -> Option<Paragraph> {
        let story = &mut self.stories[state.story];
        let mut paragraph = std::mem::take(&mut story.paragraph);
        let properties = &state.paragraph;
        paragraph.kind = properties.kind.clone();
        paragraph.alignment = properties.alignment;
        paragraph.indent_left = properties.left;
        paragraph.indent_right = properties.right;
        paragraph.indent_first_line = properties.first;
        paragraph.space_before = properties.before;
        paragraph.space_after = properties.after;
        paragraph.page_break_before = properties.page_break_before;
        paragraph.keep_with_next = properties.keep_with_next;
        paragraph.line_spacing = properties.line.map(|line| {
            if properties.line_multiple {
                LineSpacing::Multiple(line.unsigned_abs())
            } else if line < 0 {
                LineSpacing::Exact(line.unsigned_abs())
            } else {
                LineSpacing::AtLeast(line as u32)
            }
        });
        paragraph.shading = properties
            .shading
            .and_then(|index| self.colors.get(index).cloned().flatten());
        paragraph.list = match properties.list_override {
            Some(ls) => {
                let ordered = self
                    .overrides
                    .get(&ls)
                    .and_then(|list| self.lists.get(list))
                    .and_then(|levels| levels.get(properties.list_level as usize).copied())
                    .unwrap_or(true);
                Some(ListItem {
                    ordered,
                    level: properties.list_level,
                })
            }
            None => properties
                .legacy_list
                .map(|ordered| ListItem { ordered, level: 0 }),
        };
        // A list paragraph's indent belongs to the list.
        if paragraph.list.is_some() {
            paragraph.indent_left = 0;
            paragraph.indent_first_line = 0;
        }
        paragraph
            .inlines
            .retain(|inline| !matches!(inline, Inline::Text(text, _) if text.is_empty()));
        Some(paragraph)
    }

    fn end_paragraph(&mut self, state: &GroupState) {
        let Some(paragraph) = self.take_paragraph(state) else {
            return;
        };
        let story = &mut self.stories[state.story];
        if state.paragraph.in_table {
            let table = story.table.get_or_insert_with(TableBuilder::default);
            table.cell.push(Block::Paragraph(paragraph));
        } else {
            if let Some(table) = story.table.take() {
                if let Some(table) = build_table(table, &self.colors) {
                    story.blocks.push(Block::Table(table));
                }
            }
            story.blocks.push(Block::Paragraph(paragraph));
        }
    }

    fn end_paragraph_if_open(&mut self, state: &GroupState) {
        if !self.stories[state.story].paragraph.is_empty() {
            self.end_paragraph(state);
        }
    }

    fn end_group(&mut self, state: &GroupState, parent: Option<&GroupState>) {
        match state.destination {
            Destination::Picture => {
                if parent.is_some_and(|parent| parent.destination == Destination::Picture) {
                    return;
                }
                if let Some(picture) = self.picture.take() {
                    if let Some(image) = picture_image(picture) {
                        // A picture in a shape's properties goes in where
                        // the shape is.
                        let story = parent.map(|parent| parent.story).unwrap_or(0);
                        self.stories[story]
                            .paragraph
                            .inlines
                            .push(Inline::Image(image));
                    }
                }
            }
            Destination::FontTable => {
                if let Some(id) = self.font_being_read {
                    let name = self.font_name.trim().to_string();
                    if !name.is_empty() {
                        self.fonts.insert(id, (name, self.font_charset));
                    }
                    self.font_name.clear();
                }
            }
            Destination::StyleSheet => {
                if !self.style_name.trim().is_empty() {
                    let id = self.style_being_read.unwrap_or(0);
                    self.styles.insert(id, self.style_name.trim().to_string());
                    self.style_name.clear();
                }
                self.style_being_read = None;
            }
            Destination::ListTable => {
                if parent.is_some_and(|parent| parent.destination == Destination::ListTable) {
                    // A `\list` group closing: the list is defined.
                    let closing_list = self
                        .list_being_read
                        .as_ref()
                        .is_some_and(|(id, _)| id.is_some());
                    if closing_list {
                        if let Some((Some(id), levels)) = self.list_being_read.take() {
                            self.lists.insert(id, levels);
                        }
                    }
                }
            }
            Destination::Text => {
                if let Some((story, header)) = self.header_story {
                    let parent_story = parent.map(|parent| parent.story);
                    if state.story == story && parent_story != Some(story) {
                        // The header or footer group is done.
                        if !self.stories[story].paragraph.is_empty() {
                            self.end_paragraph(state);
                        }
                        let mut finished = std::mem::take(&mut self.stories[story]);
                        if let Some(table) = finished
                            .table
                            .take()
                            .and_then(|table| build_table(table, &self.colors))
                        {
                            finished.blocks.push(Block::Table(table));
                        }
                        if header {
                            self.section_header = finished.blocks;
                        } else {
                            self.section_footer = finished.blocks;
                        }
                        self.header_story = None;
                    }
                }
            }
            _ => {}
        }
        if state.destination == Destination::ShapeProperties
            && parent.is_some_and(|parent| parent.destination != Destination::ShapeProperties)
        {
            self.pictures_in_shape = false;
        }
    }

    fn end_section(&mut self) {
        let story = &mut self.stories[0];
        if let Some(table) = story
            .table
            .take()
            .and_then(|table| build_table(table, &self.colors))
        {
            story.blocks.push(Block::Table(table));
        }
        let blocks = std::mem::take(&mut story.blocks);
        self.sections.push(FlowSection {
            page: self.section_page,
            header: std::mem::take(&mut self.section_header),
            footer: std::mem::take(&mut self.section_footer),
            blocks,
        });
    }

    fn finish(mut self) -> FlowDocument {
        let open = !self.stories[0].paragraph.is_empty();
        if open {
            if let Some(table) = self.stories[0]
                .table
                .take()
                .and_then(|table| build_table(table, &self.colors))
            {
                self.stories[0].blocks.push(Block::Table(table));
            }
            let paragraph = std::mem::take(&mut self.stories[0].paragraph);
            self.stories[0].blocks.push(Block::Paragraph(paragraph));
        }
        if self.sections.is_empty() && self.section_page == PageSetup::default() {
            self.section_page = self.document_page;
        }
        let has_content = !self.stories[0].blocks.is_empty() || self.stories[0].table.is_some();
        if has_content || self.sections.is_empty() {
            self.end_section();
        }
        let default_font = self
            .default_font
            .and_then(|font| self.fonts.get(&font))
            .map(|(name, _)| name.clone())
            .filter(|name| !name.is_empty());
        let title = self
            .info
            .get("title")
            .map(|title| title.trim().to_string())
            .filter(|t| !t.is_empty());
        let author = self
            .info
            .get("author")
            .map(|author| author.trim().to_string())
            .filter(|a| !a.is_empty());
        FlowDocument {
            title,
            author,
            default_font,
            default_size: None,
            language: matches!(self.default_encoding, TextEncoding::ShiftJis)
                .then(|| "ja-JP".to_string()),
            sections: self.sections,
        }
    }
}

fn build_table(builder: TableBuilder, colors: &[Option<String>]) -> Option<Table> {
    let mut rows = builder.rows;
    // Cells written after the last \row still count.
    if !builder.cells.is_empty() {
        rows.push((builder.definitions.clone(), builder.cells, builder.header));
    }
    if rows.is_empty() {
        return None;
    }
    // The grid: every cell boundary any row has.
    let mut boundaries: Vec<i32> = rows
        .iter()
        .flat_map(|(definitions, _, _)| definitions.iter().map(|definition| definition.right))
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    let left = 0;
    let mut columns = Vec::new();
    let mut previous = left;
    for boundary in &boundaries {
        columns.push((boundary - previous).max(1) as u32);
        previous = *boundary;
    }
    let mut table = Table {
        columns,
        rows: Vec::new(),
        borders: builder.borders,
    };
    for (definitions, cells, header) in rows {
        let mut row = Row {
            cells: Vec::new(),
            header,
        };
        let mut previous = left;
        for (index, blocks) in cells.into_iter().enumerate() {
            let definition = definitions.get(index).cloned().unwrap_or_default();
            if definition.horizontal_merge == Some(false) {
                // Merged into the cell before.
                if let Some(last) = row.cells.last_mut() {
                    last.span += 1;
                    previous = definition.right;
                    continue;
                }
            }
            let right = if definitions.get(index).is_some() {
                definition.right
            } else {
                previous + 1440
            };
            let span = boundaries
                .iter()
                .filter(|boundary| **boundary > previous && **boundary <= right)
                .count()
                .max(1) as u32;
            previous = right;
            row.cells.push(Cell {
                blocks,
                span,
                vertical_merge: definition.vertical_merge,
                shading: definition
                    .shading
                    .and_then(|index| colors.get(index).cloned().flatten()),
            });
        }
        table.rows.push(row);
    }
    if table.columns.len() != table.grid_width() {
        table.columns = Vec::new();
    }
    Some(table)
}

fn picture_image(picture: Picture) -> Option<Image> {
    let mut data = picture.binary;
    if data.is_empty() {
        data = picture
            .hex
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
            .collect();
    }
    if data.is_empty() {
        return None;
    }
    if picture.kind == Some("dib") {
        data = bitmap_file(&data)?;
    }
    if picture.kind == Some("unsupported") {
        return None;
    }
    let mut image = Image::new(data, None)?;
    let scale =
        |value: u64, percent: Option<i32>| value * percent.unwrap_or(100).max(1) as u64 / 100;
    if let (Some(width), Some(height)) = (picture.goal_width, picture.goal_height) {
        if width > 0 && height > 0 {
            image.width = scale(width as u64 * EMU_PER_TWIP, picture.scale_x);
            image.height = scale(height as u64 * EMU_PER_TWIP, picture.scale_y);
        }
    } else if let (Some(width), Some(height)) = (picture.pixel_width, picture.pixel_height) {
        if width > 0 && height > 0 && matches!(image.format, ImageFormat::Emf | ImageFormat::Wmf) {
            // A metafile's \picw is in hundredths of a millimetre.
            image.width = scale(width as u64 * 360, picture.scale_x);
            image.height = scale(height as u64 * 360, picture.scale_y);
        }
    }
    Some(image)
}

/// A device-independent bitmap as a `.bmp` file: the header in front.
fn bitmap_file(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let header = u32::from_le_bytes(dib[0..4].try_into().ok()?) as usize;
    let bits = u16::from_le_bytes(dib[14..16].try_into().ok()?) as usize;
    let used = u32::from_le_bytes(dib[32..36].try_into().ok()?) as usize;
    let palette = if used > 0 {
        used
    } else if bits <= 8 {
        1 << bits
    } else {
        0
    };
    let offset = 14 + header + palette * 4;
    let mut out = Vec::with_capacity(dib.len() + 14);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((dib.len() + 14) as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

fn kind_of_style(name: &str) -> ParagraphKind {
    let lower = name.to_ascii_lowercase();
    if let Some(level) = lower
        .strip_prefix("heading ")
        .and_then(|level| level.trim().parse::<u8>().ok())
    {
        return ParagraphKind::Heading(level.clamp(1, 9));
    }
    // A Japanese Word names them 見出し 1.
    if let Some(level) = name
        .strip_prefix("見出し")
        .and_then(|level| level.trim().parse::<u8>().ok())
    {
        return ParagraphKind::Heading(level.clamp(1, 9));
    }
    match lower.as_str() {
        "title" | "表題" => ParagraphKind::Title,
        "quote" | "引用文" => ParagraphKind::Quote,
        _ => ParagraphKind::Normal,
    }
}

/// The encoding an RTF font's `\fcharset` means.
fn charset_encoding(charset: i32) -> Option<TextEncoding> {
    match charset {
        0 => Some(TextEncoding::Windows1252),
        77 => Some(TextEncoding::MacRoman),
        128 => Some(TextEncoding::ShiftJis),
        129 => Some(TextEncoding::EucKr),
        134 => Some(TextEncoding::Gb18030),
        136 => Some(TextEncoding::Big5),
        _ => None,
    }
}

/// The encoding a Windows code page number means.
fn codepage_encoding(codepage: i32) -> Option<TextEncoding> {
    match codepage {
        932 | 10001 => Some(TextEncoding::ShiftJis),
        1252 | 437 | 850 => Some(TextEncoding::Windows1252),
        10000 => Some(TextEncoding::MacRoman),
        936 => Some(TextEncoding::Gb18030),
        950 => Some(TextEncoding::Big5),
        949 => Some(TextEncoding::EucKr),
        65001 => Some(TextEncoding::Utf8),
        _ => None,
    }
}

// ---- writing ---------------------------------------------------------------

/// A document as RTF, as Word writes it: Unicode as `\uN` with a `?` for
/// readers that do not know it.
pub fn write_rtf(document: &FlowDocument) -> String {
    let mut writer = RtfWriter::default();
    writer.fonts.push(
        document
            .default_font
            .clone()
            .unwrap_or_else(|| "Times New Roman".to_string()),
    );
    // Gather the fonts and colours first: the tables come before the text.
    for section in &document.sections {
        for block in section
            .header
            .iter()
            .chain(&section.footer)
            .chain(&section.blocks)
        {
            writer.collect(block);
        }
    }
    let mut body = String::new();
    let count = document.sections.len();
    for (index, section) in document.sections.iter().enumerate() {
        let page = &section.page;
        let _ = writeln!(
            body,
            "\\sectd\\pgwsxn{}\\pghsxn{}\\marglsxn{}\\margrsxn{}\\margtsxn{}\\margbsxn{}\\headery{}\\footery{}{}",
            page.width,
            page.height,
            page.left,
            page.right,
            page.top,
            page.bottom,
            page.header,
            page.footer,
            if page.landscape() { "\\lndscpsxn" } else { "" }
        );
        if !section.header.is_empty() {
            body.push_str("{\\header ");
            writer.blocks(&section.header, page.text_width(), false, &mut body);
            body.push_str("}\n");
        }
        if !section.footer.is_empty() {
            body.push_str("{\\footer ");
            writer.blocks(&section.footer, page.text_width(), false, &mut body);
            body.push_str("}\n");
        }
        writer.blocks(&section.blocks, page.text_width(), false, &mut body);
        if index + 1 < count {
            body.push_str("\\sect\n");
        }
    }
    let mut out = String::from(
        "{\\rtf1\\ansi\\ansicpg1252\\uc1\\deff0\\deflang1033\\deflangfe1041\n{\\fonttbl",
    );
    for (index, font) in writer.fonts.iter().enumerate() {
        let charset = if !font.is_ascii() || JAPANESE_FONTS.contains(&font.as_str()) {
            128
        } else {
            0
        };
        let _ = write!(
            out,
            "{{\\f{index}\\fnil\\fcharset{charset} {};}}",
            escape_text(font)
        );
    }
    out.push_str("}\n{\\colortbl;");
    for color in &writer.colors {
        let value =
            |range: std::ops::Range<usize>| u8::from_str_radix(&color[range], 16).unwrap_or(0);
        let _ = write!(
            out,
            "\\red{}\\green{}\\blue{};",
            value(0..2),
            value(2..4),
            value(4..6)
        );
    }
    out.push_str("}\n{\\stylesheet{\\ql\\f0 Normal;}");
    for level in 1..=9u32 {
        let size = [32u32, 28, 26, 24, 22, 22, 21, 21, 21][level as usize - 1];
        let _ = write!(
            out,
            "{{\\s{level}\\ql\\keepn\\sb240\\sa60\\outlinelevel{}\\b\\fs{size}\\sbasedon0\\snext0 heading {level};}}",
            level - 1
        );
    }
    out.push_str("{\\s20\\qc\\sb240\\sa120\\b\\fs40\\sbasedon0\\snext0 Title;}{\\s21\\li864\\ri864\\i\\sbasedon0\\snext0 Quote;}{\\s22\\f0\\fs20\\sbasedon0\\snext22 HTML Preformatted;}}\n");
    // A bullet list and one numbered list per list in the document.
    out.push_str("{\\*\\listtable");
    out.push_str(&list_definition(1, false));
    for index in 0..writer.numbered_lists {
        out.push_str(&list_definition(100 + index as i32, true));
    }
    out.push_str("}\n{\\*\\listoverridetable{\\listoverride\\listid1\\listoverridecount0\\ls1}");
    for index in 0..writer.numbered_lists {
        let _ = write!(
            out,
            "{{\\listoverride\\listid{}\\listoverridecount0\\ls{}}}",
            100 + index,
            index + 2
        );
    }
    out.push_str("}\n");
    if document.title.is_some() || document.author.is_some() {
        out.push_str("{\\info");
        if let Some(title) = &document.title {
            let _ = write!(out, "{{\\title {}}}", escape_text(title));
        }
        if let Some(author) = &document.author {
            let _ = write!(out, "{{\\author {}}}", escape_text(author));
        }
        out.push_str("}\n");
    }
    if let Some(section) = document.sections.first() {
        let page = &section.page;
        let _ = writeln!(
            out,
            "\\paperw{}\\paperh{}\\margl{}\\margr{}\\margt{}\\margb{}{}",
            page.width,
            page.height,
            page.left,
            page.right,
            page.top,
            page.bottom,
            if page.landscape() { "\\landscape" } else { "" }
        );
    }
    if let Some(size) = document.default_size {
        let _ = writeln!(out, "\\fs{size}");
    }
    out.push_str(&body);
    out.push('}');
    out
}

const JAPANESE_FONTS: &[&str] = &[
    "MS Mincho",
    "MS Gothic",
    "MS PMincho",
    "MS PGothic",
    "Meiryo",
    "Yu Mincho",
    "Yu Gothic",
];

fn list_definition(id: i32, ordered: bool) -> String {
    let mut out = format!("{{\\list\\listtemplateid{id}\\listhybrid");
    for level in 0..9 {
        let indent = 720 + 720 * level;
        if ordered {
            let format = [0, 4, 2][level as usize % 3];
            let _ = write!(
                out,
                "{{\\listlevel\\levelnfc{format}\\levelnfcn{format}\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat1{{\\leveltext\\'02\\'0{level}.;}}{{\\levelnumbers\\'01;}}\\fi-360\\li{indent}\\lin{indent}}}"
            );
        } else {
            let _ = write!(
                out,
                "{{\\listlevel\\levelnfc23\\levelnfcn23\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat1{{\\leveltext\\'01\\u8226 ?;}}{{\\levelnumbers;}}\\fi-360\\li{indent}\\lin{indent}}}"
            );
        }
    }
    let _ = write!(out, "{{\\listname ;}}\\listid{id}}}");
    out
}

#[derive(Default)]
struct RtfWriter {
    fonts: Vec<String>,
    colors: Vec<String>,
    numbered_lists: usize,
    /// While writing: the `\ls` of the numbered list being continued.
    open_list: Option<usize>,
    lists_written: usize,
    counted_list_open: bool,
}

impl RtfWriter {
    fn font_index(&mut self, font: &str) -> usize {
        if let Some(index) = self.fonts.iter().position(|known| known == font) {
            return index;
        }
        self.fonts.push(font.to_string());
        self.fonts.len() - 1
    }

    fn color_index(&mut self, color: &str) -> usize {
        let color = color.trim_start_matches('#').to_ascii_uppercase();
        if let Some(index) = self.colors.iter().position(|known| *known == color) {
            return index + 1;
        }
        self.colors.push(color);
        self.colors.len()
    }

    fn collect(&mut self, block: &Block) {
        match block {
            Block::Paragraph(paragraph) => {
                if paragraph.list.is_some_and(|item| item.ordered) {
                    if !self.counted_list_open {
                        self.numbered_lists += 1;
                        self.counted_list_open = true;
                    }
                } else if paragraph.list.is_none() {
                    self.counted_list_open = false;
                }
                if let Some(shading) = &paragraph.shading {
                    self.color_index(shading);
                }
                for inline in &paragraph.inlines {
                    if let Inline::Text(_, format) = inline {
                        if let Some(font) = &format.font {
                            self.font_index(font);
                        }
                        if let Some(font) = &format.east_asian_font {
                            self.font_index(font);
                        }
                        for color in [&format.color, &format.shading].into_iter().flatten() {
                            self.color_index(color);
                        }
                        if let Some(highlight) =
                            format.highlight.as_deref().and_then(highlight_color)
                        {
                            self.color_index(highlight);
                        }
                    }
                }
            }
            Block::Table(table) => {
                self.counted_list_open = false;
                for row in &table.rows {
                    for cell in &row.cells {
                        if let Some(shading) = &cell.shading {
                            self.color_index(shading);
                        }
                        for block in &cell.blocks {
                            self.collect(block);
                        }
                    }
                }
            }
        }
    }

    fn blocks(&mut self, blocks: &[Block], width: u32, in_table: bool, out: &mut String) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => {
                    self.paragraph(paragraph, in_table, out);
                    out.push_str("\\par\n");
                }
                Block::Table(table) => {
                    self.open_list = None;
                    if in_table {
                        // A table in a table is written as its text.
                        for row in &table.rows {
                            for cell in &row.cells {
                                self.blocks(&cell.blocks, width, true, out);
                            }
                        }
                    } else {
                        self.table(table, width, out);
                    }
                }
            }
        }
    }

    fn paragraph(&mut self, paragraph: &Paragraph, in_table: bool, out: &mut String) {
        out.push_str("\\pard\\plain");
        match &paragraph.kind {
            ParagraphKind::Normal => {}
            ParagraphKind::Heading(level) => {
                let level = (*level).clamp(1, 9);
                let size = [32u32, 28, 26, 24, 22, 22, 21, 21, 21][level as usize - 1];
                let _ = write!(
                    out,
                    "\\s{level}\\keepn\\outlinelevel{}\\b\\fs{size}",
                    level - 1
                );
            }
            ParagraphKind::Title => out.push_str("\\s20\\qc\\b\\fs40"),
            ParagraphKind::Quote => out.push_str("\\s21\\li864\\ri864\\i"),
            ParagraphKind::Preformatted => out.push_str("\\s22\\fs20"),
        }
        if in_table {
            out.push_str("\\intbl");
        }
        match paragraph.alignment {
            Some(Alignment::Left) => out.push_str("\\ql"),
            Some(Alignment::Center) => out.push_str("\\qc"),
            Some(Alignment::Right) => out.push_str("\\qr"),
            Some(Alignment::Justify) => out.push_str("\\qj"),
            None => {}
        }
        match paragraph.list {
            Some(item) => {
                let ls = if item.ordered {
                    match self.open_list {
                        Some(ls) => ls,
                        None => {
                            self.lists_written += 1;
                            let ls = self.lists_written + 1;
                            self.open_list = Some(ls);
                            ls
                        }
                    }
                } else {
                    1
                };
                let indent = 720 + 720 * item.level.min(8) as i32;
                let _ = write!(
                    out,
                    "\\fi-360\\li{indent}\\lin{indent}\\ls{ls}\\ilvl{}",
                    item.level.min(8)
                );
            }
            None => {
                self.open_list = None;
                if paragraph.indent_left != 0 {
                    let _ = write!(out, "\\li{0}\\lin{0}", paragraph.indent_left);
                }
                if paragraph.indent_first_line != 0 {
                    let _ = write!(out, "\\fi{}", paragraph.indent_first_line);
                }
            }
        }
        if paragraph.indent_right != 0 {
            let _ = write!(out, "\\ri{0}\\rin{0}", paragraph.indent_right);
        }
        if let Some(before) = paragraph.space_before {
            let _ = write!(out, "\\sb{before}");
        }
        if let Some(after) = paragraph.space_after {
            let _ = write!(out, "\\sa{after}");
        }
        match paragraph.line_spacing {
            Some(LineSpacing::Multiple(line)) => {
                let _ = write!(out, "\\sl{line}\\slmult1");
            }
            Some(LineSpacing::Exact(line)) => {
                let _ = write!(out, "\\sl-{line}\\slmult0");
            }
            Some(LineSpacing::AtLeast(line)) => {
                let _ = write!(out, "\\sl{line}\\slmult0");
            }
            None => {}
        }
        if paragraph.page_break_before {
            out.push_str("\\pagebb");
        }
        if paragraph.keep_with_next {
            out.push_str("\\keepn");
        }
        if let Some(shading) = &paragraph.shading {
            let index = self.color_index(shading);
            let _ = write!(out, "\\cbpat{index}");
        }
        out.push(' ');
        let mut open_link: Option<&str> = None;
        for inline in &paragraph.inlines {
            let link = match inline {
                Inline::Text(_, format) => format.link.as_deref(),
                _ => None,
            };
            if link != open_link {
                if open_link.is_some() {
                    out.push_str("}}}");
                }
                if let Some(target) = link {
                    let instruction = match target.strip_prefix('#') {
                        Some(anchor) => format!("HYPERLINK \\\\l \"{}\"", escape_text(anchor)),
                        None => format!("HYPERLINK \"{}\"", escape_text(target)),
                    };
                    let _ = write!(
                        out,
                        "{{\\field{{\\*\\fldinst {{{instruction}}}}}{{\\fldrslt {{"
                    );
                }
                open_link = link;
            }
            match inline {
                Inline::Text(text, format) => {
                    let properties = self.run_properties(format);
                    if properties.is_empty() {
                        out.push_str(&escape_text(text));
                    } else {
                        let _ = write!(out, "{{{properties} {}}}", escape_text(text));
                    }
                }
                Inline::Tab => out.push_str("\\tab "),
                Inline::LineBreak => out.push_str("\\line "),
                Inline::PageBreak => out.push_str("\\page "),
                Inline::Image(image) => picture(image, out),
            }
        }
        if open_link.is_some() {
            out.push_str("}}}");
        }
    }

    fn run_properties(&mut self, format: &CharFormat) -> String {
        let mut out = String::new();
        if let Some(font) = &format.font {
            let index = self.font_index(font);
            let _ = write!(out, "\\f{index}");
        }
        if let Some(font) = &format.east_asian_font {
            let index = self.font_index(font);
            let _ = write!(out, "\\dbch\\af{index}");
        }
        if let Some(size) = format.size {
            let _ = write!(out, "\\fs{size}");
        }
        if format.bold {
            out.push_str("\\b");
        }
        if format.italic {
            out.push_str("\\i");
        }
        if format.underline || format.link.is_some() {
            out.push_str("\\ul");
        }
        if format.strike {
            out.push_str("\\strike");
        }
        if format.superscript {
            out.push_str("\\super");
        }
        if format.subscript {
            out.push_str("\\sub");
        }
        if format.small_caps {
            out.push_str("\\scaps");
        }
        if format.all_caps {
            out.push_str("\\caps");
        }
        if format.hidden {
            out.push_str("\\v");
        }
        let color = format
            .color
            .clone()
            .or_else(|| format.link.as_ref().map(|_| "0563C1".to_string()));
        if let Some(color) = color {
            let index = self.color_index(&color);
            let _ = write!(out, "\\cf{index}");
        }
        if let Some(highlight) = format.highlight.as_deref().and_then(highlight_color) {
            let index = self.color_index(highlight);
            let _ = write!(out, "\\highlight{index}");
        }
        if let Some(shading) = &format.shading {
            let index = self.color_index(shading);
            let _ = write!(out, "\\chcbpat{index}");
        }
        out
    }

    fn table(&mut self, table: &Table, width: u32, out: &mut String) {
        let grid = table.grid_width().max(1);
        let columns = if table.columns.len() == grid {
            table.columns.clone()
        } else {
            Table::even_columns(grid, width)
        };
        for row in &table.rows {
            let mut definition = String::from("\\trowd\\trgaph108\\trleft-108");
            if row.header {
                definition.push_str("\\trhdr");
            }
            let mut column = 0usize;
            let mut right = 0u32;
            for cell in &row.cells {
                let span = cell.span.max(1) as usize;
                right += columns.iter().skip(column).take(span).sum::<u32>();
                column += span;
                match cell.vertical_merge {
                    VerticalMerge::Restart => definition.push_str("\\clvmgf"),
                    VerticalMerge::Continue => definition.push_str("\\clvmrg"),
                    VerticalMerge::None => {}
                }
                if let Some(shading) = &cell.shading {
                    let index = self.color_index(shading);
                    let _ = write!(definition, "\\clcbpat{index}");
                }
                if table.borders {
                    definition.push_str("\\clbrdrt\\brdrs\\brdrw10\\clbrdrl\\brdrs\\brdrw10\\clbrdrb\\brdrs\\brdrw10\\clbrdrr\\brdrs\\brdrw10");
                }
                let _ = write!(definition, "\\cellx{right}");
            }
            out.push_str(&definition);
            out.push('\n');
            for cell in &row.cells {
                let blocks: Vec<&Block> = cell.blocks.iter().collect();
                if blocks.is_empty() {
                    out.push_str("\\pard\\plain\\intbl ");
                }
                for (index, block) in blocks.iter().enumerate() {
                    match block {
                        Block::Paragraph(paragraph) => {
                            self.paragraph(paragraph, true, out);
                            if index + 1 < blocks.len() {
                                out.push_str("\\par\n");
                            }
                        }
                        Block::Table(_) => {
                            self.blocks(std::slice::from_ref(block), width, true, out);
                            if index + 1 == blocks.len() {
                                out.push_str("\\pard\\plain\\intbl ");
                            }
                        }
                    }
                }
                out.push_str("\\cell\n");
            }
            out.push_str(&definition);
            out.push_str("\\row\n");
        }
        out.push_str("\\pard\\plain\n");
    }
}

fn picture(image: &Image, out: &mut String) {
    let kind = match image.format {
        ImageFormat::Png => "\\pngblip",
        ImageFormat::Jpeg => "\\jpegblip",
        ImageFormat::Emf => "\\emfblip",
        ImageFormat::Wmf => "\\wmetafile8",
        // RTF has no blip for these.
        ImageFormat::Gif | ImageFormat::Bmp | ImageFormat::Tiff => return,
    };
    let (pixel_width, pixel_height) = ImageFormat::pixel_size(&image.data).unwrap_or((
        (image.width / EMU_PER_PIXEL) as u32,
        (image.height / EMU_PER_PIXEL) as u32,
    ));
    let _ = writeln!(
        out,
        "{{\\*\\shppict{{\\pict{kind}\\picw{pixel_width}\\pich{pixel_height}\\picwgoal{}\\pichgoal{}",
        image.width / EMU_PER_TWIP,
        image.height / EMU_PER_TWIP
    );
    for chunk in image.data.chunks(64) {
        for byte in chunk {
            let _ = write!(out, "{byte:02x}");
        }
        out.push('\n');
    }
    out.push_str("}}");
}

/// Text for an RTF stream: the specials escaped, everything past ASCII as
/// `\uN?`.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            '\n' => out.push_str("\\line "),
            '\u{c}' => out.push_str("\\page "),
            '\r' => {}
            '\u{a0}' => out.push_str("\\~"),
            ch if (ch as u32) < 0x80 => out.push(ch),
            ch => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{}?", *unit as i16);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_style_rtf_reads_with_its_formatting() {
        let rtf = br#"{\rtf1\ansi\ansicpg932\deff0{\fonttbl{\f0\froman\fcharset0 Times New Roman;}{\f1\fnil\fcharset128 \'82\'6c\'82\'72 \'96\'be\'92\'a9;}}
{\colortbl;\red255\green0\blue0;\red255\green255\blue0;}
{\stylesheet{\s0 Normal;}{\s1\b\fs32 heading 1;}}
{\info{\title Sample}}
\paperw11906\paperh16838\margl1701\margr1701
\pard\s1 Heading\par
\pard\qc{\b bold}{\i italic} {\cf1 red} {\highlight2 marked}\par
\pard{\f1 \'93\'fa\'96\'7b\'8c\'ea}\u26085?\par
\pard\li720\fi-360 indented\tab tab\line next\par
{\field{\*\fldinst HYPERLINK "https://example.com/"}{\fldrslt link}}\par
}"#;
        let document = read_rtf(rtf).unwrap();
        assert_eq!(document.title.as_deref(), Some("Sample"));
        assert_eq!(document.sections[0].page.left, 1701);
        let paragraphs = document.paragraphs();
        assert_eq!(paragraphs[0].kind, ParagraphKind::Heading(1));
        assert_eq!(paragraphs[0].text(), "Heading");
        assert_eq!(paragraphs[1].alignment, Some(Alignment::Center));
        let formats: Vec<(&str, &CharFormat)> = paragraphs[1]
            .inlines
            .iter()
            .filter_map(|inline| match inline {
                Inline::Text(text, format) => Some((text.as_str(), format)),
                _ => None,
            })
            .collect();
        assert!(formats
            .iter()
            .any(|(text, format)| *text == "bold" && format.bold));
        assert!(formats
            .iter()
            .any(|(text, format)| *text == "italic" && format.italic));
        assert!(formats
            .iter()
            .any(|(text, format)| *text == "red" && format.color.as_deref() == Some("FF0000")));
        assert!(formats.iter().any(
            |(text, format)| *text == "marked" && format.highlight.as_deref() == Some("yellow")
        ));
        // Shift_JIS bytes in a fcharset128 font, and \u with its fallback skipped.
        assert_eq!(paragraphs[2].text(), "日本語日");
        assert_eq!(paragraphs[3].indent_left, 720);
        assert_eq!(paragraphs[3].indent_first_line, -360);
        assert_eq!(paragraphs[3].text(), "indented\ttab\nnext");
        let link = paragraphs[4]
            .inlines
            .iter()
            .find_map(|inline| match inline {
                Inline::Text(text, format) if text == "link" => format.link.clone(),
                _ => None,
            });
        assert_eq!(link.as_deref(), Some("https://example.com/"));
    }

    #[test]
    fn tables_merge_cells_and_keep_widths() {
        let rtf = br#"{\rtf1\ansi
\trowd\trhdr\clbrdrt\brdrs\cellx2000\cellx4000\cellx6000
\pard\intbl A\cell B\cell C\cell\row
\trowd\clmgf\cellx2000\clmrg\cellx4000\cellx6000
\pard\intbl wide\cell\cell end\cell\row
\pard after\par}"#;
        let document = read_rtf(rtf).unwrap();
        let blocks = &document.sections[0].blocks;
        let Block::Table(table) = &blocks[0] else {
            panic!("a table first: {blocks:?}")
        };
        assert!(table.borders);
        assert_eq!(table.columns, vec![2000, 2000, 2000]);
        assert!(table.rows[0].header);
        assert_eq!(table.rows[0].cells.len(), 3);
        assert_eq!(table.rows[1].cells.len(), 2);
        assert_eq!(table.rows[1].cells[0].span, 2);
        let Block::Paragraph(after) = &blocks[1] else {
            panic!("then a paragraph")
        };
        assert_eq!(after.text(), "after");
    }

    #[test]
    fn a_written_document_reads_back_the_same() {
        let mut heading = Paragraph::with_text("第一章");
        heading.kind = ParagraphKind::Heading(1);
        let mut body = Paragraph {
            alignment: Some(Alignment::Justify),
            ..Paragraph::default()
        };
        body.push_text("日本語と ", &CharFormat::default());
        body.push_text(
            "bold {braces}",
            &CharFormat {
                bold: true,
                color: Some("00FF00".into()),
                size: Some(28),
                font: Some("Arial".into()),
                ..CharFormat::default()
            },
        );
        body.push_text(
            "site",
            &CharFormat {
                link: Some("https://example.com/".into()),
                ..CharFormat::default()
            },
        );
        let mut first = Paragraph::with_text("one");
        first.list = Some(ListItem {
            ordered: true,
            level: 0,
        });
        let mut bullet = Paragraph::with_text("dot");
        bullet.list = Some(ListItem {
            ordered: false,
            level: 1,
        });
        let png = {
            // A 1×1 PNG.
            let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            data.extend_from_slice(&1u32.to_be_bytes());
            data.extend_from_slice(&1u32.to_be_bytes());
            data.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
            data
        };
        let mut picture = Paragraph::default();
        picture
            .inlines
            .push(Inline::Image(Image::new(png.clone(), None).unwrap()));
        let table = Table {
            columns: vec![3000, 3000],
            rows: vec![
                Row {
                    cells: vec![Cell::with_text("a"), Cell::with_text("b")],
                    header: true,
                },
                Row {
                    cells: vec![Cell {
                        span: 2,
                        ..Cell::with_text("wide")
                    }],
                    header: false,
                },
            ],
            borders: true,
        };
        let document = FlowDocument {
            title: Some("題名".into()),
            sections: vec![
                FlowSection {
                    blocks: vec![
                        Block::Paragraph(heading),
                        Block::Paragraph(body),
                        Block::Paragraph(first),
                        Block::Paragraph(bullet),
                        Block::Paragraph(picture),
                        Block::Table(table),
                    ],
                    footer: vec![Block::Paragraph(Paragraph::with_text("footer"))],
                    ..FlowSection::default()
                },
                FlowSection {
                    blocks: vec![Block::Paragraph(Paragraph::with_text("second section"))],
                    page: PageSetup {
                        width: 16838,
                        height: 11906,
                        ..PageSetup::default()
                    },
                    ..FlowSection::default()
                },
            ],
            ..FlowDocument::default()
        };
        let rtf = write_rtf(&document);
        assert!(rtf.is_ascii(), "RTF is seven-bit");
        let read = read_rtf(rtf.as_bytes()).unwrap();
        assert_eq!(read.title.as_deref(), Some("題名"));
        assert_eq!(read.sections.len(), 2);
        assert!(read.sections[1].page.landscape());
        assert_eq!(read.sections[0].footer.len(), 1);
        let paragraphs = read.paragraphs();
        assert_eq!(paragraphs[0].kind, ParagraphKind::Heading(1));
        assert_eq!(paragraphs[0].text(), "第一章");
        assert_eq!(paragraphs[1].text(), "日本語と bold {braces}site");
        assert_eq!(paragraphs[1].alignment, Some(Alignment::Justify));
        let bold = paragraphs[1]
            .inlines
            .iter()
            .find_map(|inline| match inline {
                Inline::Text(text, format) if text.starts_with("bold") => Some(format.clone()),
                _ => None,
            })
            .unwrap();
        assert!(bold.bold);
        assert_eq!(bold.size, Some(28));
        assert_eq!(bold.font.as_deref(), Some("Arial"));
        assert_eq!(bold.color.as_deref(), Some("00FF00"));
        assert!(paragraphs[1].inlines.iter().any(|inline| matches!(
            inline,
            Inline::Text(text, format) if text == "site" && format.link.as_deref() == Some("https://example.com/")
        )));
        assert_eq!(
            paragraphs[2].list,
            Some(ListItem {
                ordered: true,
                level: 0
            })
        );
        assert_eq!(
            paragraphs[3].list,
            Some(ListItem {
                ordered: false,
                level: 1
            })
        );
        let image = paragraphs[4]
            .inlines
            .iter()
            .find_map(|inline| match inline {
                Inline::Image(image) => Some(image),
                _ => None,
            })
            .expect("the picture came back");
        assert_eq!(image.data, png);
        let table = read.sections[0]
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Table(table) => Some(table),
                _ => None,
            })
            .expect("the table came back");
        assert!(table.rows[0].header);
        assert_eq!(table.rows[1].cells[0].span, 2);
        assert_eq!(table.columns, vec![3000, 3000]);
    }
}
