# Windows Word overflow across nested table cells

The archive contains vector PDFs and print-layout drawings from Windows
Word 16.0.19127.20936 for 54 read-only DOCX fixtures across 72 pages.
The fixtures exercise three levels of tables in compatibility modes 12, 14,
and 15. They cover horizontal overflow in growing rows, clipping in exact
middle or outer rows, and both rules in repeated headers on the next page.

The Windows gate compares complete layout origins and unclipped PDF glyphs
with Word's drawing and PDF, then checks the glyphs visible in both PDFs.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_nested_table_depth_cases.py`.
