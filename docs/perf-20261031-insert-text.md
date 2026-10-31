# Text insertion through draw-command generation

The workload measures a Japanese character inserted at the start of a nonempty
paragraph on page one, including editing, cache invalidation, visible-page
layout and draw-command generation. It uses real font shaping and a no-op
painter. Loading, initial whole-document layout, Undo, browser event handling,
Skia/browser rasterization and presentation are outside the timed interval.
The first input after complete layout is reported separately from repeated
insert/Undo cycles; a bounded Undo repaint does not request pagination reuse.

## Profile and change

Sampling stacks are filtered to `profile_insert_text::measure_insert_text` so
loading, initial layout and Undo do not enter the input profile. The initial
profile found selection-layout destruction in the first repaint: incremental
registration discarded the selection data for all later paragraphs even when
the changed paragraph retained the same geometry.

`NativeDocument` now requests the renderer's existing completed-pagination edit
path for a recorded, single-paragraph text insertion. It checks both the history
revision and the actual recorded command's render footprint. An `InsertText`
request can execute a different content-control or form-field action, so the
request type alone does not establish the scope of the edit.

The renderer checks the edited paragraph's geometry before keeping the later
layout and selection data. A change in height, page, column or position follows
the existing invalidation path. Incomplete layouts, notes, pageless mode and
edits inside table cells keep the regular layout path. A cell's text height
alone cannot prove that its table's columns and other cells stayed unchanged.
Browser typing already uses this renderer mechanism;
this change connects native typing to it without changing the layout algorithm.

Full dirty state from the prior paint caused selection registration to skip
saving the tail even when pagination reuse succeeded. Save that tail for
completed-pagination edit candidates too, restoring it only after the same
geometry checks succeed.
This also covers a full-page repaint that stops after the stable edit.
The host must retain the later page slots before that repaint too; truncation
after the partial walk cannot restore slots discarded at render entry.

The large Yokohama fixture also lost table editing metadata after an early
exit. For a proven-stable body edit, retain the complete
table resize metadata and cell registry, including fragment ids and paragraph
mappings. A geometry-changing edit discards this snapshot and follows normal
registration.

A second profile aggregated 50 fresh processes per version. In the baseline,
118 of 301 input samples included destruction of the selection's line-layout
vectors. After retaining completed pagination, 58 of 181 input samples landed
on the comment-anchor scan and 19 on horizontal-float-wrap detection. These are
sample counts, not elapsed-time proportions with precise confidence bounds.
Source lines were resolved from the profiling executable's dSYM.

For a recorded single-paragraph text edit, native typing now saves the previous
negative answers without retaining the document. After the edit it checks the
changed paragraph, including nested text-box comment anchors, before installing
the negative answers for the new immutable snapshot. Unknown or positive answers
still take the regular scan. Generic cache drops remain conservative. The
recorded command must prove that no other paragraph or story changed; structural
and alternate form-field/control commands cannot opt in through the request type.

The final fresh-input Yokohama profile contained 195 input samples. The render
entry point's self samples fell from 91/181 in the intermediate profile to
16/195; its resolved source locations no longer included either full-document
scan. Paragraph rendering, copying and hashing remained. These profiles locate
work; elapsed-time improvements are measured separately below.

## Measurements

First input after complete layout, 500 fresh processes per fixture and version:

| Fixture | Baseline median / p95 | Final median / p95 | Final inputs over 1 ms |
| --- | ---: | ---: | ---: |
| contract | 0.213 / 0.857 ms | 0.220 / 0.808 ms | 15/500 |
| nda | 0.196 / 0.525 ms | 0.202 / 0.697 ms | 13/500 |
| yokohama | 1.359 / 3.155 ms | 0.381 / 0.640 ms | 11/500 |

All 500 final inputs per fixture passed completed-pagination geometry checks.
The large document's median improved by 72%; small-document medians remained
around 0.2 ms and did not improve. Every fixture's final p95 is below 1 ms,
but this is not a maximum-latency guarantee. Final maximums were 3.642, 17.222
and 4.592 ms respectively. All samples, including these outliers, are retained.

Repeated insert/Undo, median of five run medians / five run p95s:

| Fixture | Baseline median / p95 | Final median / p95 |
| --- | ---: | ---: |
| contract | 0.068 / 0.155 ms | 0.069 / 0.133 ms |
| nda | 0.063 / 0.069 ms | 0.063 / 0.081 ms |
| yokohama | 0.180 / 0.271 ms | 0.084 / 0.088 ms |

These measurements ran on an Apple M1 Pro with macOS 27.0.1 and Rust 1.93.1.
Our builds and tests finished before paired timing. External machine load
varied; the build-completion snapshot had load averages 38.91, 56.53, 58.47.
The earlier intermediate profiles ran under much higher load, so they are used
to locate work rather than to compare elapsed latency with this final series.

## Measurement method

Compare the frozen baseline (renderer at `ba2d090`, with the workload using the
previous typing path) and the final renderer using the same `profiling` build
profile. Timings are elapsed wall-clock time, including editing and visible-page
rendering inside `measure_insert_text`.

For first input, launch 500 fresh processes per fixture and version with arguments
`1 0`. Alternate the order of baseline and final for each pair, and report the
median and nearest-rank p95 across the 500 first-input samples. Loading and the
initial full layout still happen before each timed input.
The series started with 30 pairs, extended to 100 after observing tail latency,
then prospectively extended to 500 to check the quantiles. Every earlier sample
is included in the final 500; profiling-process samples are not pooled into it.

For insert/Undo repetitions, run five pairs per fixture, alternating version
order, with 100 warmups and 500 measured operations per run. Report the median
of the five run medians and the median of the five run p95s. These are separate
from the fresh-process first-input measurements.

## Correctness checks

The editor and layout unit suites passed: 2,342 and 703 tests respectively
(21 and 12 existing ignored tests). Native rendering's editing operations,
tables, view geometry, content controls, and selection/history suites passed:
42 tests, with two existing ignored tests.

The new regressions compare stable typing and wrapping text against a full
pixel repaint, retain later-page carets across repeated typing, and verify the
complete table resize/cell metadata in the Yokohama document. The renderer unit
regression also starts with full dirty state and checks the later host pages
and selection entries.
Three cache regressions additionally verify that a local edit skips the full
scans without retaining a strong/weak document reference across mutation, that
new anchors (including nested text boxes) and horizontal wrapping are detected,
and that positive or unknown answers are never carried forward as negative ones.

Clippy passed for editor/layout/perf (`--all-targets --all-features`) and the
native crate (`--all-targets`), both with `-D warnings`.

```sh
cargo test -j 2 -p jade-docx-editor -p jade-docx-layout --lib
cargo test -j 2 --manifest-path crates/jade-docx-native-render/Cargo.toml --locked \
  --test editing_operations --test tables --test view_geometry \
  --test content_controls --test selection_and_history -- --test-threads=1
```

## Reproduction

```sh
cargo build -j 2 -p jade-docx-perf --example profile_insert_text --profile profiling
target/profiling/examples/profile_insert_text public/fixtures/contract.docx 500 100 > contract-insert.json
target/profiling/examples/profile_insert_text public/fixtures/nda.docx 500 100 > nda-insert.json
target/profiling/examples/profile_insert_text public/fixtures/yokohama.docx 500 100 > yokohama-insert.json
```

See [the workload documentation](../crates/jade-docx-perf/README.md) for output
fields and sampling instructions. `first_pagination_preserved` verifies that
the initial input actually used the geometry-checked reuse path.
