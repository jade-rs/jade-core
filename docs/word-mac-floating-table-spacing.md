# Mac floating tables and paragraph space after

The existing `word-mac-probe-floating-order.docx` places a correctly ordered
floating table at the right margin and an incorrectly ordered `tblpPr` table
inline. Word for Mac 16.114 (build 16.114.26101123, macOS 27.0.1) also wraps
Filler 8 just above the floating table: its text clears the table exclusion,
but its 10pt space after reaches it. The prior renderer omitted this wrap,
leaving the following body text about one line too high.

The evidence is unchanged:

- DOCX SHA-256: `eb417a0f7dc283af99425b43a28a356f90ed2ba5a53112e93508b763996b6ba7`.
- Word PDF SHA-256: `079f2ff60c9a8731869fedd7c155b9c3e5870793d185aab7a3d0321761f5dd96`.
- Word glyph JSON SHA-256: `cb175d41ea402a4d627322a067593deb8a62a9a11b079e75dbb8aded5c045504`.

The native renderer was compared using the Office Arial face from the local
Word installation (`arial.ttf` SHA-256
`15663c7e8495594c6136eb51b11983eb8f5bbcdd07bb2c327e0d1e83415b0f06`).
The diagnostic opens the original DOCX through `NativeDocument::open_with_font_faces`
and `system_fonts::word_document_faces`, then compares every visible glyph from
`layout_debug_lines` in points with the immutable Word glyph JSON. Page/text and
glyph counts must agree; no origin translation or scale fitting is applied.

| Whole-probe measure | Before | After |
| --- | ---: | ---: |
| Visible glyphs compared | 1,070 | 1,070 |
| Page count | 1 | 1 |
| Maximum baseline difference | 14.666703pt | 0.120438pt |
| Glyphs beyond 0.375pt on either axis | 372 | 7 |

The remaining seven glyphs spell ORDERED inside the floating table. Their
horizontal difference is up to 5.424160pt and is separate work. This probe is
therefore **not** added to the strict whole-probe Word-coordinate set.

The renderer marks table exclusions separately from drawing/frame exclusions.
For the observed Mac mode-14 paged body policy, it resolves the actual space
after, respecting contextual suppression and a paragraph-final page break.
It first wraps with the original exclusions to obtain the natural paragraph
bottom. Only a table whose top falls between that bottom and the end of space
after gets an expanded query band; the stored frame and bottom stay fixed.
Shared `jade-docx-layout` still decides the resulting line breaks. Shape
look-ahead, other compatibility modes, pageless documents and other stories
keep their existing behavior. No new Word capture establishes their table
spacing policy.

The bundled-font snapshot changes only for floating-order: 440 of 1,273 glyph
positions on its one page. The DOCX/Word-reference hashes and font hashes stay
the same, as do all other 31 probes. Bundled Noto Sans JP aliases produce
different line endings from Office Arial; that snapshot protects regressions
and is not evidence of Word parity on its own.

Regression tests cover overlapping and clear space after, a paragraph that
continues past the table top, fixed frame/bottom geometry, drawing separation,
platform/mode/story gates, contextual suppression and page-break suppression.

Validation:

- The new wrap regression failed before the fix (400px instead of 300px).
- `cargo test -p jade-docx-editor --lib -- --test-threads=1`: 2,302 passed,
  21 ignored.
- `cargo test --manifest-path crates/jade-docx-native-render/Cargo.toml --tests -- --test-threads=1`:
  159 existing tests passed, six ignored, including the native coordinate
  snapshot and strict Word-coordinate comparisons. An additional local-only
  Office-font diagnostic passed and was removed before committing.
- `cargo clippy -p jade-docx-layout -p jade-docx-editor --all-targets --all-features -- -D warnings`:
  passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
