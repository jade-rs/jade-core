# DOCX performance workloads

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
