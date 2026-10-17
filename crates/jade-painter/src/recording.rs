//! A [`Painter`] that records operations instead of drawing them.
//!
//! This is the reference non-canvas backend. It exists so drawing code can be
//! exercised without a browser: a test asserts the sequence of operations a
//! command produces, which is the part that must stay identical when a real
//! backend such as Skia is added later.

use crate::{ImageRect, ImageSource, Painter, TextMetrics};
use core::convert::Infallible;
use std::cell::RefCell;
use std::rc::Rc;

/// One recorded drawing operation.
///
/// Floats are kept as-is rather than rounded so a test can assert exact
/// geometry; callers comparing positions should round at the assertion.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    DrawImage {
        key: String,
        source: Option<[f64; 4]>,
        destination: ImageRect,
    },
    Save,
    Restore,
    Translate(f64, f64),
    Scale(f64, f64),
    Rotate(f64),
    SetFillStyle(String),
    SetStrokeStyle(String),
    SetLineWidth(f64),
    SetLineDash(Vec<f64>),
    SetShadowColor(String),
    SetShadowBlur(f64),
    SetShadowOffsetX(f64),
    SetShadowOffsetY(f64),
    SetBlurFilter(f64),
    SetFont(String),
    SetTextBaseline(String),
    FillText(String, f64, f64),
    StrokeText(String, f64, f64),
    MeasureText(String),
    FillRect(f64, f64, f64, f64),
    HorizontalGradientRect(ImageRect, [u8; 4], [u8; 4]),
    ClearRect(f64, f64, f64, f64),
    StrokeRect(f64, f64, f64, f64),
    SetGlobalAlpha(f64),
    BeginPath,
    ClosePath,
    MoveTo(f64, f64),
    LineTo(f64, f64),
    Rect(f64, f64, f64, f64),
    Arc(f64, f64, f64, f64, f64),
    QuadraticCurveTo(f64, f64, f64, f64),
    BezierCurveTo(f64, f64, f64, f64, f64, f64),
    EllipseAnticlockwise(f64, f64, f64, f64, f64, f64, f64, bool),
    Ellipse(f64, f64, f64, f64, f64, f64, f64),
    Fill,
    Stroke,
    Clip,
}

/// Records every [`Painter`] call. Cheap to clone; clones share one log.
#[derive(Clone, Default)]
pub struct RecordingPainter {
    ops: Rc<RefCell<Vec<Op>>>,
    /// Last value passed to [`Painter::set_fill_style_str`], so
    /// [`Painter::use_fill_style_for_stroke`] can resolve to a concrete colour
    /// and tests can assert what a decoration was actually drawn in.
    last_fill: Rc<RefCell<String>>,
    /// Width returned by [`Painter::measure_text_width`], per character of the
    /// measured string. Real metrics are not this backend's job.
    width_per_char: f64,
}

impl RecordingPainter {
    pub fn new() -> Self {
        Self {
            ops: Rc::new(RefCell::new(Vec::new())),
            last_fill: Rc::new(RefCell::new(String::new())),
            width_per_char: 0.0,
        }
    }

    /// Makes [`Painter::measure_text_width`] return `width_per_char` for each
    /// character, so code paths that branch on measured width can be driven.
    pub fn with_width_per_char(width_per_char: f64) -> Self {
        Self {
            ops: Rc::new(RefCell::new(Vec::new())),
            last_fill: Rc::new(RefCell::new(String::new())),
            width_per_char,
        }
    }

    pub fn ops(&self) -> Vec<Op> {
        self.ops.borrow().clone()
    }

    pub fn clear(&self) {
        self.ops.borrow_mut().clear();
    }

    /// Number of recorded operations matching `predicate`.
    pub fn count(&self, predicate: impl Fn(&Op) -> bool) -> usize {
        self.ops.borrow().iter().filter(|op| predicate(op)).count()
    }

    fn push(&self, op: Op) {
        self.ops.borrow_mut().push(op);
    }
}

impl Painter for RecordingPainter {
    type Error = Infallible;
    fn draw_image(
        &self,
        image: ImageSource<'_>,
        source: Option<[f64; 4]>,
        destination: ImageRect,
    ) -> Result<bool, Self::Error> {
        self.ops.borrow_mut().push(Op::DrawImage {
            key: image.key.to_string(),
            source,
            destination,
        });
        Ok(true)
    }

    fn save(&self) {
        self.push(Op::Save);
    }

    fn restore(&self) {
        self.push(Op::Restore);
    }

    fn translate(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.push(Op::Translate(x, y));
        Ok(())
    }

    fn scale(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.push(Op::Scale(x, y));
        Ok(())
    }

    fn rotate(&self, angle: f64) -> Result<(), Self::Error> {
        self.push(Op::Rotate(angle));
        Ok(())
    }

    fn set_fill_style_str(&self, value: &str) {
        *self.last_fill.borrow_mut() = value.to_string();
        self.push(Op::SetFillStyle(value.to_string()));
    }

    fn set_stroke_style_str(&self, value: &str) {
        self.push(Op::SetStrokeStyle(value.to_string()));
    }

    fn set_line_width(&self, width: f64) {
        self.push(Op::SetLineWidth(width));
    }

    fn set_global_alpha(&self, alpha: f64) {
        self.push(Op::SetGlobalAlpha(alpha));
    }

    fn set_line_dash(&self, segments: &[f64]) {
        self.push(Op::SetLineDash(segments.to_vec()));
    }

    fn set_shadow_color(&self, value: &str) {
        self.push(Op::SetShadowColor(value.to_string()));
    }

    fn set_shadow_blur(&self, value: f64) {
        self.push(Op::SetShadowBlur(value));
    }

    fn set_shadow_offset_x(&self, value: f64) {
        self.push(Op::SetShadowOffsetX(value));
    }

    fn set_shadow_offset_y(&self, value: f64) {
        self.push(Op::SetShadowOffsetY(value));
    }

    fn set_blur_filter(&self, radius_px: f64) {
        self.push(Op::SetBlurFilter(radius_px));
    }

    fn set_font(&self, value: &str) {
        self.push(Op::SetFont(value.to_string()));
    }

    fn set_text_baseline(&self, value: &str) {
        self.push(Op::SetTextBaseline(value.to_string()));
    }

    fn fill_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.push(Op::FillText(text.to_string(), x, y));
        Ok(())
    }

    fn stroke_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.push(Op::StrokeText(text.to_string(), x, y));
        Ok(())
    }

    fn measure_text_width(&self, text: &str) -> Result<f64, Self::Error> {
        self.push(Op::MeasureText(text.to_string()));
        Ok(text.chars().count() as f64 * self.width_per_char)
    }

    /// Only `width` is populated: this backend has no glyph outlines to derive
    /// an ink box from, and tests that need real metrics use a real backend.
    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, Self::Error> {
        Ok(TextMetrics {
            width: self.measure_text_width(text)?,
            ..TextMetrics::default()
        })
    }

    fn fill_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.push(Op::FillRect(x, y, width, height));
    }

    fn fill_horizontal_gradient_rect(
        &self,
        rect: ImageRect,
        start: [u8; 4],
        end: [u8; 4],
    ) -> Result<(), Self::Error> {
        self.push(Op::HorizontalGradientRect(rect, start, end));
        Ok(())
    }

    fn clear_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.push(Op::ClearRect(x, y, width, height));
    }

    fn stroke_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.push(Op::StrokeRect(x, y, width, height));
    }

    fn begin_path(&self) {
        self.push(Op::BeginPath);
    }

    fn close_path(&self) {
        self.push(Op::ClosePath);
    }

    fn move_to(&self, x: f64, y: f64) {
        self.push(Op::MoveTo(x, y));
    }

    fn line_to(&self, x: f64, y: f64) {
        self.push(Op::LineTo(x, y));
    }

    fn rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.push(Op::Rect(x, y, width, height));
    }

    fn arc(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.push(Op::Arc(x, y, radius, start_angle, end_angle));
        Ok(())
    }

    fn quadratic_curve_to(&self, cp_x: f64, cp_y: f64, x: f64, y: f64) {
        self.push(Op::QuadraticCurveTo(cp_x, cp_y, x, y));
    }

    fn bezier_curve_to(&self, cp1_x: f64, cp1_y: f64, cp2_x: f64, cp2_y: f64, x: f64, y: f64) {
        self.push(Op::BezierCurveTo(cp1_x, cp1_y, cp2_x, cp2_y, x, y));
    }

    fn ellipse_with_anticlockwise(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
        anticlockwise: bool,
    ) -> Result<(), Self::Error> {
        self.push(Op::EllipseAnticlockwise(
            x,
            y,
            radius_x,
            radius_y,
            rotation,
            start_angle,
            end_angle,
            anticlockwise,
        ));
        Ok(())
    }

    fn ellipse(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.push(Op::Ellipse(
            x,
            y,
            radius_x,
            radius_y,
            rotation,
            start_angle,
            end_angle,
        ));
        Ok(())
    }

    fn fill(&self) {
        self.push(Op::Fill);
    }

    fn stroke(&self) {
        self.push(Op::Stroke);
    }

    fn clip(&self) {
        self.push(Op::Clip);
    }

    fn use_fill_style_for_stroke(&self) {
        let fill = self.last_fill.borrow().clone();
        self.push(Op::SetStrokeStyle(fill));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_metrics_carry_the_advance_and_nothing_it_cannot_know() {
        let painter = RecordingPainter::with_width_per_char(7.0);

        let metrics = painter.measure_text_metrics("abc").expect("infallible");

        assert_eq!(metrics.width, 21.0);
        // A recorder has no glyph outlines, so the ink box stays zero rather
        // than guessing. Code that needs real vertical metrics must run against
        // a backend that has fonts.
        assert_eq!(metrics.actual_bounding_box_ascent, 0.0);
        assert_eq!(metrics.font_bounding_box_ascent, 0.0);
        assert_eq!(metrics.actual_bounding_box_right, 0.0);
    }

    #[test]
    fn measuring_is_recorded_so_a_test_can_see_what_was_asked_for() {
        let painter = RecordingPainter::new();

        painter.measure_text_width("横浜").expect("infallible");

        assert_eq!(painter.ops(), vec![Op::MeasureText("横浜".to_string())]);
    }
}
