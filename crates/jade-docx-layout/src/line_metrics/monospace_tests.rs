use super::{word_monospace_advance_px, word_monospace_cell_em, word_monospace_em_px};
use proptest::prelude::*;

// Build the public fixed-pitch repertoire as a lookup table, independently of
// the scalar decoding and range matching performed by the implementation.
fn reference_cells() -> Vec<Option<f64>> {
    let mut cells = vec![None; 0x110000];
    for (first, last, width) in [
        (0x20, 0x7e, 0.5),
        (0xff61, 0xff9f, 0.5),
        (0x3000, 0x30ff, 1.0),
        (0x31f0, 0x31ff, 1.0),
        (0x3200, 0x33ff, 1.0),
        (0x3400, 0x4dbf, 1.0),
        (0x4e00, 0x9fff, 1.0),
        (0xf900, 0xfaff, 1.0),
        (0xff01, 0xff60, 1.0),
        (0xffe0, 0xffe6, 1.0),
        (0x20000, 0x2fffd, 1.0),
    ] {
        cells[first..=last].fill(Some(width));
    }
    cells
}

fn reference_cell(text: &str, cells: &[Option<f64>]) -> Option<f64> {
    let chars: Vec<_> = text.chars().collect();
    match chars.as_slice() {
        [ch] => cells[*ch as usize],
        _ => None,
    }
}

#[test]
fn fixed_pitch_cells_match_every_unicode_scalar_and_reject_appended_scalars() {
    let cells = reference_cells();
    assert_eq!(word_monospace_cell_em(""), None);
    for (value, &cell) in cells.iter().enumerate() {
        let Some(ch) = char::from_u32(value as u32) else {
            continue;
        };
        let text = ch.to_string();
        assert_eq!(word_monospace_cell_em(&text), cell, "U+{value:04X}");
        // Each valid scalar can occur before another scalar. Byte length alone
        // must never confuse a multi-scalar string with one UTF-8 character.
        for suffix in ['\0', '\u{3099}', '日', '🦀'] {
            let mut combined = text.clone();
            combined.push(suffix);
            assert_eq!(word_monospace_cell_em(&combined), None, "{combined:?}");
        }
    }
}

const FONTS: [(&str, Option<f64>); 12] = [
    ("10.50pt 'ＭＳ 明朝', 'MS Mincho', serif", Some(14.0)),
    ("bold 12.00pt 'MS Gothic', serif", Some(16.0)),
    ("italic 6.00pt \"MS-Gothic\", serif", Some(8.0)),
    ("24.00pt 'MS Mincho'", Some(32.0)),
    ("10.50pt 'MS PMincho', 'MS Mincho'", None),
    ("12.00pt 'Century', 'MS Mincho'", None),
    ("12.00pt 'MS UI Gothic'", None),
    ("0.00pt 'MS Mincho'", None),
    ("-12.00pt 'MS Mincho'", None),
    ("NaNpt 'MS Mincho'", None),
    ("not a descriptor", None),
    ("", None),
];

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => any::<char>().prop_map(|ch| ch.to_string()),
        3 => prop::sample::select(vec!["", "A", " ", "あ", "字", "ｱ", "é", "\t", "か\u{3099}", "♥️", "𠀀", "\0日"])
            .prop_map(String::from),
        2 => prop::collection::vec(any::<char>(), 0..10)
            .prop_map(|chars| chars.into_iter().collect()),
    ]
}

fn check_history(history: &[(usize, String)]) -> Result<(), TestCaseError> {
    let cells = reference_cells();
    // Reuse the caller's buffer, so the cache must follow descriptor content
    // even when the String's allocation stays at the same address.
    let mut descriptor = String::with_capacity(256);
    for (step, (font, text)) in history.iter().enumerate() {
        let (source, em) = FONTS[*font];
        descriptor.clear();
        descriptor.push_str(source);
        let cell = reference_cell(text, &cells);
        let advance = cell.zip(em).map(|(cell, em)| cell * em);
        for _ in 0..2 {
            prop_assert_eq!(
                word_monospace_em_px(&descriptor),
                em,
                "step={}, font={:?}",
                step,
                source
            );
            prop_assert_eq!(
                word_monospace_cell_em(text),
                cell,
                "step={}, text={:?}",
                step,
                text
            );
            prop_assert_eq!(
                word_monospace_advance_px(&descriptor, text),
                advance,
                "step={}, font={:?}, text={:?}",
                step,
                source,
                text
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn fixed_pitch_advance_follows_descriptor_and_text_histories(
        history in prop::collection::vec((0usize..FONTS.len(), text()), 1..65)
    ) {
        check_history(&history)?;
    }

    #[test]
    fn fixed_pitch_cells_reject_all_multi_scalar_strings(
        chars in prop::collection::vec(any::<char>(), 2..33)
    ) {
        let text: String = chars.into_iter().collect();
        prop_assert_eq!(word_monospace_cell_em(&text), None);
    }
}

#[test]
fn fixed_pitch_cache_handles_every_descriptor_transition_and_restoration() {
    let mut history = Vec::new();
    for first in 0..FONTS.len() {
        for second in 0..FONTS.len() {
            for font in [first, second, first] {
                for text in ["A", "字", "ｱ", "𠀀", "か\u{3099}", "", "é"] {
                    history.push((font, text.to_owned()));
                }
            }
        }
    }
    check_history(&history).unwrap();
}
