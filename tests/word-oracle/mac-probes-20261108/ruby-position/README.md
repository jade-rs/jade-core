# Word for Mac: ruby guide position

Captured 2026-11-08 with Microsoft Word for Mac 16.114.3. `export.applescript`
opens only the generated `ruby-position.docx` and saves it as PDF. The template
is the existing no-grid ruby probe; the generator replaces its body. The script
requires absolute DOCX/PDF paths. Run `make_probe.py` then `extract.py` (PyMuPDF)
to rebuild the input and re-extract original glyph origins from the exported PDF.
The glyph JSON records both source and PDF SHA256 hashes. Coordinates are PDF
points from the page top; preserve individual glyph origins rather than grouping
nearby baselines. Rendered PDF was visually checked as well.

All cases use Arial 8pt guide ABC, Arial 12pt BASE, hpsRaise=30 (15pt).

| guide properties | base baseline | guide baseline | base minus guide |
| --- | ---: | ---: | ---: |
| normal | 94.56 | 79.68 | 14.88 |
| position +2pt | 133.68 | 118.56 | 15.12 |
| position −3pt | 172.80 | 157.92 | 14.88 |
| superscript | 211.92 | 193.68 | 18.24 |
| subscript | 251.28 | 236.88 | 14.40 |
| superscript + position +2pt | 290.40 | 272.16 | 18.24 |
| position −30pt | 329.52 | 314.64 | 14.88 |

Word ignores explicit `w:position` within `w:rt`: the −30pt case leaves the guide
above the base, and its following line remains 13.44pt below the base. The
superscript with +2pt position has exactly the same relative baseline as the
superscript without it. Minor 0.24pt baseline quantization is visible in this
Word PDF export. Superscript/subscript still shrink and move the guide.

This evidence checks relative guide positioning in these horizontal explicit-raise
cases. It does not establish full native/Word coordinate parity, omitted-raise
behavior, vertical ruby, or ruby wrapping.

## Native comparison

The opt-in integration test `ruby_word_positions` checks all seven relative
base/guide baselines against this PDF. It verifies source and PDF hashes and
requires the captured Office Arial font hash. It uses a 0.375 pt tolerance
(half a CSS pixel), with no fitted baseline or page-origin offset. The test
covers relative vertical placement only; absolute coordinates, line spacing,
font sizes and horizontal glyph advances are outside its assertions.

```sh
JADE_OFFICE_ARIAL='/Applications/Microsoft Word.app/Contents/Resources/DFonts/arial.ttf' \
  cargo +1.93.1 test \
  --manifest-path crates/jade-docx-native-render/Cargo.toml \
  --test ruby_word_positions -- --ignored --nocapture
```

Run from the `jade-core` repository. Office fonts remain local and are not
included in the fixture.

### 2026-11-08 comparison result — unresolved subscript difference

The opt-in test was executed and **fails** for the subscript case: Word's
base-minus-guide baseline distance is 14.40 pt; native is 14.00 pt, exceeding
the 0.375 pt tolerance. The other six cases are within tolerance.
`native-comparison.json` records every measured distance. The test remains
explicitly ignored by default as an Office-font-dependent, known failing
compatibility regression; this is not a passing Word parity check.

The current Arial path falls back to the general script baseline formula.
More Word measurements (including several sizes and ordinary text) are needed
to distinguish an Arial metric difference from ruby-specific behavior before
changing that shared formula. No tolerance or renderer value was changed to
make this comparison pass. `native-preview.png` was visually inspected beside
`word-preview.png`; its render uses the installed document fonts. The PNGs
have different raster scales and are not a pixel-difference assertion.
