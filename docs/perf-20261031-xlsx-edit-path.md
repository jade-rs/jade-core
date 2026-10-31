# A cell edit no longer reads the workbook twice

Every `HeadlessWorkbook` write patches the package and reads the model back
from it; a value edit then recalculates, writes the cached values and read the
package a second time. On a 20,000-row sheet one typed number cost 2.7 s of
CPU. The work now done once, or not at all:

- **Feature parsers skip what a part does not mention.** Data validations,
  conditional formats, auto filters, hyperlinks, table parts, drawings and the
  filter-row state each parsed the whole sheet part again. Each now returns
  empty when the part never names its element (the way sparklines already
  did); a malformed part still fails in the grid projection, which reads all
  of it.
- **Rows and cells are found by binary search.** The reader keeps them sorted;
  `Sheet::row`, `Sheet::cell_at` and `Row::cell` replace linear scans, one of
  which ran once per formula during recalculation (quadratic in the sheet).
- **Recalculated values go into the model in place.** When no chart or pivot
  cache follows them and no full calculation is requested, the cells take
  their new values and the package is not read again.
- **Typed values go into the model in place.** Numbers, text and booleans over
  cells without formulas, on sheets without tables, array formulas, charts or
  pivots, change nothing the model derives from elsewhere; the write's model
  is updated rather than reread. Both in-place paths are checked against a
  full reread by `debug_assert_eq!`, so every test build compares them.
- **Unchanged parts are copied compressed.** Writes use
  `OpcPackage::open_preserving`, so only the parts an edit changed are deflated
  again (pictures and other sheets no longer are).
- **One structure projection per patch.** `patch_sheet` found the worksheet's
  structure twice; the offset-only projections no longer rebuild each start
  tag's attributes either.
- **Shared formulas are indexed.** The calculation engine looked a cell's
  R1C1 formula up with a linear scan of the sheet's distinct formulas; a hash
  index, checked on every hit, makes a sheet of distinct formulas load in
  linear time.

The browser and native shells also lay the sheet out after each edit. Text
measurement compared a CSS font string built per cell; it now compares the
font itself. Layout found each cell's merged range by scanning every merge
(thousands on a form laid out in merged cells, Excel 方眼紙); a per-row index
replaces the scan.

## Measurement

`cargo run --release --example edit_profile -p jade-xlsx-headless-editor --
book.xlsx 3` prints each phase's CPU time (`getrusage`), which on a loaded
machine is steadier than wall time. Books are generated: 10 columns of text,
numbers, `SUM(B:G)` and a dependent formula per row; the second variant gives
every row a distinct formula and a merged pair of cells. Baseline 95379de and
candidate ran interleaved; medians of three runs (two for the last book).

| CPU ms | 20,000 rows: base / new | 2,000 rows: base / new | 8,000 rows, distinct formulas: base / new |
| --- | --- | --- | --- |
| `read_xlsx` | 662 / 299 | 72 / 29 | 259 / 125 |
| `open_for_editing` | 2,100 / 770 | 175 / 67 | 922 / 339 |
| `recalculate` | 439 / 188 | 19 / 18 | 296 / 95 |
| `set` (one cell) | 2,700 / 816 | 223 / 71 | 1,100 / 335 |
| `undo` | 683 / 343 | 65 / 31 | 266 / 131 |
| `save_bytes` | 418 / 208 | 19 / 18 | 296 / 98 |

jade-office's `jade-office-perf xlsx-edit` (typing five numbers over a
number and repainting a 2560×1600 viewport after each, on the tiny-skia
painter) went from 8.05 s to 6.07 s of user CPU on the 20,000-row book with
the shell changes, open included.

## What is left

A typed number still deflates the edited sheet twice (the edit, then its
cached values), builds the calculation engine's model from every cell and
formula, and lays the whole sheet out again in the shells. Each needs the
model, the engine model or the layout kept across edits rather than rebuilt.
