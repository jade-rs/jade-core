use jade_xlsx_reader::{read_xlsx, ShapeKind};
use std::rc::Rc;

#[test]
fn embedded_media_relationships_and_declared_content_types_are_resolved_once() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/pictures-workbook.xlsx"
    ))
    .unwrap();
    let shapes = &book.sheets[0].shapes;
    assert_eq!(shapes.len(), 12);
    assert!(shapes.iter().all(|shape| shape.kind == ShapeKind::Picture));
    let picture = |name: &str| {
        shapes
            .iter()
            .find(|shape| shape.name == name)
            .unwrap()
            .picture
            .as_ref()
            .unwrap()
    };
    let png = picture("Original").media.as_ref().unwrap();
    assert_eq!(png.content_type, "image/png");
    assert!(png.bytes.starts_with(b"\x89PNG"));
    assert!(Rc::ptr_eq(
        &png.bytes,
        &picture("Crop").media.as_ref().unwrap().bytes
    ));
    assert_eq!(
        picture("SVG").media.as_ref().unwrap().content_type,
        "image/svg+xml"
    );
    assert_eq!(picture("Crop").crop, [50000, 0, 0, 0]);
    assert_eq!(picture("Outset").crop, [-50000, 0, 0, 0]);
    assert!(picture("Flip").flip_h);
    assert_eq!(picture("Rotate").rotation, 5_400_000);
    assert_eq!(picture("Opacity").opacity, 50_000);
    assert!(picture("Missing").media.is_none());
    assert_eq!(picture("Tiled").unsupported_properties, ["tile"]);
    assert!(picture("Linked").media.is_none());
    assert_eq!(
        picture("Linked").external_target.as_deref(),
        Some("https://example.invalid/image.png")
    );
    assert_eq!(
        shapes
            .iter()
            .find(|shape| shape.name == "Far")
            .unwrap()
            .from_offset,
        (900 * 9525, 900 * 9525)
    );
}

#[test]
fn destination_insets_and_unsupported_effects_are_retained_without_inventing_support() {
    let shapes=jade_xlsx_reader::drawing::parse_drawing(r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:absoluteAnchor><xdr:pos x="0" y="0"/><xdr:ext cx="952500" cy="952500"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="1" name="Effects"/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="image" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><a:grayscl/></a:blip><a:stretch><a:fillRect l="10000" t="20000" r="10000" b="0"/></a:stretch><a:tile/></xdr:blipFill><xdr:spPr><a:prstGeom prst="roundRect"/><a:effectLst/></xdr:spPr></xdr:pic></xdr:absoluteAnchor></xdr:wsDr>"#).unwrap();
    let picture = shapes[0].picture.as_ref().unwrap();
    assert_eq!(picture.fill, [10000, 20000, 10000, 0]);
    assert_eq!(
        picture.unsupported_properties,
        ["grayscl", "tile", "geometry: roundRect"]
    );
}
