# Jade calculation engine adapter

This is IronCalc `ironcalc_base` 0.8.3 from crates.io, with scoped Excel date
and calculation-clock/row-visibility adapters. The package is named `jade-xlsx-calc-engine` so a published Jade
crate cannot silently resolve an unpatched upstream version. The Rust library
keeps its upstream `ironcalc_base` name. Upstream licensing is MIT OR Apache-2.0;
the license texts and original README/Cargo.toml are included.

`upstream-sha256.json` records the original packaged files. They were verified
against the official 0.8.3 `.crate` archive (SHA-256
`ce1dd83cada7da742782076daccda92e33162e9f4396a958ae3e7c5bb64e6a01`). `jade.patch` records
the local differences against that source. When upgrading, review this patch,
refresh the provenance files and run upstream and Jade compatibility tests.

The 2026-10-23 XLSX parity changes extend defined names from references and
LAMBDAs to constants and expressions. Local formulas parse in their sheet's
context; recursive names terminate with a circular-reference error. Name
registration also exposes `supported_function_names()` for reproducible
registration audits. `anchored_formula_value()` exposes a typed result through
`AnchoredValue` for validation predicates and dynamic list sources without
writing a temporary formula into the source workbook. Integration tests are in
`crates/jade-xlsx-headless-editor/tests/defined_names.rs` and
`tests/manages_sheets.rs` (a deleted reference propagates through a name).

Changes add `date_system::with_date_system` and shared date base/bounds used by
conversion, date functions, financial functions and both clock backends. The
thread-local setting is synchronous and scoped, restoring the previous value
on normal return and unwinding. Nested operations and concurrent threads are
isolated. Callers must scope the whole projection/evaluation/formatting
operation; do not keep a scope open across asynchronous work. Jade scopes each
projection and never changes source cell values or formula text for epoch
adaptation. The default is the upstream 1900 behavior.

`clock::with_milliseconds` pins one instant for a synchronous evaluation,
including unsupported-formula re-projection. Its scopes are nested, restored
on unwind and isolated per thread. The formatter extracts time components
after one second-rounding step, including minute/midnight carry. Build metadata
reports the pinned engine/adapter version; it does not inspect the consuming
repository's Git checkout or require a `.git` directory.

`row_visibility::with_visibility` supplies effective row visibility for each
worksheet during evaluation and all re-projections. A default and sparse
exceptions cover implicit rows without creating a million engine row-style
records or linearly searching them for each subtotal cell. Model visibility
queries and SUBTOTAL use the scope; outside it the upstream row-style behavior
is retained. Nested scopes, unwind and thread isolation are tested. SUBTOTAL
modes 110/111 now skip hidden rows, matching the other 101–111 modes. Scoped
filtered ranges are sorted/merged and queried without row-style walks. Saved
worksheet/table filter bodies classify hidden rows as filtered; other hidden
rows retain manual behavior. Unknown active ranges make modes 1–11 produce an
engine error so Jade retains their caches and reports the skip; modes 101–111
can still omit hidden rows. Known manual-hidden overrides distinguish mixed
hiding inside those ranges. An uncertain persisted filter snapshot takes
precedence over those overrides for modes 1–11, allowing Jade to retain their
caches while 101–111 can still use the physical hidden flags. Jade now reapplies
supported predicates explicitly; mixed-hiding Excel reference captures remain
incomplete.

Run `cargo test --manifest-path third_party/ironcalc-base/Cargo.toml --locked --lib`
from the repository root. The upstream suite has eight existing ignored tests;
Jade's date-system scope tests are not ignored. Run the XLSX compatibility and
schema gates for engine integration in Rust, browser WASM and public React APIs.
Excel reference captures remain necessary for complete formula parity,
especially boundary dates and locale-sensitive date text.

Excel parity changes included in the canonical `jade.patch` (apply it once
to the original 0.8.3 archive), each pinned by a case in
`crates/jade-xlsx-headless-editor/tests/excel_conformance.rs`:

- ROUND, ROUNDUP and ROUNDDOWN scale at 15 significant digits as Excel does,
  so `ROUND(1.005,2)` is 1.01.
- The 1900 date system counts Excel's fictional 29 February 1900: serial 1 is
  1900-01-01, 59 is 1900-02-28, 60 is the leap day (DAY 29, shown 2/29/1900)
  and 0 is "January 0". DATE builds January and February 1900 on that
  calendar; every date→serial conversion goes through one function. IronCalc
  followed Google Sheets here; its early-date tests now assert Excel's values.
- AGGREGATE (`functions/aggregate.rs`): all nineteen functions and eight
  options, reference and array forms. Its name is not in `language.bin`, so
  `Functions::lookup` falls back to `jade_lookup` for Jade's additions.
- ROWS and COLUMNS take arrays (constants and dynamic-array results), and a
  single value counts as one row by one column.
- `xlsx_function_prefix` exposes which functions an `.xlsx` formula must write
  with `_xlfn.` / `_xlfn._xlws.`, and DAYS, FORECAST.LINEAR, GAMMA.DIST, GAUSS,
  IMSECH, NETWORKDAYS.INTL and WORKDAY.INTL gain the prefix upstream omitted.
- The number formatter (TEXT) reads the Japanese date codes — era `g`/`gg`/
  `ggg`, era year `e`/`ee` (`formatter/japanese.rs`), weekday `aaa`/`aaaa` —
  and a bare locale tag such as `[$-411]`, which it used to reject as
  `#VALUE!`; and fractions (`# ?/?`, `?/??`, `# ?/8`, `formatter/fraction.rs`),
  which the token parser does not model.
- Japanese Excel (`functions/japanese.rs`): ASC converts katakana and JIS (new) converts back; the *B
  text functions count Shift_JIS bytes as a Japanese Excel does (REPLACEB too); DATESTRING
  and NUMBERSTRING (new) write eras and kanji numerals. The new names answer
  through `jade_lookup`.
- Tables: `Model::add_table` registers
  an Excel table and `Parser::set_tables` hands the parser the new list, so
  structured references (`Table1[Amount]`, `Table1[[#Totals],[Qty]]`) parse
  in formulas set afterwards. The parser used to keep the table list it was
  built with, so a table added later never resolved and the formula read
  `#ERROR!`. Pinned by `structured_references_to_a_table_recalculate` in
  `crates/jade-xlsx-calc/tests/recalculates.rs`.
- DBCS, ENCODEURL and HYPERLINK (`functions/links.rs`): DBCS is Japanese Excel's name for JIS; ENCODEURL writes every
  byte outside the unreserved characters as `%XX` (UTF-8) and is stored as
  `_xlfn.ENCODEURL`; HYPERLINK shows its friendly name, or the link when there
  is none. The names answer through `jade_lookup`. Pinned by cases in
  `crates/jade-xlsx-headless-editor/tests/excel_conformance.rs`.
- PHONETIC (`phonetic.rs`, `functions/japanese.rs`): the furigana Excel saved with text cells reaches the engine
  through `phonetic::with_readings` for one calculation, as row visibility
  does; PHONETIC joins the referenced cells' readings, or their text where
  they have none. Pinned by `crates/jade-xlsx-calc/tests/phonetic.rs` and
  conformance cases.
- BAHTTEXT (`functions/thai.rs`): the
  amount rounded to the satang in Thai words, บาท then ถ้วน or the satang and
  สตางค์, ลบ before a negative; ten is สิบ, twenty ยี่สิบ, and a final one
  reads เอ็ด after any higher digit, the millions included. The name answers
  through `jade_lookup`. Pinned by conformance cases.


The compatibility-call milestone registers `ECMA.CEILING`, `SINGLE` and
`ANCHORARRAY` in lookup, function metadata and the English inventory (506
names). ECMA.CEILING shares CEILING's two-argument arithmetic and keeps its
bare saved spelling. Valid SINGLE/ANCHORARRAY calls lower to the existing
intersection/spill AST operators, case-insensitively; invalid arities use the
normal argument-count error. Their saved names receive `_xlfn.`. Names bound
with `_xlpm.` do not enter this operator-name special case.

Implicit intersection now scalarizes arrays to their top-left item, retains
scalar/error types, and intersects a range using the caller's row/column on
the referenced worksheet. Value/reference entry points use the same scalar
result. This fixes a larger-array scalar-context assertion and cross-sheet
range rejection. The FORMULATEXT signature now retains reference operands;
ordinary multi-cell references use the top-left formula, invalid scalar inputs
return VALUE, and formulas longer than 8192 UTF-16 units return NA. Formula text
still uses the engine's normalized English spelling; protected/hidden formula
handling and further Office oracle cases remain open. The upstream tests that
expected unimplemented range behavior now assert actual top-left formulas;
explicit @/scalar operands assert VALUE. See `docs/xlsx/compatibility-functions.md`
in jade-office for runtime evidence and the remaining ibisOffice function gaps.


CEILING and ECMA.CEILING share numeric coercion and broadcast scalar/row/column
operands over arrays, preserving per-element errors and the existing million-
cell output-matrix limit. The compatibility suite covers both spellings,
negative/zero significance, precision, lambda shadowing, spill growth/history,
cycles and saved prefixes. Regenerate and replay the canonical patch with
`python3 scripts/xlsx/replay_engine_patch.py --archive /path/to/ironcalc_base-0.8.3.crate --update`
from the jade-office root; omit `--update` for verification only. The original
archive and provenance inventory remain hash-pinned.


Spill references evaluate the anchor before reading its extent, reject ordinary
scalar-formula cells and non-reference/multi-cell operands with REF, and
propagate anchor/operand errors (including cycles and blocked spills). A
one-cell dynamic array still has a valid array extent. ANCHORARRAY and the
spill operator share this path; classic CSE/other boundary behavior still
needs application captures.

GETPIVOTDATA uses a model-owned `Vec<Arc<pivot::Region>>` populated by the XLSX
host, so independent models have independent pivot identities and visible
axis descriptors. It reads saved visible summary cells, including their normal
formula dependencies, without aggregating source/cache records or claiming a
pivot refresh. Row/column repeated prefixes, separate cache/field item indices,
report filter selection, display/root captions, typed/date criteria and array
broadcasts are projected by the adapter. Missing or ambiguous layout metadata
produces a calculation artifact that lets Jade preserve imported formula
caches; genuinely invalid visible queries return REF. Reference-only evaluation
now preserves single-cell/range defined names, with the existing name recursion
guards. The registry contains 507 names. Integration tests are in
`crates/jade-xlsx-calc/tests/pivot_calculation.rs`; model independence, coordinate
overflow and cycles are covered in `src/pivot.rs`. Legacy descriptors, OLAP and
application oracle coverage remain incomplete. See jade-office's
`docs/xlsx/getpivotdata.md` for exact scope and regression evidence.


Optional `dependencies::ReadDependencies` traces cell/range references inside
one model. `trace_value_reads(limit)` enables it before evaluation and
`take_value_reads()` consumes it. Ordinary calculations allocate no trace.
Direct/dynamic/named references, actual value reads and spill-anchor edges are
recorded; contained cell reads compact into existing rectangles. Formatting
formulas are excluded. Restarted spill evaluation resets the trace. A caller
must refuse an incomplete trace when its edge budget was reached. Reference
operands can conservatively overapproximate value dependencies; this is not an
Excel dependency-chain serialization API or a static all-branches oracle.
The XLSX adapter uses the same evaluated values and observed references to
order pivot cache refresh, rechecking changing footprints/dependencies and
refusing cycles within an atomic batch. Tests cover model isolation, optional
tracing, edge budgets, range compaction and lazy branches; broader scheduling
and application oracles remain incomplete.
