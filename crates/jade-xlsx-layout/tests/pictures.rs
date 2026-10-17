use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::read_xlsx;
fn layout() -> SheetLayout {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/pictures-workbook.xlsx"
    ))
    .unwrap();
    layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default())
}
#[test]
fn crop_and_negative_crop_share_source_and_destination_geometry() {
    let layout = layout();
    let crop = layout
        .shapes
        .iter()
        .find(|shape| shape.name == "Crop")
        .unwrap();
    assert_eq!(
        crop.rect,
        Rect {
            x: 120.0,
            y: 20.0,
            width: 80.0,
            height: 60.0
        }
    );
    let picture = crop.picture.as_ref().unwrap();
    assert_eq!(picture.source, [0.5, 0.0, 0.5, 1.0]);
    assert_eq!(
        picture.image_rect,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 60.0
        }
    );
    let outset = layout
        .shapes
        .iter()
        .find(|shape| shape.name == "Outset")
        .unwrap()
        .picture
        .as_ref()
        .unwrap();
    assert_eq!(outset.source, [0.0, 0.0, 1.0, 1.0]);
    assert!((outset.image_rect.x - 80.0 / 3.0).abs() < 1e-9);
    assert!((outset.image_rect.width - 160.0 / 3.0).abs() < 1e-9);
}
#[test]
fn rotated_visible_bounds_hit_testing_and_absolute_scroll_extent_are_consistent() {
    let layout = layout();
    let shape = layout
        .shapes
        .iter()
        .find(|shape| shape.name == "Rotate")
        .unwrap();
    let bound = shape.paint_rect();
    assert!((bound.x - 330.0).abs() < 1e-9);
    assert!((bound.y - 10.0).abs() < 1e-9);
    assert!((bound.width - 60.0).abs() < 1e-9);
    assert!((bound.height - 80.0).abs() < 1e-9);
    assert_eq!(layout.shape_at(340.0, 15.0).unwrap().name, "Rotate");
    assert!(layout.shape_at(325.0, 50.0).is_none());
    assert!(layout
        .shapes_in(Rect {
            x: 335.0,
            y: 10.0,
            width: 10.0,
            height: 8.0
        })
        .any(|shape| shape.name == "Rotate"));
    assert!(layout.content_width >= 980.0);
    assert!(layout.content_height >= 960.0);
    assert!(layout.cell_at(970.0, 950.0).is_some());
}

#[test]
fn stretch_destination_insets_use_the_anchor_size_and_unsupported_effects_are_not_painted() {
    let mut book = read_xlsx(include_bytes!(
        "../../../public/fixtures/pictures-workbook.xlsx"
    ))
    .unwrap();
    let picture = book.sheets[0].shapes[0].picture.as_mut().unwrap();
    picture.fill = [10000, 20000, 10000, 0];
    picture.unsupported_properties.push("tile".into());
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let picture = layout.shapes[0].picture.as_ref().unwrap();
    assert_eq!(
        picture.image_rect,
        Rect {
            x: 8.0,
            y: 12.0,
            width: 64.0,
            height: 48.0
        }
    );
    assert!(!picture.supported);
}
