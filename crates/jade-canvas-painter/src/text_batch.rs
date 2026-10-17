use std::rc::Rc;

#[cfg(target_arch = "wasm32")]
use js_sys::{Array as JsArray, Float64Array, Uint16Array};
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;
use web_sys::CanvasRenderingContext2d;

/// Minimum glyph-run length (in UTF-16 code units) for the batched
/// `__jadeFillTextGlyphRun*` helper. Runs shorter than this fall back to the
/// per-glyph `fill_text` loop, which crosses the wasm→JS string boundary once
/// per glyph (one `decodeText`).
///
/// Tuned from `capture-edit-scroll-trace` on yokohama.docx (dense CJK body):
/// the value gates the initial-render immediate draw *and* the draw-command
/// replay (both call `fill_bmp_glyph_run_batch*`). CJK body text segments into
/// many sub-16-glyph runs, so a threshold of 16 left ~20% of drawn glyphs on
/// the per-glyph path. Lowering to 4 cuts the fallback share to ~5% (batching
/// ~76% of the previously-unbatched glyphs) and drops the worker
/// decode+fillText+decodeText+set_font self-time cluster ~14% per sample
/// (2.04% → 1.75% of worker samples). Going to 2 removes a further ~3pp of
/// fallback glyphs but showed no additional worker-time gain (the batch's
/// fixed per-call overhead — one typed-array copy plus the JS-side loop — is
/// least amortized on 2-3 glyph runs), so 4 is the sweet spot. Output is
/// pixel-identical at any threshold: the JS helper draws each glyph with its
/// own `fillText(charAt(i), xPositions[i], y)` — the batch only concatenates
/// for the boundary transfer, never for shaping.
#[cfg(target_arch = "wasm32")]
pub(crate) const MIN_GLYPH_RUN_BATCH_LEN: usize = 4;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = r#"
let textCapture = null;
export function __jadeBeginTextCapture() { textCapture = new WeakMap(); }
export function __jadeEndTextCapture() { textCapture = null; }
export function __jadeCaptureText(ctx, text, x, y) {
  if (!textCapture || !text || ctx.globalAlpha === 0) return;
  let state = textCapture.get(ctx);
  if (!state) { state = { runs: [], metrics: new Map() }; textCapture.set(ctx, state); }
  const key = JSON.stringify([ctx.font, ctx.textBaseline, text]);
  let metrics = state.metrics.get(key);
  if (!metrics) { metrics = ctx.measureText(text); state.metrics.set(key, metrics); }
  const m = ctx.getTransform();
  const offset = ctx.textAlign === "center" ? -metrics.width / 2 :
    ctx.textAlign === "right" || (ctx.textAlign === "end" && ctx.direction !== "rtl") ||
    (ctx.textAlign === "start" && ctx.direction === "rtl") ? -metrics.width : 0;
  const originX = x + offset;
  state.runs.push({ text, width: metrics.width,
    ascent: metrics.fontBoundingBoxAscent || metrics.actualBoundingBoxAscent || 0,
    descent: metrics.fontBoundingBoxDescent || metrics.actualBoundingBoxDescent || 0,
    transform: [m.a, m.b, m.c, m.d, m.a * originX + m.c * y + m.e, m.b * originX + m.d * y + m.f] });
}
export function __jadeTakeTextCapture(ctx, scale) {
  const runs = textCapture?.get(ctx)?.runs || [];
  return runs.filter(run => {
    const [a, b, c, d, e, f] = run.transform;
    const corners = [[0, -run.ascent], [run.width, -run.ascent],
      [0, run.descent], [run.width, run.descent]].map(([x, y]) => [a*x + c*y + e, b*x + d*y + f]);
    const xs = corners.map(point => point[0]), ys = corners.map(point => point[1]);
    return Math.max(...xs) > 0 && Math.min(...xs) < ctx.canvas.width &&
      Math.max(...ys) > 0 && Math.min(...ys) < ctx.canvas.height;
  }).map(run => ({ ...run, transform: run.transform.map(value => value / scale) }));
}
export function __jadeFillTextGlyphRun(ctx, text, xPositions, fontIndexes, fonts, y, len) {
  let activeFontIndex = -1;
  for (let i = 0; i < len; i += 1) {
    const fontIndex = fontIndexes[i];
    if (fontIndex !== activeFontIndex) {
      const font = fonts[fontIndex];
      if (typeof font === "string") {
        ctx.font = font;
      }
      activeFontIndex = fontIndex;
    }
    ctx.fillText(text.charAt(i), xPositions[i], y);
    if (textCapture) __jadeCaptureText(ctx, text.charAt(i), xPositions[i], y);
  }
}

export function __jadeFillTextGlyphRunSingleFont(ctx, text, xPositions, y, len) {
  for (let i = 0; i < len; i += 1) {
    ctx.fillText(text.charAt(i), xPositions[i], y);
    if (textCapture) __jadeCaptureText(ctx, text.charAt(i), xPositions[i], y);
  }
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = __jadeBeginTextCapture)]
    fn begin_text_capture_js();
    #[wasm_bindgen(js_name = __jadeEndTextCapture)]
    fn end_text_capture_js();
    #[wasm_bindgen(js_name = __jadeCaptureText)]
    fn capture_text_js(context: &CanvasRenderingContext2d, text: &str, x: f64, y: f64);
    #[wasm_bindgen(js_name = __jadeTakeTextCapture)]
    pub fn take_text_capture(context: &CanvasRenderingContext2d, scale: f64) -> JsValue;
    #[wasm_bindgen(catch, js_name = __jadeFillTextGlyphRun)]
    fn fill_text_glyph_run_js(
        context: &CanvasRenderingContext2d,
        text: &str,
        x_positions: &Float64Array,
        font_indexes: &Uint16Array,
        fonts: &JsArray,
        y: f64,
        len: u32,
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(catch, js_name = __jadeFillTextGlyphRunSingleFont)]
    fn fill_text_glyph_run_single_font_js(
        context: &CanvasRenderingContext2d,
        text: &str,
        x_positions: &Float64Array,
        y: f64,
        len: u32,
    ) -> Result<(), JsValue>;
}

pub(crate) fn capture_text(context: &CanvasRenderingContext2d, text: &str, x: f64, y: f64) {
    #[cfg(target_arch = "wasm32")]
    if TEXT_CAPTURE_ENABLED.with(std::cell::Cell::get) {
        capture_text_js(context, text, x, y);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (context, text, x, y);
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static TEXT_CAPTURE_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(target_arch = "wasm32")]
pub fn begin_text_capture() {
    TEXT_CAPTURE_ENABLED.with(|enabled| enabled.set(true));
    begin_text_capture_js();
}

#[cfg(target_arch = "wasm32")]
pub fn end_text_capture() {
    TEXT_CAPTURE_ENABLED.with(|enabled| enabled.set(false));
    end_text_capture_js();
}

#[cfg(not(target_arch = "wasm32"))]
pub fn begin_text_capture() {}
#[cfg(not(target_arch = "wasm32"))]
pub fn end_text_capture() {}
#[cfg(not(target_arch = "wasm32"))]
pub fn take_text_capture(_context: &CanvasRenderingContext2d, _scale: f64) -> JsValue {
    JsValue::NULL
}

#[cfg(target_arch = "wasm32")]
struct GlyphRunBatchBuffers {
    x_positions: Float64Array,
    font_indexes: Uint16Array,
    fonts: JsArray,
}

#[cfg(target_arch = "wasm32")]
impl GlyphRunBatchBuffers {
    fn new() -> Self {
        Self {
            x_positions: Float64Array::new_with_length(0),
            font_indexes: Uint16Array::new_with_length(0),
            fonts: JsArray::new(),
        }
    }

    fn ensure_capacity(&mut self, len: usize) {
        let len = len as u32;
        if self.x_positions.length() < len {
            self.x_positions = Float64Array::new_with_length(len);
        }
        if self.font_indexes.length() < len {
            self.font_indexes = Uint16Array::new_with_length(len);
        }
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static GLYPH_RUN_BATCH_BUFFERS: RefCell<GlyphRunBatchBuffers> =
        RefCell::new(GlyphRunBatchBuffers::new());
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn fill_bmp_glyph_run_batch(
    context: &CanvasRenderingContext2d,
    text: &str,
    x_positions: &[f64],
    font_indexes: &[u16],
    fonts: &[Rc<str>],
    y: f64,
) -> Result<bool, JsValue> {
    let text_len = text.encode_utf16().count();
    if text_len < MIN_GLYPH_RUN_BATCH_LEN
        || x_positions.len() != font_indexes.len()
        || x_positions.len() != text_len
        || fonts.is_empty()
    {
        return Ok(false);
    }

    GLYPH_RUN_BATCH_BUFFERS.with(|buffers| {
        let mut buffers = buffers.borrow_mut();
        buffers.ensure_capacity(x_positions.len());
        buffers.fonts.set_length(0);
        for font in fonts {
            buffers.fonts.push(&JsValue::from_str(font));
        }
        // Copy into reusable JS typed arrays. This keeps the exact glyph
        // positioning behavior while avoiding a fresh typed-array backing store
        // for every rendered line/run.
        buffers
            .x_positions
            .subarray(0, x_positions.len() as u32)
            .copy_from(x_positions);
        buffers
            .font_indexes
            .subarray(0, font_indexes.len() as u32)
            .copy_from(font_indexes);
        fill_text_glyph_run_js(
            context,
            text,
            &buffers.x_positions,
            &buffers.font_indexes,
            &buffers.fonts,
            y,
            x_positions.len() as u32,
        )
    })?;
    Ok(true)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn fill_bmp_glyph_run_batch_single_font(
    context: &CanvasRenderingContext2d,
    text: &str,
    x_positions: &[f64],
    y: f64,
) -> Result<bool, JsValue> {
    let text_len = text.encode_utf16().count();
    if text_len < MIN_GLYPH_RUN_BATCH_LEN || x_positions.len() != text_len {
        return Ok(false);
    }

    GLYPH_RUN_BATCH_BUFFERS.with(|buffers| {
        let mut buffers = buffers.borrow_mut();
        buffers.ensure_capacity(x_positions.len());
        buffers
            .x_positions
            .subarray(0, x_positions.len() as u32)
            .copy_from(x_positions);
        fill_text_glyph_run_single_font_js(
            context,
            text,
            &buffers.x_positions,
            y,
            x_positions.len() as u32,
        )
    })?;
    Ok(true)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fill_bmp_glyph_run_batch(
    _context: &CanvasRenderingContext2d,
    _text: &str,
    _x_positions: &[f64],
    _font_indexes: &[u16],
    _fonts: &[Rc<str>],
    _y: f64,
) -> Result<bool, JsValue> {
    Ok(false)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fill_bmp_glyph_run_batch_single_font(
    _context: &CanvasRenderingContext2d,
    _text: &str,
    _x_positions: &[f64],
    _y: f64,
) -> Result<bool, JsValue> {
    Ok(false)
}
