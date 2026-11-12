# Arabic word wrapping in narrow table cells

The synthetic DOCX uses five fixed cell widths from 19 to 32 pt in Word
compatibility mode 14. It covers repeated Arabic words, vowel marks,
and a right-to-left paragraph with explicit right alignment. The widths
exercise breaks at spaces and within words wider than a cell.

The reference records shaped text origins from the five-page Word for Mac
PDF. The comparison matches Arabic spans at their leftmost glyph origins
and baselines. Where a PDF span combines adjacent text across a nonjoining
letter, the reference uses the corresponding raw glyph origins for each
span. The per-axis tolerance is 0.65 pt.
