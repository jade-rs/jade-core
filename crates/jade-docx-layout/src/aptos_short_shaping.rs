//! Font-unit fallback for Aptos 2.01 PDF subsets without layout tables.
//!
//! Word PDF subsets preserve used outlines and advances but drop GPOS/GSUB.
//! These short-run metrics come from the corresponding full font programs,
//! independently of document coordinates. Other versions and features retain
//! the normal shaper, and Mac never selects this fallback.
use crate::font_measurer::RegisteredFontStyle;
use rustybuzz::{ttf_parser::Tag, Face, Feature};

#[path = "aptos_short_shaping_data.rs"]
mod data;

pub(super) fn subset_face(
    face: &Face<'_>,
    family: &str,
    weight: u16,
    style: RegisteredFontStyle,
) -> Option<usize> {
    let family_offset = if family.eq_ignore_ascii_case("Aptos") {
        0
    } else if family.eq_ignore_ascii_case("Aptos Display") {
        4
    } else {
        return None;
    };
    if !matches!(weight, 400 | 700)
        || face.weight().to_number() != weight
        || face.is_italic() != (style == RegisteredFontStyle::Italic)
        || face.units_per_em() != 2048
        || face.raw_face().table(Tag::from_bytes(b"GPOS")).is_some()
        || !face.names().into_iter().any(|name| {
            name.name_id == 5
                && name.to_string().is_some_and(|value| {
                    value == "Version 2.01" || value.starts_with("Version 2.01;")
                })
        })
    {
        return None;
    }
    Some(
        family_offset
            + 2 * usize::from(weight >= 600)
            + usize::from(style == RegisteredFontStyle::Italic),
    )
}

pub(super) fn advance_units(
    face: usize,
    text: &str,
    mut kern: bool,
    features: &[Feature],
) -> Option<i32> {
    if crate::line_metrics::word_layout_platform()
        != crate::line_metrics::WordLayoutPlatform::Windows
        || !(1..=2).contains(&text.len())
        || !text.bytes().all(|c| (b' '..=b'~').contains(&c))
    {
        return None;
    }
    let mut proportional = false;
    let mut tabular = false;
    let mut ligatures = true;
    for feature in features {
        if feature.start != 0 || feature.end != u32::MAX {
            return None;
        }
        match feature.tag.to_bytes() {
            [b'k', b'e', b'r', b'n'] => kern = feature.value != 0,
            [b'l', b'i', b'g', b'a'] => ligatures = feature.value != 0,
            [b'p', b'n', b'u', b'm'] => proportional = feature.value != 0,
            [b't', b'n', b'u', b'm'] => tabular = feature.value != 0,
            // Aptos 2.01 has lining default digits and no clig/calt feature.
            [b'l', b'n', b'u', b'm'] | [b'c', b'l', b'i', b'g'] | [b'c', b'a', b'l', b't'] => {}
            _ if feature.value == 0 => {}
            _ => return None,
        }
    }
    // These fonts apply tnum after pnum when both are enabled.
    let form = if tabular {
        2u32
    } else {
        u32::from(proportional)
    };
    let single = |ascii: u8| -> i32 {
        i32::from(if form == 1 && ascii.is_ascii_digit() {
            data::PROPORTIONAL_DIGITS[face][usize::from(ascii - b'0')]
        } else {
            data::ASCII[face][usize::from(ascii - b' ')]
        })
    };
    let bytes = text.as_bytes();
    let mut units = single(bytes[0]);
    if bytes.len() == 2 {
        units += single(bytes[1]);
        let state = u32::from(kern) | (u32::from(ligatures) << 1);
        let key = (form << 24) | (state << 16) | (u32::from(bytes[0]) << 8) | u32::from(bytes[1]);
        if let Ok(index) = data::PAIRS.binary_search_by_key(&key, |entry| entry.0) {
            units += i32::from(data::PAIRS[index].1[face]);
        }
    }
    Some(units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line_metrics::{set_word_layout_platform, word_layout_platform, WordLayoutPlatform};

    #[test]
    fn font_units_respect_kerning_numeric_forms_and_feature_overrides() {
        let previous = word_layout_platform();
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let feature = |tag: &[u8; 4], value| Feature::new(Tag::from_bytes(tag), value, ..);
        assert!(data::PAIRS.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for face in 0..8 {
            let digit = advance_units(face, "1", true, &[]).unwrap();
            let proportional = advance_units(face, "1", true, &[feature(b"pnum", 1)]).unwrap();
            assert!(proportional < digit);
            assert_eq!(
                advance_units(face, "1", true, &[feature(b"tnum", 1)]),
                Some(digit)
            );
            assert_eq!(
                advance_units(face, "1", true, &[feature(b"pnum", 1), feature(b"pnum", 0)]),
                Some(digit)
            );
            assert_eq!(
                advance_units(face, "1", true, &[feature(b"tnum", 1), feature(b"pnum", 1)]),
                Some(digit)
            );
            let no_kern = advance_units(face, "AV", false, &[]).unwrap();
            assert!(advance_units(face, "AV", true, &[]).unwrap() < no_kern);
            assert_eq!(
                advance_units(face, "AV", true, &[feature(b"kern", 0)]),
                Some(no_kern)
            );
        }
        set_word_layout_platform(previous);
    }

    #[test]
    fn mac_unknown_features_and_long_or_non_ascii_runs_keep_the_existing_shaper() {
        let previous = word_layout_platform();
        set_word_layout_platform(WordLayoutPlatform::Windows);
        for text in ["", "AVX", "日", "\n"] {
            assert_eq!(advance_units(0, text, true, &[]), None);
        }
        assert_eq!(
            advance_units(
                0,
                "12",
                true,
                &[Feature::new(Tag::from_bytes(b"onum"), 1, ..)]
            ),
            None
        );
        assert_eq!(
            advance_units(
                0,
                "12",
                true,
                &[Feature::new(Tag::from_bytes(b"kern"), 1, 1..)]
            ),
            None
        );
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert_eq!(advance_units(0, "AV", true, &[]), None);
        set_word_layout_platform(previous);
    }
}
