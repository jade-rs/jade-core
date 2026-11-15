Windows legacy character-effect assertions
==========================================

`windows/graphic-text.json` pins 47 independently captured Windows paint cases:
the unchanged `char-effects.docx` and 46 reproducible controls. Captures retain
Word 16.0.19127.20936, the WINWORD executable SHA-256, OS, UTC capture ID,
byte-identical input, original PDF and raw EMF for every page. Each document was
opened read-only in a fresh Word process. Add new version directories and
observations rather than replacing an older version's originals.

The matrix covers plain, shadow, outline, emboss and imprint at 12/20/40 pt,
black/automatic and explicit red, Times New Roman/Arial/Calibri/Yu Mincho,
and compatibility modes 12/14/15. Regenerate the 46 controls with
`generate_windows_graphic_text_probes.py --output-directory <new-directory>`.
The generator tests require byte equality with the captured inputs.

The Windows CI gate checks all physical glyph copies from EMF against native
PDF text paint, visible PDF paint against visible native paint, RGBA groups,
page and glyph counts, and vector outlines. PDF text extraction alone can
deduplicate close shadow copies and cannot establish outline fidelity.
The outline check compares absolute bidirectional contour positions within
0.375 pt without alignment, fitting, scaling or rebasing. Stroke width must
agree within 0.01 pt; color, opacity, caps, joins and dashes must agree.
Deleting a contour, moving text, changing paint color or replacing vector
outlines with a raster image fails the check. Invisible native search text
remains available but is checked separately from visible paint.

Only font programs extracted from the original Word PDF and DOCX source bytes
are supplied to the renderer. Expected coordinates are never renderer inputs.
The unchanged original has outlined glyphs omitted from its embedded font
subset. Its linked plain font control removes exactly the four legacy effect
flags and preserves every other package part, text, font, hidden-text flag and
setting. Its Word PDF supplies the complete original font program; all expected
paint still comes from the unchanged original's own PDF and EMF.

Windows paints shadow as an ordered gray/source pair and emboss/imprint as
three ordered glyph copies. The measured emboss/imprint offsets have fixed
0.96 pt depth; the measured outline has a 0.14 pt stroke with round joins.
These changes apply only to isolated, unscaled Windows legacy effects without
OpenType feature overrides. Other effect combinations retain their prior path
until measured. Mac layout and paint policy are unchanged; the full imported
Mac native and browser coordinate gates remain mandatory on macOS.

The existing four-plane typography, note-rule, print, shared-source and older
Word-version gates remain enabled with their original tolerances and records.
The additional original blank-page and printer-canvas captures are retained as
raw observations and are not substituted for accepted coordinate expectations.
