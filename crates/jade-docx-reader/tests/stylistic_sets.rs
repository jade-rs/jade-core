use jade_docx_reader::{BodyElement, DocumentParser};

#[test]
fn all_enabled_stylistic_sets_are_read() {
    let document = DocumentParser::new().parse_document_xml(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body><w:p><w:r><w:rPr><w14:stylisticSets><w14:styleSet w14:id="2" w14:val="false"/><w14:styleSet w14:id="1"/><w14:styleSet w14:id="4"></w14:styleSet><w14:styleSet w14:id="20"/><w14:styleSet w14:id="4"/><w14:styleSet w14:id="21"/></w14:stylisticSets></w:rPr><w:t>Office</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
    let BodyElement::Paragraph(p) = &document.body.unwrap().elements[0] else {
        panic!("paragraph")
    };
    let effects = p.runs[0]
        .properties
        .as_ref()
        .unwrap()
        .text_effects
        .as_ref()
        .unwrap();
    assert_eq!(effects.stylistic_set, Some(1));
    assert_eq!(
        serde_json::to_value(effects).unwrap()["stylisticSets"],
        serde_json::json!([1, 4, 20])
    );
}

#[test]
fn styles_inherit_sets_and_empty_direct_sets_reset_them() {
    use jade_docx_reader::StylesPart;
    let styles = StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:docDefaults><w:rPrDefault><w:rPr><w14:stylisticSets><w14:styleSet w14:id="1"/><w14:styleSet w14:id="4"/><w14:styleSet w14:id="2" w14:val="0"/></w14:stylisticSets></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Child"><w:rPr><w:i/></w:rPr></w:style><w:style w:type="character" w:styleId="Reset"><w:rPr><w14:stylisticSets/></w:rPr></w:style></w:styles>"#).unwrap();
    let inherited = styles
        .resolve_effective_run_properties(Some("Child"), None)
        .unwrap();
    assert_eq!(
        inherited.text_effects.unwrap().stylistic_sets,
        Some(vec![1, 4])
    );
    for rpr in [
        "<w14:stylisticSets/>",
        "<w14:stylisticSets></w14:stylisticSets>",
        "<w:rStyle w:val=\"Reset\"/>",
    ] {
        let document = DocumentParser::new().parse_document_xml(&format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body><w:p><w:r><w:rPr>{rpr}</w:rPr><w:t>Office</w:t></w:r></w:p></w:body></w:document>"#)).unwrap();
        let BodyElement::Paragraph(p) = &document.body.unwrap().elements[0] else {
            panic!("paragraph")
        };
        let props = styles
            .resolve_effective_run_properties(Some("Child"), p.runs[0].properties.as_ref())
            .unwrap();
        let effects = props.text_effects.unwrap();
        assert_eq!(effects.stylistic_set, None, "{rpr}");
        assert_eq!(effects.stylistic_sets, Some(vec![]), "{rpr}");
    }
}
