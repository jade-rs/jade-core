# DOCX edit integrity checks

Run the independent text model and package checks:

```sh
cargo test -p jade-docx-headless-editor --test edit_integrity
PROPTEST_CASES=512 cargo test -p jade-docx-headless-editor --test edit_integrity
cargo test -p jade-docx-writer
```

The default is 128 cases per property, each with 1–64 operations. Each step
checks all ZIP entries (including decompression/CRC), parses every XML and
relationship part with roxmltree, checks content types, relationship targets,
relationship IDs and XML references, reopens the exported DOCX, compares paragraph
text with an independent string-vector model, and checks preservation of an
unrelated custom XML part. Positions are UTF-8 byte offsets at scalar boundaries.
Text extracted by roxmltree is also compared with the editor reader, so shared
reader/writer mistakes cannot hide XML normalization losses.
The operations are insertion, paragraph splitting, and range replacement
(including deletion and joining paragraphs). A second property interleaves
insertion, splitting, range replacement, undo and redo on a long-lived document and checks every
export. Shrunk failures are persisted beside the integration test.

The control-character regression reproduced a successful export containing
NUL in `word/document.xml`. The writer now returns an error for XML 1.0 forbidden
characters in UTF-8 XML parts, including comments, notes and relationships
([XML 1.0 character rules](https://www.w3.org/TR/REC-xml/#charsets)).
It preserves opaque source XML in other encodings and binary parts. This does
not change the in-memory editing model or silently delete user input.

Insertions and replacements now normalize CRLF and CR to model line breaks,
matching browser paste behavior. Previously a raw CR could be written into `w:t`
and reopen as a space. Snapshot batches account for the normalized byte length.
The model checks normalization, persistence and history together. Focused tests
also protect CR/LF/tab attribute escaping in comment text and author values.

Run coverage-guided fuzzing (requires nightly Rust and cargo-fuzz):

```sh
cargo install cargo-fuzz --locked --version 0.13.2
mkdir -p fuzz/corpus/edit_docx
cp fuzz/seeds/edit_docx/* fuzz/corpus/edit_docx/
cargo +nightly fuzz run edit_docx -- -max_total_time=300 -max_len=385 -timeout=60
```

The target selects among committed hello, table, comment, hyperlink and field
fixtures. Each input drives up to 32 insert/split/replace/comment operations.
After every successful save it validates every XML part independently and
reopens the package. Explicitly rejected operations are allowed. Reproduce a
crash with `cargo +nightly fuzz run edit_docx fuzz/artifacts/edit_docx/<file>`.
Keep minimized crash inputs and add focused regressions before fixing them.

Committed seeds include both 8-operation and 32-operation sequences for each
fixture. The per-input timeout allows the longer sequences to finish under ASan.

The DOCX edit integrity workflow runs enlarged properties and a five-minute
fuzz campaign on relevant pull requests, nightly, and by manual dispatch. It
uploads regression seeds and fuzz artifacts. Existing workspace tests provide
additional coverage of formatting, tables, revisions and annotations.

These checks establish package/text invariants for the generated workflows;
they do not prove all OOXML schema rules, all editor commands, rendering parity,
or that Microsoft Word will never show a repair dialog. Actual Word validation
and browser rendering tests remain separate compatibility checks.

## Local verification

Validated against `jade-core` main `9c3e2215`:

- Before the fixes, forbidden-control export and CR persistence properties failed.
- 512 cases per property: 1,024 generated edit/history sequences, up to 64 operations
  each, plus seven focused regression/package tests; all passed.
- Reader, writer, document-core and headless-editor library tests: 2,190 passed,
  two existing ignored tests, zero failures.
- `cargo clippy --locked -p jade-docx-writer -p jade-docx-reader -p jade-docx-document-core -p jade-docx-headless-editor --all-targets -- -D warnings` passed.
- Workspace and standalone fuzz harness formatting checks passed.
- ASan/libFuzzer: 68 runs in 304 seconds, including all ten committed seed
  sequences (five fixture types, 8/32 operations); no crashes, sanitizer errors
  or package invariant failures. Final coverage: 28,206 edges, 95,027 features.
  Throughput was limited by concurrent host load; this is a bounded campaign,
  supplemented by the model-based properties and recurring CI campaigns.
