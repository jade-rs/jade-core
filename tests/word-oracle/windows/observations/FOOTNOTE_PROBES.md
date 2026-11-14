# Versioned Windows footnote observations

These 43 additional documents retain measurements from Windows Word
16.0.19127.20936. They are observations, not newly accepted Jade coordinates.
`check_windows_observations.py` checks the original input, PDF, EMF, capture and
Word executable identities; coordinate promotion requires separate comparisons
with the independently rendered document. Existing coordinate tolerances and
Mac originals are unchanged.

| Capture | Documents | Purpose |
| --- | ---: | --- |
| `20261114T095940292Z-d0b413d9` | 24 | Arial, Century, Yu Mincho and MS Mincho separator fonts at 10.5/12/20pt, with short and split notes |
| `20261114T102909672Z-a3c76d0d` | 3 | Distinct ordinary/continuation font choices and named text stories |
| `20261114T103542955Z-0b02677b` | 7 | MS Mincho 20pt at bottom margins 1128–1140 twips, in two-twip steps |
| `20261114T104404068Z-422ee230` | 2 | Splits inside a paragraph containing manual line breaks |
| `20261114T104544207Z-adc363b2` | 1 | A document created and saved by Word itself, with an 80-paragraph footnote and named separator stories |
| `20261114T105523119Z-25f33e56` | 6 | Arial pairs and text at 10.5/12/20pt with explicit kerning off/on, without footnotes |

Every archive contains the original `capture.json`, byte-identical input DOCX,
Word PDF and every page's raw EMF. Captures are appended under their full Word
version. Their original metadata bytes and existing archives are preserved.
The public fixture aliases must match those archived inputs byte for byte.

The named-story documents distinguish `SEPARATOR` from `CONTINUATION`. In these
actual captures, Word paints the ordinary story on the continuation pages too,
including in the Word-authored document. This observation must not be replaced
by an inferred expected line or by a general description of Word's default.

Two further captures preserve the same Word-authored input after selecting
print layout before PDF export (`20261114T120021549Z-c32eac41`), and a new
157-paragraph note followed by a section with a 300pt top margin
(`20261114T121138495Z-3fe33843`). Original coverage is now 237 captures / 870
documents. The latter input is also reproduced by the probe generator.

Word's own PDF and EMF disagree for these two inputs. In the Word-authored
case, note line 69 is on PDF page two and EMF page three. Selecting print
layout before exporting does not resolve that discrepancy. In the section
case, both outputs place the following section on page six, but their
continuation coordinates differ. All original outputs remain unchanged.

The 42 consistent inputs are registered in `font-coordinates.json`: 189
cases / 95,561 glyph origins must match all four coordinate planes. The
58 `note-rules.json` cases also check actual PDF paint, including explicit
zero-rule expectations for the two custom text separators. Unregistered
empty references remain errors.

`print-coordinates.json` separately requires the two inconsistent inputs to
match Word's actual PDF text origins and rule paint. Those checks also assert
that the original PDF/EMF disagreement remains present; they do not declare
the EMF coordinates matched or put these cases in the four-plane registry.
The renderer receives only DOCX bytes and embedded font programs.
The independently extracted Arial Regular 7.07 kerning facts record the full
font's identity and numeric table entries. The Word PDF's embedded subset omits
that kerning table. Font-table facts are separate from expected glyph origins.
