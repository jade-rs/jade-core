//! Shared run-flag encoding and Windows style-cache keys.
use jade_docx_reader::document::{FontHint, RunProperties};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

#[inline]
pub(super) fn boolean_flags(props: &RunProperties) -> u64 {
    // Two bits preserve None, explicit false and explicit true. They must
    // remain distinct while direct formatting is resolved through styles.
    [
        props.bold,
        props.italic,
        props.bold_complex,
        props.italic_complex,
        props.underline,
        props.strike,
        props.double_strike,
        props.caps,
        props.small_caps,
        props.vanish,
        props.spec_vanish,
        props.shadow,
        props.outline,
        props.emboss,
        props.imprint,
        props.snap_to_grid,
    ]
    .into_iter()
    .enumerate()
    .fold(0_u64, |bits, (index, flag)| {
        bits | (match flag {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        } << (index * 2))
    })
}

pub(super) fn windows_key(properties: Option<&RunProperties>, grid: Option<f64>) -> u64 {
    windows_key_impl::<true>(properties, grid)
}

/// The effective-properties cache checks full structural equality within each
/// bucket. Its bucket hash need not repeat the language work in the style key.
pub(super) fn windows_key_impl<const INCLUDE_LANGUAGES: bool>(
    properties: Option<&RunProperties>,
    grid: Option<f64>,
) -> u64 {
    let mut hasher = FxHasher::default();
    let mut present = 0_u64;
    macro_rules! optional {
        ($bit:expr, $value:expr) => {
            if let Some(value) = $value {
                present |= 1 << $bit;
                value.hash(&mut hasher);
            }
        };
    }
    optional!(0, grid.map(f64::to_bits));
    if let Some(props) = properties {
        1_u8.hash(&mut hasher);
        let flags = boolean_flags(props);
        flags.hash(&mut hasher);
        optional!(1, props.style.as_ref());
        optional!(2, props.underline_kind.as_ref());
        optional!(3, props.underline_color.as_ref());
        optional!(
            4,
            props
                .text_effects
                .as_ref()
                .map(|effects| format!("{effects:?}"))
        );
        optional!(5, props.emphasis.as_ref());
        optional!(19, props.east_asian_layout.as_ref());
        // Language selects fallback faces and their metrics even when all
        // authored font slots are unchanged. Keep both optional fields distinct.
        if INCLUDE_LANGUAGES {
            optional!(20, props.language.as_ref());
            optional!(21, props.east_asia_language.as_ref());
        }
        if let Some(ruby) = props.ruby.as_ref() {
            present |= 1 << 6;
            ruby.text.hash(&mut hasher);
            ruby.size_pt.map(f32::to_bits).hash(&mut hasher);
            ruby.align.hash(&mut hasher);
            ruby.raise_half_points.hash(&mut hasher);
            ruby.fonts.hash(&mut hasher);
            format!("{:?}", ruby.guide_runs).hash(&mut hasher);
            ruby.continuation.hash(&mut hasher);
        }
        if let Some(border) = props.border.as_ref() {
            present |= 1 << 7;
            border.val.hash(&mut hasher);
            border.size_eighths.hash(&mut hasher);
            border.color.hash(&mut hasher);
        }
        optional!(8, props.baseline.as_ref());
        optional!(9, props.text_position.map(f32::to_bits));
        optional!(10, props.char_spacing.map(f32::to_bits));
        optional!(11, props.kerning.map(f32::to_bits));
        optional!(12, props.text_scale.map(f32::to_bits));
        optional!(13, props.fit_text.as_ref().map(|fit| (fit.val, fit.id)));
        optional!(14, props.font_size.map(f32::to_bits));
        optional!(15, props.font_size_complex.map(f32::to_bits));
        optional!(16, props.font_family.as_ref());
        optional!(17, props.color.as_ref());
        if let Some(fonts) = props.fonts.as_ref() {
            present |= 1 << 18;
            fonts.ascii.hash(&mut hasher);
            fonts.h_ansi.hash(&mut hasher);
            fonts.east_asia.hash(&mut hasher);
            fonts.complex.hash(&mut hasher);
            let hint = match fonts.hint.as_ref() {
                Some(FontHint::EastAsia) => 1_u8,
                Some(FontHint::Complex) => 2,
                Some(FontHint::Default) => 3,
                None => 0,
            };
            hint.hash(&mut hasher);
        }
    } else {
        0_u8.hash(&mut hasher);
    }
    present.hash(&mut hasher);
    hasher.finish()
}

// Keep optional language hashing out of the large repaint loops. Passing the
// hasher by value also avoids making the hot loop's accumulator escape.
#[cold]
#[inline(never)]
pub(super) fn hash_windows_languages(props: &RunProperties, mut hasher: FxHasher) -> FxHasher {
    if jade_docx_layout::line_metrics::word_layout_platform()
        == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
    {
        hash_language(props.language.as_deref(), &mut hasher);
        hash_language(props.east_asia_language.as_deref(), &mut hasher);
    }
    hasher
}

#[inline]
fn hash_language(language: Option<&str>, hasher: &mut FxHasher) {
    let Some(language) = language else {
        0_u64.hash(hasher);
        return;
    };
    let bytes = language.as_bytes();
    if bytes.len() == 5 {
        // Common BCP47 tags (ja-JP, en-US, ko-KR) fit in one word. Include
        // their byte length, so absent, empty and embedded NULs stay distinct.
        let lower = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        (u64::from(lower) | (u64::from(bytes[4]) << 32) | (6_u64 << 56)).hash(hasher);
    } else if bytes.len() <= 7 {
        let mut packed = [0_u8; 8];
        packed[..bytes.len()].copy_from_slice(bytes);
        packed[7] = bytes.len() as u8 + 1;
        u64::from_le_bytes(packed).hash(hasher);
    } else {
        // Long/private-use and arbitrary UTF-8 tags retain every byte.
        9_u64.hash(hasher);
        language.hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_hash_keeps_presence_lengths_fields_and_long_suffixes_distinct() {
        use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let previous = jade_docx_layout::line_metrics::word_layout_platform();
        let tags = [
            None,
            Some(""),
            Some("a"),
            Some("a\0"),
            Some("a\0\0"),
            Some("ja"),
            Some("ja-JP"),
            Some("ja-jp"),
            Some("zh-Hant"),
            Some("ko-KR"),
            Some("en-US-x-first"),
            Some("en-US-x-second"),
            Some("日本"),
            Some("日本語"),
        ];
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let mut keys = std::collections::HashSet::new();
        for language in tags {
            for east_asia_language in tags {
                let properties = RunProperties {
                    language: language.map(Into::into),
                    east_asia_language: east_asia_language.map(Into::into),
                    ..Default::default()
                };
                let mut seed = FxHasher::default();
                0x1234_u64.hash(&mut seed);
                let key = hash_windows_languages(&properties, seed).finish();
                assert!(keys.insert(key), "{language:?}/{east_asia_language:?}");
            }
        }
        set_word_layout_platform(WordLayoutPlatform::Mac);
        for tag in tags {
            let properties = RunProperties {
                language: tag.map(Into::into),
                ..Default::default()
            };
            let mut seed = FxHasher::default();
            0x1234_u64.hash(&mut seed);
            let expected = seed.finish();
            assert_eq!(hash_windows_languages(&properties, seed).finish(), expected);
        }
        set_word_layout_platform(previous);
    }

    #[test]
    fn windows_language_values_and_field_positions_remain_distinct() {
        let languages = [
            None,
            Some(""),
            Some("ja-JP"),
            Some("zh-TW"),
            Some("ko-KR"),
            Some("en-US"),
        ];
        let mut keys = std::collections::HashSet::new();
        for language in languages {
            for east_asia_language in languages {
                let properties = RunProperties {
                    language: language.map(Into::into),
                    east_asia_language: east_asia_language.map(Into::into),
                    ..Default::default()
                };
                assert!(
                    keys.insert(windows_key(Some(&properties), None)),
                    "{properties:?}"
                );
            }
        }
    }
    // The reference uses base-four place values of independently generated
    // ternary digits. Keep the exact key representation, not only uniqueness.
    fn check_boolean_encoding(digits: [u8; 15]) {
        let setters: [fn(&mut RunProperties, Option<bool>); 15] = [
            |p, v| p.bold = v,
            |p, v| p.italic = v,
            |p, v| p.bold_complex = v,
            |p, v| p.italic_complex = v,
            |p, v| p.underline = v,
            |p, v| p.strike = v,
            |p, v| p.double_strike = v,
            |p, v| p.caps = v,
            |p, v| p.small_caps = v,
            |p, v| p.vanish = v,
            |p, v| p.spec_vanish = v,
            |p, v| p.shadow = v,
            |p, v| p.outline = v,
            |p, v| p.emboss = v,
            |p, v| p.imprint = v,
        ];
        let mut props = RunProperties::default();
        let mut expected = 0_u64;
        let mut place = 1_u64;
        for (setter, digit) in setters.into_iter().zip(digits) {
            setter(
                &mut props,
                [None, Some(false), Some(true)][usize::from(digit)],
            );
            expected += u64::from(digit) * place;
            place *= 4;
        }
        assert_eq!(boolean_flags(&props), expected, "digits={digits:?}");
    }

    #[test]
    fn boolean_encoding_preserves_all_single_and_paired_field_states() {
        for digit in 0..3 {
            check_boolean_encoding([digit; 15]);
        }
        for left in 0..15 {
            for right in left..15 {
                for a in 0..3 {
                    for b in 0..3 {
                        let mut digits = [0; 15];
                        digits[left] = a;
                        digits[right] = b;
                        check_boolean_encoding(digits);
                    }
                }
            }
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(256))]
        #[test]
        fn boolean_encoding_matches_ternary_field_states(
            digits in proptest::array::uniform15(0_u8..3),
        ) {
            check_boolean_encoding(digits);
        }
    }

    #[test]
    fn explicit_boolean_values_and_field_positions_remain_distinct() {
        let setters: [fn(&mut RunProperties, Option<bool>); 16] = [
            |p, v| p.bold = v,
            |p, v| p.italic = v,
            |p, v| p.bold_complex = v,
            |p, v| p.italic_complex = v,
            |p, v| p.underline = v,
            |p, v| p.strike = v,
            |p, v| p.double_strike = v,
            |p, v| p.caps = v,
            |p, v| p.small_caps = v,
            |p, v| p.vanish = v,
            |p, v| p.spec_vanish = v,
            |p, v| p.shadow = v,
            |p, v| p.outline = v,
            |p, v| p.emboss = v,
            |p, v| p.imprint = v,
            |p, v| p.snap_to_grid = v,
        ];
        let mut keys = std::collections::HashSet::new();
        assert!(keys.insert(windows_key(None, None)));
        assert!(keys.insert(windows_key(Some(&RunProperties::default()), None)));
        for setter in setters {
            for flag in [false, true] {
                let mut props = RunProperties::default();
                setter(&mut props, Some(flag));
                assert!(keys.insert(windows_key(Some(&props), None)));
            }
        }
    }
    #[test]
    fn absent_fields_do_not_alias_zero_or_another_fields_value() {
        let mut keys = std::collections::HashSet::new();
        let empty = RunProperties::default();
        assert!(keys.insert(windows_key(Some(&empty), None)));
        assert!(keys.insert(windows_key(Some(&empty), Some(0.0))));
        assert!(keys.insert(windows_key(Some(&empty), Some(-0.0))));
        for field in 0..9 {
            let mut props = RunProperties::default();
            match field {
                0 => props.font_size = Some(0.0),
                1 => props.font_size_complex = Some(0.0),
                2 => props.text_position = Some(0.0),
                3 => props.char_spacing = Some(0.0),
                4 => props.kerning = Some(0.0),
                5 => props.text_scale = Some(0.0),
                6 => props.style = Some("".into()),
                7 => props.font_family = Some("".into()),
                _ => props.color = Some("".into()),
            }
            assert!(
                keys.insert(windows_key(Some(&props), None)),
                "field {field}"
            );
        }
    }
}
