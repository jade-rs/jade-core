//! Shapes carry their preset geometry, fill, outline and text format.
mod common;

use common::{styles, workbook, SheetPart};
use jade_xlsx_layout::{
    layout_workbook_sheet,
    shape_style::{PathOp, PlacedShapeStyle},
    LayoutOptions,
};
use jade_xlsx_reader::read_xlsx;

const DEFAULT_STYLE: &str = r#"<xdr:style><a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="50000"/></a:schemeClr></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></xdr:style>"#;

fn anchor(row: u32, object: &str) -> String {
    format!(
        r#"<xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>{object}<xdr:clientData/></xdr:twoCellAnchor>"#,
        row + 4
    )
}

fn shape(id: u32, geometry: &str, properties: &str, style: &str, text: &str) -> String {
    format!(
        r#"<xdr:sp><xdr:nvSpPr><xdr:cNvPr id="{id}" name="S{id}"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr><a:xfrm rot="5400000"><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></a:xfrm><a:prstGeom prst="{geometry}"><a:avLst/></a:prstGeom>{properties}</xdr:spPr>{style}<xdr:txBody><a:bodyPr anchor="ctr"/><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="1400" b="1"/><a:t>{text}</a:t></a:r></a:p></xdr:txBody></xdr:sp>"#
    )
}

fn placed() -> Vec<PlacedShapeStyle> {
    let drawing = format!(
        r#"<?xml version="1.0"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">{}{}{}{}</xdr:wsDr>"#,
        anchor(0, &shape(2, "ellipse", "", DEFAULT_STYLE, "Hello")),
        anchor(
            5,
            &shape(
                3,
                "rightArrow",
                r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln w="38100"><a:noFill/></a:ln>"#,
                DEFAULT_STYLE,
                "Go"
            )
        ),
        anchor(10, &shape(4, "rect", "", "", "Bare")),
        anchor(
            15,
            r#"<xdr:cxnSp><xdr:nvCxnSpPr><xdr:cNvPr id="5" name="C"/><xdr:cNvCxnSpPr/></xdr:nvCxnSpPr><xdr:spPr><a:xfrm flipH="1"><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></a:xfrm><a:prstGeom prst="straightConnector1"><a:avLst/></a:prstGeom><a:ln w="19050"><a:solidFill><a:srgbClr val="00B050"/></a:solidFill><a:prstDash val="dash"/><a:tailEnd type="triangle"/></a:ln></xdr:spPr></xdr:cxnSp>"#
        ),
    );
    let sheet = SheetPart::new("Sheet1", r#"<sheetData/><drawing r:id="rId1"/>"#).rel(
        "rId1",
        "drawing",
        "../drawings/drawing1.xml",
    );
    let book = read_xlsx(&workbook(
        vec![sheet],
        &styles("", "", ""),
        &[("xl/drawings/drawing1.xml", &drawing)],
    ))
    .unwrap();
    let layout = layout_workbook_sheet(&book, 0, &LayoutOptions::default()).unwrap();
    assert_eq!(layout.shapes[0].font.size_points, 14.0);
    assert!(layout.shapes[0].font.bold);
    layout
        .shapes
        .iter()
        .map(|shape| shape.style.clone().expect("a shape has a style"))
        .collect()
}

#[test]
fn the_shape_style_fills_and_outlines_a_shape_the_file_leaves_unstyled() {
    let shapes = placed();
    let ellipse = &shapes[0];
    assert_eq!(ellipse.fill, Some([68, 114, 196, 255]));
    let stroke = ellipse.stroke.unwrap();
    assert!(stroke[2] < 196 && stroke[2] > 60, "{stroke:?}");
    // lnRef 2 is the theme's 1 pt line.
    assert!((ellipse.stroke_width - 12_700.0 / 9_525.0).abs() < 1e-9);
    assert!(ellipse
        .path
        .iter()
        .any(|op| matches!(op, PathOp::Cubic(..))));
    assert_eq!(ellipse.text_color, [255, 255, 255, 255]);
    assert_eq!(
        (ellipse.text_align.as_str(), ellipse.text_anchor.as_str()),
        ("ctr", "ctr")
    );
    assert!((ellipse.rotation_radians - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
}

#[test]
fn sppr_overrides_the_style_and_presets_have_their_outline() {
    let shapes = placed();
    let arrow = &shapes[1];
    assert_eq!(arrow.fill, Some([255, 0, 0, 255]));
    assert_eq!(arrow.stroke, None);
    let corners = arrow
        .path
        .iter()
        .filter(|op| matches!(op, PathOp::Line(..)))
        .count();
    assert_eq!(corners, 6);
    // No fill, outline or style at all: nothing but the text.
    let bare = &shapes[2];
    assert_eq!((bare.fill, bare.stroke), (None, None));
    assert_eq!(bare.text_color, [0, 0, 0, 255]);
}

#[test]
fn connectors_are_open_lines_with_dashes_and_arrowheads() {
    let shapes = placed();
    let line = &shapes[3];
    assert_eq!(line.fill, None);
    assert_eq!(line.stroke, Some([0, 176, 80, 255]));
    assert!(line.flip_h);
    assert!(!line.path.iter().any(|op| matches!(op, PathOp::Close)));
    assert_eq!(line.dash, vec![8.0, 6.0]);
    assert_eq!(line.arrows.len(), 1);
    assert_eq!(line.arrows[0].len(), 3);
}
