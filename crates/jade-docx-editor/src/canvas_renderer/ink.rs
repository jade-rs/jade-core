//! Word ink drawn from its strokes (`RunImage::ink`): pens as round strokes,
//! the pencil grainier, the highlighter a translucent flat tip, and Office's
//! ink effects (rainbow, galaxy, metallics, lava, ocean) as colour along or
//! across the stroke.

use super::*;
use jade_docx_reader::inkml::{InkContent, InkTool, InkTrace};

/// Where a stroke's point lands on the page.
type Place<'a> = dyn Fn(&(f64, f64)) -> (f64, f64) + 'a;

/// An effect's colours along a stroke.
fn effect_colors(effect: &str) -> Option<&'static [&'static str]> {
    Some(match effect {
        "rainbow" => &[
            "#E71224", "#F7630C", "#FFC114", "#16C60C", "#0078D7", "#886CE4",
        ],
        "galaxy" => &["#3B1F6B", "#6E2C9E", "#2B4FA3", "#5B2A86"],
        "gold" => &["#B8860B", "#E6C35C", "#C9A227", "#F2D27A"],
        "silver" => &["#8E8E8E", "#D9D9D9", "#A6A6A6", "#EDEDED"],
        "bronze" => &["#8C5A2B", "#C88A4E", "#A0683A", "#D9A066"],
        "rosegold" => &["#B76E79", "#E8B4B8", "#C98B8F", "#F1C9C7"],
        "lava" => &["#7A0000", "#E81123", "#FF8C00", "#E81123"],
        "ocean" => &["#004E8C", "#0099BC", "#00B7C3", "#0063B1"],
        _ => return None,
    })
}

impl<HostPainter: Painter> CanvasRenderer<HostPainter>
where
    HostPainter::Error: PainterError,
{
    /// Draws `ink`'s strokes in the box `draw_spec` gives the drawing whose
    /// natural size is `natural` (the box the strokes are in).
    pub(super) fn draw_ink(
        &self,
        ink: &InkContent,
        draw_spec: ImageDrawSpec,
        natural: (f64, f64),
    ) -> Result<(), JsValue> {
        if !self.should_draw_current_page_for_render() {
            return Ok(());
        }
        let painter = self.host.painter();
        let sx = draw_spec.width / natural.0.max(1e-6);
        let sy = draw_spec.height / natural.1.max(1e-6);
        let place = |&(x, y): &(f64, f64)| (draw_spec.x + x * sx, draw_spec.y + y * sy);
        painter.save();
        for trace in &ink.traces {
            Self::draw_ink_trace(&painter, trace, &place, sx.min(sy))
                .map_err(PainterError::into_js_value)?;
        }
        painter.restore();
        self.invalidate_context_state_tracking();
        Ok(())
    }

    pub(crate) fn draw_ink_trace<P: Painter>(
        painter: &P,
        trace: &InkTrace,
        place: &Place<'_>,
        scale: f64,
    ) -> Result<(), P::Error> {
        let points: Vec<(f64, f64)> = trace.points.iter().map(place).collect();
        if points.is_empty() {
            return Ok(());
        }
        let brush = &trace.brush;
        let opacity = (1.0 - f64::from(brush.transparency) / 255.0)
            * match brush.tool {
                InkTool::Highlighter => 0.45,
                InkTool::Pencil => 0.75,
                InkTool::Pen => 1.0,
            };
        let width = match brush.tool {
            // The highlighter's tip is a tall rectangle.
            InkTool::Highlighter => brush.height_px.max(brush.width_px),
            _ => brush.width_px,
        } * scale;
        let width = width.max(0.5);
        painter.save();
        painter.set_global_alpha(opacity);
        let round = brush.tool != InkTool::Highlighter;
        let paint = |points: &[(f64, f64)], color: &str, width: f64, round: bool| {
            if brush.tool == InkTool::Highlighter {
                Self::ink_chisel(
                    painter,
                    points,
                    color,
                    (brush.width_px * scale).max(0.5),
                    (brush.height_px * scale).max(0.5),
                )
            } else {
                Self::ink_polyline(painter, points, color, width, round)
            }
        };
        match brush.effect.as_deref().and_then(effect_colors) {
            Some(colors) => {
                // Colour changes along the stroke, a band at a time.
                let band = (points.len() / (colors.len() * 2)).max(1);
                for (index, pair) in points.windows(2).enumerate() {
                    let color = colors[(index / band) % colors.len()];
                    paint(pair, color, width, round && opacity >= 1.0)?;
                }
                if points.len() == 1 {
                    paint(&points, colors[0], width, round)?;
                }
                // Metallics and galaxy catch the light down the middle.
                if brush.effect.as_deref().is_some_and(|effect| {
                    matches!(effect, "gold" | "silver" | "bronze" | "rosegold" | "galaxy")
                }) {
                    painter.set_global_alpha(opacity * 0.6);
                    Self::ink_polyline(painter, &points, "#FFFFFF", width * 0.25, round)?;
                }
            }
            None => {
                paint(&points, &brush.color, width, round && opacity >= 1.0)?;
                if brush.tool == InkTool::Pencil && width > 1.5 {
                    // Graphite grain: a lighter core.
                    painter.set_global_alpha(opacity * 0.35);
                    Self::ink_polyline(painter, &points, "#FFFFFF", width * 0.3, true)?;
                }
            }
        }
        painter.restore();
        Ok(())
    }

    /// Sweep an axis-aligned rectangular nib. All segment hulls have the same
    /// winding and share one fill, so translucent joins do not darken.
    fn ink_chisel<P: Painter>(
        painter: &P,
        points: &[(f64, f64)],
        color: &str,
        width: f64,
        height: f64,
    ) -> Result<(), P::Error> {
        painter.set_fill_style_str(color);
        painter.begin_path();
        let pairs: Vec<_> = if points.len() == 1 {
            vec![(&points[0], &points[0])]
        } else {
            points.windows(2).map(|p| (&p[0], &p[1])).collect()
        };
        for (a, b) in pairs {
            let mut corners = Vec::with_capacity(8);
            for &(x, y) in [a, b] {
                for (dx, dy) in [
                    (-width / 2.0, -height / 2.0),
                    (width / 2.0, -height / 2.0),
                    (width / 2.0, height / 2.0),
                    (-width / 2.0, height / 2.0),
                ] {
                    corners.push((x + dx, y + dy));
                }
            }
            corners.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            corners.dedup();
            let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
                (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
            };
            let mut hull = Vec::new();
            for &point in &corners {
                while hull.len() >= 2
                    && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0
                {
                    hull.pop();
                }
                hull.push(point);
            }
            let lower = hull.len();
            for &point in corners.iter().rev().skip(1) {
                while hull.len() > lower
                    && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0
                {
                    hull.pop();
                }
                hull.push(point);
            }
            hull.pop();
            painter.move_to(hull[0].0, hull[0].1);
            for &(x, y) in &hull[1..] {
                painter.line_to(x, y);
            }
            painter.close_path();
        }
        painter.fill();
        Ok(())
    }

    /// Strokes a polyline, with round joints and ends when `round`.
    fn ink_polyline<P: Painter>(
        painter: &P,
        points: &[(f64, f64)],
        color: &str,
        width: f64,
        round: bool,
    ) -> Result<(), P::Error> {
        painter.set_stroke_style_str(color);
        painter.set_fill_style_str(color);
        painter.set_line_width(width);
        if points.len() > 1 {
            painter.begin_path();
            painter.move_to(points[0].0, points[0].1);
            for point in &points[1..] {
                painter.line_to(point.0, point.1);
            }
            painter.stroke();
        }
        // Round joints, where overlapping does not show; otherwise round ends.
        let ends = [points[0], points[points.len() - 1]];
        let dots: &[(f64, f64)] = if round { points } else { &ends };
        {
            painter.begin_path();
            for &(x, y) in dots {
                painter.move_to(x + width / 2.0, y);
                painter.arc(x, y, width / 2.0, 0.0, std::f64::consts::TAU)?;
            }
            painter.fill();
        }
        Ok(())
    }
}
