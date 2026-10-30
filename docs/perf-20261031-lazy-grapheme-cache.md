# Allocate draw-command grapheme caches on demand

A Chromium CPU sampling trace of 32 unique IME preedit updates in an annotated
300-page document with formatting marks enabled included this editor-worker
allocation stack:

```
Dlmalloc::malloc
  <- __rdl_alloc
  <- hashbrown::RawTableInner::new_uninitialized
  <- draw_command::executor::execute_command
  <- CanvasRenderer::draw_whitespace_marks
  <- render_text_line
```

The trace used SDK CI run 41110592875, artifact `e2e-build` 13003308407,
SDK commit `f131a2aed877ed0b5b137b4fbdb91e9c1bc70fcc`, core
`b1b3245664630561dd5c4f9f40ee7ac9c97e1ed5`, and WASM SHA-256
`88d4f52ee22eadaa1b1ad330540b72b83722fb7fbed52b9b985af5385d323345`.
This is an allocation-path observation, not an allocation count, CPU-time
measurement, or native latency comparison. The CI artifact uses `wasm-dev`;
production latency needs a matched release-build comparison.

`execute_command` creates a fresh `FontStateTracker` for every command.
Previously its constructor immediately reserved the 2,048-entry cache limit,
which the current hash table rounds up to capacity 3,584. Whitespace marks
usually measure only one symbol. Shape-only commands do not measure any text.
The existing derived `Default` already starts with no allocation.

Use that default in `new`, growing the cache only on a short-text cache miss.
The cache key, measured values, 2,048-entry limit, eviction, invalidation,
font selection and drawing commands are unchanged. Long strings still bypass
the cache. A newly created tracker has capacity zero; after one measured symbol
its capacity is below 16. Large replay sessions may incur incremental growth
allocations, a tradeoff against reserving the maximum for every command.

Test-first validation:

- The constructor/long-string/single-mark allocation regression fails before
  the fix: initial capacity is 3,584 rather than zero.
- The 96-case stateful property test also fails before the fix, shrinking to
  a single tracker replacement. Retain its seed for replay.
- The property model tracks measured `(font, text)` pairs independently across
  font switches, canvas-state invalidation, cache clearing and replacement.
  Check returned widths, actual backend measurement counts and cache length
  after each operation. Long strings remain uncached.
- A separate 4,096-measurement regression checks the entry limit after every
  insertion and a hit for the final inserted entry after eviction.

These deterministic checks establish the allocation reduction and cache
semantics. They do not establish a change in end-to-end IME frame latency.

Local verification (Rust 1.93.1, macOS, unchanged fixture/font gates):

```sh
cargo test -p jade-docx-editor --all-features --lib
cargo clippy -p jade-docx-editor --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --manifest-path crates/jade-docx-native-render/Cargo.toml --locked --test word_mac_coordinates -- --test-threads=1
```

The editor suite passes 2,338 tests (21 existing ignored). Warnings-denied
Clippy and formatting pass. The native Word coordinate target passes five
checks; the Office Arial-specific check remains explicitly ignored without
that captured font. No fixture, tolerance or expected coordinate changes.
