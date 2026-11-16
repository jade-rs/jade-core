# Independent Windows cell and wrap controls

Thirty additional controls preserve ordinary read-only captures from Windows Word
**16.0.19127.20936** (executable SHA-256
`7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312`).
Their original ZIPs retain capture metadata, exact DOCX input bytes, PDF and EMFs.
They append to the existing archive without changing any earlier capture.

Six MS Mincho half-width-kana controls use 16.5/25.5pt text and line widths
329.9/330/330.1pt or 509.9/510/510.1pt. Word fits 39 cells below the exact nominal
width and 40 cells at or above it, while painting with its realized printer pitch.
All six pass the existing four coordinate planes and background assertions.

Twenty-four independent controls combine MS Mincho/Yu Mincho, half-width kana or
ideographic gaps, and 8.5/11.5/12.5/14.5/15.5/17.5pt text. Eight pass every existing
coordinate and background assertion and join the regression corpus. The remaining
sixteen are archived observations, not passing expectations.

The original Word PDF and EMF themselves disagree at the final half-width-kana
origin by approximately 0.75pt at 14.5pt and 1.09pt at 15.5pt. Their PDF font sizes
are 14.52pt and 15.48pt. These sources require further screen/print investigation;
passing one original output cannot qualify as a four-plane coordinate match.
A wider application of the 1024-unit printer model does not explain every control.
No expected coordinate is averaged, translated, or supplied to the renderer.

The corpus now contains **1,010 typography cases / 158,341 glyph origins / 570
shaded cases**, and **1,406 release inputs**. Original observations retain **995
captures / 1,906 documents / 1,859 source-byte groups**. Earlier 996 typography,
1,392 release and 965 observation entries remain unchanged prefixes.

Reproduce every added source, including unresolved observations, with:

```sh
python scripts/word-oracle/generate_windows_nonhalf_cell_probes.py /tmp/nonhalf
python scripts/word-oracle/generate_windows_printer_halfwrap_probes.py /tmp/halfwrap
```

Tests require byte identity against all thirty archived sources. CI asserts all
accepted cases with the unchanged 0.375pt per-axis limit and background/continuity
gates. Mac originals, expected coordinates, rendering code, tolerances and the
independent macOS native/browser assertions are unchanged by this data addition.
