use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, Axis, CellRef, StructureChange};
use jade_xlsx_writer::{restructure, write_xlsx, CellEdit, NewValue, SheetEdits};
const ORIGINAL: &[u8] = include_bytes!("../../../public/fixtures/pictures-workbook.xlsx");
#[test]
fn editing_cells_preserves_picture_bytes_relationships_and_all_transforms() {
    let written = write_xlsx(
        ORIGINAL,
        &[SheetEdits {
            part_name: "xl/worksheets/sheet1.xml".into(),
            cells: vec![CellEdit {
                reference: CellRef::new(0, 0),
                value: NewValue::Number(42.0),
                style_index: None,
            }],
        }],
    )
    .unwrap();
    let original = OpcPackage::open(ORIGINAL).unwrap();
    let edited = OpcPackage::open(&written).unwrap();
    for name in [
        "xl/drawings/drawing1.xml",
        "xl/drawings/_rels/drawing1.xml.rels",
        "xl/media/quadrants.png",
        "xl/media/vector",
    ] {
        assert_eq!(original.part(name), edited.part(name), "{name}");
    }
    assert_eq!(
        read_xlsx(ORIGINAL).unwrap().sheets[0].shapes,
        read_xlsx(&written).unwrap().sheets[0].shapes
    );
}
#[test]
fn grid_structure_moves_cell_anchors_and_preserves_absolute_pictures() {
    let before = read_xlsx(ORIGINAL).unwrap();
    let written = restructure(
        ORIGINAL,
        &before,
        &StructureChange {
            sheet: "Pictures".into(),
            axis: Axis::Rows,
            at: 0,
            count: 2,
            inserted: true,
        },
    )
    .unwrap();
    let after = read_xlsx(&written).unwrap();
    for shape in &before.sheets[0].shapes {
        let next = after.sheets[0]
            .shapes
            .iter()
            .find(|next| next.name == shape.name)
            .unwrap();
        if shape.name == "Original" || shape.name == "Crop" {
            assert_eq!(next.from.row, shape.from.row + 2);
        } else {
            assert_eq!(next, shape);
        }
        assert_eq!(next.picture, shape.picture);
    }
}
