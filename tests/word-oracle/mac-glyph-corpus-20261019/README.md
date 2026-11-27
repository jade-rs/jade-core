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

## Replay the corpus

Run from the repository root on macOS with the Word and macOS fonts installed
and PyMuPDF available to Python. The replay uses the current fixture when its
SHA-256 matches the capture and retrieves the recorded source revision from
Git otherwise. A shallow checkout must contain that revision.

```sh
cargo build --locked --manifest-path crates/jade-docx-native-render/Cargo.toml \
  --example word_coordinates
mkdir -p "$HOME/Library/Caches/jade-word-oracle"
cp -p crates/jade-docx-native-render/target/debug/examples/word_coordinates \
  "$HOME/Library/Caches/jade-word-oracle/word_coordinates"
python3 scripts/word-oracle/check_mac_glyph_corpus.py \
  "$HOME/Library/Caches/jade-word-oracle/word_coordinates" \
  mac-glyph-replay
```

The output directory must be new. Copying the executable into the cache keeps
it available while a shared Cargo cache prunes old targets. The replay opens
each document in final view with formatting marks hidden, including
`review-kinds` and `review-duplicate-ids`, whose saved revision view shows
markup. The renderer receives only the DOCX and these view options.

`report.json` records both comparisons for each fixture. The gate checks page
count and matches every non-whitespace glyph by text, page and origin, with a
tolerance of 0.375 pt on each axis. The four text-free references require the
same page count and zero glyphs. `yokohama_table` uses the PDF paint trace
because it includes text painted past the page edge. Five fixtures use the
archived JSON for the gate because direct PDF extraction changes glyphs.
Pending fixtures keep both results in the report and still require the
recorded page count and nonempty glyphs.

## Replay on 2026-11-27

[Per-fixture results](replay-20261127.tsv) cover all 118 documents at Jade
revision `81f90d38eda433d8387167b5385d767ec7f1cae8` on macOS 27.0.1
using PyMuPDF 1.28.2. The original Word 16.114 exports and JSON references
supply the expected coordinates. The replay used the recorded Git revision
for 48 inputs whose current fixture was missing or had a different hash.

108 fixtures passed against the PDF and five passed against the archived JSON.
The other five have pending comparisons:

| Fixture | Difference |
| --- | --- |
| `char-effects` | Word paints outline as paths, layers shadow, and uses separate highlights for emboss and imprint. |
| `math` | Word's Cambria Math PDF font maps some glyphs to private-use characters. Equation positions need a separate comparison. |
| `table-object-edge-cases` | Word's PDF exposes Arabic presentation forms. Matching Arabic scalars for diagnosis reveals up to 2.94 pt difference in glyph origins. |
| `toc-tc-field-edge-cases` | Word moves visible text across a line break around four empty simple TC fields. |
| `unicode-segmentation` | Word paints the joined family emoji as one glyph. Jade's PDF shows missing glyphs over the following text. |

The five archived-reference results and five pending comparisons are tracked
in [issue #222].

[issue #222]: https://github.com/jade-rs/jade-core/issues/222
