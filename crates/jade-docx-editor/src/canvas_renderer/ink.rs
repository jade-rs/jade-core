//! Word ink drawn from its strokes (`RunImage::ink`), the way `jade-ink`
//! paints every Office ink: pens as round strokes, the pencil grainier, the
//! highlighter a translucent flat tip, and Office's ink effects as colour
//! along or across the stroke.

use super::*;
use jade_docx_reader::inkml::{InkContent, InkTrace};

use jade_ink::paint::Place;

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
        jade_ink::paint::paint_trace(painter, trace, place, scale)
    }
}
