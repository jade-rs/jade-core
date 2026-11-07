use super::*;
use jade_docx_layout::layout_cache::ParagraphHashableProps;
use jade_docx_reader::document::{Indent, Numbering, Spacing};
use proptest::prelude::*;
use std::hash::{Hash, Hasher};

// Compare every write, including option tags, order, lengths and str sentinels.
// A final hash alone could hide a changed input behind a collision.
#[derive(Default, Debug, PartialEq, Eq)]
struct HashWrites(Vec<Vec<u8>>);

impl Hasher for HashWrites {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.push(bytes.to_vec());
    }
}

fn legacy(props: &ParagraphProperties) -> ParagraphHashableProps {
    let spacing = props.spacing.as_ref();
    let indent = props.indent.as_ref();
    let numbering = props.numbering.as_ref();
    // The original render-hash call, using the public owned implementation as
    // the oracle rather than repeating the borrowed view's field conversions.
    ParagraphHashableProps::new(
        props.alignment.as_deref(),
        spacing.and_then(|s| s.before),
        spacing.and_then(|s| s.after),
        spacing.and_then(|s| s.line),
        indent.and_then(|i| i.physical_left(props.bidi.unwrap_or(false))),
        indent.and_then(|i| i.physical_right(props.bidi.unwrap_or(false))),
        indent.and_then(|i| i.first_line),
        indent.and_then(|i| i.hanging),
        indent.and_then(|i| i.physical_left_chars(props.bidi.unwrap_or(false))),
        indent.and_then(|i| i.physical_right_chars(props.bidi.unwrap_or(false))),
        props.style.as_deref(),
        numbering.and_then(|n| n.id.as_deref()),
        numbering.and_then(|n| n.level),
    )
}

fn check(props: Option<&ParagraphProperties>) {
    let mut expected = HashWrites::default();
    props.map(legacy).hash(&mut expected);
    let mut actual = HashWrites::default();
    props
        .map(BorrowedParagraphHashableProps::from)
        .hash(&mut actual);
    assert_eq!(actual, expected, "props={props:?}");
}

#[test]
fn absence_empty_and_unicode_strings_match_legacy() {
    check(None);
    check(Some(&ParagraphProperties::default()));
    let values = [
        None,
        Some(""),
        Some("left"),
        Some("日本語\0🧪\n𝄞"),
        Some("e\u{301}"),
    ];
    for alignment in values {
        for style in values {
            for id in values {
                for level in [None, Some(0), Some(i32::MAX as u32), Some(u32::MAX)] {
                    check(Some(&ParagraphProperties {
                        alignment: alignment.map(Into::into),
                        style: style.map(Into::into),
                        numbering: Some(Numbering {
                            id: id.map(Into::into),
                            level,
                        }),
                        ..Default::default()
                    }));
                }
            }
        }
    }
}

#[test]
fn float_cast_edges_and_logical_indent_precedence_match_legacy() {
    let values = [
        None,
        Some(0.0),
        Some(-0.0),
        Some(0.009),
        Some(-0.019),
        Some(f32::MIN_POSITIVE),
        Some(f32::from_bits(1)),
        Some(f32::MAX),
        Some(f32::MIN),
        Some(f32::INFINITY),
        Some(f32::NEG_INFINITY),
        Some(f32::NAN),
        Some(f32::from_bits(0xffc0_1234)),
        Some(21_474_836.0),
        Some(-21_474_836.0),
    ];
    for value in values {
        for bidi in [None, Some(false), Some(true)] {
            // Logical sides can be independently absent, falling back to the
            // physical side selected by bidi, including character-unit indents.
            for logical_sides in 0..4 {
                let props = ParagraphProperties {
                    bidi,
                    spacing: Some(Spacing {
                        before: value,
                        after: value,
                        line: value,
                        ..Default::default()
                    }),
                    indent: Some(Indent {
                        left: Some(1.25),
                        right: Some(-2.5),
                        start: (logical_sides & 1 != 0).then_some(value).flatten(),
                        end: (logical_sides & 2 != 0).then_some(value).flatten(),
                        first_line: value,
                        hanging: value,
                        left_chars: Some(i32::MIN),
                        right_chars: Some(i32::MAX),
                        start_chars: (logical_sides & 1 != 0).then_some(0),
                        end_chars: (logical_sides & 2 != 0).then_some(-123),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                check(Some(&props));
                // Exercise both physical fields with the same extreme value.
                let mut physical = props.clone();
                let indent = physical.indent.as_mut().unwrap();
                indent.left = value;
                indent.right = value;
                indent.start = None;
                indent.end = None;
                check(Some(&physical));
            }
        }
    }
}

fn optional_string() -> impl Strategy<Value = Option<String>> {
    proptest::option::of(
        proptest::collection::vec(any::<char>(), 0..48)
            .prop_map(|chars| chars.into_iter().collect()),
    )
}

fn properties() -> impl Strategy<Value = ParagraphProperties> {
    (
        optional_string(),
        optional_string(),
        proptest::option::of(any::<bool>()),
        proptest::option::of(proptest::array::uniform3(
            proptest::option::of(any::<u32>()),
        )),
        proptest::option::of((
            proptest::array::uniform6(proptest::option::of(any::<u32>())),
            proptest::array::uniform6(proptest::option::of(any::<i32>())),
        )),
        proptest::option::of((optional_string(), proptest::option::of(any::<u32>()))),
    )
        .prop_map(
            |(alignment, style, bidi, spacing, indent, numbering)| ParagraphProperties {
                alignment: alignment.map(Into::into),
                style: style.map(Into::into),
                bidi,
                spacing: spacing.map(|v| Spacing {
                    before: v[0].map(f32::from_bits),
                    after: v[1].map(f32::from_bits),
                    line: v[2].map(f32::from_bits),
                    ..Default::default()
                }),
                indent: indent.map(|(v, c)| Indent {
                    left: v[0].map(f32::from_bits),
                    right: v[1].map(f32::from_bits),
                    start: v[2].map(f32::from_bits),
                    end: v[3].map(f32::from_bits),
                    first_line: v[4].map(f32::from_bits),
                    hanging: v[5].map(f32::from_bits),
                    left_chars: c[0],
                    right_chars: c[1],
                    start_chars: c[2],
                    end_chars: c[3],
                    first_line_chars: c[4],
                    hanging_chars: c[5],
                }),
                numbering: numbering.map(|(id, level)| Numbering { id, level }),
                ..Default::default()
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_property_hash_writes_match_legacy(props in proptest::option::of(properties())) {
        check(props.as_ref());
    }
}
