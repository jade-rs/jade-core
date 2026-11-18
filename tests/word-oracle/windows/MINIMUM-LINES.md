# Minimum and automatic line spacing observations

`minimum-line-observations.json` retains 108 real Windows Word captures derived
from the unchanged `exact-and-minimum-lines-mac/source.docx` input. The parent
reference was produced by Word for Mac 16.114.3. Its source, manifest and drawing
coordinates remain unchanged.

Each control pairs a two-line paragraph with explicit minimum spacing and a
two-line automatic-spacing control. The matrix covers Yu Mincho and MS Mincho
with Latin/CJK text, Arial and Courier New with Latin text, uniform 12/20pt or
mixed 20/10pt runs, 12/20/28pt minima, and compatibility modes 14/15.

The original DOCX, PDF, EMF and capture manifest are retained under the exact
Word product version, with an immutable raw ZIP registered in the observations
index. The manifest records Word executable identity and host information;
embedded font facts retain font versions and metrics. These captures use
Windows Word 16.0.19127.20936, Arial 7.07, Courier New 6.95, MS Mincho 5.32 and
Yu Mincho 1.92. No result is assumed to apply to another Word/font version.

The required source tests reproduce every input byte, preserve all package
parts except `document.xml` and `settings.xml`, protect the original Mac
reference, and independently compare Word's screen and PDF origins.

`check_windows_minimum_lines.py` asserts all 108 controls and 6,264 original
glyph origins. It compares native screen/logical coordinates and independently
painted PDF coordinates against original Word EMF/PDF data, checks that PDF
export preserves screen pixels, requires the original embedded font programs,
and rejects extra guide/border paint. Mutation tests reject moved, missing,
reordered and repaginated glyphs. GitHub Actions requires this gate on Windows.

Windows minimum-spaced lines now use each physical line's known font box and
place additional minimum leading above that box. For the original Yu Mincho
12pt body with a 12pt minimum, Windows Word's first baselines are 86.28/106.34pt;
the corrected native baselines are 86.26/106.33pt. Mac keeps its original
calculation and reference data. The original `exact-and-minimum-lines-mac`
input is also registered with independent Mac/Windows expected coordinates.

The native renderer receives source DOCX bytes and fonts, never expected
coordinates. The original captures and font/version facts remain retained
after promotion to a required gate. Observations outside this validated scope
retain their pending status.
