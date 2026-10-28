# Late-page IME composition previews

On SDK 0.0.60, a synthetic 300-page document with paragraphs, tables and
header/footer stories walks 3,900 body elements for every uncommitted Japanese
composition update on the last page. Renderer counters show ink-overflow band
fallbacks and full-repaint escalation. The escalated repaint conservatively
rejects the visible-window seek, so it walks the prefix again.

`CanvasDocumentView::preview_composition_text_async` now suppresses band redraw
for lazy previews as well as eager previews. This renders the visible page
directly with the existing validated local seek. Committed text edits retain
band redraw, and invalid or missing seek snapshots retain their conservative
fallback. The suppression is reset after the preview render.

The browser regression uses the real SDK worker and WASM, with only the native
host boundary mocked. It replaces, grows, shrinks and cancels Japanese preview
text on page 300, bounds both the edit and forced dirty-bitmap refresh to fewer
than 100 body elements, requires local seeks, checks active bitmap generation
convergence and nonblank pixels, compares full RGBA hashes against a from-start
render, and verifies unchanged preview history and one saved commit. Before
the change it fails with a 3,900-element preview walk.

Timing and CPU tracing are separate. An exploratory same-release control with
band redraw disabled reduced the worker median from roughly 42 ms to 9 ms for
32 serialized preview updates, and the body walk from 3,900 to 26. This control
motivates the scoped fix; it is not a candidate-build performance result.
Synthetic headless composition does not measure physical macOS IME input or
the GPUI/CEF transport.

Validation commands and candidate results will be recorded after the focused
browser run and paired builds complete.
