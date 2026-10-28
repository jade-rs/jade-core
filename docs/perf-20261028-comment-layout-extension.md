# Forward layout extension with anchored comments

The SDK 0.0.58 renderer allows fresh visible-window seeks in commented documents,
but `prepare_layout_extension_seek` still uses the stricter ordinary/replay
compatibility guard. A document with a first-page comment therefore reports
`layout_extension_seek.last_miss = "options"` for every background extension and
walks the whole prefix again. This work starts from core main `c6d5692`, whose
source tree equals the previously released core `88076d1`.

Layout-only extension draws nothing, preserves prefix selection and table
geometry, and validates a page-top or spanning checkpoint before resuming. It
can use the existing forward-window compatibility guard: comment ranges and
markers do not require note-style global prefix state. Ordinary/mid-page pixel
seeks and cached paragraph replay retain their existing comment guards. Notes,
rendered revisions, mirrored margins, vertical text, invalid checkpoints, dirty
full repaints, and the seek opt-out retain conservative fallbacks.

A focused test failed before the production change with `commented layout
expansion must resume: options`. The replacement test host records per-page
paint operations. Tests cover repeated extensions, spanning paragraphs, missing
comment definitions, range/marker/run-reference anchors, retained pixels and
selection, and fresh annotation/pixel parity against an independent from-start
renderer. A 32-case property test varies the chunk history and anchor kind.

Validation (Rust 1.93.1):

```sh
cargo fmt -p jade-docx-editor --check
cargo test --locked -p jade-docx-editor --lib anchored_comment
cargo test --locked -p jade-docx-editor --lib
cargo clippy --locked -p jade-docx-editor --all-targets -- -D warnings
```

The focused tests passed (7 tests, including 32-case property coverage). The
whole editor library passed 2,285 tests with 20 existing ignored tests; clippy
passed without warnings. The extension tests bound each chunk's body walk by
`chunk_pages * 3 + 4` and compare all retained line geometry, later comment
annotations, paint operations and selection against independent full walks.

A browser baseline with SDK 0.0.58 and a valid first-page comment walked 26,280
body elements while completing pages 82 through 120 in twenty two-page
requests. Every extension missed with `options`. The same structural browser
gate fails on 0.0.58 for first-page and middle-page anchors. This identifies
prefix replay as the target; it does not establish native keyboard-to-paint
latency or a wall-time speedup. Paired WASM/browser profiling must verify those
separately with identical UI and build flags.
