use jade_text_layout::{wrap_text, WrapOptions};

/// One unit per ASCII character, two per anything wider — the same shape as a
/// monospace measurement, so the expected line ends are arithmetic.
fn monospace(text: &str) -> f64 {
    text.chars()
        .map(|c| if c.is_ascii() { 1.0 } else { 2.0 })
        .sum()
}

fn wrap(text: &str, max_width: f64) -> Vec<String> {
    wrap_text(text, max_width, &mut monospace, WrapOptions::default())
}

fn wrap_with(text: &str, max_width: f64, options: WrapOptions) -> Vec<String> {
    wrap_text(text, max_width, &mut monospace, options)
}

#[test]
fn text_that_fits_stays_on_one_line() {
    assert_eq!(wrap("hello", 10.0), ["hello"]);
}

#[test]
fn latin_breaks_at_a_space_rather_than_mid_word() {
    assert_eq!(wrap("hello there world", 12.0), ["hello there", "world"]);
}

#[test]
fn the_space_a_line_breaks_at_is_not_carried_to_the_next_line() {
    let lines = wrap("aaa bbb", 4.0);
    assert_eq!(lines, ["aaa", "bbb"]);
}

#[test]
fn a_word_longer_than_the_line_breaks_because_it_must() {
    assert_eq!(
        wrap("supercalifragilistic", 6.0),
        ["superc", "alifra", "gilist", "ic"]
    );
}

#[test]
fn east_asian_text_breaks_between_any_two_characters() {
    // 2 units each, so three per 6-unit line.
    assert_eq!(wrap("横浜市市税条例", 6.0), ["横浜市", "市税条", "例"]);
}

#[test]
fn a_closing_bracket_does_not_start_a_line() {
    // Without kinsoku the break would land before "」", pushing it down.
    let lines = wrap("「横浜」市", 6.0);
    assert_eq!(lines, ["「横", "浜」市"]);
    for line in &lines {
        assert!(
            !line.starts_with('」'),
            "{line} starts with a prohibited character"
        );
    }
}

#[test]
fn an_opening_bracket_does_not_end_a_line() {
    let lines = wrap("市税「横浜」", 6.0);
    for line in &lines {
        assert!(
            !line.ends_with('「'),
            "{line} ends with a prohibited character"
        );
    }
}

#[test]
fn a_full_stop_is_pulled_back_onto_the_line_it_ends() {
    let lines = wrap("条例。市税", 6.0);
    assert_eq!(lines[0], "条例。");
}

#[test]
fn turning_kinsoku_off_breaks_wherever_it_fits() {
    let lines = wrap_with(
        "「横浜」市",
        6.0,
        WrapOptions {
            kinsoku: false,
            ..WrapOptions::default()
        },
    );
    assert_eq!(lines, ["「横浜", "」市"]);
}

#[test]
fn allowing_mid_word_breaks_fills_the_line() {
    let lines = wrap_with(
        "hello there",
        8.0,
        WrapOptions {
            break_inside_words: true,
            ..WrapOptions::default()
        },
    );
    assert_eq!(lines, ["hello th", "ere"]);
}

#[test]
fn an_explicit_newline_always_breaks() {
    assert_eq!(wrap("a\nb", 100.0), ["a", "b"]);
    assert_eq!(wrap("", 100.0), [""]);
}

#[test]
fn a_line_narrower_than_one_character_still_makes_progress() {
    // Otherwise the filler would loop forever on a zero-width column.
    assert_eq!(wrap("横浜", 1.0), ["横", "浜"]);
}

#[test]
fn a_latin_word_after_east_asian_text_moves_whole_without_dragging_the_cjk_back() {
    // "手引書（IPA産業" — 12 units of CJK then a Latin word. A 14-unit line
    // holds the CJK and "IP"; the word may not split, so it moves to the next
    // line whole — but the break goes right before it, not back to the start
    // of the paragraph (there is no space to fall back to).
    assert_eq!(
        wrap("手引書は産業IPA分野", 14.0),
        ["手引書は産業", "IPA分野"]
    );
}

#[test]
fn a_latin_word_after_a_space_and_cjk_breaks_before_the_word_not_the_space() {
    // Falling back to the last space would leave "2026年7月," alone on the
    // first line. The CJK after the comma-space is breakable; only "p.49" is
    // a word.
    assert_eq!(wrap("手引書 資料の p.49", 14.0), ["手引書 資料の", "p.49"]);
    assert_eq!(
        wrap("手引書は産業 IPA分野", 14.0),
        ["手引書は産業", "IPA分野"]
    );
}

#[test]
fn a_latin_word_that_starts_the_line_still_breaks_inside_when_it_must() {
    assert_eq!(wrap("IPAIPAIPA横", 6.0), ["IPAIPA", "IPA横"]);
}

#[test]
fn a_break_at_a_space_does_not_hand_the_next_line_a_full_stop() {
    // The space at the break is swallowed, so a break that lands on it puts
    // whatever follows at the line start. Breaking "横浜 、市" after "横浜"
    // looks legal — a space is prohibited at neither end of a line — and
    // starts the next line with "、" all the same.
    assert_eq!(wrap("横浜 、市", 6.0), ["横", "浜 、", "市"]);
}

#[test]
fn a_break_before_a_space_does_not_strand_an_opening_bracket() {
    // The same blindness from the other side: the spaces before a break stay
    // on the line it ends, where they render as nothing, so the character a
    // reader sees last is the "「".
    assert_eq!(wrap("条例「 ab", 7.0), ["条例", "「 ab"]);
}
