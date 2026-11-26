# Windows Word page breaks in floating tables

The archive contains the Windows Word 16.0.19127.20936 PDF and print layout
drawings for 81 read-only DOCX inputs. The 27 cases cover compatibility
modes 12, 14, and 15. The manifest pins the exact input, application,
PDF, and page drawing bytes.

- `pageBreakBefore` on the first row can move the floating table and the
  following body text to a new page. A later break splits the table and
  makes Word draw additional copies of the fragments.
- In modes 12 and 14, a copy can begin below the last fragment on the
  previous copy's last page. In mode 15, it starts on a fresh page.
  The captures include multiple breaks and repeated table headers.
- The captures vary text, margin and page anchors, direct and inherited
  paragraph settings, inline break placement, and horizontal wrap
  distances.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_floating_table_break_cases.py`.
The Windows gate matches physical page dimensions and glyph origins in
JADE's layout and vector PDF against Word's two outputs. The body and
table are independent drawing flows, so glyphs are matched by position
within their physical page.
