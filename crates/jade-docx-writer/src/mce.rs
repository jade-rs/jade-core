//! Markup Compatibility (ISO/IEC 29500-3) prefix repair for WordprocessingML
//! parts.
//!
//! Word refuses to open a package when `mc:Ignorable` (or `mc:MustUnderstand`,
//! or a `Requires` on `mc:Choice`) names a prefix that the part does not
//! declare — removing a single `xmlns:w15` from an otherwise valid
//! `document.xml` is enough. Documents that went through a namespace-rewriting
//! tool (lxml, ElementTree) lose exactly those declarations because the
//! prefixes are referenced only from attribute *values*, which no XML
//! serializer tracks. jade keeps the raw XML of every part it does not edit and
//! re-serializes the ones it does, so such a document would come out of every
//! export unopenable. This module puts the missing declarations back on the
//! root element when the prefix is one of the namespaces Word itself declares,
//! and drops any prefix it cannot resolve from the compatibility list — an
//! undeclared prefix cannot occur in the content, so nothing is lost.

/// Namespace prefixes Word 365 declares on WordprocessingML roots, keyed by
/// the prefix Word uses. Only these are added; an unknown prefix is removed
/// from the compatibility attribute instead of being guessed.
const WELL_KNOWN_PREFIXES: &[(&str, &str)] = &[
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    (
        "aink",
        "http://schemas.microsoft.com/office/drawing/2016/ink",
    ),
    (
        "am3d",
        "http://schemas.microsoft.com/office/drawing/2017/model3d",
    ),
    (
        "cx",
        "http://schemas.microsoft.com/office/drawing/2014/chartex",
    ),
    (
        "cx1",
        "http://schemas.microsoft.com/office/drawing/2015/9/8/chartex",
    ),
    (
        "cx2",
        "http://schemas.microsoft.com/office/drawing/2015/10/21/chartex",
    ),
    (
        "cx3",
        "http://schemas.microsoft.com/office/drawing/2016/5/9/chartex",
    ),
    (
        "cx4",
        "http://schemas.microsoft.com/office/drawing/2016/5/10/chartex",
    ),
    (
        "cx5",
        "http://schemas.microsoft.com/office/drawing/2016/5/11/chartex",
    ),
    (
        "cx6",
        "http://schemas.microsoft.com/office/drawing/2016/5/12/chartex",
    ),
    (
        "cx7",
        "http://schemas.microsoft.com/office/drawing/2016/5/13/chartex",
    ),
    (
        "cx8",
        "http://schemas.microsoft.com/office/drawing/2016/5/14/chartex",
    ),
    (
        "m",
        "http://schemas.openxmlformats.org/officeDocument/2006/math",
    ),
    (
        "mc",
        "http://schemas.openxmlformats.org/markup-compatibility/2006",
    ),
    ("o", "urn:schemas-microsoft-com:office:office"),
    ("oel", "http://schemas.microsoft.com/office/2019/extlst"),
    (
        "r",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    ),
    (
        "sl",
        "http://schemas.openxmlformats.org/schemaLibrary/2006/main",
    ),
    ("v", "urn:schemas-microsoft-com:vml"),
    (
        "w",
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    ),
    ("w10", "urn:schemas-microsoft-com:office:word"),
    (
        "w14",
        "http://schemas.microsoft.com/office/word/2010/wordml",
    ),
    (
        "w15",
        "http://schemas.microsoft.com/office/word/2012/wordml",
    ),
    (
        "w16",
        "http://schemas.microsoft.com/office/word/2018/wordml",
    ),
    (
        "w16cex",
        "http://schemas.microsoft.com/office/word/2018/wordml/cex",
    ),
    (
        "w16cid",
        "http://schemas.microsoft.com/office/word/2016/wordml/cid",
    ),
    (
        "w16du",
        "http://schemas.microsoft.com/office/word/2023/wordml/word16du",
    ),
    (
        "w16sdtdh",
        "http://schemas.microsoft.com/office/word/2020/wordml/sdtdatahash",
    ),
    (
        "w16sdtfl",
        "http://schemas.microsoft.com/office/word/2024/wordml/sdtformatlock",
    ),
    (
        "w16se",
        "http://schemas.microsoft.com/office/word/2015/wordml/symex",
    ),
    (
        "wne",
        "http://schemas.microsoft.com/office/word/2006/wordml",
    ),
    (
        "wp",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    ),
    (
        "wp14",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing",
    ),
    (
        "wpc",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas",
    ),
    (
        "wpg",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
    ),
    (
        "wpi",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingInk",
    ),
    (
        "wps",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
    ),
];

fn well_known_namespace(prefix: &str) -> Option<&'static str> {
    WELL_KNOWN_PREFIXES
        .iter()
        .find(|(known, _)| *known == prefix)
        .map(|(_, uri)| *uri)
}

/// Whether a package part is one this repair applies to: the XML parts of the
/// main document story (`word/document.xml`, styles, headers, footnotes, the
/// glossary document, …). Other parts never carry `mc:Ignorable`.
pub fn is_wordprocessing_xml_part(path: &str) -> bool {
    path.starts_with("word/") && path.ends_with(".xml")
}

/// The root start tag of a part rewritten so that every prefix named by its
/// Markup Compatibility attributes is declared. `range` is the byte span of
/// the original start tag; the bytes outside it are unchanged, so a caller
/// can stream `xml[..range.start]`, `tag`, `xml[range.end..]` without
/// copying a multi-megabyte `document.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairedRootTag {
    pub range: std::ops::Range<usize>,
    pub tag: Vec<u8>,
}

/// Returns the part with every prefix named by its Markup Compatibility
/// attributes declared on the root element, or `None` when the part already
/// is consistent (or is not something this repair understands), so an
/// untouched part keeps its exact bytes.
pub fn repair_markup_compatibility_prefixes(xml: &[u8]) -> Option<Vec<u8>> {
    let repaired = repair_markup_compatibility_root_tag(xml)?;
    let mut out = Vec::with_capacity(xml.len() + repaired.tag.len());
    out.extend_from_slice(&xml[..repaired.range.start]);
    out.extend_from_slice(&repaired.tag);
    out.extend_from_slice(&xml[repaired.range.end..]);
    Some(out)
}

/// The rewritten root start tag alone; `None` when the part is consistent.
/// See [`repair_markup_compatibility_prefixes`] for the semantics.
pub fn repair_markup_compatibility_root_tag(xml: &[u8]) -> Option<RepairedRootTag> {
    // Only the tags read below are decoded: validating a multi-megabyte
    // `document.xml` as UTF-8 on every export cost more than the scan.
    let root = root_start_tag(xml)?;
    let tag = std::str::from_utf8(&xml[root.start..root.end]).ok()?;
    let attributes = parse_attributes(tag);

    let declared: Vec<&str> = attributes
        .iter()
        .filter_map(|attribute| attribute.name.strip_prefix("xmlns:"))
        .collect();

    // Prefixes referenced from compatibility attributes anywhere in the part.
    // `Requires` sits on `mc:Choice` elements deep in the body, so the whole
    // part is scanned for those.
    let mut referenced: Vec<String> = Vec::new();
    let mut push_tokens = |value: &str| {
        for token in value.split_ascii_whitespace() {
            if !referenced.iter().any(|existing| existing == token) {
                referenced.push(token.to_string());
            }
        }
    };
    for attribute in &attributes {
        if attribute.name == "mc:Ignorable" || attribute.name == "mc:MustUnderstand" {
            push_tokens(&attribute.value);
        }
    }
    // `Requires` sits on `mc:Choice` deep in the body. A part that never
    // declares `mc` cannot contain one, so skip the whole-part scan for it.
    if declared.contains(&"mc") {
        for choice in element_start_tags(xml, "mc:Choice") {
            let Ok(choice) = std::str::from_utf8(&xml[choice.start..choice.end]) else {
                continue;
            };
            for attribute in parse_attributes(choice) {
                if attribute.name == "Requires" || attribute.name == "mc:Requires" {
                    push_tokens(&attribute.value);
                }
            }
        }
    }

    let mut additions = String::new();
    let mut unresolved: Vec<String> = Vec::new();
    for prefix in referenced {
        if declared.iter().any(|known| *known == prefix) {
            continue;
        }
        match well_known_namespace(&prefix) {
            Some(uri) => {
                additions.push_str(" xmlns:");
                additions.push_str(&prefix);
                additions.push_str("=\"");
                additions.push_str(uri);
                additions.push('"');
            }
            None => unresolved.push(prefix),
        }
    }

    if additions.is_empty() && unresolved.is_empty() {
        return None;
    }

    let mut new_tag = String::with_capacity(tag.len() + additions.len());
    if unresolved.is_empty() {
        new_tag.push_str(tag);
    } else {
        // Rebuild the root tag with the unresolvable prefixes dropped from the
        // compatibility lists; every other byte of the tag is kept as is.
        let mut cursor = 0;
        for attribute in &attributes {
            if attribute.name != "mc:Ignorable" && attribute.name != "mc:MustUnderstand" {
                continue;
            }
            let kept: Vec<&str> = attribute
                .value
                .split_ascii_whitespace()
                .filter(|token| !unresolved.iter().any(|dropped| dropped == token))
                .collect();
            new_tag.push_str(&tag[cursor..attribute.value_start]);
            new_tag.push_str(&kept.join(" "));
            cursor = attribute.value_end;
        }
        new_tag.push_str(&tag[cursor..]);
    }

    let insert_at = if new_tag.ends_with("/>") {
        new_tag.len() - 2
    } else {
        new_tag.len() - 1
    };
    new_tag.insert_str(insert_at, &additions);

    Some(RepairedRootTag {
        range: root.start..root.end,
        tag: new_tag.into_bytes(),
    })
}

#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

/// Byte span of the root element's start tag, including `<` and `>`; skips
/// the XML declaration, comments and processing instructions before it.
fn root_start_tag(xml: &[u8]) -> Option<Span> {
    let mut cursor = 0;
    while let Some(offset) = memchr::memchr(b'<', &xml[cursor..]) {
        let start = cursor + offset;
        let rest = &xml[start..];
        if rest.starts_with(b"<?") {
            cursor = start + memchr::memmem::find(rest, b"?>")? + 2;
        } else if rest.starts_with(b"<!--") {
            cursor = start + memchr::memmem::find(rest, b"-->")? + 3;
        } else if rest.starts_with(b"<!") {
            cursor = start + memchr::memchr(b'>', rest)? + 1;
        } else {
            let end = start_tag_end(xml, start)?;
            return Some(Span { start, end });
        }
    }
    None
}

/// Every start tag named `name` in document order.
fn element_start_tags<'a>(xml: &'a [u8], name: &str) -> impl Iterator<Item = Span> + 'a {
    let needle = format!("<{name}").into_bytes();
    let finder = memchr::memmem::Finder::new(&needle).into_owned();
    let mut cursor = 0;
    std::iter::from_fn(move || {
        loop {
            let start = cursor + finder.find(&xml[cursor..])?;
            let after = xml.get(start + needle.len()).copied();
            let is_boundary =
                matches!(after, Some(c) if c.is_ascii_whitespace() || c == b'/' || c == b'>');
            if is_boundary {
                let end = start_tag_end(xml, start)?;
                cursor = end;
                return Some(Span { start, end });
            }
            cursor = start + needle.len();
        }
    })
}

/// End (exclusive) of the start tag beginning at `start`, honouring quoted
/// attribute values that may contain `>`.
fn start_tag_end(xml: &[u8], start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (index, byte) in xml.iter().enumerate().skip(start + 1) {
        match quote {
            Some(open) => {
                if *byte == open {
                    quote = None;
                }
            }
            None => match byte {
                b'"' | b'\'' => quote = Some(*byte),
                b'>' => return Some(index + 1),
                _ => {}
            },
        }
    }
    None
}

#[derive(Debug)]
struct RawAttribute {
    name: String,
    value: String,
    /// Byte offsets of the value (excluding quotes), relative to the tag.
    value_start: usize,
    value_end: usize,
}

fn parse_attributes(tag: &str) -> Vec<RawAttribute> {
    let bytes = tag.as_bytes();
    let mut attributes = Vec::new();
    // Skip `<name`.
    let mut cursor = bytes
        .iter()
        .position(|byte| byte.is_ascii_whitespace() || *byte == b'>' || *byte == b'/')
        .unwrap_or(bytes.len());
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] == b'>' || bytes[cursor] == b'/' {
            break;
        }
        let name_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != b'=' && !bytes[cursor].is_ascii_whitespace()
        {
            cursor += 1;
        }
        let name = tag[name_start..cursor].to_string();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            break;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            break;
        }
        let quote = bytes[cursor];
        if quote != b'"' && quote != b'\'' {
            break;
        }
        let value_start = cursor + 1;
        let Some(length) = bytes[value_start..].iter().position(|byte| *byte == quote) else {
            break;
        };
        let value_end = value_start + length;
        attributes.push(RawAttribute {
            name,
            value: tag[value_start..value_end].to_string(),
            value_start,
            value_end,
        });
        cursor = value_end + 1;
    }
    attributes
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";

    fn repair(xml: &str) -> Option<String> {
        repair_markup_compatibility_prefixes(xml.as_bytes())
            .map(|bytes| String::from_utf8(bytes).expect("utf8"))
    }

    #[test]
    fn consistent_part_is_left_untouched() {
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:document xmlns:w=\"{W}\" xmlns:mc=\"{MC}\" xmlns:w14=\"{W14}\" mc:Ignorable=\"w14\"><w:body/></w:document>"
        );
        assert_eq!(repair(&xml), None);
    }

    #[test]
    fn declares_well_known_prefixes_named_by_ignorable() {
        let xml = format!(
            "<?xml version='1.0' encoding='UTF-8'?>\n<w:document xmlns:w=\"{W}\" xmlns:mc=\"{MC}\" xmlns:w14=\"{W14}\" mc:Ignorable=\"w14 w15 wp14\"><w:body/></w:document>"
        );
        let repaired = repair(&xml).expect("repaired");
        let root_start = repaired.find("<w:document").expect("root");
        let root_end = root_start + repaired[root_start..].find('>').expect("root end");
        let root = &repaired[root_start..root_end];
        assert!(
            root.contains(" xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\"")
        );
        assert!(root.contains(
            " xmlns:wp14=\"http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing\""
        ));
        assert!(root.contains("mc:Ignorable=\"w14 w15 wp14\""));
        assert!(repaired.starts_with("<?xml version='1.0' encoding='UTF-8'?>\n<w:document"));
        assert!(repaired.ends_with("<w:body/></w:document>"));
        // Idempotent: a repaired part is consistent.
        assert_eq!(repair(&repaired), None);
    }

    #[test]
    fn drops_unknown_prefixes_from_the_compatibility_lists() {
        let xml = format!(
            "<w:document xmlns:w=\"{W}\" xmlns:mc=\"{MC}\" xmlns:w14=\"{W14}\" mc:Ignorable=\"w14 zz w15\" mc:MustUnderstand=\"zz\"><w:body/></w:document>"
        );
        let repaired = repair(&xml).expect("repaired");
        assert!(repaired.contains("mc:Ignorable=\"w14 w15\""));
        assert!(repaired.contains("mc:MustUnderstand=\"\""));
        assert!(repaired.contains(" xmlns:w15=\""));
        assert!(!repaired.contains("xmlns:zz"));
    }

    #[test]
    fn declares_prefixes_required_by_alternate_content_choices() {
        let xml = format!(
            "<w:document xmlns:w=\"{W}\" xmlns:mc=\"{MC}\"><w:body><w:p><w:r><mc:AlternateContent><mc:Choice Requires=\"wpg\"><w:t>a</w:t></mc:Choice><mc:Fallback/></mc:AlternateContent></w:r></w:p></w:body></w:document>"
        );
        let repaired = repair(&xml).expect("repaired");
        let root_end = repaired.find('>').expect("root");
        assert!(repaired[..root_end].contains(
            " xmlns:wpg=\"http://schemas.microsoft.com/office/word/2010/wordprocessingGroup\""
        ));
        assert!(repaired.contains("<mc:Choice Requires=\"wpg\">"));
    }

    /// The scan reads the part as bytes and decodes only the tags it looks
    /// at, so text it never reads does not have to be valid UTF-8, and a
    /// longer name that starts with `mc:Choice` is not a choice.
    #[test]
    fn scans_choices_without_decoding_the_rest_of_the_part() {
        let mut xml =
            format!("<w:document xmlns:w=\"{W}\" xmlns:mc=\"{MC}\"><w:body><w:p><w:r><w:t>")
                .into_bytes();
        xml.extend_from_slice(&[0xff, 0xfe]);
        xml.extend_from_slice(
            b"</w:t><mc:Choicer Requires=\"w16\"/>\
              <mc:AlternateContent><mc:Choice Requires=\"wps\"/><mc:Fallback/></mc:AlternateContent>\
              </w:r></w:p></w:body></w:document>",
        );
        let root = repair_markup_compatibility_root_tag(&xml).expect("repaired");
        let tag = std::str::from_utf8(&root.tag).expect("tag");
        assert!(tag.contains(" xmlns:wps="), "{tag}");
        assert!(!tag.contains(" xmlns:w16="), "{tag}");
        assert_eq!(root.range.start, 0);
    }

    #[test]
    fn honours_a_self_closing_root_and_a_leading_comment() {
        let xml = format!(
            "<?xml version=\"1.0\"?><!-- generated --><w:styles xmlns:w=\"{W}\" xmlns:mc=\"{MC}\" mc:Ignorable=\"w14\"/>"
        );
        let repaired = repair(&xml).expect("repaired");
        assert_eq!(
            repaired,
            format!(
                "<?xml version=\"1.0\"?><!-- generated --><w:styles xmlns:w=\"{W}\" xmlns:mc=\"{MC}\" mc:Ignorable=\"w14\" xmlns:w14=\"{W14}\"/>"
            )
        );
    }

    #[test]
    fn ignores_parts_without_compatibility_attributes_and_non_xml() {
        assert_eq!(repair("<w:document><w:body/></w:document>"), None);
        assert_eq!(repair_markup_compatibility_prefixes(b"\x89PNG\r\n"), None);
        assert_eq!(repair(""), None);
    }

    #[test]
    fn only_wordprocessing_xml_parts_are_candidates() {
        assert!(is_wordprocessing_xml_part("word/document.xml"));
        assert!(is_wordprocessing_xml_part("word/styles.xml"));
        assert!(is_wordprocessing_xml_part("word/glossary/document.xml"));
        assert!(!is_wordprocessing_xml_part("word/media/image1.png"));
        assert!(!is_wordprocessing_xml_part("word/_rels/document.xml.rels"));
        assert!(!is_wordprocessing_xml_part("[Content_Types].xml"));
        assert!(!is_wordprocessing_xml_part("docProps/core.xml"));
    }

    #[test]
    fn root_tag_repair_reports_the_span_of_the_original_start_tag() {
        let xml = format!(
            "<?xml version=\"1.0\"?>\n<w:document xmlns:w=\"{W}\" xmlns:mc=\"{MC}\" mc:Ignorable=\"w14\"><w:body/></w:document>"
        );
        let repaired = repair_markup_compatibility_root_tag(xml.as_bytes()).expect("repaired");
        let start = xml.find("<w:document").expect("root");
        let end = xml.find("<w:body").expect("body");
        assert_eq!(repaired.range, start..end);
        assert_eq!(
            String::from_utf8(repaired.tag).expect("utf8"),
            format!(
                "<w:document xmlns:w=\"{W}\" xmlns:mc=\"{MC}\" mc:Ignorable=\"w14\" xmlns:w14=\"{W14}\">"
            )
        );
    }
}
