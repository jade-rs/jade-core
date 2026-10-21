use crate::color::{
    apply_color_transforms, parse_color_transform, parse_explicit_color, parse_hsl_color,
    parse_linear_rgb,
};
use crate::document::ThemeColorTransform;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

type ParseResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ThemeFonts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub east_asia: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complex: Option<String>,
    #[serde(default)]
    pub script_fonts: HashMap<String, String>,
}

impl ThemeFonts {
    fn set_latin(&mut self, value: Option<String>) {
        self.latin = normalize_typeface(value);
    }

    fn set_east_asia(&mut self, value: Option<String>) {
        self.east_asia = normalize_typeface(value);
    }

    fn set_complex(&mut self, value: Option<String>) {
        self.complex = normalize_typeface(value);
    }

    fn insert_script_font(&mut self, script: &str, value: Option<String>) {
        if let Some(normalized) = normalize_typeface(value) {
            self.script_fonts.insert(script.to_string(), normalized);
        }
    }

    fn preferred_latin(&self) -> Option<&str> {
        self.latin.as_deref().filter(|value| !value.is_empty())
    }

    fn preferred_complex(&self) -> Option<&str> {
        self.complex.as_deref().filter(|value| !value.is_empty())
    }

    fn preferred_east_asia(&self) -> Option<&str> {
        if let Some(value) = self.east_asia.as_deref().filter(|value| !value.is_empty()) {
            return Some(value);
        }

        for script in ["Jpan", "Hans", "Hant", "Hang"] {
            if let Some(value) = self.script_fonts.get(script) {
                if !value.is_empty() {
                    return Some(value.as_str());
                }
            }
        }

        self.script_fonts
            .values()
            .find(|value| !value.is_empty())
            .map(|value| value.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ThemePart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub major: ThemeFonts,
    pub minor: ThemeFonts,
    /// DrawingML color scheme keyed by semantic slots such as `accent1`,
    /// `dk1`, and `hlink`, with each palette value's ordered transforms resolved.
    #[serde(default)]
    pub colors: HashMap<String, String>,
}

impl ThemePart {
    pub fn from_xml(xml: &str) -> ParseResult<Self> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut part = ThemePart::default();
        let mut buf = Vec::new();
        let mut in_major = false;
        let mut in_minor = false;
        let mut in_color_scheme = false;
        let mut current_color_slot: Option<String> = None;
        let mut current_color_value: Option<(String, String, Vec<ThemeColorTransform>)> = None;

        loop {
            let event = reader.read_event_into(&mut buf);
            let is_start = matches!(&event, Ok(Event::Start(_)));
            match event {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    let name = local_name(name.as_ref());
                    match name {
                        b"clrScheme" => {
                            in_color_scheme = true;
                            part.name = attr_value(e, b"name").filter(|value| !value.is_empty());
                        }
                        b"majorFont" => in_major = true,
                        b"minorFont" => in_minor = true,
                        b"srgbClr" if in_color_scheme => {
                            current_color_value = None;
                            if let (Some(slot), Some(color)) = (
                                current_color_slot.as_ref(),
                                attr_value(e, b"val").and_then(normalize_hex_color),
                            ) {
                                part.colors.insert(slot.clone(), color.clone());
                                current_color_value =
                                    is_start.then(|| (slot.clone(), color, Vec::new()));
                            }
                        }
                        b"sysClr" if in_color_scheme => {
                            current_color_value = None;
                            if let (Some(slot), Some(color)) = (
                                current_color_slot.as_ref(),
                                attr_value(e, b"lastClr")
                                    .or_else(|| attr_value(e, b"val"))
                                    .and_then(normalize_hex_color),
                            ) {
                                part.colors.insert(slot.clone(), color.clone());
                                current_color_value =
                                    is_start.then(|| (slot.clone(), color, Vec::new()));
                            }
                        }
                        b"prstClr" if in_color_scheme => {
                            current_color_value = None;
                            if let (Some(slot), Some(color)) =
                                (current_color_slot.as_ref(), parse_explicit_color(e))
                            {
                                part.colors.insert(slot.clone(), color.clone());
                                current_color_value =
                                    is_start.then(|| (slot.clone(), color, Vec::new()));
                            }
                        }
                        b"scrgbClr" | b"hslClr" if in_color_scheme => {
                            current_color_value = None;
                            if let (Some(slot), Some(color)) = (
                                current_color_slot.as_ref(),
                                if name == b"hslClr" {
                                    parse_hsl_color(e)
                                } else {
                                    parse_linear_rgb(e)
                                },
                            ) {
                                part.colors.insert(slot.clone(), color.clone());
                                current_color_value =
                                    is_start.then(|| (slot.clone(), color, Vec::new()));
                            }
                        }
                        b"lumMod" | b"lumOff" | b"tint" | b"shade" => {
                            if let (Some((_, _, transforms)), Some(transform)) =
                                (current_color_value.as_mut(), parse_color_transform(e))
                            {
                                transforms.push(transform);
                            }
                        }
                        b"latin" => {
                            let value = attr_value(e, b"typeface");
                            if in_major {
                                part.major.set_latin(value);
                            } else if in_minor {
                                part.minor.set_latin(value);
                            }
                        }
                        b"ea" => {
                            let value = attr_value(e, b"typeface");
                            if in_major {
                                part.major.set_east_asia(value);
                            } else if in_minor {
                                part.minor.set_east_asia(value);
                            }
                        }
                        b"cs" => {
                            let value = attr_value(e, b"typeface");
                            if in_major {
                                part.major.set_complex(value);
                            } else if in_minor {
                                part.minor.set_complex(value);
                            }
                        }
                        b"font" => {
                            if in_major || in_minor {
                                let script = attr_value(e, b"script");
                                let typeface = attr_value(e, b"typeface");
                                if let Some(script) = script {
                                    if in_major {
                                        part.major.insert_script_font(&script, typeface);
                                    } else {
                                        part.minor.insert_script_font(&script, typeface);
                                    }
                                }
                            }
                        }
                        _ if in_color_scheme && is_theme_color_slot(name) => {
                            current_color_slot =
                                Some(String::from_utf8_lossy(name).to_ascii_lowercase());
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = e.name();
                    let name = local_name(name.as_ref());
                    match name {
                        b"clrScheme" => {
                            in_color_scheme = false;
                            current_color_slot = None;
                            current_color_value = None;
                        }
                        b"srgbClr" | b"sysClr" | b"scrgbClr" | b"hslClr" | b"prstClr" => {
                            if let Some((slot, color, transforms)) = current_color_value.take() {
                                if let Some(resolved) = apply_color_transforms(&color, &transforms)
                                {
                                    part.colors.insert(slot, resolved);
                                }
                            }
                        }
                        b"majorFont" => in_major = false,
                        b"minorFont" => in_minor = false,
                        _ if is_theme_color_slot(name) => {
                            current_color_slot = None;
                            current_color_value = None;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("Theme XML parsing error: {err}").into()),
                _ => {}
            }
            buf.clear();
        }

        Ok(part)
    }

    pub fn resolve_theme_font(&self, key: &str) -> Option<&str> {
        let key = key.trim().as_bytes();
        let prefix = key.get(..5)?;
        let fonts = if prefix.eq_ignore_ascii_case(b"major") {
            &self.major
        } else if prefix.eq_ignore_ascii_case(b"minor") {
            &self.minor
        } else {
            return None;
        };
        let contains = |part: &[u8]| {
            key.windows(part.len())
                .any(|window| window.eq_ignore_ascii_case(part))
        };
        if contains(b"eastasia") {
            return fonts.preferred_east_asia();
        }
        if contains(b"bidi") || contains(b"cs") {
            return fonts.preferred_complex();
        }
        fonts.preferred_latin()
    }

    pub fn resolve_theme_token(&self, token: &str) -> Option<&str> {
        let token = token.trim();
        for (key, fonts, slot) in [
            ("+mn-lt", &self.minor, 0),
            ("+mj-lt", &self.major, 0),
            ("+mn-ea", &self.minor, 1),
            ("+mj-ea", &self.major, 1),
            ("+mn-cs", &self.minor, 2),
            ("+mj-cs", &self.major, 2),
        ] {
            if token.eq_ignore_ascii_case(key) {
                return match slot {
                    0 => fonts.preferred_latin(),
                    1 => fonts.preferred_east_asia(),
                    _ => fonts.preferred_complex(),
                };
            }
        }
        None
    }

    /// Whether resolving this run can fill a face or replace a theme token.
    pub fn has_resolvable_font_reference(&self, props: &crate::document::RunProperties) -> bool {
        let Some(fonts) = props.fonts.as_ref() else {
            return false;
        };
        [
            (&fonts.ascii, &fonts.ascii_theme),
            (&fonts.h_ansi, &fonts.h_ansi_theme),
            (&fonts.east_asia, &fonts.east_asia_theme),
            (&fonts.complex, &fonts.complex_theme),
        ]
        .into_iter()
        .any(|(face, theme)| match face.as_deref() {
            Some(face) => face.trim().starts_with('+') && self.resolve_theme_token(face).is_some(),
            None => theme
                .as_deref()
                .is_some_and(|key| self.resolve_theme_font(key).is_some()),
        })
    }

    pub fn resolve_theme_color(&self, key: &str) -> Option<&str> {
        self.colors
            .get(&key.trim().to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Resolves every theme font reference on `props` to a concrete face:
    /// `+mn-lt`-style tokens stored in the name slots, and `w:asciiTheme`-style
    /// attributes filling slots that have no explicit name. Shared by the
    /// editor's load-time flattening and the headless/PDF layout path so both
    /// lay out theme-font documents with the same faces.
    pub fn apply_to_run_properties(&self, props: &mut crate::document::RunProperties) {
        if self.has_resolvable_font_reference(props) {
            let fonts = props.fonts.as_mut().expect("resolvable font reference");

            let resolve_slot =
                |slot: &mut Option<std::rc::Rc<str>>, theme_key: Option<&std::rc::Rc<str>>| {
                    if let Some(value) = slot.as_deref() {
                        if let Some(font) = self.resolve_theme_token(value) {
                            *slot = Some(std::rc::Rc::from(font));
                        }
                    } else if let Some(theme_key) = theme_key {
                        if let Some(font) = self.resolve_theme_font(theme_key) {
                            *slot = Some(std::rc::Rc::from(font));
                        }
                    }
                };

            let ascii_theme = fonts.ascii_theme.clone();
            let h_ansi_theme = fonts.h_ansi_theme.clone();
            let east_asia_theme = fonts.east_asia_theme.clone();
            let complex_theme = fonts.complex_theme.clone();
            resolve_slot(&mut fonts.ascii, ascii_theme.as_ref());
            resolve_slot(&mut fonts.h_ansi, h_ansi_theme.as_ref());
            resolve_slot(&mut fonts.east_asia, east_asia_theme.as_ref());
            resolve_slot(&mut fonts.complex, complex_theme.as_ref());
        }

        if props.font_family.is_none() {
            if let Some(family) = props
                .fonts
                .as_ref()
                .and_then(|fonts| fonts.preferred_latin())
            {
                props.font_family = Some(std::rc::Rc::from(family));
            }
        }
    }
}

fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|b| *b == b':') {
        Some(idx) => &name[idx + 1..],
        None => name,
    }
}

fn attr_value(element: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in element.attributes().flatten() {
        let attr_name = local_name(attr.key.as_ref());
        if attr_name == key {
            let encoded = String::from_utf8_lossy(&attr.value);
            return unescape(&encoded).ok().map(|value| value.into_owned());
        }
    }
    None
}

fn normalize_typeface(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn is_theme_color_slot(name: &[u8]) -> bool {
    matches!(
        name,
        b"dk1"
            | b"lt1"
            | b"dk2"
            | b"lt2"
            | b"accent1"
            | b"accent2"
            | b"accent3"
            | b"accent4"
            | b"accent5"
            | b"accent6"
            | b"hlink"
            | b"folHlink"
    )
}

std::thread_local! {
    /// The colour scheme of the document being read, while one is: text
    /// that names a theme colour (`w:color w:themeColor`) takes its colour
    /// from it, as Word draws it, rather than from the cached `w:val`.
    static READING_COLORS: std::cell::RefCell<Option<HashMap<String, String>>> =
        const { std::cell::RefCell::new(None) };
}

/// While alive, run colours that name a theme slot resolve against
/// `theme`'s colour scheme (see [`text_theme_color`]). Dropping it restores
/// whatever scope was there before.
pub struct ThemeColorScope {
    previous: Option<HashMap<String, String>>,
}

impl ThemeColorScope {
    pub fn enter(theme: Option<&ThemePart>) -> Self {
        let colors = theme.map(|theme| theme.colors.clone());
        let previous = READING_COLORS.with(|cell| cell.replace(colors));
        ThemeColorScope { previous }
    }
}

impl Drop for ThemeColorScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        READING_COLORS.with(|cell| *cell.borrow_mut() = previous);
    }
}

/// `w:color`'s theme reference (ECMA-376 Part 1 §17.3.2.6) in the colour
/// scheme being read: `themeColor` names the slot (`accent1`, `text1`, …,
/// mapped as Word's default `w:clrSchemeMapping` does), `themeTint` and
/// `themeShade` (hex bytes) lighten and darken it in HSL, as Word does.
/// `None` outside a [`ThemeColorScope`] or for an unknown slot.
pub fn text_theme_color(
    theme_color: &str,
    tint: Option<&str>,
    shade: Option<&str>,
) -> Option<String> {
    let slot = match theme_color {
        "dark1" | "text1" => "dk1",
        "light1" | "background1" => "lt1",
        "dark2" | "text2" => "dk2",
        "light2" | "background2" => "lt2",
        "hyperlink" => "hlink",
        "followedHyperlink" => "folHlink",
        other if other.starts_with("accent") => other,
        _ => return None,
    };
    let base = READING_COLORS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|colors| colors.get(slot).cloned())
    })?;
    let hex = base.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    let fraction = |value: Option<&str>| {
        value
            .and_then(|value| u8::from_str_radix(value.trim(), 16).ok())
            .map(|value| value as f64 / 255.0)
    };
    let (h, s, mut l) = rgb_to_hsl(r, g, b);
    if let Some(tint) = fraction(tint) {
        l = l * tint + (1.0 - tint);
    }
    if let Some(shade) = fraction(shade) {
        l *= shade;
    }
    let (r, g, b) = hsl_to_rgb(h, s, l.clamp(0.0, 1.0));
    Some(format!("#{r:02X}{g:02X}{b:02X}"))
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let to = |value: f64| (value * 255.0).round().clamp(0.0, 255.0) as u8;
    if s <= f64::EPSILON {
        return (to(l), to(l), to(l));
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (to(hue(h + 1.0 / 3.0)), to(hue(h)), to(hue(h - 1.0 / 3.0)))
}

fn normalize_hex_color(value: String) -> Option<String> {
    let value = value.trim().trim_start_matches('#');
    (value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("#{}", value.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_theme_colours_follow_the_scheme_being_read() {
        let mut theme = ThemePart::default();
        theme.colors.insert("accent1".into(), "#4472C4".into());
        theme.colors.insert("dk1".into(), "#000000".into());
        assert_eq!(text_theme_color("accent1", None, None), None);
        {
            let _scope = ThemeColorScope::enter(Some(&theme));
            assert_eq!(
                text_theme_color("accent1", None, None).as_deref(),
                Some("#4472C4")
            );
            // Word's heading 1: accent1 at shade BF.
            assert_eq!(
                text_theme_color("accent1", None, Some("BF")).as_deref(),
                Some("#2F5496")
            );
            assert_eq!(
                text_theme_color("text1", Some("80"), None).as_deref(),
                Some("#7F7F7F")
            );
        }
        assert_eq!(text_theme_color("accent1", None, None), None);
    }

    #[test]
    fn inherited_theme_faces_resolve_once_without_replacing_explicit_faces() {
        use crate::document::{RunFonts, RunProperties};
        let theme = ThemePart {
            major: ThemeFonts {
                latin: Some("Aptos Display".into()),
                ..ThemeFonts::default()
            },
            minor: ThemeFonts {
                latin: Some("Aptos".into()),
                east_asia: Some("MS Mincho".into()),
                ..ThemeFonts::default()
            },
            ..ThemePart::default()
        };
        assert_eq!(
            theme.resolve_theme_font("  MaJoRhAnSi  "),
            Some("Aptos Display")
        );
        assert_eq!(theme.resolve_theme_font("MiNoReAsTaSiA"), Some("MS Mincho"));
        assert_eq!(theme.resolve_theme_token("  +MN-LT  "), Some("Aptos"));
        assert_eq!(theme.resolve_theme_token("+mn-cs"), None);
        let mut inherited = RunProperties {
            fonts: Some(RunFonts {
                ascii_theme: Some("minorHAnsi".into()),
                h_ansi_theme: Some("minorHAnsi".into()),
                east_asia_theme: Some("minorEastAsia".into()),
                complex_theme: Some("minorBidi".into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };
        assert!(theme.has_resolvable_font_reference(&inherited));
        theme.apply_to_run_properties(&mut inherited);
        assert_eq!(
            inherited.fonts.as_ref().unwrap().ascii.as_deref(),
            Some("Aptos")
        );
        assert_eq!(
            inherited.fonts.as_ref().unwrap().east_asia.as_deref(),
            Some("MS Mincho")
        );
        assert!(!theme.has_resolvable_font_reference(&inherited));
        let resolved = inherited.clone();
        theme.apply_to_run_properties(&mut inherited);
        assert_eq!(inherited, resolved);
        inherited.fonts.as_mut().unwrap().ascii = Some("Arial".into());
        theme.apply_to_run_properties(&mut inherited);
        assert_eq!(
            inherited.fonts.as_ref().unwrap().ascii.as_deref(),
            Some("Arial")
        );
    }

    #[test]
    fn parses_theme_fonts() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Test">
  <a:themeElements>
    <a:fontScheme name="Test">
      <a:majorFont>
        <a:latin typeface="Cambria"/>
        <a:ea typeface=""/>
        <a:cs typeface="Times New Roman"/>
        <a:font script="Jpan" typeface="ＭＳ Ｐゴシック"/>
      </a:majorFont>
      <a:minorFont>
        <a:latin typeface="Calibri"/>
        <a:ea typeface=""/>
        <a:cs typeface="Times New Roman"/>
        <a:font script="Jpan" typeface="ＭＳ Ｐゴシック"/>
      </a:minorFont>
    </a:fontScheme>
  </a:themeElements>
</a:theme>
"#;

        let theme = ThemePart::from_xml(xml).expect("parse theme");
        assert_eq!(theme.major.latin.as_deref(), Some("Cambria"));
        assert_eq!(theme.minor.latin.as_deref(), Some("Calibri"));
        assert_eq!(
            theme.resolve_theme_font("majorEastAsia"),
            Some("ＭＳ Ｐゴシック")
        );
        assert_eq!(theme.resolve_theme_token("+mn-lt"), Some("Calibri"));
        assert_eq!(theme.resolve_theme_token("+mj-ea"), Some("ＭＳ Ｐゴシック"));
    }

    #[test]
    fn resolves_ordered_palette_transforms_only_inside_color_values() {
        let xml = r#"<d:theme xmlns:d="http://schemas.openxmlformats.org/drawingml/2006/main">
          <d:themeElements><d:clrScheme name="Transformed">
            <d:accent1><d:srgbClr val="0000FF"><d:shade val="50000"/><d:tint val="50000"/></d:srgbClr><d:shade val="0"/></d:accent1>
            <d:accent2><d:srgbClr val="0000FF"><d:tint val="50000"/><d:shade val="50000"/></d:srgbClr></d:accent2>
            <d:dk1><d:sysClr val="windowText" lastClr="0000FF"><d:lumMod val="50000"/><d:lumOff val="12500"/></d:sysClr></d:dk1>
            <d:lt1><d:sysClr val="window" lastClr="FAFBFC"/><d:tint val="0"/></d:lt1>
            <d:accent3><d:srgbClr val="123456"><d:shade val="NaN"/><d:tint val="Infinity"/></d:srgbClr></d:accent3>
          </d:clrScheme><d:fmtScheme name="Other"><d:accent1><d:srgbClr val="FFFFFF"><d:shade val="0"/></d:srgbClr></d:accent1></d:fmtScheme></d:themeElements>
        </d:theme>"#;
        let theme = ThemePart::from_xml(xml).expect("parse transformed palette");
        assert_eq!(theme.resolve_theme_color("accent1"), Some("#8080BF"));
        assert_eq!(theme.resolve_theme_color("accent2"), Some("#404080"));
        assert_eq!(theme.resolve_theme_color("dk1"), Some("#0000BF"));
        assert_eq!(theme.resolve_theme_color("lt1"), Some("#FAFBFC"));
        assert_eq!(theme.resolve_theme_color("accent3"), Some("#123456"));
    }

    #[test]
    fn resolves_preset_palette_colors_and_scoped_transforms() {
        for (name, expected) in [
            ("blue", "#0000FF"),
            ("dkBlue", "#00008B"),
            ("darkBlue", "#00008B"),
            ("gray", "#808080"),
            ("grey", "#808080"),
            ("green", "#008000"),
            ("medAquamarine", "#66CDAA"),
            ("mediumAquamarine", "#66CDAA"),
            ("ltGoldenrodYellow", "#FAFAD2"),
            ("lightGoldenrodYellow", "#FAFAD2"),
        ] {
            let xml = format!(
                r#"<d:theme xmlns:d="http://schemas.openxmlformats.org/drawingml/2006/main"><d:themeElements><d:clrScheme><d:accent1><d:prstClr val="{name}"/></d:accent1><d:accent2><d:prstClr val="blue"><d:shade val="50000"/><d:tint val="50000"/></d:prstClr><d:shade val="0"/></d:accent2></d:clrScheme><d:fmtScheme><d:accent1><d:prstClr val="red"/></d:accent1></d:fmtScheme></d:themeElements></d:theme>"#
            );
            let theme = ThemePart::from_xml(&xml).unwrap();
            assert_eq!(
                theme.colors.get("accent1").map(String::as_str),
                Some(expected),
                "{name}"
            );
            assert_eq!(
                theme.colors.get("accent2").map(String::as_str),
                Some("#8080BF")
            );
        }
        for name in ["Blue", "rebeccapurple", "constructor", "__proto__", ""] {
            let xml = format!(
                r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme><a:accent1><a:srgbClr val="112233"/><a:prstClr val="{name}"><a:tint val="0"/></a:prstClr><a:shade val="0"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
            );
            let theme = ThemePart::from_xml(&xml).unwrap();
            assert_eq!(
                theme.colors.get("accent1").map(String::as_str),
                Some("#112233")
            );
        }
    }

    #[test]
    fn resolves_hsl_palette_colors_and_scoped_transforms() {
        for (hue, expected) in [
            (0, "#FF0000"),
            (2_700_000, "#FFBF00"),
            (3_600_000, "#FFFF00"),
            (7_200_000, "#00FF00"),
            (10_800_000, "#00FFFF"),
            (14_400_000, "#0000FF"),
            (18_000_000, "#FF00FF"),
        ] {
            for (sat, lum) in [("100000", "50000"), ("100.000%", "50.000%")] {
                let xml = format!(
                    r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="HSL"><a:accent1><a:hslClr hue="{hue}" sat="{sat}" lum="{lum}"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
                );
                let theme = ThemePart::from_xml(&xml).unwrap();
                assert_eq!(
                    theme.colors.get("accent1").map(String::as_str),
                    Some(expected)
                );
            }
        }
        let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="HSL">
            <a:accent1><a:hslClr hue="14400000" sat="100%" lum="25%"><a:shade val="40000"/><a:tint val="50000"/></a:hslClr><a:shade val="0"/></a:accent1>
            <a:accent2><a:hslClr hue="21599999" sat="0" lum="50000"/></a:accent2>
            <a:accent3><a:hslClr hue="0" sat="100000" lum="0"/></a:accent3>
            <a:accent4><a:hslClr hue="0" sat="100000" lum="100000"/></a:accent4>
            <a:accent5><a:hslClr hue="0" sat="-10000" lum="200000"/></a:accent5>
            <a:accent6><a:hslClr hue="0" sat="50%" lum="50%"/></a:accent6>
        </a:clrScheme><a:fmtScheme><a:hslClr hue="0" sat="100000" lum="50000"/></a:fmtScheme></a:themeElements></a:theme>"#;
        let theme = ThemePart::from_xml(xml).unwrap();
        for (slot, expected) in [
            ("accent1", "#808099"),
            ("accent2", "#808080"),
            ("accent3", "#000000"),
            ("accent4", "#FFFFFF"),
            ("accent5", "#FFFFFF"),
            ("accent6", "#BF4040"),
        ] {
            assert_eq!(
                theme.colors.get(slot).map(String::as_str),
                Some(expected),
                "{slot}"
            );
        }
    }

    #[test]
    fn rejects_malformed_hsl_palette_components() {
        for attrs in [
            r#"hue="0" sat="100000""#,
            r#"sat="100000" lum="50000""#,
            r#"hue="-1" sat="100000" lum="50000""#,
            r#"hue="21600000" sat="100000" lum="50000""#,
            r#"hue="0.0" sat="100000" lum="50000""#,
            r#"hue="0%" sat="100000" lum="50000""#,
            r#"hue="0" sat="NaN" lum="50000""#,
            r#"hue="0" sat="100000" lum="Infinity""#,
            r#"hue="0" sat="1e5" lum="50000""#,
            r#"hue="0" sat="100000.0" lum="50000""#,
            r#"hue="0" sat=".5%" lum="50000""#,
            r#"hue="0" sat="50.%" lum="50000""#,
            r#"hue="0" sat="50%%" lum="50000""#,
            r#"hue="0" sat="2147483648" lum="50000""#,
        ] {
            let xml = format!(
                r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="HSL"><a:accent1><a:hslClr {attrs}><a:tint val="50000"/></a:hslClr></a:accent1><a:accent2><a:srgbClr val="123456"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#
            );
            let theme = ThemePart::from_xml(&xml).unwrap();
            assert!(!theme.colors.contains_key("accent1"), "{attrs}");
            assert_eq!(
                theme.colors.get("accent2").map(String::as_str),
                Some("#123456")
            );
        }
    }

    #[test]
    fn resolves_linear_rgb_palette_values_and_scoped_transforms() {
        let xml = r#"<d:theme xmlns:d="http://schemas.openxmlformats.org/drawingml/2006/main"><d:themeElements>
          <d:clrScheme name="Linear">
            <d:dk1><d:scrgbClr r="50000" g="50000" b="50000"/></d:dk1>
            <d:lt1><d:scrgbClr r="50.000%" g="50%" b="50%"/></d:lt1>
            <d:accent1><d:scrgbClr r="0" g="0" b="100000"><d:shade val="40000"/><d:tint val="50000"/></d:scrgbClr><d:shade val="0"/></d:accent1>
            <d:accent2><d:scrgbClr r="-10000" g="200000" b="0"/></d:accent2>
            <d:accent3><d:scrgbClr r="313" g="314" b="0"/></d:accent3>
            <d:accent4><d:scrgbClr r="0%" g="0%" b="100%"><d:lumMod val="50000"/></d:scrgbClr></d:accent4>
          </d:clrScheme><d:fmtScheme><d:dk1><d:scrgbClr r="0" g="0" b="0"/></d:dk1></d:fmtScheme>
        </d:themeElements></d:theme>"#;
        let theme = ThemePart::from_xml(xml).expect("parse linear palette");
        // ISO/IEC 29500's scRGB example equates linear 50% with sRGB BC.
        assert_eq!(theme.resolve_theme_color("dk1"), Some("#BCBCBC"));
        assert_eq!(theme.resolve_theme_color("lt1"), Some("#BCBCBC"));
        assert_eq!(theme.resolve_theme_color("accent1"), Some("#8080B3"));
        assert_eq!(theme.resolve_theme_color("accent2"), Some("#00FF00"));
        assert_eq!(theme.resolve_theme_color("accent3"), Some("#0A0A00"));
        assert_eq!(theme.resolve_theme_color("accent4"), Some("#000080"));
    }

    #[test]
    fn rejects_malformed_linear_rgb_palette_components() {
        for components in [
            r#"r="NaN" g="0" b="0""#,
            r#"r="0" g="Infinity" b="0""#,
            r#"r="0" g="0""#,
            r#"r="" g="0" b="0""#,
            r#"r="50.0" g="0" b="0""#,
            r#"r="5e4" g="0" b="0""#,
            r#"r="50%%" g="0" b="0""#,
            r#"r=".5%" g="0" b="0""#,
            r#"r="50.%" g="0" b="0""#,
            r#"r="2147483648" g="0" b="0""#,
        ] {
            let xml = format!(
                r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme><a:accent1><a:scrgbClr {components}><a:tint val="0"/></a:scrgbClr></a:accent1><a:accent2><a:srgbClr val="123456"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#
            );
            let theme = ThemePart::from_xml(&xml).expect("parse malformed linear palette");
            assert_eq!(theme.resolve_theme_color("accent1"), None, "{components}");
            assert_eq!(theme.resolve_theme_color("accent2"), Some("#123456"));
        }
    }

    #[test]
    fn parses_theme_color_scheme_slots() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Colors">
  <a:themeElements>
    <a:clrScheme name="Custom">
      <a:dk1><a:sysClr val="windowText" lastClr="102030"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FAFBFC"/></a:lt1>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
    </a:clrScheme>
  </a:themeElements>
</a:theme>
"#;

        let theme = ThemePart::from_xml(xml).expect("parse color scheme");
        assert_eq!(theme.resolve_theme_color("accent1"), Some("#4472C4"));
        assert_eq!(theme.resolve_theme_color("ACCENT2"), Some("#ED7D31"));
        // System colours use their stable lastClr fallback outside Word's host
        // OS theme, matching how Office preserves a portable display colour.
        assert_eq!(theme.resolve_theme_color("dk1"), Some("#102030"));
        assert_eq!(theme.resolve_theme_color("lt1"), Some("#FAFBFC"));
        assert_eq!(theme.resolve_theme_color("hlink"), Some("#0563C1"));
    }
}
