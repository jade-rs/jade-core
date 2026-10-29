# Word for Mac: next-paragraph square floats in compatibility mode 15

The captured Word for Mac 16.114 (build 16.114.26101123) reference is
`tests/word-oracle/mac-probes-20261019/word-mac-probe-float-lookahead.json`.
Its generator and README describe the five cases. The DOCX and Word reference
are unchanged by this fix.

| Case | Placement of next paragraph’s shape | Word preceding paragraph | Jade with Office Arial |
| --- | --- | --- | --- |
| A | At anchor top, no space after | 2 lines | 2 lines |
| B | At anchor top, 12pt space after | 2 lines | 2 lines |
| C | At anchor top, 1pt border plus 4pt space | 2 lines | 2 lines |
| D | 24pt above anchor, 6pt space after | 3 lines | 3 lines |
| E | Page-relative top at 90pt | 2 lines | 2 lines |

In D, the three Office Arial lines end with “long”, “wrap”, and “shows.”,
matching the Word reference. Word’s PDF extraction omits spaces; the comparison
ignores whitespace only. This establishes wrapping for this probe, rather than
full glyph-coordinate parity. The existing strict Word-coordinate set remains
unchanged.

Mode 15 now uses the anchor’s first-pass position and actual vertical overlap.
Mode 14 retains its existing touch/spacing rule. The shared layout layer computes
the preceding paragraph’s physical height, including variable font boxes,
numbering ascent, inline objects, float-band continuations and clearance. The
renderer pins the shape to its first-pass position if wrapping grows that
paragraph. Anchor pagination is measured only if the geometric exclusions
actually overlap the earlier text. Its keepNext group borrows the current
shared document; creating lookahead does not clone those paragraphs or images.

An anchor moved to another page or column must not wrap text on the previous
surface. A pin is also scoped to its original page and column if the anchor
subsequently advances during layout. A regression sweeps page-boundary positions with and without keepLines;
before the pagination guard, filler=47 and keepLines=false grew the preceding
paragraph from two to three lines incorrectly. A paragraph already continuing
to another page is conservatively excluded from this same-surface lookahead;
last-page lookahead for such paragraphs remains separate work.

The bundled-font coordinate snapshot changes only for this float-lookahead
probe (415 of 2,375 glyph positions; page count remains five). It uses Noto Sans JP
aliases, so its line endings differ from Office Arial. The Office-font comparison
above supports the change; the snapshot alone is not Word-parity evidence.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy -p jade-docx-layout -p jade-docx-editor --all-targets --all-features -- -D warnings`
- `cargo test -p jade-docx-layout -p jade-docx-editor --lib --all-features`:
  2,302 editor and 714 layout tests pass (34 existing ignored tests).
- `cargo test --manifest-path crates/jade-docx-native-render/Cargo.toml --test word_mac_coordinates -- --test-threads=1`:
  all three tests pass, including the 32-document bundled-font snapshot and
  the existing strict Word-coordinate comparisons.

Regression coverage includes the captured upward-offset behavior, empty final
manual-break lines, mixed-font pinning, page-boundary anchor movement and pin
scope across pages/columns. The physical-height property test samples 128
variable-height band sequences with shared float bands and clearance.
