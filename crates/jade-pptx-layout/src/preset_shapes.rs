//! ECMA-376 preset geometries evaluated from their shape-guide definitions.
//!
//! `preset_shapes.txt` is `presetShapeDefinitions.xml` (ECMA-376 Part 1,
//! Annex D) flattened to one record per line:
//!
//! ```text
//! @name                 a shape
//! a name fmla           an <avLst> default adjustment
//! g name fmla           a <gdLst> guide
//! r l t r b             the text rectangle
//! p w h fill stroke     a path; then M/L/Q/C points, A wR hR stAng swAng, Z
//! ```
//!
//! Guides are evaluated in the shape's own pixel space, the way PowerPoint
//! evaluates them in EMUs: every formula is scale-linear, so the outline and
//! text rectangle come out the same.

use std::collections::HashMap;
use std::sync::OnceLock;

use jade_pptx_reader::ShapeGuides;

use crate::{PathFill, PlacedPath, PlacedSubpath, Rect};

const DATA: &str = include_str!("preset_shapes.txt");

/// Segments per Bézier curve.
const CURVE_SEGMENTS: usize = 16;
/// Largest angle one arc segment spans, in radians.
const ARC_STEP: f64 = std::f64::consts::PI / 24.0;

struct Definition {
    adjustments: Vec<(&'static str, &'static str)>,
    guides: Vec<(&'static str, &'static str)>,
    text: Option<[&'static str; 4]>,
    paths: Vec<PathDefinition>,
}

struct PathDefinition {
    width: f64,
    height: f64,
    fill: PathFill,
    stroked: bool,
    commands: Vec<(char, Vec<&'static str>)>,
}

fn definitions() -> &'static HashMap<&'static str, Definition> {
    static DEFINITIONS: OnceLock<HashMap<&'static str, Definition>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let mut map = HashMap::new();
        let mut current: Option<(&str, Definition)> = None;
        for line in DATA.lines() {
            if let Some(name) = line.strip_prefix('@') {
                if let Some((name, def)) = current.take() {
                    map.insert(name, def);
                }
                current = Some((
                    name,
                    Definition {
                        adjustments: Vec::new(),
                        guides: Vec::new(),
                        text: None,
                        paths: Vec::new(),
                    },
                ));
                continue;
            }
            let Some((_, def)) = current.as_mut() else {
                continue;
            };
            let mut parts = line.split(' ');
            let Some(kind) = parts.next() else { continue };
            match kind {
                "a" | "g" => {
                    let Some((name, fmla)) = line[2..].split_once(' ') else {
                        continue;
                    };
                    if kind == "a" {
                        def.adjustments.push((name, fmla));
                    } else {
                        def.guides.push((name, fmla));
                    }
                }
                "r" => {
                    let v: Vec<&str> = parts.collect();
                    if v.len() == 4 {
                        def.text = Some([v[0], v[1], v[2], v[3]]);
                    }
                }
                "p" => {
                    let v: Vec<&str> = parts.collect();
                    def.paths.push(PathDefinition {
                        width: v.first().and_then(|s| s.parse().ok()).unwrap_or(0.0),
                        height: v.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0),
                        fill: match v.get(2).copied() {
                            Some("none") => PathFill::None,
                            Some("lighten") => PathFill::Lighten,
                            Some("lightenLess") => PathFill::LightenLess,
                            Some("darken") => PathFill::Darken,
                            Some("darkenLess") => PathFill::DarkenLess,
                            _ => PathFill::Norm,
                        },
                        stroked: !matches!(v.get(3).copied(), Some("false") | Some("0")),
                        commands: Vec::new(),
                    });
                }
                "M" | "L" | "Q" | "C" | "A" | "Z" => {
                    if let Some(path) = def.paths.last_mut() {
                        path.commands
                            .push((kind.chars().next().unwrap_or('Z'), parts.collect()));
                    }
                }
                _ => {}
            }
        }
        if let Some((name, def)) = current.take() {
            map.insert(name, def);
        }
        map
    })
}

/// Whether `name` is an ECMA-376 preset this module can evaluate.
pub fn is_known(name: &str) -> bool {
    definitions().contains_key(name)
}

/// The outline of preset `name` at `w` × `h` with `adjustments` (an
/// `<a:avLst>`'s `(name, value)` pairs): its paths in normalized shape
/// coordinates and its text rectangle in pixels. For any DrawingML host —
/// Word's shapes evaluate the same definitions PowerPoint's do.
pub fn outline(
    name: &str,
    adjustments: &[(String, i64)],
    w: f64,
    h: f64,
) -> Option<(Vec<PlacedPath>, Rect)> {
    evaluate(name, adjustments, w, h).map(|preset| (preset.paths, preset.text))
}

/// One evaluated preset: its paths in normalized shape coordinates (0..1
/// is the shape's box; some presets reach outside it) and its text rect in
/// shape pixels.
pub(crate) struct EvaluatedPreset {
    pub paths: Vec<PlacedPath>,
    pub text: Rect,
}

/// Evaluates preset `name` at `w` × `h` pixels with the shape's own
/// `<a:avLst>` overrides.
pub(crate) fn evaluate(
    name: &str,
    adjustments: &[(String, i64)],
    w: f64,
    h: f64,
) -> Option<EvaluatedPreset> {
    if !(w.is_finite() && h.is_finite()) || w < 0.0 || h < 0.0 {
        return None;
    }
    let def = definitions().get(name)?;
    let mut guides = ShapeGuides::new(w, h);
    for (adj_name, fmla) in &def.adjustments {
        let value = adjustments
            .iter()
            .find(|(key, _)| key == adj_name)
            // A lone `adj` override also answers a preset whose first
            // handle is `adj1`, the way older writers name it.
            .or_else(|| {
                (*adj_name == "adj1" || *adj_name == "adj")
                    .then(|| {
                        adjustments
                            .iter()
                            .find(|(key, _)| key == "adj" || key == "adj1")
                    })
                    .flatten()
            })
            .map(|(_, v)| *v as f64)
            .unwrap_or_else(|| guides.eval(fmla));
        guides.set(adj_name, value);
    }
    for (guide_name, fmla) in &def.guides {
        guides.define(guide_name, fmla);
    }

    let text = match def.text {
        Some([l, t, r, b]) => {
            let (l, t, r, b) = (guides.arg(l), guides.arg(t), guides.arg(r), guides.arg(b));
            Rect {
                x: l,
                y: t,
                width: (r - l).max(0.0),
                height: (b - t).max(0.0),
            }
        }
        None => Rect {
            x: 0.0,
            y: 0.0,
            width: w,
            height: h,
        },
    };

    let norm = |x: f64, y: f64| {
        (
            if w > 0.0 { x / w } else { 0.0 },
            if h > 0.0 { y / h } else { 0.0 },
        )
    };
    let mut paths = Vec::new();
    for path in &def.paths {
        // A path with its own w/h maps that space onto the shape's box.
        let sx = if path.width > 0.0 {
            w / path.width
        } else {
            1.0
        };
        let sy = if path.height > 0.0 {
            h / path.height
        } else {
            1.0
        };
        let point = |args: &[&str], i: usize| -> (f64, f64) {
            let x = args.get(2 * i).map_or(0.0, |t| guides.arg(t)) * sx;
            let y = args.get(2 * i + 1).map_or(0.0, |t| guides.arg(t)) * sy;
            (x, y)
        };
        let mut subpaths: Vec<PlacedSubpath> = Vec::new();
        let mut current: Vec<(f64, f64)> = Vec::new();
        let mut cursor = (0.0, 0.0);
        let finish = |subpaths: &mut Vec<PlacedSubpath>, current: &mut Vec<(f64, f64)>, closed| {
            if current.len() >= 2 {
                subpaths.push(PlacedSubpath {
                    points: current.iter().map(|&(x, y)| norm(x, y)).collect(),
                    closed,
                });
            }
            current.clear();
        };
        for (kind, args) in &path.commands {
            match kind {
                'M' => {
                    finish(&mut subpaths, &mut current, false);
                    cursor = point(args, 0);
                    current.push(cursor);
                }
                'L' => {
                    if current.is_empty() {
                        current.push(cursor);
                    }
                    cursor = point(args, 0);
                    current.push(cursor);
                }
                'Q' => {
                    if current.is_empty() {
                        current.push(cursor);
                    }
                    let (c, end) = (point(args, 0), point(args, 1));
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
                'C' => {
                    if current.is_empty() {
                        current.push(cursor);
                    }
                    let (c1, c2, end) = (point(args, 0), point(args, 1), point(args, 2));
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
                'A' => {
                    if current.is_empty() {
                        current.push(cursor);
                    }
                    let arg = |i: usize| args.get(i).map_or(0.0, |t| guides.arg(t));
                    let (wr, hr) = (arg(0) * sx, arg(1) * sy);
                    cursor = arc_to(&mut current, cursor, wr, hr, arg(2), arg(3));
                }
                'Z' => {
                    finish(&mut subpaths, &mut current, true);
                }
                _ => {}
            }
        }
        finish(&mut subpaths, &mut current, false);
        if !subpaths.is_empty() {
            paths.push(PlacedPath {
                subpaths,
                fill: path.fill,
                stroked: path.stroked,
            });
        }
    }
    Some(EvaluatedPreset { paths, text })
}

/// Appends a DrawingML `arcTo` from `cursor` and returns its end point.
///
/// `stAng`/`swAng` are visual angles on the ellipse (60000ths of a degree,
/// clockwise in y-down space); they convert to the parametric angle before
/// sampling so a squashed ellipse bends where PowerPoint bends it.
pub(crate) fn arc_to(
    points: &mut Vec<(f64, f64)>,
    cursor: (f64, f64),
    wr: f64,
    hr: f64,
    start_60000ths: f64,
    sweep_60000ths: f64,
) -> (f64, f64) {
    if wr == 0.0 && hr == 0.0 {
        return cursor;
    }
    let visual_start = (start_60000ths / 60_000.0).to_radians();
    let sweep = (sweep_60000ths / 60_000.0).to_radians();
    let param = |a: f64| (wr * a.sin()).atan2(hr * a.cos());
    let t1 = param(visual_start);
    let mut dt = param(visual_start + sweep) - t1;
    if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
        dt = std::f64::consts::TAU * sweep.signum();
    } else if sweep > 0.0 && dt < 0.0 {
        dt += std::f64::consts::TAU;
    } else if sweep < 0.0 && dt > 0.0 {
        dt -= std::f64::consts::TAU;
    }
    let center = (cursor.0 - wr * t1.cos(), cursor.1 - hr * t1.sin());
    let steps = ((dt.abs() / ARC_STEP).ceil() as usize).max(1);
    let mut end = cursor;
    for step in 1..=steps {
        let t = t1 + dt * step as f64 / steps as f64;
        end = (center.0 + wr * t.cos(), center.1 + hr * t.sin());
        points.push(end);
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_evaluates_to_finite_paths() {
        assert_eq!(definitions().len(), 187);
        for name in definitions().keys() {
            let preset = evaluate(name, &[], 200.0, 120.0).expect(name);
            assert!(!preset.paths.is_empty(), "{name} has no paths");
            for path in &preset.paths {
                for sub in &path.subpaths {
                    assert!(
                        sub.points
                            .iter()
                            .all(|(x, y)| x.is_finite() && y.is_finite()),
                        "{name} has a non-finite point"
                    );
                }
            }
            let t = preset.text;
            assert!(t.x.is_finite() && t.y.is_finite() && t.width >= 0.0 && t.height >= 0.0);
        }
    }

    #[test]
    fn adjustments_override_the_defaults() {
        // rightArrow adj1 is the shaft thickness: 50% of the height by default.
        let thin = evaluate("rightArrow", &[("adj1".into(), 20_000)], 100.0, 100.0).unwrap();
        let ys: Vec<f64> = thin.paths[0].subpaths[0]
            .points
            .iter()
            .map(|p| p.1)
            .collect();
        assert!(ys.iter().any(|y| (y - 0.4).abs() < 1e-9));
    }

    #[test]
    fn a_quarter_arc_on_a_flat_ellipse_ends_where_the_visual_angle_says() {
        let mut points = Vec::new();
        let end = arc_to(&mut points, (100.0, 0.0), 100.0, 50.0, 0.0, 5_400_000.0);
        assert!((end.0 - 0.0).abs() < 1e-9 && (end.1 - 50.0).abs() < 1e-9);
    }
}
