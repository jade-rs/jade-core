//! Paragraph borders (`w:pBdr`) and character borders (`w:bdr`) in the PDF,
//! each side drawn in its `ST_Border` line style as the editor draws it: the
//! rules across the band and the pattern along it come from
//! [`jade_docx_layout::border_styles`].

use jade_docx_layout::border_styles::{
    BorderLineStyle, BorderSide, Pattern, Tone, pattern, pattern_polylines, pattern_runs, stripes,
    tone_color,
};
use jade_docx_layout::text_extract::{DocumentLineLayout, LineInfo};
use jade_docx_layout::units::points_to_px;
use jade_docx_reader::document::{BorderEdge, ParagraphBorders};
use jade_docx_reader::{BodyElement, Document as DocxDocument, Paragraph as DocxParagraph};
use lopdf::Object;
use lopdf::content::Operation;

use crate::{PdfColor, parse_hex_color, px_to_pt};

/// One side of a border box, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RenderBorderEdge {
    pub style: BorderLineStyle,
    /// The band across the edge, in px.
    pub width: f64,
    pub color: PdfColor,
}

impl RenderBorderEdge {
    /// `edge` as drawn, or `None` when it draws nothing (`none`, `nil`, an
    /// art style a paragraph cannot take). `auto` and absent colours follow
    /// `text_color`, then black.
    pub(crate) fn from_edge(edge: &BorderEdge, text_color: Option<PdfColor>) -> Option<Self> {
        if !edge.is_visible() {
            return None;
        }
        let style = BorderLineStyle::parse(edge.val.as_deref().unwrap_or("single"))?;
        let points = edge.size_eighths.map_or(0.5, |size| f64::from(size) / 8.0);
        Some(Self {
            style,
            width: points_to_px(points as f32).max(1.0),
            color: edge
                .color
                .as_deref()
                .and_then(parse_hex_color)
                .or(text_color)
                .unwrap_or(PdfColor {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                }),
        })
    }
}

/// A box with up to four bordered sides, in px on its page; each side's band
/// lies inside the box.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RenderBorderBox {
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub top: Option<RenderBorderEdge>,
    pub bottom: Option<RenderBorderEdge>,
    pub left: Option<RenderBorderEdge>,
    pub right: Option<RenderBorderEdge>,
}

fn edge_space_px(edge: Option<&BorderEdge>) -> f64 {
    edge.filter(|edge| edge.is_visible())
        .map_or(0.0, |edge| points_to_px(edge.space.unwrap_or(0) as f32))
}

/// The body paragraphs in the order the line layout numbers them, `None` for
/// the slots that are not top-level paragraphs (math, table cell paragraphs).
fn layout_paragraphs(document: &DocxDocument) -> Vec<Option<&DocxParagraph>> {
    fn table_slots(table: &jade_docx_reader::Table, out: &mut Vec<Option<&DocxParagraph>>) {
        for cell in table.rows.iter().flat_map(|row| &row.cells) {
            for block in &cell.blocks {
                match block {
                    BodyElement::Table(nested) => table_slots(nested, out),
                    _ => out.push(None),
                }
            }
        }
    }
    let mut out = Vec::new();
    for element in document.body.iter().flat_map(|body| &body.elements) {
        match element {
            BodyElement::Paragraph(paragraph) => out.push(Some(paragraph.as_ref())),
            BodyElement::Math(_) => out.push(None),
            BodyElement::Table(table) => table_slots(table, &mut out),
        }
    }
    out
}

/// The paragraph border boxes of the document's top-level paragraphs: one per
/// paragraph and page, from the left indent to the right one and from its
/// first line's top to its last line's bottom on that page, pushed out by
/// each side's `w:space`. The top border is drawn only where the paragraph
/// starts and the bottom one only where it ends.
pub(crate) fn collect_paragraph_border_boxes(
    document: &DocxDocument,
    layout: &DocumentLineLayout,
    content_left: f64,
    content_right: f64,
) -> Vec<RenderBorderBox> {
    let paragraphs = layout_paragraphs(document);
    let mut boxes: Vec<RenderBorderBox> = Vec::new();
    let mut lines: Vec<&LineInfo> = layout.get_lines().collect();
    lines.sort_by_key(|line| {
        (
            line.paragraph_index,
            line.page,
            line.line_index_in_paragraph,
        )
    });
    let mut index = 0;
    while index < lines.len() {
        let first = lines[index];
        let mut end = index + 1;
        while end < lines.len()
            && lines[end].paragraph_index == first.paragraph_index
            && lines[end].page == first.page
        {
            end += 1;
        }
        let group = &lines[index..end];
        index = end;
        let Some(Some(paragraph)) = paragraphs.get(first.paragraph_index) else {
            continue;
        };
        let Some(properties) = paragraph.properties.as_ref() else {
            continue;
        };
        let Some(borders) = properties.borders.as_ref() else {
            continue;
        };
        let ParagraphBorders {
            top,
            bottom,
            left,
            right,
            ..
        } = borders;
        let resolve = |edge: &Option<BorderEdge>| {
            edge.as_ref()
                .and_then(|edge| RenderBorderEdge::from_edge(edge, None))
        };
        let starts = group[0].line_index_in_paragraph == 0;
        let ends = !lines[end..]
            .iter()
            .take_while(|line| line.paragraph_index == first.paragraph_index)
            .any(|_| true);
        let (top_edge, bottom_edge) = (
            resolve(top).filter(|_| starts),
            resolve(bottom).filter(|_| ends),
        );
        let (left_edge, right_edge) = (resolve(left), resolve(right));
        if top_edge.is_none()
            && bottom_edge.is_none()
            && left_edge.is_none()
            && right_edge.is_none()
        {
            continue;
        }
        let bidi = properties.bidi.unwrap_or(false);
        let indent = properties.indent.as_ref();
        let indent_left = indent
            .and_then(|indent| indent.physical_left(bidi))
            .map_or(0.0, points_to_px);
        let indent_right = indent
            .and_then(|indent| indent.physical_right(bidi))
            .map_or(0.0, points_to_px);
        let band = |edge: &Option<RenderBorderEdge>| edge.as_ref().map_or(0.0, |edge| edge.width);
        let x = content_left + indent_left - edge_space_px(left.as_ref()) - band(&left_edge);
        let right_x =
            content_right - indent_right + edge_space_px(right.as_ref()) + band(&right_edge);
        let text_top = group[0].y;
        let text_bottom = group
            .iter()
            .map(|line| line.y + line.height)
            .fold(text_top, f64::max);
        let y = text_top
            - if top_edge.is_some() {
                edge_space_px(top.as_ref()) + band(&top_edge)
            } else {
                0.0
            };
        let bottom_y = text_bottom
            + if bottom_edge.is_some() {
                edge_space_px(bottom.as_ref()) + band(&bottom_edge)
            } else {
                0.0
            };
        boxes.push(RenderBorderBox {
            page: first.page,
            x,
            y,
            width: (right_x - x).max(0.0),
            height: (bottom_y - y).max(0.0),
            top: top_edge,
            bottom: bottom_edge,
            left: left_edge,
            right: right_edge,
        });
    }
    boxes
}

/// The character border boxes on `line`: consecutive spans with the same
/// `w:bdr` share one box, as Word draws them, the height of the line.
pub(crate) fn collect_character_border_boxes(line: &LineInfo) -> Vec<RenderBorderBox> {
    let mut boxes: Vec<(BorderEdge, RenderBorderBox)> = Vec::new();
    for span in &line.spans {
        let Some(properties) = span.properties.as_ref() else {
            continue;
        };
        let Some(border) = properties.border.as_ref() else {
            continue;
        };
        let text_color = properties.color.as_deref().and_then(parse_hex_color);
        let Some(edge) = RenderBorderEdge::from_edge(border, text_color) else {
            continue;
        };
        let x = line.x + span.x_offset;
        if let Some((previous, bordered)) = boxes.last_mut()
            && previous == border
            && (bordered.x + bordered.width - x).abs() < 0.5
        {
            bordered.width = x + span.width - bordered.x;
            continue;
        }
        boxes.push((
            border.clone(),
            RenderBorderBox {
                page: line.page,
                x,
                y: line.y,
                width: span.width,
                height: line.height,
                top: Some(edge.clone()),
                bottom: Some(edge.clone()),
                left: Some(edge.clone()),
                right: Some(edge),
            },
        ));
    }
    boxes.into_iter().map(|(_, bordered)| bordered).collect()
}

/// Draws `bordered`'s sides.
pub(crate) fn push_border_box_operations(
    operations: &mut Vec<Operation>,
    bordered: &RenderBorderBox,
    page_height_pt: f64,
) {
    let RenderBorderBox {
        x,
        y,
        width,
        height,
        ..
    } = *bordered;
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let sides = [
        (&bordered.top, BorderSide::Top, x, width, y),
        (&bordered.bottom, BorderSide::Bottom, x, width, y + height),
        (&bordered.left, BorderSide::Left, y, height, x),
        (&bordered.right, BorderSide::Right, y, height, x + width),
    ];
    for (edge, side, start, length, outer) in sides {
        if let Some(edge) = edge {
            let thickness = edge.width.min(width).min(height);
            push_border_edge_operations(
                operations,
                edge,
                side,
                (start, length),
                (outer, thickness),
                page_height_pt,
            );
        }
    }
}

/// One side: `length` px along its edge from `start` (x for top and bottom,
/// y for left and right), its band `thickness` px across from `outer`, the
/// band edge away from the box's inside.
fn push_border_edge_operations(
    operations: &mut Vec<Operation>,
    edge: &RenderBorderEdge,
    side: BorderSide,
    (start, length): (f64, f64),
    (outer, thickness): (f64, f64),
    page_height_pt: f64,
) {
    if length <= 0.0 || thickness <= 0.0 {
        return;
    }
    let across = |from_outer: f64| match side {
        BorderSide::Top | BorderSide::Left => outer + from_outer,
        BorderSide::Bottom | BorderSide::Right => outer - from_outer,
    };
    let horizontal = matches!(side, BorderSide::Top | BorderSide::Bottom);
    // A point of the edge's frame (along, across) on the page, in PDF points.
    let point = |along: f64, from_outer: f64| {
        let (x, y) = if horizontal {
            (start + along, across(from_outer))
        } else {
            (across(from_outer), start + along)
        };
        (px_to_pt(x), page_height_pt - px_to_pt(y))
    };
    let line_pattern: Pattern = pattern(edge.style);
    let (line_width, polylines) = pattern_polylines(line_pattern, length, thickness);
    operations.push(Operation::new("q", vec![]));
    if !polylines.is_empty() {
        crate::push_stroke_color(operations, edge.color);
        operations.push(Operation::new(
            "w",
            vec![Object::Real(px_to_pt(line_width) as f32)],
        ));
        for line in polylines {
            for (index, (along, from_outer)) in line.into_iter().enumerate() {
                let (x, y) = point(along, from_outer);
                operations.push(Operation::new(
                    if index == 0 { "m" } else { "l" },
                    vec![Object::Real(x as f32), Object::Real(y as f32)],
                ));
            }
            operations.push(Operation::new("S", vec![]));
        }
        operations.push(Operation::new("Q", vec![]));
        return;
    }
    let runs = pattern_runs(line_pattern, length, thickness);
    for stripe in stripes(edge.style) {
        let color = match stripe.tone {
            Tone::Line => edge.color,
            tone => parse_hex_color(&tone_color(&css_color(edge.color), tone, side))
                .unwrap_or(edge.color),
        };
        crate::push_fill_color(operations, color);
        let near = stripe.start * thickness;
        let far = (stripe.start + stripe.width) * thickness;
        for (offset, run) in &runs {
            let (x0, y0) = point(*offset, near);
            let (x1, y1) = point(offset + run, far);
            operations.push(Operation::new(
                "re",
                vec![
                    Object::Real(x0.min(x1) as f32),
                    Object::Real(y0.min(y1) as f32),
                    Object::Real((x1 - x0).abs() as f32),
                    Object::Real((y1 - y0).abs() as f32),
                ],
            ));
        }
        operations.push(Operation::new("f", vec![]));
    }
    operations.push(Operation::new("Q", vec![]));
}

fn css_color(color: PdfColor) -> String {
    let channel = |value: f64| (value * 255.0).round().clamp(0.0, 255.0) as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(val: &str) -> RenderBorderEdge {
        RenderBorderEdge::from_edge(
            &BorderEdge {
                val: Some(val.into()),
                size_eighths: Some(24),
                space: Some(0),
                color: Some("#ff0000".into()),
            },
            None,
        )
        .unwrap()
    }

    fn ops_of(style: &str) -> Vec<String> {
        let mut operations = Vec::new();
        push_border_box_operations(
            &mut operations,
            &RenderBorderBox {
                page: 1,
                x: 10.0,
                y: 10.0,
                width: 200.0,
                height: 40.0,
                top: Some(edge(style)),
                bottom: None,
                left: None,
                right: None,
            },
            800.0,
        );
        operations
            .iter()
            .map(|operation| operation.operator.clone())
            .collect()
    }

    #[test]
    fn every_line_style_draws_its_own_way() {
        let count = |ops: &[String], op: &str| ops.iter().filter(|o| o.as_str() == op).count();
        assert_eq!(count(&ops_of("single"), "re"), 1);
        assert_eq!(count(&ops_of("double"), "re"), 2);
        assert_eq!(count(&ops_of("triple"), "re"), 3);
        assert_eq!(count(&ops_of("thinThickThinMediumGap"), "re"), 3);
        assert!(count(&ops_of("dashed"), "re") >= 8, "dashes along the edge");
        assert!(count(&ops_of("dotDotDash"), "re") > count(&ops_of("dotDash"), "re"));
        assert!(count(&ops_of("wave"), "l") > 20, "a wavy stroke");
        assert_eq!(count(&ops_of("doubleWave"), "S"), 2);
        assert!(count(&ops_of("dashDotStroked"), "S") > 4);
        // The 3-D styles shade: two colours across the band.
        assert_eq!(count(&ops_of("threeDEmboss"), "rg"), 2);
        let none = BorderEdge {
            val: Some("none".into()),
            ..BorderEdge::default()
        };
        assert!(RenderBorderEdge::from_edge(&none, None).is_none());
    }

    #[test]
    fn a_thin_thick_border_puts_its_thin_rule_outside() {
        let mut operations = Vec::new();
        push_border_edge_operations(
            &mut operations,
            &edge("thinThickSmallGap"),
            BorderSide::Top,
            (0.0, 100.0),
            (0.0, 6.0),
            100.0,
        );
        let rects: Vec<(f32, f32)> = operations
            .iter()
            .filter(|operation| operation.operator == "re")
            .map(|operation| {
                let y = operation.operands[1].as_float().unwrap();
                let h = operation.operands[3].as_float().unwrap();
                (y, h)
            })
            .collect();
        assert_eq!(rects.len(), 2);
        // PDF y grows upwards: the outer (top) rule is the higher one.
        assert!(rects[0].0 > rects[1].0, "{rects:?}");
        assert!(rects[0].1 < rects[1].1, "thin outside, thick inside");
    }
}
