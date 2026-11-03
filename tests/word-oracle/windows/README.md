# Windows Word references by product version

Evidence is append-only, grouped by the complete WINWORD.EXE product version
and an independent capture ID:

```
word-16.0.19127.20860/20261101T130521628Z-902306ab/
```

`Application.Version` (`16.0`) and `Application.Build` (`16.0.19127`) are not
specific enough. `capture.json` records both and the full product version,
executable SHA-256, UTC acquisition time, Windows OS description, original
DOCX hashes and exported PDF hashes. Word opens source files read-only;
changed source/executable bytes invalidate a capture. Repeated captures use
new IDs, so neither a new build nor another run overwrites historical data.

Each directory preserves the original Word PDFs, independently extracted
`word-reference.json`, native `comparison.json`, and `verification.json`.
The latter identifies the engine commit and whether source changes were
present. Reports retain failures as well as successes.

`index.json` registers explicitly promoted `strict.json` sets. Promotion is
refused unless every glyph, page and origin matches Word within 0.375pt.
Tests compare every registered capture/version and the original legacy
capture. Mac evidence and its thresholds are independent and unchanged.
The portable CI check verifies the controlled probes' baselines using
bundled stand-ins. Full installed-font X/Y comparisons run on the Windows
Word machine; no Microsoft font binaries are redistributed to CI.

Collected product versions: **16.0.19127.20860**. No other installed Word
version is claimed. This version currently has eleven passing unique documents,
60 pages and 6,794 glyphs. The earlier numbering-label capture retains its
1.83pt failure; a later capture records the corrected result. Contract and
Yokohama references still have subpoint residuals and are not promoted.
The 2026-11-03 `numbering-restart` capture is also retained as pending:
both outputs contain 2 pages and 563 glyphs, but only 321 glyphs satisfy
the full X/Y threshold (X p95 2.824pt, Y p95 0.232pt).
Another capture that day preserves `yokohama_page1`: 1 page and 853 glyphs
in both outputs, with 282 passing full X/Y (X p95 7.272pt, Y p95 0.118pt).
These remaining horizontal differences are not hidden by origin alignment.

Parent-repository commands (run from jade-office):

```powershell
./scripts/check-word-windows.ps1 -Documents <DOCX paths> -PythonExecutable <python.exe>
python scripts/archive-word-windows-capture.py <capture.json> --strict <passing fixture names>
./scripts/test-windows-word.ps1
```

The Python tools require PyMuPDF. Archiving validates hashes, refuses an
existing destination, rejects mixed-version references, and never promotes
coordinate failures. A future Word build automatically gets its own version
directory; collecting one does not replace older builds' tests.

The later `20261103T061428054Z-5f5076bb` capture promotes
`numbering-restart`: all 563 glyphs on both pages pass the unchanged 0.375pt
X/Y threshold. The earlier failing capture remains intact. Windows fixes
remove synthetic bold widening from fixed-pitch Latin cells, include both
tracking intervals in full grid cells, and retain Japanese-face numeric gaps
under the default font hint. Mac metric selection is unchanged.
