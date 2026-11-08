# Ruby guide decorations — Word for Mac probe

Status: **prepared, Word export not completed**. No Word compatibility conclusion
is established by this fixture yet. The export attempt on 2026-11-08 ended
with AppleEvent timeout (-1712) while the Mac remained locked. The process
was confirmed terminal; it is no longer exporting in the background.

The seven rows compare no decoration, single underline, double underline,
word-only underline, strike, double strike, and strike plus underline. Each
ruby guide contains `AB CD`, in red Arial 8 pt, above `BASE` in Arial 12 pt.
Underlines explicitly request green. Every case has the same ruby properties
(left alignment, explicit 15 pt raise) and a following ordinary paragraph.

`source-template.docx` is the same no-grid template used by the adjacent
`ruby-position` experiment. `make_probe.py` replaces only its document body
and generates `ruby-decorations.docx`; no user document is included.

After exporting the synthetic document to `ruby-decorations-word.pdf` in
Microsoft Word, run `extract.py` using Python with PyMuPDF. It records source
and PDF SHA-256 hashes, glyph origins, colors, and vector drawing coordinates
in `word-glyphs.json`, and renders page one to `word-preview.png` for visual
inspection. Inspect both the vectors and preview before making a renderer
change. Decoration existence, colors, placement, line widths, and handling
of the space are all intentionally unresolved until the export is examined.
