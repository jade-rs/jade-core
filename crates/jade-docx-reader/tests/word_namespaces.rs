use jade_docx_reader::{BodyElement, Document, DocumentParser};
use proptest::prelude::*;

const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn text(document: &Document) -> String {
    document
        .body
        .as_ref()
        .expect("document body")
        .elements
        .iter()
        .flat_map(|element| match element {
            BodyElement::Paragraph(paragraph) => paragraph.runs.iter(),
            _ => panic!("paragraph expected"),
        })
        .map(|run| run.text.as_str())
        .collect()
}

#[test]
fn aliased_word_namespace_reads_text_and_raw_tree() {
    let xml = format!(
        r#"<ns0:document xmlns:ns0="{WORD}"><ns0:body><ns0:p><ns0:r><ns0:t xml:space="preserve"> 日本語 &amp; text </ns0:t></ns0:r></ns0:p></ns0:body></ns0:document>"#
    );
    let parser = DocumentParser::new();
    assert_eq!(
        text(&parser.parse_document_xml(&xml).unwrap()),
        " 日本語 & text "
    );
    let (document, tree) = parser.parse_document_xml_with_raw_tree(&xml).unwrap();
    assert_eq!(text(&document), " 日本語 & text ");
    assert_eq!(tree.unwrap().root().name, "w:document");
}

#[test]
fn default_namespace_and_scoped_alias_rebinding_are_resolved() {
    let xml = format!(
        r#"<document xmlns="{WORD}" xmlns:a="{WORD}"><body><a:p><a:r><a:t>Before</a:t></a:r></a:p><a:extension xmlns:a="urn:extension"><a:p><a:r><a:t>Hidden</a:t></a:r></a:p></a:extension><a:p><a:r><a:t>After</a:t></a:r></a:p></body></document>"#
    );
    assert_eq!(
        text(&DocumentParser::new().parse_document_xml(&xml).unwrap()),
        "BeforeAfter"
    );
}

#[test]
fn word_attribute_aliases_are_used_but_unqualified_attributes_are_not() {
    let xml = format!(
        r#"<document xmlns='{WORD}' xmlns:n='{WORD}'><body><p><pPr><jc val='left' n:val='right'/></pPr><r><t>Text</t></r></p></body></document>"#
    );
    let document = DocumentParser::new().parse_document_xml(&xml).unwrap();
    let BodyElement::Paragraph(paragraph) = &document.body.as_ref().unwrap().elements[0] else {
        panic!("paragraph expected");
    };
    assert_eq!(
        paragraph.properties.as_ref().unwrap().alignment.as_deref(),
        Some("right")
    );
}

#[test]
fn canonical_root_still_resolves_aliases_declared_in_the_body() {
    let xml = format!(
        r#"<w:document xmlns:w='{WORD}'><w:body><n:p xmlns:n='{WORD}'><n:r><n:t>Nested alias</n:t></n:r></n:p><p xmlns='{WORD}'><r><t xml:space='preserve'> and default</t></r></p></w:body></w:document>"#
    );
    let parser = DocumentParser::new();
    let (document, tree) = parser.parse_document_xml_with_raw_tree(&xml).unwrap();
    assert_eq!(text(&document), "Nested alias and default");
    assert!(tree
        .unwrap()
        .to_string()
        .unwrap()
        .contains("<w:t>Nested alias</w:t>"));
}

proptest! {
    #[test]
    fn arbitrary_word_prefix_matches_canonical_document(prefix in "ns[A-Za-z0-9_]{0,12}") {
        let canonical = format!(r#"<w:document xmlns:w="{WORD}"><w:body><w:p><w:r><w:t>Stable text</w:t></w:r></w:p></w:body></w:document>"#);
        let aliased = canonical.replace("w:", &format!("{prefix}:")).replace("xmlns:w=", &format!("xmlns:{prefix}="));
        let parser = DocumentParser::new();
        prop_assert_eq!(text(&parser.parse_document_xml(&aliased).unwrap()), text(&parser.parse_document_xml(&canonical).unwrap()));
    }

    #[test]
    fn normalization_preserves_unicode_text_and_namespace_scope(
        prefix in "ns[A-Za-z0-9_]{0,12}",
        content in "[ぁ-ん一-龥a-zA-Z0-9 &<>]{0,80}",
    ) {
        let escaped = quick_xml::escape::escape(&content);
        let xml = format!(r#"<{prefix}:document xmlns:{prefix}='{WORD}'><{prefix}:body><{prefix}:p><{prefix}:r><{prefix}:t xml:space='preserve'>{escaped}</{prefix}:t></{prefix}:r></{prefix}:p><{prefix}:opaque xmlns:{prefix}='urn:foreign'><{prefix}:p>hidden</{prefix}:p></{prefix}:opaque><{prefix}:p><{prefix}:r><{prefix}:t>after</{prefix}:t></{prefix}:r></{prefix}:p></{prefix}:body></{prefix}:document>"#);
        let normalized = jade_docx_reader::word_namespaces::normalize(&xml).unwrap();
        prop_assert_eq!(text(&DocumentParser::new().parse_document_xml(&xml).unwrap()), format!("{content}after"));
        let foreign = format!("<{prefix}:p>hidden</{prefix}:p>");
        prop_assert!(normalized.contains(&foreign));
        let repeated = jade_docx_reader::word_namespaces::normalize(&normalized).unwrap();
        prop_assert_eq!(repeated.as_ref(), normalized.as_ref());
    }
}
