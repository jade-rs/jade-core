# Windows numeric spacing and compatibility controls

The 100 inputs in `numeric-spacing-observations.json` derive from the unchanged
Mac `aptos-numeric-ranges-mac/source.docx`. Their original Windows Word captures
are stored under Word **16.0.19127.20936**, with capture IDs, executable hashes,
unaltered source bytes, original PDFs and EMFs. Later Word versions must add
captures rather than replace these originals.

The controls cover inherited Japanese fonts, four font-hint profiles,
autoSpaceDN absent/on/off, 12/14/20/40pt, all four Aptos/Display styles, and
compatibility modes 14/15. Numeric-kerning controls also retain default,
tabular/proportional number spacing and explicit standard ligatures. The
production generator reproduces every DOCX byte from the Mac parent.

The prior automatic Office-font renderer failed 84 inputs. The corrected
renderer passes all 100 against the original EMF, unclipped/visible Word PDF
and painted native PDF, both with actual Office fonts and with original Word
PDF font programs in an isolated cache. Each axis retains the existing
0.375pt limit. Expectations are never translated to match the renderer.

Word's PDF subsets omit GPOS/GSUB. `aptos-short-shaping-metrics.json` identifies
the eight actual Aptos **2.01;O365** full font programs used to reproduce their
short ASCII design-unit advances. The source contains font units, without
document coordinates or full font files. Reproduction requires the recorded
local cache files and `uharfbuzz==0.57.0`:

```powershell
python scripts/word-oracle/generate_windows_aptos_short_shaping_metrics.py $env:LOCALAPPDATA output.rs
```

The fallback requires the known family, font version, weight/style and missing
GPOS. Other versions, unsupported features and longer runs retain the shaper.
Mac layout never selects this fallback. Actual full fonts keep their own
OpenType tables.

GitHub Actions requires the source/evidence tests and all 1,287 registered
Windows font comparisons. Both native OS jobs also run the native-shaping
library tests. Existing Mac Word goldens remain unchanged; the separate local
921-input Mac regression comparison checks glyphs, pages, screen pixels and
every painted PDF page against the frozen pre-change renderer. This local
comparison does not create new Word-for-Mac reference data.
