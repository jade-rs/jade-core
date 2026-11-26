# Windows Word text wrap before floating tables

The archive contains PDF and print layout drawings from Windows Word
16.0.19127.20936 for 151 read-only DOCX inputs. Its manifest records
hashes for the DOCX files, Word executable, PDFs, and page drawings.

- Page, margin, and text anchors can position a table above its logical
  body location. The cases cover positive and negative offsets, omitted
  offsets, one-twip displacements, and explicit top alignment.
- Table placement and wrapping are measured around the page bottom in
  compatibility modes 12, 14, and 15. In legacy modes some table rows
  move to the next page. In mode 15, a table that fits a fresh page
  can move there together with the following text.
- The cases include paragraphs moved by widow/orphan control. The two
  mode-15 cases with a word wider than the available side are pending.

Regenerate the DOCX inputs with
`python scripts/word-oracle/generate_windows_floating_table_wrap_cases.py`.
The Windows gate compares physical page dimensions and per-page glyph
origins in the layout and vector PDF against Word's archived outputs.
