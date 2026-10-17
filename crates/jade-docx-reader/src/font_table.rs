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
                                alt_name: None,
                                panose: None,
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
                        _ => {}
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
}
