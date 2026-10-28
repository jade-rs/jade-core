# Late-page IME composition previews

On SDK 0.0.60, a synthetic 300-page document walks 3,900 body elements for
every uncommitted Japanese replacement on its last page. The ink guard compares
cached line boxes translated from absolute document coordinates against ink
computed directly in page-local coordinates. A 3.8e-11 CSS-pixel difference at
the bottom edge falsely triggers band escalation, whose conservative full
repaint rejects the visible-window seek and walks the prefix again.

The guard now absorbs 1e-7 CSS pixels of numerical roundoff. Band clips already
expand vertically by a full device pixel; real overflow, nonfinite ink and
geometry changes retain their existing escalation. Composition uses its
existing band policy on every page. The fix does not relax seek validation.

A genuinely wrapping/shrinking preview must still repaint the full visible
window. Its retry now carries only the seek snapshot consumed by the first
band pass and revalidates its geometry/layout anchor, visible-start boundary
and position at or before the edited paragraph. A failed check retains the
from-start fallback. This exception is one-shot and scoped to band escalation;
ordinary full-layout renders retain their policy, and pagination is not frozen.

A deterministic Rust test records the measured page-300 geometry and fails on
the old strict comparison. A property test varies absolute offsets through
100 million CSS pixels and proves both roundoff acceptance and rejection of
real top/bottom overflow. Existing exact-spacing and unmeasurable-ink tests
remain mandatory.

The companion SDK/Elsa tests cover active-generation, nonblank, exact
from-start pixel parity, preview history, cancellation and one complete saved
commit with zero retries. Profiling compares 1-, 10- and 300-page documents;
CPU traces are collected separately and normalized per preview update.

Two band-suppression approaches were rejected: disabling all lazy-preview
bands doubled one-page worker time, and disabling only late-page bands lost
cheap partial paints on a 10-page control. The numerical fix preserves bands
instead. Physical macOS IME and GPUI/CEF transport need separate native checks.

## Persistent proofreading marks

The 300-page Elsa regression with persistent marks exposed a second full-prefix
walk after each preview: `applyDecorationResults` invalidated paragraph drawing
commands and repainted from the start. A missing draw-command cache caused full
pixel damage even though layout anchors remained valid. Restricting the seek to
resolved dirty rectangles therefore did not fix this path (3,900 body elements).

Decoration repaint now freezes unchanged selection and pagination, retains full
pixel invalidation when rectangles are unavailable, and requests the existing
validated visible-window seek. It draws fresh visible content rather than replaying
prefix commands. Unsupported or stale anchors retain the from-start fallback;
offscreen pixels refresh on demand. The Rust regression covers both missing
reverse-index entries and indexed paragraphs with no drawing cache, and rejects a
stale page-width anchor. The Elsa regression checks replacement, wrapping,
shrinking, cancellation, undo history, saved text, annotation pixels and exact
parity with an independent from-start repaint.
