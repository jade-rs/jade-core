# Explicit kerning and nominal CJK wrap widths

Six independent Windows Word captures exercise 20pt Yu Mincho at 319.95
and 320.00pt and MS Mincho at 319.95pt. Each run explicitly specifies
`w:kern` 0 or 32767. The filenames' `enabled`/`disabled` labels encode those
authored values respectively. Reproduce the original DOCX bytes with
`scripts/word-oracle/generate_windows_explicit_kerning_wrap_probes.py`.

At 319.95pt Word wraps Yu Mincho after 15 characters for both settings.
The previous renderer fitted 16 because disabling pair kerning also disabled
the nominal full-cell lookup, leaving a host-quantized 19.995pt cell.
Windows normalization now falls back to the existing cached intrinsic CJK
em when that lookup is absent. The existing 1/64px proximity guard, original
Mac lookup and cache constructor are unchanged. The 320.00pt controls keep
16 characters per line.

An unrelated one-character edit also exposed an export bug: `w:kern` 32767
was rewritten to 3276 by the font-size ceiling. Kerning specifies a minimum
size for applying kerning ([Microsoft's measurement reference](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.hpsmeasuretype.val)),
so its imported threshold is now emitted independently of `w:sz` and
`w:szCs` limits. The edit/export/reopen regression checks that the threshold
stays 16383.5pt while the actual run font size stays 20pt, and preserves the
explicit zero control as well.

Normal-read-only captures from Word 16.0.19127.20936 retain original source,
EMF, PDF, product/file/application versions, build and executable SHA-256 in
immutable ZIP archives under `observations/word-16.0.19127.20936/`. Source and
archive identities are asserted. Later Word versions retain separate
observations.

The coordinate registry includes 1,039 font cases / 160,547 glyph origins /
599 background cases; the edit/export/reopen corpus contains 1,435 inputs.
The original observation registry contains 1,014 captures / 1,925 documents /
1,878 source byte groups. Mac references, independent platform expectations,
existing registry prefixes and tolerances are unchanged.
