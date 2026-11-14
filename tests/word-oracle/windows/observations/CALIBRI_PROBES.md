# Windows Calibri kerning observations

Twelve original inputs retain Word 16.0.19127.20936 input/PDF/EMF bytes and
complete capture/executable provenance. Archive coverage is 245 captures /
946 documents. Mac originals and tolerances are unchanged.

- `20261114T141251318Z-71154518`: Calibri Regular at 10.5/12/20/22pt, with
  explicit kerning disabled/enabled. Paragraph marks use the same requested
  face and size; the body grid is inactive. All four unkerned cases matched
  before the change; all four kerned cases exposed missing pair adjustments.
- `20261114T142114395Z-08d9c370`: additional 22pt controls with 0.25/0.5pt
  tracking, with kerning disabled/enabled. Word's own PDF and EMF origins
  disagree beyond 0.375pt. These remain unpromoted observations; do not
  relabel either output as a successful four-plane renderer match.

The independently identified Calibri Regular 6.27 font has SHA-256
`57e93ff143572bcc092c1f56202d43ddc07bb119aa104c6d7af163470fb551f6`.
Its GPOS Latin default `kern` feature uses lookup 1 (IgnoreMarks), with
XAdvance adjustments only. Numeric facts for all 611 nonzero printable-ASCII
pairs are retained in calibri-kerning-font-facts.json; font programs are not
replaced by expected origins. The existing Mac pair policy is unchanged.

The untracked controls establish requested-size pair advances. A diagnostic
attempt to scale all advances to a rounded PDF font size fails these
controls and must not be used to fit tracking-related PDF/EMF differences.
