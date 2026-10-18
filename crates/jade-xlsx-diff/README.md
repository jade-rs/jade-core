# structural-xlsx-diff

Structure-aware Excel `.xlsx` comparison for Rust, the command line, and WebAssembly.
Rows and columns are aligned before cells are compared, so inserting one record does not
turn every following coordinate into a false-positive change.

## 取り込みについて

このディレクトリは [bokuweb/xlsx-diff](https://github.com/bokuweb/xlsx-diff) を
このリポジトリへ取り込んだもの。取り込み元は
`c9494cd44452572d2f98b3190e8f7ce6b813157e` ("Add side-by-side report layout (#5)")。

上流に追従するときは、その差分をこのディレクトリへ当てて `rev` の記載を更新する。
WASM ラッパ (`crates/xlsx-diff-wasm`) とブラウザ playground は使っていないので
持ってきていない。ライブラリ名とバイナリ名は上流のまま `xlsx_diff` / `xlsx-diff`。

ワークスペースからは path 依存で参照している:

```toml
xlsx_diff = { package = "structural-xlsx-diff", path = "crates/xlsx-diff" }
```

## Library

```rust
use std::path::Path;
use xlsx_diff::{diff_workbooks, load_workbook};

let old_path = Path::new("old.xlsx");
let new_path = Path::new("new.xlsx");
let old = load_workbook(old_path)?;
let new = load_workbook(new_path)?;
let diff = diff_workbooks(old_path, new_path, &old, &new)?;

if diff.has_changes() {
    println!("{} cells changed", diff.summary.cells_changed);
}
# Ok::<(), anyhow::Error>(())
```

Use `DiffOptions` with `load_workbook_with_options` and
`diff_workbooks_with_options` to configure whitespace normalization, cached-formula-value
policy, match thresholds, and resource limits. Input limits are enforced before ZIP
extraction and alignment.

Human-readable reports default to the merged event layout. Library users can select the
side-by-side layout without changing the JSON diff contract:

```rust
use std::path::Path;
use xlsx_diff::{ReportLayout, write_html_report_with_layout};

write_html_report_with_layout(
    Path::new("diff.html"),
    &diff,
    ReportLayout::SideBySide,
)?;
```

## CLI

Print JSON to stdout:

```bash
xlsx-diff old.xlsx new.xlsx
```

Write individual reports:

```bash
xlsx-diff old.xlsx new.xlsx \
  --json diff.json \
  --html diff.html \
  --xlsx diff.xlsx
```

Render HTML and XLSX reports with aligned old/new columns:

```bash
xlsx-diff old.xlsx new.xlsx \
  --html diff.html \
  --xlsx diff.xlsx \
  --report-layout side-by-side
```

The default is `--report-layout merged`. JSON output is unchanged by report layout.

Write every report format and return status 1 when a difference exists:

```bash
xlsx-diff old.xlsx new.xlsx \
  --out-dir reports \
  --basename diff \
  --check
```

`--preserve-whitespace`, `--max-input-bytes`, `--max-uncompressed-bytes`,
`--max-zip-entries`, `--max-rows-per-sheet`, `--max-columns-per-sheet`, `--max-cells`,
`--max-alignment-pairs`, `--max-assignment-size`,
`--compare-cached-formula-values`, `--report-layout`, and the match-score flags override
the safe defaults shown by `xlsx-diff --help`.

## Diff contract

- JSON contains `schema_version: 1`.
- The machine-readable contract is [`schema/diff-v1.schema.json`](schema/diff-v1.schema.json).
- All numeric row, column, and range fields are one-based.
- `coordinate_system` is serialized as `one_based`.
- Cell changes retain normalized text plus `old_value_kind` and `new_value_kind`.
- Match confidence and duplicate-signature diagnostics are heuristic signals, not IDs.
- A breaking JSON or Rust API change requires a SemVer major release after `1.0.0`.

## Supported comparison

- Sheet add/remove/rename and relative reorder
- Table-region detection using connected non-empty cells
- Row/column add/remove/move with bounded alignment and global move assignment
- Cell value and value-type changes
- Formula changes with local and workbook-wide sheet-reference insertion mapping
- Merged-cell range changes
- JSON, HTML, and XLSX reports

## Deliberate limitations

Formatting, conditional formatting, comments, hyperlinks, hidden state, filters, data
validation, named ranges, charts, images, pivot tables, VBA, and workbook metadata are not
compared. External-workbook references, 3D references, whole-row/whole-column references,
and structured table references are compared conservatively; a changed reference is
reported rather than guessed. Cached formula values are ignored by default when formulas
are structurally equivalent because XLSX writers may leave stale caches; strict comparison
is available through `FormulaValueComparison::CompareCached` or the CLI flag.

Alignment is heuristic when rows or columns are duplicated. Move and rename candidates use
a maximum-total-score one-to-one assignment, bounded by `max_assignment_size`. Inspect the
serialized `alignment` diagnostics and tune `DiffOptions` for domain-specific data. Table
regions use 8-neighbor connectivity, so visually separate blocks that touch diagonally are
treated as one region. Inputs exceeding configured limits fail with an error.

## Accuracy gates

The test suite includes a synthetic golden corpus plus a checked-in public corpus sourced from
versioned ONS data and commit-pinned CoreXLSX/calamine fixtures. The public corpus manifest records
source, revision, license, attribution, byte count, and SHA-256; CI verifies it without network
access. Declarative mutations cover row/column insertion, typed values, formulas, merged cells,
sparse regions, and multi-sheet operations. Their declared structural events must retain 100%
precision and recall, and the complete normalized diffs are reviewed as `insta` snapshots. See
[`tests/fixtures/public/README.md`](tests/fixtures/public/README.md) for provenance and update rules.

Property and stress tests also cover compound insertion, workbook-wide and renamed-sheet formula
shifts, cached formula values, duplicate matching, the 512-assignment boundary, 512 duplicate
rows, 16,384-column sparse sheets, 128-sheet workbooks, cell types, and deterministic output.
The scale inputs and reviewed results are documented in
[`tests/fixtures/p3/README.md`](tests/fixtures/p3/README.md). Production workbooks should be added
as anonymized cases when new domain-specific ambiguity is found.

### Large-workbook profiling

The profiling example generates two dense workbooks, inserts one row, changes three cells, and
asserts the exact diff. Its defaults reproduce a 50-column by 50,000-row workload:

```bash
cargo run --release -p structural-xlsx-diff \
  --example profile_large_workbooks -- --generate-only

cargo run --release -p structural-xlsx-diff \
  --example profile_large_workbooks -- --compare-only
```

Use `--rows`, `--columns`, and `--output-dir` to change the workload or fixture location. To
capture a sampling profile with [samply](https://github.com/mstange/samply), reuse the generated
files so fixture creation is excluded from the recording:

```bash
samply record --save-only --output xlsx-diff-profile.json.gz -- \
  target/release/examples/profile_large_workbooks --compare-only
```

## Browser playground

The playground runs locally in a Web Worker; selected workbooks are not uploaded. A manual
`Build private playground` workflow also creates a short-lived artifact for authorized
repository users without publishing a Pages site.

Build it locally:

```bash
cd crates/xlsx-diff-wasm
wasm-pack build --release --target web --out-dir ../playground/www/pkg

cd ../../
python3 -m http.server 8080 --directory crates/playground/www
```

Open <http://localhost:8080>. Long-running work can be cancelled by terminating the worker.

## Development and release policy

The minimum supported Rust version is 1.88. Pull requests must pass formatting, Clippy,
workspace tests, rustdoc, WASM build, package verification, and RustSec audit checks.
Release notes follow [Keep a Changelog](https://keepachangelog.com/) and versions follow
[Semantic Versioning](https://semver.org/). See [SECURITY.md](SECURITY.md) for reporting.
Maintainer release steps are in [RELEASING.md](RELEASING.md).

## License

MIT. See [LICENSE](LICENSE).
