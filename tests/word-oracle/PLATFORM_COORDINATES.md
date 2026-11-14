Shared Mac and Windows coordinate assertions
===========================================

`platform-coordinates.json` links each unchanged Mac corpus `source.docx` to its
existing Mac manifest and one or more Windows Word captures. Source bytes must
match the Mac source hash and the archived Windows input exactly. Each Windows
capture retains the complete Word product version, executable hash, OS, UTC
capture ID, original PDF and raw EMFs. Add new captures instead of replacing an
older Word version's evidence.

Expected coordinates belong to the Word platform/version that produced them.
Mac and Windows coordinates may differ; neither platform's expectation is
substituted for the other's. Mac tolerances remain those of the original Mac
manifest. Windows uses the existing 0.375 pt limit on both axes without fitting.

GitHub Actions runs the original Mac corpus browser assertions on macOS and
checks that every registered shared case remains included. On Windows, the
native renderer opens that same Mac original and must pass all four comparisons:
raw Word drawing origins, unclipped Word PDF text, visible Word PDF text, and
the independently painted native PDF. Page count, text/order and glyph count
must also match. Missing evidence or a mismatched platform, Word version,
source hash or reference hash fails CI.

The shared cases cover floating horizontal table placement, disabled hyphenation,
contextual paragraph spacing in compatibility modes 14/15, and paragraph-border
wrapping with Windows Word 16.0.19127.20936. The floating-table Mac original
records Word 16.114.2; the older hyphenation Mac capture does not record a Word
version. Its original PDF/coordinate hashes remain pinned; no version is inferred.
The full existing Mac corpus
and previous Windows version gates remain enabled. New cases should be added
only after all four Windows comparisons pass; failing captures remain evidence
for fixes, never new accepted coordinates.

The checker also requires every previously asserted shared case and its original
Windows version/capture ID. Adding newer Word versions cannot silently remove an
older version's assertion.

The separate `windows/font-coordinates.json` registry asserts 40 font/typography
documents (27,247 glyph origins) against Word 16.0.19127.20936. Its Windows CI
gate extracts font programs from the preserved original Word PDFs and passes
only those fonts and the original DOCX to the renderer. Expected coordinates
are never renderer inputs. This permits exact font testing on hosted runners
without installing Office fonts. The same four comparisons and 0.375 pt limit
apply. Coverage, source hashes, version metadata, original PDFs and EMFs are
checked before rendering. The public encrypted fixture uses its documented test
password and retains the encrypted source hash separately from decrypted bytes.

The shared registry also pairs the unchanged `footnote-mac` and
`footnote-table-each-page-mac` sources with their Windows observations. These
Windows checks use original PDF font programs so Office fonts are not required
on hosted runners. The note matrix retains all 24 Word captures; only the 12
fully matching matrix cases are promoted. Century superscript baselines and
20pt Yu Mincho marker widths remain unpromoted evidence, never accepted errors.
