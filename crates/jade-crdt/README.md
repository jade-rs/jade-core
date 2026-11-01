# jade-crdt

Experimental, opt-in document synchronization primitives backed by Loro 1.17.1.
This crate does not change the editor's default storage or enable networking.
The host owns transport, persistence, XML bindings, payload schemas and legal
amendment/effective-date policy.

## Identity and persistence

Create a document once, persist its snapshot, then open every replica from that
snapshot. Independently creating identical text does not create shared history.
Envelopes bind snapshots, updates, versions and anchors to document, baseline and
genesis identities; malformed, incomplete or incompatible imports are rejected.
These checks are not authentication. Hosts must authenticate their transport.
Snapshots persist content and facts; local Undo history is session-local.

Structured nodes have stable, unique IDs, ordered children, string attributes and
text. Deleted nodes remain tombstones, and their IDs cannot be reused. Text ranges
and anchors use UTF-8 byte offsets. Anchors follow concurrent edits. Local Undo
is selective; imported remote edits remain. Immutable causal facts share the sync
history but are not removed by text Undo; their meaning belongs to the host.
Opaque binary parts can be represented by the host, but the crate does not provide
semantic merging for them.

## Incremental capture

`Document::reconcile` accepts a complete visible forest. For frequent local edits,
`Document::reconcile_subtrees` accepts disjoint `(expected, next)` subtree pairs.
It reads and compares only those subtrees before planning granular operations.
Scope roots retain identity and position. To insert, delete or move a scope root,
capture its surviving common parent instead. Overlapping or stale scopes fail
before text, facts, sync frontiers or local Undo change. All scopes and supplied
facts form one transaction. Structural operations are validated on a fork and
replayed locally to preserve Undo; text-only edits reuse the stable-ID index.
Import and Undo/Redo invalidate that index.

The XML reader's optional `DocxXmlDocument::track_changes` / `changed_elements`
API can identify native edit scopes. Start an epoch before editing and acknowledge
it only after successful reconciliation. Reading is non-destructive, so capture
can be retried. Mutation marks the borrowed element's whole subtree; structural
insert/remove operations mark their parent. Removed elements are omitted from
reported IDs. Change tracking is independent of rollback journals and survives a
checkpoint clone. The host maps native IDs to stable CRDT IDs and chooses scope
boundaries. This crate deliberately has no editor/OOXML adapter dependency.

Unknown editor commands or package-wide replacements should use full capture.
Scheduling/coalescing captures and flushing before save, Undo/Redo, import and IME
preview are host responsibilities. There is no background thread in this crate.

## Validation

```
cargo test --locked -p jade-crdt
cargo test --locked -p jade-docx-reader --lib
```

The suites cover concurrent imports, duplicate delivery, selective Undo, anchors,
atomic rejection, identity checks, causal graphs and structured reconciliation.
A property test compares scoped and full reconciliation through random Unicode
text, attributes, kind changes, insertion, removal and reordering; every step
checks snapshot reopen and Undo/Redo. A reader property test reconstructs XML from
reported dirty scopes across structural edits and journal rollback.
