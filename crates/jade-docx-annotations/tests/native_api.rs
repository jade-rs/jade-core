use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

#[test]
fn public_api_transforms_docx_from_a_native_caller() {
    let mut package = ZipWriter::new(Cursor::new(Vec::new()));
    package
        .start_file("word/document.xml", SimpleFileOptions::default())
        .unwrap();
    package
        .write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:ins w:id="1"><w:r><w:t>current</w:t></w:r></w:ins><w:del w:id="2"><w:r><w:delText>old</w:delText></w:r></w:del></w:p></w:body></w:document>"#)
        .unwrap();
    let source = package.finish().unwrap().into_inner();

    let clean_copy = futures::executor::block_on(
        jade_docx_annotations::transform_docx_annotations(&source, true, false),
    )
    .unwrap();
    let mut result = ZipArchive::new(Cursor::new(clean_copy)).unwrap();
    let mut document = String::new();
    result
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut document)
        .unwrap();
    assert!(document.contains("current"));
    assert!(!document.contains("old"));
    assert!(!document.contains("<w:ins"));
    assert!(!document.contains("<w:del"));
}
