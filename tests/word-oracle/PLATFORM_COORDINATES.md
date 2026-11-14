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

The initial shared cases cover floating horizontal table placement and disabled
hyphenation with Windows Word 16.0.19127.20936. The floating-table Mac original
records Word 16.114.2; the older hyphenation Mac capture does not record a Word
version. Its original PDF/coordinate hashes remain pinned; no version is inferred.
The full existing Mac corpus
and previous Windows version gates remain enabled. New cases should be added
only after all four Windows comparisons pass; failing captures remain evidence
for fixes, never new accepted coordinates.
