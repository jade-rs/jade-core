# Windows Word styled page breaks

The archive contains 153 original PDFs exported by Windows Word
16.0.19127.20936. The inputs cover 51 cases in compatibility modes 12, 14,
and 15. The capture manifest records the executable version and hash,
input and PDF hashes, and Word's page counts.

The inputs exercise direct and inherited `pageBreakBefore`, on/off
overrides, column boundaries, and the interaction between paragraph-final
manual page breaks and section transitions. A final manual break
suppresses the before spacing of the next ordinary paragraph. Before
spacing can remain when that paragraph has `pageBreakBefore` or starts a
section. Word collapses the remaining before spacing against the preceding
paragraph's authored after spacing. Next-page and odd/even section
transitions consume the trailing manual break and determine the page
number of the incoming section. A visible trailing manual break also
starts continuous and next-column sections in column one of the next
page. Hidden text after the break leaves that page advance in effect.

Regenerate the inputs with
`python scripts/word-oracle/generate_windows_styled_page_break_cases.py`.
The PDFs were exported from read-only inputs after Word repaginated them.
The Windows CI command verifies input hashes, PDF page sizes, physical page counts,
layout coordinates, and the text and glyph coordinates of JADE's vector
PDFs against the captured Word output.
