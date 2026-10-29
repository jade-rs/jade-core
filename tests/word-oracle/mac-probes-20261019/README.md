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
| `floating-order` | A table whose `w:tblpPr` comes before `w:tblStyle`, against the schema's order, is laid out inline where it stands; the same table with the elements in order floats at the right margin and text wraps around it. Word also narrows the single-line paragraph just above the floating table, whose line clears the table but whose space after does not. Jade now includes this space-after overlap for mode-14 Mac body paragraphs, using the natural paragraph bottom before narrowing the line. The table frame stays fixed; drawing look-ahead uses its separate rules. The full probe remains outside the strict Word-coordinate set because the floating cell text still has a horizontal offset; see `docs/word-mac-floating-table-spacing.md`. |
| `float-lookahead` | A paragraph before a floating shape's anchor paragraph, in compatibility mode 15. It keeps the full width when the shape sits at or below its end, whatever its space after or bottom border; a line that the shape, placed 24pt above its paragraph, overlaps wraps around it, so the earlier paragraph takes three lines. In compatibility mode 14 (floating-wrap-sides, floating-square-wrap) Word also lays a paragraph out around a paragraph-relative shape that only touches the end of its space after, when the paragraph's text crosses the shape, and leaves the shape where that first pass put it; jade reproduces that mode 14 look-ahead for square wrapping. In mode 15 jade now applies actual overlap from the next anchor on the same flow surface, including its first-pass position when the preceding paragraph grows. Paragraphs already continuing to another page remain outside this same-surface look-ahead. The full probe is not yet in the strict Word-coordinate set. |
| `kern-default` | Kerned runs close up every pair of the face's legacy `kern` table, including pairs with a space, which Arial's and Times New Roman's GPOS leave out. |
| `merged-rows` | Rows beside a cell merged over every row, in MS PGothic 10.5pt with 4pt margins. A row ends where its tallest cell's bottom margin and bottom border end, less the widest bottom border in the row, merged cells counting with their own margin and declared border: under continuations without `w:tcMar`, a 1.5pt merged border makes each row it passes 1.2pt shorter and a 3pt one 2.9pt; 8pt of continuation bottom margin makes each row 4pt taller. An at-least row is its height plus the padding, with the bottom border below. |
| `object-lines` | A line that an inline picture makes taller than its auto-spaced text is the picture, the text's descent and the spacing multiple's extra over single spacing, set below: a 20pt picture after `x ` in 10.5pt Arial takes 22.2pt at single spacing, 24.0pt at 1.15 lines and 28.2pt at 1.5. A line of nothing but a picture taller than the mark leaves out the descent: 20.1, 21.9 and 26.1pt. An inline VML text box's line also holds its outline below the box, 0.96pt for the default 0.75pt stroke (2.88pt for a 3pt one in a capture not committed here). |
| `object-spaces` | With `w:balanceSingleByteDoubleByteWidth` (image-inline's Japanese template), an ASCII space right before or right after an inline picture is half an em, in Yu Mincho and Century alike (5.25pt at 10.5pt, 10pt at 20pt). The other spaces in the line keep their advance, as do spaces beside a floating picture and every space without the setting. A run of spaces before the picture widens as a whole; jade widens the one next to it, so the probe leaves that case out. |
| `partial-grid` | An autofit table whose `w:tblGrid` lists fewer columns than its three cells is sized from its content: each column gets its widest content plus a share of the rest in proportion to it. A 50% table is 253.8pt on a 496.8pt text width in compatibility mode 14 (the cell margins hang outside the text). |

## Consecutive-space capture inputs

The original `object-spaces` observation above omits consecutive spaces.
Generate separate inputs before measuring those cases in Word:

```sh
python3 tests/word-oracle/mac-probes-20261019/generators/make-object-spaces-probe.py public/fixtures/image-inline.docx /tmp/word-space-runs --space-runs
python3 -m unittest discover -s tests/word-oracle/mac-probes-20261019/generators -p test_object_spaces_probe.py
```

This writes `word-mac-probe-object-space-runs-balanced.docx`,
`word-mac-probe-object-space-runs-unbalanced.docx`, and a JSON paragraph
manifest. The pair has identical document XML; only the compatibility setting
`balanceSingleByteDoubleByteWidth` differs. It retains the original six
paragraphs and adds two/three spaces before or after the picture, three spaces
split across runs, a floating-picture control, and a text-only control. Drawing
IDs are unique and all pictures are 20pt. Without the flag, the original
six-paragraph probe and output name remain unchanged.

These generated files have **not** been measured in Word. Export each as PDF
in Word, then capture every glyph with the SDK's
`scripts/word-pdf-glyph-reference.py`. Retain both source/PDF hashes, Word
version/build, OS, and matching paragraph manifest with the fresh references.
Do not replace the existing `object-spaces` oracle or assert a new space-width
rule from these unmeasured inputs.
