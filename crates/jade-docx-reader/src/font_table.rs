//! `word/fontTable.xml` — the document's font manifest.
//!
//! Word records every referenced font here together with substitution
//! metadata: `w:altName` names the face the authoring machine substituted (or
//! the author's declared fallback), and `w:panose1` classifies the design for
//! metric-compatible substitution. jade uses the alt names to extend a
//! family's CSS stack so a host without the primary face falls back the same
//! way Word would, instead of relying only on a hard-coded alias table.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

type ParseResult<T> = Result<T, Box<dyn std::error::Error>>;

/// One `<w:font>` declaration.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FontDeclaration {
    pub name: String,
    /// `w:altName` — the substitute face Word records for this font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_name: Option<String>,
    /// `w:panose1` — the ten-byte PANOSE classification, kept verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panose: Option<String>,
    /// `w:embedRegular` / `w:embedBold` / `w:embedItalic` /
    /// `w:embedBoldItalic`: font files the author embedded for this family
    /// (see [`crate::embedded_font`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedded: Vec<EmbeddedFontReference>,
}

/// Which face of a family an embedded font file supplies.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EmbeddedFontStyle {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl EmbeddedFontStyle {
    fn from_element(local_name: &[u8]) -> Option<Self> {
        match local_name {
            b"embedRegular" => Some(Self::Regular),
            b"embedBold" => Some(Self::Bold),
            b"embedItalic" => Some(Self::Italic),
            b"embedBoldItalic" => Some(Self::BoldItalic),
            _ => None,
        }
    }

    pub fn is_bold(self) -> bool {
        matches!(self, Self::Bold | Self::BoldItalic)
    }

    pub fn is_italic(self) -> bool {
        matches!(self, Self::Italic | Self::BoldItalic)
    }
}

/// One `w:embed*` element (ECMA-376 Part 1 §17.8.3.3-6).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedFontReference {
    pub style: EmbeddedFontStyle,
    /// `r:id`: the font part, relative to the fontTable part's relationships.
    pub relationship_id: String,
    /// `w:fontKey`: the GUID that obfuscates the part (§17.8.1). Absent for a
    /// font stored in the clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_key: Option<String>,
    /// `w:subsetted`: the file only holds the glyphs the document uses.
    #[serde(default)]
    pub subsetted: bool,
}

/// Parsed `word/fontTable.xml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FontTablePart {
    pub fonts: Vec<FontDeclaration>,
}

impl FontTablePart {
    pub fn from_xml(xml: &str) -> ParseResult<Self> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut part = FontTablePart::default();
        let mut current: Option<FontDeclaration> = None;
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    let name = local_name(name.as_ref());
                    match name {
                        b"font" => {
                            let font_name = attr_value(e, b"w:name");
                            current = font_name.map(|name| FontDeclaration {
                                name,
                                ..FontDeclaration::default()
                            });
                        }
                        b"altName" => {
                            if let Some(current) = current.as_mut() {
                                current.alt_name = attr_value(e, b"w:val")
                                    .map(|value| value.trim().to_string())
                                    .filter(|value| !value.is_empty());
                            }
                        }
                        b"panose1" => {
                            if let Some(current) = current.as_mut() {
                                current.panose = attr_value(e, b"w:val")
                                    .map(|value| value.trim().to_string())
                                    .filter(|value| !value.is_empty());
                            }
                        }
                        other => {
                            if let (Some(current), Some(style)) =
                                (current.as_mut(), EmbeddedFontStyle::from_element(other))
                            {
                                if let Some(reference) = embedded_font_reference(e, style) {
                                    current.embedded.push(reference);
                                }
                            }
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = e.name();
                    if local_name(name.as_ref()) == b"font" {
                        if let Some(font) = current.take() {
                            part.fonts.push(font);
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("fontTable XML parsing error: {err}").into()),
                _ => {}
            }
            buf.clear();
        }

        // An empty `<w:font ...>` never produces an End event pairing; flush
        // any dangling declaration (Empty events push into `current` above).
        if let Some(font) = current.take() {
            part.fonts.push(font);
        }

        Ok(part)
    }

    /// The alt-name substitution map: primary family → substitutes. Keys keep
    /// the authored spelling; consumers normalize for lookup.
    pub fn alt_names(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fonts.iter().filter_map(|font| {
            let alt = font.alt_name.as_deref()?;
            // A self-referential alt name adds nothing to a fallback stack.
            (!alt.eq_ignore_ascii_case(&font.name)).then_some((font.name.as_str(), alt))
        })
    }
}

fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|&byte| byte == b':') {
        Some(index) => &name[index + 1..],
        None => name,
    }
}

fn embedded_font_reference(
    element: &quick_xml::events::BytesStart<'_>,
    style: EmbeddedFontStyle,
) -> Option<EmbeddedFontReference> {
    let mut relationship_id = None;
    let mut font_key = None;
    let mut subsetted = false;
    for attr in element.attributes().flatten() {
        let value = String::from_utf8_lossy(&attr.value).trim().to_string();
        match local_name(attr.key.as_ref()) {
            b"id" if !value.is_empty() => relationship_id = Some(value),
            b"fontKey" if !value.is_empty() => font_key = Some(value),
            b"subsetted" => subsetted = !matches!(value.as_str(), "0" | "false" | "off"),
            _ => {}
        }
    }
    Some(EmbeddedFontReference {
        style,
        relationship_id: relationship_id?,
        font_key,
        subsetted,
    })
}

fn attr_value(element: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    element.attributes().flatten().find_map(|attr| {
        (attr.key.as_ref() == name)
            .then(|| String::from_utf8_lossy(&attr.value).to_string())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_font_declarations_with_alt_names_and_panose() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:font w:name="ＭＳ 明朝">
    <w:altName w:val="MS Mincho"/>
    <w:panose1 w:val="02020609040205080304"/>
  </w:font>
  <w:font w:name="HGP創英角ﾎﾟｯﾌﾟ体">
    <w:altName w:val="HGPSoeiKakupoptai"/>
  </w:font>
  <w:font w:name="Century">
    <w:panose1 w:val="02040604050505020304"/>
  </w:font>
  <w:font w:name="SelfAlias">
    <w:altName w:val="selfalias"/>
  </w:font>
</w:fonts>
"#;
        let part = FontTablePart::from_xml(xml).expect("parse fontTable");
        assert_eq!(part.fonts.len(), 4);
        assert_eq!(part.fonts[0].name, "ＭＳ 明朝");
        assert_eq!(part.fonts[0].alt_name.as_deref(), Some("MS Mincho"));
        assert_eq!(
            part.fonts[0].panose.as_deref(),
            Some("02020609040205080304")
        );
        assert_eq!(part.fonts[2].alt_name, None);

        assert!(part.fonts.iter().all(|font| font.embedded.is_empty()));
        let alt_names: Vec<(&str, &str)> = part.alt_names().collect();
        assert_eq!(
            alt_names,
            vec![
                ("ＭＳ 明朝", "MS Mincho"),
                ("HGP創英角ﾎﾟｯﾌﾟ体", "HGPSoeiKakupoptai"),
            ],
            "fonts without an alt name — and self-referential ones — are skipped"
        );
    }

    #[test]
    fn parses_embedded_font_references() {
        let xml = r#"<w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:font w:name="Jade Sans">
    <w:embedRegular r:id="rId1" w:fontKey="{0EC2A5B4-6A1C-4C5B-9D3E-1F2A3B4C5D6E}"/>
    <w:embedBold r:id="rId2" w:fontKey="{11111111-2222-3333-4444-555555555555}" w:subsetted="1"/>
    <w:embedItalic w:fontKey="{11111111-2222-3333-4444-555555555555}"/>
  </w:font>
</w:fonts>"#;
        let part = FontTablePart::from_xml(xml).expect("parse fontTable");
        let embedded = &part.fonts[0].embedded;
        assert_eq!(embedded.len(), 2, "an embed without r:id names no part");
        assert_eq!(embedded[0].style, EmbeddedFontStyle::Regular);
        assert_eq!(embedded[0].relationship_id, "rId1");
        assert_eq!(
            embedded[0].font_key.as_deref(),
            Some("{0EC2A5B4-6A1C-4C5B-9D3E-1F2A3B4C5D6E}")
        );
        assert!(!embedded[0].subsetted);
        assert_eq!(embedded[1].style, EmbeddedFontStyle::Bold);
        assert!(embedded[1].subsetted);
        assert!(embedded[1].style.is_bold() && !embedded[1].style.is_italic());
    }
}
