# DOCX performance workloads

The Callgrind CI workload includes both `keystroke` (formatting marks hidden)
and `keystroke-marks` (formatting marks visible). Both perform the same 20
Japanese text insertions and bounded page-one repaints after initial layout.
The marks workload includes control-mark font descriptor generation on input
repaints, which the hidden-marks workload does not exercise. A
regression test checks that paragraph marks reach the painter during both the
initial layout and the input repaint, with the hidden-marks case as a control.
That observation is compiled only for tests and adds no counters to the
Callgrind binary.

`keystroke-band` types the same 20 insertions with band redraw on, as
`NativeDocument::apply_and_repaint_grouped` does for every keystroke: once the
recorded command proves a single-paragraph edit, it enables band redraw,
restores the text-edit cache and calls `prepare_completed_pagination_edit`
with the paragraph's height from before the edit. The renderer then repaints
only the edited paragraph's band when `compute_band_redraw_plan` accepts it.
`keystroke` and `keystroke-marks` never enable it and measure the full-page
fallback, so changes to the band plan, the band clip or the geometry stability
check only show up in `keystroke-band`. Unit tests check that every keystroke
takes a band on all three CI fixtures (no gate rejects any of them), that
`keystroke` takes none, and that the edited paragraph's text reaches the
painter in a band repaint that draws less than the page repaint does.

`keystroke-band` is a diagnostic, outside the frozen 18-case gate
(`scripts/perf/check_regressions.py`). The workflow measures it in its own step
on base and head, into `perf/band/`, and appends that table, with the gated
`keystroke` rows copied beside it, to the pull-request comment. The
`run-callgrind.sh` default scenario list leaves it out; select it with
`PERF_SCENARIOS=keystroke-band`.

The CI grafts the same harness onto the base and head commits. Instruction
counts measure engine work; they do not measure browser rasterization,
composition previews, frame latency, or native macOS IME latency. They
supplement the matched end-to-end composition comparison.

To measure text insertion through visible-page layout and draw-command generation:

```sh
cargo build -p jade-docx-perf --example profile_insert_text --profile profiling
target/profiling/examples/profile_insert_text public/fixtures/contract.docx 1000 100 > insert-text.json
```

The arguments after the fixture are measured iterations and warmup iterations.
The example inserts `あ` at the beginning of a nonempty paragraph on page one,
then repaints through that page with the document's numbering, headers and
footers. Undo and its repaint restore the document outside the timed interval.
The initial load and whole-document layout are also outside the timer.

JSON output contains nanosecond samples for the complete operation (`total_ns`),
editing and cache invalidation (`edit_ns`), and repaint (`repaint_ns`). `first_ns`
contains the first operation's total, edit and repaint durations, before warmup.
`first_pagination_preserved` reports whether that edit reused the completed
pagination after checking the edited paragraph's geometry. The example follows
`NativeDocument`'s typing path and checks the recorded command's footprint before
requesting this reuse. Undo's bounded repaint does not request that optimization.
Use repeated runs and report both the median and p95. Keep the same fixture,
build profile and machine conditions when comparing binaries.

The performance host uses real font shaping and a no-op painter. This measures
the renderer's layout and drawing work, but excludes browser/Skia rasterization,
browser event handling and presentation on screen.

For sampling profiles, keep only stacks containing
`profile_insert_text::measure_insert_text` to exclude loading, initial layout
and Undo:

```sh
samply record --save-only --unstable-presymbolicate -o insert-text.json.gz -- \
  target/profiling/examples/profile_insert_text public/fixtures/contract.docx 10000 100 \
  > insert-text-samples.json
```

To sample only first input after complete layout, aggregate fresh processes
instead of insert/Undo cycles (apply the same stack filter):

```sh
samply record --save-only --unstable-presymbolicate --iteration-count 50 \
  --reuse-threads --main-thread-only -r 10000 -o first-input.json.gz -- \
  target/profiling/examples/profile_insert_text public/fixtures/yokohama.docx 1 0 \
  > first-input-samples.json
```
