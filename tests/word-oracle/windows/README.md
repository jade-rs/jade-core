# Windows Word coordinate references

References are separated by complete Word product version and immutable capture ID.
The captured application is **16.0.19127.20860**; no other Word build has been
measured. New builds create additional directories and keep older versions' tests.

The regression index covers **31 unique documents, 166 pages and 31,298 visible
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
unchanged; legacy Windows table modes retain their previous policy until
separately measured.

Every capture preserves original PDFs, optionally original page EMFs, raw
references, comparisons and verification metadata. Metadata binds source,
application executable, PDF and EMF hashes, full version, engine commit and
source cleanliness. Historical failures remain intact; only explicitly selected
fully passing fixtures enter `index.json`. Promotion refuses overwritten,
incomplete, mixed-version, changed or mismatching evidence.

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
