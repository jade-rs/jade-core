use jade_docx_reader::{RunProperties, StylesPart};

#[test]
fn visual_styles_inherit_and_explicit_resets_override() {
    for baseline in ["superscript", "subscript"] {
        for paired in [false, true] {
            let properties = |reset: bool| {
                let values = if reset {
                    ["0", "none", "baseline"]
                } else {
                    ["1", "yellow", baseline]
                };
                ["dstrike", "highlight", "vertAlign"]
                    .into_iter()
                    .zip(values)
                    .map(|(tag, value)| {
                        if paired {
                            format!("<w:{tag} w:val=\"{value}\"></w:{tag}>")
                        } else {
                            format!("<w:{tag} w:val=\"{value}\"/>")
                        }
                    })
                    .collect::<String>()
            };
            let xml = format!(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr>{}</w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Base"><w:rPr>{}</w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style>
<w:style w:type="character" w:styleId="Reset"><w:rPr>{}</w:rPr></w:style>
</w:styles>"#,
                properties(false),
                properties(false),
                properties(true)
            );
            let styles = StylesPart::from_xml(&xml).unwrap();
            for paragraph in [None, Some("Base"), Some("Child")] {
                let resolved = styles
                    .resolve_effective_run_properties(paragraph, None)
                    .unwrap();
                assert_eq!(resolved.double_strike, Some(true));
                assert_eq!(resolved.highlight.as_deref(), Some("yellow"));
                assert_eq!(resolved.baseline.as_deref(), Some(baseline));
                let reset = RunProperties {
                    style: Some("Reset".into()),
                    ..Default::default()
                };
                let resolved = styles
                    .resolve_effective_run_properties(paragraph, Some(&reset))
                    .unwrap();
                assert_eq!(resolved.double_strike, Some(false));
                assert_eq!(resolved.highlight.as_deref(), Some("none"));
                assert_eq!(resolved.baseline.as_deref(), Some("baseline"));
                let direct = RunProperties {
                    double_strike: Some(false),
                    highlight: Some("none".into()),
                    baseline: Some("baseline".into()),
                    ..Default::default()
                };
                let resolved = styles
                    .resolve_effective_run_properties(paragraph, Some(&direct))
                    .unwrap();
                assert_eq!(resolved.double_strike, Some(false));
                assert_eq!(resolved.highlight.as_deref(), Some("none"));
                assert_eq!(resolved.baseline.as_deref(), Some("baseline"));
            }
        }
    }
}

#[test]
fn visual_properties_outside_run_properties_are_ignored() {
    let styles = StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:dstrike/><w:highlight w:val="yellow"/><w:vertAlign w:val="superscript"/></w:pPr></w:style></w:styles>"#).unwrap();
    let resolved = styles
        .resolve_effective_run_properties(Some("Base"), None)
        .unwrap_or_default();
    assert_eq!(resolved.double_strike, None);
    assert_eq!(resolved.highlight, None);
    assert_eq!(resolved.baseline, None);
}
