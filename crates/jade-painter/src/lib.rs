//! Backend-agnostic 2D painting surface.
//!
//! `jade-docx-editor` draws documents through the browser's `CanvasRenderingContext2d`.
//! This trait names exactly the operations that renderer already performs, so a
//! non-browser backend (Skia, and in turn a native shell) can be plugged in
//! without the drawing code knowing which one it is talking to.
//!
//! ## Why the signatures look like `web_sys`
//!
//! Method names, argument order, and — importantly — **which methods can fail**
//! all mirror `web_sys::CanvasRenderingContext2d` exactly. That is deliberate:
//! migrating existing drawing code becomes a change of the parameter's *type*
//! with no edit to the call sites themselves, so a migration step cannot
//! silently add, drop, or reorder a canvas operation. Fallibility is copied for
//! the same reason — a method that returns `Result` here does so because the
//! canvas one does, not because a backend is expected to fail.
//!
//! Methods take `&self` rather than `&mut self` because the canvas API does, and
//! the renderer paints from `&self` methods throughout. Backends that need
//! mutable state use interior mutability.

pub mod paper;
pub mod recording;

/// Destination geometry for an encoded image, in the current painter coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub struct ImageSource<'a> {
    /// A content-aware key unique within a painter's lifetime. Reusing a key
    /// means the bytes are identical, including across document replacements.
    pub key: &'a str,
    pub bytes: &'a [u8],
}

/// The subset of canvas `TextMetrics` the renderer reads.
///
/// Canvas returns all of these from one `measureText` call, so a backend that
/// is a canvas pays nothing to fill them in. Backends that compute metrics
/// themselves may find the ink box more expensive than the advance, which is
/// why [`Painter::measure_text_width`] stays a separate method for the
/// per-grapheme hot path.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextMetrics {
    /// Advance width.
    pub width: f64,
    /// Ascent of the font's bounding box, independent of the string's glyphs.
    pub font_bounding_box_ascent: f64,
    /// Descent of the font's bounding box, as a positive distance below the
    /// baseline.
    pub font_bounding_box_descent: f64,
    /// Ascent of the inked area of this particular string.
    pub actual_bounding_box_ascent: f64,
    /// Descent of the inked area of this particular string, positive downwards.
    pub actual_bounding_box_descent: f64,
    /// Distance from the text origin to the right edge of the inked area. Wider
    /// than `width` when the last glyph overhangs its advance.
    pub actual_bounding_box_right: f64,
}

/// The 2D drawing operations `jade-docx-editor` performs.
///
/// Painter state (transform, styles, clip) is a stack manipulated with
/// [`Painter::save`] / [`Painter::restore`], and paths are built with
/// [`Painter::begin_path`] and consumed by [`Painter::fill`],
/// [`Painter::stroke`], or [`Painter::clip`] — the canvas model.
pub trait Painter {
    /// Failure reported by the backend. `JsValue` for the canvas backend;
    /// backends that cannot fail use [`core::convert::Infallible`].
    type Error;

    /// Draws an encoded image or a previously decoded resource with the same
    /// key. Source coordinates are normalized to the image's natural size.
    /// False means decoding is unavailable, allowing a visible fallback.
    fn draw_image(
        &self,
        _image: ImageSource<'_>,
        _source: Option<[f64; 4]>,
        _destination: ImageRect,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    // ---- state stack -------------------------------------------------------

    /// Pushes the current transform, styles, and clip region.
    fn save(&self);
    /// Pops the state pushed by the matching [`Painter::save`].
    fn restore(&self);

    // ---- transform ---------------------------------------------------------

    fn translate(&self, x: f64, y: f64) -> Result<(), Self::Error>;
    fn scale(&self, x: f64, y: f64) -> Result<(), Self::Error>;
    /// Rotates the coordinate system clockwise by `angle` radians.
    fn rotate(&self, angle: f64) -> Result<(), Self::Error>;

    // ---- styles ------------------------------------------------------------

    /// Sets the fill style from a CSS color string.
    fn set_fill_style_str(&self, value: &str);
    /// Sets the stroke style from a CSS color string.
    fn set_stroke_style_str(&self, value: &str);
    fn set_line_width(&self, width: f64);
    /// Scales the opacity of everything drawn subsequently, on top of any
    /// alpha already in the fill or stroke colour.
    ///
    /// Part of the saved state, so the usual way to bound it is `save`, draw,
    /// `restore` rather than setting it back to 1.0 by hand.
    fn set_global_alpha(&self, alpha: f64);
    /// Sets the stroke dash pattern as alternating on/off lengths. An empty
    /// slice restores a solid line.
    ///
    /// Table borders use this for Word's dashed and dotted line styles, so the
    /// pattern is part of stroke state and is saved and restored with it.
    fn set_line_dash(&self, segments: &[f64]);
    fn set_shadow_color(&self, value: &str);
    fn set_shadow_blur(&self, value: f64);
    fn set_shadow_offset_x(&self, value: f64);
    fn set_shadow_offset_y(&self, value: f64);
    /// Applies a Gaussian blur to subsequent drawing. Zero disables it.
    /// Like the other styles, this is part of the saved painter state.
    fn set_blur_filter(&self, radius_px: f64);

    // ---- text --------------------------------------------------------------

    /// Sets the font from a CSS font shorthand, e.g. `"bold 10.50pt 'Noto Sans JP'"`.
    fn set_font(&self, value: &str);
    /// Sets the baseline the y coordinate of text operations refers to.
    fn set_text_baseline(&self, value: &str);
    fn fill_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error>;
    fn stroke_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error>;
    /// Advance width of `text` under the current font.
    ///
    /// Line breaking never goes through here — `jade-docx-layout` owns wrap and
    /// per-line advance decisions. This exists for painting-time adjustments
    /// that need the width of what is about to be drawn.
    fn measure_text_width(&self, text: &str) -> Result<f64, Self::Error>;
    /// Full metrics for `text` under the current font.
    ///
    /// Separate from [`Self::measure_text_width`] because the renderer reads
    /// these only for line heights and ink overhang, while advance width is
    /// measured per grapheme while wrapping.
    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, Self::Error>;

    // ---- rectangles --------------------------------------------------------

    fn fill_rect(&self, x: f64, y: f64, width: f64, height: f64);
    /// Fills a horizontal RGBA gradient without changing the current style/path.
    /// Backends can override the bounded rectangle fallback with a native shader.
    fn fill_horizontal_gradient_rect(
        &self,
        rect: ImageRect,
        start: [u8; 4],
        end: [u8; 4],
    ) -> Result<(), Self::Error> {
        if !rect.width.is_finite()
            || !rect.height.is_finite()
            || rect.width <= 0.0
            || rect.height <= 0.0
        {
            return Ok(());
        }
        let steps = rect.width.ceil().clamp(1.0, 1024.0) as u32;
        self.save();
        for index in 0..steps {
            let t = (f64::from(index) + 0.5) / f64::from(steps);
            let rgba: [u8; 4] = std::array::from_fn(|channel| {
                (f64::from(start[channel]) * (1.0 - t) + f64::from(end[channel]) * t).round() as u8
            });
            self.set_fill_style_str(&format!(
                "rgba({}, {}, {}, {})",
                rgba[0],
                rgba[1],
                rgba[2],
                f64::from(rgba[3]) / 255.0
            ));
            let left = rect.x + rect.width * f64::from(index) / f64::from(steps);
            let right = rect.x + rect.width * f64::from(index + 1) / f64::from(steps);
            self.fill_rect(left, rect.y, right - left, rect.height);
        }
        self.restore();
        Ok(())
    }
    fn clear_rect(&self, x: f64, y: f64, width: f64, height: f64);
    /// Strokes a rectangle's outline without disturbing the current path.
    ///
    /// Not the same as `rect` + `stroke`: that appends to whatever path is
    /// being built, so a caller mid-path would stroke its own work too.
    fn stroke_rect(&self, x: f64, y: f64, width: f64, height: f64);

    // ---- paths -------------------------------------------------------------

    fn begin_path(&self);
    fn close_path(&self);
    fn move_to(&self, x: f64, y: f64);
    fn line_to(&self, x: f64, y: f64);
    /// Adds a closed rectangle subpath to the current path.
    fn rect(&self, x: f64, y: f64, width: f64, height: f64);
    fn arc(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error>;
    /// Adds a quadratic Bézier from the current point, via one control point.
    fn quadratic_curve_to(&self, cp_x: f64, cp_y: f64, x: f64, y: f64);
    /// Adds a cubic Bézier from the current point, via two control points.
    fn bezier_curve_to(&self, cp1_x: f64, cp1_y: f64, cp2_x: f64, cp2_y: f64, x: f64, y: f64);
    /// Adds an elliptical arc subpath. `rotation` tilts the ellipse's axes.
    #[allow(clippy::too_many_arguments)]
    fn ellipse(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error>;
    /// [`Painter::ellipse`] with an explicit sweep direction.
    #[allow(clippy::too_many_arguments)]
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
    ) -> Result<(), Self::Error>;
    fn fill(&self);
    fn stroke(&self);
    /// Intersects the clip region with the current path.
    fn clip(&self);

    /// Copies the current fill style onto the stroke style.
    ///
    /// Text decorations (underline, strike-through, character borders) are
    /// drawn in the colour of the text they belong to. The renderer has already
    /// set that colour as the fill style before painting the glyphs, and canvas
    /// expresses "stroke in the same colour" by reading the fill style back and
    /// assigning it. Exposing the intent rather than a `fill_style()` getter
    /// keeps the trait free of the canvas value type, and lets a backend that
    /// stores styles natively just copy its own field.
    fn use_fill_style_for_stroke(&self);

    // ---- optional fast paths ----------------------------------------------

    /// Draws a run of single-code-unit glyphs at explicit x positions in one
    /// call, using the font already set.
    ///
    /// Returns `Ok(false)` when the backend has no batch path or declines this
    /// run; the caller then falls back to per-glyph drawing, which produces the
    /// same pixels. Backends that gain nothing from batching keep the default.
    fn try_fill_glyph_run_batch(
        &self,
        _text: &str,
        _x_positions: &[f64],
        _y: f64,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    /// Multi-font variant: `font_indexes[i]` selects `fonts[font_indexes[i]]`
    /// for the i-th glyph. Same `Ok(false)` fallback contract.
    ///
    /// A backend that handles this may leave its own font state changed, so the
    /// caller must treat any tracked font as invalid afterwards.
    fn try_fill_glyph_run_batch_multi_font(
        &self,
        _text: &str,
        _x_positions: &[f64],
        _font_indexes: &[u16],
        _fonts: &[std::rc::Rc<str>],
        _y: f64,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }
}
