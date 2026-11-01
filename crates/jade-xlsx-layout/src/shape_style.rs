//! Shapes drawn as Excel draws them: their preset geometry, fill and
//! outline (from `spPr`, falling back to the shape style's references), and
//! their text's colour, size and placement.

use crate::ResolvedFont;
use jade_xlsx_reader::{DrawingColor, ShapePaint, ShapeStyle, StyleTable, EMU_PER_PX};
use serde::{Deserialize, Serialize};

/// One step of an outline, in the shape's own box (0,0)–(width,height).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathOp {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// A shape's resolved look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedShapeStyle {
    /// The geometry's outline; open for a line.
    pub path: Vec<PathOp>,
    /// RGBA.
    pub fill: Option<[u8; 4]>,
    pub stroke: Option<[u8; 4]>,
    pub stroke_width: f64,
    pub dash: Vec<f64>,
    /// Arrowheads at a line's ends, as filled polygons.
    pub arrows: Vec<Vec<(f64, f64)>>,
    pub rotation_radians: f64,
    pub flip_h: bool,
    pub flip_v: bool,
    pub text_color: [u8; 4],
    /// `l`, `ctr` or `r`.
    pub text_align: String,
    /// `t`, `ctr` or `b`.
    pub text_anchor: String,
    /// Text insets: left/right and top/bottom, in px.
    pub inset_x: f64,
    pub inset_y: f64,
}

/// The Office theme, for a workbook without one.
const OFFICE: [[u8; 3]; 12] = [
    [255, 255, 255],
    [0, 0, 0],
    [231, 230, 230],
    [68, 84, 106],
    [68, 114, 196],
    [237, 125, 49],
    [165, 165, 165],
    [255, 192, 0],
    [91, 155, 213],
    [112, 173, 71],
    [5, 99, 193],
    [149, 79, 114],
];

fn scheme_index(name: &str) -> Option<usize> {
    Some(match name {
        "lt1" | "bg1" => 0,
        "dk1" | "tx1" => 1,
        "lt2" | "bg2" => 2,
        "dk2" | "tx2" => 3,
        "hlink" => 10,
        "folHlink" => 11,
        accent if accent.starts_with("accent") => 3 + accent[6..].parse::<usize>().ok()?,
        _ => return None,
    })
}

fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Resolves a DrawingML colour; `placeholder` stands in for `phClr`.
pub fn resolve_color(
    color: &DrawingColor,
    styles: &StyleTable,
    placeholder: Option<[u8; 4]>,
) -> Option<[u8; 4]> {
    let base = match (&color.scheme, color.rgb) {
        (Some(scheme), _) if scheme == "phClr" => placeholder?,
        (Some(scheme), _) => {
            let index = scheme_index(scheme)?;
            let rgb = styles
                .theme_colors
                .get(index)
                .copied()
                .flatten()
                .unwrap_or(OFFICE[index]);
            [rgb[0], rgb[1], rgb[2], 255]
        }
        (None, Some(rgb)) => [rgb[0], rgb[1], rgb[2], 255],
        (None, None) => return None,
    };
    let mut rgb = [base[0], base[1], base[2]].map(|c| f64::from(c) / 255.0);
    let mut alpha = f64::from(base[3]) / 255.0;
    let mut lum_mod: Option<f64> = None;
    let mut lum_off = 0.0;
    let mut sat_mod: Option<f64> = None;
    for (name, value) in &color.transforms {
        let amount = f64::from(*value) / 100_000.0;
        match name.as_str() {
            "shade" => rgb = rgb.map(|c| to_srgb(to_linear(c) * amount)),
            "tint" => rgb = rgb.map(|c| to_srgb(to_linear(c) * amount + (1.0 - amount))),
            "lumMod" => lum_mod = Some(lum_mod.unwrap_or(1.0) * amount),
            "lumOff" => lum_off += amount,
            "satMod" => sat_mod = Some(sat_mod.unwrap_or(1.0) * amount),
            "alpha" => alpha = amount,
            "alphaMod" => alpha *= amount,
            _ => {}
        }
    }
    if lum_mod.is_some() || lum_off != 0.0 || sat_mod.is_some() {
        let (h, s, l) = rgb_to_hsl(rgb);
        let l = (l * lum_mod.unwrap_or(1.0) + lum_off).clamp(0.0, 1.0);
        let s = (s * sat_mod.unwrap_or(1.0)).clamp(0.0, 1.0);
        rgb = hsl_to_rgb(h, s, l);
    }
    let byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(alpha)])
}

fn rgb_to_hsl([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s == 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f64| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

fn paint(
    paint: &ShapePaint,
    reference: Option<&(u32, DrawingColor)>,
    styles: &StyleTable,
) -> Option<[u8; 4]> {
    let referenced = || {
        let (index, color) = reference?;
        if *index == 0 {
            return None;
        }
        resolve_color(color, styles, None)
    };
    match paint {
        ShapePaint::Unset => referenced(),
        ShapePaint::None => None,
        ShapePaint::Solid(color) => resolve_color(color, styles, referenced()),
        ShapePaint::Gradient(stops) => {
            let colors: Vec<[u8; 4]> = stops
                .iter()
                .filter_map(|stop| resolve_color(stop, styles, referenced()))
                .collect();
            if colors.is_empty() {
                return None;
            }
            let mut sum = [0u32; 4];
            for color in &colors {
                for (total, channel) in sum.iter_mut().zip(color) {
                    *total += u32::from(*channel);
                }
            }
            Some(sum.map(|total| (total / colors.len() as u32) as u8))
        }
    }
}

/// Resolves a shape's look for a box `width` × `height` px, and the font its
/// text takes.
pub fn place(
    style: &ShapeStyle,
    width: f64,
    height: f64,
    styles: &StyleTable,
    font: &mut ResolvedFont,
    connector: bool,
) -> PlacedShapeStyle {
    let geometry = style
        .geometry
        .as_deref()
        .unwrap_or(if connector { "line" } else { "rect" });
    let open = is_line(geometry) || connector;
    let fill = if open {
        None
    } else {
        paint(&style.fill, style.style_fill.as_ref(), styles)
    };
    let stroke = paint(&style.line, style.style_line.as_ref(), styles);
    // The Office theme's line widths: ½, 1 and 1½ pt.
    let theme_width = style
        .style_line
        .as_ref()
        .map(|(index, _)| 6_350 * i64::from((*index).clamp(1, 3)))
        .unwrap_or(9_525);
    let stroke_width = (style.line_width.unwrap_or(theme_width) as f64 / EMU_PER_PX).max(0.5);
    let dash = match style.line_dash.as_deref() {
        Some("dash") => vec![4.0, 3.0],
        Some("sysDash") => vec![3.0, 1.0],
        Some("sysDot") => vec![1.0, 1.0],
        Some("dot") => vec![1.0, 3.0],
        Some("lgDash") => vec![8.0, 3.0],
        Some("dashDot" | "sysDashDot") => vec![4.0, 3.0, 1.0, 3.0],
        Some("lgDashDot") => vec![8.0, 3.0, 1.0, 3.0],
        _ => Vec::new(),
    }
    .into_iter()
    .map(|step| step * stroke_width)
    .collect();
    if let Some(size) = style.text_size {
        font.size_points = size as f32 / 100.0;
    }
    font.bold = style.text_bold;
    font.italic = style.text_italic;
    let text_color = style
        .text_color
        .as_ref()
        .and_then(|color| resolve_color(color, styles, None))
        .or_else(|| {
            style
                .style_font
                .as_ref()
                .and_then(|color| resolve_color(color, styles, None))
        })
        .unwrap_or([0, 0, 0, 255]);
    let path = preset_path(geometry, width, height, &style.adjustments);
    let mut arrows = Vec::new();
    if let Some(((start, after_start), (before_end, end))) =
        line_ends(&path).filter(|_| open && stroke.is_some())
    {
        // Each head points along the stretch of line it ends: the first or
        // last leg of an elbow, the curve's tangent there.
        let size = (stroke_width * 3.0).max(6.0);
        if let Some(kind) = style.head_end.as_deref() {
            arrows.push(arrowhead(after_start, start, size, kind));
        }
        if let Some(kind) = style.tail_end.as_deref() {
            arrows.push(arrowhead(before_end, end, size, kind));
        }
    }
    PlacedShapeStyle {
        path,
        fill,
        stroke,
        stroke_width,
        dash,
        arrows,
        rotation_radians: (f64::from(style.rotation) / 60_000.0).to_radians(),
        flip_h: style.flip_h,
        flip_v: style.flip_v,
        text_color,
        text_align: style.text_align.clone().unwrap_or_else(|| "l".into()),
        text_anchor: style.text_anchor.clone().unwrap_or_else(|| "t".into()),
        // 0.1 inch beside the text and 0.05 inch above and below, unless the
        // body says otherwise.
        inset_x: style.text_insets[0].map_or(9.6, |emu| emu as f64 / EMU_PER_PX),
        inset_y: style.text_insets[1].map_or(4.8, |emu| emu as f64 / EMU_PER_PX),
    }
}

/// A triangle at `tip`, pointing away from `from`.
fn arrowhead(from: (f64, f64), tip: (f64, f64), size: f64, kind: &str) -> Vec<(f64, f64)> {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let length = (dx * dx + dy * dy).sqrt().max(1e-9);
    let (ux, uy) = (dx / length, dy / length);
    let back = (tip.0 - ux * size, tip.1 - uy * size);
    let half = size * if kind == "arrow" { 0.45 } else { 0.5 };
    let mut points = vec![
        tip,
        (back.0 - uy * half, back.1 + ux * half),
        (back.0 + uy * half, back.1 - ux * half),
    ];
    if kind == "diamond" || kind == "oval" {
        let far = (tip.0 - ux * size * 2.0, tip.1 - uy * size * 2.0);
        points = vec![tip, points[1], far, points[2]];
    }
    points
}

fn is_line(geometry: &str) -> bool {
    geometry == "line"
        || geometry.starts_with("straightConnector")
        || geometry.starts_with("bentConnector")
        || geometry.starts_with("curvedConnector")
}

fn adjust(adjustments: &[(String, i64)], name: &str, default: f64) -> f64 {
    adjustments
        .iter()
        .find(|(key, _)| key == name)
        .map_or(default, |(_, value)| *value as f64)
        / 100_000.0
}

/// A line's two ends, each with the nearest other point that gives its
/// direction there: the next corner, or a curve's control point.
type LineEnd = ((f64, f64), (f64, f64));

fn line_ends(path: &[PathOp]) -> Option<(LineEnd, LineEnd)> {
    let points: Vec<(f64, f64)> = path
        .iter()
        .flat_map(|op| match *op {
            PathOp::Move(x, y) | PathOp::Line(x, y) => vec![(x, y)],
            PathOp::Cubic(x1, y1, x2, y2, x, y) => vec![(x1, y1), (x2, y2), (x, y)],
            PathOp::Close => Vec::new(),
        })
        .collect();
    let (&start, &end) = (points.first()?, points.last()?);
    let after_start = *points.iter().find(|point| **point != start)?;
    let before_end = *points.iter().rev().find(|point| **point != end)?;
    Some(((start, after_start), (before_end, end)))
}

fn polygon(points: &[(f64, f64)]) -> Vec<PathOp> {
    let mut path = Vec::with_capacity(points.len() + 1);
    for (index, (x, y)) in points.iter().enumerate() {
        path.push(if index == 0 {
            PathOp::Move(*x, *y)
        } else {
            PathOp::Line(*x, *y)
        });
    }
    path.push(PathOp::Close);
    path
}

fn ellipse(x: f64, y: f64, width: f64, height: f64) -> Vec<PathOp> {
    const K: f64 = 0.552_284_75;
    let (rx, ry) = (width / 2.0, height / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    vec![
        PathOp::Move(cx + rx, cy),
        PathOp::Cubic(cx + rx, cy + ry * K, cx + rx * K, cy + ry, cx, cy + ry),
        PathOp::Cubic(cx - rx * K, cy + ry, cx - rx, cy + ry * K, cx - rx, cy),
        PathOp::Cubic(cx - rx, cy - ry * K, cx - rx * K, cy - ry, cx, cy - ry),
        PathOp::Cubic(cx + rx * K, cy - ry, cx + rx, cy - ry * K, cx + rx, cy),
        PathOp::Close,
    ]
}

fn round_rect(width: f64, height: f64, radius: f64) -> Vec<PathOp> {
    const K: f64 = 0.552_284_75;
    let r = radius.min(width / 2.0).min(height / 2.0).max(0.0);
    let c = r * (1.0 - K);
    vec![
        PathOp::Move(r, 0.0),
        PathOp::Line(width - r, 0.0),
        PathOp::Cubic(width - c, 0.0, width, c, width, r),
        PathOp::Line(width, height - r),
        PathOp::Cubic(width, height - c, width - c, height, width - r, height),
        PathOp::Line(r, height),
        PathOp::Cubic(c, height, 0.0, height - c, 0.0, height - r),
        PathOp::Line(0.0, r),
        PathOp::Cubic(0.0, c, c, 0.0, r, 0.0),
        PathOp::Close,
    ]
}

fn star(points: usize, inner: f64, width: f64, height: f64) -> Vec<PathOp> {
    let (cx, cy) = (width / 2.0, height / 2.0);
    let corners: Vec<(f64, f64)> = (0..points * 2)
        .map(|index| {
            let angle =
                -std::f64::consts::FRAC_PI_2 + index as f64 * std::f64::consts::PI / points as f64;
            let radius = if index % 2 == 0 { 1.0 } else { inner };
            (
                cx + cx * radius * angle.cos(),
                cy + cy * radius * angle.sin(),
            )
        })
        .collect();
    polygon(&corners)
}

/// The outline of preset geometry `name` in a `w` × `h` box.
pub fn preset_path(name: &str, w: f64, h: f64, adjustments: &[(String, i64)]) -> Vec<PathOp> {
    let ss = w.min(h);
    let adj = |key: &str, default: f64| adjust(adjustments, key, default);
    match name {
        "line" => vec![PathOp::Move(0.0, 0.0), PathOp::Line(w, h)],
        // Elbow and curved connectors, as Office's preset definitions run
        // them from the top-left corner to the bottom-right.
        "bentConnector2" => vec![
            PathOp::Move(0.0, 0.0),
            PathOp::Line(w, 0.0),
            PathOp::Line(w, h),
        ],
        "bentConnector3" => {
            let x = w * adj("adj1", 50_000.0);
            vec![
                PathOp::Move(0.0, 0.0),
                PathOp::Line(x, 0.0),
                PathOp::Line(x, h),
                PathOp::Line(w, h),
            ]
        }
        "bentConnector4" => {
            let (x, y) = (w * adj("adj1", 50_000.0), h * adj("adj2", 50_000.0));
            vec![
                PathOp::Move(0.0, 0.0),
                PathOp::Line(x, 0.0),
                PathOp::Line(x, y),
                PathOp::Line(w, y),
                PathOp::Line(w, h),
            ]
        }
        "bentConnector5" => {
            let (x1, y, x3) = (
                w * adj("adj1", 50_000.0),
                h * adj("adj2", 50_000.0),
                w * adj("adj3", 50_000.0),
            );
            vec![
                PathOp::Move(0.0, 0.0),
                PathOp::Line(x1, 0.0),
                PathOp::Line(x1, y),
                PathOp::Line(x3, y),
                PathOp::Line(x3, h),
                PathOp::Line(w, h),
            ]
        }
        "curvedConnector2" => vec![
            PathOp::Move(0.0, 0.0),
            PathOp::Cubic(w / 2.0, 0.0, w, h / 2.0, w, h),
        ],
        "curvedConnector3" => {
            let x2 = w * adj("adj1", 50_000.0);
            vec![
                PathOp::Move(0.0, 0.0),
                PathOp::Cubic(x2 / 2.0, 0.0, x2, h / 4.0, x2, h / 2.0),
                PathOp::Cubic(x2, h * 0.75, (w + x2) / 2.0, h, w, h),
            ]
        }
        name if is_line(name) => vec![PathOp::Move(0.0, 0.0), PathOp::Line(w, h)],
        "roundRect" | "flowChartAlternateProcess" => round_rect(w, h, ss * adj("adj", 16_667.0)),
        "flowChartTerminator" => round_rect(w, h, h / 2.0),
        "ellipse"
        | "flowChartConnector"
        | "donut"
        | "cloud"
        | "smileyFace"
        | "sun"
        | "pie"
        | "chord"
        | "noSmoking"
        | "flowChartOnlineStorage"
        | "heart" => ellipse(0.0, 0.0, w, h),
        "triangle" | "flowChartExtract" => {
            polygon(&[(w * adj("adj", 50_000.0), 0.0), (w, h), (0.0, h)])
        }
        "flowChartMerge" => polygon(&[(0.0, 0.0), (w, 0.0), (w / 2.0, h)]),
        "rtTriangle" => polygon(&[(0.0, 0.0), (0.0, h), (w, h)]),
        "diamond" | "flowChartDecision" => {
            polygon(&[(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)])
        }
        "parallelogram" | "flowChartInputOutput" => {
            let x = ss
                * adj(
                    "adj",
                    if name == "parallelogram" {
                        25_000.0
                    } else {
                        20_000.0
                    },
                );
            polygon(&[(x, 0.0), (w, 0.0), (w - x, h), (0.0, h)])
        }
        "trapezoid" => {
            let x = ss * adj("adj", 25_000.0);
            polygon(&[(0.0, h), (x, 0.0), (w - x, 0.0), (w, h)])
        }
        "pentagon" => polygon(&[
            (w / 2.0, 0.0),
            (w, h * 0.382),
            (w * 0.809, h),
            (w * 0.191, h),
            (0.0, h * 0.382),
        ]),
        "hexagon" | "flowChartPreparation" => {
            let x = ss * adj("adj", 25_000.0);
            polygon(&[
                (x, 0.0),
                (w - x, 0.0),
                (w, h / 2.0),
                (w - x, h),
                (x, h),
                (0.0, h / 2.0),
            ])
        }
        "octagon" => {
            let x = ss * adj("adj", 29_289.0);
            polygon(&[
                (x, 0.0),
                (w - x, 0.0),
                (w, x),
                (w, h - x),
                (w - x, h),
                (x, h),
                (0.0, h - x),
                (0.0, x),
            ])
        }
        "star4" => star(4, 0.25, w, h),
        "star5" => star(5, 0.382, w, h),
        "star6" => star(6, 0.5, w, h),
        "star7" => star(7, 0.55, w, h),
        "star8" => star(8, 0.6, w, h),
        "star10" | "star12" | "star16" | "star24" | "star32" => {
            star(name[4..].parse().unwrap_or(10), 0.75, w, h)
        }
        "rightArrow" | "leftArrow" => {
            let shaft = h * adj("adj1", 50_000.0) / 2.0;
            let head = (ss * adj("adj2", 50_000.0)).min(w);
            let points = [
                (0.0, h / 2.0 - shaft),
                (w - head, h / 2.0 - shaft),
                (w - head, 0.0),
                (w, h / 2.0),
                (w - head, h),
                (w - head, h / 2.0 + shaft),
                (0.0, h / 2.0 + shaft),
            ];
            if name == "leftArrow" {
                polygon(&points.map(|(x, y)| (w - x, y)))
            } else {
                polygon(&points)
            }
        }
        "upArrow" | "downArrow" => {
            let shaft = w * adj("adj1", 50_000.0) / 2.0;
            let head = (ss * adj("adj2", 50_000.0)).min(h);
            let points = [
                (w / 2.0 - shaft, h),
                (w / 2.0 - shaft, head),
                (0.0, head),
                (w / 2.0, 0.0),
                (w, head),
                (w / 2.0 + shaft, head),
                (w / 2.0 + shaft, h),
            ];
            if name == "downArrow" {
                polygon(&points.map(|(x, y)| (x, h - y)))
            } else {
                polygon(&points)
            }
        }
        "leftRightArrow" => {
            let shaft = h * adj("adj1", 50_000.0) / 2.0;
            let head = (ss * adj("adj2", 50_000.0)).min(w / 2.0);
            polygon(&[
                (0.0, h / 2.0),
                (head, 0.0),
                (head, h / 2.0 - shaft),
                (w - head, h / 2.0 - shaft),
                (w - head, 0.0),
                (w, h / 2.0),
                (w - head, h),
                (w - head, h / 2.0 + shaft),
                (head, h / 2.0 + shaft),
                (head, h),
            ])
        }
        "chevron" => {
            let x = (ss * adj("adj", 50_000.0)).min(w);
            polygon(&[
                (0.0, 0.0),
                (w - x, 0.0),
                (w, h / 2.0),
                (w - x, h),
                (0.0, h),
                (x, h / 2.0),
            ])
        }
        "homePlate" | "flowChartOffpageConnector" => {
            let x = (ss * adj("adj", 50_000.0)).min(w);
            polygon(&[(0.0, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (0.0, h)])
        }
        "plus" | "mathPlus" | "flowChartOr" => {
            let x = ss * adj("adj", 25_000.0);
            polygon(&[
                (x, 0.0),
                (w - x, 0.0),
                (w - x, x),
                (w, x),
                (w, h - x),
                (w - x, h - x),
                (w - x, h),
                (x, h),
                (x, h - x),
                (0.0, h - x),
                (0.0, x),
                (x, x),
            ])
        }
        "can" | "flowChartMagneticDisk" => {
            const K: f64 = 0.552_284_75;
            let ry = (ss * adj("adj", 25_000.0) / 2.0).min(h / 2.0);
            let rx = w / 2.0;
            vec![
                PathOp::Move(0.0, ry),
                PathOp::Cubic(0.0, ry - ry * K, rx - rx * K, 0.0, rx, 0.0),
                PathOp::Cubic(rx + rx * K, 0.0, w, ry - ry * K, w, ry),
                PathOp::Line(w, h - ry),
                PathOp::Cubic(w, h - ry + ry * K, rx + rx * K, h, rx, h),
                PathOp::Cubic(rx - rx * K, h, 0.0, h - ry + ry * K, 0.0, h - ry),
                PathOp::Close,
            ]
        }
        "wedgeRectCallout" | "wedgeRoundRectCallout" | "wedgeEllipseCallout" => {
            let tip = (
                w / 2.0 + w * adj("adj1", -20_833.0),
                h / 2.0 + h * adj("adj2", 62_500.0),
            );
            let mut outline = polygon(&callout(w, h, tip));
            if name == "wedgeEllipseCallout" {
                outline = ellipse(0.0, 0.0, w, h);
                outline.pop();
                outline.extend(polygon(&[
                    (w / 2.0 - w / 8.0, h / 2.0),
                    tip,
                    (w / 2.0 + w / 8.0, h / 2.0),
                ]));
            }
            outline
        }
        _ => polygon(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]),
    }
}

/// A rectangle with a pointer out to `tip` from its nearest edge.
fn callout(w: f64, h: f64, tip: (f64, f64)) -> Vec<(f64, f64)> {
    let inside = (0.0..=w).contains(&tip.0) && (0.0..=h).contains(&tip.1);
    let corners = vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
    if inside {
        return corners;
    }
    let (dx, dy) = ((tip.0 - w / 2.0) / w, (tip.1 - h / 2.0) / h);
    let base = |from: f64, length: f64| {
        let centre = from.clamp(length * 0.25, length * 0.75);
        (centre - length / 8.0, centre + length / 8.0)
    };
    if dy.abs() >= dx.abs() {
        let (a, b) = base(tip.0, w);
        if dy > 0.0 {
            vec![(0.0, 0.0), (w, 0.0), (w, h), (b, h), tip, (a, h), (0.0, h)]
        } else {
            vec![
                (0.0, 0.0),
                (a, 0.0),
                tip,
                (b, 0.0),
                (w, 0.0),
                (w, h),
                (0.0, h),
            ]
        }
    } else {
        let (a, b) = base(tip.1, h);
        if dx > 0.0 {
            vec![(0.0, 0.0), (w, 0.0), (w, a), tip, (w, b), (w, h), (0.0, h)]
        } else {
            vec![
                (0.0, 0.0),
                (w, 0.0),
                (w, h),
                (0.0, h),
                (0.0, b),
                tip,
                (0.0, a),
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_colours_take_their_transforms() {
        let styles = StyleTable::default();
        let accent = DrawingColor {
            scheme: Some("accent1".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_color(&accent, &styles, None),
            Some([68, 114, 196, 255])
        );
        let darker = DrawingColor {
            transforms: vec![("shade".into(), 50_000)],
            ..accent.clone()
        };
        let [r, g, b, _] = resolve_color(&darker, &styles, None).unwrap();
        assert!(r < 68 && g < 114 && b < 196);
        let lighter = DrawingColor {
            transforms: vec![("lumMod".into(), 20_000), ("lumOff".into(), 80_000)],
            ..accent
        };
        let [r, g, b, _] = resolve_color(&lighter, &styles, None).unwrap();
        assert!(r > 200 && g > 200 && b > 230);
    }

    #[test]
    fn presets_stay_in_their_box() {
        for name in [
            "rect",
            "roundRect",
            "ellipse",
            "triangle",
            "diamond",
            "rightArrow",
            "leftArrow",
            "upArrow",
            "downArrow",
            "star5",
            "chevron",
            "hexagon",
            "plus",
            "can",
            "wedgeRectCallout",
            "bentConnector2",
            "bentConnector3",
            "bentConnector4",
            "bentConnector5",
            "curvedConnector2",
            "curvedConnector3",
        ] {
            for op in preset_path(name, 100.0, 50.0, &[]) {
                let points = match op {
                    PathOp::Move(x, y) | PathOp::Line(x, y) => vec![(x, y)],
                    PathOp::Cubic(a, b, c, d, e, f) => vec![(a, b), (c, d), (e, f)],
                    PathOp::Close => Vec::new(),
                };
                for (x, y) in points {
                    let callout = name == "wedgeRectCallout";
                    assert!(
                        callout || ((-0.01..=100.01).contains(&x) && (-0.01..=50.01).contains(&y)),
                        "{name}: ({x}, {y})"
                    );
                }
            }
        }
    }

    /// An elbow turns halfway across, or where its adjustment says; arrow
    /// heads point along the leg they end, not corner to corner.
    #[test]
    fn connectors_bend_and_their_heads_follow_the_end_legs() {
        assert_eq!(
            preset_path("bentConnector3", 100.0, 50.0, &[]),
            [
                PathOp::Move(0.0, 0.0),
                PathOp::Line(50.0, 0.0),
                PathOp::Line(50.0, 50.0),
                PathOp::Line(100.0, 50.0),
            ]
        );
        let adjusted = preset_path("bentConnector3", 100.0, 50.0, &[("adj1".into(), 25_000)]);
        assert_eq!(adjusted[1], PathOp::Line(25.0, 0.0));
        assert_eq!(
            line_ends(&preset_path("bentConnector3", 100.0, 50.0, &[])),
            Some((((0.0, 0.0), (50.0, 0.0)), ((50.0, 50.0), (100.0, 50.0))))
        );
        // A curve leaves and arrives level, as its control points say.
        assert_eq!(
            line_ends(&preset_path("curvedConnector3", 100.0, 50.0, &[])),
            Some((((0.0, 0.0), (25.0, 0.0)), ((75.0, 50.0), (100.0, 50.0))))
        );
        // A straight line's heads are as they were: corner to corner.
        assert_eq!(
            line_ends(&preset_path("straightConnector1", 100.0, 50.0, &[])),
            Some((((0.0, 0.0), (100.0, 50.0)), ((0.0, 0.0), (100.0, 50.0))))
        );
    }
}
