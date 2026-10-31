# Word for Mac script baseline recapture

`word-mac-probe-vert-align.json` was captured by Word for Mac 16.114.2
(build 16.114.26102345) on macOS 27.0.1 from the unchanged
`public/fixtures/word-mac-probe-vert-align.docx`. Word used Save As > PDF,
optimized for printing (local export). The PDF SHA-256 and source DOCX SHA-256
are embedded in the JSON. No document edits were made before exporting.

The PDF remains at the capture workstation:
`~/Downloads/Jade-Word-Probes-20261030/jade-script-baseline-20261030.pdf`.

Extracted with the SDK `scripts/word-pdf-glyph-reference.py` at commit
9311d73ff821cd2bdb224601e780528c6574c613, using PyMuPDF. This version preserves
an individual baseline on every glyph, including subscript glyphs grouped
within 1pt of the body row. The older 20261019 reference only retained a row
baseline; treating it as each glyph's actual baseline loses up to 0.96pt.
Both captures contain the same 224 visible glyphs at identical horizontal
origins, with identical page and text order.

This is diagnostic evidence, not a claim that this probe passes strict native
or browser coordinate parity. The existing reference is retained unchanged.

## Inline-object split-space capture

The balanced/unbalanced DOCX pair was generated with the opt-in `--space-runs`
mode of `mac-probes-20261019/generators/make-object-spaces-probe.py` at core
3218f290b2a1fa933690cc081bab3d006180613d, using
`public/fixtures/image-inline.docx`. Their document XML is identical; only
`balanceSingleByteDoubleByteWidth` differs. Exact generated inputs are stored
in `public/fixtures/word-mac-probe-object-space-runs-{balanced,unbalanced}.docx`.
The case manifest records paragraph order, including split runs and text/floating
controls.

Both were exported without edits using the same Word/macOS/local PDF settings
as above. PDF files remain in `~/Downloads/Jade-Word-Probes-20261030/`. JSON
records the exact source/PDF hashes and individual glyph baselines. Both have
one page, 14 rows, and 225 visible glyphs. All corresponding baseline positions
are identical between variants. The table reports the largest horizontal change
per paragraph in PDF points; it is Word-to-Word evidence, not native parity.

| Case | Maximum balanced/unbalanced x difference (pt) |
| --- | ---: |
| split-single-before | 2.523 |
| single-both | 5.045 |
| paragraph-end | 0.000 |
| paragraph-start | 2.523 |
| long-line | 5.046 |
| floating-single | 0.000 |
| two-before | 5.045 |
| three-before | 7.568 |
| two-after | 5.045 |
| three-after | 7.568 |
| split-three-before | 7.568 |
| split-three-after | 7.568 |
| floating-three | 7.568 |
| text-three | 7.572 |

Laid out in Word for Mac's own Yu Mincho, both variants and the original
`object-spaces` probe match Word within half a CSS pixel: run
`object_spaces_match_word_with_office_yu_mincho` (ignored by default) with
`JADE_OFFICE_YU_MINCHO` pointing at `Microsoft Word.app/Contents/Resources/DFonts/yumin.ttf`.
