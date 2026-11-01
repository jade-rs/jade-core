# jade-docx-crdt

Opt-in OOXML adapter for the sibling `jade-crdt` engine. It owns stable native XML
bindings, package projection, incremental dirty-scope capture and snapshot rebind.
The editor storage and network transport remain opt-in host responsibilities.

`LiveDocument::create` bootstraps a package; persist `inner.snapshot()` and use
`LiveDocument::open` for subsequent sessions. `capture_changed` consumes the reader’s
non-destructive change-tracking epoch without serializing a ZIP or traversing
unchanged paragraphs. A false result requests full `capture`; acknowledge the XML
epoch only after successful capture. Commands replacing package-wide resources
must call `require_full_capture`. `nodes::prepare_nodes` provides validated,
namespace-aware persistent identities and rejects invalid metadata atomically.

`with_fact_policy` injects deterministic, side-effect-free host metadata derivation.
During incremental capture it receives disjoint dirty scopes, not an entire forest.
Capture rejects policy errors before mutating the replica or native bindings.
Text changes and derived opaque facts share a transaction. Facts persist through
text Undo/Redo; the policy also runs after history changes. Legal amendment graphs,
effective dates, persistence formats and UI projection do not belong in this crate.

`worker/capture.ts` exports self-contained `createDeferredCrdt` and
`createCrdtReply` factories. A host may serialize these factories into an existing
worker bridge. They coalesce edit captures after rendering, invalidate stale jobs,
and flush before save, history, import and composition previews. Capture failure
retains pending edits and blocks dependent commands until the session is recovered.
The host supplies dispatch, capture, error, reply and yield callbacks.
`crdtWorkerBridge` and `crdtSdkBridge` also own the SDK source adaptation: command
routing, scheduler wiring, export flush, error delivery and bitmap transfers.
They require each supported SDK boundary to occur exactly once and reject drift
or repeated application. Hosts only import/re-export the adapters; no application
policy, storage format, or network transport is baked into the bridge.

This adapter uses Loro snapshots from `jade-crdt`. The separate
`jade-collaboration` repository uses an RGA/sequencer protocol and `DocxOp` model;
these are not compatible snapshot formats. This crate moves shared local editing
mechanics upstream without silently replacing either engine or enabling networking.

## Validation

```sh
cargo test --locked -p jade-docx-crdt
node --experimental-strip-types --test crates/jade-docx-crdt/worker/*.test.ts
# Install the version in formal/lean-toolchain via elan, then:
(cd crates/jade-docx-crdt/formal && lean Capture.lean)
JADE_LEAN_ORACLE=1 node --experimental-strip-types --test crates/jade-docx-crdt/worker/*.test.ts
```

Property tests compare incremental and full capture across Unicode text and
structural edits, snapshot reopen and Undo/Redo, and check persistent node identity
idempotency. Regression tests cover remote rebinding, opaque package parts,
metadata-only imports, atomic policy rejection and dirty-scope retries. Worker
tests cover coalescing, stale jobs, failures and save/history/IME barriers.

The state-machine PBT runs 128 reproducible seeds × 256 operations: Unicode input,
projected edits, rejected edits, timers, stale/repeated callbacks, yields, reads,
save/history/IME barriers and persistent capture failure. With `JADE_LEAN_ORACLE=1`
it also compares reference transitions against the executable `step` in
`formal/Capture.lean`. The dedicated CI proof job requires this comparison.

Lean proves that all finite event traces preserve the capture invariant; stale
jobs do nothing; staging/yielding do not capture; a successful save catches up to
every acknowledged edit; and failure preserves input and blocks further edits.
The proof models an active serialized worker with unbounded generation counters
and edit-batch counts. It does **not** prove the TypeScript runtime, OOXML/Loro
semantics, browser rendering, scheduler fairness or eventual capture. PBT links
observable worker behavior to the proved model on generated traces; it is not a
proof of implementation equivalence. The executable oracle and proofs use the
same transition function and have no extra package dependencies.
