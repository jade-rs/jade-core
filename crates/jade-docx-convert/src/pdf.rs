//! A PDF opened as an editable document, as Word's PDF 変換 (PDF Reflow)
//! opens one: the pages' text read with its positions, fonts and colours,
//! put back into lines and the lines into paragraphs — their alignment,
//! indents and headings worked out from where they sit — lists from their
//! bullets and numbers, simple ruled tables from their rules, pictures
//! where they are drawn, and what repeats at the top and bottom of every
//! page taken as the header and footer.
//!
//! A PDF has no paragraphs, only glyphs placed on pages; what comes back is
//! Word's guess at the document they were drawn from, as Word's is.

use std::collections::{BTreeMap, HashMap};

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::flow::*;

/// A run of text as the page draws it: its baseline's start in points from
/// the page's top-left, its width, size and look.
#[derive(Clone, Debug)]
struct TextPiece {
    x: f64,
    y: f64,
    width: f64,
    size: f64,
    text: String,
    font: String,
    bold: bool,
    italic: bool,
    color: Option<String>,
}

/// A picture as the page draws it.
#[derive(Clone, Debug)]
struct PicturePiece {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    image: Image,
}

/// A rule: a line drawn, or a thin filled rectangle, horizontal or vertical.
#[derive(Clone, Copy, Debug)]
struct Rule {
    horizontal: bool,
    /// y for a horizontal rule, x for a vertical one.
    at: f64,
    from: f64,
    to: f64,
}

#[derive(Default)]
struct PageContent {
    width: f64,
    height: f64,
    text: Vec<TextPiece>,
    pictures: Vec<PicturePiece>,
    rules: Vec<Rule>,
}

type Matrix = [f64; 6];

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn number(object: &Object) -> f64 {
    object.as_float().map(f64::from).unwrap_or(0.0)
}

/// A font's `/ToUnicode` CMap: each code — `code_bytes` long — to its text.
/// The PDF reference reads text through it before the font's encoding: a
/// subset font's codes (Word for Mac's, for one) are only its own numbering
/// of the glyphs it kept, whatever `/Encoding` names.
struct ToUnicode {
    code_bytes: usize,
    map: HashMap<u32, String>,
}

impl ToUnicode {
    fn parse(content: &[u8]) -> Option<Self> {
        let text = String::from_utf8_lossy(content);
        // Tokens: hex strings, array brackets and keywords.
        let mut tokens: Vec<String> = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '<' => {
                    let hex: String = chars.by_ref().take_while(|ch| *ch != '>').collect();
                    tokens.push(format!("<{}", hex.split_whitespace().collect::<String>()));
                }
                '[' | ']' => tokens.push(ch.to_string()),
                '%' => {
                    for ch in chars.by_ref() {
                        if ch == '\n' || ch == '\r' {
                            break;
                        }
                    }
                }
                ch if ch.is_whitespace() => {}
                ch => {
                    let mut word = ch.to_string();
                    while let Some(next) = chars.peek() {
                        if next.is_whitespace() || matches!(next, '<' | '[' | ']' | '/' | '%') {
                            break;
                        }
                        word.push(*next);
                        chars.next();
                    }
                    tokens.push(word);
                }
            }
        }
        let hex = |token: &str| -> Option<(u32, usize)> {
            let digits = token.strip_prefix('<')?;
            Some((u32::from_str_radix(digits, 16).ok()?, digits.len() / 2))
        };
        let utf16 = |token: &str| -> Option<String> {
            let digits = token.strip_prefix('<')?;
            let units: Vec<u16> = (0..digits.len() / 4)
                .filter_map(|at| u16::from_str_radix(digits.get(at * 4..at * 4 + 4)?, 16).ok())
                .collect();
            Some(String::from_utf16_lossy(&units))
        };
        let mut map = HashMap::new();
        let mut code_bytes = 0;
        let mut at = 0;
        let mut mode = "";
        while at < tokens.len() {
            let token = tokens[at].as_str();
            match token {
                "begincodespacerange" | "beginbfchar" | "beginbfrange" => {
                    mode = token;
                    at += 1;
                    continue;
                }
                "endcodespacerange" | "endbfchar" | "endbfrange" => {
                    mode = "";
                    at += 1;
                    continue;
                }
                _ => {}
            }
            match mode {
                "begincodespacerange" => {
                    if let Some((_, bytes)) = hex(token) {
                        code_bytes = code_bytes.max(bytes);
                    }
                    at += 1;
                }
                "beginbfchar" => {
                    if let (Some((code, bytes)), Some(text)) =
                        (hex(token), tokens.get(at + 1).and_then(|next| utf16(next)))
                    {
                        code_bytes = code_bytes.max(bytes);
                        map.insert(code, text);
                    }
                    at += 2;
                }
                "beginbfrange" => {
                    let (Some((low, bytes)), Some((high, _))) =
                        (hex(token), tokens.get(at + 1).and_then(|next| hex(next)))
                    else {
                        at += 1;
                        continue;
                    };
                    code_bytes = code_bytes.max(bytes);
                    let high = high.min(low + 0xFFFF);
                    if tokens.get(at + 2).map(String::as_str) == Some("[") {
                        let mut index = at + 3;
                        let mut code = low;
                        while index < tokens.len() && tokens[index] != "]" {
                            if let Some(text) = utf16(&tokens[index]) {
                                if code <= high {
                                    map.insert(code, text);
                                }
                                code += 1;
                            }
                            index += 1;
                        }
                        at = index + 1;
                    } else if let Some(start) = tokens.get(at + 2).and_then(|next| utf16(next)) {
                        // Consecutive codes: the destination's last unit counts up.
                        let mut units: Vec<u16> = start.encode_utf16().collect();
                        for code in low..=high {
                            map.insert(code, String::from_utf16_lossy(&units));
                            if let Some(last) = units.last_mut() {
                                *last = last.wrapping_add(1);
                            }
                        }
                        at += 3;
                    } else {
                        at += 3;
                    }
                }
                _ => at += 1,
            }
        }
        (!map.is_empty()).then_some(Self {
            code_bytes: code_bytes.clamp(1, 4),
            map,
        })
    }
}

/// A font as the text in it is decoded and measured.
struct FontInfo<'a> {
    to_unicode: Option<ToUnicode>,
    encoding: Option<lopdf::Encoding<'a>>,
    /// Two-byte codes (a CID font).
    two_byte: bool,
    widths: HashMap<u32, f64>,
    default_width: f64,
    name: String,
    bold: bool,
    italic: bool,
}

impl FontInfo<'_> {
    fn decode(&self, bytes: &[u8]) -> String {
        if let Some(cmap) = &self.to_unicode {
            let width = if self.two_byte { 2 } else { cmap.code_bytes };
            let mut text = String::new();
            let mut unmapped = false;
            for chunk in bytes.chunks(width) {
                let code = chunk
                    .iter()
                    .fold(0u32, |code, byte| (code << 8) | *byte as u32);
                match cmap.map.get(&code) {
                    Some(mapped) => text.push_str(mapped),
                    None => unmapped = true,
                }
            }
            if !unmapped {
                return text;
            }
        }
        if let Some(encoding) = &self.encoding {
            if let Ok(text) = encoding.bytes_to_string(bytes) {
                return text;
            }
        }
        if self.two_byte {
            let units: Vec<u16> = bytes
                .chunks(2)
                .map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]))
                .collect();
            return String::from_utf16_lossy(&units);
        }
        bytes.iter().map(|byte| *byte as char).collect()
    }

    /// The width of `bytes` in text space units (thousandths of the size),
    /// with how many single-byte spaces it has (for word spacing).
    fn width(&self, bytes: &[u8]) -> (f64, usize, usize) {
        let mut total = 0.0;
        let mut spaces = 0;
        let mut codes = 0;
        if self.two_byte {
            for pair in bytes.chunks(2) {
                let code = u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]) as u32;
                total += self
                    .widths
                    .get(&code)
                    .copied()
                    .unwrap_or(self.default_width);
                codes += 1;
            }
        } else {
            for byte in bytes {
                total += self
                    .widths
                    .get(&(*byte as u32))
                    .copied()
                    .unwrap_or(self.default_width);
                if *byte == 32 {
                    spaces += 1;
                }
                codes += 1;
            }
        }
        (total, spaces, codes)
    }
}

fn font_info<'a>(document: &'a Document, font: &'a Dictionary) -> FontInfo<'a> {
    let base = font
        .get(b"BaseFont")
        .and_then(Object::as_name)
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .unwrap_or_default();
    // "ABCDEF+Family-Bold": the subset tag off, the style read from the rest.
    let without_tag = match base.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|ch| ch.is_ascii_uppercase()) => {
            rest.to_string()
        }
        _ => base.clone(),
    };
    let lower = without_tag.to_ascii_lowercase();
    let bold = [
        "bold", "black", "heavy", "semibold", "demibold", "-w6", "-w7", "-w8", "-w9",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    let italic = lower.contains("italic") || lower.contains("oblique");
    let family = without_tag
        .split([',', '-'])
        .next()
        .unwrap_or(&without_tag)
        .trim()
        .to_string();
    let subtype = font
        .get(b"Subtype")
        .and_then(Object::as_name)
        .unwrap_or(b"");
    let two_byte = subtype == b"Type0";
    let mut widths = HashMap::new();
    let mut default_width = 500.0;
    if two_byte {
        if let Ok(descendant) = font
            .get_deref(b"DescendantFonts", document)
            .and_then(Object::as_array)
            .and_then(|array| {
                array
                    .first()
                    .ok_or(lopdf::Error::DictKey("DescendantFonts".into()))
            })
            .and_then(|object| document.dereference(object).map(|(_, object)| object))
            .and_then(Object::as_dict)
        {
            default_width = descendant.get(b"DW").map(number).unwrap_or(1000.0);
            if let Ok(array) = descendant
                .get_deref(b"W", document)
                .and_then(Object::as_array)
            {
                let mut at = 0;
                while at + 1 < array.len() {
                    let first = number(&array[at]) as u32;
                    match document
                        .dereference(&array[at + 1])
                        .map(|(_, object)| object)
                    {
                        Ok(Object::Array(list)) => {
                            for (index, width) in list.iter().enumerate() {
                                widths.insert(first + index as u32, number(width));
                            }
                            at += 2;
                        }
                        _ if at + 2 < array.len() => {
                            let last = number(&array[at + 1]) as u32;
                            let width = number(&array[at + 2]);
                            for code in first..=last.min(first + 65535) {
                                widths.insert(code, width);
                            }
                            at += 3;
                        }
                        _ => break,
                    }
                }
            }
        }
    } else {
        let first = font.get(b"FirstChar").map(number).unwrap_or(0.0) as u32;
        if let Ok(list) = font
            .get_deref(b"Widths", document)
            .and_then(Object::as_array)
        {
            for (index, width) in list.iter().enumerate() {
                let width = document
                    .dereference(width)
                    .map(|(_, object)| number(object))
                    .unwrap_or(0.0);
                widths.insert(first + index as u32, width);
            }
        }
        if let Some(average) =
            (!widths.is_empty()).then(|| widths.values().sum::<f64>() / widths.len() as f64)
        {
            default_width = average.max(250.0);
        }
    }
    let to_unicode = font
        .get_deref(b"ToUnicode", document)
        .and_then(Object::as_stream)
        .ok()
        .and_then(|stream| stream.get_plain_content().ok())
        .and_then(|content| ToUnicode::parse(&content));
    FontInfo {
        to_unicode,
        encoding: font.get_font_encoding(document).ok(),
        two_byte,
        widths,
        default_width,
        name: family,
        bold,
        italic,
    }
}

#[derive(Clone)]
struct TextState {
    font: Option<Vec<u8>>,
    size: f64,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
    rise: f64,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            font: None,
            size: 12.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            scale: 1.0,
            leading: 0.0,
            rise: 0.0,
        }
    }
}

#[derive(Clone)]
struct GraphicsState {
    ctm: Matrix,
    text: TextState,
    fill: Option<String>,
}

struct Interpreter<'a> {
    document: &'a Document,
    page_height: f64,
    out: PageContent,
    fonts: HashMap<(ObjectId, Vec<u8>), FontInfo<'a>>,
    depth: usize,
}

fn rgb(r: f64, g: f64, b: f64) -> Option<String> {
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let color = format!("{:02X}{:02X}{:02X}", channel(r), channel(g), channel(b));
    (color != "000000").then_some(color)
}

impl<'a> Interpreter<'a> {
    fn run(
        &mut self,
        content: &[u8],
        resources: Option<&'a Dictionary>,
        resources_id: ObjectId,
        base: Matrix,
    ) {
        let Ok(content) = Content::decode(content) else {
            return;
        };
        let mut state = GraphicsState {
            ctm: base,
            text: TextState::default(),
            fill: None,
        };
        let mut stack: Vec<GraphicsState> = Vec::new();
        let mut text_matrix = IDENTITY;
        let mut line_matrix = IDENTITY;
        let mut path: Vec<(f64, f64)> = Vec::new();
        let mut segments: Vec<((f64, f64), (f64, f64))> = Vec::new();
        let mut rectangles: Vec<(f64, f64, f64, f64)> = Vec::new();
        for operation in &content.operations {
            let operands = &operation.operands;
            let n = |index: usize| operands.get(index).map(number).unwrap_or(0.0);
            match operation.operator.as_str() {
                "q" => stack.push(state.clone()),
                "Q" => {
                    if let Some(previous) = stack.pop() {
                        state = previous;
                    }
                }
                "cm" => {
                    let m = [n(0), n(1), n(2), n(3), n(4), n(5)];
                    state.ctm = multiply(&m, &state.ctm);
                }
                "rg" => state.fill = rgb(n(0), n(1), n(2)),
                "g" => state.fill = rgb(n(0), n(0), n(0)),
                "k" => {
                    let k = n(3);
                    state.fill = rgb(
                        (1.0 - n(0)) * (1.0 - k),
                        (1.0 - n(1)) * (1.0 - k),
                        (1.0 - n(2)) * (1.0 - k),
                    );
                }
                "sc" | "scn" if operands.len() == 3 => state.fill = rgb(n(0), n(1), n(2)),
                "BT" => {
                    text_matrix = IDENTITY;
                    line_matrix = IDENTITY;
                }
                "Tf" => {
                    state.text.font = operands
                        .first()
                        .and_then(|o| o.as_name().ok())
                        .map(<[u8]>::to_vec);
                    state.text.size = n(1);
                }
                "Tc" => state.text.char_spacing = n(0),
                "Tw" => state.text.word_spacing = n(0),
                "Tz" => state.text.scale = n(0) / 100.0,
                "TL" => state.text.leading = n(0),
                "Ts" => state.text.rise = n(0),
                "Td" => {
                    line_matrix = multiply(&[1.0, 0.0, 0.0, 1.0, n(0), n(1)], &line_matrix);
                    text_matrix = line_matrix;
                }
                "TD" => {
                    state.text.leading = -n(1);
                    line_matrix = multiply(&[1.0, 0.0, 0.0, 1.0, n(0), n(1)], &line_matrix);
                    text_matrix = line_matrix;
                }
                "Tm" => {
                    line_matrix = [n(0), n(1), n(2), n(3), n(4), n(5)];
                    text_matrix = line_matrix;
                }
                "T*" => {
                    line_matrix = multiply(
                        &[1.0, 0.0, 0.0, 1.0, 0.0, -state.text.leading],
                        &line_matrix,
                    );
                    text_matrix = line_matrix;
                }
                "Tj" | "'" | "\"" | "TJ" => {
                    if operation.operator == "'" || operation.operator == "\"" {
                        if operation.operator == "\"" {
                            state.text.word_spacing = n(0);
                            state.text.char_spacing = n(1);
                        }
                        line_matrix = multiply(
                            &[1.0, 0.0, 0.0, 1.0, 0.0, -state.text.leading],
                            &line_matrix,
                        );
                        text_matrix = line_matrix;
                    }
                    let items: Vec<&Object> = match operation.operator.as_str() {
                        "TJ" => operands
                            .first()
                            .and_then(|o| o.as_array().ok())
                            .map(|a| a.iter().collect())
                            .unwrap_or_default(),
                        _ => operands.last().into_iter().collect(),
                    };
                    self.show_text(&items, &state, &mut text_matrix, resources, resources_id);
                }
                "Do" => {
                    let Some(name) = operands.first().and_then(|o| o.as_name().ok()) else {
                        continue;
                    };
                    self.draw_xobject(name, &state, resources, resources_id);
                }
                "m" => {
                    path.clear();
                    path.push(apply(&state.ctm, n(0), n(1)));
                }
                "l" => {
                    let point = apply(&state.ctm, n(0), n(1));
                    if let Some(last) = path.last().copied() {
                        segments.push((last, point));
                    }
                    path.push(point);
                }
                "re" => {
                    let (x0, y0) = apply(&state.ctm, n(0), n(1));
                    let (x1, y1) = apply(&state.ctm, n(0) + n(2), n(1) + n(3));
                    rectangles.push((x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)));
                }
                "S" | "s" | "B" | "B*" | "b" | "b*" => {
                    for ((x0, y0), (x1, y1)) in segments.drain(..) {
                        self.add_segment(x0, y0, x1, y1);
                    }
                    for (x0, y0, x1, y1) in rectangles.drain(..) {
                        // A stroked rectangle is four rules.
                        self.add_segment(x0, y0, x1, y0);
                        self.add_segment(x0, y1, x1, y1);
                        self.add_segment(x0, y0, x0, y1);
                        self.add_segment(x1, y0, x1, y1);
                    }
                    path.clear();
                }
                "f" | "F" | "f*" => {
                    // A thin filled rectangle is a rule; a cell's shading is not.
                    for (x0, y0, x1, y1) in rectangles.drain(..) {
                        if (y1 - y0) < 2.5 && (x1 - x0) > 4.0 {
                            self.add_segment(x0, (y0 + y1) / 2.0, x1, (y0 + y1) / 2.0);
                        } else if (x1 - x0) < 2.5 && (y1 - y0) > 4.0 {
                            self.add_segment((x0 + x1) / 2.0, y0, (x0 + x1) / 2.0, y1);
                        }
                    }
                    segments.clear();
                    path.clear();
                }
                "n" => {
                    segments.clear();
                    rectangles.clear();
                    path.clear();
                }
                _ => {}
            }
        }
    }

    fn add_segment(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        let (top0, top1) = (self.page_height - y0, self.page_height - y1);
        if (y0 - y1).abs() < 1.0 && (x1 - x0).abs() > 3.0 {
            self.out.rules.push(Rule {
                horizontal: true,
                at: (top0 + top1) / 2.0,
                from: x0.min(x1),
                to: x0.max(x1),
            });
        } else if (x0 - x1).abs() < 1.0 && (y1 - y0).abs() > 3.0 {
            self.out.rules.push(Rule {
                horizontal: false,
                at: (x0 + x1) / 2.0,
                from: top0.min(top1),
                to: top0.max(top1),
            });
        }
    }

    fn font(
        &mut self,
        resources: Option<&'a Dictionary>,
        resources_id: ObjectId,
        name: &[u8],
    ) -> Option<&FontInfo<'a>> {
        let key = (resources_id, name.to_vec());
        if !self.fonts.contains_key(&key) {
            let fonts = resources?
                .get_deref(b"Font", self.document)
                .ok()?
                .as_dict()
                .ok()?;
            let font = fonts.get_deref(name, self.document).ok()?.as_dict().ok()?;
            let info = font_info(self.document, font);
            self.fonts.insert(key.clone(), info);
        }
        self.fonts.get(&key)
    }

    fn show_text(
        &mut self,
        items: &[&Object],
        state: &GraphicsState,
        text_matrix: &mut Matrix,
        resources: Option<&'a Dictionary>,
        resources_id: ObjectId,
    ) {
        let Some(font_name) = state.text.font.clone() else {
            return;
        };
        let text_state = state.text.clone();
        let ctm = state.ctm;
        let fill = state.fill.clone();
        let page_height = self.page_height;
        let size = text_state.size;
        let start = multiply(
            &[
                size * text_state.scale,
                0.0,
                0.0,
                size,
                0.0,
                text_state.rise,
            ],
            &multiply(text_matrix, &ctm),
        );
        let (x, y) = (start[4], start[5]);
        let effective_size = (start[2] * start[2] + start[3] * start[3]).sqrt();
        let mut text = String::new();
        let mut advance = 0.0;
        let Some(font) = self.font(resources, resources_id, &font_name) else {
            return;
        };
        let (name, bold, italic) = (font.name.clone(), font.bold, font.italic);
        for item in items {
            match item {
                Object::String(bytes, _) => {
                    let (width, spaces, codes) = font.width(bytes);
                    let piece = width / 1000.0 * size
                        + text_state.char_spacing * codes as f64
                        + text_state.word_spacing * spaces as f64;
                    advance += piece * text_state.scale;
                    text.push_str(&font.decode(bytes));
                }
                Object::Integer(_) | Object::Real(_) => {
                    let adjustment = number(item);
                    advance -= adjustment / 1000.0 * size * text_state.scale;
                    // A wide gap is a space the PDF left out.
                    if adjustment < -250.0 && !text.ends_with(' ') && !text.is_empty() {
                        text.push(' ');
                    }
                }
                _ => {}
            }
        }
        // Where the text ends, through the same matrices.
        let end = apply(&multiply(text_matrix, &ctm), advance, 0.0);
        let width = ((end.0 - x).powi(2) + (end.1 - y).powi(2)).sqrt();
        *text_matrix = multiply(&[1.0, 0.0, 0.0, 1.0, advance, 0.0], text_matrix);
        let text: String = text
            .chars()
            .filter(|ch| !ch.is_control() || *ch == '\t')
            .collect();
        if text.trim().is_empty() && width < effective_size * 0.2 {
            return;
        }
        self.out.text.push(TextPiece {
            x,
            y: page_height - y,
            width,
            size: effective_size.max(1.0),
            text,
            font: name,
            bold,
            italic,
            color: fill,
        });
    }

    fn draw_xobject(
        &mut self,
        name: &[u8],
        state: &GraphicsState,
        resources: Option<&'a Dictionary>,
        resources_id: ObjectId,
    ) {
        let Some(xobjects) = resources
            .and_then(|r| r.get_deref(b"XObject", self.document).ok())
            .and_then(|o| o.as_dict().ok())
        else {
            return;
        };
        let Ok(reference) = xobjects.get(name) else {
            return;
        };
        let Ok((id, object)) = self.document.dereference(reference) else {
            return;
        };
        let Ok(stream) = object.as_stream() else {
            return;
        };
        let subtype = stream
            .dict
            .get(b"Subtype")
            .and_then(Object::as_name)
            .unwrap_or(b"");
        if subtype == b"Image" {
            if let Some(image) = image_of(self.document, stream) {
                let (x0, y0) = apply(&state.ctm, 0.0, 0.0);
                let (x1, y1) = apply(&state.ctm, 1.0, 1.0);
                let (width, height) = ((x1 - x0).abs(), (y1 - y0).abs());
                if width > 2.0 && height > 2.0 {
                    self.out.pictures.push(PicturePiece {
                        x: x0.min(x1),
                        y: self.page_height - y0.max(y1),
                        width,
                        height,
                        image,
                    });
                }
            }
        } else if subtype == b"Form" && self.depth < 6 {
            let matrix = stream
                .dict
                .get(b"Matrix")
                .and_then(Object::as_array)
                .map(|m| {
                    let v: Vec<f64> = m.iter().map(number).collect();
                    if v.len() == 6 {
                        [v[0], v[1], v[2], v[3], v[4], v[5]]
                    } else {
                        IDENTITY
                    }
                })
                .unwrap_or(IDENTITY);
            let form_resources = stream
                .dict
                .get_deref(b"Resources", self.document)
                .ok()
                .and_then(|o| o.as_dict().ok())
                .or(resources);
            if let Ok(content) = stream.get_plain_content() {
                self.depth += 1;
                let base = multiply(&matrix, &state.ctm);
                let resources_id = id.unwrap_or(resources_id);
                self.run(&content, form_resources, resources_id, base);
                self.depth -= 1;
            }
        }
    }
}

/// A picture's bytes as a file: a JPEG as it is stored, an 8-bit RGB or
/// grey bitmap as a PNG.
fn image_of(document: &Document, stream: &lopdf::Stream) -> Option<Image> {
    let filters: Vec<Vec<u8>> = match stream.dict.get(b"Filter") {
        Ok(Object::Name(name)) => vec![name.clone()],
        Ok(Object::Array(list)) => list
            .iter()
            .filter_map(|o| o.as_name().ok().map(<[u8]>::to_vec))
            .collect(),
        _ => Vec::new(),
    };
    if filters.last().is_some_and(|filter| filter == b"DCTDecode") {
        return Image::new(stream.content.clone(), None);
    }
    if !filters.iter().all(|filter| filter == b"FlateDecode") {
        return None;
    }
    let width = stream.dict.get(b"Width").ok().map(number)? as u32;
    let height = stream.dict.get(b"Height").ok().map(number)? as u32;
    let bits = stream
        .dict
        .get(b"BitsPerComponent")
        .map(number)
        .unwrap_or(8.0) as u32;
    let space = stream
        .dict
        .get_deref(b"ColorSpace", document)
        .ok()
        .and_then(|o| o.as_name().ok())
        .unwrap_or(b"DeviceRGB");
    let channels = match space {
        b"DeviceRGB" => 3,
        b"DeviceGray" => 1,
        _ => return None,
    };
    if bits != 8 || width == 0 || height == 0 {
        return None;
    }
    let pixels = stream.get_plain_content().ok()?;
    if pixels.len() < (width * height * channels) as usize {
        return None;
    }
    Image::new(png(width, height, channels, &pixels), None)
}

/// A PNG of 8-bit `channels`-channel pixels.
fn png(width: u32, height: u32, channels: u32, pixels: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let row = (width * channels) as usize;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for line in pixels.chunks(row).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw).ok();
    let compressed = encoder.finish().unwrap_or_default();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let chunk = |out: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc_data = kind.to_vec();
        crc_data.extend_from_slice(data);
        out.extend_from_slice(&crc_data);
        out.extend_from_slice(&crc32(&crc_data).to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, if channels == 3 { 2 } else { 0 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &compressed);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn page_content(document: &Document, page_id: ObjectId) -> PageContent {
    let page = document.get_dictionary(page_id).ok();
    let media_box: Vec<f64> = page
        .and_then(|page| page.get(b"MediaBox").ok())
        .and_then(|object| object.as_array().ok())
        .map(|array| array.iter().map(number).collect())
        .unwrap_or_else(|| vec![0.0, 0.0, 612.0, 792.0]);
    let (left, bottom) = (
        media_box.first().copied().unwrap_or(0.0),
        media_box.get(1).copied().unwrap_or(0.0),
    );
    let width = media_box.get(2).copied().unwrap_or(612.0) - left;
    let height = media_box.get(3).copied().unwrap_or(792.0) - bottom;
    // The page's resources, inline or referenced (and inherited).
    let (inline, referenced) = document
        .get_page_resources(page_id)
        .unwrap_or((None, Vec::new()));
    let resources = inline.or_else(|| {
        referenced
            .iter()
            .find_map(|id| document.get_dictionary(*id).ok())
    });
    let content = document.get_page_content(page_id).unwrap_or_default();
    let mut interpreter = Interpreter {
        document,
        page_height: height,
        out: PageContent {
            width,
            height,
            ..PageContent::default()
        },
        fonts: HashMap::new(),
        depth: 0,
    };
    interpreter.run(
        &content,
        resources,
        page_id,
        [1.0, 0.0, 0.0, 1.0, -left, -bottom],
    );
    interpreter.out
}

// ---- lines and paragraphs ------------------------------------------------------

/// A line of text on a page.
#[derive(Clone, Debug)]
struct Line {
    x0: f64,
    x1: f64,
    y: f64,
    size: f64,
    pieces: Vec<TextPiece>,
}

impl Line {
    fn text(&self) -> String {
        self.pieces
            .iter()
            .map(|piece| piece.text.as_str())
            .collect()
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3000..=0x30ff | 0x3400..=0x9fff | 0xff00..=0xffef)
}

/// The pieces in lines: those on one baseline, left to right, with a space
/// where a gap is a word's.
fn lines_of(pieces: Vec<TextPiece>) -> Vec<Line> {
    // 影付き and 中抜き text is drawn twice, the copy a little off: one of
    // each is the text.
    let mut kept: HashMap<String, Vec<(f64, f64, f64)>> = HashMap::new();
    let mut pieces: Vec<TextPiece> = pieces
        .into_iter()
        .filter(|piece| {
            if piece.text.trim().is_empty() {
                return true;
            }
            let near = piece.size.max(1.0) * 0.2;
            let drawn = kept.entry(piece.text.clone()).or_default();
            if drawn.iter().any(|&(x, y, size)| {
                (size - piece.size).abs() < 0.5
                    && (x - piece.x).abs() < near
                    && (y - piece.y).abs() < near
            }) {
                return false;
            }
            drawn.push((piece.x, piece.y, piece.size));
            true
        })
        .collect();
    pieces.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    let mut lines: Vec<Line> = Vec::new();
    for piece in pieces {
        let same = lines
            .iter_mut()
            .rev()
            .take(3)
            .find(|line| (line.y - piece.y).abs() < piece.size.min(line.size) * 0.35);
        match same {
            Some(line) => {
                line.pieces.push(piece);
            }
            None => lines.push(Line {
                x0: piece.x,
                x1: piece.x + piece.width,
                y: piece.y,
                size: piece.size,
                pieces: vec![piece],
            }),
        }
    }
    for line in &mut lines {
        line.pieces.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut merged: Vec<TextPiece> = Vec::new();
        for piece in std::mem::take(&mut line.pieces) {
            if let Some(last) = merged.last_mut() {
                let gap = piece.x - (last.x + last.width);
                let joins_cjk = last.text.chars().last().is_some_and(is_cjk)
                    || piece.text.chars().next().is_some_and(is_cjk);
                let needs_space = gap > piece.size.min(last.size) * 0.22
                    && !last.text.ends_with(' ')
                    && !piece.text.starts_with(' ')
                    && !joins_cjk;
                let same_look = last.font == piece.font
                    && last.bold == piece.bold
                    && last.italic == piece.italic
                    && last.color == piece.color
                    && (last.size - piece.size).abs() < 0.5;
                if same_look && gap < piece.size * 3.0 {
                    if needs_space {
                        last.text.push(' ');
                    }
                    last.text.push_str(&piece.text);
                    last.width = piece.x + piece.width - last.x;
                    continue;
                }
                if needs_space {
                    last.text.push(' ');
                }
            }
            merged.push(piece);
        }
        line.pieces = merged;
        line.x0 = line.pieces.first().map(|p| p.x).unwrap_or(line.x0);
        line.x1 = line
            .pieces
            .iter()
            .map(|p| p.x + p.width)
            .fold(line.x0, f64::max);
        // The size most of the line is in.
        let mut weights: Vec<(f64, usize)> = Vec::new();
        for piece in &line.pieces {
            let count = piece.text.chars().count();
            match weights
                .iter_mut()
                .find(|(size, _)| (size - piece.size).abs() < 0.3)
            {
                Some(entry) => entry.1 += count,
                None => weights.push((piece.size, count)),
            }
        }
        line.size = weights
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(size, _)| *size)
            .unwrap_or(line.size);
    }
    lines.retain(|line| !line.text().trim().is_empty());
    lines
}

/// The most common value of `values`, rounded to `step`.
fn mode(values: impl Iterator<Item = f64>, step: f64) -> Option<f64> {
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    for value in values {
        *counts.entry((value / step).round() as i64).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(value, _)| value as f64 * step)
}

/// The bullet or number a list item starts with, and what it is.
fn list_marker(text: &str) -> Option<(usize, bool)> {
    let trimmed = text.trim_start();
    let skipped = text.len() - trimmed.len();
    let mut chars = trimmed.char_indices();
    let (_, first) = chars.next()?;
    if matches!(
        first,
        '•' | '・' | '●' | '○' | '■' | '□' | '◆' | '▪' | '‣' | '–' | '-' | '*'
    ) {
        let rest = &trimmed[first.len_utf8()..];
        return rest
            .chars()
            .next()
            .filter(|ch| matches!(ch, ' ' | '\t' | '\u{3000}'))
            .map(|separator| (skipped + first.len_utf8() + separator.len_utf8(), false));
    }
    // 1. 1) (1) a. (a)
    let bracketed = trimmed.starts_with('(') || trimmed.starts_with('（');
    let body = trimmed.trim_start_matches(['(', '（']);
    let label: String = body
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .collect();
    if label.is_empty()
        || label.len() > 3
        || (!label.chars().all(|ch| ch.is_ascii_digit()) && label.len() > 1)
    {
        return None;
    }
    let after = &body[label.len()..];
    let close = after.chars().next()?;
    let closes = matches!(
        (bracketed, close),
        (true, ')' | '）') | (false, '.' | ')' | '）' | '．')
    );
    let rest = &after[close.len_utf8()..];
    let separator = rest
        .chars()
        .next()
        .filter(|ch| matches!(ch, ' ' | '\t' | '\u{3000}'));
    if closes {
        if let Some(separator) = separator {
            let length = text.len() - rest.len() + separator.len_utf8();
            return Some((length.min(text.len()), true));
        }
    }
    None
}

/// The pages' lines as paragraphs.
fn paragraphs_of(
    pages: &[(Vec<Line>, f64, f64)],
    body_size: f64,
    margins: (f64, f64),
) -> Vec<(usize, Paragraph, f64)> {
    let (left, right) = margins;
    let width = (right - left).max(1.0);
    let mut out: Vec<(usize, Paragraph, f64)> = Vec::new();
    let mut heading_sizes: Vec<f64> = pages
        .iter()
        .flat_map(|(lines, _, _)| lines.iter())
        .map(|line| (line.size * 2.0).round() / 2.0)
        .filter(|size| *size >= body_size * 1.25)
        .collect();
    heading_sizes.sort_by(|a, b| b.total_cmp(a));
    heading_sizes.dedup();
    let mut previous: Option<Line> = None;
    // Each paragraph's lines' left and right ends, to tell its alignment.
    let mut geometry: Vec<Vec<(f64, f64)>> = Vec::new();
    for (page_index, (lines, _, _)) in pages.iter().enumerate() {
        let spacing = mode(
            lines
                .windows(2)
                .map(|pair| pair[1].y - pair[0].y)
                .filter(|gap| *gap > 0.0),
            0.5,
        )
        .unwrap_or(body_size * 1.4);
        for (index, line) in lines.iter().enumerate() {
            let gap = if index == 0 {
                f64::INFINITY
            } else {
                line.y - lines[index - 1].y
            };
            let starts_new = match &previous {
                None => true,
                Some(prev) => {
                    let prev_full = prev.x1 > right - (width * 0.12).max(prev.size * 3.0);
                    let same_size = (prev.size - line.size).abs() < 0.6;
                    let indented =
                        line.x0 > left + line.size * 0.8 && prev.x0 <= left + line.size * 0.5;
                    let continues_page = index == 0 && prev_full && same_size && !indented;
                    !(continues_page
                        || (gap <= spacing * 1.35
                            && prev_full
                            && same_size
                            && !indented
                            && list_marker(&line.text()).is_none()))
                }
            };
            if starts_new {
                let mut paragraph = Paragraph::default();
                let size = (line.size * 2.0).round() / 2.0;
                if let Some(level) = heading_sizes
                    .iter()
                    .position(|heading| (heading - size).abs() < 0.01)
                {
                    if line.text().chars().count() < 150 {
                        paragraph.kind = ParagraphKind::Heading((level as u8 + 1).min(9));
                    }
                }
                if let Some((_, ordered)) = list_marker(&line.text()) {
                    if paragraph.kind == ParagraphKind::Normal {
                        paragraph.list = Some(ListItem {
                            ordered,
                            level: ((line.x0 - left) / (body_size * 2.0)).max(0.0) as u8,
                        });
                    }
                }
                if index > 0 && gap > spacing * 1.35 {
                    paragraph.space_before = Some(((gap - spacing).max(0.0) * 20.0).round() as u32);
                }
                out.push((page_index, paragraph, line.y));
                geometry.push(Vec::new());
                // Where the paragraph starts tells its first-line indent.
                let paragraph = &mut out.last_mut().expect("just pushed").1;
                paragraph.indent_first_line = ((line.x0 - left) * 20.0).round() as i32;
            } else {
                let paragraph = &mut out.last_mut().expect("a paragraph is open").1;
                // A second line says where the paragraph's left edge is.
                let first = paragraph.indent_first_line;
                let indent = ((line.x0 - left) * 20.0).round() as i32;
                if paragraph
                    .inlines
                    .iter()
                    .all(|inline| !matches!(inline, Inline::LineBreak))
                    && paragraph.indent_left == 0
                {
                    paragraph.indent_left = indent.max(0);
                    paragraph.indent_first_line = first - indent;
                }
                // Continued: the line break was the page's, not the text's.
                let last_char = paragraph.text().chars().last();
                let next_char = line.text().chars().next();
                let joins = last_char.is_some_and(|ch| is_cjk(ch) || ch == '-')
                    || next_char.is_some_and(is_cjk);
                if !joins {
                    paragraph.push_text(" ", &CharFormat::default());
                }
            }
            let paragraph = &mut out.last_mut().expect("a paragraph is open").1;
            // A list's marker is the list's, not the text's: its characters
            // are left out, across however many pieces they were drawn in.
            let mut skip = if paragraph.list.is_some() && paragraph.is_empty() {
                list_marker(&line.text())
                    .map(|(length, _)| line.text()[..length].chars().count())
                    .unwrap_or(0)
            } else {
                0
            };
            for piece in &line.pieces {
                let format = CharFormat {
                    bold: piece.bold,
                    italic: piece.italic,
                    size: Some((piece.size * 2.0).round() as u32)
                        .filter(|size| (*size as f64 - body_size * 2.0).abs() > 0.9),
                    font: Some(piece.font.clone()).filter(|font| !font.is_empty()),
                    color: piece.color.clone(),
                    ..CharFormat::default()
                };
                let count = piece.text.chars().count();
                let text: String = piece.text.chars().skip(skip).collect();
                skip = skip.saturating_sub(count);
                paragraph.push_text(&text, &format);
            }
            if let Some(lines) = geometry.last_mut() {
                lines.push((line.x0, line.x1));
            }
            previous = Some(line.clone());
        }
    }
    // Alignment, from where the lines sit: clear of both margins by the same
    // amount, centred; against the right edge but not starting together on
    // the left, right-aligned; every line but the last across the full
    // width, justified.
    for ((_, paragraph, _), lines) in out.iter_mut().zip(&geometry) {
        let size = 10.0f64;
        let centred = lines.iter().all(|(x0, x1)| {
            let (left_gap, right_gap) = (x0 - left, right - x1);
            left_gap > width * 0.05 && (left_gap - right_gap).abs() < size * 0.6
        });
        let lefts_differ = lines
            .windows(2)
            .any(|pair| (pair[0].0 - pair[1].0).abs() > 3.0);
        let righted = lines.iter().all(|(_, x1)| right - x1 < size * 0.6)
            && if lines.len() == 1 {
                lines[0].0 > left + width * 0.3
            } else {
                lefts_differ
            };
        let justified = lines.len() > 2
            && lines[..lines.len() - 1]
                .iter()
                .all(|(_, x1)| right - x1 < size * 0.6)
            && lines.last().is_some_and(|(_, x1)| right - x1 > size);
        paragraph.alignment = if centred {
            Some(Alignment::Center)
        } else if righted {
            Some(Alignment::Right)
        } else if justified {
            Some(Alignment::Justify)
        } else {
            None
        };
        if centred || righted {
            paragraph.indent_left = 0;
            paragraph.indent_first_line = 0;
        }
    }
    for (_, paragraph, _) in &mut out {
        if paragraph.list.is_some() || paragraph.kind != ParagraphKind::Normal {
            paragraph.indent_left = 0;
            paragraph.indent_first_line = 0;
        }
        trim_paragraph(paragraph);
    }
    out
}

fn trim_paragraph(paragraph: &mut Paragraph) {
    if let Some(Inline::Text(text, _)) = paragraph.inlines.first_mut() {
        *text = text.trim_start().to_string();
    }
    if let Some(Inline::Text(text, _)) = paragraph.inlines.last_mut() {
        *text = text.trim_end().to_string();
    }
    paragraph
        .inlines
        .retain(|inline| !matches!(inline, Inline::Text(text, _) if text.is_empty()));
}

// ---- tables -------------------------------------------------------------------------

struct FoundTable {
    top: f64,
    bottom: f64,
    left: f64,
    right: f64,
    xs: Vec<f64>,
    ys: Vec<f64>,
    horizontals: Vec<Rule>,
    verticals: Vec<Rule>,
}

fn cluster(values: &mut Vec<f64>, tolerance: f64) {
    values.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::new();
    for value in values.drain(..) {
        match out.last() {
            Some(last) if (value - last).abs() <= tolerance => {}
            _ => out.push(value),
        }
    }
    *values = out;
}

/// Ruled tables: rules that cross one another in a grid of at least two
/// cells.
fn tables_of(rules: &[Rule]) -> Vec<FoundTable> {
    let horizontals: Vec<Rule> = rules
        .iter()
        .copied()
        .filter(|rule| rule.horizontal)
        .collect();
    let verticals: Vec<Rule> = rules
        .iter()
        .copied()
        .filter(|rule| !rule.horizontal)
        .collect();
    // Grouped by touching: a union of rules that meet.
    let all: Vec<Rule> = horizontals
        .iter()
        .chain(verticals.iter())
        .copied()
        .collect();
    let mut group: Vec<usize> = (0..all.len()).collect();
    fn root(group: &mut [usize], mut index: usize) -> usize {
        while group[index] != index {
            group[index] = group[group[index]];
            index = group[index];
        }
        index
    }
    let touches = |a: &Rule, b: &Rule| -> bool {
        let tolerance = 2.0;
        match (a.horizontal, b.horizontal) {
            (true, false) => {
                b.at >= a.from - tolerance
                    && b.at <= a.to + tolerance
                    && a.at >= b.from - tolerance
                    && a.at <= b.to + tolerance
            }
            (false, true) => {
                a.at >= b.from - tolerance
                    && a.at <= b.to + tolerance
                    && b.at >= a.from - tolerance
                    && b.at <= a.to + tolerance
            }
            _ => {
                (a.at - b.at).abs() < tolerance
                    && a.from <= b.to + tolerance
                    && b.from <= a.to + tolerance
            }
        }
    };
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            if touches(&all[i], &all[j]) {
                let (a, b) = (root(&mut group, i), root(&mut group, j));
                group[a] = b;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<Rule>> = BTreeMap::new();
    for (index, rule) in all.iter().enumerate() {
        let key = root(&mut group, index);
        groups.entry(key).or_default().push(*rule);
    }
    let mut out = Vec::new();
    for rules in groups.into_values() {
        let mut ys: Vec<f64> = rules
            .iter()
            .filter(|r| r.horizontal)
            .map(|r| r.at)
            .collect();
        let mut xs: Vec<f64> = rules
            .iter()
            .filter(|r| !r.horizontal)
            .map(|r| r.at)
            .collect();
        cluster(&mut ys, 2.0);
        cluster(&mut xs, 2.0);
        if ys.len() < 2 || xs.len() < 2 {
            continue;
        }
        if (ys.len() - 1) * (xs.len() - 1) < 2 {
            continue;
        }
        out.push(FoundTable {
            top: ys[0],
            bottom: *ys.last().expect("two or more"),
            left: xs[0],
            right: *xs.last().expect("two or more"),
            horizontals: rules.iter().copied().filter(|r| r.horizontal).collect(),
            verticals: rules.iter().copied().filter(|r| !r.horizontal).collect(),
            xs,
            ys,
        });
    }
    out
}

impl FoundTable {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.left - 1.0
            && x <= self.right + 1.0
            && y >= self.top - 1.0
            && y <= self.bottom + 1.0
    }

    /// Whether a rule runs along `at` from `from` to `to`.
    fn ruled(&self, horizontal: bool, at: f64, from: f64, to: f64) -> bool {
        let rules = if horizontal {
            &self.horizontals
        } else {
            &self.verticals
        };
        rules.iter().any(|rule| {
            (rule.at - at).abs() < 2.0 && rule.from <= from + 2.0 && rule.to >= to - 2.0
        })
    }

    #[allow(clippy::needless_range_loop)]
    fn build(&self, pieces: &[TextPiece], body_size: f64) -> Table {
        let rows = self.ys.len() - 1;
        let columns = self.xs.len() - 1;
        let mut texts: Vec<Vec<Vec<TextPiece>>> = vec![vec![Vec::new(); columns]; rows];
        for piece in pieces {
            let cx = piece.x + piece.width.min(piece.size) / 2.0;
            let cy = piece.y - piece.size * 0.3;
            let column = self
                .xs
                .windows(2)
                .position(|pair| cx >= pair[0] - 1.0 && cx <= pair[1] + 1.0);
            let row = self
                .ys
                .windows(2)
                .position(|pair| cy >= pair[0] - 1.0 && cy <= pair[1] + 1.0);
            if let (Some(row), Some(column)) = (row, column) {
                texts[row][column].push(piece.clone());
            }
        }
        let mut table = Table {
            columns: self
                .xs
                .windows(2)
                .map(|pair| ((pair[1] - pair[0]) * 20.0).round().max(1.0) as u32)
                .collect(),
            rows: Vec::new(),
            borders: true,
        };
        for row in 0..rows {
            let mut cells = Vec::new();
            let mut column = 0;
            while column < columns {
                // Columns with no rule between them are one cell.
                let mut span = 1;
                while column + span < columns
                    && !self.ruled(
                        false,
                        self.xs[column + span],
                        self.ys[row],
                        self.ys[row + 1],
                    )
                {
                    span += 1;
                }
                let above_open = row > 0
                    && !self.ruled(true, self.ys[row], self.xs[column], self.xs[column + span]);
                let below_open = row + 1 < rows
                    && !self.ruled(
                        true,
                        self.ys[row + 1],
                        self.xs[column],
                        self.xs[column + span],
                    );
                let vertical_merge = match (above_open, below_open) {
                    (true, _) => VerticalMerge::Continue,
                    (false, true) => VerticalMerge::Restart,
                    _ => VerticalMerge::None,
                };
                let mut pieces: Vec<TextPiece> = (column..column + span)
                    .flat_map(|c| texts[row][c].clone())
                    .collect();
                if vertical_merge == VerticalMerge::Continue {
                    pieces.clear();
                }
                let lines = lines_of(pieces);
                let mut blocks = Vec::new();
                for line in lines {
                    let mut paragraph = Paragraph::default();
                    for piece in &line.pieces {
                        paragraph.push_text(
                            &piece.text,
                            &CharFormat {
                                bold: piece.bold,
                                italic: piece.italic,
                                size: Some((piece.size * 2.0).round() as u32)
                                    .filter(|size| (*size as f64 - body_size * 2.0).abs() > 0.9),
                                ..CharFormat::default()
                            },
                        );
                    }
                    trim_paragraph(&mut paragraph);
                    blocks.push(Block::Paragraph(paragraph));
                }
                cells.push(Cell {
                    blocks,
                    span: span as u32,
                    vertical_merge,
                    shading: None,
                });
                column += span;
            }
            table.rows.push(Row {
                cells,
                header: false,
            });
        }
        // A continued cell's restart is the one above it: fix the starts.
        table
    }
}

// ---- the document ----------------------------------------------------------------------

/// Reads a PDF as a document.
pub fn read_pdf(bytes: &[u8]) -> Result<FlowDocument, String> {
    let document =
        Document::load_mem(bytes).map_err(|error| format!("not a readable PDF: {error}"))?;
    if document.is_encrypted() {
        return Err("the PDF is password-protected".to_string());
    }
    let pages: Vec<ObjectId> = document.get_pages().values().copied().collect();
    if pages.is_empty() {
        return Err("the PDF has no pages".to_string());
    }
    let contents: Vec<PageContent> = pages
        .iter()
        .map(|page| page_content(&document, *page))
        .collect();
    if contents
        .iter()
        .all(|page| page.text.is_empty() && page.pictures.is_empty())
    {
        return Err(
            "the PDF has no text to convert (a scanned PDF needs text recognition)".to_string(),
        );
    }
    // Repeated at the top or bottom of most pages: the header and footer.
    let page_count = contents.len();
    let normalize = |text: &str| -> String {
        text.chars()
            .map(|ch| if ch.is_ascii_digit() { '#' } else { ch })
            .filter(|ch| !ch.is_whitespace())
            .collect()
    };
    let mut repeated: HashMap<(bool, String), usize> = HashMap::new();
    let page_lines: Vec<Vec<Line>> = contents
        .iter()
        .map(|page| lines_of(page.text.clone()))
        .collect();
    for (page, lines) in contents.iter().zip(&page_lines) {
        let mut seen = std::collections::HashSet::new();
        for line in lines {
            let top = line.y < page.height * 0.1;
            let bottom = line.y > page.height * 0.9;
            if top || bottom {
                let key = (top, normalize(&line.text()));
                if seen.insert(key.clone()) {
                    *repeated.entry(key).or_default() += 1;
                }
            }
        }
    }
    let furniture = |line: &Line, height: f64| -> Option<bool> {
        let top = line.y < height * 0.1;
        let bottom = line.y > height * 0.9;
        if !(top || bottom) || page_count < 2 {
            return None;
        }
        let count = repeated
            .get(&(top, normalize(&line.text())))
            .copied()
            .unwrap_or(0);
        (count * 10 >= page_count * 6).then_some(top)
    };
    let mut header_text: Option<Paragraph> = None;
    let mut footer_text: Option<Paragraph> = None;
    let body_size = mode(
        contents
            .iter()
            .flat_map(|page| page.text.iter())
            .flat_map(|piece| std::iter::repeat_n(piece.size, piece.text.chars().count().min(200))),
        0.5,
    )
    .unwrap_or(10.5);
    let mut pages_for_flow: Vec<(Vec<Line>, f64, f64)> = Vec::new();
    let mut tables_by_page: Vec<Vec<(FoundTable, Table)>> = Vec::new();
    for (page, lines) in contents.iter().zip(page_lines) {
        let tables = tables_of(&page.rules);
        let mut body_lines = Vec::new();
        for line in lines {
            match furniture(&line, page.height) {
                Some(true) => {
                    if header_text.is_none() {
                        header_text = Some(Paragraph::with_text(line.text().trim()));
                    }
                }
                Some(false) => {
                    if footer_text.is_none()
                        && !line
                            .text()
                            .trim()
                            .chars()
                            .all(|ch| ch.is_ascii_digit() || ch == '-' || ch.is_whitespace())
                    {
                        footer_text = Some(Paragraph::with_text(line.text().trim()));
                    }
                }
                None => {
                    if !tables
                        .iter()
                        .any(|table| table.contains(line.x0 + 1.0, line.y - line.size * 0.3))
                    {
                        body_lines.push(line);
                    }
                }
            }
        }
        let built: Vec<(FoundTable, Table)> = tables
            .into_iter()
            .map(|table| {
                let inside: Vec<TextPiece> = page
                    .text
                    .iter()
                    .filter(|piece| table.contains(piece.x + 1.0, piece.y - piece.size * 0.3))
                    .cloned()
                    .collect();
                let built = table.build(&inside, body_size);
                (table, built)
            })
            .collect();
        tables_by_page.push(built);
        pages_for_flow.push((body_lines, page.width, page.height));
    }
    let left = mode(
        pages_for_flow
            .iter()
            .flat_map(|(lines, _, _)| lines.iter().map(|line| line.x0)),
        2.0,
    )
    .unwrap_or(72.0);
    // The text's right edge: where most full lines end, not the furthest a
    // line overflows.
    let right = {
        let mut ends: Vec<f64> = pages_for_flow
            .iter()
            .flat_map(|(lines, _, _)| lines.iter().map(|line| line.x1))
            .collect();
        ends.sort_by(f64::total_cmp);
        ends.get(ends.len() * 9 / 10)
            .copied()
            .unwrap_or(left + 100.0)
            .max(left + 100.0)
    };
    let paragraphs = paragraphs_of(&pages_for_flow, body_size, (left, right));
    // Paragraphs, tables and pictures in page order, by where they start.
    let mut placed: Vec<(usize, f64, Block)> = paragraphs
        .into_iter()
        .map(|(page, paragraph, y)| (page, y, Block::Paragraph(paragraph)))
        .collect();
    for (page, tables) in tables_by_page.into_iter().enumerate() {
        for (found, table) in tables {
            placed.push((page, found.top, Block::Table(table)));
        }
    }
    let text_width = (right - left).max(72.0);
    for (page_index, page) in contents.iter().enumerate() {
        for picture in &page.pictures {
            let mut image = picture.image.clone();
            image.width = (picture.width * 12700.0) as u64;
            image.height = (picture.height * 12700.0) as u64;
            image.fit_width((text_width * 12700.0) as u64);
            let mut paragraph = Paragraph::default();
            let centre = picture.x + picture.width / 2.0;
            if (centre - (left + right) / 2.0).abs() < text_width * 0.1
                && picture.width < text_width * 0.9
            {
                paragraph.alignment = Some(Alignment::Center);
            }
            paragraph.inlines.push(Inline::Image(image));
            placed.push((page_index, picture.y, Block::Paragraph(paragraph)));
        }
    }
    placed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let blocks: Vec<Block> = placed.into_iter().map(|(_, _, block)| block).collect();
    let first = &contents[0];
    let top = pages_for_flow
        .first()
        .and_then(|(lines, _, _)| lines.first().map(|line| line.y - line.size))
        .unwrap_or(72.0);
    let twips = |points: f64| (points.max(0.0) * 20.0).round() as u32;
    let page = PageSetup {
        width: twips(first.width),
        height: twips(first.height),
        top: twips(top.clamp(18.0, first.height / 3.0)),
        bottom: twips((first.height * 0.08).max(36.0)),
        left: twips(left.clamp(18.0, first.width / 3.0)),
        right: twips((first.width - right).clamp(18.0, first.width / 3.0)),
        header: twips(36.0),
        footer: twips(36.0),
    };
    let language = blocks
        .iter()
        .take(50)
        .any(|block| matches!(block, Block::Paragraph(p) if p.text().chars().any(is_cjk)))
        .then(|| "ja-JP".to_string());
    let title = document
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|info| document.dereference(info).ok())
        .and_then(|(_, info)| info.as_dict().ok())
        .and_then(|info| info.get(b"Title").ok())
        .and_then(|title| title.as_str().ok())
        .map(|title| {
            if title.starts_with(&[0xfe, 0xff]) {
                let units: Vec<u16> = title[2..]
                    .chunks(2)
                    .map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]))
                    .collect();
                String::from_utf16_lossy(&units)
            } else {
                String::from_utf8_lossy(title).into_owned()
            }
        })
        .filter(|title| !title.trim().is_empty());
    Ok(FlowDocument {
        title,
        author: None,
        default_font: None,
        default_size: Some((body_size * 2.0).round() as u32),
        language,
        sections: vec![FlowSection {
            page,
            header: header_text.map(Block::Paragraph).into_iter().collect(),
            footer: footer_text.map(Block::Paragraph).into_iter().collect(),
            blocks,
        }],
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_to_unicode_cmap_maps_codes_by_char_and_by_range() {
        let cmap = ToUnicode::parse(
            b"/CIDInit /ProcSet findresource begin\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
2 beginbfchar\n<03> <0020>\n<11> <3042>\nendbfchar\n\
2 beginbfrange\n<20> <22> <0041>\n<30> <31> [<65E5> <672C>]\nendbfrange\nendcmap",
        )
        .unwrap();
        assert_eq!(cmap.code_bytes, 1);
        let text =
            |codes: &[u32]| -> String { codes.iter().map(|code| cmap.map[code].clone()).collect() };
        assert_eq!(
            text(&[0x11, 0x03, 0x20, 0x21, 0x22, 0x30, 0x31]),
            "あ ABC日本"
        );
    }

    use super::*;

    #[test]
    fn list_markers_are_recognised() {
        assert_eq!(list_marker("• item"), Some(("• ".len(), false)));
        assert_eq!(list_marker("1. first"), Some(("1. ".len(), true)));
        assert_eq!(list_marker("(2) second"), Some(("(2) ".len(), true)));
        assert_eq!(list_marker("1.5 inches"), None);
        assert_eq!(list_marker("Hello"), None);
    }

    #[test]
    fn a_png_is_written_that_decodes() {
        let png = png(2, 1, 3, &[255, 0, 0, 0, 255, 0]);
        assert_eq!(ImageFormat::sniff(&png), Some(ImageFormat::Png));
        assert_eq!(ImageFormat::pixel_size(&png), Some((2, 1)));
    }
}
