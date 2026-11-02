//! Insert ▸ Shapes: a line, a rectangle, an oval, an arrow or another of
//! Excel's gallery put on a sheet as a new object in its drawing, styled by
//! the theme's first accent as Excel draws a new one — a shape filled and
//! ready for text, a line as a connector.

use jade_opc::OpcPackage;
use jade_xlsx_reader::CellRef;

use crate::charts::{append_to_drawing, drawing_for};
use crate::patch::escape;
use crate::{Result, WriteError};

const DRAWINGML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// English Metric Units in a CSS px at 96 an inch.
const EMU_PER_PX: f64 = 9525.0;

/// The preset geometries Insert ▸ Shapes offers, each with the name Excel
/// gives a new one, in the order of Excel's gallery: lines, rectangles,
/// basic shapes, block arrows, stars and callouts.
pub const SHAPES: [(&str, &str); 28] = [
    ("line", "Straight Connector"),
    ("straightConnector1", "Straight Arrow Connector"),
    ("bentConnector3", "Connector: Elbow"),
    ("curvedConnector3", "Connector: Curved"),
    ("rect", "Rectangle"),
    ("roundRect", "Rectangle: Rounded Corners"),
    ("ellipse", "Oval"),
    ("triangle", "Isosceles Triangle"),
    ("rtTriangle", "Right Triangle"),
    ("parallelogram", "Parallelogram"),
    ("trapezoid", "Trapezoid"),
    ("diamond", "Diamond"),
    ("pentagon", "Regular Pentagon"),
    ("hexagon", "Hexagon"),
    ("octagon", "Octagon"),
    ("plus", "Cross"),
    ("can", "Cylinder"),
    ("heart", "Heart"),
    ("smileyFace", "Smiley Face"),
    ("rightArrow", "Arrow: Right"),
    ("leftArrow", "Arrow: Left"),
    ("upArrow", "Arrow: Up"),
    ("downArrow", "Arrow: Down"),
    ("leftRightArrow", "Arrow: Left-Right"),
    ("homePlate", "Arrow: Pentagon"),
    ("chevron", "Arrow: Chevron"),
    ("star5", "Star: 5 Points"),
    ("wedgeRectCallout", "Speech Bubble: Rectangle"),
];

/// Whether `preset` is one of the lines, put in as a connector (`cxnSp`)
/// with no fill or text, as Excel puts a line.
pub fn is_line(preset: &str) -> bool {
    matches!(
        preset,
        "line" | "straightConnector1" | "bentConnector3" | "curvedConnector3"
    )
}

/// Puts a `preset` shape on the sheet with its top-left at `at`'s, `size`
/// px wide and tall, named `name`; returns the object's id.
pub fn insert_shape(
    package: &mut OpcPackage,
    sheet_part: &str,
    preset: &str,
    at: CellRef,
    size: (f64, f64),
    name: &str,
) -> Result<u32> {
    if !SHAPES.iter().any(|(known, _)| *known == preset) {
        return Err(WriteError::Picture("that shape cannot be inserted"));
    }
    let drawing = drawing_for(package, sheet_part)?;
    let emu = |px: f64| (px.max(1.0) * EMU_PER_PX).round() as i64;
    let (cx, cy) = (emu(size.0), emu(size.1));
    let name = escape(name);
    if is_line(preset) {
        // The arrow is the line's end, as Excel's Line Arrow has it.
        let arrow = if preset == "straightConnector1" {
            "<a:ln><a:tailEnd type=\"triangle\"/></a:ln>"
        } else {
            ""
        };
        return append_to_drawing(package, &drawing, |p, id| {
            format!(
                "<{p}oneCellAnchor><{p}from><{p}col>{}</{p}col><{p}colOff>0</{p}colOff><{p}row>{}</{p}row><{p}rowOff>0</{p}rowOff></{p}from><{p}ext cx=\"{cx}\" cy=\"{cy}\"/>\
<{p}cxnSp macro=\"\" xmlns:a=\"{DRAWINGML_NS}\"><{p}nvCxnSpPr><{p}cNvPr id=\"{id}\" name=\"{name} {id}\"/><{p}cNvCxnSpPr/></{p}nvCxnSpPr>\
<{p}spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"{preset}\"><a:avLst/></a:prstGeom>{arrow}</{p}spPr>\
<{p}style><a:lnRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:lnRef><a:fillRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"tx1\"/></a:fontRef></{p}style>\
</{p}cxnSp><{p}clientData/></{p}oneCellAnchor>",
                at.column, at.row
            )
        });
    }
    append_to_drawing(package, &drawing, |p, id| {
        format!(
            "<{p}oneCellAnchor><{p}from><{p}col>{}</{p}col><{p}colOff>0</{p}colOff><{p}row>{}</{p}row><{p}rowOff>0</{p}rowOff></{p}from><{p}ext cx=\"{cx}\" cy=\"{cy}\"/>\
<{p}sp macro=\"\" textlink=\"\" xmlns:a=\"{DRAWINGML_NS}\"><{p}nvSpPr><{p}cNvPr id=\"{id}\" name=\"{name} {id}\"/><{p}cNvSpPr/></{p}nvSpPr>\
<{p}spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"{preset}\"><a:avLst/></a:prstGeom></{p}spPr>\
<{p}style><a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"><a:shade val=\"50000\"/></a:schemeClr></a:lnRef><a:fillRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"lt1\"/></a:fontRef></{p}style>\
<{p}txBody><a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" rtlCol=\"0\" anchor=\"ctr\"/><a:lstStyle/><a:p><a:pPr algn=\"ctr\"/><a:endParaRPr lang=\"en-US\" sz=\"1100\"/></a:p></{p}txBody></{p}sp><{p}clientData/></{p}oneCellAnchor>",
            at.column, at.row
        )
    })
}

/// Insert ▸ Shapes ▸ Freeform: a shape whose outline runs through `points`
/// (px from the shape's top-left; at least two), closed and filled as a
/// new shape is when `closed`, an open line otherwise. Its top-left sits
/// `offset` px into `at`; it is named `name`. Returns the object's id.
pub fn insert_freeform(
    package: &mut OpcPackage,
    sheet_part: &str,
    at: CellRef,
    offset: (f64, f64),
    points: &[(f64, f64)],
    closed: bool,
    name: &str,
) -> Result<u32> {
    if points.len() < 2 {
        return Err(WriteError::Picture("a freeform needs two points or more"));
    }
    let drawing = drawing_for(package, sheet_part)?;
    let emu = |px: f64| (px.max(0.0) * EMU_PER_PX).round() as i64;
    let corners: Vec<(i64, i64)> = points.iter().map(|&(x, y)| (emu(x), emu(y))).collect();
    let cx = corners.iter().map(|p| p.0).max().unwrap_or(0).max(1);
    let cy = corners.iter().map(|p| p.1).max().unwrap_or(0).max(1);
    let (x0, y0) = (emu(offset.0), emu(offset.1));
    let mut path = String::new();
    for (index, (x, y)) in corners.iter().enumerate() {
        let step = if index == 0 { "moveTo" } else { "lnTo" };
        path.push_str(&format!("<a:{step}><a:pt x=\"{x}\" y=\"{y}\"/></a:{step}>"));
    }
    if closed {
        path.push_str("<a:close/>");
    }
    let geometry = format!(
        "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"{cx}\" h=\"{cy}\">{path}</a:path></a:pathLst></a:custGeom>"
    );
    // An open freeform is a line: no fill, nothing to type into.
    let (fill, text) = if closed {
        (
            "",
            "<a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" rtlCol=\"0\" anchor=\"ctr\"/><a:lstStyle/><a:p><a:pPr algn=\"ctr\"/><a:endParaRPr lang=\"en-US\" sz=\"1100\"/></a:p>",
        )
    } else {
        ("<a:noFill/>", "")
    };
    let name = escape(name);
    append_to_drawing(package, &drawing, |p, id| {
        let text = if text.is_empty() {
            String::new()
        } else {
            format!("<{p}txBody>{text}</{p}txBody>")
        };
        format!(
            "<{p}oneCellAnchor><{p}from><{p}col>{}</{p}col><{p}colOff>{x0}</{p}colOff><{p}row>{}</{p}row><{p}rowOff>{y0}</{p}rowOff></{p}from><{p}ext cx=\"{cx}\" cy=\"{cy}\"/>\
<{p}sp macro=\"\" textlink=\"\" xmlns:a=\"{DRAWINGML_NS}\"><{p}nvSpPr><{p}cNvPr id=\"{id}\" name=\"{name} {id}\"/><{p}cNvSpPr/></{p}nvSpPr>\
<{p}spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>{geometry}{fill}</{p}spPr>\
<{p}style><a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"><a:shade val=\"50000\"/></a:schemeClr></a:lnRef><a:fillRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"lt1\"/></a:fontRef></{p}style>\
{text}</{p}sp><{p}clientData/></{p}oneCellAnchor>",
            at.column, at.row
        )
    })
}
