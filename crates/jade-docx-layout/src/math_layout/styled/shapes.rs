//! Stretchy characters drawn as paths, for a host with no math face bound.
//!
//! Word draws a delimiter around a tall fraction, a radical sign over a long
//! base, a brace under a sum and a wide hat from Cambria Math's glyph variants
//! and assemblies. Without the face those glyphs are not available, so the
//! layout draws the same shapes from strokes and curves at the size the
//! content needs.
use super::MathPathCommand::{self, *};

/// Whether `ch` has a drawn form when it has to stretch vertically.
pub(crate) fn has_vertical(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '|'
            | '│'
            | '∣'
            | '‖'
            | '∥'
            | '⟨'
            | '⟩'
            | '〈'
            | '〉'
            | '⌈'
            | '⌉'
            | '⌊'
            | '⌋'
            | '⟦'
            | '⟧'
            | '/'
            | '\\'
            | '∫'
    )
}

/// A delimiter `height` tall and `width` wide with its top-left corner at
/// (`x`, `top`). Returns the path and whether it is filled (else stroked).
pub(crate) fn vertical(
    ch: char,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    thickness: f64,
) -> Option<(Vec<MathPathCommand>, bool)> {
    let left = x + width * 0.15;
    let right = x + width * 0.85;
    let bottom = top + height;
    let middle = top + height / 2.0;
    let mirror = |commands: Vec<MathPathCommand>| -> Vec<MathPathCommand> {
        let axis = x + width / 2.0;
        commands
            .into_iter()
            .map(|c| match c {
                Move(a, b) => Move(2.0 * axis - a, b),
                Line(a, b) => Line(2.0 * axis - a, b),
                Quad(a, b, c, d) => Quad(2.0 * axis - a, b, 2.0 * axis - c, d),
                Cubic(a, b, c, d, e, f) => {
                    Cubic(2.0 * axis - a, b, 2.0 * axis - c, d, 2.0 * axis - e, f)
                }
                Close => Close,
            })
            .collect()
    };
    let paren = || {
        // A crescent: the outer curve down, the inner one back up.
        let bulge = (width * 0.7).max(thickness * 2.0);
        let inner = left + (thickness * 1.6).min(width * 0.35);
        vec![
            Move(right, top),
            Cubic(
                right - bulge,
                top + height * 0.18,
                right - bulge,
                bottom - height * 0.18,
                right,
                bottom,
            ),
            Line(right, bottom - thickness * 0.2),
            Cubic(
                inner + (right - bulge - left) * 0.5,
                bottom - height * 0.22,
                inner + (right - bulge - left) * 0.5,
                top + height * 0.22,
                right,
                top + thickness * 0.2,
            ),
            Close,
        ]
    };
    let bracket = || {
        vec![
            Move(right, top + thickness / 2.0),
            Line(left + thickness / 2.0, top + thickness / 2.0),
            Line(left + thickness / 2.0, bottom - thickness / 2.0),
            Line(right, bottom - thickness / 2.0),
        ]
    };
    let brace = || {
        let mid_x = x + width * 0.5;
        let tip = left;
        let curl = (height * 0.08).min(width * 1.2);
        vec![
            Move(right, top + thickness / 2.0),
            Cubic(mid_x, top, mid_x, top + curl * 0.3, mid_x, top + curl),
            Line(mid_x, middle - curl),
            Cubic(
                mid_x,
                middle - curl * 0.3,
                tip + (mid_x - tip) * 0.3,
                middle,
                tip,
                middle,
            ),
            Cubic(
                tip + (mid_x - tip) * 0.3,
                middle,
                mid_x,
                middle + curl * 0.3,
                mid_x,
                middle + curl,
            ),
            Line(mid_x, bottom - curl),
            Cubic(
                mid_x,
                bottom - curl * 0.3,
                mid_x,
                bottom,
                right,
                bottom - thickness / 2.0,
            ),
        ]
    };
    let angle = || vec![Move(right, top), Line(left, middle), Line(right, bottom)];
    let ceiling = || {
        vec![
            Move(right, top + thickness / 2.0),
            Line(left + thickness / 2.0, top + thickness / 2.0),
            Line(left + thickness / 2.0, bottom),
        ]
    };
    let floor = || {
        vec![
            Move(left + thickness / 2.0, top),
            Line(left + thickness / 2.0, bottom - thickness / 2.0),
            Line(right, bottom - thickness / 2.0),
        ]
    };
    let center = x + width / 2.0;
    Some(match ch {
        '(' => (paren(), true),
        ')' => (mirror(paren()), true),
        '[' => (bracket(), false),
        ']' => (mirror(bracket()), false),
        '{' => (brace(), false),
        '}' => (mirror(brace()), false),
        '⟨' | '〈' => (angle(), false),
        '⟩' | '〉' => (mirror(angle()), false),
        '⌈' => (ceiling(), false),
        '⌉' => (mirror(ceiling()), false),
        '⌊' => (floor(), false),
        '⌋' => (mirror(floor()), false),
        '|' | '│' | '∣' => (vec![Move(center, top), Line(center, bottom)], false),
        '‖' | '∥' => {
            let gap = (width * 0.18).max(thickness * 1.5);
            (
                vec![
                    Move(center - gap, top),
                    Line(center - gap, bottom),
                    Move(center + gap, top),
                    Line(center + gap, bottom),
                ],
                false,
            )
        }
        '⟦' => {
            let mut c = bracket();
            c.extend([
                Move(left + thickness * 2.5, top + thickness / 2.0),
                Line(left + thickness * 2.5, bottom - thickness / 2.0),
            ]);
            (c, false)
        }
        '⟧' => {
            let mut c = bracket();
            c.extend([
                Move(left + thickness * 2.5, top + thickness / 2.0),
                Line(left + thickness * 2.5, bottom - thickness / 2.0),
            ]);
            (mirror(c), false)
        }
        '/' => (vec![Move(left, bottom), Line(right, top)], false),
        '\\' => (vec![Move(left, top), Line(right, bottom)], false),
        '∫' => {
            // A long s: hooks at both ends of a slanted stem.
            let hook = width * 0.25;
            (
                vec![
                    Move(right + hook * 0.2, top + hook),
                    Cubic(
                        right,
                        top,
                        right - hook,
                        top,
                        right - hook * 1.6,
                        top + hook * 1.5,
                    ),
                    Line(left + hook * 1.6, bottom - hook * 1.5),
                    Cubic(
                        left + hook,
                        bottom,
                        left,
                        bottom,
                        left - hook * 0.2,
                        bottom - hook,
                    ),
                ],
                false,
            )
        }
        _ => return None,
    })
}

/// A horizontal character (a brace over or under, an arrow, a bar, a wide
/// accent) `width` wide and `height` tall from (`x`, `top`). Returns the path
/// and whether it is filled.
pub(crate) fn horizontal(
    ch: char,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    thickness: f64,
) -> Option<(Vec<MathPathCommand>, bool)> {
    let right = x + width;
    let bottom = top + height;
    let middle = top + height / 2.0;
    let center = x + width / 2.0;
    let flip = |commands: Vec<MathPathCommand>| -> Vec<MathPathCommand> {
        let axis = top + height / 2.0;
        commands
            .into_iter()
            .map(|c| match c {
                Move(a, b) => Move(a, 2.0 * axis - b),
                Line(a, b) => Line(a, 2.0 * axis - b),
                Quad(a, b, c, d) => Quad(a, 2.0 * axis - b, c, 2.0 * axis - d),
                Cubic(a, b, c, d, e, f) => {
                    Cubic(a, 2.0 * axis - b, c, 2.0 * axis - d, e, 2.0 * axis - f)
                }
                Close => Close,
            })
            .collect()
    };
    // An over-brace: ends down, a point up in the middle.
    let brace = || {
        let curl = (width * 0.06).min(height * 1.2);
        vec![
            Move(x, bottom),
            Cubic(x, middle, x + curl * 0.3, middle, x + curl, middle),
            Line(center - curl, middle),
            Cubic(center - curl * 0.3, middle, center, middle, center, top),
            Cubic(
                center,
                middle,
                center + curl * 0.3,
                middle,
                center + curl,
                middle,
            ),
            Line(right - curl, middle),
            Cubic(right - curl * 0.3, middle, right, middle, right, bottom),
        ]
    };
    let bracket = || {
        vec![
            Move(x + thickness / 2.0, bottom),
            Line(x + thickness / 2.0, top + thickness / 2.0),
            Line(right - thickness / 2.0, top + thickness / 2.0),
            Line(right - thickness / 2.0, bottom),
        ]
    };
    let paren = || {
        vec![
            Move(x, bottom),
            Cubic(
                x + width * 0.2,
                top,
                right - width * 0.2,
                top,
                right,
                bottom,
            ),
        ]
    };
    let head = (height * 0.9).min(width * 0.4);
    let arrow_right = |y: f64| {
        vec![
            Move(x, y),
            Line(right, y),
            Move(right - head, y - head * 0.6),
            Line(right, y),
            Line(right - head, y + head * 0.6),
        ]
    };
    let arrow_left = |y: f64| {
        vec![
            Move(right, y),
            Line(x, y),
            Move(x + head, y - head * 0.6),
            Line(x, y),
            Line(x + head, y + head * 0.6),
        ]
    };
    Some(match ch {
        '⏞' => (brace(), false),
        '⏟' => (flip(brace()), false),
        '⎴' => (bracket(), false),
        '⎵' => (flip(bracket()), false),
        '⏜' => (paren(), false),
        '⏝' => (flip(paren()), false),
        '→' | '\u{20D7}' | '⟶' => (arrow_right(middle), false),
        '←' | '\u{20D6}' | '⟵' => (arrow_left(middle), false),
        '↔' | '\u{20E1}' | '⟷' => {
            let mut c = arrow_right(middle);
            c.extend([
                Move(x + head, middle - head * 0.6),
                Line(x, middle),
                Line(x + head, middle + head * 0.6),
            ]);
            (c, false)
        }
        '⇒' | '⟹' => {
            let gap = (height * 0.18).max(thickness);
            (
                vec![
                    Move(x, middle - gap),
                    Line(right - head * 0.4, middle - gap),
                    Move(x, middle + gap),
                    Line(right - head * 0.4, middle + gap),
                    Move(right - head, middle - head * 0.7),
                    Line(right, middle),
                    Line(right - head, middle + head * 0.7),
                ],
                false,
            )
        }
        '⇐' | '⟸' => {
            let gap = (height * 0.18).max(thickness);
            (
                vec![
                    Move(right, middle - gap),
                    Line(x + head * 0.4, middle - gap),
                    Move(right, middle + gap),
                    Line(x + head * 0.4, middle + gap),
                    Move(x + head, middle - head * 0.7),
                    Line(x, middle),
                    Line(x + head, middle + head * 0.7),
                ],
                false,
            )
        }
        '⇔' | '⟺' => {
            let gap = (height * 0.18).max(thickness);
            (
                vec![
                    Move(x + head * 0.4, middle - gap),
                    Line(right - head * 0.4, middle - gap),
                    Move(x + head * 0.4, middle + gap),
                    Line(right - head * 0.4, middle + gap),
                    Move(right - head, middle - head * 0.7),
                    Line(right, middle),
                    Line(right - head, middle + head * 0.7),
                    Move(x + head, middle - head * 0.7),
                    Line(x, middle),
                    Line(x + head, middle + head * 0.7),
                ],
                false,
            )
        }
        '\u{20D1}' => (
            vec![
                Move(x, middle),
                Line(right, middle),
                Line(right - head, middle - head * 0.6),
            ],
            false,
        ),
        '\u{20D0}' => (
            vec![
                Move(right, middle),
                Line(x, middle),
                Line(x + head, middle - head * 0.6),
            ],
            false,
        ),
        '‾' | '¯' | '\u{0305}' | '\u{0304}' | '_' | '\u{0332}' | '─' | '—' => {
            (vec![Move(x, middle), Line(right, middle)], false)
        }
        '^' | 'ˆ' | '\u{0302}' => (
            vec![Move(x, bottom), Line(center, top), Line(right, bottom)],
            false,
        ),
        'ˇ' | '\u{030C}' => (
            vec![Move(x, top), Line(center, bottom), Line(right, top)],
            false,
        ),
        '~' | '˜' | '\u{0303}' => (
            vec![
                Move(x, bottom - height * 0.2),
                Cubic(
                    x + width * 0.2,
                    top - height * 0.1,
                    x + width * 0.35,
                    top,
                    center,
                    middle,
                ),
                Cubic(
                    right - width * 0.35,
                    bottom,
                    right - width * 0.2,
                    bottom + height * 0.1,
                    right,
                    top + height * 0.2,
                ),
            ],
            false,
        ),
        '˘' | '\u{0306}' => (
            vec![Move(x, top), Cubic(x, bottom, right, bottom, right, top)],
            false,
        ),
        '\u{0311}' => (
            vec![Move(x, bottom), Cubic(x, top, right, top, right, bottom)],
            false,
        ),
        '˙' | '\u{0307}' => (dots(&[center], middle, thickness), true),
        '¨' | '\u{0308}' => {
            let gap = (width * 0.25).max(thickness * 2.0);
            (dots(&[center - gap, center + gap], middle, thickness), true)
        }
        '\u{20DB}' | '⋯' => {
            let gap = (width * 0.3).max(thickness * 2.5);
            (
                dots(&[center - gap, center, center + gap], middle, thickness),
                true,
            )
        }
        '`' | '\u{0300}' => (vec![Move(x, top), Line(right, bottom)], false),
        '´' | '\u{0301}' => (vec![Move(x, bottom), Line(right, top)], false),
        '˚' | '\u{030A}' => (dots(&[center], middle, thickness * 1.6), false),
        _ => return None,
    })
}

/// Filled round dots of radius about the rule thickness.
fn dots(centers: &[f64], y: f64, thickness: f64) -> Vec<MathPathCommand> {
    let r = (thickness * 0.9).max(0.6);
    // A circle from four cubic quarter arcs.
    let k = 0.552_284_75 * r;
    let mut out = Vec::new();
    for &cx in centers {
        out.extend([
            Move(cx + r, y),
            Cubic(cx + r, y + k, cx + k, y + r, cx, y + r),
            Cubic(cx - k, y + r, cx - r, y + k, cx - r, y),
            Cubic(cx - r, y - k, cx - k, y - r, cx, y - r),
            Cubic(cx + k, y - r, cx + r, y - k, cx + r, y),
            Close,
        ]);
    }
    out
}

/// A radical sign from (`x`, `top`) `width` wide down to `bottom`, with its
/// vinculum running on to `rule_end`.
pub(crate) fn radical(
    x: f64,
    top: f64,
    bottom: f64,
    width: f64,
    rule_end: f64,
    thickness: f64,
) -> Vec<MathPathCommand> {
    let height = bottom - top;
    let y = top + thickness / 2.0;
    vec![
        Move(x, top + height * 0.62),
        Line(x + width * 0.22, top + height * 0.52),
        Line(x + width * 0.52, bottom),
        Line(x + width, y),
        Line(rule_end, y),
    ]
}

/// The bounds of a path: left, top, right, bottom.
pub(crate) fn bounds(commands: &[MathPathCommand]) -> Option<(f64, f64, f64, f64)> {
    let mut b: Option<(f64, f64, f64, f64)> = None;
    let mut add = |px: f64, py: f64| {
        b = Some(match b {
            None => (px, py, px, py),
            Some((l, t, r, btm)) => (l.min(px), t.min(py), r.max(px), btm.max(py)),
        });
    };
    for c in commands {
        match *c {
            Move(a, b) | Line(a, b) => add(a, b),
            Quad(a, b, c, d) => {
                add(a, b);
                add(c, d);
            }
            Cubic(a, b, c, d, e, f) => {
                add(a, b);
                add(c, d);
                add(e, f);
            }
            Close => {}
        }
    }
    b
}
