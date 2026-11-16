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
ideographic gaps, and 8.5/11.5/12.5/14.5/15.5/17.5pt text. Eleven pass every existing
coordinate and background assertion and join the regression corpus. The remaining
thirteen are archived observations, not passing expectations.

The original Word PDF and EMF themselves disagree at the final half-width-kana
origin by approximately 0.75pt at 14.5pt and 1.09pt at 15.5pt. Their PDF font sizes
are 14.52pt and 15.48pt. These sources require further screen/print investigation;
passing one original output cannot qualify as a four-plane coordinate match.
A wider application of the 1024-unit printer model does not explain every control.
The independent full-cell and ideographic-gap originals instead support extending
that model only for regular MS Mincho half-point full cells and ideographic
spaces. The unchanged integer-size originals, including 22pt, reject a blanket
application to every fractional device em. Half
cells outside half-device ems and the other font families retain their prior
policy; their disagreements cannot justify silently changing these expectations.
No expected coordinate is averaged, translated, or supplied to the renderer.

The corpus now contains **1,016 typography cases / 158,602 glyph origins / 576
shaded cases**, and **1,412 release inputs**. Original observations retain **995
captures / 1,906 documents / 1,859 source-byte groups**. Earlier 1,010 typography,
1,406 release and all 995 observation entries remain unchanged prefixes. Six
additional assertions cover three originally archived full-cell size controls and
three independently captured ideographic-gap controls.

Reproduce every added source, including unresolved observations, with:

```sh
python scripts/word-oracle/generate_windows_nonhalf_cell_probes.py /tmp/nonhalf
python scripts/word-oracle/generate_windows_printer_halfwrap_probes.py /tmp/halfwrap
```

Tests require byte identity against all thirty archived sources. CI asserts all
accepted cases with the unchanged 0.375pt per-axis limit and background/continuity
gates. Mac originals, expected coordinates, tolerances and the independent macOS
native/browser assertions are retained. The existing Windows paragraph policy
guards the new width correction; Mac follows its original path.
All 632 Mac-mode comparisons preserve exact glyphs, page counts, screen PNGs,
and every PDF page's pixels and drawing commands. Actual macOS native/browser
CI is separately required before merge.
