//! Ink kept after a slide show (インク注釈を保持しますか? › 保持): each pen or
//! highlighter stroke lands on its slide as a freeform line, which PowerPoint
//! opens, moves and deletes like any other shape.

use super::objects::insertion_identity;
use super::{apply_patches, child, parse_nodes};
use crate::{Result, WriteError};
use serde::{Deserialize, Serialize};

/// One stroke, in EMU on the slide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InkPath {
    pub points_emu: Vec<(i64, i64)>,
    /// `RRGGBB`.
    pub color: String,
    pub width_emu: i64,
    /// A highlighter's stroke: half transparent.
    pub highlighter: bool,
}

pub(crate) fn insert_ink(xml: &str, paths: &[InkPath]) -> Result<String> {
    let mut xml = xml.to_string();
    for path in paths {
        if path.points_emu.len() < 2
            || path.color.len() != 6
            || !path.color.bytes().all(|b| b.is_ascii_hexdigit())
            || path.width_emu <= 0
        {
            return Err(WriteError::InvalidShapeStyle);
        }
        let nodes = parse_nodes(&xml)?;
        let (tree, id, name) = insertion_identity(&nodes, "Ink")?;
        let p = if tree.strict {
            "http://purl.oclc.org/ooxml/presentationml/main"
        } else {
            "http://schemas.openxmlformats.org/presentationml/2006/main"
        };
        let a = super::drawing_ns(tree.strict);
        let min_x = path.points_emu.iter().map(|p| p.0).min().unwrap_or(0);
        let min_y = path.points_emu.iter().map(|p| p.1).min().unwrap_or(0);
        let max_x = path.points_emu.iter().map(|p| p.0).max().unwrap_or(0);
        let max_y = path.points_emu.iter().map(|p| p.1).max().unwrap_or(0);
        let (w, h) = ((max_x - min_x).max(1), (max_y - min_y).max(1));
        let mut commands = String::new();
        for (i, &(x, y)) in path.points_emu.iter().enumerate() {
            let tag = if i == 0 { "moveTo" } else { "lnTo" };
            commands.push_str(&format!(
                "<inkD:{tag}><inkD:pt x=\"{}\" y=\"{}\"/></inkD:{tag}>",
                x - min_x,
                y - min_y
            ));
        }
        let alpha = if path.highlighter {
            "<inkD:alpha val=\"50000\"/>"
        } else {
            ""
        };
        let fragment = format!(
            "<inkP:sp xmlns:inkP=\"{p}\" xmlns:inkD=\"{a}\"><inkP:nvSpPr><inkP:cNvPr id=\"{id}\" name=\"{name}\"/><inkP:cNvSpPr/><inkP:nvPr/></inkP:nvSpPr><inkP:spPr><inkD:xfrm><inkD:off x=\"{min_x}\" y=\"{min_y}\"/><inkD:ext cx=\"{w}\" cy=\"{h}\"/></inkD:xfrm><inkD:custGeom><inkD:avLst/><inkD:gdLst/><inkD:ahLst/><inkD:cxnLst/><inkD:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><inkD:pathLst><inkD:path w=\"{w}\" h=\"{h}\">{commands}</inkD:path></inkD:pathLst></inkD:custGeom><inkD:noFill/><inkD:ln w=\"{}\" cap=\"rnd\"><inkD:solidFill><inkD:srgbClr val=\"{}\">{alpha}</inkD:srgbClr></inkD:solidFill><inkD:round/></inkD:ln></inkP:spPr></inkP:sp>",
            path.width_emu,
            path.color.to_ascii_uppercase()
        );
        let at = child(&nodes, tree, 1, "extLst").map_or(tree.range.end, |node| node.outer.start);
        xml = apply_patches(&xml, vec![(at..at, fragment)]);
    }
    Ok(xml)
}
