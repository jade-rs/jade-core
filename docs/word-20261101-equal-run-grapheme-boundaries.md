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
source-offset correspondence. The original implementation fails the width
regression. Word source fixtures, fonts, reference observations, tolerance,
seeds and CI retry policy are unchanged. No performance or native input
latency improvement is claimed.

Validation on core `06e1d5239ce254c2e190a82fe9188f69d8010cbe`: combined
locked library tests passed (2,401 editor and 721 layout tests; 21 and 12
existing ignored tests respectively). Clippy for the layout library and tests
with warnings denied and the workspace Rustfmt check passed with Rust 1.93.1.
The locked wasm32 editor library check also passed.
