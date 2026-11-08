use jade_docx_reader::{document::RunTextEffects, RunProperties, StylesPart};

#[test]
fn typography_inherits_per_setting_and_preserves_explicit_resets() {
    for paired in [false, true] {
        let element = |name: &str, value: &str| {
            if paired {
                format!("<w14:{name} w14:val=\"{value}\"></w14:{name}>")
            } else {
                format!("<w14:{name} w14:val=\"{value}\"/>")
            }
        };
        let styles = StylesPart::from_xml(&format!(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
<w:docDefaults><w:rPrDefault><w:rPr>{}{}</w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Base"><w:rPr>{}{}</w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style>
<w:style w:type="character" w:styleId="Reset"><w:rPr>{}{}{}</w:rPr></w:style>
</w:styles>"#, element("ligatures", "standard"), element("numSpacing", "proportional"),
            element("numForm", "oldStyle"), element("cntxtAlts", "1"),
            element("ligatures", "none"), element("numForm", "default"), element("cntxtAlts", "0"))).unwrap();
        for paragraph in ["Base", "Child"] {
            let inherited = styles
                .resolve_effective_run_properties(Some(paragraph), None)
                .unwrap();
            let effects = inherited.text_effects.unwrap();
            assert_eq!(effects.ligatures.as_deref(), Some("standard"));
            assert_eq!(effects.number_form.as_deref(), Some("oldStyle"));
            assert_eq!(effects.contextual_alternates, Some(true));
            let direct = RunProperties {
                style: Some("Reset".into()),
                text_effects: Some(Box::new(RunTextEffects {
                    number_spacing: Some("tabular".into()),
                    ..Default::default()
                })),
                ..Default::default()
            };
            let reset = styles
                .resolve_effective_run_properties(Some(paragraph), Some(&direct))
                .unwrap();
            let effects = reset.text_effects.unwrap();
            assert_eq!(effects.ligatures.as_deref(), Some("none"));
            assert_eq!(effects.number_form.as_deref(), Some("default"));
            assert_eq!(effects.number_spacing.as_deref(), Some("tabular"));
            assert_eq!(effects.contextual_alternates, Some(false));
        }
    }
}

#[test]
fn typography_outside_run_properties_does_not_leak() {
    let styles = StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:style w:type="paragraph" w:styleId="Base"><w:pPr><w14:ligatures w14:val="standard"/><w14:numForm w14:val="oldStyle"/><w14:numSpacing w14:val="tabular"/><w14:cntxtAlts/></w:pPr></w:style></w:styles>"#).unwrap();
    assert!(styles
        .resolve_effective_run_properties(Some("Base"), None)
        .unwrap_or_default()
        .text_effects
        .is_none());
}
