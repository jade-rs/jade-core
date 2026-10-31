//! Freeze Panes: the frozen `<pane>` of a sheet's first view.
//!
//! Excel writes a frozen sheet as a `<pane>` saying how many columns and
//! rows stay put, which cell the scrolling part starts at and which part is
//! active, followed by a `<selection>` for each part that scrolls. Freezing
//! anew replaces both; unfreezing leaves one plain selection. Everything
//! else in the view (`tabSelected`, zoom, `pivotSelection`, extensions)
//! stays as it was.

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use jade_xlsx_reader::CellRef;

use crate::patch::{apply, attribute, escape, local, Splice};
use crate::{Result, WriteError};

/// Worksheet children that come after `sheetViews`, in schema order.
const FOLLOWING: &[&[u8]] = &[
    b"sheetFormatPr",
    b"cols",
    b"sheetData",
    b"sheetCalcPr",
    b"sheetProtection",
    b"protectedRanges",
    b"scenarios",
    b"autoFilter",
    b"sortState",
    b"dataConsolidate",
    b"customSheetViews",
    b"mergeCells",
    b"phoneticPr",
    b"conditionalFormatting",
    b"dataValidations",
    b"hyperlinks",
    b"printOptions",
    b"pageMargins",
    b"pageSetup",
    b"headerFooter",
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"drawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];

/// What to freeze: `columns` on the left and `rows` at the top, the
/// scrolling part starting at `top_left` (the first unfrozen cell when
/// `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frozen {
    pub columns: u32,
    pub rows: u32,
    pub top_left: Option<CellRef>,
}

/// The sheet part with its first view frozen as `frozen` says, or
/// unfrozen for `None` (or nothing frozen).
pub fn set_freeze_pane(xml: &str, frozen: Option<Frozen>) -> Result<String> {
    let frozen = frozen.filter(|frozen| frozen.columns > 0 || frozen.rows > 0);
    if let Some(frozen) = frozen {
        if frozen.columns >= 16_384 || frozen.rows >= 1_048_576 {
            return Err(WriteError::BadFreezePane);
        }
    }
    let view = scan(xml)?;
    let prefix = view.prefix.clone();
    let Some(sheet_view) = view.sheet_view else {
        let Some(frozen) = frozen else {
            return Ok(xml.to_string());
        };
        let children = view_children(&prefix, Some(frozen), None);
        let text = format!(
            "<{prefix}sheetViews><{prefix}sheetView workbookViewId=\"0\">{children}</{prefix}sheetView></{prefix}sheetViews>"
        );
        let at = view
            .insert_views_at
            .ok_or(WriteError::NoRoot("worksheet"))?;
        return Ok(apply(
            xml,
            vec![Splice {
                start: at,
                end: at,
                text,
            }],
        ));
    };
    // A view can use a different prefix or locally override the default namespace.
    // Its own qualified name identifies the namespace for its new children.
    let prefix = &sheet_view.prefix;
    let children = view_children(prefix, frozen, sheet_view.active_selection.as_ref());
    let mut splices: Vec<Splice> = sheet_view
        .replaced
        .iter()
        .map(|&(start, end)| Splice {
            start,
            end,
            text: String::new(),
        })
        .collect();
    match sheet_view.empty_head {
        // `<sheetView …/>`: open it to hold the new children.
        Some((start, end)) => {
            let head = xml[start..end].trim_end().trim_end_matches("/>");
            splices.push(Splice {
                start,
                end,
                text: format!("{head}>{children}</{prefix}sheetView>"),
            });
        }
        None => splices.push(Splice {
            start: sheet_view.children_at,
            end: sheet_view.children_at,
            text: children,
        }),
    }
    Ok(apply(xml, splices))
}

/// The `<pane>` and `<selection>`s a view gets: Excel's for a frozen one,
/// one plain selection for an unfrozen one. `selection` is the active cell
/// and range the view had, kept in the part that is active now.
fn view_children(
    prefix: &str,
    frozen: Option<Frozen>,
    selection: Option<&(String, String)>,
) -> String {
    let Some(frozen) = frozen else {
        return match selection {
            Some((active, sqref)) => format!(
                "<{prefix}selection activeCell=\"{}\" sqref=\"{}\"/>",
                escape(active),
                escape(sqref)
            ),
            None => String::new(),
        };
    };
    let first = CellRef::new(frozen.columns, frozen.rows);
    let top_left = frozen.top_left.unwrap_or(first);
    let active_pane = match (frozen.columns > 0, frozen.rows > 0) {
        (true, true) => "bottomRight",
        (false, true) => "bottomLeft",
        _ => "topRight",
    };
    let mut pane = format!("<{prefix}pane");
    if frozen.columns > 0 {
        pane.push_str(&format!(" xSplit=\"{}\"", frozen.columns));
    }
    if frozen.rows > 0 {
        pane.push_str(&format!(" ySplit=\"{}\"", frozen.rows));
    }
    pane.push_str(&format!(
        " topLeftCell=\"{}\" activePane=\"{active_pane}\" state=\"frozen\"/>",
        address(top_left)
    ));
    let selection_in = |pane_name: &str, active: &str, sqref: &str| {
        format!(
            "<{prefix}selection pane=\"{pane_name}\" activeCell=\"{}\" sqref=\"{}\"/>",
            escape(active),
            escape(sqref)
        )
    };
    let mut children = pane;
    if frozen.columns > 0 && frozen.rows > 0 {
        let right = address(CellRef::new(frozen.columns, 0));
        let below = address(CellRef::new(0, frozen.rows));
        children.push_str(&selection_in("topRight", &right, &right));
        children.push_str(&selection_in("bottomLeft", &below, &below));
    }
    let (active, sqref) = selection
        .cloned()
        .unwrap_or_else(|| (address(first), address(first)));
    children.push_str(&selection_in(active_pane, &active, &sqref));
    children
}

fn address(cell: CellRef) -> String {
    let mut column = cell.column + 1;
    let mut letters = Vec::new();
    while column > 0 {
        letters.push(b'A' + ((column - 1) % 26) as u8);
        column = (column - 1) / 26;
    }
    letters.reverse();
    format!("{}{}", String::from_utf8_lossy(&letters), cell.row + 1)
}

#[derive(Default)]
struct SheetView {
    prefix: String,
    /// `<sheetView …/>`'s span, when it has no children.
    empty_head: Option<(usize, usize)>,
    /// Where new children go: right after the opening tag.
    children_at: usize,
    /// The `<pane>` and `<selection>`s to drop.
    replaced: Vec<(usize, usize)>,
    /// The active pane's selection (or the last one): active cell, sqref.
    active_selection: Option<(String, String)>,
}

#[derive(Default)]
struct Scan {
    prefix: String,
    sheet_view: Option<SheetView>,
    /// Where a missing `<sheetViews>` goes.
    insert_views_at: Option<usize>,
}

fn scan(xml: &str) -> Result<Scan> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut found = Scan::default();
    let mut depth = 0usize;
    let mut in_views = false;
    let mut in_first_view = false;
    let mut views_seen = false;
    let mut active_pane: Option<String> = None;
    let mut selections: Vec<(Option<String>, String, String)> = Vec::new();
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name().as_ref().to_vec();
                let name = local(&qualified).to_vec();
                match depth {
                    0 => found.prefix = crate::patch::namespace_prefix(&qualified),
                    1 if name == b"sheetViews" => {
                        views_seen = true;
                        in_views = !empty;
                    }
                    1 if found.insert_views_at.is_none()
                        && !views_seen
                        && FOLLOWING.contains(&name.as_slice()) =>
                    {
                        found.insert_views_at = Some(start);
                    }
                    2 if in_views && name == b"sheetView" && found.sheet_view.is_none() => {
                        found.sheet_view = Some(SheetView {
                            prefix: crate::patch::namespace_prefix(&qualified),
                            empty_head: empty.then_some((start, end)),
                            children_at: end,
                            ..SheetView::default()
                        });
                        in_first_view = !empty;
                    }
                    3 if in_first_view && (name == b"pane" || name == b"selection") => {
                        if name == b"pane" {
                            active_pane = attribute(element, b"activePane")?;
                        } else {
                            selections.push((
                                attribute(element, b"pane")?,
                                attribute(element, b"activeCell")?.unwrap_or_default(),
                                attribute(element, b"sqref")?.unwrap_or_default(),
                            ));
                        }
                        let close = if empty {
                            end
                        } else {
                            reader.read_to_end_into(QName(&qualified), &mut Vec::new())?;
                            reader.buffer_position() as usize
                        };
                        if let Some(view) = found.sheet_view.as_mut() {
                            view.replaced.push((start, close));
                        }
                        buf.clear();
                        continue;
                    }
                    _ => {}
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(ref element) => {
                depth = depth.saturating_sub(1);
                let name = local(element.name().as_ref()).to_vec();
                if depth == 0 && found.insert_views_at.is_none() && !views_seen {
                    found.insert_views_at = Some(start);
                }
                if depth == 1 && name == b"sheetViews" {
                    in_views = false;
                }
                if depth == 2 && name == b"sheetView" {
                    in_first_view = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if let Some(view) = found.sheet_view.as_mut() {
        // Excel names the pane each selection belongs to; the active pane's
        // is the one the cursor is in. An unsplit view's has no pane.
        let active = selections
            .iter()
            .find(|(pane, _, _)| pane.as_deref() == active_pane.as_deref())
            .or_else(|| selections.last())
            .filter(|(_, active, sqref)| !active.is_empty() || !sqref.is_empty())
            .map(|(_, active, sqref)| {
                let sqref = if sqref.is_empty() { active } else { sqref };
                let active = if active.is_empty() {
                    sqref.split([' ', ':']).next().unwrap_or(sqref)
                } else {
                    active
                };
                (active.to_string(), sqref.to_string())
            });
        view.active_selection = active;
    }
    Ok(found)
}
