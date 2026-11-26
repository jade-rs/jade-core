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

`keystroke-band` types the same 20 insertions into the same paragraph, the
way `NativeDocument` does (`open` and `apply_and_repaint_grouped` in
`jade-docx-native-render/src/document.rs`). The document is opened with its
numbering, headers, footers and comments. Each keystroke then follows the
native sequence:

- it reads the text-edit cache with `local_text_edit_cache_for_paragraph`;
- it hands the document back with `restore_document_cache` after the edit;
- once the recorded command proves a single-paragraph edit, it enables band
  redraw and calls `prepare_completed_pagination_edit` with the paragraph's
  height from before the edit;
- it sets the edit's downstream aside and calls `prepare_incremental_repaint`
  and `set_visible_page_range` for the bounded pass;
- it renders with `render_bounded_to_page_with_parts`, restores the downstream
  when its lines stood, and calls `clear_dirty_regions`.

The renderer repaints only the edited paragraph's band when
`compute_band_redraw_plan` accepts it. `keystroke` and `keystroke-marks` never
enable band redraw and measure the full-page fallback, so changes to the band
plan, the band clip or the geometry stability check only show up in
`keystroke-band`.

What the 20 keystrokes take differs by fixture, and a unit test pins it
keystroke by keystroke:

- yokohama.docx and nda.docx take a band on every keystroke and keep the
  completed pagination.
- On contract.docx the fourth keystroke wraps the edited paragraph onto a
  second line. That band is rejected as unstable and the page is repainted.
  The other 19 keystrokes take bands, but after the wrap the completed
  pagination is no longer offered for reuse, so contract's row mixes one page
  repaint with band repaints that do not reuse pagination.

Other tests check that `keystroke` takes no band, and that the edited
paragraph's text reaches the painter in a band repaint that draws less than
the page repaint does.

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
