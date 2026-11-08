//! Namespace-aware edits of a direct child. Detached XML fragments get their
//! inherited namespace and MCE processing context temporarily. Aliases and
//! opaque payloads keep their identities; temporary declarations are restored.
use crate::{
    patch::{apply, local, namespace_prefix, Splice},
    xml_edit, Result, WriteError,
};
use jade_xlsx_reader::process_content::Scope;
use quick_xml::{
    events::Event,
    name::{PrefixDeclaration, ResolveResult},
    NsReader,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct ContextXml {
    pub text: String,
    introduced: Vec<(String, Option<String>)>,
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
            introduced: missing.into_iter().map(|(k, _)| (k, None)).collect(),
        })
    }
    pub(crate) fn scoped(text: &str, bindings: &[(String, String)], scope: &Scope) -> Result<Self> {
        let mut context = Self::new(text, bindings)?;
        let content: BTreeSet<_> = scope.process_content_names().collect();
        if content.is_empty() {
            return Ok(context);
        }
        let mut ignored: BTreeSet<_> = scope.ignorable_namespaces().collect();
        ignored.extend(content.iter().map(|(ns, _)| *ns));
        let mut original = BTreeMap::new();
        {
            let mut reader = NsReader::from_str(&context.text);
            if let Event::Start(e) | Event::Empty(e) = reader.read_event()? {
                for a in e.attributes() {
                    let a = a?;
                    let (ns, local) = reader.resolver().resolve_attribute(a.key);
                    if matches!(ns, ResolveResult::Bound(n) if n.as_ref() == MC)
                        && matches!(local.as_ref(), b"Ignorable" | b"ProcessContent")
                    {
                        original.insert(
                            String::from_utf8_lossy(local.as_ref()).into_owned(),
                            (
                                String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                                a.unescape_value()?.into_owned(),
                            ),
                        );
                    }
                }
            }
        }
        let mut next = 0;
        let mut updates: Vec<(String, Option<String>)> = Vec::new();
        let mc = temporary_prefix(&context.text, &mut next);
        updates.push((
            format!("xmlns:{mc}"),
            Some(String::from_utf8_lossy(MC).into_owned()),
        ));
        let mut prefixes = BTreeMap::new();
        for ns in &ignored {
            let prefix = temporary_prefix(&context.text, &mut next);
            updates.push((format!("xmlns:{prefix}"), Some((*ns).to_owned())));
            prefixes.insert(*ns, prefix);
        }
        context
            .introduced
            .extend(updates.iter().map(|(key, _)| (key.clone(), None)));
        let ignore = ignored
            .iter()
            .map(|ns| prefixes[ns].as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let process = content
            .iter()
            .map(|(ns, name)| format!("{}:{name}", prefixes[ns]))
            .collect::<Vec<_>>()
            .join(" ");
        for (local, value) in [("Ignorable", ignore), ("ProcessContent", process)] {
            let (key, before) = match original.remove(local) {
                Some((key, value)) => (key, Some(value)),
                None => (format!("{mc}:{local}"), None),
            };
            context.introduced.push((key.clone(), before));
            updates.push((key, Some(value)));
        }
        let updates = updates
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect::<Vec<_>>();
        context.text = xml_edit::root_attributes(&context.text, &updates)?;
        Ok(context)
    }
    pub fn finish(&self, updated: &str) -> Result<String> {
        let updates: Vec<_> = self
            .introduced
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        xml_edit::root_attributes(updated, &updates)
    }
}
const MC: &[u8] = b"http://schemas.openxmlformats.org/markup-compatibility/2006";
fn temporary_prefix(text: &str, next: &mut usize) -> String {
    loop {
        let prefix = format!("_jade_ctx{}", *next);
        *next += 1;
        if !text.contains(&prefix) {
            return prefix;
        }
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

/// Direct foreign children nominated by the active expanded MCE context.
pub(crate) fn processed_children(xml: &str) -> Result<Vec<(usize, usize, ContextXml)>> {
    if !xml.contains("ProcessContent") {
        return Ok(Vec::new());
    }
    select(xml, b"", None, true)
}

fn select(
    xml: &str,
    name: &[u8],
    ns: Option<&[u8]>,
    processing: bool,
) -> Result<Vec<(usize, usize, ContextXml)>> {
    let mut reader = NsReader::from_str(xml);
    let mut scope = Scope::for_xml(xml);
    let processed = if processing {
        Some(jade_xlsx_reader::process_content::processed_elements(xml)?)
    } else {
        None
    };
    let mut depth = 0usize;
    let mut root_namespace = None;
    let mut result = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                scope.push(&reader, e)?;
                if depth == 0 {
                    root_namespace = namespace(&reader, e.name());
                }
                let expected = ns.map(|n| n.to_vec()).or_else(|| root_namespace.clone());
                let selected = if let Some(positions) = &processed {
                    positions.processes_content_at(start)
                } else {
                    expected.is_some()
                        && namespace(&reader, e.name()) == expected
                        && local(e.name().as_ref()) == name
                };
                if depth == 1 && selected {
                    let env = bindings(&reader);
                    if !empty {
                        let n = e.name().as_ref().to_vec();
                        reader.read_to_end(quick_xml::name::QName(&n))?;
                    }
                    let end = reader.buffer_position() as usize;
                    result.push((
                        start,
                        end,
                        ContextXml::scoped(&xml[start..end], &env, &scope)?,
                    ));
                    scope.pop();
                } else if !empty {
                    depth += 1;
                } else {
                    scope.pop();
                }
            }
            Event::End(_) => {
                scope.pop();
                depth = depth.saturating_sub(1);
            }
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
        new_child(xml, name)?
    };
    let updated = edit(&contextual.text)?;
    let updated = contextual.finish(&updated)?;
    set_child(xml, name, |_| updated, following)
}

/// Detach a known original element span with its in-scope namespace bindings.
pub(crate) fn fragment(xml: &str, span: std::ops::Range<usize>) -> Result<ContextXml> {
    let mut reader = NsReader::from_str(xml);
    let mut scope = Scope::for_xml(xml);
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                scope.push(&reader, e)?;
                if start == span.start {
                    return ContextXml::scoped(&xml[span], &bindings(&reader), &scope);
                }
                if matches!(event, Event::Empty(_)) {
                    scope.pop();
                }
            }
            Event::End(_) => scope.pop(),
            Event::Eof => return Err(WriteError::NoRoot("XML fragment")),
            _ => {}
        }
    }
}

/// A new direct child with the parent's in-scope namespace environment.
pub(crate) fn new_child(xml: &str, name: &[u8]) -> Result<ContextXml> {
    let mut reader = NsReader::from_str(xml);
    let mut scope = Scope::for_xml(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                scope.push(&reader, &e)?;
                let prefix = namespace_prefix(e.name().as_ref());
                return ContextXml::scoped(
                    &format!("<{prefix}{}/>", String::from_utf8_lossy(name)),
                    &bindings(&reader),
                    &scope,
                );
            }
            Event::Eof => return Err(WriteError::NoRoot("XML")),
            _ => {}
        }
    }
}

pub(crate) fn plain_empty(xml: &str, allowed: &[&[u8]]) -> Result<bool> {
    let mut r = NsReader::from_str(xml);
    let mut depth = 0;
    loop {
        match r.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                if depth != 0 {
                    return Ok(false);
                }
                for a in e.attributes() {
                    let a = a?;
                    let (ns, local) = r.resolver().resolve_attribute(a.key);
                    let processing = matches!(ns, ResolveResult::Bound(n) if n.as_ref() == MC)
                        && matches!(local.as_ref(), b"Ignorable" | b"ProcessContent");
                    if !processing
                        && a.key.as_ref() != b"xmlns"
                        && !a.key.as_ref().starts_with(b"xmlns:")
                        && !allowed.contains(&a.key.as_ref())
                    {
                        return Ok(false);
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth -= 1,
            Event::Text(t) if t.decode()?.trim().is_empty() => {}
            Event::Eof => return Ok(true),
            _ => return Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detached_context_resolves_inherited_names_and_restores_declarations() {
        let xml = r#"<root xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:one" xmlns:v="urn:one" mc:Ignorable="u" mc:ProcessContent="u:pass"><item xmlns:u="urn:two" mc:Ignorable="u" note="_jade_ctx0"><v:pass/><u:pass/><v:opaque/></item><empty/></root>"#;
        let start = xml.find("<item").unwrap();
        let end = xml.find("</item>").unwrap() + "</item>".len();
        let selected = children(xml, b"item").unwrap().remove(0).2;
        for context in [fragment(xml, start..end).unwrap(), selected] {
            let processed =
                jade_xlsx_reader::process_content::processed_elements(&context.text).unwrap();
            assert!(processed.processes_content_at(context.text.find("<v:pass").unwrap()));
            assert!(!processed.processes_content_at(context.text.find("<u:pass").unwrap()));
            assert!(!processed.processes_content_at(context.text.find("<v:opaque").unwrap()));
            assert_eq!(context.finish(&context.text).unwrap(), &xml[start..end]);
            let added = new_child(&context.text, b"leaf").unwrap();
            assert!(plain_empty(&added.text, &[]).unwrap());
            assert_eq!(added.finish(&added.text).unwrap(), "<leaf/>");
        }
        let start = xml.find("<empty/>").unwrap();
        let empty = fragment(xml, start..start + "<empty/>".len()).unwrap();
        assert!(!empty.text.contains("urn:two"));
        assert!(plain_empty(&empty.text, &[]).unwrap());
        let opaque =
            xml_edit::root_attributes(&empty.text, &[("u:state", Some("keep".into()))]).unwrap();
        assert!(!plain_empty(&opaque, &[]).unwrap());
        let lookalike =
            xml_edit::root_attributes(&empty.text, &[("u:Ignorable", Some("keep".into()))])
                .unwrap();
        assert!(!plain_empty(&lookalike, &[]).unwrap());
        assert_eq!(empty.finish(&empty.text).unwrap(), "<empty/>");
    }
}
