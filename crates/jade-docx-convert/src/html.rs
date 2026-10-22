//! HTML (`.html`, `.htm`), in and out.
//!
//! The reader is lenient, as a browser is: unclosed paragraphs, list items
//! and cells close where the next one starts, entities and the page's
//! `charset` are honoured, and inline CSS on the elements is read for what
//! Word keeps — weight, slant, underline, colour, size, face, alignment,
//! indents and spacing. Pictures come from `data:` URIs or, for relative
//! paths, from what the caller can resolve them to.
//!
//! The writer writes one self-contained page, as Word's 単一ファイル Web
//! ページ does but in plain HTML: headings as `h1`–`h6`, lists as `ul` and
//! `ol`, tables with their spans, pictures inline as `data:` URIs.

use std::fmt::Write as _;

use base64::Engine;

use crate::flow::*;
use crate::text::{detect_encoding, TextEncoding};

// ---- a lenient tree ------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct HtmlElement {
    tag: String,
    attributes: Vec<(String, String)>,
    children: Vec<HtmlNode>,
}

#[derive(Clone, Debug)]
enum HtmlNode {
    Element(HtmlElement),
    Text(String),
}

impl HtmlElement {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn find(&self, tag: &str) -> Option<&HtmlElement> {
        for child in &self.children {
            if let HtmlNode::Element(element) = child {
                if element.tag == tag {
                    return Some(element);
                }
                if let Some(found) = element.find(tag) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            match child {
                HtmlNode::Text(text) => out.push_str(text),
                HtmlNode::Element(element) => out.push_str(&element.text()),
            }
        }
        out
    }
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
const RAW: &[&str] = &["script", "style", "title", "textarea", "xmp"];
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "dl",
    "li",
    "dt",
    "dd",
    "pre",
    "blockquote",
    "table",
    "hr",
    "address",
    "center",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "figure",
    "figcaption",
    "form",
    "fieldset",
    "details",
    "summary",
];

fn parse_html(source: &str) -> HtmlElement {
    let mut stack: Vec<HtmlElement> = vec![HtmlElement {
        tag: "#root".into(),
        ..HtmlElement::default()
    }];
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut text = String::new();
    let flush = |text: &mut String, stack: &mut Vec<HtmlElement>| {
        if !text.is_empty() {
            let decoded = decode_entities(text);
            if let Some(top) = stack.last_mut() {
                top.children.push(HtmlNode::Text(decoded));
            }
            text.clear();
        }
    };
    while at < bytes.len() {
        if bytes[at] != b'<' {
            let next = source[at..]
                .find('<')
                .map(|offset| at + offset)
                .unwrap_or(bytes.len());
            text.push_str(&source[at..next]);
            at = next;
            continue;
        }
        let rest = &source[at..];
        if rest.starts_with("<!--") {
            flush(&mut text, &mut stack);
            at = rest
                .find("-->")
                .map(|end| at + end + 3)
                .unwrap_or(bytes.len());
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            flush(&mut text, &mut stack);
            at = rest
                .find('>')
                .map(|end| at + end + 1)
                .unwrap_or(bytes.len());
            continue;
        }
        let closing = rest.starts_with("</");
        let name_start = if closing { 2 } else { 1 };
        let name: String = rest[name_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == ':' || *ch == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            // A lone `<` is text.
            text.push('<');
            at += 1;
            continue;
        }
        flush(&mut text, &mut stack);
        // The tag runs to the `>` that is not in a quoted attribute.
        let mut end = at + name_start + name.len();
        let mut quote: Option<u8> = None;
        while end < bytes.len() {
            let byte = bytes[end];
            match quote {
                Some(q) if byte == q => quote = None,
                Some(_) => {}
                None if byte == b'"' || byte == b'\'' => quote = Some(byte),
                None if byte == b'>' => break,
                None => {}
            }
            end += 1;
        }
        let inside = &source[at + name_start + name.len()..end.min(bytes.len())];
        at = (end + 1).min(bytes.len());
        if closing {
            if let Some(position) = stack.iter().rposition(|element| element.tag == name) {
                if position > 0 {
                    while stack.len() > position {
                        close_top(&mut stack);
                    }
                }
            }
            continue;
        }
        let self_closing = inside.trim_end().ends_with('/');
        let attributes = parse_attributes(inside.trim_end().trim_end_matches('/'));
        auto_close(&mut stack, &name);
        let element = HtmlElement {
            tag: name.clone(),
            attributes,
            children: Vec::new(),
        };
        if VOID.contains(&name.as_str()) || self_closing {
            if let Some(top) = stack.last_mut() {
                top.children.push(HtmlNode::Element(element));
            }
            continue;
        }
        if RAW.contains(&name.as_str()) {
            let close = format!("</{name}");
            let lower = source[at..].to_ascii_lowercase();
            let end = lower
                .find(&close)
                .map(|offset| at + offset)
                .unwrap_or(bytes.len());
            let mut element = element;
            element
                .children
                .push(HtmlNode::Text(decode_entities(&source[at..end])));
            if let Some(top) = stack.last_mut() {
                top.children.push(HtmlNode::Element(element));
            }
            at = source[end..]
                .find('>')
                .map(|offset| end + offset + 1)
                .unwrap_or(bytes.len());
            continue;
        }
        stack.push(element);
    }
    flush(&mut text, &mut stack);
    while stack.len() > 1 {
        close_top(&mut stack);
    }
    stack.pop().unwrap_or_default()
}

fn close_top(stack: &mut Vec<HtmlElement>) {
    if let Some(element) = stack.pop() {
        if let Some(parent) = stack.last_mut() {
            parent.children.push(HtmlNode::Element(element));
        }
    }
}

/// What a start tag closes first: an open paragraph before a block, the
/// last item before the next, a cell before a cell, a row before a row.
fn auto_close(stack: &mut Vec<HtmlElement>, name: &str) {
    let close_until = |stack: &mut Vec<HtmlElement>, targets: &[&str], boundaries: &[&str]| {
        let found = stack
            .iter()
            .rposition(|element| targets.contains(&element.tag.as_str()));
        let boundary = stack
            .iter()
            .rposition(|element| boundaries.contains(&element.tag.as_str()));
        if let Some(found) = found {
            if boundary.is_none_or(|boundary| boundary < found) && found > 0 {
                while stack.len() > found {
                    close_top(stack);
                }
            }
        }
    };
    if BLOCKS.contains(&name) {
        close_until(
            stack,
            &["p"],
            &["div", "li", "td", "th", "blockquote", "table", "body"],
        );
    }
    match name {
        "li" => close_until(stack, &["li"], &["ul", "ol"]),
        "dt" | "dd" => close_until(stack, &["dt", "dd"], &["dl"]),
        "td" | "th" => close_until(stack, &["td", "th"], &["tr", "table"]),
        "tr" => close_until(stack, &["tr"], &["table", "tbody", "thead", "tfoot"]),
        "tbody" | "thead" | "tfoot" => close_until(stack, &["tbody", "thead", "tfoot"], &["table"]),
        "option" => close_until(stack, &["option"], &["select"]),
        _ => {}
    }
}

fn parse_attributes(inside: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let chars: Vec<char> = inside.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        while at < chars.len() && chars[at].is_whitespace() {
            at += 1;
        }
        let start = at;
        while at < chars.len() && !chars[at].is_whitespace() && chars[at] != '=' {
            at += 1;
        }
        let name: String = chars[start..at]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            at += 1;
            continue;
        }
        while at < chars.len() && chars[at].is_whitespace() {
            at += 1;
        }
        let mut value = String::new();
        if at < chars.len() && chars[at] == '=' {
            at += 1;
            while at < chars.len() && chars[at].is_whitespace() {
                at += 1;
            }
            if at < chars.len() && (chars[at] == '"' || chars[at] == '\'') {
                let quote = chars[at];
                at += 1;
                let start = at;
                while at < chars.len() && chars[at] != quote {
                    at += 1;
                }
                value = chars[start..at].iter().collect();
                at += 1;
            } else {
                let start = at;
                while at < chars.len() && !chars[at].is_whitespace() {
                    at += 1;
                }
                value = chars[start..at].iter().collect();
            }
        }
        out.push((name, decode_entities(&value)));
    }
    out
}

const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{a0}"),
    ("copy", "©"),
    ("reg", "®"),
    ("trade", "™"),
    ("hellip", "…"),
    ("mdash", "—"),
    ("ndash", "–"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("bull", "•"),
    ("middot", "·"),
    ("yen", "¥"),
    ("euro", "€"),
    ("pound", "£"),
    ("cent", "¢"),
    ("times", "×"),
    ("divide", "÷"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("deg", "°"),
    ("plusmn", "±"),
    ("sect", "§"),
    ("para", "¶"),
    ("ensp", "\u{2002}"),
    ("emsp", "\u{2003}"),
    ("thinsp", "\u{2009}"),
    ("shy", "\u{ad}"),
    ("larr", "←"),
    ("rarr", "→"),
    ("uarr", "↑"),
    ("darr", "↓"),
    ("hearts", "♥"),
    ("check", "✓"),
    ("iexcl", "¡"),
    ("iquest", "¿"),
    ("eacute", "é"),
    ("egrave", "è"),
    ("agrave", "à"),
    ("aacute", "á"),
    ("uuml", "ü"),
    ("ouml", "ö"),
    ("auml", "ä"),
    ("szlig", "ß"),
    ("ccedil", "ç"),
    ("ntilde", "ñ"),
];

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find('&') {
        out.push_str(&rest[..position]);
        rest = &rest[position..];
        let end = rest[1..]
            .find(|ch: char| ch == ';' || ch.is_whitespace() || ch == '&' || ch == '<')
            .map(|e| e + 1);
        let decoded = end
            .filter(|end| rest.as_bytes().get(*end) == Some(&b';'))
            .and_then(|end| {
                let name = &rest[1..end];
                let value = if let Some(number) =
                    name.strip_prefix("#x").or_else(|| name.strip_prefix("#X"))
                {
                    u32::from_str_radix(number, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .map(String::from)
                } else if let Some(number) = name.strip_prefix('#') {
                    number
                        .parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .map(String::from)
                } else {
                    ENTITIES
                        .iter()
                        .find(|(known, _)| *known == name)
                        .map(|(_, value)| value.to_string())
                };
                value.map(|value| (value, end + 1))
            });
        match decoded {
            Some((value, length)) => {
                out.push_str(&value);
                rest = &rest[length..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---- reading ---------------------------------------------------------------

/// Reads HTML bytes in their declared or detected encoding. `resolve`
/// gives the bytes of a picture the page names by a relative path.
pub fn read_html(bytes: &[u8], resolve: &dyn Fn(&str) -> Option<Vec<u8>>) -> FlowDocument {
    let source = decode_html(bytes);
    let root = parse_html(&source);
    let mut reader = HtmlReader {
        resolve,
        blocks: Vec::new(),
        paragraph: None,
        list_depth: Vec::new(),
        header: Vec::new(),
        footer: Vec::new(),
    };
    let body = root.find("body").unwrap_or(&root);
    reader.children(body, &Context::default());
    reader.end_paragraph();
    let mut document = FlowDocument::single(std::mem::take(&mut reader.blocks));
    document.sections[0].header = std::mem::take(&mut reader.header);
    document.sections[0].footer = std::mem::take(&mut reader.footer);
    document.title = root
        .find("title")
        .map(|title| title.text().trim().to_string())
        .filter(|title| !title.is_empty());
    if root
        .find("html")
        .and_then(|html| html.attr("lang"))
        .is_some_and(|lang| lang.starts_with("ja"))
        || source
            .chars()
            .any(|ch| matches!(ch as u32, 0x3040..=0x30ff))
    {
        document.language = Some("ja-JP".into());
    }
    document
}

/// The page's text: its byte order mark, then its `<meta charset>`, then
/// whatever decodes cleanly.
fn decode_html(bytes: &[u8]) -> String {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    if bytes.starts_with(b"\xef\xbb\xbf")
        || bytes.starts_with(b"\xff\xfe")
        || bytes.starts_with(b"\xfe\xff")
    {
        return detect_encoding(bytes).decode(bytes);
    }
    let declared = head.find("charset").and_then(|position| {
        let rest = head[position + 7..]
            .trim_start()
            .strip_prefix('=')?
            .trim_start();
        let rest = rest.trim_start_matches(['"', '\'']);
        let label: String = rest
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
            .collect();
        TextEncoding::from_label(&label)
    });
    declared
        .unwrap_or_else(|| detect_encoding(bytes))
        .decode(bytes)
}

#[derive(Clone, Debug, Default)]
struct Context {
    format: CharFormat,
    preformatted: bool,
    alignment: Option<Alignment>,
    hidden: bool,
}

struct HtmlReader<'a> {
    resolve: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    blocks: Vec<Block>,
    paragraph: Option<Paragraph>,
    /// For each open list, whether it is numbered.
    list_depth: Vec<bool>,
    header: Vec<Block>,
    footer: Vec<Block>,
}

impl HtmlReader<'_> {
    fn paragraph(&mut self, context: &Context) -> &mut Paragraph {
        if self.paragraph.is_none() {
            self.paragraph = Some(Paragraph {
                alignment: context.alignment,
                ..Paragraph::default()
            });
        }
        self.paragraph.as_mut().expect("just made")
    }

    fn end_paragraph(&mut self) {
        if let Some(mut paragraph) = self.paragraph.take() {
            // Space at the edges of a paragraph is not shown.
            trim_inlines(&mut paragraph);
            if !paragraph.inlines.is_empty()
                || paragraph.list.is_some()
                || paragraph.kind != ParagraphKind::Normal
            {
                self.blocks.push(Block::Paragraph(paragraph));
            }
        }
    }

    fn children(&mut self, element: &HtmlElement, context: &Context) {
        for child in &element.children {
            match child {
                HtmlNode::Text(text) => self.text(text, context),
                HtmlNode::Element(element) => self.element(element, context),
            }
        }
    }

    fn text(&mut self, text: &str, context: &Context) {
        if context.hidden {
            return;
        }
        if context.preformatted {
            let format = context.format.clone();
            let mut first = true;
            for line in text.split('\n') {
                if !first {
                    self.paragraph(context).inlines.push(Inline::LineBreak);
                }
                first = false;
                let mut parts = line.split('\t').peekable();
                while let Some(part) = parts.next() {
                    self.paragraph(context).push_text(part, &format);
                    if parts.peek().is_some() {
                        self.paragraph(context).inlines.push(Inline::Tab);
                    }
                }
            }
            return;
        }
        let mut collapsed = String::with_capacity(text.len());
        let mut last_space = self
            .paragraph
            .as_ref()
            .map(|paragraph| match paragraph.inlines.last() {
                Some(Inline::Text(text, _)) => text.ends_with(' '),
                Some(Inline::LineBreak) | None => true,
                _ => false,
            })
            .unwrap_or(true);
        for ch in text.chars() {
            if ch.is_whitespace() && ch != '\u{a0}' && ch != '\u{3000}' {
                if !last_space {
                    collapsed.push(' ');
                }
                last_space = true;
            } else {
                collapsed.push(ch);
                last_space = false;
            }
        }
        if collapsed.is_empty() || (collapsed == " " && self.paragraph.is_none()) {
            return;
        }
        let format = context.format.clone();
        self.paragraph(context).push_text(&collapsed, &format);
    }

    fn element(&mut self, element: &HtmlElement, parent: &Context) {
        let mut context = parent.clone();
        let style = element.attr("style").map(parse_css).unwrap_or_default();
        apply_character_css(&style, &mut context.format);
        if style
            .iter()
            .any(|(name, value)| name == "display" && value == "none")
        {
            context.hidden = true;
        }
        let tag = element.tag.as_str();
        // A page's header or footer, as LibreOffice (`title`) and Word
        // (`mso-element`) mark them.
        let story = element
            .attr("title")
            .filter(|_| tag == "div")
            .map(str::to_string)
            .or_else(|| {
                style
                    .iter()
                    .find(|(name, _)| name == "mso-element")
                    .map(|(_, value)| value.clone())
            });
        if let Some(story @ ("header" | "footer")) = story.as_deref() {
            self.end_paragraph();
            let saved = std::mem::take(&mut self.blocks);
            self.children(element, &context);
            self.end_paragraph();
            let blocks = std::mem::replace(&mut self.blocks, saved);
            if story == "header" {
                self.header = blocks;
            } else {
                self.footer = blocks;
            }
            return;
        }
        match tag {
            "head" | "script" | "style" | "title" | "noscript" | "template" | "select"
            | "button" | "iframe" | "object" => return,
            "b" | "strong" => context.format.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" => context.format.italic = true,
            "u" | "ins" => context.format.underline = true,
            "s" | "strike" | "del" => context.format.strike = true,
            "sup" => context.format.superscript = true,
            "sub" => context.format.subscript = true,
            "code" | "tt" | "kbd" | "samp" => context.format.font = Some("Courier New".into()),
            "font" => {
                if let Some(face) = element.attr("face") {
                    context.format.font = face
                        .split(',')
                        .next()
                        .map(|face| face.trim().trim_matches(['"', '\'']).to_string());
                }
                if let Some(color) = element.attr("color").and_then(css_color) {
                    context.format.color = Some(color);
                }
                if let Some(size) = element
                    .attr("size")
                    .and_then(|size| size.trim().parse::<i32>().ok())
                {
                    let points = [8, 10, 12, 14, 18, 24, 36][(size.clamp(1, 7) - 1) as usize];
                    context.format.size = Some(points * 2);
                }
            }
            "a" => {
                if let Some(href) = element.attr("href").filter(|href| !href.is_empty()) {
                    context.format.link = Some(href.to_string());
                }
            }
            "mark" => context.format.highlight = Some("yellow".into()),
            _ => {}
        }
        if let Some(align) = element.attr("align") {
            context.alignment = alignment_of(align);
        }
        if let Some(align) = style
            .iter()
            .find(|(name, _)| name == "text-align")
            .map(|(_, value)| value.as_str())
        {
            context.alignment = alignment_of(align);
        }
        if tag == "center" {
            context.alignment = Some(Alignment::Center);
        }
        match tag {
            "br" => {
                self.paragraph(&context).inlines.push(Inline::LineBreak);
            }
            "img" => {
                if context.hidden {
                    return;
                }
                if let Some(image) = self.image(element, &style) {
                    self.paragraph(&context).inlines.push(Inline::Image(image));
                }
            }
            "hr" => {
                self.end_paragraph();
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "pre" | "blockquote"
            | "address" | "center" | "dt" | "dd" | "figcaption" | "caption" | "summary"
            | "section" | "article" | "header" | "footer" | "main" | "nav" | "aside" | "figure"
            | "form" | "fieldset" | "details" => {
                self.end_paragraph();
                let kind = match tag {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        ParagraphKind::Heading(tag[1..].parse().unwrap_or(1))
                    }
                    "pre" => ParagraphKind::Preformatted,
                    "blockquote" => ParagraphKind::Quote,
                    _ => ParagraphKind::Normal,
                };
                if tag == "pre" {
                    context.preformatted = true;
                }
                let paragraph = self.paragraph(&context);
                paragraph.kind = kind;
                apply_paragraph_css(&style, paragraph);
                if tag == "dd" {
                    paragraph.indent_left = paragraph.indent_left.max(720);
                }
                if context.hidden {
                    self.paragraph = None;
                    return;
                }
                self.children(element, &context);
                self.end_paragraph();
            }
            "ul" | "ol" => {
                self.end_paragraph();
                self.list_depth.push(tag == "ol");
                self.children(element, &context);
                self.end_paragraph();
                self.list_depth.pop();
            }
            "li" => {
                self.end_paragraph();
                let ordered = self.list_depth.last().copied().unwrap_or(false);
                let level = self.list_depth.len().saturating_sub(1).min(8) as u8;
                let paragraph = self.paragraph(&context);
                paragraph.list = Some(ListItem { ordered, level });
                apply_paragraph_css(&style, paragraph);
                self.children(element, &context);
                self.end_paragraph();
            }
            "table" => {
                self.end_paragraph();
                if context.hidden {
                    return;
                }
                let table = self.table(element, &context);
                if !table.rows.is_empty() {
                    self.blocks.push(Block::Table(table));
                }
            }
            _ => self.children(element, &context),
        }
    }

    fn image(&self, element: &HtmlElement, style: &[(String, String)]) -> Option<Image> {
        let source = element.attr("src")?;
        let data = if let Some(data) = source.strip_prefix("data:") {
            let (header, payload) = data.split_once(',')?;
            if header.ends_with(";base64") {
                base64::engine::general_purpose::STANDARD
                    .decode(payload.split_whitespace().collect::<String>())
                    .ok()?
            } else {
                payload.as_bytes().to_vec()
            }
        } else {
            (self.resolve)(source)?
        };
        let mut image = Image::new(
            data,
            element
                .attr("alt")
                .filter(|alt| !alt.is_empty())
                .map(str::to_string),
        )?;
        let size = |name: &str| {
            style
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .or_else(|| element.attr(name).map(str::to_string))
                .and_then(|value| crate::docx_read::css_length_emu(&value))
        };
        match (size("width"), size("height")) {
            (Some(width), Some(height)) => {
                image.width = width;
                image.height = height;
            }
            (Some(width), None) => {
                image.height = image.height * width / image.width.max(1);
                image.width = width;
            }
            (None, Some(height)) => {
                image.width = image.width * height / image.height.max(1);
                image.height = height;
            }
            (None, None) => {}
        }
        Some(image)
    }

    fn table(&mut self, element: &HtmlElement, context: &Context) -> Table {
        let mut rows: Vec<(&HtmlElement, bool)> = Vec::new();
        fn collect<'a>(
            element: &'a HtmlElement,
            header: bool,
            rows: &mut Vec<(&'a HtmlElement, bool)>,
        ) {
            for child in &element.children {
                if let HtmlNode::Element(child) = child {
                    match child.tag.as_str() {
                        "tr" => rows.push((child, header)),
                        "thead" => collect(child, true, rows),
                        "tbody" | "tfoot" => collect(child, false, rows),
                        _ => {}
                    }
                }
            }
        }
        collect(element, false, &mut rows);
        let style = element.attr("style").map(parse_css).unwrap_or_default();
        let borders = element
            .attr("border")
            .is_some_and(|border| border.trim() != "0")
            || style
                .iter()
                .any(|(name, value)| name.starts_with("border") && !value.contains("none"))
            || element
                .attr("class")
                .is_some_and(|class| class.contains("grid"));
        let mut table = Table {
            borders,
            ..Table::default()
        };
        // Columns a cell above still covers: column → (rows left, span).
        let mut covered: std::collections::BTreeMap<usize, (usize, u32)> = Default::default();
        let saved_blocks = std::mem::take(&mut self.blocks);
        let saved_lists = std::mem::take(&mut self.list_depth);
        let mut any_cell_border = false;
        for (row_element, header) in rows {
            let mut row = Row {
                cells: Vec::new(),
                header,
            };
            let mut column = 0usize;
            let cells: Vec<&HtmlElement> = row_element
                .children
                .iter()
                .filter_map(|child| match child {
                    HtmlNode::Element(element) if element.tag == "td" || element.tag == "th" => {
                        Some(element)
                    }
                    _ => None,
                })
                .collect();
            let mut cells = cells.into_iter().peekable();
            loop {
                if let Some((remaining, span)) = covered.get_mut(&column) {
                    if *remaining > 0 {
                        *remaining -= 1;
                        let span = *span;
                        row.cells.push(Cell {
                            span,
                            vertical_merge: VerticalMerge::Continue,
                            ..Cell::default()
                        });
                        column += span as usize;
                        continue;
                    }
                }
                let Some(cell) = cells.next() else {
                    break;
                };
                let span = cell
                    .attr("colspan")
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .unwrap_or(1)
                    .clamp(1, 63);
                let rows_spanned = cell
                    .attr("rowspan")
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                let cell_style = cell.attr("style").map(parse_css).unwrap_or_default();
                if cell_style
                    .iter()
                    .any(|(name, value)| name.starts_with("border") && !value.contains("none"))
                {
                    any_cell_border = true;
                }
                let mut cell_context = context.clone();
                if cell.tag == "th" {
                    cell_context.format.bold = true;
                }
                if let Some(align) = cell.attr("align") {
                    cell_context.alignment = alignment_of(align);
                }
                self.children(cell, &cell_context);
                self.end_paragraph();
                let blocks = std::mem::take(&mut self.blocks);
                let shading = cell_style
                    .iter()
                    .find(|(name, _)| name == "background-color" || name == "background")
                    .and_then(|(_, value)| css_color(value))
                    .or_else(|| cell.attr("bgcolor").and_then(css_color));
                let vertical_merge = if rows_spanned > 1 {
                    covered.insert(column, (rows_spanned - 1, span));
                    VerticalMerge::Restart
                } else {
                    VerticalMerge::None
                };
                row.cells.push(Cell {
                    blocks,
                    span,
                    vertical_merge,
                    shading,
                });
                column += span as usize;
            }
            // Spans still running past this row's last cell.
            while let Some((remaining, span)) = covered.get_mut(&column) {
                if *remaining == 0 {
                    break;
                }
                *remaining -= 1;
                let span = *span;
                row.cells.push(Cell {
                    span,
                    vertical_merge: VerticalMerge::Continue,
                    ..Cell::default()
                });
                column += span as usize;
            }
            table.rows.push(row);
        }
        self.blocks = saved_blocks;
        self.list_depth = saved_lists;
        table.borders |= any_cell_border;
        table
    }
}

fn trim_inlines(paragraph: &mut Paragraph) {
    if let Some(Inline::Text(text, _)) = paragraph.inlines.first_mut() {
        let trimmed = text.trim_start_matches(' ').to_string();
        *text = trimmed;
    }
    if let Some(Inline::Text(text, _)) = paragraph.inlines.last_mut() {
        let trimmed = text.trim_end_matches(' ').to_string();
        *text = trimmed;
    }
    paragraph
        .inlines
        .retain(|inline| !matches!(inline, Inline::Text(text, _) if text.is_empty()));
}

fn alignment_of(value: &str) -> Option<Alignment> {
    match value.trim().to_ascii_lowercase().as_str() {
        "center" => Some(Alignment::Center),
        "right" | "end" => Some(Alignment::Right),
        "justify" => Some(Alignment::Justify),
        "left" | "start" => Some(Alignment::Left),
        _ => None,
    }
}

fn parse_css(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            Some((
                name.trim().to_ascii_lowercase(),
                value
                    .trim()
                    .trim_end_matches("!important")
                    .trim()
                    .to_string(),
            ))
        })
        .collect()
}

fn apply_character_css(style: &[(String, String)], format: &mut CharFormat) {
    for (name, value) in style {
        let lower = value.to_ascii_lowercase();
        match name.as_str() {
            "font-weight" => {
                format.bold = lower == "bold"
                    || lower == "bolder"
                    || lower.parse::<u32>().is_ok_and(|w| w >= 600)
            }
            "font-style" => format.italic = lower == "italic" || lower == "oblique",
            "text-decoration" | "text-decoration-line" => {
                if lower.contains("underline") {
                    format.underline = true;
                }
                if lower.contains("line-through") {
                    format.strike = true;
                }
                if lower == "none" {
                    format.underline = false;
                    format.strike = false;
                }
            }
            "color" => format.color = css_color(value),
            "background-color" | "background" => {
                if let Some(color) = css_color(value) {
                    format.shading = Some(color);
                }
            }
            "font-size" => {
                if let Some(points) = css_points(&lower) {
                    format.size = Some((points * 2.0).round().max(2.0) as u32);
                }
            }
            "font-family" => {
                format.font = value
                    .split(',')
                    .next()
                    .map(|face| face.trim().trim_matches(['"', '\'']).to_string())
                    .filter(|face| {
                        !face.is_empty()
                            && !matches!(face.as_str(), "serif" | "sans-serif" | "monospace")
                    });
            }
            "vertical-align" => match lower.as_str() {
                "super" => format.superscript = true,
                "sub" => format.subscript = true,
                _ => {}
            },
            "text-transform" => format.all_caps = lower == "uppercase",
            "font-variant" => format.small_caps = lower.contains("small-caps"),
            _ => {}
        }
    }
}

fn apply_paragraph_css(style: &[(String, String)], paragraph: &mut Paragraph) {
    for (name, value) in style {
        let twips =
            || css_points(&value.to_ascii_lowercase()).map(|points| (points * 20.0).round() as i32);
        match name.as_str() {
            "margin-left" | "padding-left" => {
                if let Some(value) = twips() {
                    paragraph.indent_left = value;
                }
            }
            "margin-right" => {
                if let Some(value) = twips() {
                    paragraph.indent_right = value;
                }
            }
            "text-indent" => {
                if let Some(value) = twips() {
                    paragraph.indent_first_line = value;
                }
            }
            "margin-top" => paragraph.space_before = twips().map(|v| v.max(0) as u32),
            "margin-bottom" => paragraph.space_after = twips().map(|v| v.max(0) as u32),
            "line-height" => {
                let lower = value.to_ascii_lowercase();
                paragraph.line_spacing = if let Some(percent) = lower.strip_suffix('%') {
                    percent
                        .parse::<f64>()
                        .ok()
                        .map(|p| LineSpacing::Multiple((p * 2.4).round() as u32))
                } else if let Ok(multiple) = lower.parse::<f64>() {
                    Some(LineSpacing::Multiple((multiple * 240.0).round() as u32))
                } else {
                    twips().map(|v| LineSpacing::Exact(v.max(0) as u32))
                };
            }
            "page-break-before" | "break-before" => {
                paragraph.page_break_before = matches!(value.as_str(), "always" | "page")
            }
            "background-color" | "background" => paragraph.shading = css_color(value),
            _ => {}
        }
    }
}

/// A CSS length in points.
fn css_points(value: &str) -> Option<f64> {
    let named = match value {
        "xx-small" => Some(7.0),
        "x-small" => Some(7.5),
        "small" => Some(10.0),
        "medium" => Some(12.0),
        "large" => Some(13.5),
        "x-large" => Some(18.0),
        "xx-large" => Some(24.0),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let split = value
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-'))
        .unwrap_or(value.len());
    let number: f64 = value[..split].parse().ok()?;
    Some(match &value[split..] {
        "pt" => number,
        "px" | "" => number * 0.75,
        "em" | "rem" => number * 12.0,
        "in" => number * 72.0,
        "cm" => number * 72.0 / 2.54,
        "mm" => number * 7.2 / 2.54,
        "pc" => number * 12.0,
        _ => return None,
    })
}

fn css_color(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        let hex: String = hex
            .chars()
            .take_while(|ch| ch.is_ascii_hexdigit())
            .collect();
        return match hex.len() {
            6 => Some(hex.to_ascii_uppercase()),
            3 => Some(
                hex.chars()
                    .flat_map(|ch| [ch, ch])
                    .collect::<String>()
                    .to_ascii_uppercase(),
            ),
            _ => None,
        };
    }
    if let Some(arguments) = value
        .strip_prefix("rgb(")
        .or_else(|| value.strip_prefix("rgba("))
    {
        let parts: Vec<u8> = arguments
            .trim_end_matches(')')
            .split(',')
            .take(3)
            .filter_map(|part| part.trim().parse::<f64>().ok())
            .map(|part| part.clamp(0.0, 255.0) as u8)
            .collect();
        if parts.len() == 3 {
            return Some(format!("{:02X}{:02X}{:02X}", parts[0], parts[1], parts[2]));
        }
        return None;
    }
    let named = [
        ("black", "000000"),
        ("white", "FFFFFF"),
        ("red", "FF0000"),
        ("green", "008000"),
        ("blue", "0000FF"),
        ("yellow", "FFFF00"),
        ("gray", "808080"),
        ("grey", "808080"),
        ("silver", "C0C0C0"),
        ("maroon", "800000"),
        ("navy", "000080"),
        ("purple", "800080"),
        ("teal", "008080"),
        ("olive", "808000"),
        ("lime", "00FF00"),
        ("aqua", "00FFFF"),
        ("fuchsia", "FF00FF"),
        ("orange", "FFA500"),
    ];
    named
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, hex)| hex.to_string())
}

// ---- writing ---------------------------------------------------------------

/// A document as one HTML page.
pub fn write_html(document: &FlowDocument) -> String {
    let mut out = String::new();
    let language = document.language.as_deref().unwrap_or("en");
    let font = document
        .default_font
        .as_deref()
        .unwrap_or("Times New Roman");
    let size = document.default_size.unwrap_or(21) as f64 / 2.0;
    let _ = write!(
        out,
        "<!DOCTYPE html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"generator\" content=\"jade office\">\n<title>{}</title>\n<style>\nbody {{ font-family: \"{}\", serif; font-size: {size}pt; }}\np {{ margin: 0; }}\ntable {{ border-collapse: collapse; }}\ntd, th {{ vertical-align: top; padding: 0 5.4pt; }}\ntable.grid td, table.grid th {{ border: 0.5pt solid #000; }}\n.section {{ break-before: page; page-break-before: always; }}\n</style>\n</head>\n<body>\n",
        escape_html(language),
        escape_html(document.title.as_deref().unwrap_or("")),
        escape_html(font),
    );
    for (index, section) in document.sections.iter().enumerate() {
        if index > 0 {
            out.push_str("<div class=\"section\">\n");
        }
        write_blocks(&section.blocks, &mut out);
        if index > 0 {
            out.push_str("</div>\n");
        }
    }
    out.push_str("</body>\n</html>\n");
    out
}

fn write_blocks(blocks: &[Block], out: &mut String) {
    let mut open_lists: Vec<bool> = Vec::new();
    for block in blocks {
        let item = match block {
            Block::Paragraph(paragraph) => paragraph.list,
            Block::Table(_) => None,
        };
        // Close or open lists down to this item's level.
        let depth = item.map(|item| item.level as usize + 1).unwrap_or(0);
        while open_lists.len() > depth
            || (open_lists.len() == depth
                && depth > 0
                && open_lists.last() != item.map(|item| item.ordered).as_ref())
        {
            let ordered = open_lists.pop().unwrap_or(false);
            out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
        }
        while open_lists.len() < depth {
            let ordered = item.is_some_and(|item| item.ordered);
            out.push_str(if ordered { "<ol>\n" } else { "<ul>\n" });
            open_lists.push(ordered);
        }
        match block {
            Block::Paragraph(paragraph) => write_paragraph(paragraph, out),
            Block::Table(table) => write_table(table, out),
        }
    }
    while let Some(ordered) = open_lists.pop() {
        out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
    }
}

fn write_paragraph(paragraph: &Paragraph, out: &mut String) {
    let tag = match paragraph.kind {
        _ if paragraph.list.is_some() => "li".to_string(),
        ParagraphKind::Heading(level) => format!("h{}", level.clamp(1, 6)),
        ParagraphKind::Title => "h1".to_string(),
        ParagraphKind::Preformatted => "pre".to_string(),
        ParagraphKind::Quote => "blockquote".to_string(),
        ParagraphKind::Normal => "p".to_string(),
    };
    let mut style = String::new();
    if let Some(alignment) = paragraph.alignment {
        let value = match alignment {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justify => "justify",
        };
        let _ = write!(style, "text-align: {value}; ");
    }
    if paragraph.list.is_none() && paragraph.indent_left != 0 {
        let _ = write!(
            style,
            "margin-left: {}pt; ",
            paragraph.indent_left as f64 / 20.0
        );
    }
    if paragraph.indent_right != 0 {
        let _ = write!(
            style,
            "margin-right: {}pt; ",
            paragraph.indent_right as f64 / 20.0
        );
    }
    if paragraph.list.is_none() && paragraph.indent_first_line != 0 {
        let _ = write!(
            style,
            "text-indent: {}pt; ",
            paragraph.indent_first_line as f64 / 20.0
        );
    }
    if let Some(before) = paragraph.space_before {
        let _ = write!(style, "margin-top: {}pt; ", before as f64 / 20.0);
    }
    if let Some(after) = paragraph.space_after {
        let _ = write!(style, "margin-bottom: {}pt; ", after as f64 / 20.0);
    }
    match paragraph.line_spacing {
        Some(LineSpacing::Multiple(line)) => {
            let _ = write!(style, "line-height: {}%; ", line * 100 / 240);
        }
        Some(LineSpacing::Exact(line)) | Some(LineSpacing::AtLeast(line)) => {
            let _ = write!(style, "line-height: {}pt; ", line as f64 / 20.0);
        }
        None => {}
    }
    if paragraph.page_break_before {
        style.push_str("break-before: page; page-break-before: always; ");
    }
    if let Some(shading) = &paragraph.shading {
        let _ = write!(style, "background-color: #{shading}; ");
    }
    let _ = write!(out, "<{tag}");
    if !style.is_empty() {
        let _ = write!(out, " style=\"{}\"", style.trim_end());
    }
    out.push('>');
    let mut open_link: Option<&str> = None;
    for inline in &paragraph.inlines {
        let link = match inline {
            Inline::Text(_, format) => format.link.as_deref(),
            _ => None,
        };
        if link != open_link {
            if open_link.is_some() {
                out.push_str("</a>");
            }
            if let Some(target) = link {
                let _ = write!(out, "<a href=\"{}\">", escape_html(target));
            }
            open_link = link;
        }
        match inline {
            Inline::Text(text, format) => write_run(
                text,
                format,
                paragraph.kind == ParagraphKind::Preformatted,
                out,
            ),
            Inline::Tab => out.push_str(if paragraph.kind == ParagraphKind::Preformatted {
                "\t"
            } else {
                "&emsp;"
            }),
            Inline::LineBreak => out.push_str("<br>"),
            Inline::PageBreak => {
                out.push_str("<br style=\"break-before: page; page-break-before: always\">")
            }
            Inline::Image(image) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(&image.data);
                let _ = write!(
                    out,
                    "<img src=\"data:{};base64,{encoded}\" width=\"{}\" height=\"{}\" alt=\"{}\">",
                    image.format.mime(),
                    (image.width / EMU_PER_PIXEL).max(1),
                    (image.height / EMU_PER_PIXEL).max(1),
                    escape_html(image.description.as_deref().unwrap_or(""))
                );
            }
        }
    }
    if open_link.is_some() {
        out.push_str("</a>");
    }
    if paragraph.inlines.is_empty() && tag == "p" {
        out.push_str("&nbsp;");
    }
    let _ = writeln!(out, "</{tag}>");
}

fn write_run(text: &str, format: &CharFormat, preformatted: bool, out: &mut String) {
    let mut style = String::new();
    if let Some(font) = &format.font {
        let _ = write!(style, "font-family: '{}'; ", escape_html(font));
    }
    if let Some(size) = format.size {
        let _ = write!(style, "font-size: {}pt; ", size as f64 / 2.0);
    }
    if let Some(color) = &format.color {
        let _ = write!(style, "color: #{color}; ");
    }
    let background = format.shading.clone().or_else(|| {
        format
            .highlight
            .as_deref()
            .and_then(highlight_color)
            .map(str::to_string)
    });
    if let Some(background) = background {
        let _ = write!(style, "background-color: #{background}; ");
    }
    if format.small_caps {
        style.push_str("font-variant: small-caps; ");
    }
    if format.all_caps {
        style.push_str("text-transform: uppercase; ");
    }
    if format.hidden {
        style.push_str("display: none; ");
    }
    let mut open = Vec::new();
    if !style.is_empty() {
        let _ = write!(out, "<span style=\"{}\">", style.trim_end());
        open.push("span");
    }
    for (on, tag) in [
        (format.bold, "b"),
        (format.italic, "i"),
        (format.underline && format.link.is_none(), "u"),
        (format.strike, "s"),
        (format.superscript, "sup"),
        (format.subscript, "sub"),
    ] {
        if on {
            let _ = write!(out, "<{tag}>");
            open.push(tag);
        }
    }
    let escaped = escape_html(text);
    if preformatted {
        out.push_str(&escaped);
    } else {
        // Runs of spaces keep their width.
        let mut previous_space = false;
        for ch in escaped.chars() {
            if ch == ' ' {
                if previous_space {
                    out.push_str("&nbsp;");
                } else {
                    out.push(' ');
                }
                previous_space = true;
            } else if ch == '\n' {
                out.push_str("<br>");
                previous_space = false;
            } else {
                out.push(ch);
                previous_space = false;
            }
        }
    }
    for tag in open.iter().rev() {
        let _ = write!(out, "</{tag}>");
    }
}

fn write_table(table: &Table, out: &mut String) {
    let _ = writeln!(
        out,
        "<table{}>",
        if table.borders { " class=\"grid\"" } else { "" }
    );
    if !table.columns.is_empty() && table.columns.len() == table.grid_width() {
        out.push_str("<colgroup>");
        for column in &table.columns {
            let _ = write!(out, "<col style=\"width: {}pt\">", *column as f64 / 20.0);
        }
        out.push_str("</colgroup>\n");
    }
    for (row_index, row) in table.rows.iter().enumerate() {
        out.push_str("<tr>");
        let mut column = 0usize;
        for cell in &row.cells {
            let span = cell.span.max(1) as usize;
            if cell.vertical_merge == VerticalMerge::Continue {
                column += span;
                continue;
            }
            let tag = if row.header { "th" } else { "td" };
            let _ = write!(out, "<{tag}");
            if span > 1 {
                let _ = write!(out, " colspan=\"{span}\"");
            }
            if cell.vertical_merge == VerticalMerge::Restart {
                let mut rows = 1;
                for below in table.rows.iter().skip(row_index + 1) {
                    let mut at = 0;
                    let continues = below.cells.iter().any(|cell| {
                        let here = at == column;
                        at += cell.span.max(1) as usize;
                        here && cell.vertical_merge == VerticalMerge::Continue
                    });
                    if !continues {
                        break;
                    }
                    rows += 1;
                }
                if rows > 1 {
                    let _ = write!(out, " rowspan=\"{rows}\"");
                }
            }
            if let Some(shading) = &cell.shading {
                let _ = write!(out, " style=\"background-color: #{shading}\"");
            }
            out.push('>');
            let mut inner = String::new();
            write_blocks(&cell.blocks, &mut inner);
            out.push_str(inner.trim_end());
            let _ = write!(out, "</{tag}>");
            column += span;
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_pictures(_: &str) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn a_lenient_page_reads_like_a_browser_shows_it() {
        let html = br#"<html><head><meta charset="shift_jis"><title>T</title></head><body>
<h2>Head &amp; more</h2>
<p align=center>first <b>bold <i>both</i></b> <span style="color:#f00;font-size:14pt">red</span>
<p>second<br>line &#x3042;
<ul><li>one<li>two<ol><li>inner</ol></ul>
<table border=1><tr><th colspan=2>wide<tr><td rowspan=2>tall<td>b<tr><td>c</table>
<p><a href="https://example.com/">link</a>
</body></html>"#;
        let document = read_html(html, &no_pictures);
        assert_eq!(document.title.as_deref(), Some("T"));
        let paragraphs = document.paragraphs();
        assert_eq!(paragraphs[0].kind, ParagraphKind::Heading(2));
        assert_eq!(paragraphs[0].text(), "Head & more");
        assert_eq!(paragraphs[1].alignment, Some(Alignment::Center));
        assert_eq!(paragraphs[1].text(), "first bold both red");
        assert!(paragraphs[1].inlines.iter().any(|inline| matches!(
            inline,
            Inline::Text(text, format) if text == "both" && format.bold && format.italic
        )));
        assert!(paragraphs[1].inlines.iter().any(|inline| matches!(
            inline,
            Inline::Text(text, format) if text == "red" && format.color.as_deref() == Some("FF0000") && format.size == Some(28)
        )));
        assert_eq!(paragraphs[2].text(), "second\nline あ");
        assert_eq!(
            paragraphs[3].list,
            Some(ListItem {
                ordered: false,
                level: 0
            })
        );
        assert_eq!(
            paragraphs[4].list,
            Some(ListItem {
                ordered: false,
                level: 0
            })
        );
        assert_eq!(
            paragraphs[5].list,
            Some(ListItem {
                ordered: true,
                level: 1
            })
        );
        let table = document
            .blocks()
            .find_map(|block| match block {
                Block::Table(table) => Some(table),
                _ => None,
            })
            .unwrap();
        assert!(table.borders);
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0].cells[0].span, 2);
        assert_eq!(
            table.rows[1].cells[0].vertical_merge,
            VerticalMerge::Restart
        );
        assert_eq!(
            table.rows[2].cells[0].vertical_merge,
            VerticalMerge::Continue
        );
        assert_eq!(table.rows[2].cells[1].blocks.len(), 1);
        let last = paragraphs.last().unwrap();
        assert!(last.inlines.iter().any(|inline| matches!(
            inline,
            Inline::Text(text, format) if text == "link" && format.link.as_deref() == Some("https://example.com/")
        )));
    }

    #[test]
    fn a_shift_jis_page_is_decoded_by_its_meta() {
        let (body, _) = TextEncoding::ShiftJis.encode("<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=Shift_JIS\"></head><body><p>日本語</p></body></html>");
        let document = read_html(&body, &no_pictures);
        assert_eq!(document.paragraphs()[0].text(), "日本語");
    }

    #[test]
    fn a_written_page_reads_back() {
        let mut heading = Paragraph::with_text("Title & <more>");
        heading.kind = ParagraphKind::Heading(1);
        let mut body = Paragraph {
            alignment: Some(Alignment::Right),
            indent_left: 720,
            ..Paragraph::default()
        };
        body.push_text("a  b ", &CharFormat::default());
        body.push_text(
            "bold",
            &CharFormat {
                bold: true,
                underline: true,
                color: Some("0000FF".into()),
                ..CharFormat::default()
            },
        );
        body.push_text(
            " link",
            &CharFormat {
                link: Some("https://example.com/?a=1&b=2".into()),
                ..CharFormat::default()
            },
        );
        let mut item = Paragraph::with_text("item");
        item.list = Some(ListItem {
            ordered: true,
            level: 0,
        });
        let png = {
            let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            data.extend_from_slice(&4u32.to_be_bytes());
            data.extend_from_slice(&4u32.to_be_bytes());
            data.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
            data
        };
        let mut picture = Paragraph::default();
        picture.inlines.push(Inline::Image(
            Image::new(png.clone(), Some("a picture".into())).unwrap(),
        ));
        let table = Table {
            columns: vec![1440, 1440],
            rows: vec![
                Row {
                    cells: vec![
                        Cell {
                            vertical_merge: VerticalMerge::Restart,
                            ..Cell::with_text("tall")
                        },
                        Cell::with_text("x"),
                    ],
                    header: false,
                },
                Row {
                    cells: vec![
                        Cell {
                            vertical_merge: VerticalMerge::Continue,
                            ..Cell::default()
                        },
                        Cell::with_text("y"),
                    ],
                    header: false,
                },
            ],
            borders: true,
        };
        let document = FlowDocument {
            title: Some("文書".into()),
            sections: vec![FlowSection {
                blocks: vec![
                    Block::Paragraph(heading),
                    Block::Paragraph(body),
                    Block::Paragraph(item),
                    Block::Paragraph(picture),
                    Block::Table(table),
                ],
                ..FlowSection::default()
            }],
            ..FlowDocument::default()
        };
        let html = write_html(&document);
        let read = read_html(html.as_bytes(), &no_pictures);
        assert_eq!(read.title.as_deref(), Some("文書"));
        let paragraphs = read.paragraphs();
        assert_eq!(paragraphs[0].kind, ParagraphKind::Heading(1));
        assert_eq!(paragraphs[0].text(), "Title & <more>");
        assert_eq!(paragraphs[1].text(), "a \u{a0}b bold link");
        assert_eq!(paragraphs[1].alignment, Some(Alignment::Right));
        assert_eq!(paragraphs[1].indent_left, 720);
        assert!(paragraphs[1].inlines.iter().any(|inline| matches!(
            inline,
            Inline::Text(text, format) if text == "bold" && format.bold && format.underline && format.color.as_deref() == Some("0000FF")
        )));
        assert!(paragraphs[1].inlines.iter().any(|inline| matches!(
            inline,
            Inline::Text(_, format) if format.link.as_deref() == Some("https://example.com/?a=1&b=2")
        )));
        assert_eq!(
            paragraphs[2].list,
            Some(ListItem {
                ordered: true,
                level: 0
            })
        );
        let image = paragraphs[3]
            .inlines
            .iter()
            .find_map(|inline| match inline {
                Inline::Image(image) => Some(image.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(image.data, png);
        assert_eq!(image.description.as_deref(), Some("a picture"));
        let table = read
            .blocks()
            .find_map(|block| match block {
                Block::Table(table) => Some(table.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            table.rows[0].cells[0].vertical_merge,
            VerticalMerge::Restart
        );
        assert_eq!(
            table.rows[1].cells[0].vertical_merge,
            VerticalMerge::Continue
        );
        assert_eq!(table.rows[1].cells[1].blocks.len(), 1);
    }
}
