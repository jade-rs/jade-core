use jade_docx_reader::{RunProperties, StylesPart};
use std::rc::Rc;

#[test]
fn hidden_styles_toggle_but_direct_formatting_sets_an_absolute_value() {
    for default in [false, true] {
        for paragraph in [false, true] {
            for child in [false, true] {
                for character in [false, true] {
                    let xml = format!(
                        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:vanish w:val="{default}"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Base"><w:rPr><w:vanish w:val="{paragraph}"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/><w:rPr><w:vanish w:val="{child}"></w:vanish></w:rPr></w:style>
<w:style w:type="character" w:styleId="Hidden"><w:rPr><w:vanish w:val="{character}"/></w:rPr></w:style>
</w:styles>"#
                    );
                    let styles = StylesPart::from_xml(&xml).unwrap();
                    for direct in [None, Some(false), Some(true)] {
                        let hint = RunProperties {
                            style: Some(Rc::from("Hidden")),
                            vanish: direct,
                            ..Default::default()
                        };
                        let resolved = styles
                            .resolve_effective_run_properties(Some("Child"), Some(&hint))
                            .unwrap();
                        assert_eq!(resolved.vanish, Some(direct.unwrap_or(default ^ paragraph ^ child ^ character)), "defaults={default}, paragraph={paragraph}, child={child}, character={character}, direct={direct:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn hidden_property_is_parsed_in_defaults_and_styles_without_leaking_outside_run_properties() {
    let styles = StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:vanish/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Inherited"><w:rPr/></w:style>
<w:style w:type="character" w:styleId="Hidden"><w:rPr><w:vanish/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Outside"><w:vanish/><w:rPr><w:color w:val="FF0000"/></w:rPr></w:style>
</w:styles>"#).unwrap();
    assert_eq!(
        styles.doc_default_run_properties().unwrap().vanish,
        Some(true)
    );
    assert_eq!(
        styles
            .resolve_character_run_properties("Hidden")
            .unwrap()
            .vanish,
        Some(true)
    );
    assert_eq!(
        styles
            .resolve_character_run_properties("Outside")
            .unwrap()
            .vanish,
        None
    );
    assert_eq!(
        styles
            .resolve_effective_run_properties(Some("Inherited"), None)
            .unwrap()
            .vanish,
        Some(true)
    );
}
