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
reference, and independently compare Word's screen and PDF origins. Pending
coordinate observations remain identified as pending until native rendering
passes the coordinate gate; the native renderer never supplies expected data.
