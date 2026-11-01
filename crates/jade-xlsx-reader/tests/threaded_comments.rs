//! Current Excel's threaded comments: each cell's comment and its replies
//! read in place of the note Excel writes beside them for older versions.

mod common;

use jade_xlsx_reader::{read_xlsx, CellRef, OpcPackage};

const RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2017/10/relationships/threadedComment" Target="../threadedComments/threadedComment1.xml"/></Relationships>"#;

const NOTES: &str = r#"<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>tc={A}</author></authors><commentList><comment ref="A1" authorId="0"><text><t>[Threaded comment]

Your version of Excel allows you to read this threaded comment; however, any edits to it will get removed if the file is opened in a newer version of Excel.

Comment:
    Check this</t></text></comment></commentList></comments>"#;

const THREADS: &str = r#"<ThreadedComments xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"><threadedComment ref="A1" dT="2026-11-01T10:00:00.00" personId="{P1}" id="{A}"><text>Check this</text></threadedComment><threadedComment ref="A1" dT="2026-11-01T11:00:00.00" personId="{P2}" id="{B}" parentId="{A}"><text>Done &amp; checked</text></threadedComment><threadedComment ref="B2" dT="2026-11-01T12:00:00.00" personId="{P2}" id="{C}"><text>Only in the thread</text></threadedComment></ThreadedComments>"#;

const PERSONS: &str = r#"<personList xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"><person displayName="Ann Lee" id="{P1}" userId="ann" providerId="None"/><person displayName="Bob Ito" id="{P2}" userId="bob" providerId="None"/></personList>"#;

#[test]
fn threads_are_read_in_place_of_their_placeholder_notes() {
    let mut package = OpcPackage::open(&common::sample_workbook()).unwrap();
    package.replace(
        "xl/worksheets/_rels/sheet1.xml.rels",
        RELS.as_bytes().to_vec(),
    );
    package.replace("xl/comments1.xml", NOTES.as_bytes().to_vec());
    package.replace(
        "xl/threadedComments/threadedComment1.xml",
        THREADS.as_bytes().to_vec(),
    );
    package.replace("xl/persons/person.xml", PERSONS.as_bytes().to_vec());
    let book = read_xlsx(&package.write().unwrap()).unwrap();
    let comments = &book.sheets[0].comments;
    let at = |reference: &str| {
        let reference = CellRef::parse(reference).unwrap();
        comments
            .iter()
            .find(|comment| comment.reference == reference)
            .expect(reference.column.to_string().as_str())
    };
    assert_eq!(at("A1").author.as_deref(), Some("Ann Lee"));
    assert_eq!(
        at("A1").text,
        "Ann Lee: Check this\nBob Ito: Done & checked"
    );
    assert_eq!(at("B2").text, "Bob Ito: Only in the thread");
    assert_eq!(comments.len(), 2);
}
