#![cfg(feature = "opentype-math")]

use jade_docx_layout::math_layout::styled::{
    font::{MathFontData, MathFontLayoutError},
    MathTextStyle, MathVariant, StyledMathMeasure,
};
use jade_docx_layout::math_layout::MathMetrics;
use jade_docx_reader::document::{MathElement, MathElementKind as K, MathParagraph};
use std::sync::Arc;

const FONT: &[u8] = include_bytes!("fixtures/math-script-font.ttf");
struct Fallback;
impl StyledMathMeasure for Fallback {
    type Error = &'static str;
    fn measure_glyph(&self, _: &str, style: MathTextStyle<'_>) -> Result<MathMetrics, Self::Error> {
        Ok(MathMetrics {
            width: style.size_px * 0.5,
            ascent: style.size_px * 0.8,
            descent: style.size_px * 0.2,
        })
    }
}
fn node(kind: K, children: Vec<MathElement>) -> MathElement {
    MathElement {
        kind,
        children,
        ..Default::default()
    }
}
fn text(value: &str) -> MathElement {
    MathElement {
        kind: K::Text,
        text: Some(value.into()),
        ..Default::default()
    }
}
fn sup(base: MathElement, script: MathElement) -> MathElement {
    node(
        K::Superscript,
        vec![
            node(K::Base, vec![base]),
            node(K::SuperArgument, vec![script]),
        ],
    )
}
fn nested() -> MathParagraph {
    MathParagraph {
        elements: vec![
            text("x"),
            sup(text("x"), sup(text("x"), text("x"))),
            text("x"),
        ],
        ..Default::default()
    }
}
fn style(size: f64) -> MathTextStyle<'static> {
    MathTextStyle {
        family: "Script",
        size_px: size,
        color: "000000",
        variant: MathVariant::Plain,
    }
}

#[test]
fn nested_scripts_choose_font_forms_and_keep_source_text_and_cache_identity() {
    let font = MathFontData::new("Script".into(), FONT.to_vec(), 3, 64).unwrap();
    let math = nested();
    let plan = font.layout(Fallback, &math, style(20.0), false).unwrap();
    assert_eq!(plan.text, "xxxxx");
    assert_eq!(
        plan.glyphs
            .iter()
            .map(|g| g.font_glyph.as_ref().unwrap().geometry.glyph_id)
            .collect::<Vec<_>>(),
        [1, 1, 2, 3, 1]
    );
    for (g, expected) in plan.glyphs.iter().zip([10.0, 10.0, 8.45, 6.4, 10.0]) {
        assert!((g.width - expected).abs() < 1e-9);
    }
    assert_eq!(font.cached_glyphs(), 3);
    let second = font.layout(Fallback, &math, style(40.0), false).unwrap();
    assert_eq!(font.cached_glyphs(), 3);
    for (a, b) in plan.glyphs.iter().zip(&second.glyphs) {
        let a = a.font_glyph.as_ref().unwrap();
        let b = b.font_glyph.as_ref().unwrap();
        assert!(Arc::ptr_eq(&a.outline, &b.outline));
        assert!((b.geometry.advance_px - a.geometry.advance_px * 2.0).abs() < 1e-9);
    }
}

#[test]
fn script_variants_obey_the_existing_outline_cache_budget() {
    let font = MathFontData::new("Script".into(), FONT.to_vec(), 2, 64).unwrap();
    assert!(matches!(
        font.layout(Fallback, &nested(), style(20.0), false),
        Err(MathFontLayoutError::GlyphCacheBudgetExceeded)
    ));
}

#[test]
fn missing_family_uses_host_measurement_without_caching_font_glyphs() {
    let font = MathFontData::new("Script".into(), FONT.to_vec(), 3, 64).unwrap();
    let math = nested();
    let mut other = style(20.0);
    other.family = "Other";
    let plan = font.layout(Fallback, &math, other, false).unwrap();
    assert_eq!(plan.text, "xxxxx");
    assert!(plan.glyphs.iter().all(|g| g.font_glyph.is_none()));
    assert_eq!(font.cached_glyphs(), 0);
}

#[test]
fn script_lookup_supports_single_extension_and_missing_alternates_only() {
    use jade_docx_layout::math_layout::font::OpenTypeMathFont;
    let f = OpenTypeMathFont::parse(FONT, 0).unwrap();
    for (level, glyph) in [(0, 1), (1, 2), (2, 3), (3, 3), (u32::MAX, 3)] {
        assert_eq!(f.script_glyph_id(1, level), glyph);
    }
    for level in [1, 2, 3] {
        assert_eq!(f.script_glyph_id(4, level), 5); // single substitution
        assert_eq!(f.script_glyph_id(6, level), 6); // salt is not ssty
    }
    assert_eq!(f.script_glyph_id(7, 1), 8);
    assert_eq!(f.script_glyph_id(7, 2), 7); // missing second alternate
    assert_eq!(f.script_glyph_id(u16::MAX, 1), u16::MAX);
    let no_gsub = OpenTypeMathFont::parse(include_bytes!("fixtures/math-font.ttf"), 0).unwrap();
    assert_eq!(no_gsub.script_glyph_id(1, 1), 1);
}

#[test]
fn glyph_selection_uses_nesting_even_when_root_and_script_sizes_are_equal() {
    let font = MathFontData::new("Script".into(), FONT.to_vec(), 3, 64).unwrap();
    let root_math = MathParagraph {
        elements: vec![text("x")],
        ..Default::default()
    };
    let root = font
        .layout(Fallback, &root_math, style(13.0), false)
        .unwrap();
    let nested_math = nested();
    let scripts = font
        .layout(Fallback, &nested_math, style(20.0), false)
        .unwrap();
    assert_eq!(
        root.glyphs[0].style.size_px,
        scripts.glyphs[2].style.size_px
    );
    assert_eq!(
        root.glyphs[0]
            .font_glyph
            .as_ref()
            .unwrap()
            .geometry
            .glyph_id,
        1
    );
    assert_eq!(
        scripts.glyphs[2]
            .font_glyph
            .as_ref()
            .unwrap()
            .geometry
            .glyph_id,
        2
    );
}

#[test]
fn invalid_alternate_glyph_id_keeps_the_original_without_partial_cache_entries() {
    use jade_docx_layout::math_layout::font::OpenTypeMathFont;
    let mut bytes = FONT.to_vec();
    let offset = {
        let face = ttf_parser::Face::parse(&bytes, 0).unwrap();
        let gsub = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"GSUB"))
            .unwrap();
        let start = gsub.as_ptr() as usize - bytes.as_ptr() as usize;
        start
            + gsub
                .windows(6)
                .position(|w| w == [0, 2, 0, 2, 0, 3])
                .unwrap()
    };
    bytes[offset + 4..offset + 6].copy_from_slice(&u16::MAX.to_be_bytes());
    let f = OpenTypeMathFont::parse(&bytes, 0).unwrap();
    assert_eq!(f.script_glyph_id(1, 1), 2);
    assert_eq!(f.script_glyph_id(1, 2), 1);
}

#[test]
fn existing_host_preparation_overrides_keep_their_policy() {
    use jade_docx_layout::math_layout::font::OpenTypeMathFont;
    use jade_docx_layout::math_layout::styled::{
        font::{OpenTypeMathMeasure, PreparedMathGlyph},
        layout_styled_math,
    };
    struct Legacy<'a>(OpenTypeMathMeasure<'a, Fallback>);
    impl StyledMathMeasure for Legacy<'_> {
        type Error = MathFontLayoutError<&'static str>;
        fn measure_glyph(
            &self,
            text: &str,
            style: MathTextStyle<'_>,
        ) -> Result<MathMetrics, Self::Error> {
            self.0.measure_glyph(text, style)
        }
        fn prepare_glyph(
            &self,
            text: &str,
            style: MathTextStyle<'_>,
            display: bool,
        ) -> Result<Option<PreparedMathGlyph>, Self::Error> {
            self.0.prepare_glyph(text, style, display)
        }
    }
    let face = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let legacy = Legacy(OpenTypeMathMeasure::new(&face, "Script", Fallback, 3, 64));
    let math = nested();
    let plan = layout_styled_math(&legacy, &math, style(20.0)).unwrap();
    assert!(plan
        .glyphs
        .iter()
        .all(|g| g.font_glyph.as_ref().unwrap().geometry.glyph_id == 1));
}

#[test]
fn display_style_keeps_nested_integrals_in_script_forms_without_display_stretch() {
    let font = MathFontData::new("Script".into(), FONT.to_vec(), 16, 64).unwrap();
    let mut chr = node(K::Unknown, vec![]);
    chr.tag = Some("chr".into());
    chr.attributes.insert("val".into(), "∫".into());
    let mut properties = node(K::Unknown, vec![chr]);
    properties.tag = Some("naryPr".into());
    let operator = node(K::Nary, vec![properties, node(K::Base, vec![text("x")])]);
    let math = MathParagraph {
        elements: vec![operator.clone(), sup(text("x"), operator)],
        ..Default::default()
    };
    let plan = font.layout(Fallback, &math, style(20.0), true).unwrap();
    let ids = plan
        .glyphs
        .iter()
        .filter(|g| &plan.text[g.text_range.clone()] == "∫")
        .map(|g| g.font_glyph.as_ref().unwrap().geometry.glyph_id)
        .collect::<Vec<_>>();
    assert_eq!(ids, [12, 10]);
}

#[test]
fn math_language_system_wins_over_an_earlier_latin_script_feature() {
    use jade_docx_layout::math_layout::font::OpenTypeMathFont;
    let f =
        OpenTypeMathFont::parse(include_bytes!("fixtures/math-script-scope-font.ttf"), 0).unwrap();
    assert_eq!(f.script_glyph_id(1, 1), 2);
    assert_eq!(f.script_glyph_id(1, 2), 3);
}
