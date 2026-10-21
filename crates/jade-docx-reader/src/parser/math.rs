use crate::document::{MathElement, MathElementKind, MathParagraph};
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MathParseError {
    #[error("OMML parsing error: {0}")]
    Xml(String),
    #[error("OMML parsing error: unexpected closing tag")]
    UnexpectedClose,
}

pub fn parse_math_elements(raw_xml: &str) -> Result<Vec<MathElement>, MathParseError> {
    let mut reader = Reader::from_str(raw_xml);
    reader.config_mut().trim_text(false);

    let mut buf = Vec::new();
    let mut stack: Vec<(MathElement, String)> = Vec::new();
    let mut roots: Vec<MathElement> = Vec::new();

    // Text arrives in pieces: quick-xml reports `&amp;` and `&#x2211;` as
    // references between text events, so an element's text is the pieces
    // joined, decided once the element closes.
    fn finish(mut element: MathElement, text: String) -> MathElement {
        if !text.is_empty()
            && (matches!(element.kind, MathElementKind::Text) || !text.trim().is_empty())
        {
            element.text = Some(text);
        }
        if element.children.is_empty() {
            element.children.shrink_to_fit();
        }
        element
    }

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let element = build_element(e)?;
                stack.push((element, String::new()));
            }
            Ok(Event::End(_)) => {
                let Some((element, text)) = stack.pop() else {
                    return Err(MathParseError::UnexpectedClose);
                };
                let element = finish(element, text);
                if let Some((parent, _)) = stack.last_mut() {
                    parent.children.push(element);
                } else {
                    roots.push(element);
                }
            }
            Ok(Event::Empty(ref e)) => {
                let element = build_element(e)?;
                if let Some((parent, _)) = stack.last_mut() {
                    parent.children.push(element);
                } else {
                    roots.push(element);
                }
            }
            Ok(Event::Text(e)) => {
                if let Some((_, text)) = stack.last_mut() {
                    let decoded = e
                        .decode()
                        .map_err(|err| MathParseError::Xml(err.to_string()))?;
                    let unescaped = unescape(decoded.as_ref())
                        .map_err(|err| MathParseError::Xml(err.to_string()))?;
                    text.push_str(&unescaped);
                }
            }
            Ok(Event::GeneralRef(e)) => {
                if let Some((_, text)) = stack.last_mut() {
                    if let Some(ch) = e
                        .resolve_char_ref()
                        .map_err(|err| MathParseError::Xml(err.to_string()))?
                    {
                        text.push(ch);
                    } else {
                        let name = e
                            .decode()
                            .map_err(|err| MathParseError::Xml(err.to_string()))?;
                        match name.as_ref() {
                            "amp" => text.push('&'),
                            "lt" => text.push('<'),
                            "gt" => text.push('>'),
                            "quot" => text.push('"'),
                            "apos" => text.push('\''),
                            other => {
                                text.push('&');
                                text.push_str(other);
                                text.push(';');
                            }
                        }
                    }
                }
            }
            Ok(Event::CData(e)) => {
                if let Some((_, text)) = stack.last_mut() {
                    text.push_str(std::str::from_utf8(e.as_ref()).unwrap_or_default());
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(MathParseError::Xml(err.to_string()));
            }
            _ => {}
        }
    }

    if !stack.is_empty() {
        return Err(MathParseError::UnexpectedClose);
    }

    Ok(roots)
}

pub fn parse_math_paragraph(raw_xml: String) -> Result<MathParagraph, MathParseError> {
    let elements = parse_math_elements(&raw_xml)?;
    Ok(MathParagraph {
        properties: None,
        raw_omml: raw_xml,
        elements,
    })
}

fn build_element(start: &BytesStart<'_>) -> Result<MathElement, MathParseError> {
    let name = start.name();
    let local = extract_local_name(name.as_ref());

    let mut attributes = BTreeMap::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|err| MathParseError::Xml(err.to_string()))?;
        let key = extract_attribute_name(attr.key.as_ref());
        let value = attr
            .unescape_value()
            .map_err(|err| MathParseError::Xml(err.to_string()))?;
        attributes.insert(key, value.into_owned());
    }

    let qualified = std::str::from_utf8(name.as_ref()).unwrap_or_default();
    let namespace = match qualified.split_once(':') {
        Some((prefix, _)) if prefix != "m" => Some(prefix.to_string()),
        _ => None,
    };
    Ok(MathElement {
        // A Word element inside math (`w:rPr`, `w:ins`) keeps its own name
        // and is never read as the math element of the same local name.
        kind: if namespace.is_none() {
            map_kind(local)
        } else {
            MathElementKind::Unknown
        },
        tag: Some(local.to_string()),
        attributes,
        namespace,
        ..MathElement::default()
    })
}

fn extract_local_name(name: &[u8]) -> &str {
    let name = std::str::from_utf8(name).unwrap_or_default();
    name.rsplit_once(':')
        .map(|(_, right)| right)
        .unwrap_or(name)
}

fn extract_attribute_name(name: &[u8]) -> String {
    let name = std::str::from_utf8(name).unwrap_or_default();
    name.rsplit_once(':')
        .map(|(_, right)| right.to_string())
        .unwrap_or_else(|| name.to_string())
}

fn map_kind(local: &str) -> MathElementKind {
    MathElementKind::from_omml_tag(local)
}
