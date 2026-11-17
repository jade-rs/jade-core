# Fixed-table preferred widths and output contexts

The 54 observations retain normal read-only Windows Word 16.0.19127.20936
captures, original input/PDF/EMF bytes, capture IDs and the Word executable
identity. Original embedded font programs retain Arial 7.07 and Yu Mincho 1.92
version/metric/hash provenance; full installed font files are not distributed.

`generate_windows_column_table_probes.py` reproduces every input from the
unchanged Mac `column-table-fixed-preferred-width-mac/source.docx`. Controls
combine 80/100/120pt table preferences against a saved 100pt cell/grid width,
left/center/right placement, one/equal-two/unequal-two column frames, and Word
compatibility modes 14/15. Text, cell preferences, repeated header rows and
all package parts except document/settings remain unchanged. The original Mac
manifest has no historical Word version; that missing provenance stays missing.

Six original Word exports differ between screen EMF and printable PDF:
mode 14, unequal column widths, center/right table alignment. Screen and PDF
expectations must remain independent. The source test asserts this contrast
from the original Word files and rejects substituting a single coordinate set.

Raw capture integrity and source/provenance tests are required in Windows CI.
Raw observation success does not declare Jade coordinates correct. Drawing
promotion also needs the corresponding screen, PDF and nonempty table-rule
comparisons, with unchanged per-axis tolerances and Mac originals.
