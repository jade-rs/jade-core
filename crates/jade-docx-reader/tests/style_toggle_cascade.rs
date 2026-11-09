use jade_docx_reader::{RunProperties, StylesPart};

macro_rules! check_toggle {
    ($name:ident, $tag:literal, $field:ident) => {
#[test]
fn $name() {
    for default in [false, true] {
        for base in [false, true] {
            for child in [false, true] {
                for character in [false, true] {
                    let tag = $tag;
                    let xml = format!(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:{tag} w:val="{default}"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Base"><w:rPr><w:{tag} w:val="{base}"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/><w:rPr><w:{tag} w:val="{child}"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Character"><w:rPr><w:{tag} w:val="{character}"/></w:rPr></w:style>
</w:styles>"#);
                    let styles = StylesPart::from_xml(&xml).unwrap();
                    for direct in [None, Some(false), Some(true)] {
                        let properties = RunProperties {
                            style: Some("Character".into()),
                            $field: direct,
                            ..Default::default()
                        };
                        let resolved = styles.resolve_effective_run_properties(Some("Child"), Some(&properties)).unwrap();
                        assert_eq!(resolved.$field, Some(direct.unwrap_or(default ^ base ^ child ^ character)),
                            "defaults={default}, base={base}, child={child}, character={character}, direct={direct:?}");
                    }
                }
            }
        }
    }
}
    };
}

check_toggle!(
    bold_styles_toggle_and_direct_formatting_overrides,
    "b",
    bold
);
check_toggle!(
    bold_complex_styles_toggle_and_direct_formatting_overrides,
    "bCs",
    bold_complex
);
check_toggle!(
    italic_styles_toggle_and_direct_formatting_overrides,
    "i",
    italic
);
check_toggle!(
    italic_complex_styles_toggle_and_direct_formatting_overrides,
    "iCs",
    italic_complex
);
check_toggle!(
    caps_styles_toggle_and_direct_formatting_overrides,
    "caps",
    caps
);
check_toggle!(
    small_caps_styles_toggle_and_direct_formatting_overrides,
    "smallCaps",
    small_caps
);
check_toggle!(
    strike_styles_toggle_and_direct_formatting_overrides,
    "strike",
    strike
);
check_toggle!(
    outline_styles_toggle_and_direct_formatting_overrides,
    "outline",
    outline
);
check_toggle!(
    shadow_styles_toggle_and_direct_formatting_overrides,
    "shadow",
    shadow
);
check_toggle!(
    emboss_styles_toggle_and_direct_formatting_overrides,
    "emboss",
    emboss
);
check_toggle!(
    imprint_styles_toggle_and_direct_formatting_overrides,
    "imprint",
    imprint
);
check_toggle!(
    vanish_styles_toggle_and_direct_formatting_overrides,
    "vanish",
    vanish
);
