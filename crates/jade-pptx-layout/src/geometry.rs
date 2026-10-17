//! Custom geometry: `<a:custGeom>` paths flattened to normalized polylines.
//!
//! Curves are sampled rather than kept, because the painter path is built
//! from `move_to`/`line_to` and a freeform's fidelity at slide scale does
//! not need more than a handful of segments per curve.

use jade_pptx_reader::{CustomGeometry, PathCommand};

use crate::PlacedGeometry;

/// Segments per curve or arc.
const CURVE_SEGMENTS: usize = 8;

/// The placed geometry for a custom outline, or `None` when it has no
/// drawable path so the caller can fall back to the bounding rect.
pub fn custom_geometry(custom: &CustomGeometry) -> Option<PlacedGeometry> {
    let mut subpaths: Vec<Vec<(f64, f64)>> = Vec::new();
    for path in &custom.paths {
        if path.width <= 0 || path.height <= 0 {
            continue;
        }
        let (w, h) = (path.width as f64, path.height as f64);
        let norm = |(x, y): (i64, i64)| (x as f64 / w, y as f64 / h);
        let mut current: Vec<(f64, f64)> = Vec::new();
        let mut cursor = (0.0, 0.0);
        for command in &path.commands {
            match command {
                PathCommand::MoveTo(x, y) => {
                    flush(&mut subpaths, &mut current);
                    cursor = norm((*x, *y));
                    current.push(cursor);
                }
                PathCommand::LineTo(x, y) => {
                    cursor = norm((*x, *y));
                    current.push(cursor);
                }
                PathCommand::QuadTo { control, to } => {
                    let c = norm(*control);
                    let end = norm(*to);
                    let start = cursor;
                    for step in 1..=CURVE_SEGMENTS {
                        let t = step as f64 / CURVE_SEGMENTS as f64;
                        let u = 1.0 - t;
                        current.push((
                            u * u * start.0 + 2.0 * u * t * c.0 + t * t * end.0,
                            u * u * start.1 + 2.0 * u * t * c.1 + t * t * end.1,
                        ));
                    }
                    cursor = end;
                }
                PathCommand::CubicTo {
                    control1,
                    control2,
                    to,
                } => {
                    let c1 = norm(*control1);
                    let c2 = norm(*control2);
                    let end = norm(*to);
                    let start = cursor;
                    for step in 1..=CURVE_SEGMENTS {
                        let t = step as f64 / CURVE_SEGMENTS as f64;
                        let u = 1.0 - t;
                        current.push((
                            u * u * u * start.0
                                + 3.0 * u * u * t * c1.0
                                + 3.0 * u * t * t * c2.0
                                + t * t * t * end.0,
                            u * u * u * start.1
                                + 3.0 * u * u * t * c1.1
                                + 3.0 * u * t * t * c2.1
                                + t * t * t * end.1,
                        ));
                    }
                    cursor = end;
                }
                PathCommand::ArcTo {
                    width_radius,
                    height_radius,
                    start_60000ths,
                    sweep_60000ths,
                } => {
                    // The arc starts at the current point, which lies on the
                    // ellipse at the start angle; the centre follows from it.
                    let rx = *width_radius as f64 / w;
                    let ry = *height_radius as f64 / h;
                    let start = (*start_60000ths as f64 / 60_000.0).to_radians();
                    let sweep = (*sweep_60000ths as f64 / 60_000.0).to_radians();
                    let center = (cursor.0 - rx * start.cos(), cursor.1 - ry * start.sin());
                    for step in 1..=CURVE_SEGMENTS {
                        let angle = start + sweep * step as f64 / CURVE_SEGMENTS as f64;
                        current.push((center.0 + rx * angle.cos(), center.1 + ry * angle.sin()));
                    }
                    cursor = *current.last().unwrap_or(&cursor);
                }
                PathCommand::Close => flush(&mut subpaths, &mut current),
            }
        }
        flush(&mut subpaths, &mut current);
    }
    if subpaths.is_empty() {
        None
    } else {
        Some(PlacedGeometry::Path(subpaths))
    }
}

/// Moves a finished subpath out; one point alone draws nothing.
fn flush(subpaths: &mut Vec<Vec<(f64, f64)>>, current: &mut Vec<(f64, f64)>) {
    if current.len() >= 2 {
        subpaths.push(std::mem::take(current));
    } else {
        current.clear();
    }
}
