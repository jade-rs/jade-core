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
