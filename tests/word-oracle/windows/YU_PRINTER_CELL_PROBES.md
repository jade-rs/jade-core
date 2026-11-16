# Yu Mincho full-cell printer measurements

Eight independent A3 captures exercise regular Yu Mincho at 8.5, 11.5,
12.5, 14.5, 15.5, 17.5, 20 and 22pt. The last two are integer-size controls.
Reproduce the captured DOCX bytes with
`scripts/word-oracle/generate_windows_yu_wide_halfpoint_probes.py`.

The original normal-read-only captures came from Windows Word
16.0.19127.20936. Immutable ZIP archives retain source DOCX, EMF, PDF and
capture metadata, including product/file/application versions, build and
the Word executable's SHA-256. Their entries are registered under
`observations/word-16.0.19127.20936/`; assertions verify the original ZIP and
every source identity. Observations from another Word version get separate
records rather than replacing these captures.

The previous renderer accumulated horizontal displacement at 8.5, 12.5 and
14.5pt. The same measured 600dpi/1024-unit full-cell realization already used
for MS Mincho half-point sizes also reproduces these Yu Mincho captures.
The regular Yu Mincho aliases now use that policy for full cells and
ideographic spaces. The integer-size policy, half-width kana policy and
Mac callers are unchanged.

All eight cases and four previously archived Yu Mincho ideographic-gap
cases are included in the coordinate registry. The registry asserts 1,033
font cases, 159,779 glyph origins and 593 paragraph background cases against
original Word output. The edit/export/reopen corpus contains 1,429 inputs.
There are 1,008 immutable original captures / 1,919 documents / 1,872 source
byte groups. Original registry prefixes, Mac references, separate platform
expectations and coordinate tolerances remain unchanged.
