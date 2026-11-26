# Windows Word nested table alignment

The archive contains vector PDFs and print-layout drawings from Windows
Word 16.0.19127.20936 for 150 read-only DOCX fixtures across 168 pages.
The inputs exercise the alignment and indentation of fixed-width nested
tables in compatibility modes 12, 14, and 15. They cover leftward and
rightward overflow in growing rows, clipping in exact rows, and the
placement of a nested table when its enclosing header repeats.

The Windows gate compares complete layout origins and unclipped PDF glyphs
with Word's drawing and PDF, then checks the glyphs visible in both PDFs.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_nested_table_alignment_cases.py`.
