# Windows Word floating table continuations

The archive preserves the PDF and print layout drawings produced on Windows
by Word 16.0.19127.20936 for 57 read-only DOCX inputs in compatibility modes
12, 14, and 15. Its manifest records hashes for the inputs, Word executable,
PDFs, and individual page drawings.

- Long rows cross automatic page boundaries before and after a row's
  `pageBreakBefore`. The document can also contain two explicit breaks,
  repeated table headers, or a long row spanning three pages.
- In modes 12 and 14, the opening fragment of a repeated instance can share
  the preceding instance's final page. Word paginates that fragment using a
  full body height and paints it at the end of the preceding instance. The
  inputs cover content entering the bottom margin and crossing the page edge.
- The inputs vary rows at the one-line widow boundary and text, margin, and
  page anchors. Controls cover floating tables that span pages through
  automatic breaks alone.

Regenerate the DOCX inputs with
`python scripts/word-oracle/generate_windows_floating_table_continuation_cases.py`.
The Windows page-break gate verifies the DOCX hashes, compares physical page
dimensions, and matches glyph origins against Word's archived PDF and print
layout.
