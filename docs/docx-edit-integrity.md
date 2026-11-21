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
Text extracted by roxmltree (including Word soft/no-break hyphen elements) is
also compared with the editor reader, so shared
reader/writer mistakes cannot hide XML normalization losses.
The modeled operations are insertion, paragraph splitting, and range replacement
(including deletion and joining paragraphs). A second property interleaves
insertion, splitting, range replacement, undo and redo on a long-lived document and checks every
export. A third property supplies arbitrary coordinates, including UTF-8 interior
positions, reversed ranges and missing paragraphs, to text edits, bold formatting,
comments, footnotes and endnotes. Rejections are allowed; successful saves must
remain valid. A deterministic structural/history scenario additionally exercises
tables, formatting, comment/reply/update/resolve/delete, notes, headers/footers,
and tracked changes with undo/redo. Shrunk failures are persisted beside the
integration test.

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

## Independent OOXML validation

The optional `DOCX_SCHEMA_CORPUS` environment variable persists every checked
save, including intermediate edit, undo and redo states. Validate these files
with Microsoft's [Open XML SDK validator](https://learn.microsoft.com/en-us/office/open-xml/word/how-to-validate-a-word-processing-document):

```sh
dotnet restore tools/docx-validator --locked-mode
dotnet build tools/docx-validator --no-restore --configuration Release
dotnet tools/docx-validator/bin/Release/net8.0/DocxValidator.dll --self-test
DOCX_SCHEMA_CORPUS=/tmp/docx-properties PROPTEST_CASES=512 \
  cargo test --locked -p jade-docx-headless-editor --test edit_integrity
dotnet tools/docx-validator/bin/Release/net8.0/DocxValidator.dll /tmp/docx-properties
```

Requires .NET 8. SDK version 3.5.1 and transitive dependencies are locked. The
validator targets `FileFormatVersions.Microsoft365`, checks schema and SDK
semantic constraints, reports all findings with part/XPath as JSONL, and exits
nonzero on violations, unreadable packages or an empty corpus. Its self-test
accepts a valid document and rejects a well-formed XML document containing a run
directly inside the body. This guards against accidentally testing XML syntax
alone.

This additional oracle found that anonymous comments omitted the mandatory
`w:author` attribute (32 invalid saved states in an arbitrary-coordinate run).
New comments and replies now write an empty author attribute when no author is
provided; a focused regression checks both paths.

## Coverage-guided fuzzing

Run coverage-guided fuzzing (requires nightly Rust and cargo-fuzz):

```sh
cargo install cargo-fuzz --locked --version 0.13.2
mkdir -p fuzz/corpus/edit_docx
cp fuzz/seeds/edit_docx/* fuzz/corpus/edit_docx/
DOCX_SCHEMA_CORPUS=/tmp/docx-fuzz \
  cargo +nightly fuzz run edit_docx -- -max_total_time=300 -max_len=385 -timeout=60
dotnet tools/docx-validator/bin/Release/net8.0/DocxValidator.dll /tmp/docx-fuzz
```

The target selects among committed hello, table, comment, hyperlink and field
fixtures. Each input drives up to 32 insert, split, replace, comment, bold, footnote,
endnote and paragraph-alignment operations.
After every successful save it validates every XML part independently and
reopens the package. Explicitly rejected operations are allowed. Reproduce a
crash with `cargo +nightly fuzz run edit_docx fuzz/artifacts/edit_docx/<file>`.
Keep minimized crash inputs and add focused regressions before fixing them.

Committed seeds include 8-operation and 32-operation text/comment sequences,
plus sequences covering all eight operation kinds, for each fixture. The per-input timeout allows the longer sequences to finish under ASan.

The DOCX edit integrity workflow runs enlarged properties and a five-minute
fuzz campaign on relevant pull requests, nightly, and by manual dispatch. It
validates every successful saved state against the Microsoft365 SDK schema and
uploads failing schema corpora, regression seeds and fuzz artifacts. Existing workspace tests provide
additional coverage of formatting, tables, revisions and annotations.

These checks establish package/text invariants and SDK OOXML validity for the
generated saves. This is bounded testing against the Microsoft365 validator,
not a proof covering every OOXML document, every command or every ECMA-376/ISO
29500 requirement. Rendering parity and Microsoft Word's repair behavior require
separate application-level compatibility checks.

The XML-character guard uses a vectorizable block minimum and a targeted UTF-8
noncharacter search; scalar decoding is reserved for suspicious data. The first
implementation decoded every character on every export and failed the existing
Callgrind regression gate. The same gate checks the optimized implementation.

## Local verification

Validated against `jade-core` main `9c3e2215`:

- Before the fixes, forbidden-control export and CR persistence properties failed.
- 512 cases per property: 1,536 generated edit/history/coordinate sequences, up to
  64 operations each, plus eleven focused regression/package tests; all 14 tests passed.
- Microsoft365 Open XML SDK: 34,518 saved PBT/structural/history states validated,
  zero invalid documents or errors. A preceding run reproduced 32 missing-author
  errors before the anonymous-comment fix. The five source fixtures also passed.
- Reader, writer, document-core and headless-editor library tests: 2,190 passed,
  two existing ignored tests, zero failures.
- `cargo clippy --locked -p jade-docx-writer -p jade-docx-reader -p jade-docx-document-core -p jade-docx-headless-editor --all-targets -- -D warnings` passed.
- Workspace and standalone fuzz harness formatting checks passed.
- Initial ASan/libFuzzer campaign: 68 runs in 304 seconds, including all ten original seed
  sequences (five fixture types, 8/32 operations); no crashes, sanitizer errors
  or package invariant failures. Final coverage: 28,206 edges, 95,027 features.
  Throughput was limited by concurrent host load; this is a bounded campaign,
  supplemented by the model-based properties and recurring CI campaigns.
