# jade-skia-painter

A [`jade_painter::Painter`] backend that rasterises with Skia — the engine
Chromium's own canvas is built on.

`RecordingPainter` proves the drawing code *runs* without a browser. This proves
the trait carries enough information to produce pixels, and that those pixels
match what the browser draws.

## Not part of the workspace

CI runs `cargo test --all --all-features`, so a feature flag would not keep Skia
out of everyone's builds. The crate is listed under `[workspace] exclude`
instead and is built from its own directory:

```
cd crates/jade-skia-painter
cargo test
```

`cargo metadata` on the workspace does not list this crate, and `skia-safe` does
not appear in the workspace dependency graph at all.

## What the tests establish

`tests/renders_draw_commands.rs` runs `jade-docx-editor`'s draw-command executor —
unmodified, the same one the browser replays cached paragraphs through —
against Skia and asserts the resulting pixels.

`tests/chromium_raster_parity.rs` compares a fixed scene against
`tests/fixtures/chromium-scene.png`, which Chromium produced from the same
drawing calls. Measured on that scene:

| region | pixels differing by more than 8 levels |
| --- | --- |
| fills, strokes, background | **0.00%** |
| text | 1.88% |

Glyph ink starts and ends on the same column in both, so placement is identical
to the pixel; the text-band difference is antialiasing, which Chromium tunes
(gamma and contrast) on top of Skia. The three tests assert those claims
separately rather than hiding both under one loose tolerance.

To re-record the reference after changing the scene:

```
node tests/chromium-scene.mjs tests/fixtures/chromium-scene.png
```

## Known gaps

- Shadows (`shadowColor`/`shadowBlur`/`shadowOffset*`) are accepted and ignored.
- Images are not in the `Painter` trait yet, so nothing here draws them.
- Font resolution handles the CSS shorthand jade emits (`bold`/`italic`, `<n>pt`,
  a family list); it is not a general CSS font parser.
