# Windows Word floating table overlap

This archive captures vector PDF and print layout drawings from Windows
Word 16.0.19127.20936 for 93 read-only DOCX inputs. Its manifest records
hashes for the inputs, Word executable, PDFs, and page drawings.

- Paired tables cover overlap restrictions on either table, opposite
  alignments, and authored positions just before, at, and after contact.
- Authored wrap distances are varied on both sides of a displaced table.
- Right-aligned tables cover page and margin anchors, different widths,
  cell margins, and leading-only, trailing-only, symmetric, and asymmetric
  borders.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_floating_table_overlap_cases.py`.
The Windows gate compares page dimensions and per-page glyph positions
in the layout and vector PDF with the archived Word outputs.
For the right-aligned border cases, it also compares the vector rules in
both PDFs.
