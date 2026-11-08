use jade_docx_reader::{BodyElement, DocumentParser, StylesPart};

#[test]
fn text_position_inherits_and_zero_resets_the_style_shift() {
    for paired in [false, true] {
        let position = |value: &str| {
            if paired {
                format!("<w:position w:val=\"{value}\"></w:position>")
            } else {
                format!("<w:position w:val=\"{value}\"/>")
            }
        };
        let styles = StylesPart::from_xml(&format!(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr>{}</w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Lower"><w:rPr>{}</w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Lower"/></w:style>
<w:style w:type="character" w:styleId="Reset"><w:rPr>{}</w:rPr></w:style>
</w:styles>"#,
            position("12"),
            position("-8"),
            position("0")
        ))
        .unwrap();
        assert_eq!(
            styles
                .resolve_effective_run_properties(None, None)
                .unwrap()
                .text_position,
            Some(6.0)
        );
        for paragraph in ["Lower", "Child"] {
            for (properties, expected) in [
                (String::new(), -4.0),
                ("<w:rStyle w:val=\"Reset\"/>".into(), 0.0),
                (position("0"), 0.0),
                (position("3"), 1.5),
            ] {
                let xml = format!(
                    r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:rPr>{properties}</w:rPr><w:t>text</w:t></w:r></w:p></w:body></w:document>"#
                );
                let document = DocumentParser::new().parse_document_xml(&xml).unwrap();
                let BodyElement::Paragraph(p) = &document.body.unwrap().elements[0] else {
                    panic!("paragraph")
                };
                let resolved = styles
                    .resolve_effective_run_properties(
                        Some(paragraph),
                        p.runs[0].properties.as_ref(),
                    )
                    .unwrap();
                assert_eq!(
                    resolved.text_position,
                    Some(expected),
                    "paired={paired}, paragraph={paragraph}, properties={properties}"
                );
            }
        }
    }
}

#[test]
fn invalid_or_out_of_scope_positions_do_not_override_inherited_values() {
    for value in ["NaN", "inf", "-inf", "bad", ""] {
        let styles = StylesPart::from_xml(&format!(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:position w:val="12"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Child"><w:pPr><w:position w:val="20"/></w:pPr><w:rPr><w:position w:val="{value}"/></w:rPr></w:style></w:styles>"#)).unwrap();
        assert_eq!(
            styles
                .resolve_effective_run_properties(Some("Child"), None)
                .unwrap()
                .text_position,
            Some(6.0)
        );
    }
}
