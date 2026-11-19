These 80 controls retain actual Windows Word 16.0.19127.20936 output while
varying paragraph width, compatibility mode and Latin font. Thirty-two vary
the original Arial paragraph width; 48 compare Arial bold, Calibri and Times
New Roman with natural paragraph endings and manual breaks.

Each versioned capture keeps the original DOCX/PDF/EMF/XPS bytes, full Word
version/executable identity, font program/version facts and capture tools.
The generator reproduces all 80 original source files byte-for-byte.

CI requires all 80 controls on all four original Word font coordinate planes
and paragraph backgrounds, plus original XPS origins on both native and live
render paths. Every input must preserve text, pages, order and finite
coordinates. Negative tests reject displacements on each axis and path,
missing glyphs and changes to the versioned source or font inputs.

Twelve Times New Roman controls use font programs from the same original
Word XPS because Word's PDF subsets omit required kerning data. Each font
input records its original XPS SHA-256; decoded program hashes, family and
font version are recorded with render evidence. Decoded programs are
byte-exact. A strict subset is excluded only after matching face identity,
mapped glyph IDs, advances, outlines and any shared shaping tables. This
prevents an incomplete same-face subset from shadowing space kerning.
Original XPS files remain in the capture bundles; standalone decoded font
programs are excluded from uploaded CI render artifacts.

The Windows-only modern mixed-script allocator uses the common Latin face's
average width to cap script-boundary expansion. Its proportional word-space
allocation remains separate from the compatibility-mode-14 allocator. These
controls now pass with their original coordinates. Mac originals and all
coordinate tolerances remain unchanged; macOS CI requires its independent
original Word references.
