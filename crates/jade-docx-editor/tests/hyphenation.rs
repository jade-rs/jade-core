//! Word's ハイフネーション dialog: the document's automatic hyphenation,
//! written to settings.xml where Word keeps it, read back, and undone.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::CommandExecutor;
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest, HeadlessHyphenationPatch};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn settings(document: &WordDocument) -> String {
    let exported = document.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
    let mut xml = String::new();
    archive
        .by_name("word/settings.xml")
        .expect("settings part")
        .read_to_string(&mut xml)
        .expect("read");
    xml
}

#[test]
fn hyphenation_settings_are_saved_read_back_and_undone() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let mut document = load_bytes(&std::fs::read(path).expect("fixture"));
    // hello.docx turns hyphenation off explicitly.
    assert!(settings(&document).contains(r#"<w:autoHyphenation w:val="0"/>"#));
    assert!(!document.hyphenation().auto);
    let mut executor = CommandExecutor::new();
    let request = HeadlessEditRequest {
        commands: vec![HeadlessCommand::SetHyphenation {
            patch: HeadlessHyphenationPatch {
                auto: Some(true),
                zone: Some(21.25),
                consecutive_limit: Some(2),
                hyphenate_caps: Some(false),
            },
        }],
    };
    apply_headless_commands_with_executor_to_document(&mut document, &mut executor, None, &request)
        .expect("apply");

    let xml = settings(&document);
    assert!(xml.contains("<w:autoHyphenation/>"), "{xml}");
    assert!(xml.contains(r#"<w:hyphenationZone w:val="425"/>"#), "{xml}");
    assert!(
        xml.contains(r#"<w:consecutiveHyphenLimit w:val="2"/>"#),
        "{xml}"
    );
    assert!(xml.contains("<w:doNotHyphenateCaps/>"), "{xml}");
    // In the schema's order, before the even/odd header switch.
    assert!(xml.find("w:autoHyphenation") < xml.find("w:evenAndOddHeaders"));

    let reopened = load_bytes(&document.export_docx().expect("export"));
    let read = reopened.hyphenation();
    assert!(read.auto && read.do_not_hyphenate_caps);
    assert_eq!(read.zone_twips, Some(425));
    assert_eq!(read.consecutive_limit, Some(2));

    // A limit of 0 is no limit: the element goes.
    let request = HeadlessEditRequest {
        commands: vec![HeadlessCommand::SetHyphenation {
            patch: HeadlessHyphenationPatch {
                consecutive_limit: Some(0),
                ..Default::default()
            },
        }],
    };
    apply_headless_commands_with_executor_to_document(&mut document, &mut executor, None, &request)
        .expect("apply");
    assert!(!settings(&document).contains("consecutiveHyphenLimit"));
    assert_eq!(document.hyphenation().consecutive_limit, None);

    let mut selection = None;
    {
        let mut context =
            jade_docx_editor::commands::CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut context).expect("undo");
        executor.undo(&mut context).expect("undo");
    }
    assert!(!document.hyphenation().auto);
    // The fixture's own explicit "off" is back.
    assert!(settings(&document).contains(r#"<w:autoHyphenation w:val="0"/>"#));
    assert!(!settings(&document).contains("hyphenationZone"));
}
