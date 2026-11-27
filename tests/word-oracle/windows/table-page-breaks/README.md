# Windows Word page breaks in tables

The archive contains 114 original PDFs exported by Windows Word
16.0.19127.20936 after repaginating the read-only DOCX inputs. Its manifest
records the Word executable and input hashes, PDF hashes, and physical
page counts. The 38 cases cover compatibility modes 12, 14, and 15.

- A page or column break inside a cell leaves the surrounding text on the
  same line in Word. A soft break still starts a new line, and a paragraph
  containing only a page break still occupies one paragraph line.
- Direct or inherited `pageBreakBefore` in the first paragraph of the
  first cell starts the row on the next physical page, even in a
  multicolumn section. The same setting in other paragraphs or cells,
  or inside a nested table, has no effect on pagination.
- Windows Word repeats preceding header rows above the displaced body row.
  In mode 15 it also moves the original leading headers to a fresh page
  when the first body row is forced forward. Modes 12 and 14 can break
  before a later header row, and mode 15 keeps that row with its earlier
  headers.
- The table placement controls compare explicit and omitted indentation,
  explicit cell margins, and authored and implicit default table styles.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_table_page_break_cases.py`.
The Windows page-break gate compares the layout snapshot including repeated
header instances, and checks the vector PDF's page dimensions, character
sequence, and glyph origins against Word.
