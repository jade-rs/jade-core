# Revalidate a known comment-bearing body block before a full scan

The named SDK 0.0.73 diagnostic traces for 32 growing Japanese preedits on
300 pages attribute 2.0 and 1.8125 WASM leaf samples per update to comment-anchor
detection. These are diagnostic samples, not milliseconds. That fixture has
plugin decorations but no Word comment anchors: this change does **not** remove
its negative scan, and no improvement to that workload or native IME is claimed.

The related positive case can avoid repeatedly walking a long prefix. The
renderer remembers the index of a main-body block where a real comment anchor
was found. Before a new detection it recursively checks the block currently at
that index. Finding a current anchor proves the document has an anchor. A miss,
an absent body or an out-of-range index falls back to the complete existing scan,
including footnotes, endnotes, six document header/footer stories and every
section's six header/footer stories. A successful body scan updates the index;
a failed body scan clears it. Negative answers are never inferred from the index.

The index can survive cache drops, restoration, arbitrary edits and document
replacement because it never bypasses current-content validation. It stores no
Rc, Weak reference or raw address, so it does not change document ownership or
copy-on-write behavior. The existing same-immutable-Rc boolean cache is unchanged.
Comment metadata, visibility, conservative seek/cache eligibility and geometry
are unchanged.

## Deterministic regression

`validated_comment_body_witness_avoids_repeated_prefix_scans_after_edits` creates
3,900 plain body paragraphs followed by one anchor, separately for ranges,
markers and run references. Initial detection visits 3,901 top-level blocks.
Each of 32 subsequent cache-drop/text-edit/detection cycles visits one block;
the old implementation visits 3,901 and fails the budget assertion. Removing
the final anchor must immediately produce a negative answer.

This is a deterministic reduction in anchor-detection work, not a wall-clock or
end-to-end latency benchmark. A retained witness inside a very large table or
text box may still require scanning that entire top-level block. Moving or
removing the witness can add one failed block check before the full scan.

`comment_witness_tracks_structural_edits_and_replaced_stories` checks 128
generated histories of 1–64 operations against an independent sequence of
anchor-presence bits. Operations insert, delete, replace, rotate, truncate and
reverse blocks, replace the whole document, remove its body, drop/restore caches
and replace note/header/footer stories. Anchors occur directly, in text boxes
or in text boxes nested in table cells. Every step checks the complete result.
Existing tests additionally cover all 15 story kinds, hidden/visible comments,
metadata changes, unchanged immutable documents and window rendering versus a
full-render oracle after edits.

No Word fixture, coordinate reference, tolerance or compatibility gate changes.
