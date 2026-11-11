# Mixed-script layout probes

These synthetic DOCX files are awaiting coordinate capture from Word for Mac.

- `arabic-body-bidi-anchors.docx` places Arabic spans between Latin text, numbers, and punctuation.
- `arabic-body-joining-boundaries.docx` varies run boundaries around joining letters and combining marks.
- `arabic-narrow-table-advances.docx` varies cell width across five pages, with six Arabic cases on each page. The widths are 19, 20, 22, 26, and 32 pt.
- `cjk-latin-mixed-size-boundaries.docx` varies adjacent CJK and Latin font sizes and script order to measure automatic inter-script spacing.

Export each source from Word for Mac before recording page sizes, glyph origins, and shaped span origins for the CI coordinate checker.
