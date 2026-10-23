//! SVG data from document ink Painter operations; no separate brush rules.
use super::CanvasRenderer;
use jade_docx_reader::inkml::InkTrace;
use jade_painter::recording::{Op, RecordingPainter};
use serde::Serialize;
use std::fmt::Write;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InkPreviewPath {
    pub d: String,
    pub fill: String,
    pub stroke: String,
    pub stroke_width: f64,
    pub opacity: f64,
}
#[derive(Clone)]
struct Style {
    fill: String,
    stroke: String,
    width: f64,
    alpha: f64,
}

pub(crate) fn ink_trace_preview(trace: &InkTrace) -> Result<Vec<InkPreviewPath>, String> {
    if trace
        .points
        .iter()
        .any(|(x, y)| !x.is_finite() || !y.is_finite())
        || !trace.brush.width_px.is_finite()
        || trace.brush.width_px <= 0.0
        || !trace.brush.height_px.is_finite()
        || trace.brush.height_px <= 0.0
    {
        return Err("ink preview needs finite points and positive brush dimensions".into());
    }
    let painter = RecordingPainter::new();
    let _ =
        CanvasRenderer::<RecordingPainter>::draw_ink_trace(&painter, trace, &|&point| point, 1.0);
    let mut style = Style {
        fill: "#000000".into(),
        stroke: "#000000".into(),
        width: 1.0,
        alpha: 1.0,
    };
    let mut stack = Vec::new();
    let mut d = String::new();
    let mut paths = Vec::new();
    for op in painter.ops() {
        match op {
            Op::Save => stack.push(style.clone()),
            Op::Restore => style = stack.pop().ok_or("unbalanced ink painter state")?,
            Op::SetFillStyle(color) => style.fill = color,
            Op::SetStrokeStyle(color) => style.stroke = color,
            Op::SetLineWidth(width) => style.width = width,
            Op::SetGlobalAlpha(alpha) => style.alpha = alpha,
            Op::BeginPath => d.clear(),
            Op::ClosePath => d.push_str("Z "),
            Op::MoveTo(x, y) => write!(d, "M{x} {y} ").unwrap(),
            Op::LineTo(x, y) => write!(d, "L{x} {y} ").unwrap(),
            // The native ink painter uses complete circles for round caps.
            Op::Arc(x, y, r, start, end) if start == 0.0 && end == std::f64::consts::TAU => {
                write!(
                    d,
                    "A{r} {r} 0 1 1 {} {y} A{r} {r} 0 1 1 {} {y} ",
                    x - r,
                    x + r
                )
                .unwrap();
            }
            Op::Fill | Op::Stroke => {
                let fill = matches!(op, Op::Fill);
                paths.push(InkPreviewPath {
                    d: d.clone(),
                    fill: if fill {
                        style.fill.clone()
                    } else {
                        "none".into()
                    },
                    stroke: if fill {
                        "none".into()
                    } else {
                        style.stroke.clone()
                    },
                    stroke_width: style.width,
                    opacity: style.alpha,
                });
            }
            other => return Err(format!("unsupported ink preview operation: {other:?}")),
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_docx_reader::inkml::{InkBrush, InkTool};
    fn trace(tool: InkTool) -> InkTrace {
        InkTrace {
            points: vec![(10.0, 20.0), (10.0, 40.0), (30.0, 40.0)],
            brush: InkBrush {
                tool,
                color: "#FFFF00".into(),
                width_px: 2.0,
                height_px: 12.0,
                effect: None,
                transparency: 0,
            },
        }
    }
    #[test]
    fn chisel_uses_nib_width_for_vertical_strokes_and_fills_joins_once() {
        let trace = trace(InkTool::Highlighter);
        let painter = RecordingPainter::new();
        let _ = CanvasRenderer::<RecordingPainter>::draw_ink_trace(&painter, &trace, &|&p| p, 1.0);
        let ops = painter.ops();
        assert_eq!(ops.iter().filter(|op| matches!(op, Op::Fill)).count(), 1);
        assert!(!ops.iter().any(|op| matches!(op, Op::Arc(..) | Op::Stroke)));
        let paths = ink_trace_preview(&trace).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].opacity, 0.45);
        assert_eq!(paths[0].fill, "#FFFF00");
        assert_eq!(
            paths[0].d,
            "M9 14 L11 14 L11 46 L9 46 Z M9 34 L31 34 L31 46 L9 46 Z "
        );
    }
    #[test]
    fn singleton_and_diagonal_chisel_keep_rectangle_footprint() {
        let mut trace = trace(InkTool::Highlighter);
        trace.points.truncate(1);
        assert_eq!(
            ink_trace_preview(&trace).unwrap()[0].d,
            "M9 14 L11 14 L11 26 L9 26 Z "
        );
        trace.points.push((30.0, 40.0));
        assert_eq!(
            ink_trace_preview(&trace).unwrap()[0].d,
            "M9 14 L11 14 L31 34 L31 46 L29 46 L9 26 Z "
        );
    }
    #[test]
    fn every_native_material_has_preview_and_retains_transparency() {
        for effect in [
            None,
            Some("rainbow"),
            Some("galaxy"),
            Some("gold"),
            Some("silver"),
            Some("bronze"),
            Some("rosegold"),
            Some("lava"),
            Some("ocean"),
        ] {
            for tool in [InkTool::Pen, InkTool::Pencil, InkTool::Highlighter] {
                let mut trace = trace(tool);
                trace.brush.effect = effect.map(str::to_string);
                trace.brush.transparency = 128;
                let paths = ink_trace_preview(&trace).unwrap();
                assert!(!paths.is_empty());
                assert!(paths
                    .iter()
                    .all(|p| p.opacity <= 127.0 / 255.0 && !p.d.is_empty()));
                if effect == Some("rainbow") {
                    assert!(paths
                        .iter()
                        .any(|p| p.stroke == "#E71224" || p.fill == "#E71224"));
                }
            }
        }
    }
    #[test]
    fn invalid_preview_geometry_is_rejected() {
        let mut trace = trace(InkTool::Pen);
        trace.points[0].0 = f64::NAN;
        assert!(ink_trace_preview(&trace).is_err());
        trace.points.clear();
        trace.brush.height_px = 0.0;
        assert!(ink_trace_preview(&trace).is_err());
    }
}
