//! Shared run-flag encoding and Windows style-cache keys.
use jade_docx_reader::document::{FontHint, RunProperties};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

#[inline]
pub(super) fn boolean_flags(props: &RunProperties) -> u64 {
    // Two bits preserve None, explicit false and explicit true. They must
    // remain distinct while direct formatting is resolved through styles.
    // Fixed shifts avoid constructing and iterating a temporary flag array.
    macro_rules! flag {
        ($field:ident, $index:expr) => {
            (match props.$field {
                None => 0_u64,
                Some(false) => 1,
                Some(true) => 2,
            } << ($index * 2))
        };
    }
    flag!(bold, 0)
        | flag!(italic, 1)
        | flag!(bold_complex, 2)
        | flag!(italic_complex, 3)
        | flag!(underline, 4)
        | flag!(strike, 5)
        | flag!(double_strike, 6)
        | flag!(caps, 7)
        | flag!(small_caps, 8)
        | flag!(vanish, 9)
        | flag!(spec_vanish, 10)
        | flag!(shadow, 11)
        | flag!(outline, 12)
        | flag!(emboss, 13)
        | flag!(imprint, 14)
}

pub(super) fn windows_key(properties: Option<&RunProperties>, grid: Option<f64>) -> u64 {
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
        if let Some(ruby) = props.ruby.as_ref() {
            present |= 1 << 6;
            ruby.text.hash(&mut hasher);
            ruby.size_pt.map(f32::to_bits).hash(&mut hasher);
            ruby.align.hash(&mut hasher);
            ruby.raise_half_points.hash(&mut hasher);
            ruby.fonts
                .as_ref()
                .map(|fonts| (&fonts.ascii, &fonts.east_asia))
                .hash(&mut hasher);
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

#[cfg(test)]
mod tests {
    use super::*;
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
