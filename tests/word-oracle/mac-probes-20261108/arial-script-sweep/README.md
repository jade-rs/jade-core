# Arial ordinary/ruby script offsets — pending Word measurement

Status: prepared, **not exported or measured in Word**. The Mac is locked.

The neighboring `ruby-position` probe found a 0.40 pt native/Word baseline
spacing difference for an 8 pt Arial subscript ruby guide. That one sample
cannot distinguish an Arial-specific baseline rule from a ruby-specific rule.
The existing general script sweeps measure Century, Calibri, Yu Mincho and
MS Mincho; they do not supply an Arial rule.

This probe compares ordinary and ruby superscripts/subscripts in Arial at
49 source sizes (6–30 pt by half points), totaling 196 paragraphs, including
98 ruby cases. `BASE` is always Arial 12 pt; `ABC` carries the tested size and
script. Labels use Arial 6 pt. Ruby raise is explicitly 15 pt and alignment
is left. ASCII, high ANSI and East Asian face names all request Arial to
avoid the ambiguity of an inherited East Asian font. Each ordinary run and
each ruby guide can be compared to its own base baseline without origin fitting.

Run `python3 make_probe.py` here. It uses the synthetic no-grid template in
`../ruby-position/source-template.docx`. Open the generated DOCX in Word and
export it locally as `arial-script-sweep-word.pdf`; close only this synthetic
document. Run `extract.py` using Python with PyMuPDF, then render and visually
inspect the PDF before deriving any baseline rule. The extraction preserves
individual glyph origins, source/PDF hashes and Word version metadata inherited
from the current measurement workflow. Verify that version against the actual
exporting Word installation if it changes.

The generated DOCX was structurally checked: valid document XML, 196 body
paragraphs, 98 ruby elements and 196 script formatting elements. This check
only verifies probe construction, not Word behavior.
