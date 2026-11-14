# Windows table style and story observations

Twenty additional original measurements retain Word 16.0.19127.20936 input,
PDF, every page's EMF, capture metadata and executable identity. They are not
newly accepted renderer coordinates. Total archive coverage is 239 captures /
890 documents; existing originals and coordinate tolerances are unchanged.

- `20261114T123229055Z-ab9ce843`: ten ordering diagnostics. The initial probe
  construction placed `jc`/`tblInd` before `tblStyle`; Word disregarded some
  of that direct formatting. Their byte-identical inputs are retained under
  `observations/inputs`, with the raw original outputs, for provenance.
- `20261114T123640703Z-7983f59d`: ten corrected CT_TblPr-order probes, with
  compatibility modes 12/14/15, omitted/zero indentation, and left/center/right
  alignment. These public inputs are reproduced byte for byte by
  `generate_windows_header_table_probes.py`; its tests enforce child order.

Each corrected document contains the same table in its body and header,
plus the existing footer table. The original Table Grid style inherits a
zero DXA indentation from Table Normal and also declares its own zero.
In modes 12/14, Word aligns the leading cell text with the margin even when
direct indentation is omitted. An explicit direct zero and an inherited
zero agree. In mode 15, the same text begins at 63.24pt instead of 57.60pt.
Correctly ordered center/right formatting changes the table's position.

The current renderer still displaces the inherited-zero cases horizontally.
The mode-15 controls also expose a separate Calibri title kerning difference.
Retain both failures; neither is registered as a four-plane match. Existing
Mac sources and goldens are unchanged.
