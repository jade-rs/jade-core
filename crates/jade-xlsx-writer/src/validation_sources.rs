//! Preserve validation source alternatives while editing the fields this
//! implementation understands. Unknown branches and sibling metadata survive.
use crate::{
    patch::{apply, escape, Splice},
    xml_edit, xml_edit_ns, Result,
};
use jade_xlsx_reader::{
    data_validations::{LIST_NAMESPACE, MC_NAMESPACE, XM_NAMESPACE},
    DataValidation,
};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
pub(crate) struct Sources {
    pub xml: String,
    pub first: bool,
    pub second: bool,
}
fn namespace(xml: &str) -> Result<Vec<u8>> {
    let mut r = NsReader::from_str(xml);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                return Ok(match r.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Bound(n) => n.as_ref().to_vec(),
                    _ => Vec::new(),
                })
            }
            Event::Eof => return Err(crate::WriteError::NoRoot("validation source")),
            _ => {}
        }
    }
}
fn nodes(xml: &str, ns: &[u8]) -> Result<Vec<(usize, usize, xml_edit_ns::ContextXml, u8)>> {
    let mut result = Vec::new();
    for (name, space, slot) in [
        (b"formula1".as_slice(), ns, 1),
        (b"formula2".as_slice(), ns, 2),
        (b"list".as_slice(), LIST_NAMESPACE, 1),
    ] {
        result.extend(
            xml_edit_ns::children_ns(xml, name, space)?
                .into_iter()
                .map(|(a, b, c)| (a, b, c, slot)),
        );
    }
    result.sort_by_key(|(a, _, _, _)| *a);
    Ok(result)
}
fn branches(xml: &str) -> Result<Vec<(usize, usize, xml_edit_ns::ContextXml, bool)>> {
    let mut result = Vec::new();
    for (name, fallback) in [
        (b"Choice".as_slice(), false),
        (b"Fallback".as_slice(), true),
    ] {
        result.extend(
            xml_edit_ns::children_ns(xml, name, MC_NAMESPACE)?
                .into_iter()
                .map(|(a, b, c)| (a, b, c, fallback)),
        );
    }
    result.sort_by_key(|(a, _, _, _)| *a);
    Ok(result)
}
fn formula(
    existing: Option<&str>,
    ns: &[u8],
    name: &str,
    value: Option<&str>,
    extended: bool,
) -> Result<String> {
    let Some(value) = value else {
        return Ok(String::new());
    };
    let value = value.strip_prefix('=').unwrap_or(value);
    if let Some(existing) = existing {
        if extended {
            return set_formula_value(existing, value);
        }
        return crate::cell_shift::text_content(existing, value);
    }
    let text = if extended {
        format!(
            "<f xmlns=\"{}\">{}</f>",
            String::from_utf8_lossy(XM_NAMESPACE),
            escape(value)
        )
    } else {
        escape(value)
    };
    Ok(format!(
        "<{name} xmlns=\"{}\">{text}</{name}>",
        escape(&String::from_utf8_lossy(ns))
    ))
}
fn literal(existing: Option<&str>, text: &str) -> Result<String> {
    let encoded = jade_xlsx_reader::xstring::encode_xstring(text)
        .replace('\n', "_x000A_")
        .replace('\t', "_x0009_");
    if let Some(existing) = existing {
        return crate::cell_shift::text_content(existing, &encoded);
    }
    Ok(format!(
        "<jadeList:list xmlns:jadeList=\"{}\" xml:space=\"preserve\">{}</jadeList:list>",
        String::from_utf8_lossy(LIST_NAMESPACE),
        escape(&encoded)
    ))
}
fn fallback(rule: &DataValidation) -> Option<String> {
    rule.literal_list
        .as_deref()
        .and_then(jade_xlsx_reader::data_validations::parse_literal_list)
        .map(|items| {
            let flat = jade_xlsx_reader::xstring::encode_xstring(&items.join(","));
            format!("\"{}\"", flat.replace('"', "\"\""))
        })
        .or_else(|| rule.formula1.clone())
}
fn replace_fields(
    xml: &str,
    ns: &[u8],
    rule: &DataValidation,
    extended: bool,
    rich: bool,
    first: bool,
    second: bool,
) -> Result<String> {
    let fields = nodes(xml, ns)?;
    let old_first = fields
        .iter()
        .find(|(_, _, c, slot)| *slot == 1 && namespace(&c.text).ok().as_deref() == Some(ns));
    let old_second = fields.iter().find(|(_, _, _, slot)| *slot == 2);
    let old_list = fields
        .iter()
        .find(|(_, _, c, _)| namespace(&c.text).ok().as_deref() == Some(LIST_NAMESPACE));
    let first_xml = if !first {
        String::new()
    } else if rich && rule.literal_list.is_some() {
        literal(
            old_list.map(|(_, _, c, _)| c.text.as_str()),
            rule.literal_list.as_deref().unwrap(),
        )?
    } else {
        formula(
            old_first.map(|(_, _, c, _)| c.text.as_str()),
            ns,
            "formula1",
            fallback(rule).as_deref(),
            extended,
        )?
    };
    let second_xml = if second {
        formula(
            old_second.map(|(_, _, c, _)| c.text.as_str()),
            ns,
            "formula2",
            rule.formula2.as_deref(),
            extended,
        )?
    } else {
        String::new()
    };
    let mut edits = Vec::new();
    let mut inserted_first = false;
    let mut inserted_second = false;
    for (start, end, _, slot) in fields {
        if (slot == 1 && !first) || (slot == 2 && !second) {
            continue;
        }
        let text = if slot == 1 && !inserted_first {
            inserted_first = true;
            first_xml.clone()
        } else if slot == 2 && !inserted_second {
            inserted_second = true;
            second_xml.clone()
        } else {
            String::new()
        };
        edits.push(Splice { start, end, text });
    }
    let mut result = apply(xml, edits);
    if first && !inserted_first && !first_xml.is_empty() {
        result = xml_edit::append_child(&result, &first_xml)?;
    }
    if second && !inserted_second && !second_xml.is_empty() {
        result = xml_edit::append_child(&result, &second_xml)?;
    }
    Ok(result)
}
fn canonical(
    rule: &DataValidation,
    ns: &[u8],
    extended: bool,
    old_list: Option<&str>,
) -> Result<String> {
    let rich = literal(old_list, rule.literal_list.as_deref().unwrap())?;
    let legacy = formula(None, ns, "formula1", fallback(rule).as_deref(), extended)?;
    Ok(format!("<jadeMce:AlternateContent xmlns:jadeMce=\"{}\" xmlns:jadeList=\"{}\"><jadeMce:Choice Requires=\"jadeList\">{rich}</jadeMce:Choice><jadeMce:Fallback>{legacy}</jadeMce:Fallback></jadeMce:AlternateContent>",String::from_utf8_lossy(MC_NAMESPACE),String::from_utf8_lossy(LIST_NAMESPACE)))
}
pub(crate) fn rewrite(xml: &str, rule: &DataValidation, extended: bool) -> Result<Sources> {
    let ns = namespace(xml)?;
    let rich = rule.literal_list.is_some();
    // Coverage-only edits preserve malformed imported literals verbatim; input
    // evaluation reports them as unsupported, rather than guessing candidates.
    if rule
        .literal_list
        .as_deref()
        .is_some_and(|s| jade_xlsx_reader::data_validations::parse_literal_list(s).is_none())
    {
        return Ok(Sources {
            xml: xml.into(),
            first: false,
            second: false,
        });
    }
    let mut edits = Vec::new();
    let mut first = false;
    let mut second = false;
    for (start, end, wrapper) in xml_edit_ns::children_ns(xml, b"AlternateContent", MC_NAMESPACE)? {
        let list = branches(&wrapper.text)?;
        let mut has_first = false;
        let mut has_second = false;
        for (_, _, branch, _) in &list {
            for (_, _, _, slot) in nodes(&branch.text, &ns)? {
                has_first |= slot == 1;
                has_second |= slot == 2;
            }
        }
        if !has_first && !has_second {
            continue;
        }
        let mut changes = Vec::new();
        let mut choice = false;
        let mut has_fallback = false;
        for (a, b, branch, is_fallback) in list {
            if !is_fallback
                && !jade_xlsx_reader::data_validations::validation_choice_supported(&branch.text)?
            {
                continue;
            }
            choice |= !is_fallback;
            has_fallback |= is_fallback;
            let moved = replace_fields(
                &branch.text,
                &ns,
                rule,
                extended,
                rich && !is_fallback,
                has_first,
                has_second,
            )?;
            if moved != branch.text {
                changes.push(Splice {
                    start: a,
                    end: b,
                    text: branch.finish(&moved)?,
                });
            }
        }
        let mut moved = apply(&wrapper.text, changes);
        if rich && has_first && !choice {
            let body = literal(None, rule.literal_list.as_deref().unwrap())?;
            let addition=format!("<jadeMce:Choice xmlns:jadeMce=\"{}\" xmlns:jadeList=\"{}\" Requires=\"jadeList\">{body}</jadeMce:Choice>",String::from_utf8_lossy(MC_NAMESPACE),String::from_utf8_lossy(LIST_NAMESPACE));
            // A new supported choice precedes the preserved unknown branches.
            if let Some((at, _, _, _)) = branches(&moved)?.first() {
                moved = apply(
                    &moved,
                    vec![Splice {
                        start: *at,
                        end: *at,
                        text: addition,
                    }],
                );
            } else {
                moved = xml_edit::append_child(&moved, &addition)?;
            }
        }
        if !has_fallback {
            let body = replace_fields(
                &format!(
                    "<jadeMce:Fallback xmlns:jadeMce=\"{}\"/>",
                    String::from_utf8_lossy(MC_NAMESPACE)
                ),
                &ns,
                rule,
                extended,
                false,
                has_first,
                has_second,
            )?;
            moved = xml_edit::append_child(&moved, &body)?;
        }
        first |= has_first;
        second |= has_second;
        if moved != wrapper.text {
            edits.push(Splice {
                start,
                end,
                text: wrapper.finish(&moved)?,
            });
        }
    }
    let direct_lists = xml_edit_ns::children_ns(xml, b"list", LIST_NAMESPACE)?;
    if !first && (rich || !direct_lists.is_empty()) {
        let old = direct_lists.first().map(|(_, _, c)| c.text.as_str());
        let addition = if rich {
            canonical(rule, &ns, extended, old)?
        } else {
            formula(None, &ns, "formula1", rule.formula1.as_deref(), extended)?
        };
        if let Some((start, _, _)) = direct_lists.first() {
            edits.push(Splice {
                start: *start,
                end: *start,
                text: addition,
            });
        } else {
            // Put source information before x14 coverage and extension metadata.
            let at = xml_edit_ns::children_ns(xml, b"sqref", XM_NAMESPACE)?
                .first()
                .map(|(a, _, _)| *a)
                .or(xml_edit_ns::children(xml, b"extLst")?
                    .first()
                    .map(|(a, _, _)| *a));
            if let Some(at) = at {
                edits.push(Splice {
                    start: at,
                    end: at,
                    text: addition,
                });
            } else {
                let moved = xml_edit::append_child(xml, &addition)?;
                return rewrite(&moved, rule, extended);
            }
        }
        first = true;
    }
    if first {
        for (start, end, _) in direct_lists {
            edits.push(Splice {
                start,
                end,
                text: String::new(),
            });
        }
        for (start, end, _) in xml_edit_ns::children_ns(xml, b"formula1", &ns)? {
            edits.push(Splice {
                start,
                end,
                text: String::new(),
            });
        }
    }
    if second {
        for (start, end, _) in xml_edit_ns::children_ns(xml, b"formula2", &ns)? {
            edits.push(Splice {
                start,
                end,
                text: String::new(),
            });
        }
    }
    Ok(Sources {
        xml: apply(xml, edits),
        first,
        second,
    })
}

/// Rewrite an x14 formula wrapper, including MC alternatives around its xm:f
/// leaf, without appending a second effective formula beside an old choice.
pub(crate) fn set_formula_value(wrapper: &str, value: &str) -> Result<String> {
    let (moved, found) = formula_level(wrapper, value)?;
    if found {
        Ok(moved)
    } else {
        xml_edit::append_child(&moved, &fresh_formula_leaf(value))
    }
}
fn fresh_formula_leaf(value: &str) -> String {
    format!(
        "<f xmlns=\"{}\">{}</f>",
        String::from_utf8_lossy(XM_NAMESPACE),
        escape(value.strip_prefix('=').unwrap_or(value))
    )
}
fn formula_level(xml: &str, value: &str) -> Result<(String, bool)> {
    let mut edits = Vec::new();
    let mut found = false;
    for (start, end, leaf) in xml_edit_ns::children_ns(xml, b"f", XM_NAMESPACE)? {
        found = true;
        let moved =
            crate::cell_shift::text_content(&leaf.text, value.strip_prefix('=').unwrap_or(value))?;
        if moved != leaf.text {
            edits.push(Splice {
                start,
                end,
                text: leaf.finish(&moved)?,
            });
        }
    }
    for (start, end, wrapper) in xml_edit_ns::children_ns(xml, b"AlternateContent", MC_NAMESPACE)? {
        let mut changes = Vec::new();
        let mut fallback = false;
        for (a, b, branch, is_fallback) in branches(&wrapper.text)? {
            if !is_fallback
                && !jade_xlsx_reader::data_validations::validation_choice_supported(&branch.text)?
            {
                continue;
            }
            fallback |= is_fallback;
            let (mut moved, handled) = formula_level(&branch.text, value)?;
            if !handled {
                moved = xml_edit::append_child(&moved, &fresh_formula_leaf(value))?;
            }
            if moved != branch.text {
                changes.push(Splice {
                    start: a,
                    end: b,
                    text: branch.finish(&moved)?,
                });
            }
        }
        let mut moved = apply(&wrapper.text, changes);
        if !fallback {
            moved = xml_edit::append_child(
                &moved,
                &format!(
                    "<jadeMce:Fallback xmlns:jadeMce=\"{}\">{}</jadeMce:Fallback>",
                    String::from_utf8_lossy(MC_NAMESPACE),
                    fresh_formula_leaf(value)
                ),
            )?;
        }
        found = true;
        if moved != wrapper.text {
            edits.push(Splice {
                start,
                end,
                text: wrapper.finish(&moved)?,
            });
        }
    }
    Ok((apply(xml, edits), found))
}

/// Rebase known formula sources in every alternative during a structural
/// edit. Literal list text and foreign wrappers are never formula operands.
pub(crate) fn map_formulas(
    xml: &str,
    extended: bool,
    map: &impl Fn(&str) -> String,
) -> Result<String> {
    map_source_level(
        xml,
        &namespace(xml)?,
        &[b"formula1", b"formula2"],
        extended,
        map,
    )
}

/// Extended coverage may use the same transparent alternatives as sources.
pub(crate) fn map_extended_ranges(xml: &str, map: &impl Fn(&str) -> String) -> Result<String> {
    map_source_level(xml, XM_NAMESPACE, &[b"sqref"], false, map)
}

pub(crate) fn map_source_level(
    xml: &str,
    ns: &[u8],
    names: &[&[u8]],
    wrapped: bool,
    map: &impl Fn(&str) -> String,
) -> Result<String> {
    let mut edits = Vec::new();
    for name in names {
        for (start, end, source) in xml_edit_ns::children_ns(xml, name, ns)? {
            let moved = if wrapped {
                map_source_level(&source.text, XM_NAMESPACE, &[b"f"], false, map)?
            } else {
                map_text_leaf(&source.text, map)?
            };
            if moved != source.text {
                edits.push(Splice {
                    start,
                    end,
                    text: source.finish(&moved)?,
                });
            }
        }
    }
    for (start, end, wrapper) in xml_edit_ns::children_ns(xml, b"AlternateContent", MC_NAMESPACE)? {
        let mut changes = Vec::new();
        for (a, b, branch, is_fallback) in branches(&wrapper.text)? {
            if !is_fallback
                && !jade_xlsx_reader::data_validations::validation_choice_supported(&branch.text)?
            {
                continue;
            }
            let moved = map_source_level(&branch.text, ns, names, wrapped, map)?;
            if moved != branch.text {
                changes.push(Splice {
                    start: a,
                    end: b,
                    text: branch.finish(&moved)?,
                });
            }
        }
        if !changes.is_empty() {
            edits.push(Splice {
                start,
                end,
                text: wrapper.finish(&apply(&wrapper.text, changes))?,
            });
        }
    }
    Ok(apply(xml, edits))
}

// Comments and processing instructions are not formula characters. Keep them
// when a text leaf changes; a leaf containing element children is opaque.
fn map_text_leaf(xml: &str, map: &impl Fn(&str) -> String) -> Result<String> {
    use quick_xml::{events::Event, Reader};
    let mut reader = Reader::from_str(xml);
    let mut root = true;
    let mut text = String::new();
    let mut spans = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(_) if root => root = false,
            Event::Empty(_) if root => break,
            Event::Start(_) | Event::Empty(_) => return Ok(xml.into()),
            Event::Text(e) => {
                text.push_str(&e.decode()?);
                spans.push((start, reader.buffer_position() as usize));
            }
            Event::CData(e) => {
                text.push_str(&e.decode()?);
                spans.push((start, reader.buffer_position() as usize));
            }
            Event::GeneralRef(e) => {
                text.push_str(
                    &quick_xml::escape::unescape(&format!("&{};", e.decode()?))
                        .map_err(quick_xml::Error::from)?,
                );
                spans.push((start, reader.buffer_position() as usize));
            }
            Event::End(_) | Event::Eof => break,
            _ => {}
        }
    }
    let next = map(&text);
    if next == text {
        return Ok(xml.into());
    }
    if spans.is_empty() {
        return xml_edit::append_child(xml, &escape(&next));
    }
    Ok(apply(
        xml,
        spans
            .into_iter()
            .enumerate()
            .map(|(i, (start, end))| Splice {
                start,
                end,
                text: if i == 0 { escape(&next) } else { String::new() },
            })
            .collect(),
    ))
}
