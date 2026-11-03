# Schema repairs of captured inputs

The 2026-11 schema cleanup repairs 74 of the 175 public DOCX inputs. Their
current exact bytes are pinned by `docx_sha256` in `release-corpus.json`.
`schema_repair_source_sha256`, when present, records the previous exact input
hash. This is an explicit, one-way input migration, not a new Word capture.
The original Word JSON/PDF files and their recorded input hashes are unchanged.

The repairs reorder schema-sequenced properties; supply required shading and
DrawingML anchor attributes/children; remove duplicate or forbidden properties;
restore missing markup-compatibility namespaces; use valid VML wrap/anchor and
OnOffOnly values; uppercase document GUIDs; and assign unique shape/revision IDs.
Every ZIP entry name, non-XML part, and ordered non-whitespace XML text was
checked against the preceding input. The public duplicate-revision-ID sample
now has unique IDs; its deliberately invalid counterpart is kept only under
`crates/jade-docx-editor/tests/fixtures` for robustness tests.

The floating-order probe is a deliberate malformed-order control: Word for Mac
ignores its first table's positioning because it precedes the table style.
Sorting those properties would activate the position and move the table.
The valid derivative instead removes that ignored position, preserving the
captured Mac layout. The exact original remains under native-render test
fixtures, and a coordinate regression compares both inputs. The generator
requires `--malformed` to reproduce the invalid historical control; its default
is valid. This derivative preserves the Mac observation, not the original
platform-dependent handling of invalid markup.

Native coordinate and glyph-sweep tests accept a historical reference only
when its hash equals the current input or matches this exact reviewed pair.
They continue comparing the repaired input's coordinates to the unchanged Word
and native snapshots with the existing tolerances. Unknown inputs, another
fixture's pair, reversed pairs, and any subsequent byte change still fail.
The strict proof-bearing Word evidence audit does not use these migrations:
a repaired derivative needs a fresh Word capture before it can be counted as
verified evidence for its new bytes.

The historical references establish behavior for the pre-repair documents;
passing these comparisons is regression evidence for the repaired derivatives,
not evidence that Word was rerun on them. For future content or layout changes,
recapture Word rather than extending this schema-only migration.
