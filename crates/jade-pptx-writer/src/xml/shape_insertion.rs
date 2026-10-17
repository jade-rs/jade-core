use super::{apply_patches, child, objects, parse_nodes, transform_xml};
use crate::{Result, ShapeInsert, TextFormatEdit};
use jade_pptx_reader::Transform;

fn paint(value: Option<&str>) -> String {
    match value {
        Some("none") => "<jadeDraw:noFill/>".into(),
        Some(color) => format!(
            "<jadeDraw:solidFill><jadeDraw:srgbClr val=\"{}\"/></jadeDraw:solidFill>",
            color[1..].to_ascii_uppercase()
        ),
        None => {
            "<jadeDraw:solidFill><jadeDraw:schemeClr val=\"accent1\"/></jadeDraw:solidFill>".into()
        }
    }
}
pub fn insert(xml: &str, transform: Transform, value: &ShapeInsert) -> Result<(String, u32)> {
    let nodes = parse_nodes(xml)?;
    let (tree, id, name) = objects::insertion_identity(&nodes, "Shape")?;
    let ns = super::slides::p_ns(tree.strict);
    let draw = super::drawing_ns(tree.strict);
    let geometry = transform_xml(xml, &nodes, None, transform, tree.strict)?;
    let fill = paint(value.fill.as_deref());
    let outline = paint(value.outline.as_deref());
    let width = (value.outline_width_pt.unwrap_or(1.0) * 12700.0).round() as u64;
    let preset = value.preset.as_str();
    let paragraphs = objects::text_box_paragraphs(
        &value.text,
        true,
        if value.fill.as_deref() == Some("none") {
            "tx1"
        } else {
            "lt1"
        },
    );
    let fragment = format!(
        r#"<jadeSp:sp xmlns:jadeSp="{ns}" xmlns:jadeDraw="{draw}"><jadeSp:nvSpPr><jadeSp:cNvPr id="{id}" name="{name}"/><jadeSp:cNvSpPr/><jadeSp:nvPr/></jadeSp:nvSpPr><jadeSp:spPr>{geometry}<jadeDraw:prstGeom prst="{preset}"><jadeDraw:avLst/></jadeDraw:prstGeom>{fill}<jadeDraw:ln w="{width}">{outline}<jadeDraw:prstDash val="solid"/></jadeDraw:ln></jadeSp:spPr><jadeSp:txBody><jadeDraw:bodyPr wrap="square" lIns="91440" rIns="91440" tIns="45720" bIns="45720" anchor="ctr"><jadeDraw:noAutofit/></jadeDraw:bodyPr><jadeDraw:lstStyle/>{paragraphs}</jadeSp:txBody></jadeSp:sp>"#
    );
    let at = child(&nodes, tree, 1, "extLst").map_or(tree.range.end, |node| node.outer.start);
    let inserted = apply_patches(xml, vec![(at..at, fragment)]);
    let changed = super::format_text_range(
        &inserted,
        &TextFormatEdit {
            shape_id: id,
            start: 0,
            end: value.text.encode_utf16().count(),
            format: value.format.clone(),
        },
    )?;
    Ok((changed, id))
}
