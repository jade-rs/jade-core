# Yu Mincho Latin advance controls

These 24 independently opened, read-only Word documents distinguish repeated
letters, letter pairs, spaces, and inherited versus explicit-zero kerning.
They use 9.5pt and 16pt regular Yu Mincho, with three text runs per document.
The six patterns are `B`, `BC`, `z`, `z `, ` A`, and `AgABCxyz `.

`yu-latin-advance-observations.json` records every original source and archive
hash, capture identity, baseline result, and screen/print disagreement. All
captures came from Windows Word **16.0.19127.20936**, executable SHA-256
`7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312`.
The archives retain Word's PDF, page EMF, capture metadata, and opened DOCX.
The generator reproduces all 24 opened documents byte for byte from the pinned
16pt parent. Captures are retained even when the current renderer fails them.

Fourteen controls pass the original EMF, unclipped PDF, visible PDF, painted
native PDF, and paragraph-background comparisons. They are added to the strict
coordinate and edit/export/reopen corpora: **1,053 font cases, 161,303 glyph
origins, 613 paragraph-background cases, and 1,449 release documents**. The raw
registry contains **1,048 captures, 1,959 documents, and 1,912 source byte groups**.
Existing cases remain unchanged prefixes of both corpora.

Eight controls still expose feasible renderer differences: repeated `B` at
16pt, repeated `z` at 16pt, and the mixed pattern at both sizes, each with both
kerning settings. Their baseline failures remain recorded for further work.
They have not been added as passing coordinate fixtures.

The remaining two controls, repeated `z` at 9.5pt with inherited or zero kerning,
have a maximum **0.9636969534911941pt** disagreement between Word's own EMF and
PDF. A single geometry cannot be within the existing **0.375pt** tolerance of
both originals at those positions: the two tolerance intervals are disjoint.
The original archives are retained without averaging or altering either plane.
CI reconstructs both planes from the archives, asserts the disagreement, and
prevents these cases from being labelled as passing combined-plane controls.

Ten additional originals isolate `w:useFELayout` in five of these documents.
Only that compatibility property changes between each off/on pair. The generator
reproduces all ten byte for byte. Word's EMF and PDF origins are exactly identical
within every pair, ruling out this setting as the cause of the measured advance
differences for these controls. `yu-latin-fe-layout-observations.json` retains
the source/archive hashes and pair results; CI reconstructs the origins from the
original archives and asserts the equality. These ten diagnostic documents stay
in the raw registry because their renderer differences remain unresolved.

This fixture expansion changes no Rust rendering code, Mac reference, coordinate
tolerance, or workflow gating. Mac and Windows keep their separate Word
expectations. The baseline executable's exact source hashes and Git tree are
recorded in the observation file; that executable was built with Rust 1.93.1.
