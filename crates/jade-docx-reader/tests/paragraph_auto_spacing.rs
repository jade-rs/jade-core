use jade_docx_reader::{document::Spacing, DocumentParser};

fn spacing(attributes: &str, paired: bool) -> Spacing {
    let tag = if paired {
        format!("<w:spacing {attributes}></w:spacing>")
    } else {
        format!("<w:spacing {attributes}/>")
    };
    let document = DocumentParser::new().parse_document_xml(&format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr>{tag}</w:pPr></w:p></w:body></w:document>"#
    )).unwrap();
    document.body.unwrap().elements[0]
        .as_paragraph()
        .unwrap()
        .properties
        .as_ref()
        .unwrap()
        .spacing
        .clone()
        .unwrap()
}

#[test]
fn explicit_off_and_masked_values_survive_serialization() {
    for paired in [false, true] {
        for off in ["0", "false", "off"] {
            let auto = spacing(
                r#"w:before="80" w:beforeAutospacing="1" w:afterAutospacing="on" w:afterLines="150""#,
                paired,
            );
            let mut reset = spacing(
                &format!(r#"w:beforeAutospacing="{off}" w:afterAutospacing="{off}""#),
                paired,
            );
            // The model is also saved as JSON by consumers; both unresolved
            // off values and numeric values hidden by auto must survive it.
            let auto: Spacing =
                serde_json::from_str(&serde_json::to_string(&auto).unwrap()).unwrap();
            reset = serde_json::from_str(&serde_json::to_string(&reset).unwrap()).unwrap();
            reset.inherit_spacing_sides_from(&auto);
            assert_eq!(
                (reset.before, reset.before_lines, reset.before_auto),
                (Some(4.0), None, false)
            );
            assert_eq!(
                (reset.after, reset.after_lines, reset.after_auto),
                (None, Some(1.5), false)
            );
            let restored: Spacing =
                serde_json::from_str(&serde_json::to_string(&reset).unwrap()).unwrap();
            assert_eq!(restored, reset);
        }
    }
}

#[test]
fn explicit_off_without_numeric_values_does_not_inherit_synthetic_auto_points() {
    let auto = spacing(r#"w:beforeAutospacing="1" w:afterAutospacing="1""#, false);
    let mut reset = spacing(r#"w:beforeAutospacing="0" w:afterAutospacing="0""#, false);
    reset.inherit_spacing_sides_from(&auto);
    assert_eq!((reset.before, reset.before_auto), (None, false));
    assert_eq!((reset.after, reset.after_auto), (None, false));
    reset.set_before_points(14.0);
    reset.set_after_points(6.0);
    reset.inherit_spacing_sides_from(&auto);
    assert_eq!((reset.before, reset.before_auto), (Some(14.0), false));
    assert_eq!((reset.after, reset.after_auto), (Some(6.0), false));
}
