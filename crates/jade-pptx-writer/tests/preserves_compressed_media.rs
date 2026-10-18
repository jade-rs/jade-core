use jade_opc::OpcPackage;
use jade_pptx_writer::EditSession;
use std::io::{Cursor, Write};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn stored_media_deck() -> Vec<u8> {
    let package = OpcPackage::open(DECK).unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for name in package.part_names() {
        let options = zip::write::SimpleFileOptions::default().compression_method(
            if name.starts_with("ppt/media/") {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            },
        );
        writer.start_file(name, options).unwrap();
        writer.write_all(package.part(name).unwrap()).unwrap();
    }
    writer.set_comment("preserved PPTX comment").unwrap();
    writer.finish().unwrap().into_inner()
}
fn raw_media(bytes: &[u8]) -> Vec<(String, Vec<u8>, zip::CompressionMethod)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut result = Vec::new();
    for index in 0..zip.len() {
        let file = zip.by_index(index).unwrap();
        if file.name().starts_with("ppt/media/") {
            let start = file.data_start().unwrap() as usize;
            let end = start + file.compressed_size() as usize;
            result.push((
                file.name().to_string(),
                bytes[start..end].to_vec(),
                file.compression(),
            ));
        }
    }
    result
}
#[test]
fn text_crop_notes_and_slide_edits_copy_the_original_compressed_media() {
    let bytes = stored_media_deck();
    let media = raw_media(&bytes);
    assert!(!media.is_empty());
    for edit in ["text", "crop", "notes", "slide"] {
        let mut session = EditSession::open(&bytes).unwrap();
        match edit {
            "text" => {
                session.set_shape_text(0, 2, "edited title", false).unwrap();
            }
            "notes" => {
                session.set_slide_notes(0, "edited notes").unwrap();
            }
            "slide" => {
                session.duplicate_slide(0).unwrap();
            }
            "crop" => {
                let picture = jade_pptx_writer::editable_shapes(&bytes, 1)
                    .unwrap()
                    .into_iter()
                    .find(|s| s.kind == "picture")
                    .unwrap();
                session
                    .set_picture_crop(
                        1,
                        picture.id,
                        jade_pptx_reader::SourceRect {
                            left: 25000,
                            ..Default::default()
                        },
                        true,
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(raw_media(session.bytes()), media, "{edit}");
        assert_eq!(
            zip::ZipArchive::new(Cursor::new(session.bytes()))
                .unwrap()
                .comment(),
            b"preserved PPTX comment"
        );
        assert!(session.undo());
        assert_eq!(session.bytes(), bytes);
        assert!(session.redo());
        assert_eq!(raw_media(session.bytes()), media);
    }
}
