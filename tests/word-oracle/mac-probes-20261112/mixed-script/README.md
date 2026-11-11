# Mixed-script layout probes

These synthetic DOCX files are awaiting coordinate capture from Word for Mac.

- `arabic-body-bidi-anchors.docx` places Arabic spans between Latin text, numbers, and punctuation.
- `arabic-body-joining-boundaries.docx` varies run boundaries around joining letters and combining marks.
- `arabic-narrow-table-advances.docx` varies cell width across five pages, with six Arabic cases on each page. The widths are 19, 20, 22, 26, and 32 pt.
- `cjk-latin-mixed-size-boundaries.docx` tests 10 pt and 20 pt kanji and hiragana adjacent to Latin letters and digits in both orders. It also tests explicit spaces and disabled automatic spacing.

- `hebrew-bidi-combining.docx` varies paragraph direction and alignment with pointed Hebrew, Latin text, parentheses, and decimals. It also splits one Hebrew word into runs.
- `devanagari-cluster-boundaries.docx` compares contiguous and split runs with a pre-base vowel, conjunct, and reph. Latin brackets and text mark the positions on either side.

Export each source from Word for Mac before recording page sizes, glyph origins, and shaped span origins for the CI coordinate checker.
