//! A painter that draws nothing.
//!
//! In the browser every drawing call crosses into the canvas, so what the
//! engine costs is the work it does to decide on a call, not the call. This
//! backend makes each call free and keeps only the font, which measurement
//! needs. A recording painter would charge each call an allocation the browser
//! never makes.
//!
//! Measurement is real: some paths ask the painter rather than the host, and
//! both answer from the same shaping measurer, as the browser's canvas and
//! host both answer from `measureText`.

use jade_docx_layout::layout_cache::TextMeasurer;
use jade_docx_layout::shaping_measurer::ShapingMeasurer;
use jade_painter::{Painter, TextMetrics};
#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;
use std::convert::Infallible;
use std::rc::Rc;

/// Runs shorter than this are drawn glyph by glyph by the canvas backend
/// (`jade-canvas-painter`'s `MIN_GLYPH_RUN_BATCH_LEN`); accepting them here
/// would skip work the browser does.
const MIN_GLYPH_RUN_BATCH_LEN: usize = 4;

#[derive(Clone)]
pub struct PerfPainter {
    font: Rc<RefCell<String>>,
    measurer: Rc<ShapingMeasurer>,
    #[cfg(test)]
    paragraph_marks: Rc<Cell<usize>>,
}

impl PerfPainter {
    pub fn new(measurer: Rc<ShapingMeasurer>) -> Self {
        Self {
            font: Rc::default(),
            measurer,
            #[cfg(test)]
            paragraph_marks: Rc::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn take_paragraph_mark_count(&self) -> usize {
        self.paragraph_marks.replace(0)
    }

    fn width(&self, text: &str) -> f64 {
        self.measurer
            .measure_text(text, &self.font.borrow())
            .unwrap_or(0.0)
    }

    fn metrics(&self, text: &str) -> TextMetrics {
        let width = self.width(text);
        let font = self.font.borrow();
        let (ascent, descent) = self.measurer.font_bounding_box(&font).unwrap_or((0.0, 0.0));
        let (ink_ascent, ink_descent) = self
            .measurer
            .measure_font_vertical_metrics(&font)
            .unwrap_or((ascent, descent));
        TextMetrics {
            width,
            font_bounding_box_ascent: ascent,
            font_bounding_box_descent: descent,
            actual_bounding_box_ascent: ink_ascent,
            actual_bounding_box_descent: ink_descent,
            actual_bounding_box_right: width,
        }
    }
}

fn batch_accepted(text: &str, positions: usize) -> bool {
    let len = text.encode_utf16().count();
    len >= MIN_GLYPH_RUN_BATCH_LEN && positions == len
}

impl Painter for PerfPainter {
    type Error = Infallible;

    fn save(&self) {}
    fn restore(&self) {}
    fn translate(&self, _x: f64, _y: f64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn scale(&self, _x: f64, _y: f64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn rotate(&self, _angle: f64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_fill_style_str(&self, _value: &str) {}
    fn set_stroke_style_str(&self, _value: &str) {}
    fn set_line_width(&self, _width: f64) {}
    fn set_global_alpha(&self, _alpha: f64) {}
    fn set_line_dash(&self, _segments: &[f64]) {}
    fn set_shadow_color(&self, _value: &str) {}
    fn set_shadow_blur(&self, _value: f64) {}
    fn set_shadow_offset_x(&self, _value: f64) {}
    fn set_shadow_offset_y(&self, _value: f64) {}
    fn set_blur_filter(&self, _radius_px: f64) {}
    fn set_font(&self, value: &str) {
        value.clone_into(&mut self.font.borrow_mut());
    }
    fn set_text_baseline(&self, _value: &str) {}
    fn fill_text(&self, _text: &str, _x: f64, _y: f64) -> Result<(), Self::Error> {
        #[cfg(test)]
        // CanvasRenderer's ParagraphEnd mark is U+21B5 (not a pilcrow).
        if _text == "\u{21B5}" {
            self.paragraph_marks.set(self.paragraph_marks.get() + 1);
        }
        Ok(())
    }
    fn stroke_text(&self, _text: &str, _x: f64, _y: f64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn measure_text_width(&self, text: &str) -> Result<f64, Self::Error> {
        Ok(self.width(text))
    }
    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, Self::Error> {
        Ok(self.metrics(text))
    }
    fn fill_rect(&self, _x: f64, _y: f64, _width: f64, _height: f64) {}
    fn clear_rect(&self, _x: f64, _y: f64, _width: f64, _height: f64) {}
    fn stroke_rect(&self, _x: f64, _y: f64, _width: f64, _height: f64) {}
    fn begin_path(&self) {}
    fn close_path(&self) {}
    fn move_to(&self, _x: f64, _y: f64) {}
    fn line_to(&self, _x: f64, _y: f64) {}
    fn rect(&self, _x: f64, _y: f64, _width: f64, _height: f64) {}
    fn arc(
        &self,
        _x: f64,
        _y: f64,
        _radius: f64,
        _start_angle: f64,
        _end_angle: f64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn quadratic_curve_to(&self, _cp_x: f64, _cp_y: f64, _x: f64, _y: f64) {}
    fn bezier_curve_to(&self, _c1x: f64, _c1y: f64, _c2x: f64, _c2y: f64, _x: f64, _y: f64) {}
    fn ellipse(
        &self,
        _x: f64,
        _y: f64,
        _radius_x: f64,
        _radius_y: f64,
        _rotation: f64,
        _start_angle: f64,
        _end_angle: f64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn ellipse_with_anticlockwise(
        &self,
        _x: f64,
        _y: f64,
        _radius_x: f64,
        _radius_y: f64,
        _rotation: f64,
        _start_angle: f64,
        _end_angle: f64,
        _anticlockwise: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn fill(&self) {}
    fn stroke(&self) {}
    fn clip(&self) {}
    fn use_fill_style_for_stroke(&self) {}

    fn try_fill_glyph_run_batch(
        &self,
        text: &str,
        x_positions: &[f64],
        _y: f64,
    ) -> Result<bool, Self::Error> {
        Ok(batch_accepted(text, x_positions.len()))
    }

    fn try_fill_glyph_run_batch_multi_font(
        &self,
        text: &str,
        x_positions: &[f64],
        font_indexes: &[u16],
        fonts: &[Rc<str>],
        _y: f64,
    ) -> Result<bool, Self::Error> {
        Ok(!fonts.is_empty()
            && font_indexes.len() == x_positions.len()
            && batch_accepted(text, x_positions.len()))
    }
}
