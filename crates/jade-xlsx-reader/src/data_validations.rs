//! Standard worksheet data validations.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::error::Result;
use crate::model::{CellRange, DataValidation};
use jade_opc::local_name;

pub fn parse_data_validations(xml: &str) -> Result<Vec<DataValidation>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut validations = Vec::new();
    let mut validation: Option<DataValidation> = None;
    let mut formula_slot = None;
    let mut formula = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element)
                if local_name(element.name().as_ref()) == b"dataValidation" =>
            {
                validation = parse_validation(element)?;
            }
            Event::Empty(ref element)
                if local_name(element.name().as_ref()) == b"dataValidation" =>
            {
                if let Some(validation) = parse_validation(element)? {
                    validations.push(validation);
                }
            }
            Event::Start(ref element) => match local_name(element.name().as_ref()) {
                b"formula1" if validation.is_some() => {
                    formula_slot = Some(1);
                    formula.clear();
                }
                b"formula2" if validation.is_some() => {
                    formula_slot = Some(2);
                    formula.clear();
                }
                _ => {}
            },
            Event::Text(ref text) if formula_slot.is_some() => {
                formula.push_str(&text.decode()?);
            }
            Event::GeneralRef(ref reference) if formula_slot.is_some() => {
                if let Some(decoded) = decoded_reference(reference) {
                    formula.push_str(&decoded);
                }
            }
            Event::End(ref element) => match local_name(element.name().as_ref()) {
                b"formula1" => {
                    if let Some(validation) = validation.as_mut() {
                        validation.formula1 = Some(formula.clone());
                    }
                    formula_slot = None;
                }
                b"formula2" => {
                    if let Some(validation) = validation.as_mut() {
                        validation.formula2 = Some(formula.clone());
                    }
                    formula_slot = None;
                }
                b"dataValidation" => {
                    if let Some(validation) = validation.take() {
                        validations.push(validation);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(validations)
}

fn parse_validation(element: &BytesStart<'_>) -> Result<Option<DataValidation>> {
    let Some(sqref) = attribute_value(element, b"sqref")? else {
        return Ok(None);
    };
    let ranges = sqref
        .split_ascii_whitespace()
        .map(CellRange::parse)
        .collect::<Result<Vec<_>>>()?;
    if ranges.is_empty() {
        return Ok(None);
    }

    Ok(Some(DataValidation {
        ranges,
        validation_type: attribute_value(element, b"type")?,
        operator: attribute_value(element, b"operator")?,
        allow_blank: bool_attribute(element, b"allowBlank")?,
        show_input_message: bool_attribute(element, b"showInputMessage")?,
        show_error_message: bool_attribute(element, b"showErrorMessage")?,
        prompt_title: attribute_value(element, b"promptTitle")?,
        prompt: attribute_value(element, b"prompt")?,
        error_title: attribute_value(element, b"errorTitle")?,
        error: attribute_value(element, b"error")?,
        formula1: None,
        formula2: None,
    }))
}

fn bool_attribute(element: &BytesStart<'_>, key: &[u8]) -> Result<bool> {
    Ok(attribute_value(element, key)?
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true")))
}

fn attribute_value(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if local_name(attribute.key.as_ref()) == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

fn decoded_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .ok()
        .map(|text| text.into_owned())
}
