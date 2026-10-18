# Changelog

All notable changes to this project are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Versioned, one-based JSON diff contract with cell value types and alignment diagnostics.
- Configurable normalization, match thresholds, and XLSX resource limits.
- End-to-end XLSX fixtures, property tests, and a 10,000-row alignment benchmark.
- CI, security audit, package, WASM, and tagged release workflows.
- Browser Web Worker execution with progress and cancellation.
- Bounded global assignment for sheet renames and row/column moves.
- Workbook-wide formula mapping, cached-value policy, and a golden accuracy corpus.
- A license-tracked public XLSX corpus with deterministic mutation recipes and `insta` snapshots.
- Stress coverage for duplicate-heavy, maximum-width sparse, and multi-sheet workbooks.
- Property coverage for reflexivity, duplicate insertion, forward/reverse symmetry,
  formula shifts, and whole-row/column edits.
- Property coverage for shifted used-range origins, ragged sparse rows, typed
  add/remove inversion, and every supported non-empty value kind.
- Property coverage for local and sheet-qualified formula ranges across generated
  row/column insertions, plus arbitrary formula text parser safety.
- Property coverage for whole-row/whole-column formula ranges and simultaneous
  row/column rotations.
- Property coverage for sheet-qualified whole-row/whole-column formula ranges
  across target-sheet insertions and renames.
- Property coverage for insignificant formula whitespace, Unicode defined-name
  casing, protected literals/intersections, and equivalent numeric representations.
- Property coverage for case-insensitive nested table structured references while
  preserving external-workbook and string-literal changes.
- Property coverage for quoted and unquoted three-dimensional cell/axis references,
  endpoint renames, and conservative coordinate handling.
- Property coverage for equivalent serial/ISO datetime and duration storage,
  long durations, dynamic-array references, and arbitrary duration parser input.

### Changed

- Renamed the Cargo package to `structural-xlsx-diff` to avoid a crates.io collision.
- Upgraded Calamine and quick-xml to patched releases.
- Replaced workbook-wide alignment with exact anchors and bounded DP segments.
- Preserved sparse worksheet and formula coordinates.
- Switched table-region detection to connected components and private release artifacts.
- Added typed-cell shape similarity and linear uniform-run alignment for sparse/duplicate data.

### Fixed

- Stale formula references are no longer hidden after row or column insertion.
- Sheet insertion/removal no longer reports every following sheet as reordered.
- Compound row/column insertions no longer lose alignment after populated leading columns.
- Blank merged-cell boundaries now follow neighboring structural shifts.
- Same-typed edits on minimal sheets no longer appear as column removal and addition.
- Whole-row and whole-column value replacements with a stable cell shape are no longer
  misclassified as structural removal and addition.
- Moving a populated used range now reports the leading blank rows and columns as
  structural additions or removals without introducing cell, formula, or merge noise.
- Formula range starts and sheet-qualified range ends now follow structural mappings,
  avoiding false formula changes after row or column insertion.
- Whole-row and whole-column references such as `1:3` and `A:B` now follow structural
  mappings instead of appearing as formula changes.
- Sheet-qualified whole-row and whole-column references such as `Data!1:3` and
  `Data!A:B` now follow target-sheet mappings and renames.
- Formula formatting whitespace and Unicode identifier casing no longer create
  false formula changes, while string literals and intersection operators remain distinct.
- Signed zero values now use the same canonical representation as positive zero.
- Table structured references such as `Table1[Amount]`, `[@Tax]`, and nested
  `[[#Headers],[Amount]]` forms no longer report case-only formula changes.
- Three-dimensional references such as `'Jan:Mar'!A1:B2`, `'Jan:Mar'!A:C`, and
  `Sheet1:Sheet3!1:3` now follow endpoint sheet renames without unsafe coordinate shifts.
- Serial and ISO datetime/duration cells now share millisecond-precision canonical
  values, including durations longer than 24 hours and serial duration typing.
