# Preserve proven comment absence during ordinary IME preedits

The named SDK 0.0.73 diagnostic traces attribute 2.0 and 1.8125 WASM leaf
samples per update to comment-anchor detection in the 300-page, 3,900-paragraph
Japanese IME fixture. These are samples, not milliseconds. The fixture has
plugin decorations and no Word comment anchors. The positive witness from
#259 cannot accelerate this negative scan.

This change carries evidence from a fully checked immutable document through
the synchronous composition-preview pipeline. Only a collapsed caret in a
top-level body paragraph without a text box, and nonempty text without paragraph
separators, qualifies. InsertText must report the same single-paragraph history
footprint; the paragraph count and surface must remain unchanged; and the
resulting paragraph must have no ranges, markers, references or text boxes.
The renderer still checks every document and section header/footer story,
because those bodies are owned separately from Document.

Evidence uses a Weak<Document> identity, not a raw pointer or strong owner.
Arbitrary Rc::make_mut edits invalidate it, and replacement documents do not
match. The permission for the next preview is validated before undoing the
previous preview. An existing preview without original-base evidence must
never acquire it from an intermediate rendering: undo could restore an anchor
that the intermediate preview had removed. Cache drop/restore and header/footer
setters clear the renderer's evidence and ordinary anchor-result cache.

The synchronous permission itself holds no Weak. All old weak keys are dropped
before undo/apply, so Rc::make_mut can retain both the root allocation and the
untouched body. The original insert/underline/rollback logic is extracted into
a native-testable helper used by the actual worker composition method. Revision
session handling, selection, dirty ranges, lazy seek and pixel reuse are retained.
Ordinary edits, selection replacement, tables, text boxes, unknown snapshots
and unsupported preedits keep conservative scanning.

## Regression coverage

- The 3,900-paragraph test executes 32 growing Japanese preedits through the
  real command helper and renderer. Initial detection visits 3,900 blocks;
  subsequent previews visit zero additional body blocks, and cancellation
  restores the original text and selection. It checks both root Rc identity
  and an untouched paragraph string allocation. The old scan fails at 7,800
  visits on the first update. An earlier Weak-retaining implementation failed
  the root-allocation assertion; the permission now releases those weak keys.
- A stateful PBT runs 96 histories of 1–47 operations against independent
  base/preedit text and body/note/header anchor bits. It combines preedit,
  cancellation, commit, three anchor encodings, external body/note edits,
  header replacement, document replacement, cache restoration and visibility.
  Every step checks text and conservative seek eligibility, plus anchor presence
  when comments are visible.
- Deterministic cases reject stale evidence, replacement documents, unsupported
  surfaces and separators, and evidence acquired from an intermediate preview.
  All twelve document/section header/footer variants are checked for ranges,
  markers, references and absence while the body scan remains bypassed.

The editor library's 2,347 tests pass (21 pre-existing ignored), and editor
all-target/all-feature Clippy passes with warnings denied. No Word coordinate
fixture, tolerance or compatibility gate is changed. The scan-count reduction
is deterministic; an end-to-end native IME latency improvement has not yet
been measured.

## Optimized browser comparison

Both versions were built with Rust 1.93.1, wasm-bindgen 0.2.114 and Binaryen
126 (`-O3`), with identical published SDK 0.0.73 application JavaScript.
The baseline is #259 (`ba2d0905`); the first candidate is `2cfee283`.
Headless Chromium 156.0.8078.4 performed 32 growing Japanese preedits in the
300-page decorated fixture per run, without CPU tracing, renderer diagnostics
or retries. All 16 runs preserved the entire saved source plus exactly one
accepted composition. These are synthetic browser results, not native IME.

An initial fixed alternating batch (three runs per version) was inconclusive.
A preregistered confirmation added five runs per version, retaining all results.
The combined values below are medians of eight run-level medians/p95s, not a
pooled percentile:

| Measurement (ms) | Baseline median / p95 | Candidate median / p95 |
| --- | --- | --- |
| Worker | 9.7125 / 24.6200 | 9.6350 / 25.9875 |
| Bitmap presentation | 10.3900 / 25.5250 | 10.2650 / 27.4600 |
| Next animation frame | 16.5500 / 28.5025 | 14.9250 / 33.1975 |

The tail worsened in both batches; these results do not establish a latency
improvement. A cumulative `composition_comment_anchor_scan_bypasses` renderer
statistic now distinguishes actual worker use of the optimization from the
native helper regression. It increments only when validated composition
proof replaces the body/note scan, never for ordinary immutable-cache hits.
The earlier timings belong to the candidate before this diagnostic was added.

Separately, all 12 browser IME correctness regressions passed with zero retries,
and the existing local-walk budget passed all 32 previews (26 body layout calls
per update with a local seek). Frozen Word coordinate tests and the Office
Arial floating-table case passed without fixture or tolerance changes.
