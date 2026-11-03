# Windows Word coordinate references

References are separated by complete Word product version and immutable capture ID.
The captured application is **16.0.19127.20860**; no other Word build has been
measured. New builds create additional directories and keep older versions' tests.

The regression index covers **17 unique documents, 133 pages and 18,099 visible
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
