//! Check the renderer's use of the property digest, not only its conversion.
use super::*;
use jade_docx_reader::document::{Indent, Numbering, Spacing};
use std::rc::Rc;

type PropertyEdit = fn(&mut ParagraphProperties);

fn properties() -> ParagraphProperties {
    ParagraphProperties {
        spacing: Some(Spacing::default()),
        indent: Some(Indent::default()),
        numbering: Some(Numbering {
            id: None,
            level: None,
        }),
        ..Default::default()
    }
}

fn paragraph(properties: ParagraphProperties) -> Paragraph {
    Paragraph {
        properties: Some(properties),
        runs: vec![build_run(None)],
        node_id: None,
        comment_ranges: Vec::new(),
        comment_markers: Vec::new(),
    }
}

#[test]
fn render_hash_observes_each_property_and_restores_previous_keys() {
    let renderer = test_renderer();
    let options = Options::default();
    let base = properties();
    // Each edit changes exactly one of the legacy digest's 13 fields. Testing
    // the full renderer catches a missing/constant property digest at its call
    // site, which converter-only equivalence tests cannot observe.
    let edits: [(&str, PropertyEdit); 13] = [
        ("alignment", |p| p.alignment = Some("center".into())),
        ("spacing before", |p| {
            p.spacing.as_mut().unwrap().before = Some(3.25)
        }),
        ("spacing after", |p| {
            p.spacing.as_mut().unwrap().after = Some(4.5)
        }),
        ("line spacing", |p| {
            p.spacing.as_mut().unwrap().line = Some(1.5)
        }),
        ("left indent", |p| {
            p.indent.as_mut().unwrap().left = Some(12.5)
        }),
        ("right indent", |p| {
            p.indent.as_mut().unwrap().right = Some(13.5)
        }),
        ("first line", |p| {
            p.indent.as_mut().unwrap().first_line = Some(2.5)
        }),
        ("hanging", |p| {
            p.indent.as_mut().unwrap().hanging = Some(1.25)
        }),
        ("left chars", |p| {
            p.indent.as_mut().unwrap().left_chars = Some(125)
        }),
        ("right chars", |p| {
            p.indent.as_mut().unwrap().right_chars = Some(-250)
        }),
        ("style", |p| p.style = Some("日本語\0e\u{301}🧪".into())),
        ("numbering id", |p| {
            p.numbering.as_mut().unwrap().id = Some("42".into())
        }),
        ("numbering level", |p| {
            p.numbering.as_mut().unwrap().level = Some(2)
        }),
    ];

    for content in [None, Some(ContentHash::new(7))] {
        let key =
            |props| renderer.compute_paragraph_render_hash(&paragraph(props), &options, content);
        let original = key(base.clone());
        let mut changed = Vec::new();
        for (name, edit) in edits {
            let mut props = base.clone();
            edit(&mut props);
            let digest = key(props.clone());
            assert_ne!(digest, original, "{name}, content={content:?}");
            changed.push((name, props, digest));
            assert_eq!(key(base.clone()), original, "restore after {name}");
        }
        // Revisit earlier values in reverse order, with newly allocated owned
        // strings, rather than relying on the address of the first snapshot.
        for (name, props, digest) in changed.into_iter().rev() {
            let mut reallocated = props.clone();
            for value in [&mut reallocated.alignment, &mut reallocated.style]
                .into_iter()
                .flatten()
            {
                let replacement: Rc<str> = Rc::from(value.as_ref());
                assert!(!Rc::ptr_eq(value, &replacement));
                *value = replacement;
            }
            assert_eq!(key(reallocated), digest, "revisit {name}");
            assert_eq!(key(base.clone()), original, "restore after revisit {name}");
        }
    }
}

#[test]
fn render_hash_resolves_logical_indents_on_both_sides() {
    let renderer = test_renderer();
    let options = Options::default();
    for bidi in [None, Some(false), Some(true)] {
        for sides in 0..4 {
            let mut logical = properties();
            logical.bidi = bidi;
            logical.indent = Some(Indent {
                left: Some(1.25),
                right: Some(-2.5),
                start: (sides & 1 != 0).then_some(17.0),
                end: (sides & 2 != 0).then_some(-19.0),
                left_chars: Some(125),
                right_chars: Some(-250),
                start_chars: (sides & 1 != 0).then_some(1700),
                end_chars: (sides & 2 != 0).then_some(-1900),
                ..Default::default()
            });
            // Derive physical expectations explicitly, independently of the
            // production physical_left/right helpers and borrowed conversion.
            let rtl = bidi == Some(true);
            let left_present = sides & if rtl { 2 } else { 1 } != 0;
            let right_present = sides & if rtl { 1 } else { 2 } != 0;
            let (left, right, left_chars, right_chars) = if rtl {
                (-19.0, 17.0, -1900, 1700)
            } else {
                (17.0, -19.0, 1700, -1900)
            };
            let mut physical = properties();
            physical.bidi = bidi;
            physical.indent = Some(Indent {
                left: Some(if left_present { left } else { 1.25 }),
                right: Some(if right_present { right } else { -2.5 }),
                left_chars: Some(if left_present { left_chars } else { 125 }),
                right_chars: Some(if right_present { right_chars } else { -250 }),
                ..Default::default()
            });
            for content in [None, Some(ContentHash::new(7))] {
                assert_eq!(
                    renderer.compute_paragraph_render_hash(
                        &paragraph(logical.clone()),
                        &options,
                        content
                    ),
                    renderer.compute_paragraph_render_hash(
                        &paragraph(physical.clone()),
                        &options,
                        content
                    ),
                    "bidi={bidi:?}, sides={sides}, content={content:?}"
                );
            }
        }
    }
}

#[test]
fn render_hash_keeps_legacy_float_buckets_and_string_option_tags() {
    let renderer = test_renderer();
    let options = Options::default();
    for content in [None, Some(ContentHash::new(7))] {
        let key =
            |props| renderer.compute_paragraph_render_hash(&paragraph(props), &options, content);
        // Explicit same-bucket pairs cover truncation, signed zero, NaN and
        // saturation. Only the spacing field differs between each pair.
        for (left, right) in [
            (1.001, 1.009),
            (-1.001, -1.009),
            (0.0, -0.0),
            (0.0, f32::NAN),
            (f32::MAX, f32::INFINITY),
            (f32::MIN, f32::NEG_INFINITY),
        ] {
            let mut a = properties();
            a.spacing.as_mut().unwrap().before = Some(left);
            let mut b = a.clone();
            b.spacing.as_mut().unwrap().before = Some(right);
            assert_eq!(key(a), key(b), "{left:?} vs {right:?}, content={content:?}");
        }
        let empty_string_edits: [PropertyEdit; 3] = [
            |p| p.alignment = Some("".into()),
            |p| p.style = Some("".into()),
            |p| p.numbering.as_mut().unwrap().id = Some("".into()),
        ];
        for edit in empty_string_edits {
            let absent = properties();
            let mut empty = absent.clone();
            edit(&mut empty);
            assert_ne!(
                key(absent),
                key(empty),
                "absent vs empty, content={content:?}"
            );
        }
    }
}
