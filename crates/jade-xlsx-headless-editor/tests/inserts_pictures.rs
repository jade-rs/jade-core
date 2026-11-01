//! Insert ▸ Pictures: an image from a file on a sheet, at its own size from
//! the active cell, kept as Excel keeps one.

use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRef, ShapeKind};

/// A 4 × 3 red PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 4, 0, 0, 0, 3, 8, 2, 0,
    0, 0, 59, 150, 57, 145, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 71, 12,
    56, 57, 0, 245, 49, 11, 245, 53, 123, 251, 130, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn open(name: &str) -> HeadlessWorkbook {
    let path = format!(
        "{}/../../public/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    HeadlessWorkbook::open(&std::fs::read(&path).unwrap()).expect("open")
}

fn pictures(workbook: &HeadlessWorkbook, sheet: usize) -> Vec<jade_xlsx_reader::Shape> {
    workbook.workbook().sheets[sheet]
        .shapes
        .iter()
        .filter(|shape| shape.kind == ShapeKind::Picture)
        .cloned()
        .collect()
}

#[test]
fn a_picture_goes_on_a_sheet_without_a_drawing_and_survives_saving() {
    let mut workbook = open("sample-workbook.xlsx");
    let sheet = workbook.workbook().sheets[0].name.clone();
    assert!(pictures(&workbook, 0).is_empty());
    let id = workbook
        .insert_picture(&sheet, CellRef::new(2, 3), PNG, (120.0, 90.0))
        .expect("insert");
    let reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    let placed = pictures(&reopened, 0);
    assert_eq!(placed.len(), 1);
    let picture = &placed[0];
    assert_eq!(picture.id, id);
    assert_eq!(picture.name, "Picture 1");
    let media = picture
        .picture
        .as_ref()
        .and_then(|p| p.media.as_ref())
        .expect("the image");
    assert_eq!(media.bytes.as_slice(), PNG);
    assert!(workbook.undo().expect("undo"));
    assert!(pictures(&workbook, 0).is_empty());
}

#[test]
fn a_picture_joins_a_drawing_that_has_charts_and_pictures_already() {
    let mut workbook = open("pictures-workbook.xlsx");
    let sheet = workbook.workbook().sheets[0].name.clone();
    let before = workbook.workbook().sheets[0].shapes.clone();
    let id = workbook
        .insert_picture(&sheet, CellRef::new(0, 0), PNG, (40.0, 30.0))
        .expect("insert");
    let after = &workbook.workbook().sheets[0].shapes;
    assert_eq!(after.len(), before.len() + 1);
    assert!(before.iter().all(|shape| shape.id < id));
    // What was there stays as it was.
    for shape in &before {
        assert!(after
            .iter()
            .any(|other| other.id == shape.id && other.name == shape.name));
    }
}

#[test]
fn only_pictures_go_in() {
    let mut workbook = open("sample-workbook.xlsx");
    let sheet = workbook.workbook().sheets[0].name.clone();
    assert!(workbook
        .insert_picture(&sheet, CellRef::new(0, 0), b"<svg/>", (10.0, 10.0))
        .is_err());
}

#[test]
fn shapes_go_in_filled_by_the_theme_and_named_as_excel_names_them() {
    let mut workbook = open("sample-workbook.xlsx");
    let sheet = workbook.workbook().sheets[0].name.clone();
    let rect = workbook
        .insert_shape(&sheet, CellRef::new(1, 1), "rect", (144.0, 96.0))
        .expect("rectangle");
    let oval = workbook
        .insert_shape(&sheet, CellRef::new(4, 1), "ellipse", (96.0, 96.0))
        .expect("oval");
    let reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    let shapes = &reopened.workbook().sheets[0].shapes;
    let named = |id: u32| shapes.iter().find(|shape| shape.id == id).expect("placed");
    assert_eq!(named(rect).name, format!("Rectangle {rect}"));
    assert_eq!(named(oval).name, format!("Oval {oval}"));
    let style = named(oval).style.as_ref().expect("its geometry");
    assert_eq!(style.geometry.as_deref(), Some("ellipse"));
    assert!(workbook
        .insert_shape(&sheet, CellRef::new(0, 0), "star", (10.0, 10.0))
        .is_err());
    assert!(workbook.undo().expect("undo"));
    assert_eq!(workbook.workbook().sheets[0].shapes.len(), shapes.len() - 1);
}

#[test]
fn lines_go_in_as_connectors_and_the_gallery_reads_back_as_inserted() {
    use jade_xlsx_reader::ShapeKind;
    let mut workbook = open("sample-workbook.xlsx");
    let sheet = workbook.workbook().sheets[0].name.clone();
    let ids: Vec<(u32, &str, &str)> = jade_xlsx_writer::shapes::SHAPES
        .iter()
        .enumerate()
        .map(|(index, (preset, name))| {
            let at = CellRef::new(1 + (index as u32 % 4) * 3, 1 + (index as u32 / 4) * 6);
            let id = workbook
                .insert_shape(&sheet, at, preset, (144.0, 96.0))
                .expect(preset);
            (id, *preset, *name)
        })
        .collect();
    let reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    let shapes = &reopened.workbook().sheets[0].shapes;
    for (id, preset, name) in ids {
        let shape = shapes.iter().find(|shape| shape.id == id).expect(preset);
        assert_eq!(shape.name, format!("{name} {id}"));
        let style = shape.style.as_ref().expect(preset);
        assert_eq!(style.geometry.as_deref(), Some(preset));
        let line = jade_xlsx_writer::shapes::is_line(preset);
        assert_eq!(shape.kind == ShapeKind::Connector, line, "{preset}");
        if line {
            let arrow = (preset == "straightConnector1").then(|| "triangle".to_string());
            assert_eq!(style.tail_end, arrow, "{preset}");
        }
    }
}
