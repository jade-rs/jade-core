# Word for Mac coordinate regression gate

`CI / native-macos` runs `word_mac_coordinates` for every pull request, without
Word or a locally installed Office font. It uses the native Skia renderer, the
Mac layout platform, and the repository's Noto Sans JP Regular/Bold faces as
stand-ins for the named document fonts. Other unregistered families fall back
to the first registered face; system fonts are not registered by this test.

Two separate assertions run:

- All 32 probes in `mac-probes-16114` and `mac-probes-20261019` must preserve the
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

This is not full Word compatibility coverage. `layout_debug_lines()` omits
some painted content (including numbering labels and ruby annotations), and
these 32 probes do not cover the full corpus or browser rendering. A native
snapshot guards against changes to current behavior, including current bugs;
it does not prove that the other 23 probes match Word. Relevant follow-ups:

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
