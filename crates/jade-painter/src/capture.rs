//! Deferred painting: capture drawing now, paint it later.
//!
//! Word paints a page in layers — objects behind text, then the text, then
//! objects in front of text, each layer in `relativeHeight` order — while a
//! renderer reaches those objects one anchor paragraph at a time. A
//! [`RoutedPainter`] lets the renderer keep its single walk: while a capture
//! is active, drawing calls are stored as [`PaintOp`]s instead of reaching the
//! surface, and [`replay`] paints them onto the real painter once the page's
//! layer order is known.
//!
//! Measurement always reaches the real painter, and so does `set_font`
//! (besides being recorded), because text measured during a capture must use
//! the font the capture selected.

use crate::{ImageRect, ImageSource, Painter, TextMetrics};
use std::cell::RefCell;
use std::rc::Rc;

/// One captured drawing call, with its arguments owned.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintOp {
    DrawImage {
        key: Rc<str>,
        bytes: Rc<[u8]>,
        source: Option<[f64; 4]>,
        destination: ImageRect,
    },
    Save,
    Restore,
    Translate(f64, f64),
    Scale(f64, f64),
    Rotate(f64),
    SetFillStyle(Rc<str>),
    SetStrokeStyle(Rc<str>),
    SetLineWidth(f64),
    SetGlobalAlpha(f64),
    SetLineDash(Vec<f64>),
    SetShadowColor(Rc<str>),
    SetShadowBlur(f64),
    SetShadowOffsetX(f64),
    SetShadowOffsetY(f64),
    SetBlurFilter(f64),
    SetFont(Rc<str>),
    SetTextBaseline(Rc<str>),
    FillText(Rc<str>, f64, f64),
    /// [`Painter::try_fill_shaped_text`]: the text, its clusters (byte
    /// offset, x) and `y`; drawn a cluster at a time at replay when the
    /// painter cannot shape it.
    FillShapedText(Rc<str>, Rc<[(usize, f64)]>, f64),
    StrokeText(Rc<str>, f64, f64),
    FillRect(f64, f64, f64, f64),
    HorizontalGradientRect(ImageRect, [u8; 4], [u8; 4]),
    ClearRect(f64, f64, f64, f64),
    StrokeRect(f64, f64, f64, f64),
    BeginPath,
    ClosePath,
    MoveTo(f64, f64),
    LineTo(f64, f64),
    Rect(f64, f64, f64, f64),
    Arc(f64, f64, f64, f64, f64),
    QuadraticCurveTo(f64, f64, f64, f64),
    BezierCurveTo(f64, f64, f64, f64, f64, f64),
    Ellipse(f64, f64, f64, f64, f64, f64, f64),
    EllipseAnticlockwise(f64, f64, f64, f64, f64, f64, f64, bool),
    Fill,
    Stroke,
    Clip,
    UseFillStyleForStroke,
    /// A drawing step the capturing code performs itself at replay time —
    /// typically a host-level image draw that does not go through
    /// [`Painter`]. The number identifies it to the caller of [`replay`].
    Marker(usize),
}

/// The shared buffer a capture writes into.
pub type CaptureSink = Rc<RefCell<Vec<PaintOp>>>;

/// Creates an empty capture buffer.
pub fn new_sink() -> CaptureSink {
    Rc::new(RefCell::new(Vec::new()))
}

/// A painter that either forwards to `inner` or, while `sink` is set, records.
#[derive(Clone)]
pub struct RoutedPainter<P> {
    inner: P,
    sink: Option<CaptureSink>,
}

impl<P> RoutedPainter<P> {
    /// Paints straight onto `inner`.
    pub fn direct(inner: P) -> Self {
        Self { inner, sink: None }
    }

    /// Records into `sink` when it is set, otherwise paints onto `inner`.
    pub fn new(inner: P, sink: Option<CaptureSink>) -> Self {
        Self { inner, sink }
    }

    /// The painter the drawing would reach without a capture.
    pub fn inner(&self) -> &P {
        &self.inner
    }

    /// Whether drawing is currently being recorded.
    pub fn is_capturing(&self) -> bool {
        self.sink.is_some()
    }

    /// Records `op` and reports true, or reports false when not capturing.
    fn record(&self, op: impl FnOnce() -> PaintOp) -> bool {
        match &self.sink {
            Some(sink) => {
                sink.borrow_mut().push(op());
                true
            }
            None => false,
        }
    }
}

/// Paints `ops` onto `painter`; `on_marker` performs each [`PaintOp::Marker`].
pub fn replay<P: Painter>(
    ops: &[PaintOp],
    painter: &P,
    mut on_marker: impl FnMut(usize),
) -> Result<(), P::Error> {
    for op in ops {
        match op {
            PaintOp::DrawImage {
                key,
                bytes,
                source,
                destination,
            } => {
                painter.draw_image(ImageSource { key, bytes }, *source, *destination)?;
            }
            PaintOp::Save => painter.save(),
            PaintOp::Restore => painter.restore(),
            PaintOp::Translate(x, y) => painter.translate(*x, *y)?,
            PaintOp::Scale(x, y) => painter.scale(*x, *y)?,
            PaintOp::Rotate(angle) => painter.rotate(*angle)?,
            PaintOp::SetFillStyle(value) => painter.set_fill_style_str(value),
            PaintOp::SetStrokeStyle(value) => painter.set_stroke_style_str(value),
            PaintOp::SetLineWidth(width) => painter.set_line_width(*width),
            PaintOp::SetGlobalAlpha(alpha) => painter.set_global_alpha(*alpha),
            PaintOp::SetLineDash(segments) => painter.set_line_dash(segments),
            PaintOp::SetShadowColor(value) => painter.set_shadow_color(value),
            PaintOp::SetShadowBlur(value) => painter.set_shadow_blur(*value),
            PaintOp::SetShadowOffsetX(value) => painter.set_shadow_offset_x(*value),
            PaintOp::SetShadowOffsetY(value) => painter.set_shadow_offset_y(*value),
            PaintOp::SetBlurFilter(radius) => painter.set_blur_filter(*radius),
            PaintOp::SetFont(value) => painter.set_font(value),
            PaintOp::SetTextBaseline(value) => painter.set_text_baseline(value),
            PaintOp::FillText(text, x, y) => painter.fill_text(text, *x, *y)?,
            PaintOp::FillShapedText(text, clusters, y) => {
                if !painter.try_fill_shaped_text(text, clusters, *y)? {
                    for (index, (start, x)) in clusters.iter().enumerate() {
                        let end = clusters
                            .get(index + 1)
                            .map_or(text.len(), |(next, _)| *next);
                        if let Some(cluster) = text.get(*start..end) {
                            painter.fill_text(cluster, *x, *y)?;
                        }
                    }
                }
            }
            PaintOp::StrokeText(text, x, y) => painter.stroke_text(text, *x, *y)?,
            PaintOp::FillRect(x, y, w, h) => painter.fill_rect(*x, *y, *w, *h),
            PaintOp::HorizontalGradientRect(rect, start, end) => {
                painter.fill_horizontal_gradient_rect(*rect, *start, *end)?
            }
            PaintOp::ClearRect(x, y, w, h) => painter.clear_rect(*x, *y, *w, *h),
            PaintOp::StrokeRect(x, y, w, h) => painter.stroke_rect(*x, *y, *w, *h),
            PaintOp::BeginPath => painter.begin_path(),
            PaintOp::ClosePath => painter.close_path(),
            PaintOp::MoveTo(x, y) => painter.move_to(*x, *y),
            PaintOp::LineTo(x, y) => painter.line_to(*x, *y),
            PaintOp::Rect(x, y, w, h) => painter.rect(*x, *y, *w, *h),
            PaintOp::Arc(x, y, r, start, end) => painter.arc(*x, *y, *r, *start, *end)?,
            PaintOp::QuadraticCurveTo(cx, cy, x, y) => painter.quadratic_curve_to(*cx, *cy, *x, *y),
            PaintOp::BezierCurveTo(c1x, c1y, c2x, c2y, x, y) => {
                painter.bezier_curve_to(*c1x, *c1y, *c2x, *c2y, *x, *y)
            }
            PaintOp::Ellipse(x, y, rx, ry, rotation, start, end) => {
                painter.ellipse(*x, *y, *rx, *ry, *rotation, *start, *end)?
            }
            PaintOp::EllipseAnticlockwise(x, y, rx, ry, rotation, start, end, anticlockwise) => {
                painter.ellipse_with_anticlockwise(
                    *x,
                    *y,
                    *rx,
                    *ry,
                    *rotation,
                    *start,
                    *end,
                    *anticlockwise,
                )?
            }
            PaintOp::Fill => painter.fill(),
            PaintOp::Stroke => painter.stroke(),
            PaintOp::Clip => painter.clip(),
            PaintOp::UseFillStyleForStroke => painter.use_fill_style_for_stroke(),
            PaintOp::Marker(id) => on_marker(*id),
        }
    }
    Ok(())
}

impl<P: Painter> Painter for RoutedPainter<P> {
    type Error = P::Error;

    fn draw_image(
        &self,
        image: ImageSource<'_>,
        source: Option<[f64; 4]>,
        destination: ImageRect,
    ) -> Result<bool, Self::Error> {
        if self.record(|| PaintOp::DrawImage {
            key: Rc::from(image.key),
            bytes: Rc::from(image.bytes),
            source,
            destination,
        }) {
            return Ok(true);
        }
        self.inner.draw_image(image, source, destination)
    }

    fn save(&self) {
        if !self.record(|| PaintOp::Save) {
            self.inner.save();
        }
    }

    fn restore(&self) {
        if !self.record(|| PaintOp::Restore) {
            self.inner.restore();
        }
    }

    fn translate(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        if self.record(|| PaintOp::Translate(x, y)) {
            return Ok(());
        }
        self.inner.translate(x, y)
    }

    fn scale(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        if self.record(|| PaintOp::Scale(x, y)) {
            return Ok(());
        }
        self.inner.scale(x, y)
    }

    fn rotate(&self, angle: f64) -> Result<(), Self::Error> {
        if self.record(|| PaintOp::Rotate(angle)) {
            return Ok(());
        }
        self.inner.rotate(angle)
    }

    fn set_fill_style_str(&self, value: &str) {
        if !self.record(|| PaintOp::SetFillStyle(Rc::from(value))) {
            self.inner.set_fill_style_str(value);
        }
    }

    fn set_stroke_style_str(&self, value: &str) {
        if !self.record(|| PaintOp::SetStrokeStyle(Rc::from(value))) {
            self.inner.set_stroke_style_str(value);
        }
    }

    fn set_line_width(&self, width: f64) {
        if !self.record(|| PaintOp::SetLineWidth(width)) {
            self.inner.set_line_width(width);
        }
    }

    fn set_global_alpha(&self, alpha: f64) {
        if !self.record(|| PaintOp::SetGlobalAlpha(alpha)) {
            self.inner.set_global_alpha(alpha);
        }
    }

    fn set_line_dash(&self, segments: &[f64]) {
        if !self.record(|| PaintOp::SetLineDash(segments.to_vec())) {
            self.inner.set_line_dash(segments);
        }
    }

    fn set_shadow_color(&self, value: &str) {
        if !self.record(|| PaintOp::SetShadowColor(Rc::from(value))) {
            self.inner.set_shadow_color(value);
        }
    }

    fn set_shadow_blur(&self, value: f64) {
        if !self.record(|| PaintOp::SetShadowBlur(value)) {
            self.inner.set_shadow_blur(value);
        }
    }

    fn set_shadow_offset_x(&self, value: f64) {
        if !self.record(|| PaintOp::SetShadowOffsetX(value)) {
            self.inner.set_shadow_offset_x(value);
        }
    }

    fn set_shadow_offset_y(&self, value: f64) {
        if !self.record(|| PaintOp::SetShadowOffsetY(value)) {
            self.inner.set_shadow_offset_y(value);
        }
    }

    fn set_blur_filter(&self, radius_px: f64) {
        if !self.record(|| PaintOp::SetBlurFilter(radius_px)) {
            self.inner.set_blur_filter(radius_px);
        }
    }

    /// Recorded *and* applied: text measured inside a capture must see the
    /// font the capture chose.
    fn set_font(&self, value: &str) {
        self.record(|| PaintOp::SetFont(Rc::from(value)));
        self.inner.set_font(value);
    }

    fn set_text_baseline(&self, value: &str) {
        if !self.record(|| PaintOp::SetTextBaseline(Rc::from(value))) {
            self.inner.set_text_baseline(value);
        }
    }

    fn fill_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        if self.record(|| PaintOp::FillText(Rc::from(text), x, y)) {
            return Ok(());
        }
        self.inner.fill_text(text, x, y)
    }

    /// Recorded whole, so the shaping (a ligature for its letters) happens
    /// when the capture is painted.
    fn try_fill_shaped_text(
        &self,
        text: &str,
        clusters: &[(usize, f64)],
        y: f64,
    ) -> Result<bool, Self::Error> {
        if self.record(|| PaintOp::FillShapedText(Rc::from(text), Rc::from(clusters), y)) {
            return Ok(true);
        }
        self.inner.try_fill_shaped_text(text, clusters, y)
    }

    fn stroke_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        if self.record(|| PaintOp::StrokeText(Rc::from(text), x, y)) {
            return Ok(());
        }
        self.inner.stroke_text(text, x, y)
    }

    fn measure_text_width(&self, text: &str) -> Result<f64, Self::Error> {
        self.inner.measure_text_width(text)
    }

    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, Self::Error> {
        self.inner.measure_text_metrics(text)
    }

    fn fill_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        if !self.record(|| PaintOp::FillRect(x, y, width, height)) {
            self.inner.fill_rect(x, y, width, height);
        }
    }

    fn fill_horizontal_gradient_rect(
        &self,
        rect: ImageRect,
        start: [u8; 4],
        end: [u8; 4],
    ) -> Result<(), Self::Error> {
        if self.record(|| PaintOp::HorizontalGradientRect(rect, start, end)) {
            return Ok(());
        }
        self.inner.fill_horizontal_gradient_rect(rect, start, end)
    }

    fn clear_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        if !self.record(|| PaintOp::ClearRect(x, y, width, height)) {
            self.inner.clear_rect(x, y, width, height);
        }
    }

    fn stroke_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        if !self.record(|| PaintOp::StrokeRect(x, y, width, height)) {
            self.inner.stroke_rect(x, y, width, height);
        }
    }

    fn begin_path(&self) {
        if !self.record(|| PaintOp::BeginPath) {
            self.inner.begin_path();
        }
    }

    fn close_path(&self) {
        if !self.record(|| PaintOp::ClosePath) {
            self.inner.close_path();
        }
    }

    fn move_to(&self, x: f64, y: f64) {
        if !self.record(|| PaintOp::MoveTo(x, y)) {
            self.inner.move_to(x, y);
        }
    }

    fn line_to(&self, x: f64, y: f64) {
        if !self.record(|| PaintOp::LineTo(x, y)) {
            self.inner.line_to(x, y);
        }
    }

    fn rect(&self, x: f64, y: f64, width: f64, height: f64) {
        if !self.record(|| PaintOp::Rect(x, y, width, height)) {
            self.inner.rect(x, y, width, height);
        }
    }

    fn arc(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        if self.record(|| PaintOp::Arc(x, y, radius, start_angle, end_angle)) {
            return Ok(());
        }
        self.inner.arc(x, y, radius, start_angle, end_angle)
    }

    fn quadratic_curve_to(&self, cp_x: f64, cp_y: f64, x: f64, y: f64) {
        if !self.record(|| PaintOp::QuadraticCurveTo(cp_x, cp_y, x, y)) {
            self.inner.quadratic_curve_to(cp_x, cp_y, x, y);
        }
    }

    fn bezier_curve_to(&self, cp1_x: f64, cp1_y: f64, cp2_x: f64, cp2_y: f64, x: f64, y: f64) {
        if !self.record(|| PaintOp::BezierCurveTo(cp1_x, cp1_y, cp2_x, cp2_y, x, y)) {
            self.inner.bezier_curve_to(cp1_x, cp1_y, cp2_x, cp2_y, x, y);
        }
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
        if self
            .record(|| PaintOp::Ellipse(x, y, radius_x, radius_y, rotation, start_angle, end_angle))
        {
            return Ok(());
        }
        self.inner
            .ellipse(x, y, radius_x, radius_y, rotation, start_angle, end_angle)
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
        if self.record(|| {
            PaintOp::EllipseAnticlockwise(
                x,
                y,
                radius_x,
                radius_y,
                rotation,
                start_angle,
                end_angle,
                anticlockwise,
            )
        }) {
            return Ok(());
        }
        self.inner.ellipse_with_anticlockwise(
            x,
            y,
            radius_x,
            radius_y,
            rotation,
            start_angle,
            end_angle,
            anticlockwise,
        )
    }

    fn fill(&self) {
        if !self.record(|| PaintOp::Fill) {
            self.inner.fill();
        }
    }

    fn stroke(&self) {
        if !self.record(|| PaintOp::Stroke) {
            self.inner.stroke();
        }
    }

    fn clip(&self) {
        if !self.record(|| PaintOp::Clip) {
            self.inner.clip();
        }
    }

    fn use_fill_style_for_stroke(&self) {
        if !self.record(|| PaintOp::UseFillStyleForStroke) {
            self.inner.use_fill_style_for_stroke();
        }
    }

    /// A capture declines the batch so the caller draws glyph by glyph; the
    /// pixels are the same and every glyph is recorded.
    fn try_fill_glyph_run_batch(
        &self,
        text: &str,
        x_positions: &[f64],
        y: f64,
    ) -> Result<bool, Self::Error> {
        if self.is_capturing() {
            return Ok(false);
        }
        self.inner.try_fill_glyph_run_batch(text, x_positions, y)
    }

    fn try_fill_glyph_run_batch_multi_font(
        &self,
        text: &str,
        x_positions: &[f64],
        font_indexes: &[u16],
        fonts: &[Rc<str>],
        y: f64,
    ) -> Result<bool, Self::Error> {
        if self.is_capturing() {
            return Ok(false);
        }
        self.inner
            .try_fill_glyph_run_batch_multi_font(text, x_positions, font_indexes, fonts, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{Op, RecordingPainter};

    #[test]
    fn a_capture_paints_nothing_until_it_is_replayed() {
        let surface = RecordingPainter::new();
        let sink = new_sink();
        let captured = RoutedPainter::new(surface.clone(), Some(sink.clone()));
        captured.set_fill_style_str("#ff0000");
        captured.fill_rect(1.0, 2.0, 3.0, 4.0);
        captured.set_font("12px Arial");
        // Only the font reaches the surface, so measurement inside the capture
        // sees it.
        assert_eq!(surface.ops(), vec![Op::SetFont("12px Arial".into())]);

        surface.clear();
        replay(&sink.borrow(), &surface, |_| {}).unwrap();
        assert_eq!(
            surface.ops(),
            vec![
                Op::SetFillStyle("#ff0000".into()),
                Op::FillRect(1.0, 2.0, 3.0, 4.0),
                Op::SetFont("12px Arial".into()),
            ]
        );
    }

    #[test]
    fn markers_hand_control_back_to_the_caller() {
        let surface = RecordingPainter::new();
        let mut seen = Vec::new();
        replay(
            &[PaintOp::Save, PaintOp::Marker(7), PaintOp::Restore],
            &surface,
            |id| seen.push(id),
        )
        .unwrap();
        assert_eq!(seen, vec![7]);
        assert_eq!(surface.ops(), vec![Op::Save, Op::Restore]);
    }

    #[test]
    fn a_capture_declines_glyph_batches() {
        let sink = new_sink();
        let captured = RoutedPainter::new(RecordingPainter::new(), Some(sink));
        assert!(!captured
            .try_fill_glyph_run_batch("ab", &[0.0, 1.0], 0.0)
            .unwrap());
    }
}
