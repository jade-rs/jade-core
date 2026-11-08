use jade_docx_reader::{BodyElement, DocumentParser, RunProperties, StylesPart};

fn direct(rpr: &str) -> RunProperties {
    let document = DocumentParser::new().parse_document_xml(&format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body><w:p><w:r><w:rPr>{rpr}</w:rPr><w:t>Glow</w:t></w:r></w:p></w:body></w:document>"#)).unwrap();
    let BodyElement::Paragraph(p) = &document.body.unwrap().elements[0] else {
        panic!("paragraph")
    };
    p.runs[0].properties.clone().unwrap()
}

#[test]
fn alpha_is_opacity_and_absence_is_opaque() {
    for (alpha, expected) in [
        ("<w14:alpha w14:val=\"0\"/>", 0.0),
        ("<w14:alpha w14:val=\"40000\"/>", 0.4),
        ("<w14:alpha w14:val=\"100000\"/>", 1.0),
        ("", 1.0),
    ] {
        let props = direct(&format!(
            r#"<w14:glow w14:rad="63500"><w14:srgbClr w14:val="FF0000">{alpha}</w14:srgbClr></w14:glow>"#
        ));
        let effects = props.text_effects.unwrap();
        assert_eq!(effects.glow_alpha, Some(expected), "{alpha}");
        assert_eq!(effects.glow_radius, Some(5.0));
    }
}

fn styles() -> StylesPart {
    StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:docDefaults><w:rPrDefault><w:rPr><w14:glow w14:rad="63500"><w14:srgbClr w14:val="FF0000"><w14:alpha w14:val="40000"/></w14:srgbClr></w14:glow><w14:textOutline w14:w="12700"><w14:solidFill><w14:srgbClr w14:val="00FF00"/></w14:solidFill></w14:textOutline></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Base"><w:rPr><w:i/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style><w:style w:type="character" w:styleId="Blue"><w:rPr><w14:glow><w14:srgbClr w14:val="0000FF"/></w14:glow></w:rPr></w:style></w:styles>"#).unwrap()
}

#[test]
fn style_effects_inherit_and_replacements_use_their_own_defaults() {
    let styles = styles();
    let inherited = styles
        .resolve_effective_run_properties(Some("Child"), None)
        .unwrap()
        .text_effects
        .unwrap();
    assert_eq!(inherited.glow_color.as_deref(), Some("#FF0000"));
    assert_eq!(inherited.glow_radius, Some(5.0));
    assert_eq!(inherited.glow_alpha, Some(0.4));
    assert_eq!(inherited.outline_color.as_deref(), Some("#00FF00"));
    assert_eq!(inherited.outline_width, Some(1.0));
    let props = direct("<w:rStyle w:val=\"Blue\"/>");
    let replaced = styles
        .resolve_effective_run_properties(Some("Child"), Some(&props))
        .unwrap()
        .text_effects
        .unwrap();
    assert_eq!(replaced.glow_color.as_deref(), Some("#0000FF"));
    assert_eq!(replaced.glow_radius, Some(0.0));
    assert_eq!(replaced.glow_alpha, Some(1.0));
    assert_eq!(replaced.outline_color, inherited.outline_color);
}

#[test]
fn direct_empty_effects_cancel_inherited_effects() {
    let styles = styles();
    for rpr in [
        "<w14:glow/><w14:textOutline/>",
        "<w14:glow></w14:glow><w14:textOutline><w14:noFill/></w14:textOutline>",
    ] {
        let props = direct(rpr);
        let effects = styles
            .resolve_effective_run_properties(Some("Child"), Some(&props))
            .unwrap()
            .text_effects
            .unwrap();
        assert_eq!(effects.glow_color, None, "{rpr}");
        assert_eq!(effects.glow_radius, Some(0.0), "{rpr}");
        assert_eq!(effects.outline_color, None, "{rpr}");
        assert_eq!(effects.outline_width, Some(0.0), "{rpr}");
    }
}
