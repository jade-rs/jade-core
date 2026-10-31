# Typing writes the package once

After jade-rs/jade-core#262, one number typed into a 20,000-row sheet still
cost about 0.9 s of CPU, and more with every edit: the edit was packed into
the package, the sheet recalculated, and the package packed again with the
new caches. A sample of forty edits put a quarter of the time in deflate, a
quarter in reading the sheet XML for its structure (three times per edit),
and most of the rest in the calculation engine parsing every formula again.

- **One write for an edit and its recalculation.** A plain typed value (see
  `write_edits_with_base`) now goes into the model, the model is
  recalculated, and `write_edits_with_cached_values` patches the edited
  sheets with both and packs the package once. A recalculation that spills
  or fails falls back to the two writes. `debug_assert_eq!` compares the
  model with a full reread in every test build, as the in-place paths
  already did.
- **Typing packs quickly; a save packs properly.** Inside
  `jade_opc::with_fast_compression` parts are deflated at zlib's fastest
  level (deflate_quick), about five times faster for a fifth more bytes, and
  their fingerprints (name, CRC-32, compressed size) are noted.
  `export_bytes` deflates those entries again at the default level, so a
  saved file is the size it was; a workbook undone back to what was opened
  saves the opened bytes.
- **The sheet is read only as far as the edit.** `patch_sheet` stops once
  every edit is placed and passes over rows it has nothing for without
  parsing their cells; the projection notes whether rows and cells are in
  order, so the separate ordering pass runs only for a sheet that needs it;
  and looking up given cells (`compatible_cells`) stops when all are found in
  a sheet without compatibility branches.
- **Formulas are parsed once per shape.** A model is still built for every
  recalculation, but `jade-xlsx-calc` remembers each cell's formula text and
  the shared (R1C1) form the engine parsed it to, keyed by the sheets,
  tables and names it parsed against. A formula whose shared form the new
  model already holds — every row of a filled-down column after the first —
  is set from it without parsing (`update_cell_with_shared_formula`).
- **Fewer allocations reading a sheet.** Element names every sheet is made
  of are borrowed rather than allocated, attribute values are borrowed when
  they need no unescaping, and a cell's namespace is copied only where the
  cell is recorded.

The shells also keep one text measure per workbook, with the widths it has
measured per font, so laying a sheet out again after an edit measures only
what changed.

## Measurement

`cargo run --release --example edit_profile -p jade-xlsx-headless-editor --
book.xlsx 4 B19990` (the third argument, new, is the cell typed into), on
the generated 20,000-row book of the earlier note. CPU time from
`getrusage`; the machine was heavily loaded, so single runs vary by ±10%.

| CPU ms | before | after |
| --- | --- | --- |
| `set B2`, first to eighth | 873 → 1,100 | 300 |
| `set B19990` (last rows) | — | 384–410 |
| `recalculate` | 216 | 110 |
| `open_for_editing` | 918 | 720–800 |
| `save_bytes` | 200 | 190–240 |

Before, each edit cost more than the one before it; now they cost the same.

## What is left

A typed number still reads the whole edited sheet twice when the edit is
near its end (once for its structure, once for the formula cells whose
caches change), builds the calculation engine's model from every cell, and
hashes the engine's cells with SipHash. Each is a few percent to a quarter of
what is left; keeping the engine's model across edits would remove the
largest.

## Then: charts and the relayout

Two more costs showed in a sample of jade-office's `xlsx-edit` (typing and
repainting, as the shells do):

- **A workbook with a chart typed through the two writes**, since the chart
  caches follow their cells. The single write now refreshes them from every
  formula cell the recalculation answered, and the model is reread rather
  than taken in place.
- **Laying the sheet out again formatted every number again**, parsing each
  cell's format code each time: a third of the relayout. Formatted numbers
  are remembered per format code and value.

Five numbers typed and painted on the 20,000-row book went from 3.7–4.2 s to
2.85 s of the measured region (6.07 s before jade-rs/jade-core#262).
