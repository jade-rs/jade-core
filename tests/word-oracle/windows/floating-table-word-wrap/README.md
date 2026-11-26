# Windows Word whole words beside floating tables

This archive contains vector PDF and print layout drawings from Windows
Word 16.0.19127.20936 for 63 read-only DOCX fixtures in compatibility
modes 12, 14, and 15. The manifest records hashes for the DOCX inputs,
Word executable, PDFs, and page drawings.

- Left-, center-, and right-aligned tables leave gaps of 24, 27, 36, or
  48 points beside preceding body paragraphs.
- Paragraphs following the table anchor start with a whole word, a
  single letter, or a letter followed by a wider word.
- The cases measure which side receives text and where the whole word
  starts below the table when it cannot fit.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_floating_table_word_wrap_cases.py`.
The Windows gate compares page dimensions and glyph positions with the
archived Word drawings and vector PDFs.
