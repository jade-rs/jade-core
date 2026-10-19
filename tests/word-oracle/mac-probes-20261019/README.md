# Word for Mac 16.114 probes, second Mac

Each `word-mac-probe-*.json` is the glyph reference
(`scripts/word-pdf-glyph-reference.py`) of the PDF that Word for Mac exported
from `public/fixtures/<same name>.docx`, written for jlsi/jade#504.

Capture environment: Microsoft Word for Mac 16.114 (build 16.114.26101123),
macOS 27.0.1, Japanese UI, the fonts shipped with Word, Save As > PDF through
AppleScript with the window showing the final document without markup. This
is a different Mac from the one that captured `mac-probes-16114`; three of
those probes captured again here are identical to the originals.

`generators/` holds the scripts that wrote the documents. Their styles set
`w:kern w:val="2"` in `w:rPrDefault`, so every run is kerned.

| Fixture | What Word shows |
| --- | --- |
| `vert-align` | A superscript or subscript run is drawn at 0.65 of its size in Century and Calibri and 0.6 in Yu Mincho and MS Mincho, to the nearest half point (8–20pt). The raise is not modelled yet, so the spec does not run this probe. |
| `line-pitch` | Auto line spacing at 1, 1.15 and 1.5 lines in Arial, Calibri, Times New Roman and Century at 10–12pt. Pitches match jade; a 1.5-line line stays on the page while its text clears the page foot. Arial ` A` closes up by the legacy `kern` pair. |
| `kern-default` | Kerned runs close up every pair of the face's legacy `kern` table, including pairs with a space, which Arial's and Times New Roman's GPOS leave out. |
| `partial-grid` | An autofit table whose `w:tblGrid` lists fewer columns than its three cells is sized from its content: each column gets its widest content plus a share of the rest in proportion to it. A 50% table is 253.8pt on a 496.8pt text width in compatibility mode 14 (the cell margins hang outside the text). |
