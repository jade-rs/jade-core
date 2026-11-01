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
}

fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        3 => Just(Operation::Old),
        3 => Just(Operation::New),
        1 => Just(Operation::Default),
        1 => Just(Operation::Missing),
        2 => Just(Operation::Reopen),
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

fn package(before: u8, after: u8, direct_left: bool) -> Vec<u8> {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx"),
    )
    .expect("fixture");
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Old"/><w:spacing {} {}/>{}</w:pPr><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#,
        spacing_attribute(before, "before"),
        spacing_attribute(after, "after"),
        if direct_left {
            r#"<w:ind w:left="240"/>"#
        } else {
            ""
        },
    );
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Old"><w:name w:val="Old"/><w:pPr><w:spacing w:before="40" w:after="60"/><w:ind w:left="100" w:right="140"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="New"><w:name w:val="New"/><w:pPr><w:spacing w:before="160" w:after="200"/><w:ind w:left="300" w:right="340"/></w:pPr></w:style></w:styles>"#;
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
type Formatting = (SpacingSide, SpacingSide, Option<f32>, Option<f32>);

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
        indent.left,
        indent.right,
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, max_shrink_iters: 3000, ..ProptestConfig::default() })]

    #[test]
    fn style_sequences_match_direct_formatting_model_after_every_step(
        before in 0u8..5,
        after in 0u8..5,
        direct_left in any::<bool>(),
        operations in prop::collection::vec(operation(), 1..25),
    ) {
        let mut doc = block_on(WordDocument::load(&package(before, after, direct_left), &crate::Options::default())).expect("load");
        let mut is_new = false;
        for (step, operation) in operations.iter().enumerate() {
            match operation {
                Operation::Reopen => {
                    doc = block_on(WordDocument::load(&doc.export_docx().expect("export"), &crate::Options::default())).expect("reopen");
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
                Some(if direct_left { 12.0 } else if is_new { 15.0 } else { 5.0 }),
                Some(if is_new { 17.0 } else { 7.0 }),
            );
            prop_assert_eq!(observed(&doc), expected, "live: step {} {:?}", step, operation);
            let reopened = block_on(WordDocument::load(&doc.export_docx().expect("export"), &crate::Options::default())).expect("reload");
            prop_assert_eq!(observed(&reopened), expected, "persisted: step {} {:?}", step, operation);
            let text: String = doc.paragraph(0).expect("paragraph").runs.iter().map(|run| run.text.as_str()).collect();
            prop_assert_eq!(text, "Body");
        }
    }
}
