use jade_docx_reader::{BodyElement, DocumentParser, StylesPart};

fn border(paired: bool, val: &str, color: &str) -> String {
    let attrs = format!("w:val=\"{val}\" w:sz=\"12\" w:space=\"2\" w:color=\"{color}\"");
    if paired {
        format!("<w:bdr {attrs}></w:bdr>")
    } else {
        format!("<w:bdr {attrs}/>")
    }
}

#[test]
fn borders_inherit_and_explicit_removal_overrides_the_style() {
    for paired in [false, true] {
        let styles = StylesPart::from_xml(&format!(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr>{}</w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Base"><w:rPr>{}</w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style>
<w:style w:type="character" w:styleId="Reset"><w:rPr>{}</w:rPr></w:style>
</w:styles>"#,
            border(paired, "single", "FF0000"),
            border(paired, "double", "00FF00"),
            border(paired, "nil", "auto")
        ))
        .unwrap();
        for (paragraph, inherited, color) in [
            (None, "single", "#FF0000"),
            (Some("Base"), "double", "#00FF00"),
            (Some("Child"), "double", "#00FF00"),
        ] {
            for (direct, val, expected_color) in [
                (String::new(), inherited, Some(color)),
                ("<w:rStyle w:val=\"Reset\"/>".into(), "nil", None),
                (border(paired, "none", "auto"), "none", None),
                (
                    border(paired, "dashed", "123456"),
                    "dashed",
                    Some("#123456"),
                ),
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
                let edge = resolved
                    .border
                    .as_ref()
                    .expect("explicit or inherited border");
                assert_eq!(
                    edge.val.as_deref(),
                    Some(val),
                    "paired={paired}, paragraph={paragraph:?}, direct={direct}"
                );
                assert_eq!(edge.color.as_deref(), expected_color);
                assert_eq!(edge.size_eighths, Some(12));
                assert_eq!(edge.space, Some(2));
                assert_eq!(edge.is_visible(), !matches!(val, "nil" | "none"));
            }
        }
    }
}

#[test]
fn borders_outside_run_properties_do_not_leak_into_runs() {
    let styles = StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:bdr w:val="single"/></w:pPr></w:style></w:styles>"#).unwrap();
    assert_eq!(
        styles
            .resolve_effective_run_properties(Some("Base"), None)
            .unwrap_or_default()
            .border,
        None
    );
}
