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

Excel parity changes (appended to `jade.patch` as a second set of hunks, to be
applied after the first), each pinned by a case in
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
  which the token parser does not model. Appended to `jade.patch` as a third
  set of hunks.
- Japanese Excel (`functions/japanese.rs`; fourth set of hunks in
  `jade.patch`): ASC converts katakana and JIS (new) converts back; the *B
  text functions count Shift_JIS bytes as a Japanese Excel does (REPLACEB too, fifth set of hunks); DATESTRING
  and NUMBERSTRING (new) write eras and kanji numerals. The new names answer
  through `jade_lookup`.
- Tables (sixth set of hunks in `jade.patch`): `Model::add_table` registers
  an Excel table and `Parser::set_tables` hands the parser the new list, so
  structured references (`Table1[Amount]`, `Table1[[#Totals],[Qty]]`) parse
  in formulas set afterwards. The parser used to keep the table list it was
  built with, so a table added later never resolved and the formula read
  `#ERROR!`. Pinned by `structured_references_to_a_table_recalculate` in
  `crates/jade-xlsx-calc/tests/recalculates.rs`.
- DBCS, ENCODEURL and HYPERLINK (`functions/links.rs`; seventh set of hunks in
  `jade.patch`): DBCS is Japanese Excel's name for JIS; ENCODEURL writes every
  byte outside the unreserved characters as `%XX` (UTF-8) and is stored as
  `_xlfn.ENCODEURL`; HYPERLINK shows its friendly name, or the link when there
  is none. The names answer through `jade_lookup`. Pinned by cases in
  `crates/jade-xlsx-headless-editor/tests/excel_conformance.rs`.
- PHONETIC (`phonetic.rs`, `functions/japanese.rs`; eighth set of hunks in
  `jade.patch`): the furigana Excel saved with text cells reaches the engine
  through `phonetic::with_readings` for one calculation, as row visibility
  does; PHONETIC joins the referenced cells' readings, or their text where
  they have none. Pinned by `crates/jade-xlsx-calc/tests/phonetic.rs` and
  conformance cases.
