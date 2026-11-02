//! Replying to a threaded comment: the reply joins the thread, its author
//! found in (or added to) the persons part, and the cell's comment reads it.

use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::CellRef;

const THREADS: &str = r#"<ThreadedComments xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"><threadedComment ref="B2" dT="2026-11-01T10:00:00.00" personId="{P1}" id="{A}"><text>Check this</text></threadedComment></ThreadedComments>"#;
const PERSONS: &str = r#"<personList xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"><person displayName="Ann Lee" id="{P1}" userId="ann" providerId="None"/></personList>"#;

fn workbook() -> HeadlessWorkbook {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    let book = jade_xlsx_reader::read_xlsx(&bytes).unwrap();
    let sheet = book.sheets[0].part_name.clone();
    let rels = jade_opc::rels_part_name(&sheet);
    let target = "../threadedComments/threadedComment1.xml";
    let xml = match package.part_str(&rels).unwrap() {
        Some(xml) => xml.replace(
            "</Relationships>",
            &format!(r#"<Relationship Id="rIdThread" Type="http://schemas.microsoft.com/office/2017/10/relationships/threadedComment" Target="{target}"/></Relationships>"#),
        ),
        None => format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdThread" Type="http://schemas.microsoft.com/office/2017/10/relationships/threadedComment" Target="{target}"/></Relationships>"#),
    };
    package.replace(&rels, xml.into_bytes());
    package.replace(
        "xl/threadedComments/threadedComment1.xml",
        THREADS.as_bytes().to_vec(),
    );
    package.replace("xl/persons/person.xml", PERSONS.as_bytes().to_vec());
    HeadlessWorkbook::open(&package.write().unwrap()).unwrap()
}

#[test]
fn replies_join_the_thread() {
    let mut book = workbook();
    let sheet = book.workbook().sheets[0].name.clone();
    let comment = |book: &HeadlessWorkbook| {
        book.workbook().sheets[0]
            .comment_at(CellRef::parse("B2").unwrap())
            .map(|comment| comment.text.clone())
    };
    assert_eq!(comment(&book).as_deref(), Some("Ann Lee: Check this"));
    assert!(book
        .reply_to_comment(&sheet, "B2", "Ann Lee", "Done", "2026-11-02T09:00:00")
        .unwrap());
    assert!(book
        .reply_to_comment(
            &sheet,
            "B2",
            "Bob Ito",
            "Thanks & ok",
            "2026-11-02T09:30:00"
        )
        .unwrap());
    let reopened = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert_eq!(
        comment(&reopened).as_deref(),
        Some("Ann Lee: Check this\nAnn Lee: Done\nBob Ito: Thanks & ok")
    );
    let persons = OpcPackage::open(book.bytes())
        .unwrap()
        .part_str("xl/persons/person.xml")
        .unwrap()
        .unwrap();
    assert_eq!(persons.matches("<person ").count(), 2, "{persons}");
    // A cell without a thread has nothing to reply to.
    assert!(!book
        .reply_to_comment(&sheet, "C3", "Ann Lee", "?", "2026-11-02T10:00:00")
        .unwrap());
    assert!(book.undo().unwrap());
    assert_eq!(
        comment(&book).as_deref(),
        Some("Ann Lee: Check this\nAnn Lee: Done")
    );
}
