# Preserve graphemes across identical run boundaries

A producer can split an extended grapheme across consecutive OOXML runs even
when every run has identical source metadata. The line wrapper previously
segmented each run separately. A fail-first regression measured a family ZWJ
emoji as seven clusters (70 units with a 10-unit test context), whereas the
same text in one run measured one cluster (10 units). Combining accents,
regional-indicator flags and halfwidth katakana with a voiced mark exhibit
the same class of defect.

The shared line wrapper now temporarily joins a complete extended grapheme
across contiguous, equal-metadata run ranges. Horizontal and vertical layout
use the same rule. Ordinary boundaries borrow the input slice without an
allocation. Source text, OOXML runs, byte offsets, hyperlinks and revision
metadata are retained; the editor document is not rewritten.

The rule requires exact equality of run properties (including provenance),
revision marks and hyperlinks. Inline objects and run-local fitText, ruby and
East Asian layout annotations remain separate. Horizontal paragraphs with
ADVANCE fields retain their original ranges. Incomplete clusters and clusters
with differing metadata remain unchanged. In particular, this does not claim
to resolve the existing frozen Unicode Word fixture, whose ZWJ runs have
different font properties.

Regression coverage compares one run with per-scalar runs, checks metadata
and source range preservation, and requires ordinary and excluded boundaries
to borrow the original slice. A 128-case property test generates arbitrary
run partitions and wrap widths, requiring identical horizontal and vertical
cluster geometry and line/column boundaries, exact reconstructed text and
source-offset correspondence. A second 128-case property test compares mixed
metadata against an independent whole-grapheme oracle, including partial
joins that must be rejected. A 1,025-scalar regional-indicator regression
retains exact flag pairing across a long prefix. The original implementation
fails the width regression. Word source fixtures, fonts, reference observations, tolerance,
seeds and CI retry policy are unchanged. No application or native input
latency improvement is claimed.

Pre-merge scaling review found that creating a fresh GraphemeCursor for every
run boundary rescanned regional-indicator prefixes quadratically. The joiner
now retains a forward grapheme iterator, including enough context for a
rejected partial join to revisit the current cluster. ASCII boundaries still
avoid segmentation, and ordinary ranges still borrow without an allocation.
In an isolated local benchmark of the actual extracted join helper (same Rust
1.93.1, dependency and build settings on both sides; median of five runs),
1,000/2,000/4,000/8,000 scalar runs took 3.14/12.21/48.29/231.24 ms before
and 0.031/0.060/0.119/0.235 ms after. Each result retained the same flag-pair
run ranges. These are helper measurements, not application or input-latency
measurements.

The first forward-iterator CI profile (`8c77533`) detected a 1.3105% increase
in Yokohama's complete layout instruction count. It segmented ordinary CJK
prefixes even when only two distant run boundaries were queried. That head
was held back despite all six CI checks passing. The joiner now reuses nearby
iterator boundaries and asks a GraphemeCursor about at most 16 bytes of left
context for distant boundaries. Missing Unicode context falls back to the
same retained iterator; it is never guessed. Long regional-indicator, Indic
linker and emoji/Extend/ZWJ sequences remain exact, including sparse run
ranges beyond the local window. Mixed-metadata PBT also covers Indic text,
CR/LF and repeated combining marks. Ordinary ranges still borrow the input.

A helper-only diagnostic (same Rust 1.93.1, dependency, optimized harness and
five-sample median) compared the held head with this bounded-context change.
For 1,000 calls over 8,000 CJK characters in two runs, cumulative time was
96.0 ms versus 0.339 ms. The 8,000-scalar RI case was 0.246 ms versus 0.269 ms
and retained exact flag-pair ranges. Concurrent local builds limit timing
precision; this diagnostic is not a complete-layout or application latency
claim. Updated-head CI and all 18 Callgrind comparisons remain required.

Validation on core `06e1d5239ce254c2e190a82fe9188f69d8010cbe`: combined
locked library tests passed (2,401 editor and 721 layout tests; 21 and 12
existing ignored tests respectively). Clippy for the layout library and tests
with warnings denied and the workspace Rustfmt check passed with Rust 1.93.1.
The locked wasm32 editor library check also passed.

Integrated validation on core `0894c50b105d79d09ead7a1e7989945d7d788fbb`
(including the merged replay font-feature cache and XLSX marker changes):
locked library tests passed (2,404 editor and 723 layout tests; 21 and 12
existing ignored tests). Layout library/tests Clippy with warnings denied,
workspace Rustfmt and the locked wasm32 editor library check all passed with
Rust 1.93.1. Both 128-case grapheme property tests passed in this run.

Bounded-context follow-up validation on the same `0894c50` base: all 724 layout
library tests passed (12 existing ignored), including both 128-case PBTs and
the sparse long-context regression. Layout Clippy with warnings denied and
the workspace Rustfmt check passed. Updated-head CI and performance artifacts
are still required before merge; the earlier all-green head was not merged.
