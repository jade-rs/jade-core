//! A tagged PDF (Word's ドキュメント構造タグ): the structure tree with
//! headings, paragraphs, tables and figures in reading order, marked content
//! on the pages, bookmarks from the headings, and the document's title.

mod common;

use lopdf::{Document, Object};

fn tagged(name: &str) -> Document {
    let document = common::open_fixture(name);
    let bytes = document
        .export_pdf_with_options(jade_docx_native_render::PdfExportOptions {
            tagged: true,
            ..Default::default()
        })
        .unwrap();
    Document::load_mem(&bytes).unwrap()
}

/// Every structure type in the tree, depth first, with figures' alt text.
fn structure(pdf: &Document) -> Vec<(String, Option<String>)> {
    fn walk(pdf: &Document, object: &Object, out: &mut Vec<(String, Option<String>)>) {
        let object = match object {
            Object::Reference(id) => match pdf.get_object(*id) {
                Ok(object) => object,
                Err(_) => return,
            },
            other => other,
        };
        match object {
            Object::Array(items) => {
                for item in items {
                    walk(pdf, item, out);
                }
            }
            Object::Dictionary(dictionary) => {
                if let Ok(kind) = dictionary.get(b"S").and_then(Object::as_name) {
                    let alt = dictionary
                        .get(b"Alt")
                        .ok()
                        .and_then(|alt| alt.as_str().ok())
                        .map(|alt| String::from_utf8_lossy(alt).into_owned());
                    out.push((String::from_utf8_lossy(kind).into_owned(), alt));
                }
                if let Ok(children) = dictionary.get(b"K") {
                    walk(pdf, children, out);
                }
            }
            _ => {}
        }
    }
    let catalog = pdf.catalog().unwrap();
    let root = catalog.get(b"StructTreeRoot").expect("a structure tree");
    let mut out = Vec::new();
    walk(pdf, root, &mut out);
    out
}

fn kinds(structure: &[(String, Option<String>)]) -> Vec<&str> {
    structure.iter().map(|(kind, _)| kind.as_str()).collect()
}

#[test]
fn headings_are_tagged_and_become_bookmarks() {
    let pdf = tagged("public/fixtures/toc.docx");
    let structure = structure(&pdf);
    let kinds = kinds(&structure);
    assert!(kinds.contains(&"Document"));
    assert!(kinds.iter().any(|kind| kind.starts_with('H')), "{kinds:?}");
    assert!(kinds.contains(&"P"));
    let catalog = pdf.catalog().unwrap();
    assert!(
        catalog.get(b"Outlines").is_ok(),
        "the headings are bookmarks"
    );
    let marked = catalog
        .get(b"MarkInfo")
        .and_then(|info| match info {
            Object::Reference(id) => pdf.get_object(*id),
            other => Ok(other),
        })
        .and_then(Object::as_dict)
        .and_then(|info| info.get(b"Marked"))
        .and_then(Object::as_bool)
        .unwrap_or(false);
    assert!(marked);
    // The pages' drawing is marked with the nodes.
    let page = *pdf.get_pages().values().next().unwrap();
    let content = String::from_utf8_lossy(&pdf.get_page_content(page).unwrap()).into_owned();
    assert!(content.contains("BDC") && content.contains("MCID"));
}

#[test]
fn tables_are_tagged_by_row_and_cell() {
    let pdf = tagged("public/fixtures/table3x2merged.docx");
    let structure = structure(&pdf);
    let kinds = kinds(&structure);
    assert!(kinds.contains(&"Table"), "{kinds:?}");
    assert!(kinds.contains(&"TR"));
    // A cell with nothing in it marks no content, and the PDF keeps no node
    // for it: the two with text are cells of two rows.
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "TD").count(),
        2,
        "{kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "TR").count(),
        2,
        "{kinds:?}"
    );
}

#[test]
fn pictures_are_figures() {
    let pdf = tagged("public/fixtures/image-inline.docx");
    let structure = structure(&pdf);
    assert!(kinds(&structure).contains(&"Figure"), "{structure:?}");
}

#[test]
fn an_untagged_export_has_no_structure() {
    let document = common::open_fixture("public/fixtures/toc.docx");
    let bytes = document.export_pdf().unwrap();
    let pdf = Document::load_mem(&bytes).unwrap();
    assert!(pdf.catalog().unwrap().get(b"StructTreeRoot").is_err());
}
