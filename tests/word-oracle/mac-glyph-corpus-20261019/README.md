# Word for Mac glyph corpus, 16.114 recapture

The 118 documents of `mac-glyph-corpus-20261010`, exported again on
2026-10-19 for jlsi/jade#504. That corpus stays unchanged; this is a second
observation with its own provenance.

Capture environment: Microsoft Word for Mac 16.114 (build 16.114.26101123),
macOS 27.0.1, Japanese UI, the fonts shipped with Word
(`/Applications/Microsoft Word.app/Contents/Resources/DFonts`). Each
`public/fixtures/<name>.docx` was opened and saved as PDF through AppleScript,
with the window's view set to the final document without markup:

```applescript
set v to view of active window
set show revisions and comments of v to false
set revisions view of v to revisions view final
save as d file name … file format format PDF
```

The JSON is `scripts/word-pdf-glyph-reference.py` run on each PDF in `pdf/`.
`SHA256.tsv` records the JSON, DOCX and PDF hashes. Two extraction fixes made
for this corpus change exactly one reference each, from the same PDFs:

- `yokohama_table`: the script no longer clips to the page. Word writes the
  third column of the 1080pt table past the 612pt page edge, and jade lays it
  out there too.
- `noto-jp-yokohama_page6`: Noto Sans JP draws `長`, `民` and `・` with the
  glyphs of `⻑`, `⺠` and `‧`, and the PDF names those code points. The script
  maps them back when the document contains the first and not the second.

## A different Mac from the earlier captures

This corpus was captured on a different Mac from both
`mac-glyph-corpus-20261010` (Word 16.113) and `mac-probes-16114`. Differences
between the two corpora come from the machines as well as the Word versions,
so compare them document by document before reading one as a layout rule:

- 97 of the 118 references are identical to the 2026-10-10 ones row for row.
- 13 documents name ヒラギノ角ゴ ProN W3 (`hello`, `break`, `header`,
  `hyperlink`, `line-spacing`, `align-center`, `run-styles`, `image-inline`,
  `image-floating`, `table2x2`, `table3x2merged`, `textbox`,
  `textbox-dashed`). The earlier machine's Word drew them in Cambria; this one
  draws Hiragino, as jade does on a Mac.
- The earlier capture printed tracked changes and comments inline
  (`numbering-del`, `numbering-del2`, `numbering-with-review`, `review`,
  `review-kinds`, `review-duplicate-ids`, `comment`). This one prints the final
  document, as jade draws it.
- `math`, `rich-paste-cr` and `unicode-segmentation` differ in a few glyph
  fonts or positions.

Three probes of `mac-probes-16114` (`pairs-kerned`, `squeeze-k5-b2`,
`exact-grid`) captured again on this machine are identical to the originals,
so the layout rules agree between the two Macs.

Run the corpus after building the playground. `--changes false` lays every
document out in the final view as well, including `review-kinds` and
`review-duplicate-ids`, whose saved `w:revisionView` would otherwise show
their markup:

```sh
node tests/playwright/word-glyph-sweep.mjs \
  tests/word-oracle/mac-glyph-corpus-20261019 \
  --platform mac --changes false --expected-fixtures 118
```
