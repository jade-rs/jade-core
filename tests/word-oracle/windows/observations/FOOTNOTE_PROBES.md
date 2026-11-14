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

The Word-authored case exposes remaining text/page-placement differences.
Retain it while fixing the renderer; do not promote the current Jade output.
The independently extracted Arial Regular 7.07 kerning facts record the full
font's identity and numeric table entries. The Word PDF's embedded subset omits
that kerning table. Font-table facts are separate from expected glyph origins.
