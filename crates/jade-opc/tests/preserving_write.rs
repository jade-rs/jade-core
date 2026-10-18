use jade_opc::OpcPackage;
use std::io::{Cursor, Read, Write};
fn archive(method: zip::CompressionMethod) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(method)
        .last_modified_time(zip::DateTime::from_date_and_time(2020, 2, 3, 4, 5, 6).unwrap())
        .unix_permissions(0o644);
    writer.start_file("slide.xml", options).unwrap();
    writer.write_all(b"<text>original</text>").unwrap();
    writer.start_file("media.bin", options).unwrap();
    writer.write_all(&vec![17; 512 * 1024]).unwrap();
    writer.set_comment("archive comment").unwrap();
    writer.finish().unwrap().into_inner()
}
fn raw(
    bytes: &[u8],
    name: &str,
) -> (
    Vec<u8>,
    zip::CompressionMethod,
    u32,
    Option<zip::DateTime>,
    Option<u32>,
) {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let file = zip.by_name(name).unwrap();
    let start = file.data_start().unwrap() as usize;
    let end = start + file.compressed_size() as usize;
    (
        bytes[start..end].to_vec(),
        file.compression(),
        file.crc32(),
        file.last_modified(),
        file.unix_mode(),
    )
}
#[test]
fn unchanged_stored_and_deflated_payloads_keep_compression_crc_timestamp_permissions_and_comment() {
    for method in [
        zip::CompressionMethod::Stored,
        zip::CompressionMethod::Deflated,
    ] {
        let bytes = archive(method);
        let mut package = OpcPackage::open_preserving(&bytes).unwrap();
        package.replace("slide.xml", b"<text>edited</text>".to_vec());
        let (saved, stats) = package.write_with_stats().unwrap();
        assert_eq!(stats.copied_parts, 1);
        assert_eq!(stats.recompressed_parts, 1);
        assert_eq!(
            stats.recompressed_source_bytes,
            b"<text>edited</text>".len()
        );
        assert_eq!(
            stats.copied_compressed_bytes,
            raw(&bytes, "media.bin").0.len() as u64
        );
        assert_eq!(raw(&bytes, "media.bin"), raw(&saved, "media.bin"));
        let mut zip = zip::ZipArchive::new(Cursor::new(&saved)).unwrap();
        assert_eq!(zip.comment(), b"archive comment");
        let mut text = String::new();
        zip.by_name("slide.xml")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "<text>edited</text>");
    }
}
#[test]
fn noops_keep_every_byte_and_removal_reinsertion_and_new_parts_are_written() {
    let bytes = archive(zip::CompressionMethod::Stored);
    let mut package = OpcPackage::open_preserving(&bytes).unwrap();
    assert_eq!(package.write().unwrap(), bytes);
    package.replace("slide.xml", b"<text>original</text>".to_vec());
    assert!(!package.remove("absent"));
    assert_eq!(package.write().unwrap(), bytes);
    assert!(package.remove("media.bin"));
    package.replace("media.bin", b"replacement".to_vec());
    package.replace("new.xml", b"<new/>".to_vec());
    let saved = package.write().unwrap();
    let saved = OpcPackage::open(&saved).unwrap();
    assert_eq!(saved.part("media.bin"), Some(b"replacement".as_slice()));
    assert_eq!(saved.part_names(), &["slide.xml", "media.bin", "new.xml"]);
    assert_eq!(saved.part("new.xml"), Some(b"<new/>".as_slice()));
}
#[test]
fn dereferenced_mutations_remain_tracked_and_corrupt_originals_are_rejected() {
    fn edit(package: &mut OpcPackage) {
        package.replace("slide.xml", b"changed".to_vec());
    }
    let bytes = archive(zip::CompressionMethod::Stored);
    let mut package = OpcPackage::open_preserving(&bytes).unwrap();
    edit(&mut package);
    assert_eq!(
        OpcPackage::open(&package.write().unwrap())
            .unwrap()
            .part("slide.xml"),
        Some(b"changed".as_slice())
    );
    assert!(OpcPackage::open_preserving(b"invalid").is_err());
    let mut replacement = OpcPackage::open(&bytes).unwrap();
    replacement.replace("slide.xml", b"other package".to_vec());
    let other = replacement.write().unwrap();
    *package = OpcPackage::open(&other).unwrap();
    assert_eq!(
        OpcPackage::open(&package.write().unwrap())
            .unwrap()
            .part("slide.xml"),
        Some(b"other package".as_slice())
    );
}
