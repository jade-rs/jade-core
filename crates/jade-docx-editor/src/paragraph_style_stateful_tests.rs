//! Public style changes must preserve direct formatting across DOCX persistence.
use super::*;
use futures::executor::block_on;
use proptest::prelude::*;

#[derive(Clone, Copy, Debug)]
enum Operation {
    Old,
    New,
    Default,
    Missing,
    Reopen,
    Left(i8),
    Right(i8),
    LeftChars(i8),
    RightChars(i8),
    FirstLine(i8),
}

fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        3 => Just(Operation::Old),
        3 => Just(Operation::New),
        1 => Just(Operation::Default),
        1 => Just(Operation::Missing),
        2 => Just(Operation::Reopen),
        2 => (-10i8..21).prop_map(Operation::Left),
        2 => (-10i8..21).prop_map(Operation::Right),
        2 => (-10i8..21).prop_map(Operation::LeftChars),
        2 => (-10i8..21).prop_map(Operation::RightChars),
        2 => (-10i8..21).prop_map(Operation::FirstLine),
    ]
}

// An independent side model: inherited, points, lines, disabled auto, enabled auto.
fn side(mode: u8, inherited: f32) -> (Option<f32>, Option<f32>, bool) {
    match mode {
        1 => (Some(4.0), None, false),
        2 => (None, Some(1.0), false),
        4 => (Some(14.0), None, true),
        _ => (Some(inherited), None, false),
    }
}

fn spacing_attribute(mode: u8, side: &str) -> String {
    match mode {
        1 => format!("w:{side}=\"80\""),
        2 => format!("w:{side}Lines=\"100\""),
        3 => format!("w:{side}Autospacing=\"off\""),
        4 => format!("w:{side}Autospacing=\"on\""),
        _ => String::new(),
    }
}

fn package(before: u8, after: u8, direct_left: bool, bidi: bool) -> Vec<u8> {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx"),
    )
    .expect("fixture");
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr>{}<w:pStyle w:val="Old"/><w:spacing {} {}/>{}</w:pPr><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#,
        if bidi { "<w:bidi/>" } else { "" },
        spacing_attribute(before, "before"),
        spacing_attribute(after, "after"),
        if direct_left {
            r#"<w:ind w:left="240"/>"#
        } else {
            ""
        },
    );
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Old"><w:name w:val="Old"/><w:pPr><w:spacing w:before="40" w:after="60"/><w:ind w:left="100" w:right="140" w:firstLine="120"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="New"><w:name w:val="New"/><w:pPr><w:spacing w:before="160" w:after="200"/><w:ind w:left="300" w:right="340" w:firstLine="180"/></w:pPr></w:style></w:styles>"#;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("entry");
        let name = entry.name().expect("name").to_string();
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut data).expect("read");
        match name.as_str() {
            "word/document.xml" => data = document.as_bytes().to_vec(),
            "word/styles.xml" => data = styles.as_bytes().to_vec(),
            _ => {}
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("entry");
        std::io::Write::write_all(&mut output, &data).expect("write");
    }
    output.finish().expect("finish").into_inner()
}

type SpacingSide = (Option<f32>, Option<f32>, bool);
type Edge = (Option<f32>, Option<i32>);
type Formatting = (SpacingSide, SpacingSide, Edge, Edge, Option<f32>);

fn observed(doc: &WordDocument) -> Formatting {
    let properties = doc
        .paragraph(0)
        .expect("paragraph")
        .properties
        .as_ref()
        .expect("properties");
    let spacing = properties.spacing.as_ref().expect("spacing");
    let indent = properties.indent.as_ref().expect("indent");
    (
        (spacing.before, spacing.before_lines, spacing.before_auto),
        (spacing.after, spacing.after_lines, spacing.after_auto),
        (indent.left, indent.left_chars),
        (indent.right, indent.right_chars),
        indent.first_line,
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, max_shrink_iters: 3000, ..ProptestConfig::default() })]

    #[test]
    fn style_sequences_match_direct_formatting_model_after_every_step(
        before in 0u8..5,
        after in 0u8..5,
        direct_left in any::<bool>(),
        bidi in any::<bool>(),
        operations in prop::collection::vec(operation(), 1..25),
    ) {
        let mut doc = block_on(WordDocument::load(&package(before, after, direct_left, bidi), &crate::Options::default())).expect("load");
        let mut is_new = false;
        let mut left_override = direct_left.then_some((Some(12.0), None));
        let mut right_override = None;
        let mut special_override = None;
        for (step, operation) in operations.iter().enumerate() {
            match operation {
                Operation::Reopen => {
                    doc = block_on(WordDocument::load(&doc.export_docx().expect("export"), &crate::Options::default())).expect("reopen");
                }
                Operation::Left(value) | Operation::Right(value) | Operation::LeftChars(value) | Operation::RightChars(value) | Operation::FirstLine(value) => {
                    let left = matches!(operation, Operation::Left(_));
                    let right = matches!(operation, Operation::Right(_));
                    let left_chars = matches!(operation, Operation::LeftChars(_));
                    let right_chars = matches!(operation, Operation::RightChars(_));
                    let special = matches!(operation, Operation::FirstLine(_));
                    let points = f32::from(*value);
                    let chars = i32::from(*value) * 100;
                    if left { left_override = Some((Some(points), None)); }
                    if right { right_override = Some((Some(points), None)); }
                    if left_chars { left_override = Some((None, Some(chars))); }
                    if right_chars { right_override = Some((None, Some(chars))); }
                    if special { special_override = Some(points); }
                    apply_headless_commands_to_document(&mut doc, &HeadlessEditRequest { commands: vec![HeadlessCommand::SetParagraphIndent {
                        start_paragraph: 0, end_paragraph: 0,
                        left: left.then_some(points), right: right.then_some(points),
                        first_line: special.then_some(points), hanging: None,
                        left_chars: left_chars.then_some(chars), right_chars: right_chars.then_some(chars),
                        first_line_chars: None, hanging_chars: None, clear_special: false,
                    }] }).expect("indent change");
                }
                _ => {
                    is_new = matches!(operation, Operation::New);
                    let style_id = match operation {
                        Operation::Old => Some("Old".to_string()),
                        Operation::New => Some("New".to_string()),
                        Operation::Missing => Some("Missing".to_string()),
                        _ => None,
                    };
                    apply_headless_commands_to_document(&mut doc, &HeadlessEditRequest { commands: vec![HeadlessCommand::SetParagraphStyle { start_paragraph: 0, end_paragraph: 0, style_id }] }).expect("style change");
                }
            }
            let expected = (
                side(before, if is_new { 8.0 } else { 2.0 }),
                side(after, if is_new { 10.0 } else { 3.0 }),
                left_override.unwrap_or((Some(if is_new { 15.0 } else { 5.0 }), None)),
                right_override.unwrap_or((Some(if is_new { 17.0 } else { 7.0 }), None)),
                Some(special_override.unwrap_or(if is_new { 9.0 } else { 6.0 })),
            );
            prop_assert_eq!(observed(&doc), expected, "live: step {} {:?}", step, operation);
            let reopened = block_on(WordDocument::load(&doc.export_docx().expect("export"), &crate::Options::default())).expect("reload");
            prop_assert_eq!(observed(&reopened), expected, "persisted: step {} {:?}", step, operation);
            let text: String = doc.paragraph(0).expect("paragraph").runs.iter().map(|run| run.text.as_str()).collect();
            prop_assert_eq!(text, "Body");
        }
    }
}

#[test]
fn editing_one_indent_edge_keeps_the_other_edge_and_special_indent_inherited() {
    for bidi in [false, true] {
        let mut doc = block_on(WordDocument::load(
            &package(0, 0, false, bidi),
            &crate::Options::default(),
        ))
        .expect("load");
        apply_headless_commands_to_document(
            &mut doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetParagraphIndent {
                        start_paragraph: 0,
                        end_paragraph: 0,
                        left: Some(0.0),
                        right: None,
                        first_line: None,
                        hanging: None,
                        left_chars: None,
                        right_chars: None,
                        first_line_chars: None,
                        hanging_chars: None,
                        clear_special: false,
                    },
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 0,
                        end_paragraph: 0,
                        style_id: Some("New".to_string()),
                    },
                ],
            },
        )
        .expect("edit and restyle");
        let reopened = block_on(WordDocument::load(
            &doc.export_docx().expect("export"),
            &crate::Options::default(),
        ))
        .expect("reload");
        for actual in [&doc, &reopened] {
            assert_eq!(
                observed(actual),
                (
                    (Some(8.0), None, false),
                    (Some(10.0), None, false),
                    (Some(0.0), None),
                    (Some(17.0), None),
                    Some(9.0)
                )
            );
        }
    }
}

#[test]
fn ribbon_indent_edits_keep_unedited_groups_inherited_after_restyling() {
    use jade_docx_headless_api::{HeadlessIndentDirection, HeadlessIndentTarget};
    for bidi in [false, true] {
        for target in [HeadlessIndentTarget::Start, HeadlessIndentTarget::Hanging] {
            let mut doc = block_on(WordDocument::load(
                &package(0, 0, false, bidi),
                &crate::Options::default(),
            ))
            .expect("load");
            let stop = doc
                .document
                .as_ref()
                .and_then(|d| d.default_tab_stop)
                .unwrap_or(36.0);
            apply_headless_commands_to_document(
                &mut doc,
                &HeadlessEditRequest {
                    commands: vec![
                        HeadlessCommand::AdjustParagraphIndent {
                            start_paragraph: 0,
                            end_paragraph: 0,
                            direction: HeadlessIndentDirection::Increase,
                            target,
                        },
                        HeadlessCommand::SetParagraphStyle {
                            start_paragraph: 0,
                            end_paragraph: 0,
                            style_id: Some("New".to_string()),
                        },
                    ],
                },
            )
            .expect("ribbon and restyle");
            let reopened = block_on(WordDocument::load(
                &doc.export_docx().expect("export"),
                &crate::Options::default(),
            ))
            .expect("reload");
            for actual in [&doc, &reopened] {
                let indent = actual
                    .paragraph(0)
                    .unwrap()
                    .properties
                    .as_ref()
                    .unwrap()
                    .indent
                    .as_ref()
                    .unwrap();
                assert_eq!(indent.left, Some(if bidi { 15.0 } else { stop }));
                assert_eq!(indent.right, Some(if bidi { stop } else { 17.0 }));
                if target == HeadlessIndentTarget::Start {
                    assert_eq!(indent.first_line, Some(9.0));
                    assert_eq!(indent.hanging, None);
                } else {
                    assert_eq!(indent.first_line, None);
                    assert_eq!(indent.hanging, Some(stop - if bidi { 13.0 } else { 11.0 }));
                }
            }
        }
    }
}
