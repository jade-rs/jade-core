use super::*;
use proptest::prelude::*;
use std::path::PathBuf;
use std::sync::OnceLock;

fn font_bytes() -> &'static [Vec<u8>; 2] {
    static BYTES: OnceLock<[Vec<u8>; 2]> = OnceLock::new();
    BYTES.get_or_init(|| {
        let fonts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fonts");
        [
            fs::read(fonts.join("Noto_Sans_JP/static/NotoSansJP-Regular.ttf")).unwrap(),
            fs::read(fonts.join("Noto_Sans_Thai/NotoSansThai-VariableFont_wdth,wght.ttf")).unwrap(),
        ]
    })
}

fn buffer(text: &str, language: Option<&str>, direction: Option<Direction>) -> UnicodeBuffer {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    if let Some(language) = language {
        buffer.set_language(language.parse().unwrap());
    }
    if let Some(direction) = direction {
        buffer.set_direction(direction);
    }
    buffer
}

// Independent uncached upstream oracle, preserving glyph IDs, clusters,
// break-safety flags and all four placement coordinates, not only total width.
fn facts(buffer: &GlyphBuffer) -> Vec<(u32, u32, bool, i32, i32, i32, i32)> {
    buffer
        .glyph_infos()
        .iter()
        .zip(buffer.glyph_positions())
        .map(|(info, position)| {
            (
                info.glyph_id,
                info.cluster,
                info.unsafe_to_break(),
                position.x_advance,
                position.y_advance,
                position.x_offset,
                position.y_offset,
            )
        })
        .collect()
}

#[test]
fn cache_keys_language_direction_script_features_and_kern_policy() {
    let face = Face::from_slice(&font_bytes()[0], 0).unwrap();
    let mut cache = ShapePlanCache::default();
    SHAPE_PLAN_BUILDS.with(|count| count.set(0));
    for _ in 0..2 {
        for (text, language, direction, kern_enabled, features) in [
            ("AV", None, None, true, vec![]),
            ("VA", Some("ja"), None, true, vec![]),
            ("AV", Some("en"), None, true, vec![]),
            ("AV", None, Some(Direction::RightToLeft), true, vec![]),
            ("AV", None, Some(Direction::TopToBottom), true, vec![]),
            ("あいう", None, None, true, vec![]),
            ("AV", None, None, false, vec![]),
            (
                "office",
                None,
                None,
                true,
                vec![
                    Feature::new(Tag::from_bytes(b"liga"), 0, ..),
                    Feature::new(Tag::from_bytes(b"liga"), 1, ..),
                ],
            ),
            (
                "office",
                None,
                None,
                true,
                vec![
                    Feature::new(Tag::from_bytes(b"liga"), 1, ..),
                    Feature::new(Tag::from_bytes(b"liga"), 0, ..),
                ],
            ),
            (
                "AV",
                None,
                None,
                true,
                vec![Feature::new(Tag::from_bytes(KERN), 0, ..)],
            ),
            (
                "AV",
                None,
                None,
                true,
                vec![Feature::new(Tag::from_bytes(KERN), 0, 1..2)],
            ),
        ] {
            let cached = cache.shape(
                &face,
                buffer(text, language, direction),
                kern_enabled,
                &features,
            );
            let mut reference_features = Vec::new();
            if !kern_enabled {
                reference_features.push(Feature::new(Tag::from_bytes(KERN), 0, ..));
            }
            reference_features.extend_from_slice(&features);
            let fresh = rustybuzz::shape(
                &face,
                &reference_features,
                buffer(text, language, direction),
            );
            assert_eq!(facts(&cached), facts(&fresh));
        }
    }
    assert_eq!(SHAPE_PLAN_BUILDS.with(|count| count.get()), 11);
}

#[test]
fn shape_plan_retention_is_bounded_and_evicted_keys_rebuild() {
    let face = Face::from_slice(&font_bytes()[0], 0).unwrap();
    let mut cache = ShapePlanCache::default();
    SHAPE_PLAN_BUILDS.with(|count| count.set(0));
    for value in 0..=MAX_SHAPE_PLANS_PER_FACE {
        let features = [Feature::new(Tag::from_bytes(b"ss01"), value as u32, ..)];
        cache.shape(&face, buffer("office", None, None), true, &features);
        assert!(cache.entries.len() <= MAX_SHAPE_PLANS_PER_FACE);
    }
    assert_eq!(cache.entries.len(), MAX_SHAPE_PLANS_PER_FACE);
    cache.shape(
        &face,
        buffer("office", None, None),
        true,
        &[Feature::new(Tag::from_bytes(b"ss01"), 0, ..)],
    );
    assert_eq!(
        SHAPE_PLAN_BUILDS.with(|count| count.get()),
        MAX_SHAPE_PLANS_PER_FACE + 2
    );
}

#[test]
fn registered_faces_own_separate_plans_and_registration_keeps_old_plans_valid() {
    let mut measurer = ShapingMeasurer::new(12.0);
    measurer
        .register_font_bytes("Test", 400, RegisteredFontStyle::Normal, &font_bytes()[0])
        .unwrap();
    measurer.measure_text("AV", "12pt Test").unwrap();
    measurer
        .register_font_bytes("Thai", 400, RegisteredFontStyle::Normal, &font_bytes()[1])
        .unwrap();
    measurer.measure_text("ภาษาไทย", "12pt Thai").unwrap();
    measurer.measure_text("VA", "18pt Test").unwrap();
    assert_eq!(measurer.faces[0].shape_plans.borrow().entries.len(), 1);
    assert_eq!(measurer.faces[1].shape_plans.borrow().entries.len(), 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn cached_sequences_match_uncached_shaping_after_every_step(
        operations in prop::collection::vec((0usize..2, 0usize..11, any::<bool>(),
            0u32..70, 0usize..5, 0usize..5, 0u8..16), 1..65)
    ) {
        let faces = font_bytes().each_ref().map(|bytes| Face::from_slice(bytes, 0).unwrap());
        let mut caches = [ShapePlanCache::default(), ShapePlanCache::default()];
        let texts = ["", "AVATAR office ffi", "あいうえて", "漢字「、」", "ภาษาไทย", "مرحبا",
            "אבג", "a\u{301}", "👨\u{200d}👨\u{200d}👧\u{200d}👦", "123 !", "禰\u{e0100}"];
        let languages = [None, Some("ja"), Some("en"), Some("tr"), Some("th")];
        let directions = [None, Some(Direction::LeftToRight), Some(Direction::RightToLeft),
            Some(Direction::TopToBottom), Some(Direction::BottomToTop)];
        for (step, (font, text, kern, value, language, direction, reset)) in operations.into_iter().enumerate() {
            if reset == 0 {
                caches[font] = ShapePlanCache::default();
            }
            let features = [
                Feature::new(Tag::from_bytes(b"liga"), value % 2, ..),
                Feature::new(Tag::from_bytes(KERN), (value / 2) % 2, (value as usize % 4)..),
                Feature::new(Tag::from_bytes(b"ss01"), value, 1..3),
            ];
            let cached = caches[font].shape(&faces[font], buffer(texts[text], languages[language],
                directions[direction]), kern, &features);
            let mut reference_features = Vec::new();
            if !kern {
                reference_features.push(Feature::new(Tag::from_bytes(KERN), 0, ..));
            }
            reference_features.extend_from_slice(&features);
            let fresh = rustybuzz::shape(&faces[font], &reference_features,
                buffer(texts[text], languages[language], directions[direction]));
            prop_assert_eq!(facts(&cached), facts(&fresh), "step {} font {} text {} language {} direction {}",
                step, font, text, language, direction);
            prop_assert!(caches[font].entries.len() <= MAX_SHAPE_PLANS_PER_FACE);
        }
    }
}
