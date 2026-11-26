# Windows Word preferred widths in nested tables

The archive contains vector PDFs and print-layout drawings from Windows
Word 16.0.19127.20936 for 240 read-only DOCX fixtures. The cases cover
fixed and autofit nested tables with absolute, percentage, automatic, and
omitted width. They exercise cell and grid preferences, alignment and cell
spacing, plus page breaks and clipping in growing and exact rows in
compatibility modes 12, 14, and 15.

The Windows gate compares complete layout origins and unclipped PDF glyphs
with Word's drawing and PDF, then checks the glyphs visible in both PDFs.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_nested_table_preferred_cases.py`.
