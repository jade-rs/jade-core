# Reuse resolved styles while measuring paragraph font lines

The core #353 Callgrind profile for `keystroke-marks@yokohama` attributed
4,423,358 of 69,554,319 self instructions (6.36%) to applied run-style hash
keys. Calls from `paragraph_word_font_lines_pt` contributed 1,025,500
instructions across 9,476 style-cache resolutions. This motivated checking
whether that helper can reuse the existing identity cache for its already
shared effective run properties.

Both the blank-paragraph and distinct-run paths now call the existing
`resolve_applied_run_style_cached_rc` with the existing `Rc`. No new cache,
formatting rule or painter activation is introduced. The same bounded cache
retains the `Rc` and keys grid spacing. Style/theme changes clear the
effective-property cache, so subsequent resolutions receive fresh `Rc`
identities. Changes to the document OpenType feature mode clear both applied
style caches. A miss still resolves through the previous value-keyed cache.

An ignored diagnostic loads three existing DOCX fixtures, measures all
top-level body paragraphs with Mac Word layout rules and the document's
styles/theme, and hashes the exact optional f32 line-height bits. Each of
five samples performs 40 warmed full-paragraph passes. The source diagnostic
was run before the two production call-site changes, then run afterward
with Rust 1.93.1 and identical unoptimized build settings:

| Fixture | Paragraphs | Before, microseconds | After, microseconds | Reduction | Geometry hash |
| --- | ---: | ---: | ---: | ---: | --- |
| contract | 48 | 7,545 | 6,098 | 19.18% | aa452868fd380dc9 |
| nda | 54 | 11,379 | 7,433 | 34.68% | 54007beaf99fc570 |
| yokohama | 2,746 | 1,410,615 | 766,522 | 45.66% | ef29edca582ea1aa |

All three geometry hashes matched before and after. The diagnostic also
compares each sample's full result vector with its initial vector. These
figures cover this helper's warm-cache work on top-level paragraphs only;
nested tables and complete editor/native input latency are not measured.
Concurrent local builds limit wall-time precision. All 18 exact-head CI
Callgrind comparisons remain required before merge.

All 2,404 existing editor library tests passed (22 ignored, including the
new diagnostic). Frozen Word fixtures/reference observations, fonts,
coordinate tolerances, PBT seeds and zero-retry policies are unchanged.

Editor library/test Clippy with `-D warnings`, workspace formatting, and the
locked `wasm32-unknown-unknown` editor check also passed with Rust 1.93.1.
