# Skip anchor detection while comment decorations are hidden

The named SDK 0.0.72 candidate trace for 32 growing preedits on a 300-page
annotated document attributes 2.84375 WASM leaf samples per update to
`blocks_contain_comment_anchors` in its first run. This is a diagnostic sample
count, not CPU milliseconds or an end-to-end speed claim. That trace predates
the same-immutable-document cache in core #252. Edits still invalidate that cache.

All consumers of `document_has_comments` in the renderer also require
`options.render_comments`. Hidden comments therefore do not require an anchor
scan. The renderer now retains a known answer for the same immutable snapshot,
but keeps an unscanned snapshot unknown. Re-enabling comments scans before
choosing seek/cache paths. Document edits, cache drop/restore and auxiliary-story
setters retain their existing invalidation rules. No caller option defaults or
Word comment data change.

The focused regression renders 3,900 body paragraphs followed by a comment
marker. Hidden renders perform zero anchor scans (the original implementation
performs one). Enabling comments performs one scan and exposes the marker.
Removing it during a hidden edit performs no extra scan; enabling comments
again performs another scan and removes the marker annotation.

The stateful property covers 96 generated sequences of 1–31 operations across
15 main/note/header/footer stories, three anchor forms plus absent anchors,
metadata presence, hidden/visible rendering, mutation, immutable document
replacement, cache drop/restore and unchanged rerender. It compares anchor
presence, cache eligibility, visible metadata and scan counts after every step.
The saved original-code counterexample is a single hidden render after a body
anchor mutation. Hidden comment metadata is cleared, matching the existing
public behavior; it is restored from the supplied store when enabled.

This optimization applies to `render_comments = false`. The editor defaults to
showing Word comments. No native IME latency reduction or default-mode wall-clock
improvement is claimed. Existing visible-comment behavior and Word coordinate
baselines remain separately gated.
