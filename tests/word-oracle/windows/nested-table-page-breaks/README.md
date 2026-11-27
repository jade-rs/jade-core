# Windows Word page breaks in nested tables

This archive contains vector PDF and print layout drawings from Windows
Word 16.0.19127.20936 for 48 read-only inputs. Its manifest records the
hashes of the input DOCX files, Word executable, PDFs, and page drawings.

The parent cell is 207 points wide. The inner table spans 198, 207, 216,
or 414 points. Each width covers a page break within a run, a page break
in its own run, a paragraph with page break before, and a control. The
fixtures run in compatibility modes 12, 14, and 15.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_nested_table_page_break_cases.py`.
The Windows gate compares page dimensions and glyph positions with the
archived Word drawings and vector PDFs.
