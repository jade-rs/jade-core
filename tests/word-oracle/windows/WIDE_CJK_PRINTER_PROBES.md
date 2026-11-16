# Windows Japanese printer-cell coordinates

The 65 additional originals were exported by Windows Word **16.0.19127.20936**,
using executable SHA-256
`7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312`.
Each versioned ZIP preserves the original capture metadata, source DOCX, Word PDF
and raw EMFs with individual hashes. Captures use ordinary read-only opening.
New Word versions and recaptures append evidence without replacing originals.

The A3 landscape controls include:

- 16 MS Mincho sizes from 8.5–24.5pt;
- 12 independent MS Mincho/MS Gothic/Yu Mincho controls at 7.5/19.5/25.5/28.5pt;
- 15 independent controls in those three families at 31.5–43.5pt;
- 16 MS Mincho/Yu Mincho controls with half-width kana or long ideographic gaps;
- six regular/bold Japanese controls requesting Mincho, MS-Mincho or MS-Gothic.

Thirteen size controls and every independent/full-half-space control pass the
original EMF, unclipped Word PDF, visible Word PDF and independently painted
native PDF comparisons. The 8.5, 12.5 and 14.5pt MS Mincho size controls remain
unaccepted observations with horizontal discrepancies. Other previously failing
Yu Mincho ASCII and mixed-script controls also remain preserved for further fixes.
The existing 0.375pt per-axis limit and paragraph-background/continuity gates stay
unchanged. Expected coordinates always come from original Word output.

The Windows regular CJK correction distinguishes full cells, half-width kana and
ideographic spaces. Synthetic bold keeps its separate ink expansion and zero
expansion for spaces. The size sweeps reproduce half-device-pixel printer ems by
realizing the cell on the 600dpi grid and quantizing its requested advance on a
1024-unit grid. Independent 25.5pt controls reject a rounded 1000-unit model.
The renderer receives the DOCX and original font programs, without expected
positions. Mac uses its existing coordinate paths and original references.

Windows Word resolves Japanese in these three alias requests to MS Mincho. The
Windows resolver selects that measured face before computing its line metrics,
while retaining authored alternate-name priority. Mac retains its original names
and metrics; generic Latin metric lookup is unchanged.

There are now 983 typography cases / 156,181 origins / 543 shaded cases and 1,379
release inputs. The archive retains 952 captures / 1,863 documents / 1,816 source
byte groups. The 78 new typography assertions include 62 sources from these new
captures and 16 previously preserved sources. Byte-identical recaptures remain
archived aliases instead of duplicate typography assertions.

Reproduce the exact captured inputs with:

```sh
python scripts/word-oracle/generate_windows_wide_cjk_size_probes.py /tmp/cjk-sizes
python scripts/word-oracle/generate_windows_wide_cjk_size_probes.py /tmp/cjk-independent --group half-device
python scripts/word-oracle/generate_windows_wide_cjk_size_probes.py /tmp/cjk-large --group large-half-device
python scripts/word-oracle/generate_windows_cjk_cell_probes.py /tmp/cjk-cells
python scripts/word-oracle/generate_windows_ms_font_alias_probes.py /tmp/cjk-aliases
```

The Windows font tests regenerate all 65 inputs and require byte identity with
the archived originals, including portable ZIP creator metadata. GitHub Actions
asserts the complete Windows coordinate/background corpus and the existing Mac
native/browser coordinate gates. Word source/version/reference hashes are pinned.
