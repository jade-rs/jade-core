# Word for Mac coordinate fixtures

Each fixture contains a DOCX, a compressed coordinate reference extracted from a Word for Mac PDF, and a manifest recording the source, export, and reference digests. The macOS browser coordinate job builds the DOCX renderer from this repository and compares page sizes, visible glyph origins, and image bounds to those references.

To add a fixture, export its DOCX from Word for Mac using **Save As → PDF → Best for printing**. Record the export and SHA-256 digests of the DOCX and PDF in a manifest based on an existing fixture, then run the extractor with PyMuPDF 1.26.5.

```sh
python tests/word-oracle/mac-elsa-synthetic/extract.py path/to/fixture path/to/word-export.pdf
```

Commit the synthetic DOCX, manifest, and generated `pages.json.gz`.

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

The Indic line-box fixtures check glyph baselines and following paragraph
origins across three pages at 10.5, 14, and 18 pt for Kohinoor Devanagari,
Devanagari Sangam MN, Kohinoor Bangla, Kohinoor Gujarati, Kohinoor Telugu,
and the Kannada, Malayalam, Tamil, Gurmukhi, and Oriya Sangam MN faces.

`latin-join-controls-fit-boundaries` covers ZWNJ and ZWJ around Latin
letters in fixed-width Arial table cells, including controls in separate
runs and cells narrow enough to force emergency wrapping. The five pages
also cover zero-width spaces and widths where the letters fit together.
