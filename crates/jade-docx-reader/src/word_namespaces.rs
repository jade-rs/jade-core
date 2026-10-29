//! Resolve WordprocessingML names before the semantic parser and XML editor
//! consume them. Keep declarations and values intact for unknown extensions.

use std::{borrow::Cow, collections::HashSet, io, ops::Range};

use quick_xml::{
    escape::unescape,
    events::Event,
    name::{QName, ResolveResult},
    reader::NsReader,
};

const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

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

/// Canonicalize only names bound to the Word namespace. The parser still uses
/// `w:` internally; the raw editor must receive exactly the same names.
/// Non-Word XML parts and ordinary canonical documents remain borrowed.
///
/// Original declarations and QName-valued attributes remain intact. A binding
/// of `w` to a foreign namespace is rejected rather than changing their meaning.
pub fn normalize(xml: &str) -> Result<Cow<'_, str>, Error> {
    let mut reader = NsReader::from_str(xml);
    let mut changes: Vec<(Range<usize>, String)> = Vec::new();
    let mut root_seen = false;
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            // Unrelated package parts are not validated by this adapter.
            Err(_) if !root_seen => return Ok(Cow::Borrowed(xml)),
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
        if !root_seen && matches!(resolved, ResolveResult::Unknown(_)) {
            return Ok(Cow::Borrowed(xml));
        }
        let resolved = namespace(resolved)?;
        let is_root = !root_seen;
        if is_root {
            if resolved.as_deref() != Some(WORD) {
                return Ok(Cow::Borrowed(xml));
            }
            root_seen = true;
        }
        if resolved.as_deref() == Some(WORD) && name.prefix().is_none_or(|p| p.as_ref() != b"w") {
            changes.push((
                source_range(xml, name)?,
                format!("w:{}", std::str::from_utf8(local.as_ref())?),
            ));
        }
        let (Event::Start(element) | Event::Empty(element)) = &event else {
            continue;
        };
        let mut expanded = HashSet::new();
        let mut aliased_word = false;
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
                } else if uri == WORD {
                    aliased_word = true;
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
            if resolved.as_deref() == Some(WORD)
                && attribute.key.prefix().is_none_or(|p| p.as_ref() != b"w")
            {
                changes.push((
                    source_range(xml, attribute.key)?,
                    format!("w:{}", std::str::from_utf8(local.as_ref())?),
                ));
            }
        }
        if is_root {
            // Most Word files declare namespaces once, on the root. Avoid a
            // second tokenization of their potentially very large body.
            let after_root = reader.buffer_position() as usize;
            if canonical_binding && !aliased_word && !xml[after_root..].contains("xmlns") {
                return Ok(Cow::Borrowed(xml));
            }
            if !canonical_binding {
                let end = source_range(xml, name)?.end;
                changes.push((end..end, format!(" xmlns:w=\"{WORD}\"")));
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
