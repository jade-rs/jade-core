#![cfg(feature = "opentype-math")]

use jade_docx_layout::math_layout::font::{
    MathFontError, MathGlyphBounds, MathGlyphVariant, MathOutlineError, MathStretchAxis,
    MathStretchPlan, OpenTypeMathFont,
};

const FONT: &[u8] = include_bytes!("fixtures/math-font.ttf");

fn font() -> OpenTypeMathFont<'static> {
    OpenTypeMathFont::parse(FONT, 0).unwrap()
}

fn math_offset(bytes: &[u8]) -> usize {
    let tables = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    (0..tables)
        .map(|n| 12 + 16 * n)
        .find(|&p| &bytes[p..p + 4] == b"MATH")
        .map(|p| u32::from_be_bytes(bytes[p + 8..p + 12].try_into().unwrap()) as usize)
        .unwrap()
}

#[test]
fn constants_and_script_sizes_come_from_the_chosen_font() {
    let f = font();
    let c = f.constants();
    assert_eq!(f.units_per_em(), 1000);
    assert_eq!(c.script_scale(0), Some(1.0));
    assert_eq!(c.script_scale(1), Some(0.65));
    assert_eq!(c.script_scale(2), Some(0.4));
    assert_eq!(c.script_scale(3), Some(0.4));
    assert_eq!(c.display_operator_min_height, 1800);
    assert_eq!(c.delimited_min_height, 1500);
    assert_eq!(c.leading, 110);
    assert_eq!(c.axis_height, 250);
    assert_eq!(c.superscript_shift_up, 360);
    assert_eq!(c.subscript_shift_down, 210);
    assert_eq!(c.subscript_top_max, 0);
    assert_eq!(c.superscript_bottom_min, 0);
    assert_eq!(c.sub_superscript_gap_min, 0);
    assert_eq!(c.superscript_bottom_max_with_subscript, 0);
    assert_eq!(c.superscript_baseline_drop_max, 200);
    assert_eq!(c.subscript_baseline_drop_min, 100);
    assert_eq!(c.upper_limit_gap_min, 135);
    assert_eq!(c.upper_limit_baseline_rise_min, 300);
    assert_eq!(c.lower_limit_gap_min, 135);
    assert_eq!(c.lower_limit_baseline_drop_min, 670);
    assert_eq!(c.space_after_script, 40);
    let mut invalid = c;
    invalid.script_percent = 0;
    invalid.script_script_percent = 101;
    assert_eq!(invalid.script_scale(1), None);
    assert_eq!(invalid.script_scale(2), None);
}

#[test]
fn display_variant_is_selected_by_extent_without_a_unicode_mapping() {
    let f = font();
    assert_eq!(f.glyph_id('∫'), Some(2));
    assert_eq!(f.glyph_id('x'), Some(1));
    assert_eq!(f.glyph_id('∮'), None);
    for (target, id, extent) in [
        (1000, 2, 1000),
        (1001, 3, 2200),
        (1800, 3, 2200),
        (2200, 3, 2200),
    ] {
        let Some(MathStretchPlan::Glyph(v)) = f.stretch_plan(2, MathStretchAxis::Vertical, target)
        else {
            panic!("missing adequate prepared variant for {target}");
        };
        assert_eq!(
            v,
            MathGlyphVariant {
                glyph_id: id,
                extension_units: extent
            }
        );
    }
    assert_eq!(f.italic_correction(2), 50);
    assert_eq!(f.italic_correction(3), 70);
    assert_eq!(f.italic_correction(1), 0);
    assert!(!f.is_extended_shape(2));
    assert!(f.is_extended_shape(3));
    assert!(f
        .stretch_plan(2, MathStretchAxis::Horizontal, 1800)
        .is_none());
    assert!(f.stretch_plan(1, MathStretchAxis::Vertical, 1800).is_none());
}

#[test]
fn target_beyond_prepared_variants_returns_the_assembly_recipe() {
    let f = font();
    let Some(MathStretchPlan::Assembly(a)) = f.stretch_plan(2, MathStretchAxis::Vertical, 2201)
    else {
        panic!("largest prepared glyph cannot satisfy this target");
    };
    assert_eq!(a.min_connector_overlap, 20);
    assert_eq!(a.italic_correction, 70);
    let parts: Vec<_> = a.parts().collect();
    assert_eq!(
        parts.iter().map(|p| p.glyph_id).collect::<Vec<_>>(),
        [2, 4, 3]
    );
    assert_eq!(
        parts.iter().map(|p| p.extender).collect::<Vec<_>>(),
        [false, true, false]
    );
    assert_eq!(
        (
            parts[1].start_connector,
            parts[1].end_connector,
            parts[1].full_advance
        ),
        (50, 50, 400)
    );
}

#[test]
fn ink_bounds_and_horizontal_advance_are_separate_from_extension() {
    let f = font();
    let g = f.glyph_geometry(3, 10.0).unwrap();
    assert_eq!(g.advance_px, 5.5);
    assert_eq!(
        g.ink_px,
        MathGlyphBounds {
            x_min: -0.4,
            y_min: -4.0,
            x_max: 3.2,
            y_max: 18.0
        }
    );
    for size in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        assert!(f.glyph_geometry(3, size).is_none());
    }
    assert!(f.glyph_geometry(u16::MAX, 10.0).is_none());
    assert_eq!(f.glyph_advance_units(3), Some(550));
    assert_eq!(f.glyph_advance_units(u16::MAX), None);
}

#[test]
fn variant_outline_uses_the_same_glyph_and_never_returns_a_partial_path() {
    let f = font();
    let outline = f.glyph_outline(3, 64).unwrap().unwrap();
    assert_eq!(
        outline.bounds,
        MathGlyphBounds {
            x_min: -40.0,
            y_min: -400.0,
            x_max: 320.0,
            y_max: 1800.0
        }
    );
    assert!(!outline.commands.is_empty());
    assert_eq!(
        f.glyph_outline(3, outline.commands.len()).unwrap(),
        Some(outline.clone())
    );
    assert_eq!(
        f.glyph_outline(3, outline.commands.len() - 1),
        Err(MathOutlineError::CommandBudgetExceeded)
    );
    assert_eq!(
        f.glyph_outline(3, 0),
        Err(MathOutlineError::CommandBudgetExceeded)
    );
    assert_eq!(f.glyph_outline(0, 0).unwrap(), None);
    assert_eq!(f.glyph_outline(u16::MAX, 64).unwrap(), None);
}

#[test]
fn wrong_face_and_missing_math_do_not_substitute_another_font() {
    assert!(matches!(
        OpenTypeMathFont::parse(FONT, 1),
        Err(MathFontError::InvalidFont)
    ));
    let mut bytes = FONT.to_vec();
    let p = bytes.windows(4).position(|v| v == b"MATH").unwrap();
    bytes[p..p + 4].copy_from_slice(b"TEST");
    assert!(matches!(
        OpenTypeMathFont::parse(&bytes, 0),
        Err(MathFontError::MissingMathTable)
    ));
}

#[test]
fn malformed_math_version_and_constants_cannot_become_zero_metrics() {
    let p = math_offset(FONT);
    let mut bytes = FONT.to_vec();
    bytes[p..p + 4].copy_from_slice(&[1, 1, 0, 0]);
    assert!(matches!(
        OpenTypeMathFont::parse(&bytes, 0),
        Err(MathFontError::UnsupportedMathVersion)
    ));
    for offset in [0_u16, 9, u16::MAX] {
        let mut bytes = FONT.to_vec();
        bytes[p + 4..p + 6].copy_from_slice(&offset.to_be_bytes());
        assert!(matches!(
            OpenTypeMathFont::parse(&bytes, 0),
            Err(MathFontError::IncompleteConstants)
        ));
    }
}

#[test]
fn truncated_font_data_does_not_panic() {
    for n in 0..FONT.len() {
        let _ = OpenTypeMathFont::parse(&FONT[..n], 0);
    }
}

#[test]
fn variable_table_requires_an_explicit_instance_policy() {
    let mut bytes = FONT.to_vec();
    let p = bytes.windows(4).position(|v| v == b"name").unwrap();
    bytes[p..p + 4].copy_from_slice(b"fvar");
    // The SFNT directory must stay tag-sorted for table lookup.
    let n = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let mut records: Vec<_> = bytes[12..12 + n * 16]
        .chunks_exact(16)
        .map(|r| r.to_vec())
        .collect();
    records.sort_by(|a, b| a[..4].cmp(&b[..4]));
    for (destination, record) in bytes[12..12 + n * 16].chunks_exact_mut(16).zip(records) {
        destination.copy_from_slice(&record);
    }
    assert!(matches!(
        OpenTypeMathFont::parse(&bytes, 0),
        Err(MathFontError::VariableFont)
    ));
}

#[test]
fn math_font_line_box_prefers_os2_typographic_metrics_and_falls_back_to_hhea() {
    let mut bytes = FONT.to_vec();
    let count = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let hhea = (0..count)
        .map(|i| 12 + 16 * i)
        .find(|&i| &bytes[i..i + 4] == b"hhea")
        .unwrap();
    let start = u32::from_be_bytes(bytes[hhea + 8..hhea + 12].try_into().unwrap()) as usize;
    for (offset, value) in [(4, 900i16), (6, -500), (8, 300)] {
        bytes[start + offset..start + offset + 2].copy_from_slice(&value.to_be_bytes());
    }
    let font = OpenTypeMathFont::parse(&bytes, 0).unwrap();
    let typographic = font.line_metrics(10.0).unwrap();
    assert_eq!((typographic.ascent, typographic.descent), (8.0, 4.0));
    assert!(font.line_metrics(0.0).is_none());
    let os2 = (0..count)
        .map(|i| 12 + 16 * i)
        .find(|&i| &bytes[i..i + 4] == b"OS/2")
        .unwrap();
    bytes[os2..os2 + 4].copy_from_slice(b"NO02");
    let mut directory: Vec<_> = bytes[12..12 + count * 16]
        .chunks_exact(16)
        .map(|r| r.to_vec())
        .collect();
    directory.sort_by(|a, b| a[..4].cmp(&b[..4]));
    for (slot, record) in bytes[12..12 + count * 16]
        .chunks_exact_mut(16)
        .zip(directory)
    {
        slot.copy_from_slice(&record);
    }
    let fallback = OpenTypeMathFont::parse(&bytes, 0)
        .unwrap()
        .line_metrics(10.0)
        .unwrap();
    assert_eq!((fallback.ascent, fallback.descent), (12.0, 5.0));
}
