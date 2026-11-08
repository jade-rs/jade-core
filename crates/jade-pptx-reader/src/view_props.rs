//! `ppt/viewProps.xml`: the view a deck was last saved in, and the slide
//! view's grid and guides (表示 › ガイド, グリッドとガイド).

use jade_opc::OpcPackage;
use serde::{Deserialize, Serialize};

use crate::xml_tree::{parse_tree, XmlElement};
use crate::Result;

/// `lastView`: the view PowerPoint reopens the deck in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LastView {
    /// 標準 (`sldView`, and `sldThumbnailView`, which is the same view).
    #[default]
    Normal,
    /// アウトライン表示 (`outlineView`).
    Outline,
    /// スライド一覧 (`sldSorterView`).
    SlideSorter,
    /// ノート (`notesView`).
    NotesPage,
    /// スライド マスター (`sldMasterView`).
    SlideMaster,
    /// ノート マスター (`notesMasterView`).
    NotesMaster,
    /// 配布資料マスター (`handoutView`).
    HandoutMaster,
}

impl LastView {
    pub fn from_attr(value: &str) -> Self {
        match value {
            "outlineView" => Self::Outline,
            "sldSorterView" => Self::SlideSorter,
            "notesView" => Self::NotesPage,
            "sldMasterView" => Self::SlideMaster,
            "notesMasterView" => Self::NotesMaster,
            "handoutView" => Self::HandoutMaster,
            _ => Self::Normal,
        }
    }

    pub fn attr(self) -> &'static str {
        match self {
            Self::Normal => "sldView",
            Self::Outline => "outlineView",
            Self::SlideSorter => "sldSorterView",
            Self::NotesPage => "notesView",
            Self::SlideMaster => "sldMasterView",
            Self::NotesMaster => "notesMasterView",
            Self::HandoutMaster => "handoutView",
        }
    }
}

/// One guide across the slide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guide {
    /// A horizontal guide (`orient="horz"`) sits at a height; a vertical one
    /// at a distance from the left edge.
    pub horizontal: bool,
    /// From the slide's top (or left) edge, in EMU.
    pub position_emu: i64,
}

/// Units PowerPoint stores guide positions in: eighths of a point.
pub const GUIDE_UNITS_PER_EMU: f64 = 12_700.0 / 8.0;

/// The parts of `<p:viewPr>` jade reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewProps {
    pub last_view: LastView,
    /// `<p:gridSpacing cx cy>` in EMU.
    pub grid_spacing_emu: (i64, i64),
    /// オブジェクトをグリッドに合わせる (`snapToGrid`, default on).
    pub snap_to_grid: bool,
    /// 図形の整列時にスマート ガイドを表示 — `snapToObjects`.
    pub snap_to_objects: bool,
    /// ガイド (`showGuides`).
    pub show_guides: bool,
    /// The slide view's `<p:guideLst>`.
    pub guides: Vec<Guide>,
}

impl Default for ViewProps {
    /// A new deck's: 標準, a 0.2 cm grid snapped to, no guides.
    fn default() -> Self {
        Self {
            last_view: LastView::Normal,
            grid_spacing_emu: (72_000, 72_000),
            snap_to_grid: true,
            snap_to_objects: false,
            show_guides: false,
            guides: Vec::new(),
        }
    }
}

/// Reads `viewProps.xml`, or the defaults when the deck has none.
pub fn read_view_props(bytes: &[u8]) -> Result<ViewProps> {
    let package = OpcPackage::open(bytes)?;
    let presentation = crate::presentation_part_name(&package)?;
    let Some(rels) = package.part_str(&jade_opc::rels_part_name(&presentation))? else {
        return Ok(ViewProps::default());
    };
    let Some(part) = jade_opc::parse_relationships(&rels)?
        .target_by_type_suffix("/viewProps")
        .map(|target| jade_opc::resolve_target(&presentation, target))
    else {
        return Ok(ViewProps::default());
    };
    match package.part_str(&part)? {
        Some(xml) => Ok(parse_view_props(&parse_tree(&xml)?)),
        None => Ok(ViewProps::default()),
    }
}

fn flag(element: &XmlElement, name: &str, default: bool) -> bool {
    match element.attr(name) {
        Some("1" | "true" | "on") => true,
        Some("0" | "false" | "off") => false,
        _ => default,
    }
}

pub(crate) fn parse_view_props(root: &XmlElement) -> ViewProps {
    let mut props = ViewProps {
        last_view: root
            .attr("lastView")
            .map(LastView::from_attr)
            .unwrap_or_default(),
        ..ViewProps::default()
    };
    if let Some(grid) = root.child("gridSpacing") {
        let cx = grid.attr("cx").and_then(|v| v.parse().ok());
        let cy = grid.attr("cy").and_then(|v| v.parse().ok());
        if let (Some(cx), Some(cy)) = (cx, cy) {
            if cx > 0 && cy > 0 {
                props.grid_spacing_emu = (cx, cy);
            }
        }
    }
    if let Some(common) = root
        .child("slideViewPr")
        .and_then(|view| view.child("cSldViewPr"))
    {
        props.snap_to_grid = flag(common, "snapToGrid", true);
        props.snap_to_objects = flag(common, "snapToObjects", false);
        props.show_guides = flag(common, "showGuides", false);
        if let Some(list) = common.child("guideLst") {
            props.guides = list
                .children
                .iter()
                .filter(|guide| guide.name == "guide")
                .map(|guide| Guide {
                    horizontal: guide.attr("orient") == Some("horz"),
                    position_emu: (guide
                        .attr("pos")
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                        * GUIDE_UNITS_PER_EMU)
                        .round() as i64,
                })
                .collect();
        }
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_guides_and_last_view_are_read() {
        let root = parse_tree(
            r#"<p:viewPr xmlns:p="p" lastView="sldSorterView"><p:slideViewPr><p:cSldViewPr snapToGrid="0" snapToObjects="1" showGuides="1"><p:guideLst><p:guide orient="horz" pos="2160"/><p:guide pos="2880"/></p:guideLst></p:cSldViewPr></p:slideViewPr><p:gridSpacing cx="76200" cy="76200"/></p:viewPr>"#,
        )
        .unwrap();
        let props = parse_view_props(&root);
        assert_eq!(props.last_view, LastView::SlideSorter);
        assert_eq!(props.grid_spacing_emu, (76200, 76200));
        assert!(!props.snap_to_grid && props.snap_to_objects && props.show_guides);
        // 2160 eighths of a point is 270 pt: the middle of a 7.5 in slide.
        assert_eq!(
            props.guides,
            vec![
                Guide {
                    horizontal: true,
                    position_emu: 3_429_000
                },
                Guide {
                    horizontal: false,
                    position_emu: 4_572_000
                }
            ]
        );
        assert_eq!(
            parse_view_props(&parse_tree(r#"<p:viewPr xmlns:p="p"/>"#).unwrap()),
            ViewProps::default()
        );
    }
}
