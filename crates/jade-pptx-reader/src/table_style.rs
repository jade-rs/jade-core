//! Table styles: the `ppt/tableStyles.xml` part a deck may carry, and the
//! per-table reference to a style (built-in GUID or custom) with the option
//! flags that switch its parts on.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::slide::{parse_color, parse_fill, parse_outline, Color, Fill, Outline};
use crate::xml_tree::{parse_tree, XmlElement};

/// `<a:tblPr>`: which style the table uses and which of its parts apply.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TableStyleFlags {
    /// `<a:tableStyleId>`, a GUID — a built-in PowerPoint style, or one of
    /// the deck's own in `tableStyles.xml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
    pub first_row: bool,
    pub last_row: bool,
    pub first_col: bool,
    pub last_col: bool,
    pub band_rows: bool,
    pub band_cols: bool,
}

/// One part of a table style (`wholeTbl`, `firstRow`, `band1H`, …): what its
/// cells fill with, the borders around and between them, and how their text
/// is emphasised.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TablePartStyle {
    pub fill: Option<Fill>,
    pub left: Option<Outline>,
    pub right: Option<Outline>,
    pub top: Option<Outline>,
    pub bottom: Option<Outline>,
    /// The border between rows inside the part.
    pub inside_h: Option<Outline>,
    /// The border between columns inside the part.
    pub inside_v: Option<Outline>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub text_color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TableStyle {
    pub id: String,
    pub name: Option<String>,
    pub whole: Option<TablePartStyle>,
    pub first_row: Option<TablePartStyle>,
    pub last_row: Option<TablePartStyle>,
    pub first_col: Option<TablePartStyle>,
    pub last_col: Option<TablePartStyle>,
    pub band1_h: Option<TablePartStyle>,
    pub band2_h: Option<TablePartStyle>,
    pub band1_v: Option<TablePartStyle>,
    pub band2_v: Option<TablePartStyle>,
}

/// Parses `<a:tblStyleLst>`. The `def` attribute names the default style,
/// which is almost always a built-in GUID the part does not define.
pub fn parse_table_styles(xml: &str) -> Result<Vec<TableStyle>> {
    let root = parse_tree(xml)?;
    Ok(root
        .children
        .iter()
        .filter(|child| child.name == "tblStyle")
        .map(|style| TableStyle {
            id: style.attr("styleId").unwrap_or_default().to_string(),
            name: style.attr("styleName").map(str::to_string),
            whole: style.child("wholeTbl").map(parse_part),
            first_row: style.child("firstRow").map(parse_part),
            last_row: style.child("lastRow").map(parse_part),
            first_col: style.child("firstCol").map(parse_part),
            last_col: style.child("lastCol").map(parse_part),
            band1_h: style.child("band1H").map(parse_part),
            band2_h: style.child("band2H").map(parse_part),
            band1_v: style.child("band1V").map(parse_part),
            band2_v: style.child("band2V").map(parse_part),
        })
        .collect())
}

fn parse_part(part: &XmlElement) -> TablePartStyle {
    let tc_style = part.child("tcStyle");
    let borders = tc_style.and_then(|style| style.child("tcBdr"));
    let border = |name: &str| -> Option<Outline> {
        let edge = borders?.child(name)?;
        if let Some(ln) = edge.child("ln") {
            return Some(parse_outline(ln));
        }
        // A `lnRef` names a theme line style; its colour is what matters and
        // the theme's width applies.
        edge.child("lnRef")
            .and_then(parse_color)
            .map(|color| Outline {
                color: Some(color),
                ..Outline::default()
            })
    };
    let fill = tc_style.and_then(|style| {
        style.child("fill").and_then(parse_fill).or_else(|| {
            style
                .child("fillRef")
                .and_then(parse_color)
                .map(Fill::Solid)
        })
    });
    let text = part.child("tcTxStyle");
    let flag = |name: &str| -> Option<bool> {
        match text?.attr(name) {
            Some("on") | Some("1") | Some("true") => Some(true),
            Some("off") | Some("0") | Some("false") => Some(false),
            _ => None,
        }
    };
    TablePartStyle {
        fill,
        left: border("left"),
        right: border("right"),
        top: border("top"),
        bottom: border("bottom"),
        inside_h: border("insideH"),
        inside_v: border("insideV"),
        bold: flag("b"),
        italic: flag("i"),
        text_color: text.and_then(parse_color),
    }
}

/// `<a:tblPr>` of one table.
pub(crate) fn parse_table_style_flags(tbl_pr: Option<&XmlElement>) -> TableStyleFlags {
    let flag = |name: &str| -> bool {
        matches!(
            tbl_pr.and_then(|pr| pr.attr(name)),
            Some("1") | Some("true")
        )
    };
    TableStyleFlags {
        style_id: tbl_pr
            .and_then(|pr| pr.child("tableStyleId"))
            .map(|id| id.text.trim().to_string())
            .filter(|id| !id.is_empty()),
        first_row: flag("firstRow"),
        last_row: flag("lastRow"),
        first_col: flag("firstCol"),
        last_col: flag("lastCol"),
        band_rows: flag("bandRow"),
        band_cols: flag("bandCol"),
    }
}
