//! Opening and repairing a damaged Word document, as Word's 開いて修復
//! does: what can be recovered is, and what was dropped is said.

use std::path::PathBuf;

use jade_docx_editor::word::repair::{needs_repair, repair, RepairNote};
use jade_opc::OpcPackage;

fn contract() -> Vec<u8> {
    std::fs::read(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contract.docx"
    )))
    .unwrap()
}

fn document_xml(docx: &[u8]) -> String {
    OpcPackage::open(docx)
        .unwrap()
        .part_str("word/document.xml")
        .unwrap()
        .unwrap()
}

fn with_part(docx: &[u8], name: &str, change: impl FnOnce(String) -> Option<String>) -> Vec<u8> {
    let mut package = OpcPackage::open(docx).unwrap();
    let xml = package.part_str(name).unwrap().unwrap_or_default();
    match change(xml) {
        Some(xml) => package.replace(name, xml.into_bytes()),
        None => {
            package.remove(name);
        }
    }
    package.write().unwrap()
}

#[test]
fn a_sound_document_needs_no_repair() {
    assert!(!needs_repair(&contract()));
}

#[test]
fn a_download_cut_short_keeps_the_entries_that_arrived() {
    let docx = contract();
    // Cut off before the central directory, as a stopped download is.
    let cut = &docx[..docx.len() * 9 / 10];
    assert!(needs_repair(cut));
    let repaired = repair(cut).unwrap();
    assert!(repaired.notes.contains(&RepairNote::ArchiveRebuilt));
    let xml = document_xml(&repaired.docx);
    assert!(xml.contains("秘密保持"), "the text survives");
    assert!(!needs_repair(&repaired.docx));
}

#[test]
fn a_deflated_archive_cut_short_keeps_its_whole_entries() {
    // The contract written again, every entry deflated.
    let docx = OpcPackage::open(&contract()).unwrap().write().unwrap();
    // Without its central directory.
    let directory = docx
        .windows(4)
        .position(|window| window == [0x50, 0x4b, 0x01, 0x02])
        .unwrap();
    let cut = &docx[..directory];
    assert!(needs_repair(cut));
    let repaired = repair(cut).unwrap();
    let original = document_xml(&docx);
    assert_eq!(document_xml(&repaired.docx), original);
}

#[test]
fn damaged_xml_keeps_its_text_and_says_so() {
    let docx = with_part(&contract(), "word/document.xml", |xml| {
        // Close tags lost in the middle of the body, and garbage after.
        let half = xml.len() / 2;
        let cut = xml[..half].rfind("</w:p>").unwrap();
        Some(format!(
            "{}<w:p><w:r><w:t>broken{}",
            &xml[..cut],
            &xml[cut + "</w:p>".len()..]
        ))
    });
    assert!(needs_repair(&docx));
    let repaired = repair(&docx).unwrap();
    assert!(repaired.notes.iter().any(
        |note| matches!(note, RepairNote::PartMended { part, .. } if part == "word/document.xml")
    ));
    let xml = document_xml(&repaired.docx);
    assert!(xml.contains("秘密保持"));
    assert!(xml.contains("broken"));
    assert!(!needs_repair(&repaired.docx));
}

#[test]
fn missing_content_types_are_made_again_and_dangling_relationships_dropped() {
    let docx = with_part(&contract(), "[Content_Types].xml", |_| None);
    let docx = with_part(&docx, "word/_rels/document.xml.rels", |rels| {
        Some(rels.replace(
            "</Relationships>",
            r#"<Relationship Id="rIdGone" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/gone.png"/></Relationships>"#,
        ))
    });
    assert!(needs_repair(&docx));
    let repaired = repair(&docx).unwrap();
    assert!(repaired.notes.contains(&RepairNote::ContentTypesRebuilt));
    assert!(repaired.notes.contains(&RepairNote::RelationshipDropped(
        "word/media/gone.png".into()
    )));
    let types = OpcPackage::open(&repaired.docx)
        .unwrap()
        .part_str("[Content_Types].xml")
        .unwrap()
        .unwrap();
    assert!(types.contains("wordprocessingml.document.main"));
}

#[test]
fn nothing_left_of_the_document_is_an_error() {
    assert!(repair(b"PK\x03\x04garbage").is_err());
}
