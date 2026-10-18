# Public XLSX corpus

These workbooks are immutable inputs for loader, accuracy, and regression tests. They are
redistributed under the licenses recorded in `manifest.json`; the repository's MIT license does
not replace those fixture licenses.

Every source URL is either a versioned government dataset URL or a Git commit-pinned raw URL.
CI never downloads data: `tests/public_corpus.rs` verifies each checked-in file's byte length and
SHA-256 before loading it. To update the corpus, download the declared `source_url`, verify its
license, update the checksum and size, and review the resulting snapshots in the same change.

The tests create deterministic in-memory mutations declared in `mutations.json` from these
immutable files. This keeps the original OOXML intact while exercising row/column insertion,
sheet operations, formulas, merged cells, sparse layouts, and typed values. Each mutation case
contains its ground-truth event set; the test requires exact precision and recall before recording
the complete normalized diff as an `insta` snapshot.

## Attribution

- Office for National Statistics, *Population estimates time series dataset*, version 15.
  Licensed under the Open Government Licence v3.0.
- CoreXLSX contributors. `Dates.xlsx` and `categories.xlsx` are redistributed under Apache-2.0;
  the license is included in `LICENSE-APACHE-2.0.txt`.
- Copyright (c) 2016 Johann Tuffe and calamine contributors. The four calamine test workbooks
  are redistributed under MIT; the license is included in `LICENSE-CALAMINE-MIT.txt`.

See `manifest.json` for exact source pages, revisions, download URLs, hashes, and license URLs.
