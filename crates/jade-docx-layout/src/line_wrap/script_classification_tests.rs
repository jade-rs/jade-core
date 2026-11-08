use super::{classify_script, classify_script_slow, ScriptCategory};
use proptest::prelude::*;

// Preserve the scalar-decoding classifier that preceded the byte-prefix fast
// path. In particular, an ASCII prefix wins even when later scalars have a
// different script, and the complete kana blocks override Unicode Script.
fn previous_scalar_classifier(text: &str) -> ScriptCategory {
    match text.chars().next() {
        Some(ch) if ch.is_ascii() => ScriptCategory::Latin,
        Some('\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}') => {
            ScriptCategory::EastAsia
        }
        _ => classify_script_slow(text),
    }
}

#[test]
fn script_classification_matches_previous_classifier_for_every_unicode_scalar() {
    let mut text = String::with_capacity(16);
    for value in 0..=0x10ffff {
        let Some(ch) = char::from_u32(value) else {
            continue;
        };
        for suffix in ["", "A", "ا", "日", "\u{301}"] {
            text.clear();
            text.push(ch);
            text.push_str(suffix);
            assert_eq!(
                classify_script(&text),
                previous_scalar_classifier(&text),
                "U+{value:04X}, suffix={suffix:?}"
            );
        }
    }
}

#[test]
fn script_classification_preserves_prefix_precedence_and_symbol_gaps() {
    for (text, expected) in [
        ("", ScriptCategory::Latin),
        ("aا", ScriptCategory::Latin),
        ("\0日", ScriptCategory::Latin),
        ("ا日", ScriptCategory::Complex),
        ("日ا", ScriptCategory::EastAsia),
        ("。ا", ScriptCategory::EastAsia),
        ("ーا", ScriptCategory::EastAsia),
        ("\u{301}ا", ScriptCategory::Complex),
        ("\u{4dc0}ا", ScriptCategory::Complex),
        ("\u{4dff}ا", ScriptCategory::Complex),
        ("\u{4dc0}日", ScriptCategory::EastAsia),
    ] {
        assert_eq!(classify_script(text), expected, "{text:?}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        rng_seed: proptest::test_runner::RngSeed::Fixed(20261109),
        ..ProptestConfig::default()
    })]

    #[test]
    fn script_classification_matches_previous_classifier_for_mixed_scalars(
        chars in prop::collection::vec(any::<char>(), 0..65)
    ) {
        let text: String = chars.into_iter().collect();
        prop_assert_eq!(classify_script(&text), previous_scalar_classifier(&text), "{:?}", text);
    }
}
