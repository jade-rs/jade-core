//! Word's art page borders (`w:pgBorders` with an art `w:val`, ECMA-376 Part 1
//! §17.18.2), drawn as vector motifs.
//!
//! Word draws each art border from a bitmap tile repeated along the edge,
//! `w:sz` points square (for art borders `w:sz` is in points, not eighths).
//! jade has no copy of that imagery; each art value is drawn with a vector
//! motif of the same kind — dots, squares, hearts, stars, trees, waves,
//! zigzags, … — in the art's colours, repeated the way Word repeats the tile.
//! The `basic*` and `*Black` values, like Word, use the border colour.

use super::border_styles::BorderSide;
use jade_painter::Painter;

/// The drawn shape of one tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Motif {
    Dot,
    OpenDot,
    Square,
    OpenSquare,
    Dash,
    OpenDash,
    Diamond,
    Triangle,
    Star,
    Heart,
    Ring,
    Flower,
    Tree,
    Apple,
    Snowflake,
    Moon,
    Sun,
    Balloon,
    Egg,
    Leaf,
    Gem,
    Arch,
    House,
    Chevron,
    Cross,
    Person,
    Note,
    Checker,
    /// Continuous: rules along the edge as (start, width) fractions of the
    /// tile across the band.
    Lines(&'static [(f64, f64)]),
    Wave,
    ZigZag,
    Sawtooth,
    Braid,
}

/// Where a motif's colours come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Palette {
    /// The border's own colour (`w:color`), black when automatic.
    Border,
    /// Fixed colours, cycled tile by tile.
    Fixed(&'static [&'static str]),
}

pub(super) struct ArtBorder {
    pub name: &'static str,
    pub motif: Motif,
    pub palette: Palette,
}

const fn art(name: &'static str, motif: Motif, palette: Palette) -> ArtBorder {
    ArtBorder {
        name,
        motif,
        palette,
    }
}

use Motif::*;
use Palette::{Border, Fixed};

const RED_GREEN: Palette = Fixed(&["#d0021b", "#2e7d32"]);
const GRAY: Palette = Fixed(&["#808080"]);
const MULTI: Palette = Fixed(&["#e53935", "#1e88e5", "#fdd835", "#43a047", "#8e24aa"]);
const GREEN: Palette = Fixed(&["#2e7d32", "#6d4c41"]);
const PINK: Palette = Fixed(&["#f48fb1", "#ad1457"]);
const YELLOW: Palette = Fixed(&["#fbc02d", "#f57f17"]);
const BLUE: Palette = Fixed(&["#1e88e5", "#0d47a1"]);
const BROWN: Palette = Fixed(&["#8d6e63", "#4e342e"]);
const ORANGE: Palette = Fixed(&["#fb8c00", "#2e7d32"]);
const PURPLE: Palette = Fixed(&["#8e24aa", "#fdd835"]);
const TAN: Palette = Fixed(&["#c8a165", "#8d6e63"]);

/// Every art value of `ST_Border`, by its `w:val` name.
pub(super) const ART_BORDERS: &[ArtBorder] = &[
    art("apples", Apple, RED_GREEN),
    art("archedScallops", Arch, Border),
    art("babyPacifier", Ring, PINK),
    art("babyRattle", Balloon, PINK),
    art("balloons3Colors", Balloon, MULTI),
    art(
        "balloonsHotAir",
        Balloon,
        Fixed(&["#fb8c00", "#1e88e5", "#e53935"]),
    ),
    art("basicBlackDashes", Dash, Border),
    art("basicBlackDots", Dot, Border),
    art("basicBlackSquares", Square, Border),
    art(
        "basicThinLines",
        Lines(&[(0.15, 0.1), (0.45, 0.1), (0.75, 0.1)]),
        Border,
    ),
    art("basicWhiteDashes", OpenDash, Border),
    art("basicWhiteDots", OpenDot, Border),
    art("basicWhiteSquares", OpenSquare, Border),
    art(
        "basicWideInline",
        Lines(&[(0.1, 0.15), (0.45, 0.45)]),
        Border,
    ),
    art(
        "basicWideMidline",
        Lines(&[(0.05, 0.15), (0.3, 0.4), (0.8, 0.15)]),
        Border,
    ),
    art(
        "basicWideOutline",
        Lines(&[(0.1, 0.45), (0.75, 0.15)]),
        Border,
    ),
    art("bats", Moon, Border),
    art("birds", Chevron, Border),
    art("birdsFlight", Chevron, Border),
    art("cabins", House, BROWN),
    art("cakeSlice", Triangle, PINK),
    art("candyCorn", Triangle, Fixed(&["#fb8c00", "#fdd835"])),
    art("celticKnotwork", Braid, Border),
    art(
        "certificateBanner",
        Lines(&[(0.05, 0.3), (0.5, 0.1), (0.75, 0.2)]),
        Border,
    ),
    art("chainLink", Ring, GRAY),
    art("champagneBottle", Gem, GREEN),
    art("checkedBarBlack", Checker, Border),
    art("checkedBarColor", Checker, Fixed(&["#e53935", "#1e88e5"])),
    art("checkered", Checker, Border),
    art("christmasTree", Tree, GREEN),
    art("circlesLines", Ring, Border),
    art("circlesRectangles", Ring, Border),
    art("classicalWave", Wave, Border),
    art("clocks", Ring, Border),
    art("compass", Star, Border),
    art("confetti", Dot, MULTI),
    art(
        "confettiGrays",
        Dot,
        Fixed(&["#9e9e9e", "#616161", "#bdbdbd"]),
    ),
    art("confettiOutline", OpenDot, Border),
    art("confettiStreamers", ZigZag, MULTI),
    art("confettiWhite", OpenDot, Border),
    art("cornerTriangles", Triangle, Border),
    art("couponCutoutDashes", Dash, Border),
    art("couponCutoutDots", Dot, Border),
    art("crazyMaze", OpenSquare, Border),
    art("creaturesButterfly", Flower, PURPLE),
    art("creaturesFish", Leaf, BLUE),
    art("creaturesInsects", Dot, Border),
    art("creaturesLadyBug", Dot, Fixed(&["#d0021b"])),
    art("crossStitch", Cross, Fixed(&["#c62828"])),
    art("cup", Arch, BROWN),
    art("decoArch", Arch, Border),
    art("decoArchColor", Arch, Fixed(&["#1e88e5", "#f9a825"])),
    art("decoBlocks", Square, Border),
    art("diamondsGray", Diamond, GRAY),
    art("doubleD", Arch, Border),
    art("doubleDiamonds", Diamond, Border),
    art("earth1", Ring, Fixed(&["#1e88e5", "#43a047"])),
    art("earth2", Dot, Fixed(&["#1e88e5", "#43a047"])),
    art("eclipsingSquares1", Square, Border),
    art("eclipsingSquares2", OpenSquare, Border),
    art("eggsBlack", Egg, Border),
    art("fans", Arch, Border),
    art("film", OpenSquare, Border),
    art("firecrackers", Star, Fixed(&["#e53935", "#fdd835"])),
    art("flowersBlockPrint", Flower, Border),
    art("flowersDaisies", Flower, Fixed(&["#ffffff", "#fdd835"])),
    art("flowersModern1", Flower, RED_GREEN),
    art("flowersModern2", Flower, Fixed(&["#e53935", "#fdd835"])),
    art("flowersPansy", Flower, PURPLE),
    art("flowersRedRose", Flower, RED_GREEN),
    art("flowersRoses", Flower, PINK),
    art("flowersTeacup", Flower, BLUE),
    art("flowersTiny", Flower, PINK),
    art("gems", Gem, MULTI),
    art("gingerbreadMan", Person, BROWN),
    art("gradient", Square, GRAY),
    art("handmade1", Wave, Border),
    art("handmade2", ZigZag, Border),
    art("heartBalloon", Heart, Fixed(&["#e53935"])),
    art("heartGray", Heart, GRAY),
    art("hearts", Heart, Fixed(&["#d0021b"])),
    art("heebieJeebies", ZigZag, MULTI),
    art("holly", Leaf, RED_GREEN),
    art("houseFunky", House, MULTI),
    art("hypnotic", Ring, Border),
    art("iceCreamCones", Triangle, Fixed(&["#f48fb1", "#c8a165"])),
    art("lightBulb", Balloon, YELLOW),
    art("lightning1", ZigZag, YELLOW),
    art("lightning2", ZigZag, YELLOW),
    art("mapPins", Balloon, Fixed(&["#e53935"])),
    art("mapleLeaf", Star, Fixed(&["#e65100", "#c62828"])),
    art("mapleMuffins", Arch, BROWN),
    art("marquee", Dot, YELLOW),
    art("marqueeToothed", Sawtooth, Border),
    art("moons", Moon, YELLOW),
    art("mosaic", Checker, MULTI),
    art("musicNotes", Note, Border),
    art("northwest", Triangle, BROWN),
    art("ovals", Egg, Border),
    art("packages", Square, BROWN),
    art("palmsBlack", Tree, Border),
    art("palmsColor", Tree, GREEN),
    art("paperClips", Ring, GRAY),
    art(
        "papyrus",
        Lines(&[(0.1, 0.2), (0.5, 0.1), (0.75, 0.15)]),
        TAN,
    ),
    art("partyFavor", Triangle, MULTI),
    art("partyGlass", Triangle, BLUE),
    art("pencils", Triangle, YELLOW),
    art("people", Person, Border),
    art("peopleWaving", Person, MULTI),
    art("peopleHats", Person, MULTI),
    art("poinsettias", Star, RED_GREEN),
    art("postageStamp", OpenSquare, Border),
    art("pumpkin1", Apple, ORANGE),
    art("pushPinNote2", Square, YELLOW),
    art("pushPinNote1", Square, YELLOW),
    art("pyramids", Triangle, TAN),
    art("pyramidsAbove", Triangle, TAN),
    art("quadrants", Checker, Border),
    art("rings", Ring, Fixed(&["#f9a825"])),
    art("safari", Leaf, GREEN),
    art("sawtooth", Sawtooth, Border),
    art("sawtoothGray", Sawtooth, GRAY),
    art("scaredCat", Triangle, Border),
    art("seattle", Triangle, Border),
    art("shadowedSquares", Square, Border),
    art("sharksTeeth", Sawtooth, Border),
    art("shorebirdTracks", Chevron, Border),
    art("skyrocket", Star, Fixed(&["#1e88e5", "#e53935"])),
    art("snowflakeFancy", Snowflake, BLUE),
    art("snowflakes", Snowflake, BLUE),
    art("sombrero", Arch, BROWN),
    art("southwest", ZigZag, Fixed(&["#8d6e63", "#26a69a"])),
    art("stars", Star, YELLOW),
    art("starsTop", Star, YELLOW),
    art("stars3d", Star, YELLOW),
    art("starsBlack", Star, Border),
    art("starsShadowed", Star, Border),
    art("sun", Sun, YELLOW),
    art("swirligig", Flower, MULTI),
    art("tornPaper", ZigZag, GRAY),
    art("tornPaperBlack", ZigZag, Border),
    art("trees", Tree, GREEN),
    art("triangleParty", Triangle, MULTI),
    art("triangles", Triangle, Border),
    art("triangle1", Triangle, Border),
    art("triangle2", Triangle, Border),
    art("triangleCircle1", Triangle, Border),
    art("triangleCircle2", Triangle, Border),
    art("shapes1", Gem, MULTI),
    art("shapes2", Gem, MULTI),
    art("twistedLines1", Braid, Border),
    art("twistedLines2", Braid, Border),
    art("vine", Leaf, GREEN),
    art("waveline", Wave, Border),
    art("weavingAngles", Braid, Border),
    art("weavingBraid", Braid, Border),
    art("weavingRibbon", Braid, Border),
    art("weavingStrips", Braid, Border),
    art("whiteFlowers", Flower, Fixed(&["#ffffff", "#9e9e9e"])),
    art(
        "woodwork",
        Lines(&[(0.05, 0.25), (0.4, 0.2), (0.7, 0.25)]),
        BROWN,
    ),
    art("xIllusions", Cross, Border),
    art("zanyTriangles", Triangle, MULTI),
    art("zigZag", ZigZag, Border),
    art("zigZagStitch", ZigZag, Border),
    art("custom", Square, Border),
];

/// The schema spelling of the art border named `name` (case-insensitive).
pub(crate) fn art_border_name(name: &str) -> Option<&'static str> {
    art_border_index(name).map(|index| ART_BORDERS[usize::from(index)].name)
}

/// The index of the art border named `name` (case-insensitive).
pub(super) fn art_border_index(name: &str) -> Option<u16> {
    ART_BORDERS
        .iter()
        .position(|art| art.name.eq_ignore_ascii_case(name))
        .map(|index| index as u16)
}

/// Draws an art border along the line from `start` to `end`, tiles `size`
/// square centred on it, on the `side` edge of the box. Tiles stand upright
/// on every side, as Word's do; continuous motifs (rules, waves, zigzags)
/// follow the edge.
pub(super) fn paint_art_border_edge<P: Painter>(
    context: &P,
    start: (f64, f64),
    end: (f64, f64),
    size: f64,
    index: u16,
    border_color: &str,
    side: BorderSide,
) -> Result<(), P::Error> {
    let Some(art) = ART_BORDERS.get(usize::from(index)) else {
        return Ok(());
    };
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f64::EPSILON || size <= 0.0 {
        return Ok(());
    }
    let (tx, ty) = (dx / length, dy / length);
    // Local frame: x along the edge, -y away from the box.
    let rotation = match side {
        BorderSide::Top => 0.0,
        BorderSide::Bottom => std::f64::consts::PI,
        BorderSide::Left => -std::f64::consts::FRAC_PI_2,
        BorderSide::Right => std::f64::consts::FRAC_PI_2,
    };
    let border = if border_color.trim().is_empty() || border_color.eq_ignore_ascii_case("auto") {
        "#000000"
    } else {
        border_color
    };
    let colors: Vec<&str> = match art.palette {
        Palette::Border => vec![border],
        Palette::Fixed(colors) => colors.to_vec(),
    };
    context.save();
    context.set_line_dash(&[]);
    if is_continuous(art.motif) {
        // One run along the whole edge, in an edge-aligned frame centred on
        // the edge's midpoint.
        let (cx, cy) = (start.0 + dx / 2.0, start.1 + dy / 2.0);
        context.translate(cx, cy)?;
        context.rotate(rotation)?;
        paint_continuous(context, art.motif, length, size, &colors)?;
        context.restore();
        return Ok(());
    }
    let count = (length / size).round().max(1.0) as usize;
    let tile = length / count as f64;
    for index in 0..count {
        let along = tile * (index as f64 + 0.5);
        let (cx, cy) = (start.0 + tx * along, start.1 + ty * along);
        context.save();
        // Word repeats one upright tile on every side.
        context.translate(cx, cy)?;
        // A two-colour palette on a two-tone motif is its body and accent
        // (a red apple, a green leaf); otherwise tiles cycle the colours.
        let (primary, secondary) = if colors.len() == 2 && is_two_tone(art.motif) {
            (colors[0], colors[1])
        } else {
            (
                colors[index % colors.len()],
                colors[(index + 1) % colors.len()],
            )
        };
        paint_motif(context, art.motif, size.min(tile), primary, secondary)?;
        context.restore();
    }
    context.restore();
    Ok(())
}

fn is_two_tone(motif: Motif) -> bool {
    matches!(
        motif,
        Apple | Flower | Tree | Sun | Balloon | Checker | Triangle | Ring | Leaf
    )
}

fn is_continuous(motif: Motif) -> bool {
    matches!(motif, Lines(_) | Wave | ZigZag | Sawtooth | Braid)
}

/// A continuous motif `length` long, centred on the origin, across `size`.
fn paint_continuous<P: Painter>(
    context: &P,
    motif: Motif,
    length: f64,
    size: f64,
    colors: &[&str],
) -> Result<(), P::Error> {
    let half = length / 2.0;
    let top = -size / 2.0;
    match motif {
        Lines(rules) => {
            context.set_fill_style_str(colors[0]);
            for (start, width) in rules {
                context.fill_rect(-half, top + start * size, length, (width * size).max(0.5));
            }
        }
        Wave | ZigZag | Sawtooth | Braid => {
            let period = size.max(2.0);
            let amplitude = size * 0.35;
            let steps = ((length / period) * 8.0).ceil().max(2.0) as usize;
            let point = |t: f64, phase: f64| -> (f64, f64) {
                let x = -half + length * t;
                let cycle = (x + half) / period + phase;
                let y = match motif {
                    Wave | Braid => amplitude * (cycle * std::f64::consts::TAU).sin(),
                    _ => {
                        // Triangle wave.
                        let fraction = cycle - cycle.floor();
                        amplitude * (4.0 * (fraction - 0.5).abs() - 1.0)
                    }
                };
                (x, y)
            };
            let line = |phase: f64, color: &str| {
                context.begin_path();
                for step in 0..=steps {
                    let (x, y) = point(step as f64 / steps as f64, phase);
                    if step == 0 {
                        context.move_to(x, y);
                    } else {
                        context.line_to(x, y);
                    }
                }
                context.set_stroke_style_str(color);
                context.set_line_width((size * 0.12).max(0.75));
                context.stroke();
            };
            match motif {
                Sawtooth => {
                    // Filled teeth pointing away from the box.
                    context.set_fill_style_str(colors[0]);
                    let teeth = (length / period).round().max(1.0) as usize;
                    let tooth = length / teeth as f64;
                    context.begin_path();
                    context.move_to(-half, size / 2.0);
                    for index in 0..teeth {
                        let x = -half + tooth * index as f64;
                        context.line_to(x + tooth / 2.0, -size / 2.0);
                        context.line_to(x + tooth, size / 2.0);
                    }
                    context.close_path();
                    context.fill();
                }
                Braid => {
                    line(0.0, colors[0]);
                    line(0.5, colors[colors.len().min(2) - 1]);
                }
                _ => line(0.0, colors[0]),
            }
        }
        _ => {}
    }
    Ok(())
}

/// One tile of `motif`, centred on the origin, `size` across, upright.
fn paint_motif<P: Painter>(
    context: &P,
    motif: Motif,
    size: f64,
    primary: &str,
    secondary: &str,
) -> Result<(), P::Error> {
    use std::f64::consts::{PI, TAU};
    let r = size / 2.0;
    let fill = |color: &str| {
        context.set_fill_style_str(color);
        context.fill();
    };
    let outline = |color: &str, width: f64| {
        context.set_stroke_style_str(color);
        context.set_line_width(width.max(0.5));
        context.stroke();
    };
    let circle = |x: f64, y: f64, radius: f64| -> Result<(), P::Error> {
        context.begin_path();
        context.arc(x, y, radius, 0.0, TAU)
    };
    let polygon = |points: &[(f64, f64)]| {
        context.begin_path();
        for (index, (x, y)) in points.iter().enumerate() {
            if index == 0 {
                context.move_to(*x, *y);
            } else {
                context.line_to(*x, *y);
            }
        }
        context.close_path();
    };
    let star = |points: usize, outer: f64, inner: f64| {
        let vertices: Vec<(f64, f64)> = (0..points * 2)
            .map(|index| {
                let radius = if index % 2 == 0 { outer } else { inner };
                let angle = -PI / 2.0 + PI * index as f64 / points as f64;
                (radius * angle.cos(), radius * angle.sin())
            })
            .collect();
        polygon(&vertices);
    };
    match motif {
        Dot => {
            circle(0.0, 0.0, r * 0.55)?;
            fill(primary);
        }
        OpenDot => {
            circle(0.0, 0.0, r * 0.5)?;
            fill("#ffffff");
            circle(0.0, 0.0, r * 0.5)?;
            outline(primary, r * 0.15);
        }
        Square => {
            context.set_fill_style_str(primary);
            context.fill_rect(-r * 0.6, -r * 0.6, r * 1.2, r * 1.2);
        }
        OpenSquare => {
            context.set_fill_style_str("#ffffff");
            context.fill_rect(-r * 0.6, -r * 0.6, r * 1.2, r * 1.2);
            context.set_stroke_style_str(primary);
            context.set_line_width((r * 0.15).max(0.5));
            context.stroke_rect(-r * 0.6, -r * 0.6, r * 1.2, r * 1.2);
        }
        Dash => {
            context.set_fill_style_str(primary);
            context.fill_rect(-r * 0.75, -r * 0.2, r * 1.5, r * 0.4);
        }
        OpenDash => {
            context.set_stroke_style_str(primary);
            context.set_line_width((r * 0.12).max(0.5));
            context.stroke_rect(-r * 0.75, -r * 0.2, r * 1.5, r * 0.4);
        }
        Diamond => {
            polygon(&[
                (0.0, -r * 0.9),
                (r * 0.6, 0.0),
                (0.0, r * 0.9),
                (-r * 0.6, 0.0),
            ]);
            fill(primary);
        }
        Triangle => {
            polygon(&[(0.0, -r * 0.85), (r * 0.8, r * 0.7), (-r * 0.8, r * 0.7)]);
            fill(primary);
        }
        Star => {
            star(5, r * 0.95, r * 0.4);
            fill(primary);
        }
        Heart => {
            context.begin_path();
            context.move_to(0.0, r * 0.8);
            context.bezier_curve_to(-r * 1.2, -r * 0.1, -r * 0.5, -r * 1.0, 0.0, -r * 0.35);
            context.bezier_curve_to(r * 0.5, -r * 1.0, r * 1.2, -r * 0.1, 0.0, r * 0.8);
            context.close_path();
            fill(primary);
        }
        Ring => {
            circle(0.0, 0.0, r * 0.6)?;
            outline(primary, r * 0.22);
        }
        Flower => {
            for petal in 0..5 {
                let angle = -PI / 2.0 + TAU * petal as f64 / 5.0;
                circle(r * 0.45 * angle.cos(), r * 0.45 * angle.sin(), r * 0.38)?;
                fill(primary);
            }
            circle(0.0, 0.0, r * 0.25)?;
            fill(secondary);
        }
        Tree => {
            polygon(&[(0.0, -r * 0.95), (r * 0.7, r * 0.45), (-r * 0.7, r * 0.45)]);
            fill(primary);
            context.set_fill_style_str(secondary);
            context.fill_rect(-r * 0.12, r * 0.45, r * 0.24, r * 0.45);
        }
        Apple => {
            circle(0.0, r * 0.1, r * 0.65)?;
            fill(primary);
            context.begin_path();
            context.ellipse(r * 0.25, -r * 0.65, r * 0.3, r * 0.14, -0.5, 0.0, TAU)?;
            fill(secondary);
        }
        Snowflake => {
            context.begin_path();
            for spoke in 0..3 {
                let angle = PI * spoke as f64 / 3.0;
                let (x, y) = (r * 0.85 * angle.cos(), r * 0.85 * angle.sin());
                context.move_to(-x, -y);
                context.line_to(x, y);
            }
            outline(primary, r * 0.15);
        }
        Moon => {
            context.begin_path();
            context.arc(0.0, 0.0, r * 0.75, PI * 0.25, PI * 1.75)?;
            context.arc(r * 0.3, 0.0, r * 0.55, PI * 1.6, PI * 0.4)?;
            context.close_path();
            fill(primary);
        }
        Sun => {
            circle(0.0, 0.0, r * 0.45)?;
            fill(primary);
            context.begin_path();
            for ray in 0..8 {
                let angle = TAU * ray as f64 / 8.0;
                context.move_to(r * 0.6 * angle.cos(), r * 0.6 * angle.sin());
                context.line_to(r * 0.95 * angle.cos(), r * 0.95 * angle.sin());
            }
            outline(secondary, r * 0.12);
        }
        Balloon => {
            context.begin_path();
            context.ellipse(0.0, -r * 0.2, r * 0.5, r * 0.65, 0.0, 0.0, TAU)?;
            fill(primary);
            context.begin_path();
            context.move_to(0.0, r * 0.45);
            context.line_to(0.0, r * 0.95);
            outline(secondary, r * 0.08);
        }
        Egg => {
            context.begin_path();
            context.ellipse(0.0, 0.0, r * 0.5, r * 0.75, 0.0, 0.0, TAU)?;
            fill(primary);
        }
        Leaf => {
            context.begin_path();
            context.move_to(-r * 0.9, 0.0);
            context.quadratic_curve_to(0.0, -r * 0.9, r * 0.9, 0.0);
            context.quadratic_curve_to(0.0, r * 0.9, -r * 0.9, 0.0);
            context.close_path();
            fill(primary);
        }
        Gem => {
            let vertices: Vec<(f64, f64)> = (0..6)
                .map(|index| {
                    let angle = TAU * index as f64 / 6.0;
                    (r * 0.75 * angle.cos(), r * 0.75 * angle.sin())
                })
                .collect();
            polygon(&vertices);
            fill(primary);
        }
        Arch => {
            context.begin_path();
            context.arc(0.0, r * 0.5, r * 0.8, PI, TAU)?;
            outline(primary, r * 0.2);
        }
        House => {
            polygon(&[
                (0.0, -r * 0.9),
                (r * 0.8, -r * 0.15),
                (r * 0.6, -r * 0.15),
                (r * 0.6, r * 0.8),
                (-r * 0.6, r * 0.8),
                (-r * 0.6, -r * 0.15),
                (-r * 0.8, -r * 0.15),
            ]);
            fill(primary);
        }
        Chevron => {
            context.begin_path();
            context.move_to(-r * 0.8, r * 0.1);
            context.quadratic_curve_to(-r * 0.4, -r * 0.5, 0.0, 0.0);
            context.quadratic_curve_to(r * 0.4, -r * 0.5, r * 0.8, r * 0.1);
            outline(primary, r * 0.15);
        }
        Cross => {
            context.begin_path();
            context.move_to(-r * 0.6, -r * 0.6);
            context.line_to(r * 0.6, r * 0.6);
            context.move_to(r * 0.6, -r * 0.6);
            context.line_to(-r * 0.6, r * 0.6);
            outline(primary, r * 0.2);
        }
        Person => {
            circle(0.0, -r * 0.6, r * 0.28)?;
            fill(primary);
            polygon(&[
                (-r * 0.45, -r * 0.25),
                (r * 0.45, -r * 0.25),
                (r * 0.3, r * 0.9),
                (-r * 0.3, r * 0.9),
            ]);
            fill(primary);
        }
        Note => {
            context.begin_path();
            context.ellipse(-r * 0.2, r * 0.5, r * 0.3, r * 0.22, -0.4, 0.0, TAU)?;
            fill(primary);
            context.set_fill_style_str(primary);
            context.fill_rect(r * 0.04, -r * 0.8, r * 0.1, r * 1.3);
        }
        Checker => {
            context.set_fill_style_str(primary);
            context.fill_rect(-r, -r, r, r);
            context.fill_rect(0.0, 0.0, r, r);
            context.set_fill_style_str(if primary == secondary {
                "#ffffff"
            } else {
                secondary
            });
            context.fill_rect(0.0, -r, r, r);
            context.fill_rect(-r, 0.0, r, r);
        }
        Lines(_) | Wave | ZigZag | Sawtooth | Braid => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_painter::recording::{Op, RecordingPainter};

    /// ECMA-376's art values (§17.18.2), in schema order.
    const SCHEMA: &str = "apples archedScallops babyPacifier babyRattle balloons3Colors balloonsHotAir basicBlackDashes basicBlackDots basicBlackSquares basicThinLines basicWhiteDashes basicWhiteDots basicWhiteSquares basicWideInline basicWideMidline basicWideOutline bats birds birdsFlight cabins cakeSlice candyCorn celticKnotwork certificateBanner chainLink champagneBottle checkedBarBlack checkedBarColor checkered christmasTree circlesLines circlesRectangles classicalWave clocks compass confetti confettiGrays confettiOutline confettiStreamers confettiWhite cornerTriangles couponCutoutDashes couponCutoutDots crazyMaze creaturesButterfly creaturesFish creaturesInsects creaturesLadyBug crossStitch cup decoArch decoArchColor decoBlocks diamondsGray doubleD doubleDiamonds earth1 earth2 eclipsingSquares1 eclipsingSquares2 eggsBlack fans film firecrackers flowersBlockPrint flowersDaisies flowersModern1 flowersModern2 flowersPansy flowersRedRose flowersRoses flowersTeacup flowersTiny gems gingerbreadMan gradient handmade1 handmade2 heartBalloon heartGray hearts heebieJeebies holly houseFunky hypnotic iceCreamCones lightBulb lightning1 lightning2 mapPins mapleLeaf mapleMuffins marquee marqueeToothed moons mosaic musicNotes northwest ovals packages palmsBlack palmsColor paperClips papyrus partyFavor partyGlass pencils people peopleWaving peopleHats poinsettias postageStamp pumpkin1 pushPinNote2 pushPinNote1 pyramids pyramidsAbove quadrants rings safari sawtooth sawtoothGray scaredCat seattle shadowedSquares sharksTeeth shorebirdTracks skyrocket snowflakeFancy snowflakes sombrero southwest stars starsTop stars3d starsBlack starsShadowed sun swirligig tornPaper tornPaperBlack trees triangleParty triangles triangle1 triangle2 triangleCircle1 triangleCircle2 shapes1 shapes2 twistedLines1 twistedLines2 vine waveline weavingAngles weavingBraid weavingRibbon weavingStrips whiteFlowers woodwork xIllusions zanyTriangles zigZag zigZagStitch custom";

    #[test]
    fn every_art_value_has_a_motif() {
        for name in SCHEMA.split_whitespace() {
            assert!(art_border_index(name).is_some(), "{name} has no motif");
        }
        assert_eq!(ART_BORDERS.len(), SCHEMA.split_whitespace().count());
        assert_eq!(art_border_index("HEARTS"), art_border_index("hearts"));
        assert_eq!(art_border_index("single"), None);
    }

    fn paint(name: &str, color: &str) -> Vec<Op> {
        let painter = RecordingPainter::new();
        paint_art_border_edge(
            &painter,
            (0.0, 0.0),
            (100.0, 0.0),
            20.0,
            art_border_index(name).unwrap(),
            color,
            BorderSide::Top,
        )
        .unwrap();
        painter.ops()
    }

    #[test]
    fn tiles_repeat_along_the_edge_in_the_art_colours() {
        let ops = paint("hearts", "#000000");
        let tiles = ops
            .iter()
            .filter(|op| matches!(op, Op::Translate(..)))
            .count();
        assert_eq!(tiles, 5, "a 100px edge holds five 20px tiles");
        assert!(ops.contains(&Op::SetFillStyle("#d0021b".into())));
        assert!(!ops.contains(&Op::SetFillStyle("#000000".into())));
    }

    #[test]
    fn basic_and_black_arts_take_the_border_colour() {
        let ops = paint("basicBlackDots", "#336699");
        assert!(ops.contains(&Op::SetFillStyle("#336699".into())));
        let ops = paint("zigZag", "auto");
        assert!(ops.contains(&Op::SetStrokeStyle("#000000".into())));
    }
}
