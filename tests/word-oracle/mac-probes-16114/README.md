# Word for Mac 16.114 probe references

Each `word-mac-probe-*.json` is the glyph reference
(`scripts/word-pdf-glyph-reference.py` in jlsi/jade) of the PDF that Word for
Mac exported from `public/fixtures/<same name>.docx`. The probes isolate one
rule each, so a layout change that breaks the rule fails a small document
instead of shifting a long fixture.

Capture environment: Microsoft Word for Mac 16.114 (build 16.114.26101123),
macOS 27.0.1, Japanese UI, Save As > PDF through AppleScript, the fonts
shipped with Word (`/Applications/Microsoft Word.app/Contents/Resources/DFonts`).
The 118-document corpus references (`mac-glyph-corpus-20261010`, Word 16.113)
are unchanged; these probes are additional observations.

| Fixture | What Word shows |
| --- | --- |
| `pairs-kerned` | With `w:kern`, every adjacent closing/opening punctuation pair (`。）` `）。` `）、` `、「` `」（` `（「` `。「` `）」`) is 5.25pt apart at 10.5pt; without it 10.5pt. `characterSpacingControl` does not matter (`pairs-nocompress` in the generator output is identical). |
| `squeeze-k5-b2`, `squeeze-k40-b3` | A justified `compressPunctuation` line keeps an overflowing character while the reduction per comma is at most 32 times the gap each character would gain if the line ended before it. The right indent steps by one twip around that boundary. |
| `squeeze-plain` | Without punctuation nothing is narrowed: the character goes to the next line at any overflow. |
| `squeeze-comma` | A comma that may not start a line always stays: its blank half hangs while that covers the overflow, otherwise the earlier commas take all of it, then the half hangs and they take the rest. |
| `squeeze-conditions-compress`, `squeeze-conditions-nocompress` | Squeezing happens only in `both`/`distribute` paragraphs of `compressPunctuation` documents, kerned or not. A distributed line holding one character is centred. `w:pPrDefault` turns widow control off. |
| `indent` | Paragraph indents over a numbering level `w:left="533" w:hanging="113"`: a character-unit left indent keeps the first line there and hangs the rest past it; a paragraph first-line indent cancels the level's hanging. |
| `numbering-label`, `numbering-label-faces` | The label takes the paragraph mark's faces and size; the space after it is 0.278 em of the label size whatever the faces. |
| `distribute-single` | A lone `A`, `も` or `あ` (before a trailing space) on a distributed line is centred. |
| `exact-nogrid`, `exact-grid` | An exact-height line (12, 17.5, 24, 36pt) puts its baseline 0.8 of the height below its top, whatever the face (MS Mincho, MS Gothic, Century), size (10.5–20pt) or line grid. |
| `ruby-nogrid`, `ruby-grid`, `ruby-wide-nogrid`, `ruby-wide-grid` | A `w:ruby` is one box as wide as the wider of base and guide. The narrower one spreads over it by `w:rubyAlign`: center splits the slack around it, left and right push it to one side, distributeLetter shares it between characters and distributeSpace around each character. The guide baseline is `w:hpsRaise` above the base in whole points (17 → 8pt, 21 → 10pt), and the line grows to hold the guide. |
| `eqruby-nogrid`, `eqruby-grid` | Ruby written as an `EQ \o\aX(\s\up N(guide),base)` field with an empty saved result lays out the same way; `\* jc0`–`jc4` select center, distributeLetter, distributeSpace, left and right over the `\aX` switch, and `\up N` raises N points. |

`generators/` holds the scripts that wrote the squeeze, indent, exact and ruby documents;
the numbering-label and distribute documents were derived from the indent one.
