# Word for Mac 16.114 pagination references, 2026-11-07

`word-mac-grid-tall-face.docx` is a document exported from a Word-compatible
editor: the Japanese Copyright Act (e-Gov statute text) with test strings such as
`asd` added (its document id and generated bookmark names replaced), set in bold 12pt `Arial Unicode MS` and `Arial` on an 18pt `lines`
grid (`w:docGrid w:type="lines" w:linePitch="360"`), compatibility mode 15, ragged
right. Word gives every Arial Unicode MS line two grid steps, so a page holds
about 18 rows.

Capture environment: Microsoft Word for Mac 16.114.3 (build 16.114.26102720),
macOS 27.0.1 (26A434), Japanese UI, Save As > PDF through AppleScript, with
`/Library/Fonts/Arial Unicode.ttf` and the fonts shipped with Word. The PDF was
exported on 2026-11-07, SHA-256 in each reference's `pdf_sha256`.

| File | What it holds |
| --- | --- |
| `word-mac-grid-tall-face.docx` | The whole document. It stays here rather than in `public/fixtures` because it is an export as it was, including element orders the schema gate rejects, and 265 pages for its edit sweep. |
| `word-mac-grid-tall-face.pages.json` | `generators/compact_page_reference.py` over the glyph reference: page count and every row's baseline and text (265 pages, 4692 rows). The whole glyph reference is 22MB. |
| `word-mac-probe-*.json` | Glyph references (`scripts/word-pdf-glyph-reference.py` in jlsi/jade) of the probes in `public/fixtures`, written by `generators/make_grid_tall_face_probes.py`. |

To check them again, export each DOCX to PDF from Word for Mac and run
`scripts/word-pdf-glyph-reference.py <pdf> <docx> <out.json> --word-version … --word-build …`.

## What Word shows

| Probe | Rule |
| --- | --- |
| `grid-line-unit` | One `w:beforeLines`/`w:afterLines` unit is the line grid's pitch while `lines` or `linesAndChars` is active: `afterLines="50"` spaces by 9pt on 18pt and 7.5pt on 15pt, with `w:snapToGrid w:val="0"` too. An earlier capture of the same rule measured `snapToChars` at 9pt and 24pt `lines` at 12pt. `default`, an omitted type and no grid use the nominal 12pt: 6pt. |
| `grid-line-unit-table` | A cell's line unit is the section's pitch even though the cell's own lines ignore the grid, with or without `w:adjustLineHeightInTable`. |
| `latin-east-asia-face` | Kanji whose `w:eastAsia` names a face without CJK glyphs (Arial, Times New Roman, Calibri, Century, Courier New, Verdana, Helvetica, Noto Sans) are set in MS Mincho, with its line box. An earlier capture of the same rule found no difference for `ja-JP`, `en-US` or no language. Bold MS Mincho and bold Arial Unicode MS advance 1/50 em more than regular. |
| `kinsoku-widow` | In a ragged paragraph, a kinsoku repair that overfills a line wraps it again, on the last line too: `…場合に` / `は、…あつ` / `ては、…`, `…ものとす` / `る。`. A paragraph that would leave one line after its last page break, however many pages it spans, moves the line before it along, and a line whose padding alone crosses the page foot counts as fitting. |

## Word behaviours seen here that jade does not reproduce

- A document with no paragraph style named `Normal` does not apply widow control
  in Word for Mac: an earlier version of `kinsoku-widow` without one left a last
  line alone. jade follows the specification's default there.
- A `snapToChars` section spreads two-character lines across the measure in Word.
