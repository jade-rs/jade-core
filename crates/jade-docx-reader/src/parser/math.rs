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
    let mut stack: Vec<MathElement> = Vec::new();
    let mut roots: Vec<MathElement> = Vec::new();

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let element = build_element(e)?;
                stack.push(element);
            }
            Ok(Event::End(_)) => {
                let Some(mut element) = stack.pop() else {
                    return Err(MathParseError::UnexpectedClose);
                };
                if element.children.is_empty() {
                    element.children.shrink_to_fit();
                }
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(element);
                } else {
                    roots.push(element);
                }
            }
            Ok(Event::Empty(ref e)) => {
                let element = build_element(e)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(element);
                } else {
                    roots.push(element);
                }
            }
            Ok(Event::Text(e)) => {
                if let Some(current) = stack.last_mut() {
                    let decoded = e
                        .decode()
                        .map_err(|err| MathParseError::Xml(err.to_string()))?;
                    let text = unescape(decoded.as_ref())
                        .map_err(|err| MathParseError::Xml(err.to_string()))?;
                    if !text.is_empty() {
                        let owned = text.into_owned();
                        if matches!(current.kind, MathElementKind::Text) || !owned.trim().is_empty()
                        {
                            current.text = Some(owned);
                        }
                    }
                }
            }
            Ok(Event::CData(e)) => {
                if let Some(current) = stack.last_mut() {
                    let text = std::str::from_utf8(e.as_ref()).unwrap_or_default();
                    if !text.is_empty() {
                        current.text = Some(text.to_string());
                    }
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

    Ok(MathElement {
        kind: map_kind(local),
        tag: Some(local.to_string()),
        attributes,
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
    match local {
        "oMath" => MathElementKind::Math,
        "oMathPara" => MathElementKind::MathParagraph,
        "r" => MathElementKind::Run,
        "t" => MathElementKind::Text,
        "f" => MathElementKind::Fraction,
        "num" => MathElementKind::Numerator,
        "den" => MathElementKind::Denominator,
        "rad" => MathElementKind::Radical,
        "deg" => MathElementKind::Degree,
        "e" => MathElementKind::Base,
        "sSup" => MathElementKind::Superscript,
        "sSub" => MathElementKind::Subscript,
        "sPre" => MathElementKind::PreSubSuper,
        "sup" => MathElementKind::SuperArgument,
        "sub" => MathElementKind::SubArgument,
        "nary" => MathElementKind::Nary,
        "d" => MathElementKind::Delimiter,
        "box" => MathElementKind::Box,
        "bar" => MathElementKind::Bar,
        "groupChr" => MathElementKind::GroupChar,
        "eqArr" => MathElementKind::EquationArray,
        "lim" => MathElementKind::Limit,
        "limLow" => MathElementKind::LimitLower,
        "m" => MathElementKind::Matrix,
        "mr" => MathElementKind::MatrixRow,
        "func" => MathElementKind::Function,
        "fName" => MathElementKind::FunctionName,
        _ => MathElementKind::Unknown,
    }
}
