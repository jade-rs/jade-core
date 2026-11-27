# Inline picture effect extents (not yet captured)

`wp:effectExtent` (ECMA-376 Part 1 §20.4.2.6) is the extra extent added to
each edge of a drawing's `wp:extent` to make room for its effects: shadows,
outlines, glow and the bounding box of a rotation. The renderer reserves
`(cx + l + r)` by `(cy + t + b)` for an inline picture and draws it inset by
`(l, t)`. Square, tight, through and top-and-bottom wrapping keep text clear of
the same enlarged box around a floating picture. Edges are clamped at 0, and
the extent is added as plain insets; rotation is laid out from the picture's
own transform, as before (including Word for Mac's quarter-turn swap).

These two probes are synthetic. They have not been opened in Word, and no
Word coordinates have been captured from them. Until they are, the unit tests
in `crates/jade-docx-editor/src/canvas_renderer/effect_extent_tests.rs` cover
the behaviour.

- `inline-shadow-effect-extent.docx`: a 40 by 30 pt picture with an outer
  shadow between two lines of text.
- `inline-rotation-30-effect-extent.docx`: a 40 by 20 pt picture rotated 30
  degrees, with an effect extent equal to its rotated bounding box's growth.
  That value is a guess at what Word writes, not an observation.

Still unverified against Word, and needing captures:

- pictures rotated 30, 60 and 90 degrees: what Word writes into
  `wp:effectExtent` and how much room it reserves (it may measure from the
  swapped box between 45 and 135 degrees);
- a shadowed inline picture: whether the line grows by exactly `t + b` and
  the next glyph moves by `l + r`;
- where `b` sits relative to the baseline (above it, or hanging below it as
  Word for Mac does in a line of pictures at a minimum height).

Regenerate them with:

```sh
python3 tests/word-oracle/effect-extent-probes/generators/make-effect-extent-probes.py tests/word-oracle/effect-extent-probes
```

To promote them, capture each one as described in `../mac-elsa-synthetic/README.md`
(Word for Mac, Save As → PDF → Best for printing) and in `../windows/README.md`
(Windows drawing origins). Check how far the text after each picture and the
next line move.
