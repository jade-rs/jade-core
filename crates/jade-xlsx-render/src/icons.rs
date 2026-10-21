//! The conditional-format icon sets, drawn as vectors.
//!
//! Each set lists its icons from the one for the lowest values (index 0) to
//! the one for the highest, as the file numbers them (`iconId`). The shapes
//! follow what Excel shows for each set: arrows, traffic lights, flags,
//! signs, symbols, rating bars, quarters, stars, triangles and boxes.

use jade_painter::Painter;
use jade_xlsx_layout::Rect;
use std::f64::consts::PI;

const GREEN: &str = "#1E9E4A";
const YELLOW: &str = "#F6BD16";
const RED: &str = "#E0362D";
const GRAY: &str = "#8C8C8C";
const BLACK: &str = "#3B3B3B";
const PINK: &str = "#F2A6A0";
const BLUE: &str = "#3F74C7";
const GOLD: &str = "#F2B21A";
const EMPTY: &str = "#D9D9D9";
const WHITE: &str = "#FFFFFF";

/// One icon's drawing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Icon {
    /// An arrow pointing at `degrees` clockwise from up.
    Arrow(f64, &'static str),
    Circle(&'static str),
    /// A traffic light: a coloured lamp in a dark housing.
    Lamp(&'static str),
    Flag(&'static str),
    Diamond(&'static str),
    TriangleUp(&'static str),
    TriangleDown(&'static str),
    Dash(&'static str),
    /// A symbol in a coloured disc: 0 cross, 1 exclamation, 2 tick.
    CircledSymbol(u8, &'static str),
    /// The bare symbol.
    Symbol(u8, &'static str),
    /// Signal bars: how many of four are lit.
    Bars(u8),
    /// A disc filled by quarters.
    Quarters(u8),
    /// A star: 0 empty, 1 half, 2 full.
    Star(u8),
    /// A 2×2 grid of boxes: how many are filled.
    Boxes(u8),
}

fn icon(set: &str, index: u32) -> Option<Icon> {
    use Icon::*;
    let index = index as usize;
    let pick = |icons: &[Icon]| icons.get(index).copied();
    match set {
        "3Arrows" => pick(&[Arrow(180.0, RED), Arrow(90.0, YELLOW), Arrow(0.0, GREEN)]),
        "3ArrowsGray" => pick(&[Arrow(180.0, GRAY), Arrow(90.0, GRAY), Arrow(0.0, GRAY)]),
        "4Arrows" => pick(&[
            Arrow(180.0, RED),
            Arrow(135.0, YELLOW),
            Arrow(45.0, YELLOW),
            Arrow(0.0, GREEN),
        ]),
        "4ArrowsGray" => pick(&[
            Arrow(180.0, GRAY),
            Arrow(135.0, GRAY),
            Arrow(45.0, GRAY),
            Arrow(0.0, GRAY),
        ]),
        "5Arrows" => pick(&[
            Arrow(180.0, RED),
            Arrow(135.0, YELLOW),
            Arrow(90.0, YELLOW),
            Arrow(45.0, YELLOW),
            Arrow(0.0, GREEN),
        ]),
        "5ArrowsGray" => pick(&[
            Arrow(180.0, GRAY),
            Arrow(135.0, GRAY),
            Arrow(90.0, GRAY),
            Arrow(45.0, GRAY),
            Arrow(0.0, GRAY),
        ]),
        "3Flags" => pick(&[Flag(RED), Flag(YELLOW), Flag(GREEN)]),
        "3TrafficLights1" => pick(&[Circle(RED), Circle(YELLOW), Circle(GREEN)]),
        "3TrafficLights2" => pick(&[Lamp(RED), Lamp(YELLOW), Lamp(GREEN)]),
        "4TrafficLights" => pick(&[Circle(BLACK), Circle(RED), Circle(YELLOW), Circle(GREEN)]),
        "4RedToBlack" => pick(&[Circle(BLACK), Circle(GRAY), Circle(PINK), Circle(RED)]),
        "3Signs" => pick(&[Diamond(RED), TriangleUp(YELLOW), Circle(GREEN)]),
        "3Symbols" => pick(&[
            CircledSymbol(0, RED),
            CircledSymbol(1, YELLOW),
            CircledSymbol(2, GREEN),
        ]),
        "3Symbols2" => pick(&[Symbol(0, RED), Symbol(1, YELLOW), Symbol(2, GREEN)]),
        "4Rating" => pick(&[Bars(1), Bars(2), Bars(3), Bars(4)]),
        "5Rating" => pick(&[Bars(0), Bars(1), Bars(2), Bars(3), Bars(4)]),
        "5Quarters" => pick(&[
            Quarters(0),
            Quarters(1),
            Quarters(2),
            Quarters(3),
            Quarters(4),
        ]),
        "3Stars" => pick(&[Star(0), Star(1), Star(2)]),
        "3Triangles" => pick(&[TriangleDown(RED), Dash(YELLOW), TriangleUp(GREEN)]),
        "5Boxes" => pick(&[Boxes(0), Boxes(1), Boxes(2), Boxes(3), Boxes(4)]),
        _ => None,
    }
}

/// Draws icon `index` of `set` into `rect`; an unknown icon draws nothing.
pub(crate) fn render_icon<P: Painter>(
    painter: &P,
    set: &str,
    index: u32,
    rect: Rect,
) -> Result<(), P::Error> {
    let Some(icon) = icon(set, index) else {
        return Ok(());
    };
    let size = rect.width.min(rect.height);
    if size <= 0.0 {
        return Ok(());
    }
    // Everything below is drawn in a 16-unit box.
    let unit = size / 16.0;
    let at = |x: f64, y: f64| (rect.x + x * unit, rect.y + y * unit);
    let polygon = |points: &[(f64, f64)], color: &str| {
        painter.begin_path();
        for (index, (x, y)) in points.iter().enumerate() {
            let (x, y) = at(*x, *y);
            if index == 0 {
                painter.move_to(x, y);
            } else {
                painter.line_to(x, y);
            }
        }
        painter.close_path();
        painter.set_fill_style_str(color);
        painter.fill();
    };
    let disc = |cx: f64, cy: f64, radius: f64, color: &str| -> Result<(), P::Error> {
        let (x, y) = at(cx, cy);
        painter.begin_path();
        painter.arc(x, y, radius * unit, 0.0, 2.0 * PI)?;
        painter.set_fill_style_str(color);
        painter.fill();
        Ok(())
    };
    let stroke_lines = |lines: &[&[(f64, f64)]], color: &str, width: f64| {
        painter.set_stroke_style_str(color);
        painter.set_line_width(width * unit);
        painter.set_line_dash(&[]);
        for line in lines {
            painter.begin_path();
            for (index, (x, y)) in line.iter().enumerate() {
                let (x, y) = at(*x, *y);
                if index == 0 {
                    painter.move_to(x, y);
                } else {
                    painter.line_to(x, y);
                }
            }
            painter.stroke();
        }
    };
    let symbol = |kind: u8, color: &str, width: f64| match kind {
        0 => stroke_lines(
            &[&[(5.0, 5.0), (11.0, 11.0)], &[(11.0, 5.0), (5.0, 11.0)]],
            color,
            width,
        ),
        1 => {
            stroke_lines(&[&[(8.0, 3.5), (8.0, 9.5)]], color, width);
            stroke_lines(&[&[(8.0, 11.5), (8.0, 12.8)]], color, width);
        }
        _ => stroke_lines(&[&[(4.0, 8.5), (7.0, 11.5), (12.5, 4.5)]], color, width),
    };
    match icon {
        Icon::Arrow(degrees, color) => {
            let shape = [
                (8.0, 1.0),
                (14.5, 7.5),
                (10.5, 7.5),
                (10.5, 15.0),
                (5.5, 15.0),
                (5.5, 7.5),
                (1.5, 7.5),
            ];
            let (sin, cos) = (degrees * PI / 180.0).sin_cos();
            // Diagonal arrows are drawn a little smaller to stay in the box.
            let scale = if degrees % 90.0 == 0.0 { 1.0 } else { 0.85 };
            let turned: Vec<(f64, f64)> = shape
                .iter()
                .map(|(x, y)| {
                    let (dx, dy) = ((x - 8.0) * scale, (y - 8.0) * scale);
                    (8.0 + dx * cos - dy * sin, 8.0 + dx * sin + dy * cos)
                })
                .collect();
            polygon(&turned, color);
        }
        Icon::Circle(color) => disc(8.0, 8.0, 6.5, color)?,
        Icon::Lamp(color) => {
            polygon(&[(1.5, 1.5), (14.5, 1.5), (14.5, 14.5), (1.5, 14.5)], BLACK);
            disc(8.0, 8.0, 5.0, color)?;
        }
        Icon::Flag(color) => {
            polygon(&[(3.0, 1.5), (4.5, 1.5), (4.5, 15.0), (3.0, 15.0)], BLACK);
            polygon(&[(4.5, 1.5), (14.0, 5.0), (4.5, 8.5)], color);
        }
        Icon::Diamond(color) => polygon(&[(8.0, 1.0), (15.0, 8.0), (8.0, 15.0), (1.0, 8.0)], color),
        Icon::TriangleUp(color) => polygon(&[(8.0, 2.0), (15.0, 14.0), (1.0, 14.0)], color),
        Icon::TriangleDown(color) => polygon(&[(1.0, 3.0), (15.0, 3.0), (8.0, 14.0)], color),
        Icon::Dash(color) => polygon(&[(2.0, 6.5), (14.0, 6.5), (14.0, 9.5), (2.0, 9.5)], color),
        Icon::CircledSymbol(kind, color) => {
            disc(8.0, 8.0, 7.0, color)?;
            symbol(kind, WHITE, 2.0);
        }
        Icon::Symbol(kind, color) => symbol(kind, color, 2.6),
        Icon::Bars(lit) => {
            for bar in 0..4u8 {
                let left = 1.5 + f64::from(bar) * 3.5;
                let top = 11.0 - f64::from(bar) * 3.0;
                let color = if bar < lit { BLUE } else { EMPTY };
                polygon(
                    &[
                        (left, top),
                        (left + 2.5, top),
                        (left + 2.5, 15.0),
                        (left, 15.0),
                    ],
                    color,
                );
            }
        }
        Icon::Quarters(filled) => {
            disc(8.0, 8.0, 7.0, BLACK)?;
            disc(8.0, 8.0, 6.0, WHITE)?;
            if filled > 0 {
                let (x, y) = at(8.0, 8.0);
                painter.begin_path();
                painter.move_to(x, y);
                painter.arc(
                    x,
                    y,
                    6.0 * unit,
                    -PI / 2.0,
                    -PI / 2.0 + f64::from(filled) * PI / 2.0,
                )?;
                painter.close_path();
                painter.set_fill_style_str(BLACK);
                painter.fill();
            }
        }
        Icon::Star(fill) => {
            let points: Vec<(f64, f64)> = (0..10)
                .map(|point| {
                    let radius = if point % 2 == 0 { 7.5 } else { 3.2 };
                    let angle = -PI / 2.0 + f64::from(point) * PI / 5.0;
                    (8.0 + radius * angle.cos(), 8.5 + radius * angle.sin())
                })
                .collect();
            polygon(&points, if fill == 2 { GOLD } else { EMPTY });
            if fill == 1 {
                painter.save();
                painter.begin_path();
                let (x, y) = at(0.0, 0.0);
                painter.rect(x, y, 8.0 * unit, 16.0 * unit);
                painter.clip();
                polygon(&points, GOLD);
                painter.restore();
            }
        }
        Icon::Boxes(filled) => {
            for cell in 0..4u8 {
                let (left, top) = match cell {
                    0 => (1.5, 8.5),
                    1 => (8.5, 8.5),
                    2 => (1.5, 1.5),
                    _ => (8.5, 1.5),
                };
                let color = if cell < filled { BLUE } else { EMPTY };
                polygon(
                    &[
                        (left, top),
                        (left + 6.0, top),
                        (left + 6.0, top + 6.0),
                        (left, top + 6.0),
                    ],
                    color,
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_set_has_its_count_of_icons() {
        for (set, count) in [
            ("3Arrows", 3),
            ("3ArrowsGray", 3),
            ("3Flags", 3),
            ("3TrafficLights1", 3),
            ("3TrafficLights2", 3),
            ("3Signs", 3),
            ("3Symbols", 3),
            ("3Symbols2", 3),
            ("3Stars", 3),
            ("3Triangles", 3),
            ("4Arrows", 4),
            ("4ArrowsGray", 4),
            ("4RedToBlack", 4),
            ("4Rating", 4),
            ("4TrafficLights", 4),
            ("5Arrows", 5),
            ("5ArrowsGray", 5),
            ("5Rating", 5),
            ("5Quarters", 5),
            ("5Boxes", 5),
        ] {
            for index in 0..count {
                assert!(icon(set, index).is_some(), "{set} {index}");
            }
            assert!(icon(set, count).is_none(), "{set} has only {count}");
        }
    }

    #[test]
    fn arrows_rise_with_the_value() {
        assert_eq!(icon("3Arrows", 0), Some(Icon::Arrow(180.0, RED)));
        assert_eq!(icon("3Arrows", 2), Some(Icon::Arrow(0.0, GREEN)));
    }
}
