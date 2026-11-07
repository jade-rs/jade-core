# Windows Word coordinate references

References are separated by complete Word product version and immutable capture ID.
The captured application is **16.0.19127.20860**; no other Word build has been
measured. New builds create additional directories and keep older versions' tests.

The regression index covers **68 unique documents, 348 pages and 56,460 visible
non-whitespace characters**, combining original PDF origins and Word drawing
origins. Each registered set checks text, order, page count and both absolute
coordinate axes within **0.375pt**. No origin fitting or tolerance increase is used.
Mac references and their tolerances are independent and unchanged.
Legacy partial COM traces retain their existing fallback for omitted terminal
advances; complete EMF origins are checked independently without replay overrides.

## Drawing origins and PDF origins

`Page.EnhMetaFileBits` records Word's Unicode text origins and per-character
advances. The extractor maps the whole-page logical canvas to Word's exact
PageSetup dimensions; it does not fit positions to the native renderer. It checks
EMF bounds, drawing mode, transforms, paper dimensions and hashes, and rejects
unsupported text encodings rather than guessing coordinates. CI reconstructs
registered references from the original EMF files and rejects changed origins.

The drawing sets contain all **11,868 characters** of `yokohama_page1`,
`yokohama_page6`, `contract`, `numbering-restart` and
`word-windows-yu-gothic-lines`, plus both Times New Roman boundary sweeps. The first four-document set is
`20261103T081431275Z-eac136ae`; the corrected contract is separately registered in
`20261103T082355941Z-8b9726d7` (1,907/1,907). These were captured from clean engine
source. Capture `20261103T091751013Z-0867a36b` adds both 20-page, 480-character
Times New Roman probes: Regular/Bold/Italic/Bold Italic at 9, 10.5, 12, 20
and 40pt, with and without SBCS/DBCS balancing. All 960 origins pass. Original PDF exports and their comparisons remain alongside them:
PDF export rounding still creates subpoint differences in the Yokohama and
contract PDFs, so those PDF comparisons are not promoted as passing PDF origins.

## Regression gates

The installed-font test compares every registered origin, including auxiliary
text such as footers, on the Windows Word capture machine. The Windows Actions
job also checks full X/Y origins for both Yokohama documents (6,278 characters)
the Yu Gothic probe (2,160 characters), and both Times New Roman sweeps
(960 characters) using bundled font stand-ins. It
checks 21 numbering/grid boundary origins and 144 controlled baseline lines.
The Yu Gothic fixture covers Regular, Light, Medium and Bold at six sizes and
omits styles to catch accidental import of Mac's implicit justification.
No Microsoft font binaries are redistributed.

The first installed-Arial expansion added **10,896 full
X/Y origins in nine documents (25 pages)**. It covers first-line/hanging
indents with left/center/right/justified paragraphs, left/center/right/decimal
tabs, contextual and collapsed paragraph spacing, raised/lowered runs,
superscript/subscript, and character spacing/scaling. Four Arial faces
(Regular/Bold/Italic/Bold Italic) are swept from 6 to 30pt by half points.
These traces establish Windows-only trailing-space alignment, decimal-tab
anchors, contextual-after collapse, and face-specific hinted script moves
and line sizing. Mac's coordinate data and layout policy are unchanged.

The passing captures are `20261107T031617995Z-72ef5045` (six documents) and
`20261107T032041007Z-ab4e783b` (three additional Arial faces), under
`word-16.0.19127.20860/`. Initial failures remain in the other November 7
capture directories. Each capture keeps the full Word product version,
Word executable SHA-256, OS description, original DOCX/PDF/EMF hashes,
timestamp/capture ID and engine verification commit. A new Word build or
repeated measurement creates a separate directory rather than replacing data.
Recreate the DOCX packages deterministically with
`python scripts/word-oracle/generate_windows_edge_cases.py`; never resave
these originals in Word.

Capture `20261107T035930893Z-61ca197b` adds five more documents, eight pages
and 2,303 complete drawing origins. Decimal tabs cover numeric suffixes,
currency/percent text, grouping commas, separators, split source runs and
negative/positive character spacing. Tables cover left/center/right alignment,
top/center/bottom cell content, exact row heights, 0–3pt borders, 0/4pt vertical
padding, following paragraphs and page breaks. Every new PDF origin also
passes the existing absolute tolerance. The installed-Arial gate now checks
**13,199 full X/Y origins in 14 documents (33 pages)**. The original failures
remain in the other capture directories.

Generate the follow-up packages with
`python scripts/word-oracle/generate_windows_decimal_cases.py` and
`python scripts/word-oracle/generate_windows_table_cases.py`. Windows now
anchors decimal tabs on the first point or first nonnumeric character after
a digit (allowing grouping commas), and modern tables reserve their border
frame and the bottom border below exact rows. Mac's existing policies remain
unchanged. The later legacy-mode captures below establish the Windows body
table policies independently.

Capture `20261107T050204006Z-600bbfab` adds six documents, 12 pages and
3,086 complete glyph origins for document compatibility modes 12 and 14.
All drawing and PDF origins pass. It repeats the alignment/border sweeps and
adds omitted, explicit-zero, positive and negative table indentation with
asymmetric horizontal cell margins. Windows distinguishes omitted indentation
from an explicit zero for leading placement, ignores indentation for centered
and trailing placement, and reserves the bottom border after an exact last row.
Those initial horizontal adjustments were limited to ordinary contiguous inline
body tables with positive leading padding. The later captures below extend
coverage to bidi tables, zero padding and cell spacing. Mac and unmeasured
floating contexts retain their existing placement.

The mandatory installed-Arial gate now checks **16,285 full X/Y origins in
20 documents (45 pages)**. Metadata also records Word's file version, OS
architecture and each document's actual Word-reported compatibility mode.
The clean verification engine is `6bedd8c7`; earlier failed captures remain
separate. The failed indentation capture explicitly records an uncommitted
prototype and is diagnostic only, with its original native coordinates retained.

Capture `20261107T064640987Z-1b8aa884` adds **16 documents, 86 pages and 11,414
complete drawing origins**. All PDF origins also pass. These cover bidi visual
order in modes 12/14/15; zero horizontal padding and legacy indentation;
2/6pt cell spacing; 0/0.5/1/2/3pt borders; 0/4pt vertical padding; and exact
rows with top/center/bottom content continuing onto the next page. The Windows
rules reserve each independent cell perimeter, distinguish the opening gutter
from continued-row spacing, and keep the outer frame and following paragraphs
in the flow budget. Mac's layout rules and reference bytes are unchanged.

The mandatory installed-Arial gate now checks **27,699 full X/Y origins in
36 documents (131 pages)**. Every registered drawing origin is reconstructed
from its original EMF in CI. A separate required Windows step checks that both
Windows and Mac policies retain every origin through editing, DOCX export and
undo; the full native Mac job also runs that portable test. It uses a complete
layout snapshot rather than a lazy viewport's partial line cache.

The clean verification engine is `9b409e13`. Original failures remain in
`20261107T053527509Z-bfe577cf`, `20261107T063103416Z-f69ea7c6` and
`20261107T063337099Z-03342251`, with their original native coordinates and
executable hashes. All captures preserve the actual Word-reported compatibility
mode separately from the complete application build.

Capture `20261107T071207802Z-32d1ef1a` adds **15 documents, 84 pages and 10,662
complete origins** for auto and minimum-height rows. All PDF and EMF origins
pass. It repeats the border/padding/alignment sweeps and 20-page continuations
in modes 12/14/15. Windows keeps both spacing gaps and padding outside the
declared content minimum, reserves each cell's independent top/bottom rules,
and closes the table's trailing gutter before following body text. Mac's
existing rules and golden files remain unchanged.

The mandatory Arial gate now requires **51 documents / 215 pages / 38,361
origins**; raw EMF reconstruction checks **50,229 origins**. The portable
edit/export/undo gate checks exact, auto and minimum-height tables under both
policies. The clean verification engine is `90132ee5`, with its executable
hash and compiler version retained. Initial failures remain in
`20261107T065217527Z-03cce62f` and `20261107T070018898Z-c53a0f60` with the
original `9b409e13` native traces. All 2,970 initial alignment-probe Y positions
were independently checked against clean `6bedd8c7` and are identical, showing
that the growing-row failures predate the earlier exact-row changes.

Every capture preserves original PDFs, optionally original page EMFs, raw
references, comparisons and verification metadata. Metadata binds source,
application executable, PDF and EMF hashes, full version, engine commit and
source cleanliness. Historical failures remain intact; only explicitly selected
fully passing fixtures enter `index.json`. Promotion refuses overwritten,
incomplete, mixed-version, changed or mismatching evidence.

Capture `20261107T083736578Z-61ba1645` adds three mixed-border documents
(modes 12/14/15), nine pages and 3,108 complete origins. Each row mixes
3pt/nil, nil/3pt and 1pt/2pt top/bottom borders, top/centre/bottom alignment,
exact/auto/minimum heights and 2pt/6pt cell spacing. All original PDF and EMF
origins pass with the clean `00f7aeed` engine. Its complete compiler version
and executable SHA-256 are recorded in `native-engine.json`. The original
failing `90132ee5` traces remain in `20261107T071814687Z-db7c1984`.

The Arial gate requires 54 documents, 224 pages and 41,469 origins. The
registry retains 71 distinct documents, 357 pages and 59,568 origins, plus
the historical registrations of two subsequently recaptured documents.

The required Windows CI job also compares monochrome separated-table rule
geometry against the versioned original Word PDFs. It checks all 27 spacing
documents (165 pages), including thick rules and page continuations. It
compares the union of painted vector rectangles in both directions within
0.375pt on each axis, with no fitting, scaling or allowed failures. Missing
rules, extra paint, unsupported black geometry, changed source hashes,
missing mandatory cases and page differences fail the job. Gray editor guides
are excluded from this monochrome check. The pinned PyMuPDF extractor and
its displacement, occlusion, clipping and rejection tests run in that job.
Mac golden data and coordinate thresholds remain unchanged.

Capture `20261107T090143788Z-8bbdd321` records twelve inputs with clean engine
`2b8815ed`. Six passing hairline cases (0.125/0.25/0.375pt, with and without
cell spacing, modes 12/14/15) add nine pages and 1,584 registered origins.
The other six inputs distinguish omitted spacing from an explicit zero and
retain their mixed-border failures for further work; they are not promoted.
All twelve original DOCX inputs are retained under `inputs/` and hashed in
`inputs.json`, even when their working sources are temporary probes. Full Word
build, actual compatibility mode, PDFs, EMFs, initial native traces and compiler
provenance remain alongside them. The capture/archive tools are identified by
their separate repository commit. Original `00f7aeed` traces remain unchanged
in `20261107T084840499Z-068a65aa`.

The vector gate now includes all 15 existing collapsed-table cases and the six
new hairline cases: 48 documents and 204 pages. Windows paints uniform
collapsed horizontal rules inside their measured bands, completes the closing
verticals and corners, and keeps zero-margin legacy grids at their original
border position while preserving text origins. All original glyph origins are
unchanged by these paint corrections. Mac keeps its previous painting rules.
The registry has 77 distinct documents, 366 pages and 61,152 origins, plus the
two historical duplicate registrations. The mandatory Arial subset has 60
documents, 233 pages and 43,053 origins; raw EMFs reconstruct 54,921 origins.

Capture `20261107T100425881Z-90da1943` adds twelve nonuniform collapsed-grid
documents, twelve pages and 6,108 complete origins, all matching both original
Word PDFs and EMFs. These distinguish omitted spacing and `nil`/`auto`/`pct`
forms in modes 12/14/15 while mixing exact, automatic and minimum row heights.
The source is clean engine `4ef1ae56`, with executable/compiler and capture-tool
commits retained. Every original DOCX is copied and hashed with the capture.

Windows recomputes minimum heights from resolved shared-edge winners before
reserving the opening band. Exact rows align content below their widest top
rule; growing rows retain their shared closing band. The physical right cell
paints an equal shared vertical once, so unequal bottom rules meet without
overlap. Only the final fragment closes the table; intermediate rows retain
continuous verticals. Mac keeps its earlier height/content/painting policies.
Edit/export/undo checks now also exercise the minimum-height mixed grid and
the `nil` form under both platform policies.

The required vector gate has 60 documents and 216 pages. The registry retains
89 distinct documents, 378 pages and 67,260 origins (plus two historical
duplicate registrations); the Arial gate requires 72 documents, 245 pages and
49,161 origins. Raw EMFs reconstruct 61,029 origins. Diagnostic capture
`20261107T091326151Z-fd041683` preserves all eighteen original width-type probes,
their original native failures and source DOCX snapshots. Its independent Word
comparison shows `nil`/`auto`/`pct` agree with omission, whereas missing width
or type agrees with explicit DXA zero. Negative DXA differs from both. These
explicit-zero/missing cases are superseded by the capture below. Negative DXA
remains follow-up work; historical failures are retained unchanged.

Run the same vector check from jade-core after building `word_coordinates`:

```powershell
python -m pip install -r scripts/word-oracle/requirements-borders.txt
python scripts/word-oracle/compare_windows_borders.py --index tests/word-oracle/windows/index.json --native-executable <word_coordinates.exe> --output-directory <diagnostic-directory>
```

Run from jade-office:

```powershell
./scripts/check-word-windows.ps1 -Documents <DOCX paths> -PythonExecutable <python.exe> -CaptureMetafiles
python scripts/archive-word-windows-capture.py <capture.json> --reference-kind emf --strict <passing fixture names>
./scripts/test-windows-word.ps1
```

Omitting `-CaptureMetafiles` retains the PDF-only workflow; PDF extraction needs
PyMuPDF. EMF extraction uses Python's standard library. Old numbering-label,
numbering-restart and Yokohama failures remain in their original captures and
are superseded by later passing captures, never overwritten.


Capture `20261107T104832151Z-b2b90731` preserves nine additional documents from
this PC's Word **16.0.19127.20860**, including file version, application build,
Word executable hash, OS/architecture, per-document compatibility mode and UTC
time. Original DOCX/PDF/EMF bytes and checksums remain in its own directory;
older captures are never replaced. All 4,581 origins match both original Word
PDFs and independently decoded EMFs. Clean native source `a95ede06` and its
executable/compiler provenance are retained in `native-engine.json`.

Windows distinguishes an explicit DXA zero (including a missing DXA width or
omitted type) from omitted/nil/auto/pct spacing. It uses separated cell borders
with no physical gap and reserves the final outer frame after growing rows.
The reader preserves the raw width form without changing legacy Mac numeric
values. Removing spacing clears that metadata and the original direct row XML,
including widthless elements. Edit/export/undo also covers these forms under
both platform policies. Negative DXA still needs a separate correction: the
measured Word row heights match omission, but its interior column edges differ.

The registry now has 98 distinct documents, 387 pages and 71,841 origins (plus
two historical duplicate registrations). The required Arial gate covers 81
documents, 254 pages and at least 53,742 origins. Raw EMFs reconstruct 65,610
origins. The required vector border gate includes 69 documents and 225 pages.

The archiver resolves relative paths before copying long fixture filenames.
This avoids the Win32 relative-path length limit in a deep checkout while
retaining the complete version/capture directory structure and no-overwrite
checks. Capture-integrity tests cover this path as well as provenance, source
checksums, tampering and accumulation across synthetic Word builds.

Diagnostic capture `20261107T110106544Z-29ee12e9` additionally retains four
unpromoted 2/3/4/5-column sweeps, including omitted/nil/explicit-zero controls
and negative widths -20/-40/-80 twips. It records 1,124 original Word origins
and the unchanged native results. The equal two-column document matches;
three-, four- and five-column documents remain pending. These diagnostics are
excluded from passing-reference counts and all required coverage floors.

Capture `20261107T123422628Z-6990d43d` supersedes the signed-spacing failures
above. Eleven original Word inputs are byte-identical to their deterministic
public fixtures. Word **16.0.19127.20860** was measured again with clean native
source `f41d7e73`; executable/compiler and capture-tool commits are preserved.
Every new origin matches original Word PDF text and raw EMF text at 0.375pt.

For a fixed grid, Windows Word takes signed DXA spacing from the first visible
row (direct, then exception, then table). Its n+1 gutter budgets resize the
nominal columns while retaining the authored outside extent. A later row's
negative override does not resize that grid. Equal 2/3/4/5-column sweeps,
unequal columns, row scopes and compatibility modes 12/14/15 verify this rule.
Mac retains its prior arithmetic. All eleven Mac-policy glyph arrays, page
counts and decoded RGBA images match clean engine `a95ede06` exactly on this
Windows host; the provenance is explicit about that host. Existing real Mac
Word goldens are unchanged and remain mandatory in GitHub Actions.

An exact-height narrow cell exposes four text origins hidden by its clip:
Word's complete PDF/EMF records contain 305 glyphs, while its visible PDF
contains 301. The default diagnostic and painting keep their existing visible
projection. An explicit `--unclipped-tables` diagnostic also retains hidden
origins. This capture checks all 3,700 complete origins, all 3,696 visible
snapshot origins, and all 3,696 origins extracted from actual native vector
PDFs. No origin is shifted or fitted, and every plane uses the same 0.375pt
limit. PyMuPDF flags 135/199 and projection names are recorded explicitly.

Both visible and complete checks are required for promotion; copied Word PDFs
cannot stand in for native output. The strict visible reference is registered
beside the strict EMF reference. CI reconstructs it from the original Word PDF,
checks retained input DOCX hashes, compares both native snapshot projections,
and checks actual native PDF text along with the existing border unions.
Missing visible proof, glyphs, pages, moved origins and changed provenance
fail the checks. Historical visible-only references retain their original
interpretation and bytes.

The registry now has 109 distinct documents, 398 pages and 75,541 complete
origins (plus the two earlier duplicate registrations). Arial coverage is 92
documents / 265 pages / at least 57,442 origins. Raw EMFs reconstruct 69,310
origins. The required vector gate covers 80 documents / 236 pages; the eleven
new documents additionally require actual painted-text agreement. The capture
contains original and native PDFs, raw EMFs, both native JSON/PNG projections,
all four comparison reports and the original hashed input DOCX snapshots.

For complete and visible capture together:

```powershell
./scripts/check-word-windows.ps1 -Documents <DOCX paths> -PythonExecutable <python.exe> -NativeExecutable <word_coordinates.exe> -CaptureMetafiles -UnclippedTableGlyphs
```

Diagnostic capture `20261107T130008739Z-08f6b53a` retains two additional
AutoFit/bidi unequal-column probes from Word **16.0.19127.20860** before their
fix. All 514 original Word origins, original input DOCX/PDF/EMF files and
native snapshots/PDFs are preserved. These failed probes are unpromoted and
excluded from passing-reference coverage. Each capture records the complete
Word product/file version, application build, executable hash, Windows build,
UTC timestamp, clean native-source commit/compiler/executable hash and capture
tool commits. A later measurement is a new capture directory, never a rewrite
of this evidence.
