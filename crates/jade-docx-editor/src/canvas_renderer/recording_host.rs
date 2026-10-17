//! A paged render host for renderer tests: it records drawing and measures
//! text without a browser.

use super::*;
use jade_painter::recording::RecordingPainter;

/// A paged host that records drawing and measures every character as 10px,
/// so pagination is deterministic and independent of installed fonts.
pub(super) struct RecordingHost {
    pub(super) painter: RecordingPainter,
}

impl crate::render_host::CanvasRenderHost for RecordingHost {
    type Painter = RecordingPainter;

    fn painter(&self) -> Self::Painter {
        self.painter.clone()
    }
    fn device_pixel_ratio(&self) -> f64 {
        1.0
    }
    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue> {
        Ok(false)
    }
    fn reset_pages(&mut self) -> Result<(), JsValue> {
        Ok(())
    }
    fn configure_base_canvas(
        &mut self,
        _width: f64,
        _height: f64,
        _pageless: bool,
        _margin_top: f64,
    ) -> Result<(), JsValue> {
        Ok(())
    }
    fn clear(&mut self, _page_width: f64, _pageless: bool, _full: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn ensure_pageless_height(&mut self, _h: f64, _mb: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn start_new_page(&mut self, _w: f64, _h: f64, _mt: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn device_to_css(&self, value: f64) -> f64 {
        value
    }
    fn canvas_css_size(&self) -> (f64, f64) {
        (816.0, 1056.0)
    }
    fn page_count(&self) -> usize {
        1
    }
    fn page_dimensions(&self, _page_index: usize) -> Option<(f64, f64)> {
        Some((816.0, 1056.0))
    }
    fn take_paged_snapshots(&mut self) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }
    fn take_paged_snapshots_for_pages(
        &mut self,
        _pages: Option<&[usize]>,
    ) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }
    fn context(&self) -> &web_sys::CanvasRenderingContext2d {
        unreachable!("recording host: context() must not be called")
    }
    fn context_mut(&mut self) -> &mut web_sys::CanvasRenderingContext2d {
        unreachable!("recording host: context_mut() must not be called")
    }
    fn create_page_bitmap(&self, _page_index: usize) -> Result<js_sys::Promise, JsValue> {
        unreachable!("recording host: bitmap snapshots are browser-side")
    }
    fn measure_text_width(&self, text: &str) -> Result<f64, JsValue> {
        Ok(text.chars().count() as f64 * 10.0)
    }
    fn measure_text_metrics(&self, text: &str) -> Result<jade_painter::TextMetrics, JsValue> {
        let width = text.chars().count() as f64 * 10.0;
        Ok(jade_painter::TextMetrics {
            width,
            font_bounding_box_ascent: 8.0,
            font_bounding_box_descent: 2.0,
            actual_bounding_box_ascent: 8.0,
            actual_bounding_box_descent: 2.0,
            actual_bounding_box_right: width,
        })
    }
}

pub(super) fn recording_renderer() -> CanvasRenderer<RecordingPainter> {
    CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: RecordingPainter::new(),
        }),
        false,
    )
    .expect("paged recording renderer")
}
