//! Reveal masks of `<p:animEffect filter>` (and of the matching slide
//! transitions): the part of a box shown `progress` of the way through.
//! Every mask is built from polygons wound the same way, so a nonzero
//! clip shows their union.

use jade_painter::Painter;
use jade_pptx_layout::Rect;

/// A polygon in absolute coordinates.
pub type Polygon = Vec<(f64, f64)>;

/// The polygons `filter` shows of `rect` at `progress` (0 hidden, 1 all).
pub fn reveal_polygons(rect: Rect, filter: &str, progress: f64) -> Vec<Polygon> {
    let p = progress.clamp(0.0, 1.0);
    let (name, argument) = match filter.find('(') {
        Some(open) => (
            filter[..open].trim(),
            filter[open + 1..].trim_end_matches(')').trim(),
        ),
        None => (filter.trim(), ""),
    };
    let Rect {
        x,
        y,
        width: w,
        height: h,
    } = rect;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let boxed = |x0: f64, y0: f64, x1: f64, y1: f64| vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    match name {
        "wipe" | "slide" => match argument {
            // 下から: the shown part grows upwards.
            "up" | "fromBottom" => vec![boxed(x, y + h * (1.0 - p), x + w, y + h)],
            "left" | "fromRight" => vec![boxed(x + w * (1.0 - p), y, x + w, y + h)],
            "right" | "fromLeft" => vec![boxed(x, y, x + w * p, y + h)],
            _ => vec![boxed(x, y, x + w, y + h * p)],
        },
        "barn" => match argument {
            // Edges closing in on the middle.
            "inVertical" => vec![
                boxed(x, y, x + w * p / 2.0, y + h),
                boxed(x + w * (1.0 - p / 2.0), y, x + w, y + h),
            ],
            "inHorizontal" => vec![
                boxed(x, y, x + w, y + h * p / 2.0),
                boxed(x, y + h * (1.0 - p / 2.0), x + w, y + h),
            ],
            "outHorizontal" => vec![boxed(x, cy - h * p / 2.0, x + w, cy + h * p / 2.0)],
            _ => vec![boxed(cx - w * p / 2.0, y, cx + w * p / 2.0, y + h)],
        },
        "blinds" => {
            let bands = 6;
            (0..bands)
                .map(|band| {
                    let band = f64::from(band);
                    if argument == "vertical" {
                        let left = x + w * band / 6.0;
                        boxed(left, y, left + w / 6.0 * p, y + h)
                    } else {
                        let top = y + h * band / 6.0;
                        boxed(x, top, x + w, top + h / 6.0 * p)
                    }
                })
                .collect()
        }
        "checkerboard" => {
            let cells = 8;
            let mut out = Vec::new();
            for row in 0..cells {
                for column in 0..cells {
                    let (row, column) = (f64::from(row), f64::from(column));
                    let (cw, ch) = (w / 8.0, h / 8.0);
                    // Alternate cells start half a cell later.
                    let shift = if (row + column) as i32 % 2 == 0 {
                        0.0
                    } else {
                        0.5
                    };
                    let grow = ((p * 1.5 - shift / 1.5).clamp(0.0, 1.0))
                        * if argument == "down" { ch } else { cw };
                    let (x0, y0) = (x + column * cw, y + row * ch);
                    out.push(if argument == "down" {
                        boxed(x0, y0, x0 + cw, y0 + grow.min(ch))
                    } else {
                        boxed(x0, y0, x0 + grow.min(cw), y0 + ch)
                    });
                }
            }
            out
        }
        "randombar" | "dissolve" => {
            let count = if name == "dissolve" { 16 } else { 64 };
            let mut out = Vec::new();
            if name == "dissolve" {
                for row in 0..count {
                    for column in 0..count {
                        let index = row * count + column;
                        if scatter(index) < p {
                            let (cw, ch) = (w / count as f64, h / count as f64);
                            let (x0, y0) = (x + column as f64 * cw, y + row as f64 * ch);
                            out.push(boxed(x0, y0, x0 + cw, y0 + ch));
                        }
                    }
                }
            } else {
                for index in 0..count {
                    if scatter(index) < p {
                        let i = index as f64;
                        out.push(if argument == "vertical" {
                            boxed(x + w * i / 64.0, y, x + w * (i + 1.0) / 64.0, y + h)
                        } else {
                            boxed(x, y + h * i / 64.0, x + w, y + h * (i + 1.0) / 64.0)
                        });
                    }
                }
            }
            out
        }
        "box" => {
            if argument == "in" {
                // A frame closing on the centre.
                let (iw, ih) = (w * (1.0 - p) / 2.0, h * (1.0 - p) / 2.0);
                frame(x, y, w, h, cx - iw, cy - ih, cx + iw, cy + ih)
            } else {
                vec![boxed(
                    cx - w * p / 2.0,
                    cy - h * p / 2.0,
                    cx + w * p / 2.0,
                    cy + h * p / 2.0,
                )]
            }
        }
        "circle" | "diamond" | "plus" => {
            // Big enough at 1 to cover the corners.
            let reach = if name == "circle" {
                std::f64::consts::SQRT_2
            } else {
                2.0
            };
            let (rx, ry) = (w / 2.0 * reach, h / 2.0 * reach);
            let shape = |scale: f64| -> Polygon {
                match name {
                    "circle" => (0..64)
                        .map(|i| {
                            let a = f64::from(i) / 64.0 * std::f64::consts::TAU;
                            (cx + rx * scale * a.cos(), cy + ry * scale * a.sin())
                        })
                        .collect(),
                    "diamond" => vec![
                        (cx, cy - ry * scale),
                        (cx + rx * scale, cy),
                        (cx, cy + ry * scale),
                        (cx - rx * scale, cy),
                    ],
                    _ => {
                        let (a, b) = (rx * scale, ry * scale);
                        let (c, d) = (a / 3.0, b / 3.0);
                        vec![
                            (cx - c, cy - b),
                            (cx + c, cy - b),
                            (cx + c, cy - d),
                            (cx + a, cy - d),
                            (cx + a, cy + d),
                            (cx + c, cy + d),
                            (cx + c, cy + b),
                            (cx - c, cy + b),
                            (cx - c, cy + d),
                            (cx - a, cy + d),
                            (cx - a, cy - d),
                            (cx - c, cy - d),
                        ]
                    }
                }
            };
            if argument == "in" {
                // The outside of a shrinking shape.
                let mut inner = shape(1.0 - p);
                inner.reverse();
                let mut out = vec![boxed(x - w, y - h, x + 2.0 * w, y + 2.0 * h)];
                out.push(inner);
                out
            } else {
                vec![shape(p)]
            }
        }
        "wheel" => {
            let spokes: u32 = argument.parse().unwrap_or(1).max(1);
            let reach = (w * w + h * h).sqrt();
            (0..spokes)
                .map(|spoke| {
                    let from = f64::from(spoke) / f64::from(spokes) * std::f64::consts::TAU
                        - std::f64::consts::FRAC_PI_2;
                    let sweep = std::f64::consts::TAU / f64::from(spokes) * p;
                    sector(cx, cy, reach, from, sweep)
                })
                .collect()
        }
        "wedge" => {
            let reach = (w * w + h * h).sqrt();
            let sweep = std::f64::consts::PI * p;
            vec![sector(
                cx,
                cy,
                reach,
                -std::f64::consts::FRAC_PI_2 - sweep,
                sweep * 2.0,
            )]
        }
        "strips" => {
            // A diagonal edge sweeping from one corner.
            let (sx, sy) = match argument {
                "upLeft" => (-1.0, -1.0),
                "upRight" => (1.0, -1.0),
                "downLeft" => (-1.0, 1.0),
                _ => (1.0, 1.0),
            };
            let origin = (
                if sx > 0.0 { x } else { x + w },
                if sy > 0.0 { y } else { y + h },
            );
            let reach = 2.0 * p;
            vec![vec![
                origin,
                (origin.0 + sx * w * reach, origin.1),
                (origin.0, origin.1 + sy * h * reach),
            ]]
            .into_iter()
            .map(|mut triangle| {
                if sx * sy < 0.0 {
                    triangle.reverse();
                }
                triangle
            })
            .collect()
        }
        _ => {
            if p > 0.0 {
                vec![boxed(x, y, x + w, y + h)]
            } else {
                Vec::new()
            }
        }
    }
}

/// Sets the painter's path to the mask; the caller clips to it.
pub fn reveal_path<P: Painter>(painter: &P, rect: Rect, filter: &str, progress: f64) {
    painter.begin_path();
    for polygon in reveal_polygons(rect, filter, progress) {
        let mut points = polygon.into_iter();
        if let Some((x, y)) = points.next() {
            painter.move_to(x, y);
            for (x, y) in points {
                painter.line_to(x, y);
            }
            painter.close_path();
        }
    }
    // An empty mask still clips everything away.
    painter.rect(rect.x, rect.y, 0.0, 0.0);
}

/// A fixed scatter in 0..1 for cell `index`, so random effects do not
/// flicker between frames.
fn scatter(index: usize) -> f64 {
    let mut value = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5DEE_CE66;
    value ^= value >> 29;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 32;
    (value % 10_000) as f64 / 10_000.0
}

#[allow(clippy::too_many_arguments)]
fn frame(x: f64, y: f64, w: f64, h: f64, ix0: f64, iy0: f64, ix1: f64, iy1: f64) -> Vec<Polygon> {
    vec![
        vec![(x, y), (x + w, y), (x + w, iy0), (x, iy0)],
        vec![(x, iy1), (x + w, iy1), (x + w, y + h), (x, y + h)],
        vec![(x, iy0), (ix0, iy0), (ix0, iy1), (x, iy1)],
        vec![(ix1, iy0), (x + w, iy0), (x + w, iy1), (ix1, iy1)],
    ]
}

fn sector(cx: f64, cy: f64, radius: f64, from: f64, sweep: f64) -> Polygon {
    let mut points = vec![(cx, cy)];
    let steps = ((sweep.abs() / std::f64::consts::TAU) * 64.0)
        .ceil()
        .max(1.0) as u32;
    for step in 0..=steps {
        let angle = from + sweep * f64::from(step) / f64::from(steps);
        points.push((cx + radius * angle.cos(), cy + radius * angle.sin()));
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(polygons: &[Polygon]) -> f64 {
        polygons
            .iter()
            .map(|polygon| {
                let mut sum = 0.0;
                for i in 0..polygon.len() {
                    let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
                    sum += a.0 * b.1 - b.0 * a.1;
                }
                sum / 2.0
            })
            .sum()
    }

    #[test]
    fn masks_grow_with_progress() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        assert_eq!(area(&reveal_polygons(rect, "wipe(down)", 0.5)), 2500.0);
        assert_eq!(reveal_polygons(rect, "wipe(up)", 0.5)[0][0], (0.0, 25.0));
        assert_eq!(
            area(&reveal_polygons(rect, "barn(inVertical)", 0.5)),
            2500.0
        );
        assert!((area(&reveal_polygons(rect, "blinds(horizontal)", 1.0)) - 5000.0).abs() < 1e-6);
        assert!((area(&reveal_polygons(rect, "box(in)", 0.5)) - 3750.0).abs() < 1e-6);
        for filter in [
            "circle(out)",
            "diamond(out)",
            "plus(out)",
            "wheel(4)",
            "wedge",
            "strips(downRight)",
            "checkerboard(across)",
            "dissolve",
            "randombar(horizontal)",
        ] {
            let small = area(&reveal_polygons(rect, filter, 0.2)).abs();
            let large = area(&reveal_polygons(rect, filter, 0.9)).abs();
            assert!(large > small, "{filter}: {small} → {large}");
        }
    }
}
