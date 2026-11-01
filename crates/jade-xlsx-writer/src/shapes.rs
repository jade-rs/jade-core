//! Insert ▸ Shapes: a rectangle, oval, rounded rectangle or arrow put on a
//! sheet as a new object in its drawing, filled and outlined by the
//! theme's first accent as Excel draws a new shape, ready for text.

use jade_opc::OpcPackage;
use jade_xlsx_reader::CellRef;

use crate::charts::{append_to_drawing, drawing_for};
use crate::patch::escape;
use crate::{Result, WriteError};

const DRAWINGML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// English Metric Units in a CSS px at 96 an inch.
const EMU_PER_PX: f64 = 9525.0;

/// The preset geometries Insert ▸ Shapes offers, each with the name Excel
/// gives a new one.
pub const SHAPES: [(&str, &str); 4] = [
    ("rect", "Rectangle"),
    ("ellipse", "Oval"),
    ("roundRect", "Rectangle: Rounded Corners"),
    ("rightArrow", "Arrow: Right"),
];

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
