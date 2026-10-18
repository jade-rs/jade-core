# P3 generated fixtures

`stress-cases.json` is the checked-in input specification for deterministic scale tests. The test
runner generates the repetitive workbook IR from these compact parameters, verifies exact
structural outcomes, and records normalized result snapshots under `tests/snapshots/`.

Keep scale parameters, expected assertions, and reviewed snapshots in the same change. Generated
workbook binaries are intentionally not checked in because their repeated content is completely
defined by this fixture and does not exercise the XLSX loader; public OOXML loader inputs live in
`../public/`.
