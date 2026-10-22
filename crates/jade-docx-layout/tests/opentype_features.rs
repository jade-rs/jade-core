#![cfg(feature = "native-shaping")]
//! Word 2010 typography (`w14:ligatures`, `w14:numForm`, …) reaches the
//! shaper through the font descriptor.

use jade_docx_layout::font_measurer::RegisteredFontStyle;
use jade_docx_layout::layout_cache::TextMeasurer;
use jade_docx_layout::shaping_measurer::ShapingMeasurer;
use std::path::PathBuf;

fn measurer() -> ShapingMeasurer {
    let font = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let mut measurer = ShapingMeasurer::new(12.0);
    measurer
        .register_font_path("Noto Sans JP", 400, RegisteredFontStyle::Normal, font)
        .unwrap();
    measurer
}

#[test]
fn ligature_features_reach_the_shaper() {
    let m = measurer();
    // Noto Sans JP's `liga` forms an `ff` ligature narrower than two f's.
    let joined = m
        .measure_text("ff", "ot:liga 100pt 'Noto Sans JP'")
        .unwrap();
    let apart = m
        .measure_text("ff", "ot:-liga 100pt 'Noto Sans JP'")
        .unwrap();
    assert!(joined < apart - 0.5, "joined {joined} apart {apart}");
    // A single letter has nothing to join.
    let f_joined = m.measure_text("f", "ot:liga 100pt 'Noto Sans JP'").unwrap();
    let f_apart = m
        .measure_text("f", "ot:-liga 100pt 'Noto Sans JP'")
        .unwrap();
    assert_eq!(f_joined, f_apart);
}

#[test]
fn word_typography_becomes_a_descriptor_token() {
    use jade_docx_layout::font_resolver::{opentype_features_token, resolve_run_style_for_wrap};
    use jade_docx_reader::{RunProperties, RunTextEffects};
    let mut props = RunProperties::default();
    assert_eq!(opentype_features_token(&props, false), None);
    props.text_effects = Some(Box::new(RunTextEffects {
        ligatures: Some("standardContextual".into()),
        number_form: Some("oldStyle".into()),
        number_spacing: Some("proportional".into()),
        stylistic_set: Some(3),
        ..Default::default()
    }));
    assert_eq!(
        opentype_features_token(&props, false).as_deref(),
        Some("ot:liga,clig,onum,pnum,ss03,-calt,-kern")
    );
    let style = resolve_run_style_for_wrap(Some(&props), None);
    assert!(style
        .font_descriptor_latin_ascii
        .starts_with("ot:liga,clig"));
    assert!(style.joins_characters);
    assert!(style.measures_letter_pairs());

    // Figures only: no letters join, so pairs are not measured for it.
    props.text_effects = Some(Box::new(RunTextEffects {
        number_form: Some("lining".into()),
        ..Default::default()
    }));
    let style = resolve_run_style_for_wrap(Some(&props), None);
    assert!(style
        .font_descriptor_latin_ascii
        .starts_with("ot:lnum,-liga,-clig,-calt,-kern "));
    assert!(!style.joins_characters);
}
