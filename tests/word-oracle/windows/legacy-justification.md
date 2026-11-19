These 80 controls retain actual Windows Word 16.0.19127.20936 output while
varying paragraph width, compatibility mode and Latin font. Thirty-two vary
the original Arial paragraph width; 48 compare Arial bold, Calibri and Times
New Roman with natural paragraph endings and manual breaks.

Each versioned capture keeps the original DOCX/PDF/EMF/XPS bytes, full Word
version/executable identity, font program/version facts and capture tools.
The generator reproduces all 80 original source files byte-for-byte.

CI requires 68 controls on all four original Word font coordinate planes
and paragraph backgrounds, plus 66 on both XPS render paths. It also renders
the 12 unpromoted inputs and checks text, pages, order, finite coordinates
and backgrounds for all 80. Missing or displaced required assertions are
covered by negative tests and workflow contracts. Mac originals and all
coordinate tolerances remain unchanged.

Eight natural Times New Roman controls expose missing kerning tables in
Word's extracted PDF font subsets; complete installed fonts pass those
controls. Four modern Times manual-break controls still differ with complete
fonts and remain a separate product issue. Two modern Arial width controls
also retain XPS differences. These pending observations are preserved with
their original coordinates and do not claim coordinate agreement.
