use jade_docx_reader::{BodyElement, DocumentParser, StylesPart};

#[test]
fn emphasis_inherits_and_none_cancels_the_style() {
    for paired in [false, true] {
        let mark = |value: &str| {
            if paired {
                format!("<w:em w:val=\"{value}\"></w:em>")
            } else {
                format!("<w:em w:val=\"{value}\"/>")
            }
        };
        let styles = StylesPart::from_xml(&format!(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr>{}</w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Base"><w:rPr>{}</w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style>
<w:style w:type="character" w:styleId="Reset"><w:rPr>{}</w:rPr></w:style>
</w:styles>"#,
            mark("dot"),
            mark("underDot"),
            mark("none")
        ))
        .unwrap();
        for (paragraph, inherited) in [
            (None, "dot"),
            (Some("Base"), "underdot"),
            (Some("Child"), "underdot"),
        ] {
            for (direct, expected) in [
                (String::new(), inherited),
                ("<w:rStyle w:val=\"Reset\"/>".into(), "none"),
                (mark("none"), "none"),
                (mark("circle"), "circle"),
            ] {
                let document = DocumentParser::new().parse_document_xml(&format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:rPr>{direct}</w:rPr><w:t>text</w:t></w:r></w:p></w:body></w:document>"#)).unwrap();
                let BodyElement::Paragraph(p) = &document.body.as_ref().unwrap().elements[0] else {
                    panic!("paragraph")
                };
                let resolved = styles
                    .resolve_effective_run_properties(paragraph, p.runs[0].properties.as_ref())
                    .unwrap();
                assert_eq!(
                    resolved.emphasis.as_deref(),
                    Some(expected),
                    "paired={paired}, paragraph={paragraph:?}, direct={direct}"
                );
            }
        }
    }
}

#[test]
fn emphasis_outside_run_properties_does_not_leak() {
    let styles = StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:em w:val="dot"/></w:pPr></w:style></w:styles>"#).unwrap();
    assert_eq!(
        styles
            .resolve_effective_run_properties(Some("Base"), None)
            .unwrap_or_default()
            .emphasis,
        None
    );
}
