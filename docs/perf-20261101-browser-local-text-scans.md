# Reuse absence checks during browser text insertion

The Callgrind artifact from core #348 (source
`21262e2fe4f4bbfec7a6b6db1045a0bdc9787069`, run `41279008690`) records
21,888,266 instructions in the Yokohama keystroke scenario. The exclusive cost
of `HorizontalFloatWrapCache::get` is 1,831,700 instructions (8.37%). These are
instruction counts for that harness, not browser or native input latency.
The profiling example and `NativeDocument` already reuse known absence facts
across a proven local text command; `OffscreenDocumentView` did not.

Browser/CEF committed body insertion now captures the existing renderer's
negative comment-anchor and horizontal-float facts before releasing its document
references. It restores them immediately before painting only after the executor
records a new command whose footprint is exactly the edited paragraph. The
candidate must be a collapsed, single-command insertion with no line break or
selected text box. Formatting batches, structural edits and selection replacement
retain the existing scans. The renderer rechecks the changed paragraph, including
its nested text boxes, and never carries positive or unknown facts as negatives.
The snapshot holds no strong or weak document reference across mutation.

The regression uses the browser's `ReplaceRange` command contract on 3,000
paragraphs. It checks scan counts, ownership during mutation, nonempty visible
content and equality with a fresh bounded layout. Separate cases reject a stale
history revision, a paragraph split and a mismatched paragraph. Existing cache
regressions check newly introduced anchors/floats and positive or unknown answers.
The original no-reuse implementation fails the scan-count assertion: it performs
one additional whole-document comment scan after the local insertion.

Local validation: 2,400 editor library tests passed (21 existing ignored),
the focused regression passed, and Clippy with warnings denied, the WASM target
check and workspace formatting all passed.

Reproduction commands:

```sh
cargo test --locked -p jade-docx-editor --lib
cargo clippy --locked -p jade-docx-editor --lib --tests -- -D warnings
cargo check --locked -p jade-docx-editor --target wasm32-unknown-unknown
cargo fmt --all -- --check
```

This change does not modify composition previews, edit scheduling, bitmap
transfer, geometry rules, Word references, tolerance or benchmark workloads.
It makes no end-to-end timing claim.
