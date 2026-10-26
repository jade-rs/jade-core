use jade_docx_writer::write_docx_preserving_archive;
use proptest::prelude::*;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

fn source(files: &HashMap<String, Vec<u8>>, stored: bool) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .set_raw_comment(b"source package comment".to_vec().into_boxed_slice())
        .unwrap();
    for (name, bytes) in files {
        let options = if stored {
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
        } else {
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(1))
        };
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn inflated(bytes: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut part = archive.by_index(index).unwrap();
            let name = part.name().unwrap().into_owned();
            let mut data = Vec::new();
            part.read_to_end(&mut data).unwrap(); // Also checks the ZIP CRC.
            (name, data)
        })
        .collect()
}

fn raw_member(bytes: &[u8], name: &str) -> (CompressionMethod, Vec<u8>) {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let part = archive.by_name(name).unwrap();
    let start = part.data_start().unwrap() as usize;
    let end = start + part.compressed_size() as usize;
    (part.compression(), bytes[start..end].to_vec())
}

#[test]
fn preserves_registered_unix_extra_fields_from_existing_packages() {
    use zip::write::FileOptions;

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut options = FileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .with_file_comment("original member");
    // Info-ZIP Unix version 1, four-byte UID/GID, as found in real DOCX fixtures.
    let unix_ids = [1, 4, 0xe8, 3, 0, 0, 4, 0xe8, 3, 0, 0];
    options.add_extra_field(0x7875, unix_ids, false).unwrap();
    writer.start_file("custom/opaque.bin", options).unwrap();
    writer.write_all(b"unchanged package data").unwrap();
    let source = writer.finish().unwrap().into_inner();
    let output = write_docx_preserving_archive(&source, &HashMap::new(), &HashSet::new()).unwrap();
    assert_eq!(inflated(&output), inflated(&source));
    assert_eq!(
        raw_member(&output, "custom/opaque.bin"),
        raw_member(&source, "custom/opaque.bin")
    );
    let mut before = ZipArchive::new(Cursor::new(&source)).unwrap();
    let mut after = ZipArchive::new(Cursor::new(&output)).unwrap();
    let before = before.by_name("custom/opaque.bin").unwrap();
    let after = after.by_name("custom/opaque.bin").unwrap();
    assert_eq!(after.extra_data(), before.extra_data());
    assert_eq!(after.comment(), "original member");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn property_preserved_members_overrides_additions_and_tombstones(
        initial in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..2048), 0..8),
        changes in prop::collection::vec((0usize..12, prop::collection::vec(any::<u8>(), 0..2048)), 0..8),
        deleted in prop::collection::vec(0usize..12, 0..8),
        stored in any::<bool>(),
    ) {
        let key = |index| format!("custom/part{index}.bin");
        let original: HashMap<_, _> = initial.into_iter().enumerate()
            .map(|(index, bytes)| (key(index), bytes)).collect();
        let overrides: HashMap<_, _> = changes.into_iter()
            .map(|(index, bytes)| (key(index), bytes)).collect();
        let removed: HashSet<_> = deleted.into_iter().map(key).collect();
        let source = source(&original, stored);
        let output = write_docx_preserving_archive(&source, &overrides, &removed).unwrap();
        let mut expected = original.clone();
        expected.extend(overrides.clone());
        expected.retain(|name, _| !removed.contains(name));
        prop_assert_eq!(inflated(&output), expected);
        for name in original.keys().filter(|name| !overrides.contains_key(*name) && !removed.contains(*name)) {
            prop_assert_eq!(raw_member(&output, name), raw_member(&source, name));
        }
        let archive = ZipArchive::new(Cursor::new(&output)).unwrap();
        prop_assert_eq!(archive.comment(), b"source package comment");
    }
}

#[test]
fn repairs_untouched_and_overridden_word_xml_instead_of_raw_copying_it() {
    let invalid = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="w14"><w:body/></w:document>"#.to_vec();
    let source = source(
        &HashMap::from([
            ("word/document.xml".into(), invalid.clone()),
            ("word/header1.xml".into(), invalid.clone()),
            ("custom/opaque.bin".into(), vec![29; 65_536]),
        ]),
        true,
    );
    let output = write_docx_preserving_archive(
        &source,
        &HashMap::from([("word/header1.xml".into(), invalid)]),
        &HashSet::new(),
    )
    .unwrap();
    let files = inflated(&output);
    for name in ["word/document.xml", "word/header1.xml"] {
        let xml = std::str::from_utf8(&files[name]).unwrap();
        assert!(
            xml.contains("xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\""),
            "{name}: {xml}"
        );
    }
    assert_eq!(
        raw_member(&source, "custom/opaque.bin"),
        raw_member(&output, "custom/opaque.bin")
    );
}
