# Borrow paragraph alignment tokens

Body and table line positioning call
`resolve_physical_paragraph_alignment`. It previously trimmed the token and
allocated a lowercase `String` on every call, even for the usual `left` value.
The resolver now compares the borrowed, trimmed token with ASCII case-insensitive
comparisons. The physical direction, accepted justification tokens, Unicode
whitespace trimming, and fallback for unknown values remain the same.

## Allocation regression

The isolated `alignment_allocations` integration test counts allocations on the
test thread only while resolving 16 nonempty tokens in both directions, 1,000
times. Inputs and outputs pass through `black_box`; initialization and reporting
are outside the counted region. The allocator forwards allocation, zeroed
allocation, reallocation and deallocation to `System`.

On macOS arm64 with Rust 1.93.1, in the unoptimized test profile:

| Resolver | Calls | Allocations |
| --- | ---: | ---: |
| Before, same new test | 32,000 | 32,000 (test failed) |
| Borrowed token | 32,000 | 0 (test passed) |

The zero-allocation assertion runs in CI so restoring lowercase allocation
fails the regression. This measures resolver allocations; it does not establish
an overall editor CPU, typing latency, native IME, CEF or GPUI improvement.

## Behavior regression

The fixed-seed property test uses 256 cases (seed `20261101`) with known and
unknown tokens, arbitrary Unicode text, mixed ASCII case, Unicode/ASCII
surrounding whitespace and both paragraph directions. An independent explicit
token table after the previous lowercase normalization supplies the expected
result. Existing Word references, tolerances, fixtures and wrapping rules are
unchanged.

```sh
cargo +1.93.1 test -p jade-docx-layout
cargo +1.93.1 clippy -p jade-docx-layout --all-targets -- -D warnings
cargo +1.93.1 fmt --all -- --check
```

Validation uses CLI and automated tests only; no computer use is required.
