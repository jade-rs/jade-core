# Cell registry ownership during incremental rendering

## Finding

The existing headless Chromium profile uses a 300-page document with one table
per page, formatting decorations, and 32 distinct Japanese composition previews.
In its warm baseline recording, a vector-copy specialization below
`CanvasRenderer::render_with_numbering_rc_once` accounts for 30 of 634 active
worker samples (0.9375 samples per preview). Sampling identifies a renderer-entry
allocation path; it does not by itself identify every allocation's source or
prove an elapsed speedup.

At core revision `b13f8c7`, render entry explicitly copies the complete cell
registry into `previous_cells`, then copies it again when preserving completed
pagination. `CellLayout::clone` also copies each cell's paragraph list. This work
scales with all registered cells even when a preview changes one paragraph.

## Change

`SelectionManager::take_cell_layouts` moves the existing registry before the
incremental reset. Frozen bitmap-only passes leave their registry in place.
The renderer retains the moved allocation until it knows whether the edited
layout converges. A stable pass restores it, preserving offscreen fragments and
their paragraph mappings. A nonstable pass retains the newly rendered registry.

The skipped-prefix predicate is unchanged. A stable pass copies only prefix cells
that actually need registration while keeping the complete saved registry. Other
passes consume prefix cells directly. Saving/restoring the full cell vector and
its paragraph lists no longer performs either whole-registry copy. Prefix replay,
mapping reconstruction and table resize metadata still have their existing costs.

## Verification

The fixed-seed property test exercises 96 combinations of column count, physical
page fragments, and incremental reset positions. It checks allocation ownership,
paragraph mappings, selected cells, navigation, physical hit targets, and text
bounds after an overlapping partial registration is replaced by the saved
registry. A separate test freezes the registry to protect bitmap hit targets.
An actual multi-page renderer test edits the first paragraph and checks stable
convergence, retained later pages, unchanged offscreen table geometry and hit
targets, and reuse of the original cell allocation.

Before the ownership implementation, a clone-and-clear prototype of the new API
fails the allocation property with one column and one fragment. Replacing that
copy with a move passes the property and all 713 layout library tests (12
preexisting ignored tests). The editor library passes all 2,392 tests (21
preexisting ignored tests), including the new renderer regression. Both crates
pass Clippy across all targets with warnings denied.

```sh
cargo +1.93.1 fmt --all -- --check
cargo +1.93.1 test -p jade-docx-layout --lib --offline
cargo +1.93.1 clippy -p jade-docx-layout -p jade-docx-editor --all-targets --offline -- -D warnings
cargo +1.93.1 test -p jade-docx-editor --lib --offline
```

WASM integration and an alternating baseline/candidate profile remain necessary
before claiming an elapsed improvement. Headless synthetic composition does not
establish native macOS IME or GPUI/CEF performance.
