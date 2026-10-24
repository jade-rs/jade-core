//! Namespace-aware edits of a direct child. Detached XML fragments get their
//! inherited bindings temporarily, so aliases and foreign extensions keep
//! their identities while being edited. Only introduced bindings are removed.
use crate::{
    patch::{apply, local, namespace_prefix, Splice},
    xml_edit, Result, WriteError,
};
use quick_xml::{
    events::Event,
    name::{PrefixDeclaration, ResolveResult},
    NsReader,
};

pub(crate) struct ContextXml {
    pub text: String,
    introduced: Vec<String>,
}
impl ContextXml {
    pub(crate) fn new(text: &str, bindings: &[(String, String)]) -> Result<Self> {
        let mut reader = quick_xml::Reader::from_str(text);
        let mut declared = Vec::new();
        loop {
            match reader.read_event()? {
                Event::Start(e) | Event::Empty(e) => {
                    for a in e.attributes() {
                        declared.push(String::from_utf8_lossy(a?.key.as_ref()).into_owned());
                    }
                    break;
                }
                Event::Eof => return Err(WriteError::NoRoot("XML")),
                _ => {}
            }
        }
        let missing: Vec<_> = bindings
            .iter()
            .filter(|(k, _)| !declared.contains(k))
            .cloned()
            .collect();
        let updates: Vec<_> = missing
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v.clone())))
            .collect();
        Ok(Self {
            text: xml_edit::root_attributes(text, &updates)?,
            introduced: missing.into_iter().map(|(k, _)| k).collect(),
        })
    }
    pub fn finish(&self, updated: &str) -> Result<String> {
        let updates: Vec<_> = self.introduced.iter().map(|k| (k.as_str(), None)).collect();
        xml_edit::root_attributes(updated, &updates)
    }
}
pub(crate) fn bindings(reader: &NsReader<&[u8]>) -> Vec<(String, String)> {
    reader
        .resolver()
        .bindings()
        .map(|(prefix, namespace)| {
            let key = match prefix {
                PrefixDeclaration::Default => "xmlns".into(),
                PrefixDeclaration::Named(p) => format!("xmlns:{}", String::from_utf8_lossy(p)),
            };
            (
                key,
                String::from_utf8_lossy(namespace.as_ref()).into_owned(),
            )
        })
        .collect()
}
fn namespace(reader: &NsReader<&[u8]>, name: quick_xml::name::QName<'_>) -> Option<Vec<u8>> {
    match reader.resolver().resolve_element(name).0 {
        ResolveResult::Unbound => Some(Vec::new()),
        ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
        _ => None,
    }
}

pub(crate) fn children(xml: &str, name: &[u8]) -> Result<Vec<(usize, usize, ContextXml)>> {
    select(xml, name, None, false)
}

/// Select direct children by expanded name, including a different namespace.
pub(crate) fn children_ns(
    xml: &str,
    name: &[u8],
    ns: &[u8],
) -> Result<Vec<(usize, usize, ContextXml)>> {
    select(xml, name, Some(ns), false)
}

/// Non-overlapping descendants with an explicitly identified namespace.
pub(crate) fn descendants_ns(
    xml: &str,
    name: &[u8],
    ns: &[u8],
) -> Result<Vec<(usize, usize, ContextXml)>> {
    select(xml, name, Some(ns), true)
}

fn select(
    xml: &str,
    name: &[u8],
    ns: Option<&[u8]>,
    descendants: bool,
) -> Result<Vec<(usize, usize, ContextXml)>> {
    let mut reader = NsReader::from_str(xml);
    let mut depth = 0usize;
    let mut root_namespace = None;
    let mut result = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                if depth == 0 {
                    root_namespace = namespace(&reader, e.name());
                }
                let expected = ns.map(|n| n.to_vec()).or_else(|| root_namespace.clone());
                if (depth == 1 || (descendants && depth > 0))
                    && expected.is_some()
                    && namespace(&reader, e.name()) == expected
                    && local(e.name().as_ref()) == name
                {
                    let env = bindings(&reader);
                    if !empty {
                        let n = e.name().as_ref().to_vec();
                        reader.read_to_end(quick_xml::name::QName(&n))?;
                    }
                    let end = reader.buffer_position() as usize;
                    result.push((start, end, ContextXml::new(&xml[start..end], &env)?));
                } else if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(result)
}

pub(crate) fn set_child(
    xml: &str,
    name: &[u8],
    text: impl FnOnce(&str) -> String,
    following: &[&[u8]],
) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    let mut depth = 0usize;
    let mut root_namespace = None;
    let mut prefix = String::new();
    let mut existing = None;
    let mut insertion = None;
    let mut root_end = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                if depth == 0 {
                    root_namespace = namespace(&reader, e.name());
                    prefix = namespace_prefix(e.name().as_ref());
                    if empty {
                        let new = text(&prefix);
                        if new.is_empty() {
                            return Ok(xml.into());
                        }
                        return xml_edit::append_child(xml, &new);
                    }
                }
                if depth == 1
                    && root_namespace.is_some()
                    && namespace(&reader, e.name()) == root_namespace
                {
                    let n = e.name().as_ref().to_vec();
                    if local(&n) == name {
                        if !empty {
                            reader.read_to_end(quick_xml::name::QName(&n))?;
                        }
                        if existing.is_none() {
                            existing = Some((start, reader.buffer_position() as usize));
                        }
                        continue;
                    }
                    if insertion.is_none() && following.contains(&local(&n)) {
                        insertion = Some(start);
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    root_end = Some(start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let text = text(&prefix);
    let (start, end) = match existing {
        Some(span) => span,
        None => {
            if text.is_empty() {
                return Ok(xml.into());
            }
            let at = insertion.or(root_end).ok_or(WriteError::NoRoot("XML"))?;
            (at, at)
        }
    };
    Ok(apply(xml, vec![Splice { start, end, text }]))
}

pub(crate) fn edit_child(
    xml: &str,
    name: &[u8],
    following: &[&[u8]],
    edit: impl FnOnce(&str) -> Result<String>,
) -> Result<String> {
    let mut candidates = children(xml, name)?;
    let contextual = if !candidates.is_empty() {
        candidates.remove(0).2
    } else {
        let mut reader = NsReader::from_str(xml);
        loop {
            match reader.read_event()? {
                Event::Start(e) | Event::Empty(e) => {
                    let prefix = namespace_prefix(e.name().as_ref());
                    let base = format!("<{prefix}{}/>", String::from_utf8_lossy(name));
                    break ContextXml::new(&base, &bindings(&reader))?;
                }
                Event::Eof => return Err(WriteError::NoRoot("XML")),
                _ => {}
            }
        }
    };
    let updated = edit(&contextual.text)?;
    let updated = contextual.finish(&updated)?;
    set_child(xml, name, |_| updated, following)
}

/// Detach a known original element span with its in-scope namespace bindings.
pub(crate) fn fragment(xml: &str, span: std::ops::Range<usize>) -> Result<ContextXml> {
    let mut reader = NsReader::from_str(xml);
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(_) | Event::Empty(_) if start == span.start => {
                return ContextXml::new(&xml[span], &bindings(&reader));
            }
            Event::Eof => return Err(WriteError::NoRoot("XML fragment")),
            _ => {}
        }
    }
}

/// A new direct child with the parent's in-scope namespace environment.
pub(crate) fn new_child(xml: &str, name: &[u8]) -> Result<ContextXml> {
    let mut reader = NsReader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let prefix = namespace_prefix(e.name().as_ref());
                return ContextXml::new(
                    &format!("<{prefix}{}/>", String::from_utf8_lossy(name)),
                    &bindings(&reader),
                );
            }
            Event::Eof => return Err(WriteError::NoRoot("XML")),
            _ => {}
        }
    }
}
