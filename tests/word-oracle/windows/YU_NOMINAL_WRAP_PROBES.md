# Yu Mincho nominal full-cell wrap boundaries

Five independent source DOCX files contain 128 fullwidth CJK characters at
20pt, with text widths 319.90, 319.95, 320.00, 320.05 and 320.10pt. Reproduce
the exact captured source bytes with
`scripts/word-oracle/generate_windows_yu_nominal_wrap_probes.py`.

The Windows observations preserve normal read-only captures from Word
16.0.19127.20936, including the product/file/application versions, build,
executable hash, original input, PDF and EMF. The immutable ZIP archives live
under `observations/word-16.0.19127.20936/`; their hashes and source hashes are
asserted by `check_windows_observations.py`. Future Word versions retain
separate records rather than overwriting these originals.

At 319.95pt Word wraps after 15 characters. The previous native renderer used
a host-quantized full cell of 19.995pt and fitted 16. The layout now restores
the known face's nominal full cell for Windows, using the existing Mac
normalization and its existing 1/64px proximity guard. Oracle-driven layout
remains excluded. The Mac branch and coordinate tolerances are unchanged.
At 320.00pt and wider Word and the native renderer fit 16 characters.

All five cases are asserted against original Word EMF, original PDF with and
without clipping, and newly painted native PDF. Paragraph background bounds
are checked as well. Together with the prior corpus, the pinned Windows
font registry contains 1,021 cases, 159,242 glyph origins and 581 background
cases; the edit/export/reopen corpus contains 1,417 documents. The separate
Mac reference registry continues to be asserted by the macOS native and
browser CI jobs. A platform's expected coordinates must come from that
platform's original reference, not from the other platform's output.
