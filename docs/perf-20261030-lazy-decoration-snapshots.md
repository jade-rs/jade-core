# Retain decoration pixels during IME previews

A 300-page annotated IME trace exposed a destructive snapshot between two
non-destructive operations. jade-elsa's persistent-decoration hook calls
`OffscreenDocumentView::applyDecorationResults` after a preview. Its eager
`take_paged_snapshots*` consumes the backing canvas; the lazy runtime discards
that snapshot, then its valid-pixel guard correctly requests another render.
The following preedit also loses its retained-canvas band path.

The opt-in `applyDecorationResultsLazy` uses the same parsing, invalidation and
rendering as the eager API but retains the painted canvas. It reports existing
dirty rectangles, or full-page damage when geometry is unavailable. Full-page
rectangles use absolute document Y (`page_top_y`) and include the same visible,
actually painted pages as the eager fallback. Consumers must fetch current
pixels with `create_page_bitmap`. The existing eager API and pixel/generation
checks retain their behavior.

## Repeated measurement

A headless Chromium application harness performed 32 distinct growing Japanese
preedits at page 300, with anchored red proofing marks. Three interleaved
baseline/candidate pairs yield 96 updates per variant. Each run checks a local
seek and a body walk below 100, then checks the complete saved text equals the
original plus one committed composition. Both variants walked 26 body elements
at median/p95. The candidate returned an inline bitmap for 96/96 updates;
baseline 0/96. Band updates were 90/96 versus 91/96, with full fallbacks retained.

| Milliseconds | Baseline median / p95 | Candidate median / p95 |
| --- | --- | --- |
| Worker response | 11.545 / 22.100 | 8.365 / 21.885 |
| Bitmap presentation | 18.835 / 35.675 | 10.000 / 23.440 |
| Animation frame | 21.055 / 40.890 | 18.560 / 31.050 |

Both WASM builds use `--profile profiling`, wasm-bindgen 0.2.114 and wasm-opt
`-O3 --debuginfo`. Baseline core 4daf719 has the same tree as SDK 0.0.64's
core pin d0f26b5. WASM SHA256 values:

- baseline: `07c9f3d24f7c565a34c91f3555303c158ca0722782bedde28b8c66639d878537`
- candidate: `82aaffdf57a4743c8f03826b9a46c2da79c57e8d099880556b321dae25739210`
- source DOCX: `c30c1131ed72fe42380e770c63607cfd7dec6e7ccbbedc057d4e067e374bb6b5`

These are synthetic browser composition measurements. They exclude the native
IME and GPUI/CEF transport and do not establish native keystroke latency.

## Verification

SDK `tests/playwright/tests/lazy-decoration-pixels.spec.ts` compares the entire
bitmap against the eager API for red, blue and removed underlines, both with
cached paragraph geometry and an explicitly cleared cache. It checks retained
pixels, dirty rectangles, history and saved document XML. The application gate
covers 1/10/300 pages, annotations, growing/shrinking/multiline/cancelled preedits,
local-versus-from-start pixel equality, nonblank presentation, generation
convergence and exact committed saved text. Ten existing SDK tests additionally
cover rapid/page-crossing Enter, input ordering and the low-priority arrival
window. All run headless with zero retries.

```sh
cargo fmt --all -- --check
cargo clippy --locked -p jade-docx-editor --target wasm32-unknown-unknown -- -D warnings
# In the SDK, against the candidate WASM:
pnpm exec playwright test --config tests/playwright/playwright.config.ts tests/playwright/tests/lazy-decoration-pixels.spec.ts --workers=1 --retries=0
# In jade-elsa, once per variant, alternating three times:
ELSA_IME_PAGES=300 ELSA_IME_MARKS=1 ELSA_IME_UNIQUE=1 ELSA_IME_REQUIRE_LOCAL=1 ELSA_IME_ASSETS=<variant-assets> ELSA_IME_OUTPUT=<evidence-directory> pnpm exec playwright test --config playwright.ime-perf.config.ts --reporter=line
```
