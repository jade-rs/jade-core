use jade_docx_reader::{BodyElement, DocumentParser, StylesPart};

fn shading(paired: bool, val: &str, fill: &str) -> String {
    let attrs = format!("w:val=\"{val}\" w:fill=\"{fill}\"");
    if paired {
        format!("<w:shd {attrs}></w:shd>")
    } else {
        format!("<w:shd {attrs}/>")
    }
}

#[test]
fn run_shading_inherits_and_nil_overrides_the_style() {
    for paired in [false, true] {
        let styles = StylesPart::from_xml(&format!(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr>{}</w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Base"><w:rPr>{}</w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style>
<w:style w:type="character" w:styleId="Reset"><w:rPr>{}</w:rPr></w:style>
</w:styles>"#,
            shading(paired, "clear", "FFFF00"),
            shading(paired, "clear", "00FF00"),
            shading(paired, "nil", "auto"),
        ))
        .unwrap();
        for (paragraph, fill) in [
            (None, "#FFFF00"),
            (Some("Base"), "#00FF00"),
            (Some("Child"), "#00FF00"),
        ] {
            for (direct, expected) in [
                (String::new(), Some(fill)),
                ("<w:rStyle w:val=\"Reset\"/>".into(), None),
                (shading(paired, "nil", "auto"), None),
                (shading(paired, "nil", "ABCDEF"), Some("#ABCDEF")),
                (shading(paired, "clear", "123456"), Some("#123456")),
            ] {
                let document = DocumentParser::new().parse_document_xml(&format!(
                    r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:rPr>{direct}</w:rPr><w:t>text</w:t></w:r></w:p></w:body></w:document>"#
                )).unwrap();
                let BodyElement::Paragraph(p) = &document.body.as_ref().unwrap().elements[0] else {
                    panic!("paragraph")
                };
                let resolved = styles
                    .resolve_effective_run_properties(paragraph, p.runs[0].properties.as_ref())
                    .unwrap();
                let shading = resolved
                    .shading
                    .as_ref()
                    .expect("explicit or inherited shading");
                assert_eq!(
                    shading.resolved_background_color().as_deref(),
                    expected,
                    "paired={paired}, paragraph={paragraph:?}, direct={direct}"
                );
            }
        }
    }
}

#[test]
fn paragraph_and_mark_shading_keep_separate_scopes() {
    for paired in [false, true] {
        let styles = StylesPart::from_xml(&format!(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr>{}</w:pPr></w:style></w:styles>"#,
            shading(paired, "clear", "FFFF00")
        )).unwrap();
        assert_eq!(
            styles
                .resolve_effective_run_properties(Some("Base"), None)
                .unwrap_or_default()
                .shading,
            None
        );
        let document = DocumentParser::new().parse_document_xml(&format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr>{}<w:rPr>{}</w:rPr></w:pPr><w:r><w:t>text</w:t></w:r></w:p></w:body></w:document>"#,
            shading(paired, "clear", "FFFF00"), shading(paired, "clear", "00FF00")
        )).unwrap();
        let BodyElement::Paragraph(p) = &document.body.as_ref().unwrap().elements[0] else {
            panic!("paragraph")
        };
        let props = p.properties.as_ref().unwrap();
        assert_eq!(
            props.shading.as_ref().unwrap().fill.as_deref(),
            Some("#FFFF00")
        );
        assert_eq!(
            props
                .mark_run_properties
                .as_ref()
                .unwrap()
                .shading
                .as_ref()
                .unwrap()
                .fill
                .as_deref(),
            Some("#00FF00")
        );
        assert_eq!(
            p.runs[0]
                .properties
                .as_ref()
                .and_then(|p| p.shading.as_ref()),
            None
        );
    }
}
