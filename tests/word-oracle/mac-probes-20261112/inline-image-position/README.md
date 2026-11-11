# Word for Mac inline image positioning

The four synthetic documents are ready for Word measurement. Word coordinates
and CI assertions are pending.

- `inline-image-geometry-{grid,nogrid}.docx` vary image dimensions and drawing
  run position across 16 paragraphs per document, with mixed Latin and Japanese
  text around each image.
- `inline-image-position-{grid,nogrid}.docx` hold the image, font and text
  constant across eight paragraphs. The drawing run's `w:position` in
  half-points is omitted, 0, 2, 8, -2, -8, 16 and -16, respectively.

Export each DOCX from Word for Mac using Save As PDF, Best for printing. Compare
each picture rectangle and every visible glyph origin in page coordinates.
The paired documents isolate the effect of the section's line grid. The
position-only cases also allow comparison of the image bottom against its
paragraph's text baseline.
