//! Windows style-cache keys: hash present values and one presence mask.
use jade_docx_reader::document::{FontHint, RunProperties};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

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
        // Two bits preserve None, explicit false and explicit true. They must
        // remain distinct while direct formatting is resolved through styles.
        let flags = [
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
        });
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

#[cfg(test)]
mod tests {
    use super::*;
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
