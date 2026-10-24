//! Canvas backing for [`Painter`].
//!
//! Every method forwards straight to `CanvasRenderingContext2d` with no added
//! logic, so routing a renderer through [`Painter`] cannot change what the
//! browser draws. A newtype is required because both the trait and the canvas
//! type are foreign to this crate.
//!
//! The crate is format-neutral on purpose: it started inside
//! `jade-docx-editor`, and moved out when `.xlsx` needed the same backend. Any
//! renderer that draws through [`Painter`] gets the browser for free by using
//! it.

pub mod text_batch;

use crate::text_batch::{fill_bmp_glyph_run_batch, fill_bmp_glyph_run_batch_single_font};
use jade_painter::{ImageRect, ImageSource, Painter, TextMetrics};
use std::borrow::Borrow;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::JsValue;
use web_sys::{CanvasRenderingContext2d, ImageBitmap};

/// Converts a painter's error into the `JsValue` the renderer reports.
///
/// Renderer methods return `JsValue` because their non-painting steps (host
/// calls, layout lookups) do, so a painting method that also propagates those
/// cannot simply return `P::Error`. Bounding on `Painter<Error = JsValue>`
/// instead would work, but it shuts out every infallible backend — including
/// the recording one the tests draw onto — which is exactly the coupling the
/// `Painter` seam exists to remove.
pub trait PainterError {
    fn into_js_value(self) -> JsValue;
}

impl PainterError for JsValue {
    fn into_js_value(self) -> JsValue {
        self
    }
}

impl PainterError for core::convert::Infallible {
    fn into_js_value(self) -> JsValue {
        match self {}
    }
}

/// Paints through a canvas context.
///
/// Generic over how the context is held so one implementation serves both
/// forms: `Canvas2dPainter<&CanvasRenderingContext2d>` borrows the host's
/// surface, while `Canvas2dPainter<CanvasRenderingContext2d>` owns a handle to
/// it. The owning form exists because borrowing the host locks out `&mut self`
/// for as long as the painter lives, which table rendering cannot accept — it
/// does layout bookkeeping while the painter and wrap context are alive. A
/// canvas handle is a cheap JS reference and a clone points at the same
/// surface, so the two forms paint identically.
#[derive(Clone, Copy)]
pub struct Canvas2dPainter<C, I = ()>(C, I);

pub trait CanvasImages {
    fn image(&self, key: &str) -> Option<&ImageBitmap>;
}

impl CanvasImages for () {
    fn image(&self, _: &str) -> Option<&ImageBitmap> {
        None
    }
}

impl CanvasImages for &HashMap<String, ImageBitmap> {
    fn image(&self, key: &str) -> Option<&ImageBitmap> {
        self.get(key)
    }
}

impl<C: Borrow<CanvasRenderingContext2d>> Canvas2dPainter<C> {
    pub fn new(context: C) -> Self {
        Self(context, ())
    }

    pub fn with_images(
        context: C,
        images: &HashMap<String, ImageBitmap>,
    ) -> Canvas2dPainter<C, &HashMap<String, ImageBitmap>> {
        Canvas2dPainter(context, images)
    }
}

impl<C: Borrow<CanvasRenderingContext2d>, I> Canvas2dPainter<C, I> {
    fn ctx(&self) -> &CanvasRenderingContext2d {
        self.0.borrow()
    }

    /// The canvas underneath, for calling into drawing code that has not been
    /// migrated to [`Painter`] yet. Every use is a place still tied to the
    /// browser.
    pub fn context(&self) -> &CanvasRenderingContext2d {
        self.ctx()
    }
}

impl<C: Borrow<CanvasRenderingContext2d>, I: CanvasImages> Painter for Canvas2dPainter<C, I> {
    type Error = JsValue;

    fn draw_image(
        &self,
        image: ImageSource<'_>,
        source: Option<[f64; 4]>,
        destination: ImageRect,
    ) -> Result<bool, Self::Error> {
        let Some(bitmap) = self.1.image(image.key) else {
            return Ok(false);
        };
        let [sx, sy, sw, sh] = source.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        self.ctx()
            .draw_image_with_image_bitmap_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                bitmap,
                sx * f64::from(bitmap.width()),
                sy * f64::from(bitmap.height()),
                sw * f64::from(bitmap.width()),
                sh * f64::from(bitmap.height()),
                destination.x,
                destination.y,
                destination.width,
                destination.height,
            )?;
        Ok(true)
    }

    fn save(&self) {
        self.ctx().save();
    }

    fn restore(&self) {
        self.ctx().restore();
    }

    fn translate(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.ctx().translate(x, y)
    }

    fn scale(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.ctx().scale(x, y)
    }

    fn rotate(&self, angle: f64) -> Result<(), Self::Error> {
        self.ctx().rotate(angle)
    }

    fn set_fill_style_str(&self, value: &str) {
        self.ctx().set_fill_style_str(value);
    }

    fn set_stroke_style_str(&self, value: &str) {
        self.ctx().set_stroke_style_str(value);
    }

    fn set_line_width(&self, width: f64) {
        self.ctx().set_line_width(width);
    }

    fn set_global_alpha(&self, alpha: f64) {
        self.ctx().set_global_alpha(alpha);
    }

    fn set_line_dash(&self, segments: &[f64]) {
        let array = js_sys::Array::new();
        for segment in segments {
            array.push(&JsValue::from(*segment));
        }
        // Canvas rejects an odd-length or negative pattern by ignoring it; the
        // renderer only builds valid ones, so a failure here is not actionable.
        let _ = self.ctx().set_line_dash(&array);
    }

    fn set_shadow_color(&self, value: &str) {
        self.ctx().set_shadow_color(value);
    }

    fn set_shadow_blur(&self, value: f64) {
        self.ctx().set_shadow_blur(value);
    }

    fn set_shadow_offset_x(&self, value: f64) {
        self.ctx().set_shadow_offset_x(value);
    }

    fn set_shadow_offset_y(&self, value: f64) {
        self.ctx().set_shadow_offset_y(value);
    }

    fn set_blur_filter(&self, radius_px: f64) {
        if radius_px > 0.0 && radius_px.is_finite() {
            self.ctx().set_filter(&format!("blur({radius_px}px)"));
        } else {
            self.ctx().set_filter("none");
        }
    }

    fn set_font(&self, value: &str) {
        // A canvas rejects the whole shorthand over a token it does not know,
        // so OpenType features are dropped here: the text is drawn plain.
        self.ctx()
            .set_font(&jade_painter::font_features::strip(value));
    }

    fn set_text_baseline(&self, value: &str) {
        self.ctx().set_text_baseline(value);
    }

    fn fill_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.ctx().fill_text(text, x, y)?;
        text_batch::capture_text(self.ctx(), text, x, y);
        Ok(())
    }

    fn stroke_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.ctx().stroke_text(text, x, y)?;
        text_batch::capture_text(self.ctx(), text, x, y);
        Ok(())
    }

    fn measure_text_width(&self, text: &str) -> Result<f64, Self::Error> {
        Ok(self.ctx().measure_text(text)?.width())
    }

    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, Self::Error> {
        let metrics = self.ctx().measure_text(text)?;
        Ok(TextMetrics {
            width: metrics.width(),
            font_bounding_box_ascent: metrics.font_bounding_box_ascent(),
            font_bounding_box_descent: metrics.font_bounding_box_descent(),
            actual_bounding_box_ascent: metrics.actual_bounding_box_ascent(),
            actual_bounding_box_descent: metrics.actual_bounding_box_descent(),
            actual_bounding_box_right: metrics.actual_bounding_box_right(),
        })
    }

    fn fill_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.ctx().fill_rect(x, y, width, height);
    }

    fn fill_horizontal_gradient_rect(
        &self,
        rect: jade_painter::ImageRect,
        start: [u8; 4],
        end: [u8; 4],
    ) -> Result<(), Self::Error> {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Ok(());
        }
        let gradient =
            self.ctx()
                .create_linear_gradient(rect.x, rect.y, rect.x + rect.width, rect.y);
        for (offset, color) in [(0.0, start), (1.0, end)] {
            gradient.add_color_stop(
                offset,
                &format!(
                    "rgba({}, {}, {}, {})",
                    color[0],
                    color[1],
                    color[2],
                    f64::from(color[3]) / 255.0
                ),
            )?;
        }
        self.ctx().save();
        self.ctx().set_fill_style_canvas_gradient(&gradient);
        self.ctx()
            .fill_rect(rect.x, rect.y, rect.width, rect.height);
        self.ctx().restore();
        Ok(())
    }

    fn clear_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.ctx().clear_rect(x, y, width, height);
    }

    fn stroke_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.ctx().stroke_rect(x, y, width, height);
    }

    fn begin_path(&self) {
        self.ctx().begin_path();
    }

    fn close_path(&self) {
        self.ctx().close_path();
    }

    fn move_to(&self, x: f64, y: f64) {
        self.ctx().move_to(x, y);
    }

    fn line_to(&self, x: f64, y: f64) {
        self.ctx().line_to(x, y);
    }

    fn rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.ctx().rect(x, y, width, height);
    }

    fn arc(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.ctx().arc(x, y, radius, start_angle, end_angle)
    }

    fn quadratic_curve_to(&self, cp_x: f64, cp_y: f64, x: f64, y: f64) {
        self.ctx().quadratic_curve_to(cp_x, cp_y, x, y);
    }

    fn bezier_curve_to(&self, cp1_x: f64, cp1_y: f64, cp2_x: f64, cp2_y: f64, x: f64, y: f64) {
        self.ctx().bezier_curve_to(cp1_x, cp1_y, cp2_x, cp2_y, x, y);
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
        self.ctx().ellipse_with_anticlockwise(
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
        self.ctx()
            .ellipse(x, y, radius_x, radius_y, rotation, start_angle, end_angle)
    }

    fn fill(&self) {
        self.ctx().fill();
    }

    fn stroke(&self) {
        self.ctx().stroke();
    }

    fn clip(&self) {
        self.ctx().clip();
    }

    fn use_fill_style_for_stroke(&self) {
        // `fill_style`/`set_stroke_style` are the deprecated `JsValue` accessors,
        // but they are the only pair that round-trips a style without knowing
        // whether it is a colour, gradient, or pattern.
        #[allow(deprecated)]
        {
            self.ctx().set_stroke_style(&self.ctx().fill_style());
        }
    }

    fn try_fill_glyph_run_batch(
        &self,
        text: &str,
        x_positions: &[f64],
        y: f64,
    ) -> Result<bool, Self::Error> {
        fill_bmp_glyph_run_batch_single_font(self.ctx(), text, x_positions, y)
    }

    fn try_fill_glyph_run_batch_multi_font(
        &self,
        text: &str,
        x_positions: &[f64],
        font_indexes: &[u16],
        fonts: &[Rc<str>],
        y: f64,
    ) -> Result<bool, Self::Error> {
        if fonts
            .iter()
            .any(|font| jade_painter::font_features::has_features(font))
        {
            let plain: Vec<Rc<str>> = fonts
                .iter()
                .map(|font| Rc::from(jade_painter::font_features::strip(font).as_ref()))
                .collect();
            return fill_bmp_glyph_run_batch(
                self.ctx(),
                text,
                x_positions,
                font_indexes,
                &plain,
                y,
            );
        }
        fill_bmp_glyph_run_batch(self.ctx(), text, x_positions, font_indexes, fonts, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bound `CanvasRenderHost::Painter` carries has to admit backends that
    /// cannot fail, not just the canvas.
    ///
    /// This is the whole point of the associated type: a rasteriser reports
    /// `Infallible` while the canvas reports `JsValue`, and both have to reach
    /// the renderer's own error. Tightening the bound to `Error = JsValue` —
    /// the obvious-looking simplification — would compile everywhere today and
    /// silently shut every non-browser backend out.
    #[test]
    fn the_host_painter_bound_admits_a_backend_that_cannot_fail() {
        fn accepts_host_painter<P: Painter<Error: PainterError>>() {}
        accepts_host_painter::<jade_painter::recording::RecordingPainter>();
        accepts_host_painter::<Canvas2dPainter<CanvasRenderingContext2d>>();
    }

    /// `Painter` must stay usable as `dyn`.
    ///
    /// The host hands out a concrete painter today, so nothing depends on this
    /// yet — but a runtime-selected backend would, and object safety is easy to
    /// lose by accident: one generic method on the trait, or one `Self`-typed
    /// return, closes the option off. This fails at compile time if that
    /// happens, rather than when someone tries to use it.
    #[test]
    fn painter_stays_object_safe() {
        fn _accepts_dyn(_painter: &dyn Painter<Error = JsValue>) {}
    }

    /// Both forms have to satisfy `Painter` from the one implementation. A
    /// method that quietly required the borrowed form — say by returning a
    /// reference tied to the context's lifetime rather than to `&self` — would
    /// compile for the borrowing painter and fail here, where the owning form
    /// is what table rendering depends on.
    #[test]
    fn borrowed_and_owning_forms_share_one_implementation() {
        fn assert_painter<P: Painter>() {}
        assert_painter::<Canvas2dPainter<&CanvasRenderingContext2d>>();
        assert_painter::<Canvas2dPainter<CanvasRenderingContext2d>>();
    }
}
