# Word for Mac inline image positioning

The six synthetic documents are ready for Word measurement. Word coordinates
and CI assertions are pending.

- `inline-image-geometry-{grid,nogrid}.docx` vary image dimensions and drawing
  run position across 16 paragraphs per document, with mixed Latin and Japanese
  text around each image.
- `inline-image-position-{grid,nogrid}.docx` hold the image, font and text
  constant across eight paragraphs. The drawing run's `w:position` in
  half-points is omitted, 0, 2, 8, -2, -8, 16 and -16, respectively.

- `inline-image-rotation-{grid,nogrid}.docx` apply 0°, 30°, 90° and 315°
  rotations to each image geometry and run position from the geometry cases.

Export each DOCX from Word for Mac using Save As PDF, Best for printing. Compare
each picture rectangle and every visible glyph origin in page coordinates.
The paired documents isolate the effect of the section's line grid. The
position-only cases also allow comparison of the image bottom against its
paragraph's text baseline.

For the rotation cases compare the page-space bounds of the transformed image
and the glyph origins before and after it. Use the original geometry cases as
controls for rotation-induced changes in wrapping and baselines.
