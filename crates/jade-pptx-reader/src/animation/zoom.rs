//! Summary, slide and section zooms (挿入 › ズーム).
//!
//! PowerPoint 2019 writes a zoom as a graphic frame inside
//! `mc:AlternateContent`: the choice holds `pslz:sldZm`, `psez:sectionZm`
//! or `psuz:summaryZm` with each thumbnail's target, picture, place and
//! zoom properties; the fallback a picture (or group of pictures) with a
//! slide-jump hyperlink, which is what the slide draws. The show needs the
//! choice: which slide or section a click goes to, whether it comes back
//! (ズームに戻る) and how long the zoom takes.

use serde::{Deserialize, Serialize};

use super::node::XmlNode;
use super::transition::Namespaces;

pub const NS_SLIDE_ZOOM: &str = "http://schemas.microsoft.com/office/powerpoint/2016/slidezoom";
pub const NS_SECTION_ZOOM: &str = "http://schemas.microsoft.com/office/powerpoint/2016/sectionzoom";
pub const NS_SUMMARY_ZOOM: &str = "http://schemas.microsoft.com/office/powerpoint/2016/summaryzoom";
pub const NS_P166: &str = "http://schemas.microsoft.com/office/powerpoint/2016/6/main";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoomKind {
    Slide,
    Section,
    Summary,
}

/// One zoom thumbnail: where it sits and where it goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoomFrame {
    pub kind: ZoomKind,
    /// The graphic frame's `cNvPr id`.
    pub shape_id: u32,
    pub name: String,
    /// The thumbnail's box on the slide, EMU: x, y, cx, cy.
    pub rect: (i64, i64, i64, i64),
    /// `sldId` of the target (slide zoom).
    pub slide_id: Option<u32>,
    /// `sectionId` of the target (section and summary zoom).
    pub section_id: Option<String>,
    /// ズームに戻る.
    pub return_to_parent: bool,
    /// 継続時間, ms.
    pub transition_ms: u32,
    /// ズームの背景: the thumbnail drawn without its slide's background.
    pub hide_background: bool,
}

fn emu(node: Option<&XmlNode>, name: &str) -> i64 {
    node.and_then(|node| node.attr(name))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn xfrm_rect(xfrm: Option<&XmlNode>) -> (i64, i64, i64, i64) {
    let off = xfrm.and_then(|x| x.child("off"));
    let ext = xfrm.and_then(|x| x.child("ext"));
    (emu(off, "x"), emu(off, "y"), emu(ext, "cx"), emu(ext, "cy"))
}

fn truthy(value: Option<&str>) -> Option<bool> {
    value.map(|value| matches!(value, "1" | "true"))
}

/// Every zoom in a slide's shape tree (top level, and inside groups).
pub fn read_zooms(root: &XmlNode, scope: &Namespaces) -> Vec<ZoomFrame> {
    let mut out = Vec::new();
    if let Some(tree) = root.path(&["cSld", "spTree"]) {
        walk(tree, scope, &mut out);
    }
    out
}

fn walk(tree: &XmlNode, scope: &Namespaces, out: &mut Vec<ZoomFrame>) {
    for child in tree.elements() {
        match child.local_name() {
            "AlternateContent" => {
                let mut scope = scope.clone();
                scope.declare_from(child);
                for choice in child.children_named("Choice") {
                    let mut choice_scope = scope.clone();
                    choice_scope.declare_from(choice);
                    if let Some(frame) = choice.child("graphicFrame") {
                        out.extend(zoom_frames(frame, &choice_scope));
                    }
                }
            }
            "grpSp" => walk(child, scope, out),
            _ => {}
        }
    }
}

fn zoom_frames(frame: &XmlNode, scope: &Namespaces) -> Vec<ZoomFrame> {
    let Some(data) = frame.path(&["graphic", "graphicData"]) else {
        return Vec::new();
    };
    let kind = match data.attr("uri") {
        Some(NS_SLIDE_ZOOM) => ZoomKind::Slide,
        Some(NS_SECTION_ZOOM) => ZoomKind::Section,
        Some(NS_SUMMARY_ZOOM) => ZoomKind::Summary,
        _ => return Vec::new(),
    };
    let _ = scope;
    let cnv = frame.path(&["nvGraphicFramePr", "cNvPr"]);
    let shape_id = cnv
        .and_then(|c| c.attr("id"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let name = cnv
        .and_then(|c| c.attr("name"))
        .unwrap_or_default()
        .to_string();
    let frame_rect = xfrm_rect(frame.child("xfrm"));
    let Some(zoom) = data.elements().next() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for object in zoom.elements() {
        let properties = object.child("zmPr");
        let rect = match properties
            .and_then(|p| p.child("spPr"))
            .and_then(|sp| sp.child("xfrm"))
        {
            // A summary zoom places each thumbnail inside the frame.
            Some(xfrm) if kind == ZoomKind::Summary => {
                let (x, y, cx, cy) = xfrm_rect(Some(xfrm));
                (frame_rect.0 + x, frame_rect.1 + y, cx, cy)
            }
            _ => frame_rect,
        };
        out.push(ZoomFrame {
            kind,
            shape_id,
            name: name.clone(),
            rect,
            slide_id: object.attr("sldId").and_then(|v| v.parse().ok()),
            section_id: object.attr("sectionId").map(str::to_string),
            return_to_parent: truthy(properties.and_then(|p| p.attr("returnToParent")))
                .unwrap_or(kind != ZoomKind::Slide),
            transition_ms: properties
                .and_then(|p| p.attr("transitionDur"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
            hide_background: truthy(properties.and_then(|p| p.attr("showBg")))
                .map(|show| !show)
                .unwrap_or(false),
        });
    }
    out
}
