//! Standard and x14 input rules, with namespace-aware record locations so
//! preserving writers address exactly the rules returned to the caller.
use crate::{
    error::Result,
    model::{CellRange, DataValidation},
    xstring::decode_xstring,
};
use jade_opc::local_name;
use quick_xml::{
    events::{BytesStart, Event},
    name::ResolveResult,
    NsReader,
};
use std::{collections::BTreeSet, ops::Range};
pub const X14_NAMESPACE: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
pub const XM_NAMESPACE: &[u8] = b"http://schemas.microsoft.com/office/excel/2006/main";

pub const LIST_NAMESPACE: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2011/1/ac";
pub const MC_NAMESPACE: &[u8] = b"http://schemas.openxmlformats.org/markup-compatibility/2006";

/// One interpreted rule and its position in the original worksheet XML.
/// `index` includes unmodeled siblings; `container` is an original byte span.
#[derive(Debug, Clone)]
pub struct ValidationRecord {
    pub rule: DataValidation,
    pub container: Range<usize>,
    pub index: usize,
    pub extended: bool,
}
struct Container {
    start: usize,
    depth: usize,
    extended: bool,
    next: usize,
    records: Vec<(usize, DataValidation)>,
}
struct Rule {
    value: DataValidation,
    depth: usize,
    index: usize,
}
struct Text {
    slot: u8,
    depth: usize,
    value: String,
}

pub fn parse_data_validations(xml: &str) -> Result<Vec<DataValidation>> {
    Ok(validation_records(xml)?
        .into_iter()
        .map(|r| r.rule)
        .collect())
}

/// Standard rules first, then recognized x14 extension containers in document
/// order. Foreign/nested lookalikes and records without coverage are excluded.
pub fn validation_records(xml: &str) -> Result<Vec<ValidationRecord>> {
    let mut reader = NsReader::from_str(xml);
    let mut stack: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut main: Option<Vec<u8>> = None;
    let mut container: Option<Container> = None;
    let mut rule: Option<Rule> = None;
    let mut text: Option<Text> = None;
    let mut records = Vec::new();
    let mut alternates: Vec<(usize, bool)> = Vec::new();
    let mut transparent = BTreeSet::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let ns = match reader.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => Vec::new(),
                    ResolveResult::Bound(n) => n.as_ref().to_vec(),
                    _ => b"?unknown".to_vec(),
                };
                let name = local_name(e.name().as_ref()).to_vec();
                let depth = stack.len();
                if rule.is_some() && ns == MC_NAMESPACE {
                    if name == b"AlternateContent" && !empty {
                        alternates.push((depth, false));
                        transparent.insert(depth);
                    } else if matches!(name.as_slice(), b"Choice" | b"Fallback")
                        && stack.last().is_some_and(|(ns, name)| {
                            ns == MC_NAMESPACE && name == b"AlternateContent"
                        })
                        && alternates.last().is_some_and(|(at, _)| at + 1 == depth)
                    {
                        let supported = name == b"Fallback" || choice_supported(&reader, e)?;
                        let state = alternates.last_mut().unwrap();
                        if state.1 || !supported {
                            if !empty {
                                reader.read_to_end(e.name())?;
                            }
                            continue;
                        }
                        state.1 = true;
                        if !empty {
                            transparent.insert(depth);
                        }
                    }
                }

                if depth == 0
                    && name == b"worksheet"
                    && matches!(
                        ns.as_slice(),
                        b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                            | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                    )
                {
                    main = Some(ns.clone());
                }
                let own = main.as_ref() == Some(&ns);
                let extended = ns == X14_NAMESPACE
                    && depth == 3
                    && main.as_ref().is_some_and(|m| {
                        stack[1].0 == *m
                            && stack[1].1 == b"extLst"
                            && stack[2].0 == *m
                            && stack[2].1 == b"ext"
                    });
                if !empty && name == b"dataValidations" && ((own && depth == 1) || extended) {
                    container = Some(Container {
                        start,
                        depth,
                        extended,
                        next: 0,
                        records: Vec::new(),
                    });
                } else if let Some(c) = container.as_mut() {
                    let valid_ns = if c.extended { ns == X14_NAMESPACE } else { own };
                    if name == b"dataValidation" && valid_ns && depth == c.depth + 1 {
                        let mut value = parse_validation(e, c.extended)?;
                        if c.extended {
                            value.ranges.clear();
                            value.suppress_dropdown = !flag(e, b"showDropDown")?;
                        }
                        let index = c.next;
                        c.next += 1;
                        if empty {
                            if !value.ranges.is_empty() {
                                c.records.push((index, value));
                            }
                        } else {
                            rule = Some(Rule {
                                value,
                                depth,
                                index,
                            });
                        }
                    } else if let Some(r) = rule.as_ref() {
                        let path = stack
                            .iter()
                            .enumerate()
                            .skip(r.depth + 1)
                            .filter(|(i, _)| !transparent.contains(i))
                            .map(|(_, p)| p)
                            .collect::<Vec<_>>();
                        let slot = if path.is_empty() && ns == LIST_NAMESPACE && name == b"list" {
                            Some(4)
                        } else if path.is_empty() && valid_ns && !c.extended {
                            match name.as_slice() {
                                b"formula1" => Some(1),
                                b"formula2" => Some(2),
                                _ => None,
                            }
                        } else if c.extended
                            && ns == XM_NAMESPACE
                            && name == b"sqref"
                            && path.is_empty()
                        {
                            Some(3)
                        } else if c.extended
                            && ns == XM_NAMESPACE
                            && name == b"f"
                            && path.len() == 1
                            && path[0].0 == X14_NAMESPACE
                        {
                            match path[0].1.as_slice() {
                                b"formula1" => Some(1),
                                b"formula2" => Some(2),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        if let Some(slot) = slot {
                            if empty {
                                assign(&mut rule.as_mut().unwrap().value, slot, String::new())?;
                            } else {
                                text = Some(Text {
                                    slot,
                                    depth: depth + 1,
                                    value: String::new(),
                                });
                            }
                        }
                    }
                }
                if !empty {
                    stack.push((ns, name));
                }
            }
            Event::Text(e) => {
                if let Some(t) = text.as_mut().filter(|t| t.depth == stack.len()) {
                    t.value.push_str(&e.decode()?);
                }
            }
            Event::CData(e) => {
                if let Some(t) = text.as_mut().filter(|t| t.depth == stack.len()) {
                    t.value.push_str(&e.decode()?);
                }
            }
            Event::GeneralRef(e) => {
                if let Some(t) = text.as_mut().filter(|t| t.depth == stack.len()) {
                    if let Some(s) = decoded_reference(&e) {
                        t.value.push_str(&s);
                    }
                }
            }
            Event::End(_) => {
                if text.as_ref().is_some_and(|t| t.depth == stack.len()) {
                    let t = text.take().unwrap();
                    if let Some(r) = rule.as_mut() {
                        assign(&mut r.value, t.slot, t.value)?;
                    }
                }
                if rule.as_ref().is_some_and(|r| r.depth + 1 == stack.len()) {
                    let r = rule.take().unwrap();
                    if !r.value.ranges.is_empty() {
                        container.as_mut().unwrap().records.push((r.index, r.value));
                    }
                }
                if container
                    .as_ref()
                    .is_some_and(|c| c.depth + 1 == stack.len())
                {
                    let c = container.take().unwrap();
                    let end = reader.buffer_position() as usize;
                    records.extend(c.records.into_iter().map(|(index, rule)| ValidationRecord {
                        rule,
                        container: c.start..end,
                        index,
                        extended: c.extended,
                    }));
                }
                let closed = stack.len().saturating_sub(1);
                transparent.remove(&closed);
                if alternates.last().is_some_and(|(at, _)| *at == closed) {
                    alternates.pop();
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    records.sort_by_key(|r| r.extended);
    Ok(records)
}
fn assign(rule: &mut DataValidation, slot: u8, value: String) -> Result<()> {
    match slot {
        1 => rule.formula1 = Some(value),
        2 => rule.formula2 = Some(value),
        4 => {
            rule.literal_list = Some(decode_xstring(&value));
            rule.formula1 = None;
        }
        _ => {
            rule.ranges = value
                .split_ascii_whitespace()
                .map(CellRange::parse)
                .collect::<Result<Vec<_>>>()?;
        }
    }
    Ok(())
}
fn parse_validation(e: &BytesStart<'_>, extended: bool) -> Result<DataValidation> {
    let ranges = (if extended {
        None
    } else {
        attribute(e, b"sqref")?
    })
    .unwrap_or_default()
    .split_ascii_whitespace()
    .map(CellRange::parse)
    .collect::<Result<Vec<_>>>()?;
    let text =
        |key| -> Result<Option<String>> { Ok(attribute(e, key)?.map(|s| decode_xstring(&s))) };
    Ok(DataValidation {
        ranges,
        validation_type: attribute(e, b"type")?,
        operator: attribute(e, b"operator")?,
        allow_blank: flag(e, b"allowBlank")?,
        show_input_message: flag(e, b"showInputMessage")?,
        show_error_message: flag(e, b"showErrorMessage")?,
        error_style: attribute(e, b"errorStyle")?,
        suppress_dropdown: flag(e, b"showDropDown")?,
        ime_mode: attribute(e, b"imeMode")?,
        prompt_title: text(b"promptTitle")?,
        prompt: text(b"prompt")?,
        error_title: text(b"errorTitle")?,
        error: text(b"error")?,
        formula1: None,
        formula2: None,
        literal_list: None,
    })
}
fn flag(e: &BytesStart<'_>, key: &[u8]) -> Result<bool> {
    Ok(attribute(e, key)?.is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")))
}
fn attribute(e: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attr in e.attributes() {
        let attr = attr?;
        if attr.key.as_ref() == key {
            return Ok(Some(attr.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
fn decoded_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .ok()
        .map(|s| s.into_owned())
}

/// Whether this validation parser understands every namespace in Requires.
/// The first supported Choice wins; unknown requirements select Fallback.
pub fn validation_choice_supported(xml: &str) -> Result<bool> {
    let mut reader = NsReader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => return choice_supported(&reader, &e),
            Event::Eof => return Ok(false),
            _ => {}
        }
    }
}
pub(crate) fn choice_supported(reader: &NsReader<&[u8]>, e: &BytesStart<'_>) -> Result<bool> {
    let Some(required) = attribute(e, b"Requires")? else {
        return Ok(false);
    };
    let mut any = false;
    for prefix in required.split_ascii_whitespace() {
        any = true;
        let name = format!("{prefix}:x");
        let (ns, _) = reader
            .resolver()
            .resolve_element(quick_xml::name::QName(name.as_bytes()));
        if !matches!(ns,ResolveResult::Bound(n) if matches!(n.as_ref(),
            LIST_NAMESPACE|X14_NAMESPACE|XM_NAMESPACE|
            b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"|
            b"http://purl.oclc.org/ooxml/spreadsheetml/main"))
        {
            return Ok(false);
        }
    }
    Ok(any)
}
/// One RFC 4180-style CSV record, retaining empty fields and spaces. Quoting
/// protects embedded commas, double quotes and line breaks. A malformed list
/// returns None rather than silently changing the permitted entries.
pub fn parse_literal_list(text: &str) -> Option<Vec<String>> {
    let mut chars = text.chars().peekable();
    let mut result = Vec::new();
    loop {
        let mut field = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            loop {
                match chars.next()? {
                    '"' if chars.peek() == Some(&'"') => {
                        chars.next();
                        field.push('"');
                    }
                    '"' => break,
                    c => field.push(c),
                }
            }
            if chars.peek().is_some_and(|c| *c != ',') {
                return None;
            }
        } else {
            while let Some(c) = chars.peek().copied() {
                if c == ',' {
                    break;
                }
                if matches!(c, '"' | '\r' | '\n') {
                    return None;
                }
                field.push(c);
                chars.next();
            }
        }
        result.push(field);
        if chars.next().is_none() {
            break;
        }
    }
    Some(result)
}
pub fn format_literal_list(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if s.contains([',', '"', '\r', '\n']) || (i == 0 && s.starts_with('=')) {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod literal_tests {
    use super::{format_literal_list, parse_literal_list};
    #[test]
    fn quoted_values_round_trip_without_interpreting_them_as_formulas() {
        let items = [
            "a,b",
            "a\"b",
            "line\r\nbreak",
            "",
            " trailing ",
            "日本語",
            "=A1",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(
            parse_literal_list(&format_literal_list(&items)),
            Some(items)
        );
        assert_eq!(
            parse_literal_list("\"a\"\"b\",,c,"),
            Some(vec!["a\"b".into(), "".into(), "c".into(), "".into()])
        );
        assert_eq!(
            format_literal_list(&["=A1".into(), "B2".into()]),
            "\"=A1\",B2"
        );
    }
    #[test]
    fn malformed_fields_do_not_become_different_allowed_values() {
        for text in [
            "\"open",
            "\"closed\"tail",
            "unquoted\"quote",
            "two\nrecords",
        ] {
            assert_eq!(parse_literal_list(text), None, "{text}");
        }
    }
}
