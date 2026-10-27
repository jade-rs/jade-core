# Forward visible-window seeks with comment anchors

## Finding and change

A 300-page DOCX with one real comment anchor took the conservative from-start
path for late-page text edits, even when the comment was on page one. The Elsa
browser regression against SDK 0.0.57 observed zero local seeks and 3,900 body
elements walked for one last-page insertion. Retaining only an orphan comments
part did not trigger this path after the previous fix.

Comment ranges and markers are paragraph-local. Permit a validated forward
visible-window seek at the first visible page top, or a spanning-paragraph
snapshot strictly before that page. Draw the visible paragraphs and auxiliary
stories fresh to rebuild highlight geometry and comment annotations. Continue
rejecting ordinary/mid-page seeks and paragraph render-cache replay when comment
rendering is enabled: those cached commands do not retain comment metadata.
Existing notes, revisions, pageless, mirror-margin, and snapshot validity guards
remain in force. No wrapping or layout rules change.

## Verification

- Test-first regression failed at the missing visible-window seek before the fix.
- Fixed-page edit histories and a 32-case property test compare painted operations
  and annotation geometry with an independent from-start renderer after edits.
- Naturally spanning paragraphs also cover custom marker colors, commented headers
  and footers, and missing comment definitions between successive edits.
- Elsa's zero-retry 300-page browser gate covers orphan metadata and anchors on
  pages 1, 150 and 300. Insert/undo/redo must consume a local/span seek, walk fewer
  than 100 body elements, converge the active bitmap to the current generation,
  and match the from-start bitmap pixel-for-pixel. Saving must preserve the entire
  body, anchor IDs and comment author/date/text.

Commands:

```sh
cargo fmt --check
cargo test -p jade-docx-editor --lib anchored_comment
cargo test -p jade-docx-editor --lib
cargo clippy -p jade-docx-editor --lib -- -D warnings
```

The browser gate is `tests/web/multipage-comment-seek.spec.ts` in jade-elsa, run
against an isolated production UI with this candidate WASM. Timing comparisons
use identical profiling builds, browser/UI assets, a synthetic 300-page document,
100 serial single-character transactions, whole-document saved-text checks,
zero retries, and repeated runs. CPU traces are recorded separately from timing
runs; they are not measurements of physical macOS keyboard latency.
