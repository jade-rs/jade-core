# Indexed wrap-cache sizing during edit rendering

## Finding and change

`CanvasRenderer::render_with_numbering_rc_once` sizes the paragraph wrap cache
when the document's `Rc` identity changes. Ordinary text and composition edits
create copy-on-write snapshots, so the recursive body/table paragraph count can
walk the whole document before an otherwise bounded visible-page render.

The insert-text, composition-preview and delete paths now prepare capacity from
`WordDocument::body_paragraph_count()`, the length of its maintained body index.
This includes nested table paragraphs and excludes text-box, note and math
surfaces, matching the recursive fallback. Capacity still grows within the same
8,192–32,768 bounds. Preparation also checks uniquely owned, in-place structural
edits whose `Rc` address does not change. Render callers without a `WordDocument`
retain the recursive fallback. Layout, wrapping and seek rules are unchanged.

## Correctness evidence

Three renderer regressions check capacity bounds, growth with an unchanged `Rc`,
and identical nonempty `RecordingPainter` output between indexed and fallback
paths across copy-on-write edits. A 96-case property test checks the body index
against an independent integer model through nested-table splits, text inserts,
document replacement, text-box splits and surface rebuilding. It does not claim
delete/undo/redo sequence coverage.

With `PROPTEST_RNG_SEED=20261103` and Rust 1.93.1:

```sh
cargo +1.93.1 fmt --all -- --check
cargo +1.93.1 test -p jade-docx-editor --lib indexed_wrap_cache_capacity
cargo +1.93.1 test -p jade-docx-editor --lib
cargo +1.93.1 clippy -p jade-docx-editor --all-targets -- -D warnings
```

All commands passed: 2,396 editor tests, zero failures and 21 existing ignored
tests. Seven actual-WASM headless integration cases also passed with one worker,
zero retries, zero skipped cases and zero flaky cases. These cover 1/10/300-page
composition, marked 300-page composition, rapid Enter, page-crossing Enter and
same-page bitmap generation convergence. They preserve local-seek and bounded
body-walk assertions, nonblank/current bitmaps, local/full-render pixel equality,
prepaint annotations and exact saved text. No Word reference or tolerance changed.

## Bounded headless measurements

One predeclared ABBA experiment ran baseline/candidate/candidate/baseline on the
same 300-page document with annotations and 32 distinct growing Japanese
composition previews per run. All four runs passed; all samples, including the
first preview, were retained. Non-WASM assets were byte-identical (77 files).
All four recordings have `renderDebug: true`: renderer diagnostics were enabled
to assert local seeks and stable bands. Timings include diagnostic formatting
and console logging and must not be presented as normal-release benchmarks.
The baseline is core `1da432c`, with SDK `f6eb842` runtime assets. The candidate
uses that same core plus this change.

| Pair | Worker median / p95 (ms) | Bitmap median / p95 (ms) | Frame median / p95 (ms) |
| --- | --- | --- | --- |
| Baseline 0 → candidate 1 | 5.275 / 12.530 → 3.505 / 6.570 | 6.920 / 14.380 → 5.055 / 8.350 | 15.645 / 24.870 → 6.355 / 23.370 |
| Baseline 3 → candidate 2 | 6.370 / 14.070 → 3.260 / 4.015 | 8.065 / 15.700 → 4.820 / 5.615 | 18.225 / 34.410 → 5.830 / 21.725 |

Queue p95 changed from 0.530 to 0.550 ms in the first pair and from 0.565 to
0.525 ms in the second. Each preview consumed a local seek and walked fewer than
100 body elements. Every run saved exactly the source plus one committed
composition; the 31 previews after the first retained the stable render band.

The eight-core M1 Pro host was heavily loaded: one-minute load averages in run
order were 96.97, 95.07, 79.47 and 69.84. Active worker CPU samples were
642 → 784 and 731 → 643 in the two pairs, so they do **not** establish a consistent
CPU reduction. Renderer entry and bitmap creation remain sampled hotspots.
The defensible structural improvement is removal of the repeated recursive
capacity scan; the elapsed observations are limited to this experiment. They do
not establish native macOS IME, GPUI/CEF, or general elapsed/CPU improvement.

Named profiling WASM SHA-256:

- Baseline: `ee006e3b22c182dde3b6e80f6d593d3122121ca367b266ab08694c72580adbcf`
- Candidate: `e0abf9b192cdf5b533816bbc9c5f7b5d1e96a4bf59487919343117c0d68e572d`

Local raw evidence is retained under
`/Volumes/X9/jade-release-work/core-indexed-wrap-cache-profile/`: Rust and build
receipts, `paired-abba/validation.json`, all four sample/CPU-trace recordings,
`paired-abba/elapsed-comparison.json`, `paired-abba/hotspot-analysis.json`, and
`correctness-validation.json`. Profiling binaries preserve function names and
are not published artifacts. No computer-use automation was used.
