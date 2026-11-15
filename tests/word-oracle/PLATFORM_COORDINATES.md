Shared Mac and Windows coordinate assertions
===========================================

`platform-coordinates.json` links each unchanged Mac corpus `source.docx` to its
existing Mac manifest and one or more Windows Word captures. Source bytes must
match the Mac source hash and the archived Windows input exactly. Each Windows
capture retains the complete Word product version, executable hash, OS, UTC
capture ID, original PDF and raw EMFs. Add new captures instead of replacing an
older Word version's evidence.

Expected coordinates belong to the Word platform/version that produced them.
Mac and Windows coordinates may differ; neither platform's expectation is
substituted for the other's. Mac tolerances remain those of the original Mac
manifest. Windows uses the existing 0.375 pt limit on both axes without fitting.

GitHub Actions runs the original Mac corpus browser assertions on macOS and
checks that every registered shared case remains included. On Windows, the
native renderer opens that same Mac original and must pass all four comparisons:
raw Word drawing origins, unclipped Word PDF text, visible Word PDF text, and
the independently painted native PDF. Page count, text/order and glyph count
must also match. Missing evidence or a mismatched platform, Word version,
source hash or reference hash fails CI.

The shared cases cover floating horizontal table placement, disabled hyphenation,
contextual paragraph spacing in compatibility modes 14/15, and paragraph-border
wrapping with Windows Word 16.0.19127.20936. The floating-table Mac original
records Word 16.114.2; the older hyphenation Mac capture does not record a Word
version. Its original PDF/coordinate hashes remain pinned; no version is inferred.
The full existing Mac corpus
and previous Windows version gates remain enabled. New cases should be added
only after all four Windows comparisons pass; failing captures remain evidence
for fixes, never new accepted coordinates.

The checker also requires every previously asserted shared case and its original
Windows version/capture ID. Adding newer Word versions cannot silently remove an
older version's assertion.

The separate `windows/font-coordinates.json` registry asserts 619 font/typography
documents (123,862 glyph origins) against Word 16.0.19127.20936. Its Windows CI
gate extracts font programs from the preserved original Word PDFs and passes
only those fonts and the original DOCX to the renderer. Expected coordinates
are never renderer inputs. This permits exact font testing on hosted runners
without installing Office fonts. The same four comparisons and 0.375 pt limit
apply. Coverage, source hashes, version metadata, original PDFs and EMFs are
checked before rendering. The public encrypted fixture uses its documented test
password and retains the encrypted source hash separately from decrypted bytes.

An additional [graphic-text gate](GRAPHIC_TEXT.md) asserts 62 versioned Windows
cases against original physical glyph copies, visible colors and absolute
outline geometry. It preserves the existing typography gate and Mac references.

The shared registry also pairs the unchanged `footnote-mac` and
`footnote-table-each-page-mac` and `endnote-mac` sources with their Windows observations. These
Windows checks use original PDF font programs so Office fonts are not required
on hosted runners. The note matrix retains and asserts all 24 Word captures. Independent script
and control sweeps cover four font families, four weights/slants and 49 sizes,
including 1,568 measured script runs. Century OpenType/kerning/tracking probes
cover modes 12/14/15 and default/explicit typography switches. Script metrics
are re-extracted from original Word output in CI; Mac policy is unchanged.

Nine valid endnote separator probes vary body grid pitch (inactive `default`,
18pt, 24pt) and separator size (inherited, 10.5pt, 20pt). Windows Word
16.0.19127.20936 measures the authored separator story rather than a fixed
12pt gap. The original endnote and all nine valid probes pass all four planes.
Twelve inactive-grid controls cover theme, Century, Arial and Yu Mincho at
10.5/12/20pt with plain, superscript and ordinary marker text. Their original
PDFs, raw EMFs and byte-identical inputs are retained under captures
`20261114T065723961Z-5f5c7857` and `20261114T071711373Z-e298d881`.
Mac measurement and original expectations remain unchanged. Regenerate with
`generate_windows_endnote_cases.py`; release hashes must match captured inputs.

Three earlier probes use the malformed grid type `none` instead of the schema's
inactive `default`. They remain unpromoted observations, with all original output
preserved in the first capture. They expose a 0.39–0.45pt discrepancy in how this
invalid value is handled; the strict 0.375pt gate is unchanged.

Split-footnote coverage adds ten documents varying the body length and
20/40/60-paragraph notes, plus the unchanged `footnote-long-continuation-mac`
original. Windows positions the fitting prefix at the body bottom, rather than
placing the overflowing full note directly below the body. The original output
is retained under `20261114T072318934Z-aeb89460`. Mac prefix placement and its
separate split-gap policy remain unchanged.

Textbox coverage adds 32 probes and the unchanged table/textbox/note original.
Windows uses requested-face font metrics, half-outline stroke clearance and a
four-fifths baseline for exact 14/28pt lines. Probes cover 0.5/1/2pt outlines,
0/3.6pt top insets, Arial/Century/Yu Mincho at 10.5/20pt, rectangle/rounded/triangle
presets, and exact lines. Mac uses the same pre-existing paths and goldens.

The Windows comparator matches each character/page to a distinct glyph origin
within 0.375pt on each axis. No translation or fitting is permitted. This avoids
an artificial reading-line grouping boundary when a floating textbox and a
body superscript differ by only 0.24pt. Missing, duplicate and substituted paint
still fail, including ambiguous nearby repetitions; a 0.376pt move still fails.

Ten three-line controls additionally assert successive baselines. Windows
implicit single spacing uses the requested font box, not a font-size estimate.
Original output is preserved in `20261114T080924812Z-f0e42358`; all fonts and
14/28pt exact-line controls retain the unchanged per-axis limit.

Windows endnote separator paint has an additional independent vector gate.
The original endnote, nine valid grid/size probes and twelve Arial/Century/
Yu Mincho/MS Mincho probes assert every black rule rectangle's page and four
edges against both the original Word PDF and raw EMF `PATCOPY` destination.
Missing or additional paint fails; the absolute edge limit stays 0.375pt.
The renderer receives only DOCX bytes and original embedded font programs.
No expected coordinates are supplied to it. The vector gate reuses the
separately painted PDF from the font-coordinate gate.

Capture `20261114T091931923Z-52cf7efc` retains all twelve Word
16.0.19127.20936 exports, input bytes, raw EMFs and font-table provenance
(face/version/file hash, units, line and strikeout metrics). Generate the
byte-identical inputs with `generate_windows_note_rule_cases.py`.
The rule uses the requested face's line ascent and OS/2 strikeout metrics;
its width is two inches. Mac paint and existing Mac references are unchanged.

The [Windows table identity and row controls](windows/TABLE_ROW_PROBES.md) retain
88 additional accepted originals plus two unchanged recaptures. The platform
expectations stay independent; Mac reference files and tolerances are unchanged.

Windows column overflow and preserved background observations
-----------------------------------------------------------

The original `2col`, `align-center`, `char-spacing` and `character-styles`
fixtures add four strict coordinate/no-vector-ink assertions using their
unchanged Word 16.0.19127.20936 captures. In the two-column vector PDF, painting
paper after restoring a line clip produced white rectangles absent from Word.
Windows vector export now relies on its existing line clip; interactive canvas
and all Mac paths retain their prior paint behavior. The gate still rejects
white fills after text or images, including fills that erase a background.

Twenty-five fully matching paragraph-background controls were initially added
to the release corpus and font-coordinate gate. These include the
four original green/yellow one/two-column controls, centered and left-aligned
Japanese/Latin mixtures, balancing enabled/disabled, and font/grid/default/mark
variations. Generate all 45 measured controls with
`generate_windows_paragraph_shading_probes.py`; every generated DOCX is byte
identical to its original Word capture input.

Each accepted background is checked against the original Word PDF: exact page
and rectangle counts, opaque source color, four absolute edges within 0.375pt,
and continuity within 0.01pt at touching boundaries. Additional vectors,
translucent edge pieces, white erasure and missing paint fail. Negative tests
exercise these failures; tolerances and all existing Mac references stay fixed.
The renderer receives only source DOCX bytes and embedded font programs.

Windows solid vector shading uses the authored rectangle as one opaque path.
Body shading includes Word's horizontal padding and first-line grid leading.
Japanese Arial text selects MS Mincho for its missing Japanese glyphs, while
neutral note rules retain their authored Arial line box. Word/width repair
removes ASCII pair kerning across moved line boundaries, and a soft line end
has no Latin-to-Japanese auto space for text on the next line. The Mac geometry,
paint and reference paths remain unchanged.

Sixty-two additional original Word 16.0.19127.20936 captures preserve paragraph
borders, adjacent shading, tables, spacing, font and justification variants.
The observation registry holds 609 captures / 1,520 documents / 1,481
source byte groups, retaining earlier Word builds and exact executable hashes.
Unmatched cases remain raw observations, including mixed-script justification,
bordered/table shading and adjacent-paragraph continuity. They are not accepted
coordinate expectations.

An additional 36 Word 16.0.19127.20936 captures isolate mixed-script
justification: 12 kerning-threshold controls (0, 20 and 198 half-points), and
24 short manual-break controls varying Japanese/Latin boundaries and spaces.
Both groups compare left and justified alignment with balanced widths enabled
and disabled. The original input DOCX, PDF, EMF and capture metadata remain in
versioned observation archives, including executable identity and read-only
capture provenance. `generate_windows_justification_probes.py` reproduces all
36 source files byte for byte; CI asserts their identity and archive hashes.
All original captures remain preserved. Residual mismatches require renderer
fixes before promotion into the coordinate gate; Mac expectations stay fixed.

Twelve further captures change only the manual-break controls' font size to
12, 18 or 22pt, preserving alignment, balanced-width settings and all other
package content. The size generator and CI assert exact original source bytes.
Of these 48 justification/size captures, 37 now pass the strict coordinate and
paint gates and 11 remain diagnostic observations. Every raw archive retains
Word build 16.0.19127.20936 and executable identity.

Fifty-one additional Word-matching controls initially brought the release corpus to
906 DOCX files and the versioned font-coordinate gate to 507 cases / 130,832
origins. These 76 accepted shaded controls assert opaque backgrounds, absolute
edges, rectangle counts and internal continuity. The Windows mixed-script
allocator expands bounded word spaces, then script boundaries, then eligible
residual gaps. Capacities use intrinsic font advances and authored typography;
reference positions are never renderer inputs. Joined wrap boundaries restore
known ASCII punctuation kerning, including Arial y-period, without changing
unrelated leading advances. All previous 456 cases, 855 release records and Mac
reference data remain intact. Seventeen of the 93 shading/justification/size
controls still differ from Word and remain unpromoted, alongside other raw
border/table observations.

Eighteen more original Word captures vary available line widths (186, 188,
190, 192, 194, 198, 202 and 210pt) with balanced widths enabled/disabled, plus
two left-aligned controls. Their source generator reproduces every archived
DOCX byte for byte. Sixteen pass every coordinate plane and background gate,
bringing the current release corpus to 922 documents and the font gate to
523 cases / 131,024 origins, including 92 shaded controls. Two remain raw
observations: a narrow balanced line changes word wrapping and a 202pt line
has an X-coordinate mismatch. Across all 111 recent controls, 19 remain
unpromoted. All previous reference records and Mac expectations are retained.

The edit/export gate also checks all 1,015 release documents. Reusing an authored
soft-break run during serialization preserves the original break syntax and
run properties across edits; the reader's editable run segmentation and
rendering are unchanged. Regression tests cover repeated synchronization,
tail edits and break deletion, as well as the corpus's original package data.

Four additional original Word 16.0.19127.20936 captures bracket the balanced
manual-line threshold at 186.5, 187, 187.5 and 188.5pt. Their generator changes
only the right indent of the original 188pt control and reproduces each archived
DOCX byte for byte. Word retains the final word on the first line at these widths;
the 186pt control still wraps. Windows mixed-script justification now contracts
ordinary word spaces before existing script auto spacing, within their respective
capacities, and preserves the authored hard-break range when rejoining a word.
Balanced CJK-adjacent spaces remain fixed. This policy is Windows-only and uses
intrinsic layout measurements, never expected Word positions.

These four controls and the previously unmatched balanced 188pt control pass all
four coordinate planes and the production background gate. The release corpus
now contains 927 documents; the font gate covers 528 cases / 131,084 origins,
including 97 shaded controls. All previous 523/922 records remain unchanged.
The 202pt unbalanced mismatch remains a raw observation; across the 115 recent
controls, 18 remain unpromoted. Mac's reference data and tolerances are unchanged.

Twelve further original captures independently exercise the manual-line threshold
at 12, 18 and 22pt. Their generator changes only the run size and right indent of
the same original parent; all other package parts are byte-identical. All twelve
pass every coordinate plane and the production background gate, bringing the
release corpus to 939 documents and the font gate to 540 cases / 131,228 origins,
including 109 shaded controls. All prior 528/927 records remain unchanged.
Mac rendering also remains identical for these twelve controls. Eighteen of the
127 recent controls remain raw observations with their original Word evidence.

Twelve paired left/justified controls at 12, 18 and 22pt preserve the same text
and balance settings as the original 20pt/202pt controls. They vary only run size
and, for justified lines, the right indent. Their sources reproduce the captured
DOCX bytes exactly, and all twelve pass four coordinate planes and background
paint checks against Word 16.0.19127.20936. They bring the current font gate to
552 cases / 131,372 origins, including 121 shaded controls, and the release
corpus to 951 documents. All prior 540/939 records remain unchanged. These
controls constrain future changes to the remaining 20pt mismatch; they do not
resolve it. Across the 139 recent controls, eighteen remain unpromoted.

Thirty-six original Word 16.0.19127.20936 captures separate Latin-space fonts
from explicitly selected Japanese fonts, vary terminal Japanese/Latin text and
spaces, and exercise Calibri auto spacing at 12/20/24pt with balancing on/off.
The generator reproduces every input byte for byte and preserves all package
parts outside the explicitly changed document properties and text.

Windows justification now counts an ordinary space in a known Latin face once;
a CJK fallback glyph before it does not add another ideographic opportunity.
Balanced half-em spaces and spaces in East Asian faces retain their policy.
Calibri's regular-face auto spacing now uses its intrinsic OS/2 average width
(1067/2048 em), with quarter-em spacing when balanced. A test reconstructs these
metrics from the exact original Word PDF font program, independently of expected
glyph coordinates. Both changes are Windows-only.

Thirty-three controls pass all four coordinate planes and the production
background gate. The current totals are 585 typography cases / 132,431 origins,
154 shaded controls and 984 release documents. All previous 552/951 records and
Mac expectations remain unchanged. The two justified controls ending in a space
and the longer justified Calibri control remain raw observations. Across the
175 recent controls, 21 remain unpromoted, alongside older table/border cases.

Thirty-six further original Word 16.0.19127.20936 controls isolate balanced
Arial spacing with an explicit MS Mincho East Asian face, and compare identical
Latin/East Asian font slots with and without an East Asia hint for Arial and
Calibri at 12/20/24pt. All original captures are retained, including failures.
The generator reproduces all 36 original DOCX inputs byte for byte, changing
only the stated font slots/hint and preserving every other package part.
An independent test checks the original Word PDF origins: all twelve hinted/
unhinted pairs are identical, while separate versus identical script font slots
change Arial's balanced origins. These observations constrain the font-slot
policy; an East Asia hint alone must not select a different spacing rule.

Three previously published endnote separator inputs (grid disabled, separator
font sizes omitted/10.5/20pt) now also have required glyph-origin and note-rule
assertions against their existing original Word 16.0.19127.20936 capture.
All three pass EMF, visible/unclipped Word PDF and independently painted native
PDF origins, plus original EMF/PDF and native PDF rule geometry. This closes a
coverage gap without changing the source documents or expected coordinates.
The font gate covers 588 cases / 132,467 origins and the note-rule gate 61 cases;
the release corpus remains 984 documents and all prior 585/58 registry records
are preserved.

Thirty-six additional cross-font controls vary only the East Asian font slot of
the original Arial/MS Mincho and Calibri/MS Mincho inputs. They compare same-face
unbalanced controls and both Arial/Calibri cross-face directions at 12/20/24pt,
left/justified alignment and balanced/unbalanced widths. All Word originals,
including unmatched geometry, are retained by version in the raw registry.

Windows balanced Arial spacing now distinguishes the requested script font
slots before the Japanese Arial slot is replaced by its MS Mincho fallback.
With an explicitly separate Japanese face, its Latin auto gap is a quarter em
and the adjacent half-em space cell must not be shortened again by ordinary
space-glyph pair kerning, including after wrap repair. Same-face Arial controls
retain their previous average-width gap and pair kerning, regardless of the
East Asia hint. Mac spacing and font selection are unchanged.

Thirty-one newly passing controls (10 separate-face Arial, 8 same-face/hint
controls and 13 cross-face controls) are now required coordinate/background
expectations. Together with the three existing endnote inputs newly covered
above, the font gate contains 619 cases / 132,839 origins, including 185 shaded
controls; the note-rule gate covers 61 cases and the release corpus 1,015 inputs.
All previous 588/984 records and every original Mac expectation remain intact.
The 72 new input DOCX files reproduce the original captures byte for byte.
Across 247 recent controls, 185 pass and 62 remain unpromoted original
observations, alongside older table/border cases. Requested Calibri East Asian
fallback line height and additional justified lines remain under investigation;
no failing observation is used as an accepted expectation.
