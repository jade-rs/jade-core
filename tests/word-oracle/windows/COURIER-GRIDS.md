# Courier line metrics and fixed table grids

The 34 promoted controls retain original Windows Word 16.0.19127.20936
DOCX/PDF/EMF output, capture IDs and executable SHA-256. They cover Courier New
at 10.5/12/20/40pt, all four styles, compatibility modes 14/15, single/double
automatic line spacing and exact 24pt lines. Two six-page controls cover
adjacent narrow/wide grids in both orders, conflicting row widths with either
saved grid, different styles and a separating paragraph.

Courier New 6.95's original embedded font programs retain the 2048-unit em,
hhea 1705/-615/0 and OS/2 usWinAscent/usWinDescent 1705/615. Windows therefore
uses the same 2320-unit single-line height as the existing Mac policy, instead
of the generic 1.3em fallback. This changes the 10.5pt pitch from 13.65pt to
11.895pt. The Mac metrics remain exact.

Both Word platforms share consecutive matching fixed-table grids. Windows
also fits conflicting multi-row cell preferences within the absolute preferred
table width. Its existing single-row/column rules remain separate. The original
Mac calculations are preserved.

`generate_windows_courier_grid_probes.py` reproduces all 34 inputs from the
unchanged Mac `adjacent-fixed-table-grids/source.docx`. That original has an
unbound `w14` token in `styles.xml`'s `mc:Ignorable`; Windows Word rejects it in
normal read-only opening. Its failed attempts remain versioned refusal evidence,
with no drawing expectations invented. The Windows controls bind that unused
prefix in their own styles part. They preserve all style values and document
content; the mode-15 control additionally changes compatibility mode. The Mac
source, manifest and coordinates are unchanged. Its missing historical Word
version remains missing provenance.

`font-coordinates.json` asserts every promoted case with the existing 0.375pt
per-axis gate against original EMF, unclipped/visible Word PDF origins and the
independently painted native PDF. CI requires the input/provenance tests and
the drawing comparison. Original embedded-font replay is independent of local
Office fonts.

`wrap-and-courier-observations.json` also retains unpromoted Aptos controls
whose remaining differences are still being investigated. Passing raw capture
integrity does not mean that their drawing coordinates pass.
