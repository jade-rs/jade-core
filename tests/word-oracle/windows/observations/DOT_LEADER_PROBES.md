# Windows dot leader observations

Word 16.0.19127.20936 measurements retain the original DOCX, PDF, raw EMF,
capture identity, and Word executable identity. Archive coverage is 243
captures / 934 documents. Existing captures and Mac references are unchanged.

- `20261114T125126718Z-e795e50f`: initial 15 inputs whose filenames incorrectly
  said `compat15`. The template has no settings.xml. A separate read-only Word
  COM open reports CompatibilityMode 12. Preserve these originally opened
  filenames and their exact bytes under observations/inputs; they are not
  registered as compatibility-mode-15 expectations.
- `20261114T125444122Z-6388e4a3`: the same 15 byte-identical inputs with correct
  `compat12` names, reproduced by `generate_windows_dot_leader_probes.py`.
  Arial, Century, MS Mincho and Yu Mincho run at 10.5/12/20pt; additional Yu
  Mincho probes vary the paragraph indent.
- `20261114T130556208Z-6d6ad7b2`: the unchanged original tabs.docx, measured
  separately without modifying its contents. The older multi-document capture
  is also retained.

`20261114T131426303Z-d86a8f8f` adds 13 endpoint probes at tab positions
7988–8012 twips in two-twip steps. A complete grid advance must fit before
the tab target: retaining the legacy 0.5px allowance would add one dot in
four of these cases. Mac retains its original allowance.

The original font programs' period hmtx facts are recorded separately in
dot-leader-font-facts.json. Windows first realizes the em on the 600dpi drawing
grid, then rounds the period advance on that grid, and places dots at grid
multiples. Rounding the requested-size advance only once differs for 20pt MS
Mincho and Yu Mincho. The renderer derives this spacing from font facts and
requested size; expected origins never enter layout.

All 29 registered inputs compare original EMF, unclipped PDF, visible PDF and
independently painted native PDF origins at the existing absolute 0.375pt X/Y
tolerance. Total font coverage is 218 cases / 98,822 glyph origins. Unknown
faces and styles retain their existing path; Mac dot spacing is unchanged.
