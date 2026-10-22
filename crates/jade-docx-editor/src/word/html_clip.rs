//! HTML from the clipboard as a [`RichClip`]: what 形式を選択して貼り付け's
//! HTML 形式 pastes when the copy came from another application — a web
//! page, another editor — whose clipboard carries `text/html` beside the
//! plain text.
//!
//! The markup is read the way a paste into Word reads it: block elements
//! (`p`, `div`, headings, list items, table rows) and `br` end paragraphs;
//! `b`/`strong`, `i`/`em`, `u`, `s`/`strike`/`del`, `sup` and `sub` and the
//! inline `style` declarations Word and the browsers write (`font-weight`,
//! `font-style`, `text-decoration`, `vertical-align`, `font-size`,
//! `font-family`, `color`, `text-align`) give the runs and paragraphs their
//! formatting. Scripts, styles and comments are dropped; entities are
//! decoded; runs of white space collapse as a browser shows them.

use jade_docx_headless_api::HeadlessRunStyle;

use super::clipboard::{ClipParagraph, ClipRun, RichClip};

/// Elements whose start and end break the paragraph.
const BLOCKS: [&str; 20] = [
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "tr",
    "blockquote",
    "pre",
    "ul",
    "ol",
    "table",
    "section",
    "article",
    "header",
    "footer",
    "address",
];

/// Elements whose content is not text.
const SKIPPED: [&str; 5] = ["script", "style", "head", "title", "template"];

/// The run style and paragraph alignment an open element gives.
#[derive(Clone, Default)]
struct Frame {
    tag: String,
    style: HeadlessRunStyle,
    alignment: Option<String>,
}

/// A colour as CSS writes it (`#c00`, `#C00000`, `rgb(192, 0, 0)`, a few
/// names), as `RRGGBB`.
fn css_color(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        let hex: String = match hex.len() {
            3 => hex.chars().flat_map(|c| [c, c]).collect(),
            6 => hex.to_string(),
            _ => return None,
        };
        return u32::from_str_radix(&hex, 16)
            .ok()
            .map(|_| hex.to_ascii_uppercase());
    }
    if let Some(inner) = value
        .strip_prefix("rgb(")
        .or_else(|| value.strip_prefix("rgba("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<u8> = inner
            .split([',', ' ', '/'])
            .filter(|part| !part.is_empty())
            .take(3)
            .filter_map(|part| part.trim().parse::<f32>().ok())
            .map(|channel| channel.round().clamp(0.0, 255.0) as u8)
            .collect();
        return (parts.len() == 3)
            .then(|| format!("{:02X}{:02X}{:02X}", parts[0], parts[1], parts[2]));
    }
    Some(
        match value.as_str() {
            "black" => "000000",
            "white" => "FFFFFF",
            "red" => "FF0000",
            "green" => "008000",
            "blue" => "0000FF",
            "yellow" => "FFFF00",
            "gray" | "grey" => "808080",
            "navy" => "000080",
            "maroon" => "800000",
            "purple" => "800080",
            "orange" => "FFA500",
            _ => return None,
        }
        .to_string(),
    )
}

/// A CSS length as points: `12pt`, `16px`, `1.5em` of `base`.
fn css_points(value: &str, base: f32) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    let number = |suffix: &str| value.strip_suffix(suffix)?.trim().parse::<f32>().ok();
    let points = if let Some(points) = number("pt") {
        points
    } else if let Some(pixels) = number("px") {
        pixels * 0.75
    } else if let Some(ems) = number("em").or_else(|| number("rem")) {
        ems * base
    } else if let Some(percent) = number("%") {
        percent / 100.0 * base
    } else {
        match value.as_str() {
            "x-small" => 7.5,
            "small" => 10.0,
            "medium" => 12.0,
            "large" => 13.5,
            "x-large" => 18.0,
            "xx-large" => 24.0,
            _ => return None,
        }
    };
    (points > 0.0 && points < 1640.0).then(|| (points * 2.0).round() / 2.0)
}

/// The first family of a CSS `font-family` list, unquoted, when it is a
/// face rather than a generic family.
fn css_family(value: &str) -> Option<String> {
    let first = value.split(',').next()?.trim().trim_matches(['"', '\'']);
    let generic = [
        "serif",
        "sans-serif",
        "monospace",
        "cursive",
        "fantasy",
        "system-ui",
        "inherit",
        "initial",
    ];
    (!first.is_empty() && !generic.contains(&first.to_ascii_lowercase().as_str()))
        .then(|| first.to_string())
}

/// `name="value"`, `name='value'` or `name=value` in a start tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before = lower[..at].chars().last();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => rest[1..].split(quote).next().unwrap_or(""),
            _ => rest
                .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .next()
                .unwrap_or(""),
        };
        return Some(decode_entities(value));
    }
    None
}

/// `&amp;`, `&lt;`, `&#12354;`, `&#x3042;` and their kin as text.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest[1..]
            .find(';')
            .map(|end| end + 1)
            .filter(|end| *end <= 12)
        else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "ensp" => Some('\u{2002}'),
            "emsp" => Some('\u{2003}'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &rest[end + 1..];
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

/// What an element and its `style` attribute set, over what it inherits.
fn frame_for(tag_name: &str, tag: &str, parent: &Frame) -> Frame {
    let mut frame = Frame {
        tag: tag_name.to_string(),
        style: parent.style.clone(),
        alignment: parent.alignment.clone(),
    };
    let style = &mut frame.style;
    match tag_name {
        "b" | "strong" | "th" => style.bold = Some(true),
        "i" | "em" | "cite" | "var" => style.italic = Some(true),
        "u" | "ins" => style.underline = Some(true),
        "s" | "strike" | "del" => style.strike = Some(true),
        "sup" => style.baseline = Some("superscript".to_string()),
        "sub" => style.baseline = Some("subscript".to_string()),
        "h1" => {
            style.bold = Some(true);
            style.font_size = Some(24.0);
        }
        "h2" => {
            style.bold = Some(true);
            style.font_size = Some(18.0);
        }
        "h3" => {
            style.bold = Some(true);
            style.font_size = Some(14.0);
        }
        "h4" | "h5" | "h6" => style.bold = Some(true),
        "code" | "pre" | "tt" | "kbd" => style.font_family = Some("Courier New".to_string()),
        _ => {}
    }
    if tag_name == "font" {
        if let Some(color) = attribute(tag, "color").and_then(|value| css_color(&value)) {
            style.color = Some(color);
        }
        if let Some(face) = attribute(tag, "face").and_then(|value| css_family(&value)) {
            style.font_family = Some(face);
        }
    }
    if let Some(align) = attribute(tag, "align") {
        frame.alignment = alignment(&align);
    }
    if let Some(declarations) = attribute(tag, "style") {
        let base = frame.style.font_size.unwrap_or(12.0);
        for declaration in declarations.split(';') {
            let Some((property, value)) = declaration.split_once(':') else {
                continue;
            };
            let value = value.trim().trim_end_matches("!important").trim();
            let lower = value.to_ascii_lowercase();
            let style = &mut frame.style;
            match property.trim().to_ascii_lowercase().as_str() {
                "font-weight" => {
                    style.bold = Some(
                        lower == "bold"
                            || lower == "bolder"
                            || lower.parse::<u32>().is_ok_and(|weight| weight >= 600),
                    )
                }
                "font-style" => style.italic = Some(lower == "italic" || lower == "oblique"),
                "text-decoration" | "text-decoration-line" => {
                    if lower.contains("underline") {
                        style.underline = Some(true);
                    }
                    if lower.contains("line-through") {
                        style.strike = Some(true);
                    }
                    if lower == "none" {
                        style.underline = Some(false);
                        style.strike = Some(false);
                    }
                }
                "vertical-align" => match lower.as_str() {
                    "super" => style.baseline = Some("superscript".to_string()),
                    "sub" => style.baseline = Some("subscript".to_string()),
                    "baseline" => style.baseline = None,
                    _ => {}
                },
                "font-size" => {
                    if let Some(points) = css_points(&lower, base) {
                        style.font_size = Some(points);
                    }
                }
                "font-family" => {
                    if let Some(family) = css_family(value) {
                        style.font_family = Some(family);
                    }
                }
                "color" => {
                    if let Some(color) = css_color(value) {
                        style.color = Some(color);
                    }
                }
                "text-align" => frame.alignment = alignment(&lower),
                _ => {}
            }
        }
    }
    frame
}

/// A CSS or HTML alignment as Word's `w:jc`.
fn alignment(value: &str) -> Option<String> {
    Some(
        match value.trim().to_ascii_lowercase().as_str() {
            "center" | "middle" => "center",
            "right" | "end" => "right",
            "justify" => "both",
            "left" | "start" => "left",
            _ => return None,
        }
        .to_string(),
    )
}

/// Builds the clip as the markup is read.
#[derive(Default)]
struct Builder {
    paragraphs: Vec<ClipParagraph>,
    current: ClipParagraph,
    /// White space is pending before the next text, collapsed to one, in
    /// the style of the text it was in.
    space: Option<HeadlessRunStyle>,
    /// Inside `pre`: white space is kept.
    preformatted: usize,
}

impl Builder {
    /// `text` in `style` at the end of the paragraph.
    fn push(&mut self, text: &str, style: &HeadlessRunStyle) {
        if text.is_empty() {
            return;
        }
        match self.current.runs.last_mut() {
            Some(run) if run.style == *style => run.text.push_str(text),
            _ => self.current.runs.push(ClipRun {
                text: text.to_string(),
                style: style.clone(),
            }),
        }
    }

    fn text(&mut self, text: &str, frame: &Frame) {
        if self.current.runs.is_empty() && self.current.alignment.is_none() {
            self.current.alignment = frame.alignment.clone();
        }
        if self.preformatted > 0 {
            self.push(&text.replace('\u{a0}', " "), &frame.style);
            return;
        }
        let mut collapsed = String::new();
        for character in text.chars() {
            if character.is_whitespace() && character != '\u{a0}' {
                if self.space.is_none() {
                    // The space keeps the style of the text it was in.
                    self.push(&std::mem::take(&mut collapsed), &frame.style);
                    self.space = Some(frame.style.clone());
                }
                continue;
            }
            if let Some(style) = self.space.take() {
                // None at a paragraph's start.
                if !self.current.runs.is_empty() {
                    self.push(" ", &style);
                }
            }
            collapsed.push(if character == '\u{a0}' {
                ' '
            } else {
                character
            });
        }
        self.push(&collapsed, &frame.style);
    }

    /// Ends the paragraph; `forced` (a `br`) even when it is empty.
    fn break_paragraph(&mut self, forced: bool) {
        self.space = None;
        if self.current.runs.is_empty() && !forced {
            return;
        }
        self.paragraphs.push(std::mem::take(&mut self.current));
    }

    fn finish(mut self) -> RichClip {
        self.break_paragraph(false);
        // Leading and trailing empty paragraphs are the markup's, not text.
        while self
            .paragraphs
            .last()
            .is_some_and(|paragraph| paragraph.runs.is_empty())
        {
            self.paragraphs.pop();
        }
        while self
            .paragraphs
            .first()
            .is_some_and(|paragraph| paragraph.runs.is_empty())
        {
            self.paragraphs.remove(0);
        }
        RichClip {
            paragraphs: self.paragraphs,
        }
    }
}

impl RichClip {
    /// The clipboard's `text/html` as a copy with its formatting. Only the
    /// fragment between `<!--StartFragment-->` and `<!--EndFragment-->` is
    /// read when the markup marks one, as Windows' and Office's HTML
    /// clipboard formats do.
    pub fn from_html(html: &str) -> RichClip {
        let html = match (
            html.find("<!--StartFragment-->"),
            html.find("<!--EndFragment-->"),
        ) {
            (Some(start), Some(end)) if start < end => &html[start + 20..end],
            _ => html,
        };
        let mut builder = Builder::default();
        let mut stack: Vec<Frame> = vec![Frame::default()];
        let mut skipping: Option<String> = None;
        let mut rest = html;
        while !rest.is_empty() {
            let Some(open) = rest.find('<') else {
                if skipping.is_none() {
                    let frame = stack.last().cloned().unwrap_or_default();
                    builder.text(&decode_entities(rest), &frame);
                }
                break;
            };
            if open > 0 && skipping.is_none() {
                let frame = stack.last().cloned().unwrap_or_default();
                builder.text(&decode_entities(&rest[..open]), &frame);
            }
            rest = &rest[open..];
            if let Some(comment) = rest.strip_prefix("<!--") {
                rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
                continue;
            }
            let Some(close) = rest.find('>') else {
                break;
            };
            let tag = &rest[1..close];
            rest = &rest[close + 1..];
            if tag.starts_with('!') || tag.starts_with('?') {
                continue;
            }
            let closing = tag.starts_with('/');
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == ':')
                .collect::<String>()
                .to_ascii_lowercase();
            // Office writes `o:p` and its kin; their names are not HTML's.
            let name = name.rsplit(':').next().unwrap_or("").to_string();
            if let Some(skipped) = &skipping {
                if closing && *skipped == name {
                    skipping = None;
                }
                continue;
            }
            if SKIPPED.contains(&name.as_str()) {
                if !closing && !tag.trim_end().ends_with('/') {
                    skipping = Some(name);
                }
                continue;
            }
            let block = BLOCKS.contains(&name.as_str());
            if name == "br" {
                builder.break_paragraph(true);
                continue;
            }
            if closing {
                if block {
                    builder.break_paragraph(false);
                }
                if name == "pre" {
                    builder.preformatted = builder.preformatted.saturating_sub(1);
                }
                // Back to the element's parent, closing what it left open.
                if let Some(at) = stack.iter().rposition(|frame| frame.tag == name) {
                    if at > 0 {
                        stack.truncate(at);
                    }
                }
                continue;
            }
            if block {
                builder.break_paragraph(false);
            }
            if matches!(name.as_str(), "td" | "th") && !builder.current.runs.is_empty() {
                // Cells of a row read as one line, a tab between them.
                let frame = stack.last().cloned().unwrap_or_default();
                builder.space = None;
                if let Some(run) = builder.current.runs.last_mut() {
                    run.text.push('\t');
                } else {
                    builder.text("\t", &frame);
                }
            }
            let void = matches!(
                name.as_str(),
                "img" | "hr" | "meta" | "link" | "input" | "wbr" | "col" | "area" | "source"
            ) || tag.trim_end().ends_with('/');
            if void {
                if name == "hr" {
                    builder.break_paragraph(false);
                }
                continue;
            }
            if name == "pre" {
                builder.preformatted += 1;
            }
            let parent = stack.last().cloned().unwrap_or_default();
            stack.push(frame_for(&name, tag, &parent));
        }
        builder.finish()
    }
}
