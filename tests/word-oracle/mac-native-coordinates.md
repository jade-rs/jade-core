# Word for Mac coordinate regression gate

`CI / native-macos` runs `word_mac_coordinates` for every pull request, without
Word or a locally installed Office font. It uses the native Skia renderer, the
Mac layout platform, and the repository's Noto Sans JP Regular/Bold faces as
stand-ins for the named document fonts. Other unregistered families fall back
to the first registered face; system fonts are not registered by this test.

The existing body-coordinate assertions run unchanged:

- All 34 probes in `mac-probes-16114`, `mac-probes-20261019` and
  `mac-floating-horizontal-20261030` must preserve the
  captured layout: page count, nonempty glyph count, page/text/order, X and
  painted baseline. Coordinate tolerance is **0.01 pt**. The initial snapshot
  records the behavior of `468c2d3` with this test harness; existing mismatches
  are preserved, not declared correct.
- Nine whole probes already match the Word for Mac PDF reference within
  **0.375 pt** on both axes. They must continue to match: `pairs-kerned`,
  `squeeze-comma`, `squeeze-conditions-compress`, `squeeze-conditions-nocompress`,
  `squeeze-k40-b3`, `squeeze-k5-b2`, `squeeze-plain`, `tab-shrink-comma`, and
  `tab-shrink-kanji` (all with the `word-mac-probe-` prefix). The allowlist is
  explicit; tests never silently select whichever probes currently pass.

Coordinates come from `layout_debug_lines()` after native rendering, converted
from CSS pixels to points. The debug API's documented reporting offset is
removed to recover painted X. There is no Word-driven wrapping, advance-width
replacement, origin alignment, or per-page translation. For PDF comparison
only, whitespace/object placeholders are omitted because PDF extraction omits
those glyphs, and glyphs are sorted by page/text/Y/X to ignore paint order.
The snapshot comparison includes whitespace and preserves capture order.
DOCX, Word-reference and bundled-font SHA-256 hashes must also match.

An additional native gate uses `NativeDocument::capture_full_layout_debug()`
for the two numbering-label probes and four ruby probes. It freezes every
captured auxiliary glyph's page, text, order, X and painted baseline at the same
**0.01 pt** snapshot tolerance, including whitespace. It also checks explicit
label/guide counts (9/5 numbering labels and 15/15/20/20 ruby guides), complete
layout and source inventories, absence of reported uncaptured stories, repeated
capture stability, and unchanged editable-body coordinates. Source DOCX,
Word-reference and bundled-font hashes are pinned. These auxiliary snapshots
preserve the behavior of `3a0c5b2` with the native diagnostic entry point;
they do not assert agreement with Word.

The same required test target also checks the unchanged `header-footer-sections`
fixture against its frozen Mac Word reference: its title page has no footer,
while page two inherits `SHARED FOOTER`. Complete native capture must preserve
that blank first-page footer, the page-two footer, both header texts and the
two-page count. This assertion concerns story selection and painted text; it
does not claim Office-font coordinate agreement. A separate exhaustive unit
test covers all variant-presence and flag combinations, including blank even
stories and first-page precedence on an even-numbered section start.

The required `mac_noto_circle_title_matches_original_word_coordinates` test
also compares all eight characters of `○横浜市市税条例` in the existing
`noto-yokohama_page6` fixture with its original Word PDF coordinates at the
same **0.375 pt** tolerance, with unchanged page count and source hashes.
The leading Noto Sans circle uses the embedded PDF font's 1237/2048-em
advance; `mac-noto-circle-advance.json` records its ToUnicode/cmap/hmtx
provenance. Bold, italic, other families, Windows and oracle-supplied widths
keep their previous behavior. This is a title-row gate only: other known
differences in this document are not declared fixed.

This is not full Word compatibility coverage. The original
`layout_debug_lines()` capture omits some painted content; the additional
full-capture gate covers numbering and ruby only in these six probes. These
34 probes do not cover the full corpus or browser rendering. A native
snapshot guards against changes to current behavior, including current bugs;
it does not prove that the other 25 probes match Word. Relevant follow-ups:

- [#220: unresolved Mac layout rules](https://github.com/jade-rs/jade-core/issues/220)
- [#221: reproducible Office/cloud fonts](https://github.com/jade-rs/jade-core/issues/221)
- [#222: full corpus inventory and complete raw glyph capture](https://github.com/jade-rs/jade-core/issues/222)

## Running and updating

```sh
cargo test --manifest-path crates/jade-docx-native-render/Cargo.toml --locked --test word_mac_coordinates -- --test-threads=1
```

On coordinate failure, `crates/jade-docx-native-render/target/word-mac-coordinates/`
contains `actual.json` and `differences.txt`, uploaded by the CI failure-artifact
step. Hash/schema/coverage failures report their assertion directly.

Review any intended layout change against Word before updating the snapshot:

```sh
JADE_UPDATE_WORD_MAC_SNAPSHOT=1 cargo test --manifest-path crates/jade-docx-native-render/Cargo.toml --locked --test word_mac_coordinates -- --test-threads=1
```

Updating is forbidden when `CI` is set, and still requires all nine direct Word
comparisons to pass. Commit the reviewed snapshot diff together with the fix;
do not regenerate it merely to silence a failure. Add a direct Word probe to
the allowlist only after all its captured visible glyphs match the reference.


## Freezing newly established compatibility

Every Word-backed layout fix must add its unchanged DOCX and measured Word
reference to a mandatory CI regression test in the same change. Include source,
reference and font identities; assert complete page/text identity and the
relevant layout, rather than checking only that the document opens. Test the
previous implementation first to establish that the new assertion detects the
regression. Add edit/undo/save/reopen coverage in the consuming editor when the
fix affects editing or serialization.

The mode-14/15 horizontal floating-table pair is included in all 34 required
bundled-font snapshots. The captured Office-Arial face remains an additional
local whole-probe comparison; requiring a proprietary font is not a substitute
for the mandatory CI gate. The paired snapshots protect both the corrected
legacy placement and the unchanged modern placement.

A snapshot update is an intentional compatibility decision: review the Word
evidence and each changed glyph/page/hash, commit it with the implementation,
and document the reason. Do not automatically accept new output, increase
tolerances, remove a probe or disable the gate to make CI green. The workflow
contract test also requires the coordinate step to remain unconditional and
failure-blocking on pull requests.


The additional auxiliary snapshot lives in
`mac-native-auxiliary-coordinates.json`. Its initial capture uses the same
bundled-font aliases as the existing gate. Review all coordinate and identity
changes before explicitly updating it with
`JADE_UPDATE_WORD_MAC_AUXILIARY_SNAPSHOT=1` and the command above. This separate
maintenance switch is also forbidden in CI and does not update or relax the
34-probe body snapshot or the nine direct Word comparisons. The complete test
command must pass after either kind of snapshot maintenance.
