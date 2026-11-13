# Word for Mac coordinate fixtures

Each fixture contains a DOCX, a compressed coordinate reference extracted from a Word for Mac PDF, and a manifest recording the source, export, and reference digests. The macOS browser coordinate job builds the DOCX renderer from this repository and compares page sizes, visible glyph origins, and image bounds to those references.

To add a fixture, export its DOCX from Word for Mac using **Save As → PDF → Best for printing**. Record the export and SHA-256 digests of the DOCX and PDF in a manifest based on an existing fixture, then run the extractor with PyMuPDF 1.26.5.

```sh
python tests/word-oracle/mac-elsa-synthetic/extract.py path/to/fixture path/to/word-export.pdf
```

Commit the synthetic DOCX, manifest, and generated `pages.json.gz`.

Fixtures with `capturePaintedGlyphs` compare text draw origins before clipping.
The extractor reads those origins from the Word PDF text trace.

The right-to-left repeated table-header fixtures vary the header height
and starting column across unequal columns in a continuous section.
They check where the first header and repeated headers land and where
body rows split when a column has limited space.

The discretionary-hyphen fixtures compare the positions of painted dashes
alongside the other glyphs. The page-origins fixture covers authored soft and
nonbreaking hyphens across three pages with different widths and indents. The
width probe varies the available width from 26 to 90 pt over two pages and
checks both the dash and the continuation where a soft break changes wrapping.

The explicit hyphen wrap fixture checks authored soft, nonbreaking, and literal
hyphens around wrap boundaries. The styled hyphen fixture checks those
characters across run boundaries, in body text, and in narrow table cells.

The Latin zero-width space fixture checks six narrow table cells. When the
following letter is wider than the cell, Word can place the break opportunity on
a line of its own. The fixture checks where the visible letters land, including
when each letter fits the cell and when the text contains no break opportunity.

The Courier New line-box fixture checks body baselines and fixed-width table
cells at 10.5, 14, 18, and 21 pt.

The Geeza Pro line-box fixture checks baseline placement in one- and two-line
Arabic table cells at 10.5, 14, and 21 pt.

The Arabic row-fragment fixtures cover lam–alef wrapping at varying cell
widths, at the bottom of a page, and with different paragraph after-spacing
and row-splitting settings. An `arabicRuns` entry may be a pair of the text
extracted from Word's PDF and the logical DOCX cluster, for example when the
PDF text layer omits a combining mark. The extractor still verifies the PDF
text and the comparison uses its measured origin.

The Indic line-box fixtures check glyph baselines and following paragraph
origins across three pages at 10.5, 14, and 18 pt for Kohinoor Devanagari,
Devanagari Sangam MN, Kohinoor Bangla, Kohinoor Gujarati, Kohinoor Telugu,
and the Kannada, Malayalam, Tamil, Gurmukhi, and Oriya Sangam MN faces.

`devanagari-cluster-boundaries` checks pre-base vowel signs, conjuncts, and
reph alongside Latin text. The `shapedGlyphClusters` extraction recipe
verifies the PDF's visual-order glyphs and combines their leftmost origin
and shared baseline into the corresponding logical DOCX cluster.
`devanagari-mixed-kohinoor` checks Latin and Kohinoor Devanagari sharing
a line at different point sizes, including a manual line break.
`probe-devanagari-line-box-devanagari` checks Arial brackets and Devanagari
Sangam MN sharing a line at 10.5, 14, and 18 pt.

`latin-join-controls-fit-boundaries` covers ZWNJ and ZWJ around Latin
letters in fixed-width Arial table cells, including controls in separate
runs and cells narrow enough to force emergency wrapping. The five pages
also cover zero-width spaces and widths where the letters fit together.

`latin-join-control-line-metrics` checks the extra line height of a
soft-wrapped ZWJ in narrow Arial table cells. Its twelve pages cover four
font sizes, single and 1.15 line spacing, and inherited and explicit spacing.

`zwnj-table-line-metrics` checks isolated soft-wrapped ZWNJ lines in narrow
Verdana and Courier New table cells. Its eight pages cover four sizes per
font and check the position of both the following glyph and the paragraph
after the table.

The column-width controls cover a fixed preferred width in equal columns,
a fixed cell narrower than its preferred table, a vertical merge starting in
the second column, and an explicit column break in a continuous section.
Two nine-row tables check unequal columns in both wide-first and
narrow-first order.
