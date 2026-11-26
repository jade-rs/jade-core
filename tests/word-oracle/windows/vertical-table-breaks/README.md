# Windows Word page breaks in vertical table text

The archive contains 117 original PDFs from Windows Word 16.0.19127.20936
after repaginating the read-only inputs. Its manifest records the Word and
DOCX hashes, PDF hashes, and physical page counts. The 39 cases cover
compatibility modes 12, 14, and 15.

- Word wraps vertical cell text within the height set by the horizontal
  cells in the row and the authored row height. Rows containing only
  vertical cells have their own automatic height.
- Page breaks within one run or in separate runs leave the vertical text
  in the same flow. The font controls include two point sizes for six
  faces so column pitch is checked independently of the row height.
- Japanese and Latin can use different orientation within one cell.
  The captures include Japanese, accented Latin, and mixed text in
  clockwise, bottom-up, and upright vertical writing.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_vertical_table_break_cases.py`.
The Windows page-break gate checks the unclipped layout and both vector PDF
projections against Word's physical pages and glyph origins. It also checks
the visible vector PDF's text sequence.
