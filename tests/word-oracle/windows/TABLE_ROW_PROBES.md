# Windows table identity and row indentation originals

These 90 read-only Word observations contain 88 new source documents and two
unchanged recaptures of existing Normal Table controls, with and without an
effects copy. Both unchanged recaptures pass before and after the change. Their
older accepted records stay authoritative; recaptures remain original evidence.

All originals were captured on Windows with Word **16.0.19127.20936**. Every
capture records product/file version, application version/build, executable
SHA-256, OS, UTC identity, input hashes and PDF/physical-page EMF hashes. The
executable SHA-256 is
`7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312`.

| Capture | Observations | Purpose |
| --- | ---: | --- |
| `20261114T195153941Z-3fc093f1` | 43 | Style id/name/custom flag, first versus second row, width kinds, compatibility modes and unchanged recapture |
| `20261114T200241643Z-226501da` | 6 | First-row exception versus direct DXA zero/positive and automatic indentation |
| `20261114T200434720Z-56b2954e` | 6 | Missing style name, independently varied id and custom flag |
| `20261114T200901748Z-eb28e42c` | 16 | Center/right alignment in modes 12 and 15 with zero/positive parent and row indentation |
| `20261114T203937561Z-a3f5a222` | 19 | Primary identity/custom flag with a conflicting effects copy and unchanged recapture |

The input packages were never resaved by Word. Immutable observation ZIPs retain
original metadata bytes. The separate accepted copies use repository LF JSON;
their drawing references are extracted again from their original EMFs.
`generate_windows_table_row_probes.py` reproduces all 90 DOCX inputs byte for byte
from three hash-pinned public sources. Generator tests also check the scope and
order of row exceptions and reject paragraph attributes in run spacing.

## Measured differences

The previous renderer passes **52/90** complete comparisons. The correction
passes **90/90**, retaining all 52 matching controls. Each comparison asserts
unclipped EMF origins, unclipped Word PDF origins, visible Word PDF origins and
the renderer's painted PDF origins at the existing absolute **0.375pt X/Y**
limit. Expected origins are never supplied to the renderer; only original DOCX
bytes and font programs extracted from Word's original PDF are supplied.

* A built-in Normal Table declaration requires the normalized `Normal Table`
  name and a false/omitted `customStyle` flag on Windows. A `TableNormal` id by
  itself does not select the built-in zero indentation. The controls put the
  first `A` at 57.6pt for the built-in declaration and 69.6pt for the authored
  custom/renamed/nameless declaration with 240-twip indentation.
* An authored primary table style remains authoritative when the effects copy
  declares a built-in style with the same package-local id. Importing that
  fallback must not replace the primary custom/renamed/nameless declaration.
* Only the first row's `tblPrEx/tblInd` sets table indentation. It overrides
  both inherited style indentation and direct table indentation, including
  direct zero, positive and automatic declarations. The second-row controls
  retain the inherited table origin.
* First-row `nil`, `auto` and `pct` reset inherited/direct indentation without
  interpreting the stored number as a physical offset. They also select the
  nonindented border edge instead of the legacy text edge. These controls put
  the first `A` at 63pt regardless of parent indentation.
* Center/right alignment ignores numeric first-row indentation in the ordinary
  table controls for compatibility modes 12, 14 and 15. The mode-15 positive
  controls prevent a regression that the mode-14-only controls would miss.

The 88 new accepted cases extend the independent Windows font registry from
329 to 417; all previous records remain unchanged. Their original Word PDFs
contain no visible vector ink, so CI also rejects extra exported border/grid
paint. The versioned observation inventory retains all 90 documents, including
the unchanged recaptures.

## Mac isolation

Row indentation exceptions are consumed only by the Windows geometry policy.
Mac retains its prior style identity classification and direct indentation
behavior. Mac reference files and tolerances are unchanged. In addition to the
required native/browser Mac CI jobs, a local comparison covers 361 old and new
controls against the prior renderer, asserting exact glyph arrays, page counts
and first-page PNG bytes. Existing native table editing and PDF export tests
remain required; diagnostic/external ignores are unchanged.
