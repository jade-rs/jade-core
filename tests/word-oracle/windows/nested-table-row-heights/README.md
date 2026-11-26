# Windows Word nested tables and parent row height

The archive contains vector PDFs and print-layout drawings from Windows
Word 16.0.19127.20936 for 90 read-only DOCX fixtures. The inputs exercise
exact row heights from 20 to 60 points, a minimum-height control, parent
cell spacing, and several ways of authoring a page break inside the inner
table. Repeated-header cases verify the clip on the first page and after
pagination. All cases cover compatibility modes 12, 14, and 15.

Word's print-layout drawings retain glyph origins outside an exact row's
clip. Its visible PDF clips those glyphs. The Windows gate compares the
complete layout and unclipped vector glyphs, then checks the visible PDF
against Word's visible output.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_nested_table_overflow_cases.py`.
