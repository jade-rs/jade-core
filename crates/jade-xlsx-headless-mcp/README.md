# jade-xlsx-headless-mcp

A Model Context Protocol server that reads and edits `.xlsx` over stdio, so an
agent can work on a workbook without a spreadsheet application.

```console
$ jade-xlsx-headless-mcp
```

Newline-delimited JSON-RPC 2.0 on stdin and stdout. `initialize` and
`tools/list` publish the whole interface; a client needs no other
documentation.

## The session

`open_workbook` parses the file once and returns a `session_id`; every other
tool takes it. Reading a cell out of a 5000-row sheet is then a lookup rather
than a parse, and a run of writes patches one copy in memory instead of the
file once per edit. `export_workbook` writes it out; `close_workbook` frees it.

| tool | |
| --- | --- |
| `open_workbook` | from a `path` or `bytes_base64` |
| `summary` | sheets and visibility, used ranges, merges, frozen panes, tables, header/footer, page breaks, shapes and text boxes, defined names, the date epoch |
| `read_cell` | stored value, display text, formula, format code |
| `read_cells` | up to 1000 cells across sheets in one ordered call |
| `read_range` | a rectangle as a grid, empty cells included |
| `read_sheet` | the used range as rows of text |
| `find` | cells whose displayed text or formula source matches |
| `set_cell` | a value or explicit formula, keeping the cell's format |
| `set_cells` | several values/formulas in one atomic sheet update |
| `set_workbook_cells` | several values/formulas across sheets, atomically |
| `recalculate` | evaluate every formula and refresh the caches |
| `fill_range` | continue a run the way the fill handle does: series, dates, counting text, or a repeated pattern |
| `set_format` | bold, italic, colours, a fill, a number format, alignment — added to the style table, values untouched |
| `copy_format` | the format painter: give cells the formats of others, values untouched |
| `insert_rows`, `delete_rows`, `insert_columns`, `delete_columns` | change the grid's shape; formulas, merges, names, tables, comments, shapes, charts and page breaks everywhere follow |
| `set_header_footer` | the printed header and footer of a sheet, in Excel's `&L` / `&C` / `&R` / `&P` language |
| `set_shape_text` | the text of a text box or shape, position and formatting kept |
| `set_note`, `delete_note` | a cell's note (legacy comment), parts created as needed |
| `undo`, `redo` | step the session back through its writes, and forward again |
| `export_workbook` | to a path, or back as bytes |
| `close_workbook`, `list_sessions` | |

## Two things a caller has to know

**A cell answers with what it holds and what it shows.** A date is a number
plus a format: `display` is the only field that reads it as a date, and
`number` is the only one you can compute with.

**Writes recalculate.** Use `as_formula: true` to write a string as a formula;
its leading `=` is optional. Every write then evaluates the workbook's
formulas (the engine is ironcalc) and refreshes their cached values, so a
`read_cell` right after a `set_cell` reports the dependent's new number, and
the exported file carries current caches. A formula the engine cannot
evaluate — an unsupported function, or any formula in a 1904-epoch workbook —
keeps its cached value, and the file keeps `calcPr/@fullCalcOnLoad` so the
next spreadsheet recalculates it. `formula_state` says where each formula
stands: `cached` when its result is current, `stale` when the file still
requests recalculation, and `uncalculated` when it has no result at all.
`recalculate` runs the same evaluation on demand — useful for a workbook
opened stale — and reports what it refreshed and what it skipped, and why.
