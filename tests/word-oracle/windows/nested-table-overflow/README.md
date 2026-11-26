# Windows Word nested tables in growing rows

The archive contains vector PDFs and print-layout drawings from Windows
Word 16.0.19127.20936 for 96 read-only DOCX fixtures. Each placement is
tested in compatibility modes 12, 14, and 15.

The inner table fits its enclosing cell or spans twice its width. The cases
cover the second cell, a right-to-left parent table, a parent with cell
spacing, and a repeated header. They exercise page breaks in the same run,
in their own run, and before a paragraph, with a no-break control.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_nested_table_overflow_cases.py`.
The Windows gate compares page dimensions and glyph origins with both captured
outputs.
