# Arial script baselines in Word for Mac

This probe records the baseline of regular Arial superscripts and subscripts
in ordinary text and ruby guides. It covers 49 source sizes from 6 to 30 pt in
half-point steps, for 196 cases. The `BASE` text uses Arial 12 pt. The `ABC`
text uses the tested size. Ruby guides specify a 15 pt raise. The ASCII, high
ANSI and East Asian font settings all name Arial.

The eight-page PDF was captured in Microsoft Word for Mac 16.115.1
(16.115.26111533) through Print → PDF → Save as PDF on macOS 27.0.1 (26A434).
Document metadata was removed without changing the extracted glyphs or their
positions. SHA-256 hashes:

- DOCX: `4d1a618597008c18a52f50193a7eeef6aac971997345d3a8054c69159c632bec`
- PDF: `b3d27e7b65baae34c0b5415d9bae121ba49707492422f8183b6bb514527cc5c4`

Run `python3 extract.py` with PyMuPDF to regenerate `word-baselines.csv`.
Each row records the difference between the source baselines of `BASE` and
`ABC` on the same page. The Ruby rows include the specified 15 pt raise.
`make_probe.py` generates the source from the synthetic no-grid template at
`../ruby-position/source-template.docx`.

The two contexts use the same half-point script moves within 0.375 pt except
for 9 and 9.5 pt subscripts. At those sizes the ordinary script stays on its
baseline and the ruby script moves about 1 pt down from its guide baseline.
