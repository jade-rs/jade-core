//! Small, preserving edits of direct children of SpreadsheetML containers.
use crate::patch::{apply, local, Splice};
use crate::{shift::rewrite, Result};
use quick_xml::{events::Event, name::QName, Reader};

pub(crate) fn root_attributes(xml: &str, updates: &[(&str, Option<String>)]) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                return Ok(apply(
                    xml,
                    vec![Splice {
                        start,
                        end: reader.buffer_position() as usize,
                        text: exact_head(e, updates, matches!(event, Event::Empty(_)))?,
                    }],
                ))
            }
            Event::Eof => return Err(crate::WriteError::NoRoot("XML")),
            _ => {}
        }
    }
}

fn exact_head(
    e: &quick_xml::events::BytesStart<'_>,
    updates: &[(&str, Option<String>)],
    empty: bool,
) -> Result<String> {
    let mut head = format!("<{}", String::from_utf8_lossy(e.name().as_ref()));
    let mut seen = Vec::new();
    for attr in e.attributes() {
        let attr = attr?;
        let key = String::from_utf8_lossy(attr.key.as_ref());
        let value = match updates
            .iter()
            .find(|(name, _)| name.as_bytes() == attr.key.as_ref())
        {
            Some((name, value)) => {
                seen.push(*name);
                value.clone()
            }
            None => Some(attr.unescape_value()?.into_owned()),
        };
        if let Some(value) = value {
            head.push_str(&format!(" {key}=\"{}\"", crate::patch::escape(&value)));
        }
    }
    for (key, value) in updates {
        if !seen.contains(key) {
            if let Some(value) = value {
                head.push_str(&format!(" {key}=\"{}\"", crate::patch::escape(value)));
            }
        }
    }
    head.push_str(if empty { "/>" } else { ">" });
    Ok(head)
}

pub(crate) fn append_child(xml: &str, child: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(_) => depth += 1,
            Event::Empty(ref e) if depth == 0 => {
                let end = reader.buffer_position() as usize;
                let head = xml[start..end].trim_end_matches("/>");
                return Ok(apply(
                    xml,
                    vec![Splice {
                        start,
                        end,
                        text: format!(
                            "{head}>{child}</{}>",
                            String::from_utf8_lossy(e.name().as_ref())
                        ),
                    }],
                ));
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(apply(
                        xml,
                        vec![Splice {
                            start,
                            end: start,
                            text: child.into(),
                        }],
                    ));
                }
            }
            Event::Eof => return Err(crate::WriteError::NoRoot("XML")),
            _ => {}
        }
    }
}

pub(crate) fn child_xml(xml: &str, name: &[u8]) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                if depth == 1 && local(e.name().as_ref()) == name {
                    if !empty {
                        let qualified = e.name().as_ref().to_vec();
                        reader.read_to_end_into(QName(&qualified), &mut Vec::new())?;
                    }
                    return Ok(Some(
                        xml[start..reader.buffer_position() as usize].to_string(),
                    ));
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => return Ok(None),
            _ => {}
        }
        buf.clear();
    }
}

/// Edits a container while retaining its namespace and all sibling content.
pub(crate) fn edit_child(
    xml: &str,
    name: &[u8],
    following: &[&[u8]],
    edit: impl FnOnce(&str) -> Result<String>,
) -> Result<String> {
    let existing = child_xml(xml, name)?;
    // Discover the root prefix for a newly created child.
    let mut reader = Reader::from_str(xml);
    let prefix = loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                break crate::patch::namespace_prefix(e.name().as_ref())
            }
            Event::Eof => return Err(crate::WriteError::NoRoot("XML")),
            _ => {}
        }
    };
    let base = existing.unwrap_or_else(|| format!("<{prefix}{}/>", String::from_utf8_lossy(name)));
    let updated = edit(&base)?;
    crate::header_footer::set_child(xml, name, |_| updated, following)
}

pub(crate) fn child_attributes(
    xml: &str,
    name: &[u8],
    updates: &[(&str, Option<String>)],
    following: &[&[u8]],
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                if depth == 1 && local(element.name().as_ref()) == name {
                    return Ok(apply(
                        xml,
                        vec![Splice {
                            start,
                            end: reader.buffer_position() as usize,
                            text: rewrite(element, updates, empty)?,
                        }],
                    ));
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    crate::header_footer::set_child(
        xml,
        name,
        |prefix| {
            let tag = format!("{prefix}{}", String::from_utf8_lossy(name));
            let attrs: String = updates
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .as_ref()
                        .map(|value| format!(" {name}=\"{}\"", crate::patch::escape(value)))
                })
                .collect();
            format!("<{tag}{attrs}/>")
        },
        following,
    )
}
