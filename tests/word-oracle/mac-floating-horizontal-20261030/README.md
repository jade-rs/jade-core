# Word Mac floating-table horizontal placement

Captured in Microsoft Word for Mac 16.114.2 (16.114.26102345), macOS 27.0.1,
Japanese UI, Office Arial. Both unchanged DOCX files were opened individually,
then exported using Save As > PDF > Best for printing (local export). One page
and 47 visible glyphs per document. The PDFs remain in the capture directory;
the JSON records their SHA-256 and the exact DOCX SHA-256. References were
extracted with jade's `scripts/word-pdf-glyph-reference.py`, without translation
or scaling. JSON baselines retain the measured PDF coordinates.

The paired documents differ only in `compatibilityMode` (14/15). Their eight
108pt single-cell floating tables use margin/page anchors, left/center/right
alignment, and absolute `tblpX=0`. Default horizontal cell padding is 5.4pt;
MRPAD overrides left/right padding to 3/12pt. Their top positions are 80, 150,
220, 290, 360, 430, 500, 570pt. Page size is 612×792pt, left/right margins 57.6pt.

| Label | Anchor / placement | Mode 14 first glyph X | Mode 15 first glyph X |
| --- | --- | ---: | ---: |
| ML | margin / left | 57.60 | 63.36 |
| MC | margin / center | 257.28 | 257.28 |
| MR | margin / right | 457.20 | 451.44 |
| PL | page / left | 0.00 | 5.76 |
| PC | page / center | 257.28 | 257.28 |
| PR | page / right | 514.80 | 509.04 |
| MX | margin / absolute 0 | 57.60 | 63.36 |
| MRPAD | margin / right, 3/12pt padding | 461.28 | 449.04 |

Legacy placement follows the cell text edge for left/right/absolute placement,
where modern placement follows the table frame. Center stays frame-centered.
The 5.76pt measured delta includes border/grid quantization beyond 5.4pt cell
padding. Do not fit a global offset to these PDF positions or change the
existing 0.375pt coordinate tolerance. These captures do not establish nested,
RTL, cell-spacing, text-anchor or inside/outside placement policies.

Generate structurally identical package parts (ZIP timestamps may differ):

```sh
python3 tests/word-oracle/mac-floating-horizontal-20261030/make-probe.py \
  public/fixtures/word-mac-probe-floating-order.docx /tmp/floating-horizontal
```

Each regenerated ZIP member was compared byte-for-byte with its captured
source. Source and PDF identities:

| Mode | DOCX SHA-256 | PDF SHA-256 |
| --- | --- | --- |
| 14 | `a325e0ac1ee3c724a242367e6d067d73641b20ee0c821788f0ca6fb42d2d7f96` | `47c9aa1da4d9866f03cf621fd312e2a109361b5457665f7a033c1bb87c52c7b1` |
| 15 | `3f89f455ba8b8367be54e71d7403fb7786a884324b501bfdc1e789e2c5e91404` | `d8d6f07c21d497677586bb6fe4e86561e657355d7c0770ece5c3389dcde2145a` |
