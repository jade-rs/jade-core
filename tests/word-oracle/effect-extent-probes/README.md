# Inline picture effect extents (not yet captured)

`wp:effectExtent` (ECMA-376 Part 1 §20.4.2.6) is the extra extent added to
each edge of a drawing's `wp:extent` to make room for its effects: shadows,
outlines, glow and the bounding box of a rotation. The renderer reserves
`(cx + l + r)` by `(cy + t + b)` for an inline picture and draws it inset by
`(l, t)`. Square, tight, through and top-and-bottom wrapping keep text clear of
the same enlarged box around a floating picture.

These two probes are synthetic. They have not been opened in Word, and no
Word coordinates have been captured from them. Until they are, the unit tests
in `crates/jade-docx-editor/src/canvas_renderer/effect_extent_tests.rs` cover
the behaviour.

- `inline-shadow-effect-extent.docx`: a 40 by 30 pt picture with an outer
  shadow between two lines of text.
- `inline-rotation-30-effect-extent.docx`: a 40 by 20 pt picture rotated 30
  degrees, with the effect extent of its rotated bounding box.

Regenerate them with:

```sh
python3 tests/word-oracle/effect-extent-probes/generators/make-effect-extent-probes.py tests/word-oracle/effect-extent-probes
```

To promote them, capture each one as described in `../mac-elsa-synthetic/README.md`
(Word for Mac, Save As → PDF → Best for printing) and in `../windows/README.md`
(Windows drawing origins). Check how far the text after each picture and the
next line move.
