//! Custom geometry: `<a:custGeom>` paths flattened to normalized polylines.
//!
//! Curves are sampled rather than kept, because the painter path is built
//! from `move_to`/`line_to` and a freeform's fidelity at slide scale does
//! not need more than a handful of segments per curve.

use jade_pptx_reader::{CustomGeometry, PathCommand};

use crate::{PathFill, PlacedGeometry, PlacedPath, PlacedSubpath};

/// Segments per curve or arc.
const CURVE_SEGMENTS: usize = 8;

/// The placed geometry for a custom outline, or `None` when it has no
/// drawable path so the caller can fall back to the bounding rect.
///
/// Outlines that are all filled, stroked and closed stay `Path`; anything
/// else — an open curve, a stroke-only or fill-only path — becomes `Paths`
/// so the renderer leaves it open and honours each path's flags.
pub fn custom_geometry(custom: &CustomGeometry) -> Option<PlacedGeometry> {
    let mut placed: Vec<PlacedPath> = Vec::new();
    for path in &custom.paths {
        let mut subpaths: Vec<PlacedSubpath> = Vec::new();
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
                    flush(&mut subpaths, &mut current, false);
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
                    // Angles are visual in the path's own units, so the
                    // arc samples there and normalizes after.
                    let mut sampled = Vec::new();
                    let end = crate::preset_shapes::arc_to(
                        &mut sampled,
                        (cursor.0 * w, cursor.1 * h),
                        *width_radius as f64,
                        *height_radius as f64,
                        *start_60000ths as f64,
                        *sweep_60000ths as f64,
                    );
                    current.extend(sampled.into_iter().map(|(x, y)| (x / w, y / h)));
                    cursor = (end.0 / w, end.1 / h);
                }
                PathCommand::Close => flush(&mut subpaths, &mut current, true),
            }
        }
        flush(&mut subpaths, &mut current, false);
        if !subpaths.is_empty() {
            placed.push(PlacedPath {
                subpaths,
                fill: if path.filled {
                    PathFill::Norm
                } else {
                    PathFill::None
                },
                stroked: path.stroked,
            });
        }
    }
    if placed.is_empty() {
        return None;
    }
    let classic = placed
        .iter()
        .all(|p| p.fill == PathFill::Norm && p.stroked && p.subpaths.iter().all(|sub| sub.closed));
    Some(if classic {
        PlacedGeometry::Path(
            placed
                .into_iter()
                .flat_map(|p| p.subpaths.into_iter().map(|sub| sub.points))
                .collect(),
        )
    } else {
        PlacedGeometry::Paths(placed)
    })
}

/// Moves a finished subpath out; one point alone draws nothing.
fn flush(subpaths: &mut Vec<PlacedSubpath>, current: &mut Vec<(f64, f64)>, closed: bool) {
    if current.len() >= 2 {
        subpaths.push(PlacedSubpath {
            points: std::mem::take(current),
            closed,
        });
    } else {
        current.clear();
    }
}
