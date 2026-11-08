use jade_docx_reader::{BodyElement, DocumentParser, StylesPart};

#[test]
fn char_spacing_inherits_and_zero_resets_the_style_spacing() {
    for paired in [false, true] {
        let spacing = |value: &str| {
            if paired {
                format!("<w:spacing w:val=\"{value}\"></w:spacing>")
            } else {
                format!("<w:spacing w:val=\"{value}\"/>")
            }
        };
        let styles = StylesPart::from_xml(&format!(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr>{}</w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Lower"><w:rPr>{}</w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Lower"/></w:style>
<w:style w:type="character" w:styleId="Reset"><w:rPr>{}</w:rPr></w:style>
</w:styles>"#,
            spacing("120"),
            spacing("-80"),
            spacing("0")
        ))
        .unwrap();
        assert_eq!(
            styles
                .resolve_effective_run_properties(None, None)
                .unwrap()
                .char_spacing,
            Some(6.0)
        );
        for paragraph in ["Lower", "Child"] {
            for (properties, expected) in [
                (String::new(), -4.0),
                ("<w:rStyle w:val=\"Reset\"/>".into(), 0.0),
                (spacing("0"), 0.0),
                (spacing("30"), 1.5),
                (spacing("NaN"), -4.0),
                (spacing("inf"), -4.0),
                (spacing("bad"), -4.0),
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
                    resolved.char_spacing,
                    Some(expected),
                    "paired={paired}, paragraph={paragraph}, properties={properties}"
                );
            }
        }
    }
}

#[test]
fn invalid_or_out_of_scope_spacings_do_not_override_inherited_values() {
    for value in ["NaN", "inf", "-inf", "bad", ""] {
        let styles = StylesPart::from_xml(&format!(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:spacing w:val="120"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Child"><w:pPr><w:spacing w:val="20"/></w:pPr><w:rPr><w:spacing w:val="{value}"/></w:rPr></w:style></w:styles>"#)).unwrap();
        assert_eq!(
            styles
                .resolve_effective_run_properties(Some("Child"), None)
                .unwrap()
                .char_spacing,
            Some(6.0)
        );
    }
}

#[test]
fn paragraph_mark_spacing_does_not_replace_paragraph_spacing() {
    for tag in [
        r#"<w:spacing w:val="40"/>"#,
        r#"<w:spacing w:val="40"></w:spacing>"#,
    ] {
        let xml = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:spacing w:after="240"/><w:rPr>{tag}</w:rPr></w:pPr><w:r><w:t>text</w:t></w:r></w:p></w:body></w:document>"#
        );
        let document = DocumentParser::new().parse_document_xml(&xml).unwrap();
        let BodyElement::Paragraph(p) = &document.body.unwrap().elements[0] else {
            panic!("paragraph")
        };
        let properties = p.properties.as_ref().unwrap();
        assert_eq!(
            properties
                .mark_run_properties
                .as_ref()
                .unwrap()
                .char_spacing,
            Some(2.0)
        );
        assert_eq!(properties.spacing.as_ref().unwrap().after, Some(12.0));
        assert!(p.runs[0]
            .properties
            .as_ref()
            .and_then(|p| p.char_spacing)
            .is_none());
    }
}
