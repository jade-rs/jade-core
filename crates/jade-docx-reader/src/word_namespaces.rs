//! Resolve Office Open XML names before the semantic parser and XML editor
//! consume them. Keep declarations and values intact for unknown extensions.

use std::{borrow::Cow, collections::HashSet, io, ops::Range};

use quick_xml::{
    escape::unescape,
    events::Event,
    name::{NamespaceResolver, QName, ResolveResult},
    reader::NsReader,
};

const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

const CANONICAL_NAMESPACES: &[(&str, &str)] = &[
    ("w", WORD),
    (
        "wp",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    ),
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    (
        "pic",
        "http://schemas.openxmlformats.org/drawingml/2006/picture",
    ),
    (
        "r",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    ),
];

fn canonical_prefix(uri: &str, include_drawings: bool) -> Option<&'static str> {
    if uri == WORD {
        return Some("w");
    }
    if !include_drawings {
        return None;
    }
    CANONICAL_NAMESPACES[1..]
        .iter()
        .find_map(|&(prefix, known_uri)| (uri == known_uri).then_some(prefix))
}

type NameChange = (Range<usize>, String);

fn canonical_name(
    xml: &str,
    resolver: &NamespaceResolver,
    name: QName<'_>,
    local: &[u8],
    uri: Option<&str>,
    declarations: &mut HashSet<&'static str>,
    include_drawings: bool,
) -> Result<Option<NameChange>, Error> {
    let Some(prefix) = uri.and_then(|uri| canonical_prefix(uri, include_drawings)) else {
        return Ok(None);
    };
    if name
        .prefix()
        .is_some_and(|p| p.as_ref() == prefix.as_bytes())
    {
        return Ok(None);
    }

    let probe = format!("{prefix}:probe");
    let (binding, _) = resolver.resolve_element(QName(probe.as_bytes()));
    if matches!(binding, ResolveResult::Unknown(_)) {
        declarations.insert(prefix);
    } else if namespace(binding)?.as_deref() != uri {
        return Err(invalid(format!(
            "The reserved prefix {prefix} is bound to another namespace"
        )));
    }

    Ok(Some((
        source_range(xml, name)?,
        format!("{prefix}:{}", std::str::from_utf8(local)?),
    )))
}

type Error = Box<dyn std::error::Error>;

fn invalid(message: impl Into<String>) -> Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into()).into()
}

fn namespace(result: ResolveResult<'_>) -> Result<Option<Cow<'_, str>>, Error> {
    match result {
        ResolveResult::Bound(uri) => Ok(Some(unescape(std::str::from_utf8(uri.0)?)?)),
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Unknown(prefix) => Err(invalid(format!(
            "Unbound XML namespace prefix: {}",
            String::from_utf8_lossy(&prefix)
        ))),
    }
}

fn source_range(xml: &str, name: QName<'_>) -> Result<Range<usize>, Error> {
    let start = (name.0.as_ptr() as usize)
        .checked_sub(xml.as_ptr() as usize)
        .ok_or_else(|| invalid("XML name is outside its source"))?;
    let end = start + name.0.len();
    if xml.get(start..end).map(str::as_bytes) != Some(name.0) {
        return Err(invalid("XML name is outside its source"));
    }
    Ok(start..end)
}

/// Canonicalize recognized namespaces for semantic parsing.
/// Non-Word XML parts and ordinary canonical documents remain borrowed.
///
/// Original declarations and QName-valued attributes remain intact. A binding
/// of `w` to a foreign namespace is rejected rather than changing their meaning.
pub fn normalize(xml: &str) -> Result<Cow<'_, str>, Error> {
    // Image references have a blip element, whose local name survives
    // aliasing. Shape-only parts can keep the Word namespace fast path.
    normalize_namespaces(xml, memchr::memmem::find(xml.as_bytes(), b"blip").is_some())
}

/// Preserve DrawingML spellings in the editable tree.
pub(crate) fn normalize_word_only(xml: &str) -> Result<Cow<'_, str>, Error> {
    normalize_namespaces(xml, false)
}

fn normalize_namespaces(xml: &str, include_drawings: bool) -> Result<Cow<'_, str>, Error> {
    let mut reader = NsReader::from_str(xml);
    let mut changes = Vec::new();
    let mut declarations = HashSet::new();
    let mut root_name_end = None;
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            // Unrelated package parts are not validated by this adapter.
            Err(_) if root_name_end.is_none() => return Ok(Cow::Borrowed(xml)),
            Err(error) => return Err(error.into()),
        };
        let name = match &event {
            Event::Start(element) | Event::Empty(element) => element.name(),
            Event::End(element) => element.name(),
            Event::Eof => break,
            _ => continue,
        };
        let resolver = reader.resolver();
        let (resolved, local) = resolver.resolve_element(name);
        if root_name_end.is_none() && matches!(resolved, ResolveResult::Unknown(_)) {
            return Ok(Cow::Borrowed(xml));
        }
        let resolved = namespace(resolved)?;
        let is_root = root_name_end.is_none();
        if is_root {
            if resolved.as_deref() != Some(WORD) {
                return Ok(Cow::Borrowed(xml));
            }
            root_name_end = Some(source_range(xml, name)?.end);
        }
        changes.extend(canonical_name(
            xml,
            resolver,
            name,
            local.as_ref(),
            resolved.as_deref(),
            &mut declarations,
            include_drawings,
        )?);
        let (Event::Start(element) | Event::Empty(element)) = &event else {
            continue;
        };
        let mut expanded = HashSet::new();
        let mut aliased_namespace = false;
        let mut canonical_binding = false;
        for attribute in element.attributes() {
            let attribute = attribute?;
            if attribute.key.0 == b"xmlns" || attribute.key.0.starts_with(b"xmlns:") {
                let uri = attribute.normalized_value(quick_xml::XmlVersion::Implicit1_0)?;
                if attribute.key.0 == b"xmlns:w" {
                    // Keeping the original declarations also preserves QName
                    // values such as mc:Ignorable. Do not repurpose a foreign w.
                    if uri != WORD {
                        return Err(invalid(
                            "The reserved Word prefix w is bound to another namespace",
                        ));
                    }
                    canonical_binding = true;
                }
                if let Some(prefix) = canonical_prefix(&uri, include_drawings) {
                    aliased_namespace |=
                        attribute.key.0.strip_prefix(b"xmlns:") != Some(prefix.as_bytes());
                }
                continue;
            }
            let (resolved, local) = resolver.resolve_attribute(attribute.key);
            let resolved = namespace(resolved)?;
            if !expanded.insert((
                resolved.as_deref().map(str::to_owned),
                local.as_ref().to_vec(),
            )) {
                return Err(invalid("Duplicate expanded XML attribute name"));
            }
            changes.extend(canonical_name(
                xml,
                resolver,
                attribute.key,
                local.as_ref(),
                resolved.as_deref(),
                &mut declarations,
                include_drawings,
            )?);
        }
        if is_root {
            // Most Word files declare namespaces once, on the root. Avoid a
            // second tokenization of their potentially very large body.
            let after_root = reader.buffer_position() as usize;
            if canonical_binding
                && !aliased_namespace
                && memchr::memmem::find(&xml.as_bytes()[after_root..], b"xmlns").is_none()
            {
                return Ok(Cow::Borrowed(xml));
            }
            if !canonical_binding {
                declarations.insert("w");
            }
        }
    }
    if let Some(end) = root_name_end {
        for &(prefix, uri) in CANONICAL_NAMESPACES {
            if declarations.contains(prefix) {
                changes.push((end..end, format!(" xmlns:{prefix}=\"{uri}\"")));
            }
        }
    }
    if changes.is_empty() {
        return Ok(Cow::Borrowed(xml));
    }
    // Attribute edits may have been queued before the root declaration.
    changes.sort_by_key(|(range, _)| range.start);
    let mut normalized = String::with_capacity(xml.len());
    let mut cursor = 0;
    for (range, replacement) in changes {
        normalized.push_str(&xml[cursor..range.start]);
        normalized.push_str(&replacement);
        cursor = range.end;
    }
    normalized.push_str(&xml[cursor..]);
    Ok(Cow::Owned(normalized))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_document_borrows_original_bytes() {
        let xml = format!(
            r#"<?xml version='1.0'?><w:document xmlns:w='{WORD}' xmlns:x='urn:extension'><w:body><w:p x:keep='yes'/></w:body></w:document>"#
        );
        assert!(matches!(normalize(&xml).unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn only_word_names_change_and_original_namespace_aliases_survive() {
        let xml = format!(
            r#"<?xml version='1.0'?><n:document xmlns:n='{WORD}' xmlns:x='urn:extension' x:type='n:custom'><n:body><!-- n:p --><n:p x:keep='n:literal' n:rsidR='0011'><n:r><n:t><![CDATA[n:literal]]></n:t></n:r><x:opaque n:keep='yes' plain='unchanged'/></n:p></n:body></n:document>"#
        );
        let expected = xml
            .replace("<n:", "<w:")
            .replace("</n:", "</w:")
            .replace(" n:rsidR=", " w:rsidR=")
            .replace(" n:keep=", " w:keep=")
            .replace("<w:document", &format!("<w:document xmlns:w=\"{WORD}\""));
        assert_eq!(normalize(&xml).unwrap(), expected);
    }

    #[test]
    fn default_namespace_does_not_apply_to_unprefixed_attributes() {
        let xml = format!(
            r#"<document xmlns='{WORD}' xmlns:n='{WORD}'><body><p><pPr><jc val='left' n:val='right'/></pPr></p></body></document>"#
        );
        let normalized = normalize(&xml).unwrap();
        assert!(normalized.contains("<w:jc val='left' w:val='right'/>"));
    }

    #[test]
    fn foreign_canonical_prefix_is_rejected_instead_of_changing_qname_values() {
        for xml in [
            format!(
                r#"<n:document xmlns:n='{WORD}' xmlns:w='urn:extension' type='w:custom'><n:body/></n:document>"#
            ),
            format!(
                r#"<w:document xmlns:w='{WORD}'><w:body><x:opaque xmlns:x='urn:x' xmlns:w='urn:extension' type='w:custom'/></w:body></w:document>"#
            ),
        ] {
            assert!(normalize(&xml)
                .unwrap_err()
                .to_string()
                .contains("bound to another namespace"));
        }
    }

    #[test]
    fn unrelated_xml_is_byte_identical_even_with_word_descendants() {
        let xml = format!(r#"<custom xmlns:n='{WORD}'><n:p n:val='1'/></custom>"#);
        assert!(matches!(normalize(&xml).unwrap(), Cow::Borrowed(_)));
        assert!(matches!(normalize("contents").unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn duplicate_expanded_attributes_are_rejected() {
        let xml = format!(
            r#"<n:document xmlns:n='{WORD}' xmlns:other='{WORD}'><n:body><n:p n:rsidR='one' other:rsidR='two'/></n:body></n:document>"#
        );
        assert!(normalize(&xml)
            .unwrap_err()
            .to_string()
            .contains("Duplicate expanded"));
    }

    #[test]
    fn entity_encoded_namespace_uri_is_resolved() {
        let xml = format!(
            r#"<n:document xmlns:n='{}'><n:body/></n:document>"#,
            WORD.replace("/main", "/m&#97;in")
        );
        assert!(normalize(&xml).unwrap().contains("<w:body/>"));
    }
}
