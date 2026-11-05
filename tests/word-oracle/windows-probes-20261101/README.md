# Windows Word coordinate evidence, 2026-11-01

Captured with the installed Windows Word **16.0.19127** using a separate COM
instance, read-only document opens, Repaginate and ExportAsFixedFormat (PDF).
Source documents were not resaved. `coordinates.json` records the DOCX and PDF
SHA-256 hashes and the unaligned PDF glyph origins, in points from the page's
top left. Extraction uses PyMuPDF rawdict character origins, not ink boxes.

Seven fixtures cover 54 pages and 5,106 non-whitespace glyphs:

- Four controlled fixtures: Arial, Times New Roman, MS Mincho and Yu Mincho;
  10.5, 12 and 20pt; three single-spaced paragraphs per case; with/without
  `useFELayout`, with/without an 18pt document line grid (48 pages, 1,800 glyphs).
- The existing shared `word-mac-probe-pairs-kerned` fixture rendered by Windows
  Word (one page, 96 glyphs).
- The existing shared `word-mac-probe-squeeze-plain` fixture rendered by Windows
  Word (four pages, 2,695 glyphs).
- The existing shared `word-mac-probe-indent` fixture rendered by Windows Word
  (one page, 515 glyphs, including the separately painted numbering labels).

`word_windows_coordinates` verifies exact page counts, glyph counts, text and
spatial order, and every glyph's X/Y origin within **0.375pt**. It applies no
translation, scaling, tolerance fitting or snapshot updates. The production
font lookup registers installed faces; Windows font installation is required.

Findings reproduced by this evidence:

- Known font line boxes apply even without `useFELayout` on Windows.
- Yu Mincho uses OS/2 usWinAscent/usWinDescent (2038/598 per 2048 em), unlike
  the Mac hhea box (1802/455).
- A snapped line centres its font box in whole authored grid pitches. A 20pt
  run on an 18pt grid takes 36pt, rather than replacing the grid pitch with a
  body-height heuristic. Tiny placeholder-pitch handling is preserved.
- Fixed Japanese faces retain full cells; heuristic 1% widening and 18%
  punctuation shrink were causing wrong pair positions and a spurious fifth
  page in the four-page squeeze probe. Kerned adjacent punctuation shares
  its touching half-em blank.
- A numbering suffix space uses an Arial-width space even when the paragraph
  mark names MS Mincho; using an MS Mincho half-em shifts the body and wrap.

Mac references and snapshots are unchanged. Run `word_mac_coordinates` as well
to verify the Mac policy and bundled-font coordinate snapshots on Windows.
This does not replace a macOS/CoreText/GPUI end-to-end run.

The parent jade-office repository provides `scripts/test-windows-word.ps1`,
`capture-word-windows.ps1`, `make-windows-coordinate-probe.py` and
`compare-word-coordinates.py`. The native renderer example `word_coordinates`
captures both body and auxiliary (numbering/ruby) glyphs, and a first-page PNG.
Reference JSON files are checked out byte-for-byte (`-text`) so Windows CRLF
conversion cannot invalidate evidence hashes. Captured PDFs are retained for
independent re-extraction and visual inspection.

`pending/contract.json` and its PDF retain additional Windows Word evidence
that has **not** passed the strict 0.375pt gate. After the fixes, all 1,907
glyphs and all three pages correspond, with X/Y median absolute differences
of 0.144/0.157pt and 95th percentiles of 0.314/0.207pt. 111 glyphs still exceed
0.375pt in at least one axis. This case is explicitly pending, not included
among the seven passing fixtures; neither tolerance nor Mac snapshots were
relaxed to hide it.

The bundled-standin baseline test runs on every OS, including Mac CI: 144
Windows Word line baselines across all four controlled fixtures. It checks
page count and vertical policy; the Windows-only installed-font test checks
both axes, text, glyph counts and numbering for all seven passing fixtures.
