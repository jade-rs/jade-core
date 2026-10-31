//! Typing packs the sheets it writes at zlib's fastest level, since the
//! package is written again on every entry; a save packs them as well as
//! any other write, and a workbook undone back to what was opened saves the
//! opened bytes.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/large-workbook.xlsx"
    ))
    .unwrap()
}

/// Each entry's name and compressed size.
fn sizes(bytes: &[u8]) -> Vec<(String, u64)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let entry = archive.by_index_raw(index).unwrap();
            (entry.name().unwrap().into_owned(), entry.compressed_size())
        })
        .collect()
}

#[test]
fn a_save_repacks_what_typing_packed_quickly() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.set(&sheet, "B2", CellInput::Number(1234.0)).unwrap();
    book.set(&sheet, "B3", CellInput::Text("typed".into()))
        .unwrap();
    let in_memory = sizes(book.bytes());
    let saved = book.save_bytes().unwrap();
    let packed = sizes(&saved);
    let part = &book.workbook().sheets[0].part_name;
    let size_of = |entries: &[(String, u64)]| {
        entries
            .iter()
            .find(|(name, _)| name == part)
            .map(|(_, size)| *size)
            .unwrap()
    };
    assert!(
        size_of(&packed) < size_of(&in_memory),
        "{} < {}",
        size_of(&packed),
        size_of(&in_memory)
    );
    // Only how it is packed differs.
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.workbook(), book.workbook());
}

#[test]
fn undone_back_to_the_opened_file_it_saves_the_opened_bytes() {
    let opened = fixture();
    let mut book = HeadlessWorkbook::open_for_editing(&opened).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    let start = book.bytes().to_vec();
    book.set(&sheet, "B2", CellInput::Number(1234.0)).unwrap();
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), start.as_slice());
    assert_eq!(book.save_bytes().unwrap(), start);
}
