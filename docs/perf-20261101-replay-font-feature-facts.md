# Reuse immutable font-feature facts during glyph replay

The Callgrind artifact for core #352 (source
`ef61f12348af4eeca5eab9003256c5f242c8720d`, run `41285125300`)
records 21,884,827 instructions in the Yokohama keystroke scenario.
`jade_painter::font_features::has_features` accounts for 674,200 exclusive
instructions, or 3.08%. This identifies repeated work in glyph replay; it does
not measure browser or native input latency.

`FontStateTracker` now retains the last `Rc<str>` font descriptor and the result
of the existing feature predicate. Repeated glyph runs using that allocation
reuse the fact. A different allocation still invokes the original parser, even
when its contents happen to be equal. Owning the descriptor prevents an address
from being recycled into a false cache hit. Retention is bounded to one
descriptor, and copy-on-write mutations get a different allocation.

Canvas state invalidation does not invalidate an immutable string's feature
fact. Font application, glyph positions, shaping, fallback painting and the
feature parser are unchanged. This cache holds no document reference and adds
no scheduling, bitmap or layout rule.

The regression replays 100 two-character lines for both ordinary and featured
fonts. It requires all 200 visible characters and exact equality of every
recorded painter operation against a fresh tracker per line, while reducing
the replay eligibility predicate from 100 checks to one. The original
implementation fails with 100 checks rather than one. A 96-case property test
compares against the uncached parser across canvas invalidation, tracker
cloning/reset, fresh equal allocations and copy-on-write case changes. A
retention test checks eviction and final release through weak references.

The deterministic predicate reduction is not an end-to-end timing claim. Use
the exact PR's Callgrind comparison and browser/native measurements before
claiming overall performance gains. Word fixtures, fonts, references,
tolerances, seeds and retry policy are unchanged.

Validation on core `06e1d5239ce254c2e190a82fe9188f69d8010cbe`: editor library
tests passed (2,404 passed; 21 existing ignored), Clippy for library/tests with
warnings denied passed, Rustfmt passed, and the locked wasm32 editor check
passed with Rust 1.93.1.
