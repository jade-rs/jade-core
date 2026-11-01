# Reuse owned table resize metadata buffers during lazy edits

Retained 300-page composition CPU profiles show `Vec::clone` self samples below
`table_resize_metadata` on the preview path. Those recordings had renderer debug
logging enabled and a heavily loaded host; they identify an allocation site,
not normal-release or native macOS IME latency.

Four existing lazy-edit synchronization points now refresh the owned table
resize snapshot with `Vec::clone_from`: text insertion, composition preview,
history actions and paragraph insertion. `TableLayoutInfo::clone_from` also
reuses surviving tables' column-width and row-height vectors. Values still copy
in linear time; this change neither skips metadata refresh nor shares mutable
renderer storage with an earlier snapshot. The public owned getter and eager
render paths are unchanged.

Buffers retain capacity until their owner is dropped. Removing table entries
drops their nested vectors; subsequent additions can allocate again. There is
no fixed memory bound or claim that every edit becomes allocation-free.

A regression was run before implementation and failed because the derived
`clone_from` replaced nested allocations. Pointer regressions now check reuse
through size reductions and empty metadata. A 96-case property test compares
variable-length metadata transitions with serialization of the source fields,
then mutates the renderer to check snapshot isolation. Its fixed seed is
`PROPTEST_RNG_SEED=20261104`.

Formatting, the full editor library suite (2395 passed, 21 existing ignored),
and all-target Clippy with warnings denied passed. Eight actual-WASM headless
cases passed with zero retries: four composition cases (including 300 pages),
three paragraph-insertion cases and one table metadata transition case. The
complete table metadata attachment, including IDs and geometry through preview,
commit, paragraph insertion, undo and redo, exactly matches the original
metadata implementation.

The first metadata integration assertion incorrectly required an ID absent
from the initial eager snapshot to remain absent after lazy editing. That
assertion also failed against the original implementation. The corrected test
checks unchanged geometry, a valid ID once assigned, and stable subsequent IDs;
it passed against both builds. The initial failure is retained in local evidence.

No new timing experiment is part of this change, and no native or release speed
claim is made. Existing Word references and tolerances are unchanged.
