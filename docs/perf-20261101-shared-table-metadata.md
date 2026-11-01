# Share completed table geometry during IME updates

A named headless Chromium profile of 32 distinct growing Japanese preedits in a
300-page, 300-table document found deep table-vector copies in two places:
`CanvasRenderer::render_with_numbering_rc_once` preserves the previous render,
and `OffscreenDocumentView` caches the latest completed table metadata. The
baseline core is `b13f8c7b741fbd70243426528e2791c16bfa64f4`; SDK is
`3a04695a21b85d47fde6ade166c4c22f053aa927`. Baseline WASM SHA256 is
`4d5a5eb96036701b3a0020b19902c1676f19579c04b6a1cb19c3723fce5dfaac`.

Two 32-update runs contain 18 and 26 exclusive `Vec::to_vec` worker samples
under renderer entry, plus 12 and 8 `Vec::clone` samples under the metadata
getter. The worker has 561 and 562 active samples respectively. These are
sampled attribution observations, not elapsed-time or native latency claims.

The renderer now owns each metadata vector through `Rc<Vec<_>>`. Internal view
updates retain immutable shared snapshots; mutations use `Rc::make_mut`.
The public owned `TableResizeMetadata` getter still returns independent vectors.
The serialized JSON and the last-completed-view snapshot lifecycle remain the
same, including asynchronous update ordering and the busy-renderer fallback.
Pixel-only rendering restores the existing shared allocation. Layout changes
can allocate through copy-on-write while earlier completed snapshots stay valid.

## Regression coverage

`shared_table_resize_metadata_preserves_wire_format_and_snapshot_isolation`
freezes empty and populated JSON, compares shared and owned serialization,
checks allocation sharing, and mutates all three renderer vectors while
retaining an immutable completed snapshot.

`page_bitmap_rerenders_preserve_table_editing_metadata` checks authoritative
geometry and shared allocation identity across out-of-order page bitmaps,
then checks geometry and old-snapshot isolation after layout extension. The
existing table resize, selection, history and Word layout tests retain their
assertions and fixtures.

## Verification

```sh
cargo +1.93.1 fmt --all -- --check
cargo +1.93.1 test -p jade-docx-editor --lib
cargo +1.93.1 clippy -p jade-docx-editor --all-targets -- -D warnings
```

Formatting, the full unit run (2,392 passed, 21 ignored as before), and
Clippy with warnings denied all pass on the unchanged source diff. The PR
description records the separate matched WASM/preedit profile and metadata
parity results; Rust checks alone do not establish a latency improvement.

Exact-diff Rust validation receipt:
`/Volumes/X9/jade-release-work/core-ime-table-shared-validation/validation.json`.

Local evidence root:
`/Volumes/X9/jade-release-work/core-shared-table-named-profile`.
The source diff is preserved as `candidate-source.patch` (SHA256
`226338e9b3d5697ef6add392a387926bde1c6a1705f2161b08429817a03d4561`).
Build, zero-retry runs, CPU traces, comparison, and metadata parity receipts
are recorded there. Baseline traces and attribution are in sibling
`sdk82-named-preedit-profile`.

All browser workloads are synthetic headless composition. They exclude native
macOS IME, CEF and GPUI transport and cannot establish native keystroke latency.
