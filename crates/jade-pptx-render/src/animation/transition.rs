//! Slide transitions as pixel blends of the outgoing and incoming slides'
//! bitmaps (RGBA, premultiplied or not — both go through unchanged).

use jade_pptx_layout::Rect;
use jade_pptx_reader::animation::TransitionEffect;

use crate::animation::reveal::{reveal_polygons, Polygon};

/// The two slides, rendered at the same size.
#[derive(Debug, Clone, Copy)]
pub struct TransitionFrame<'a> {
    pub from: &'a [u8],
    pub to: &'a [u8],
    pub width: u32,
    pub height: u32,
}

/// How a transition moves the slides, reduced to what can be drawn flat.
#[derive(Debug, Clone, PartialEq)]
enum Plan {
    /// The new slide shows through a mask.
    Mask(String),
    Fade {
        through_black: bool,
    },
    Flash,
    Cut {
        through_black: bool,
    },
    /// (dx, dy): the direction of travel.
    Push(f64, f64),
    Cover(f64, f64),
    Uncover(f64, f64),
    Zoom {
        inward: bool,
    },
}

fn travel(dir: Option<&str>) -> (f64, f64) {
    match dir.unwrap_or("l") {
        "r" => (1.0, 0.0),
        "u" => (0.0, -1.0),
        "d" => (0.0, 1.0),
        "lu" => (-1.0, -1.0),
        "ru" => (1.0, -1.0),
        "ld" => (-1.0, 1.0),
        "rd" => (1.0, 1.0),
        _ => (-1.0, 0.0),
    }
}

fn wipe_filter(dir: Option<&str>) -> String {
    let side = match dir.unwrap_or("l") {
        "r" => "right",
        "u" => "up",
        "d" => "down",
        _ => "left",
    };
    format!("wipe({side})")
}

fn plan(effect: &TransitionEffect) -> Plan {
    let dir = effect.attr("dir");
    let black = effect
        .attr("thruBlk")
        .is_some_and(|value| value == "1" || value == "true");
    let horizontal = |value: Option<&str>| value.unwrap_or("horz") == "horz";
    match effect.name.as_str() {
        "fade" => Plan::Fade {
            through_black: black,
        },
        "cut" => Plan::Cut {
            through_black: black,
        },
        "flash" => Plan::Flash,
        "push" | "pan" | "gallery" | "conveyor" | "ferris" | "switch" | "flip" | "prism" => {
            Plan::Push(travel(dir.or(Some("l"))).0, travel(dir.or(Some("l"))).1)
        }
        "cover" => {
            let (x, y) = travel(dir);
            Plan::Cover(x, y)
        }
        "pull" => {
            let (x, y) = travel(dir);
            Plan::Uncover(x, y)
        }
        "wipe" => Plan::Mask(wipe_filter(dir)),
        "split" => {
            let orient = if horizontal(effect.attr("orient")) {
                "Horizontal"
            } else {
                "Vertical"
            };
            let way = if effect.attr("dir").unwrap_or("out") == "in" {
                "in"
            } else {
                "out"
            };
            Plan::Mask(format!("barn({way}{orient})"))
        }
        "doors" | "window" => {
            let orient = if effect.attr("dir").unwrap_or("vert") == "horz" {
                "Horizontal"
            } else {
                "Vertical"
            };
            Plan::Mask(format!("barn(out{orient})"))
        }
        "randomBar" => Plan::Mask(format!(
            "randombar({})",
            if horizontal(dir) {
                "horizontal"
            } else {
                "vertical"
            }
        )),
        "blinds" | "comb" => Plan::Mask(format!(
            "blinds({})",
            if horizontal(dir) {
                "horizontal"
            } else {
                "vertical"
            }
        )),
        "checker" => Plan::Mask(format!(
            "checkerboard({})",
            if horizontal(dir) { "across" } else { "down" }
        )),
        "circle" => Plan::Mask("circle(out)".to_string()),
        "diamond" => Plan::Mask("diamond(out)".to_string()),
        "plus" => Plan::Mask("plus(out)".to_string()),
        "wedge" => Plan::Mask("wedge".to_string()),
        "wheel" | "wheelReverse" => {
            Plan::Mask(format!("wheel({})", effect.attr("spokes").unwrap_or("4")))
        }
        "strips" => Plan::Mask(format!(
            "strips({})",
            match dir.unwrap_or("lu") {
                "ru" => "upRight",
                "ld" => "downLeft",
                "rd" => "downRight",
                _ => "upLeft",
            }
        )),
        "zoom" | "newsflash" | "warp" | "flythrough" => Plan::Zoom {
            inward: effect.attr("dir").unwrap_or("in") == "in",
        },
        "reveal" => Plan::Fade {
            through_black: true,
        },
        "shape" => Plan::Mask("circle(out)".to_string()),
        "dissolve" | "vortex" | "ripple" | "honeycomb" | "glitter" | "shred" => {
            Plan::Mask("dissolve".to_string())
        }
        "prstTrans" => match effect.attr("prst").unwrap_or("") {
            "curtains" | "drape" => Plan::Mask("barn(outVertical)".to_string()),
            "wind" | "airplane" | "origami" => Plan::Uncover(-1.0, 0.0),
            "fallOver" | "peelOff" | "pageCurlDouble" | "pageCurlSingle" => Plan::Uncover(0.0, 1.0),
            "crush" | "fracture" => Plan::Mask("dissolve".to_string()),
            _ => Plan::Fade {
                through_black: false,
            },
        },
        // Morph without a morph renderer, and anything unknown, fades.
        _ => Plan::Fade {
            through_black: false,
        },
    }
}

/// The transition `progress` (0..=1) of the way from `frame.from` to
/// `frame.to`, as RGBA.
pub fn compose_transition(
    frame: TransitionFrame<'_>,
    effect: &TransitionEffect,
    progress: f64,
) -> Vec<u8> {
    let p = progress.clamp(0.0, 1.0);
    let (width, height) = (frame.width as usize, frame.height as usize);
    let size = width * height * 4;
    if frame.from.len() < size || frame.to.len() < size {
        return frame.to.to_vec();
    }
    let mut out = vec![0u8; size];
    match plan(effect) {
        Plan::Fade { through_black } => {
            if through_black {
                let (source, weight) = if p < 0.5 {
                    (frame.from, 1.0 - p * 2.0)
                } else {
                    (frame.to, p * 2.0 - 1.0)
                };
                blend_black(&mut out, source, weight);
            } else {
                blend(&mut out, frame.from, frame.to, p);
            }
        }
        Plan::Cut { through_black } => {
            if through_black && p < 0.5 {
                blend_black(&mut out, frame.from, 0.0);
            } else {
                out.copy_from_slice(&frame.to[..size]);
            }
        }
        Plan::Flash => {
            if p < 0.5 {
                blend_white(&mut out, frame.from, p * 2.0);
            } else {
                blend_white(&mut out, frame.to, 2.0 - p * 2.0);
            }
        }
        Plan::Mask(filter) => {
            out.copy_from_slice(&frame.from[..size]);
            let rect = Rect {
                x: 0.0,
                y: 0.0,
                width: width as f64,
                height: height as f64,
            };
            let polygons = reveal_polygons(rect, &filter, p);
            let mask = rasterize(&polygons, width, height);
            for (index, covered) in mask.into_iter().enumerate() {
                if covered {
                    let at = index * 4;
                    out[at..at + 4].copy_from_slice(&frame.to[at..at + 4]);
                }
            }
        }
        Plan::Push(dx, dy) => {
            let (ox, oy) = (dx * p * width as f64, dy * p * height as f64);
            shifted(&mut out, frame.from, width, height, ox, oy, false);
            let (nx, ny) = (ox - dx * width as f64, oy - dy * height as f64);
            shifted(&mut out, frame.to, width, height, nx, ny, true);
        }
        Plan::Cover(dx, dy) => {
            out.copy_from_slice(&frame.from[..size]);
            let (nx, ny) = (
                dx * (p - 1.0) * width as f64,
                dy * (p - 1.0) * height as f64,
            );
            shifted(&mut out, frame.to, width, height, nx, ny, true);
        }
        Plan::Uncover(dx, dy) => {
            out.copy_from_slice(&frame.to[..size]);
            let (ox, oy) = (dx * p * width as f64, dy * p * height as f64);
            shifted(&mut out, frame.from, width, height, ox, oy, true);
        }
        Plan::Zoom { inward } => {
            // The new slide grows from the centre over the old one.
            out.copy_from_slice(&frame.from[..size]);
            let scale = if inward {
                p.max(0.01)
            } else {
                1.0 + (1.0 - p) * 2.0
            };
            let (cx, cy) = (width as f64 / 2.0, height as f64 / 2.0);
            for y in 0..height {
                for x in 0..width {
                    let sx = cx + (x as f64 + 0.5 - cx) / scale;
                    let sy = cy + (y as f64 + 0.5 - cy) / scale;
                    if sx < 0.0 || sy < 0.0 || sx >= width as f64 || sy >= height as f64 {
                        continue;
                    }
                    let source = (sy as usize * width + sx as usize) * 4;
                    let target = (y * width + x) * 4;
                    let weight = if inward { 1.0 } else { p };
                    for channel in 0..4 {
                        let old = f64::from(out[target + channel]);
                        let new = f64::from(frame.to[source + channel]);
                        out[target + channel] = (old + (new - old) * weight).round() as u8;
                    }
                }
            }
        }
    }
    out
}

fn blend(out: &mut [u8], a: &[u8], b: &[u8], t: f64) {
    let weight = (t * 256.0).round() as u32;
    for ((out, &a), &b) in out.iter_mut().zip(a).zip(b) {
        *out = ((u32::from(a) * (256 - weight) + u32::from(b) * weight) >> 8) as u8;
    }
}

fn blend_black(out: &mut [u8], source: &[u8], weight: f64) {
    for (index, (out, &value)) in out.iter_mut().zip(source).enumerate() {
        *out = if index % 4 == 3 {
            value
        } else {
            (f64::from(value) * weight).round() as u8
        };
    }
}

fn blend_white(out: &mut [u8], source: &[u8], weight: f64) {
    for (index, (out, &value)) in out.iter_mut().zip(source).enumerate() {
        *out = if index % 4 == 3 {
            value
        } else {
            (f64::from(value) + (255.0 - f64::from(value)) * weight).round() as u8
        };
    }
}

/// Copies `source` moved by (ox, oy) px into `out`; outside it, `out`
/// keeps what it has unless `overlay` is false (then it turns black).
fn shifted(
    out: &mut [u8],
    source: &[u8],
    width: usize,
    height: usize,
    ox: f64,
    oy: f64,
    overlay: bool,
) {
    let (ox, oy) = (ox.round() as i64, oy.round() as i64);
    for y in 0..height as i64 {
        let sy = y - oy;
        if sy < 0 || sy >= height as i64 {
            if !overlay {
                let start = y as usize * width * 4;
                out[start..start + width * 4].fill(0);
            }
            continue;
        }
        for x in 0..width as i64 {
            let sx = x - ox;
            let target = (y as usize * width + x as usize) * 4;
            if sx < 0 || sx >= width as i64 {
                if !overlay {
                    out[target..target + 4].fill(0);
                }
                continue;
            }
            let at = (sy as usize * width + sx as usize) * 4;
            out[target..target + 4].copy_from_slice(&source[at..at + 4]);
        }
    }
}

/// Which pixel centres the polygons cover (nonzero winding), row by row.
pub(crate) fn rasterize(polygons: &[Polygon], width: usize, height: usize) -> Vec<bool> {
    let mut mask = vec![false; width * height];
    let edges: Vec<((f64, f64), (f64, f64))> = polygons
        .iter()
        .flat_map(|polygon| {
            (0..polygon.len()).map(move |i| (polygon[i], polygon[(i + 1) % polygon.len()]))
        })
        .filter(|(a, b)| a.1 != b.1)
        .collect();
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    for y in 0..height {
        let cy = y as f64 + 0.5;
        crossings.clear();
        for &(a, b) in &edges {
            let (low, high, winding) = if a.1 < b.1 { (a, b, 1) } else { (b, a, -1) };
            if cy < low.1 || cy >= high.1 {
                continue;
            }
            let t = (cy - low.1) / (high.1 - low.1);
            crossings.push((low.0 + (high.0 - low.0) * t, winding));
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        for pair in 0..crossings.len() {
            let (x, w) = crossings[pair];
            let before = winding;
            winding += w;
            if before == 0 && winding != 0 {
                // A span opens here; it closes at the crossing that brings
                // the winding back to zero.
                let mut level = winding;
                let mut end = width as f64;
                for &(x2, w2) in &crossings[pair + 1..] {
                    level += w2;
                    if level == 0 {
                        end = x2;
                        break;
                    }
                }
                let from = (x - 0.5).ceil().max(0.0) as usize;
                let to = ((end - 0.5).ceil().max(0.0) as usize).min(width);
                for column in from..to {
                    mask[y * width + column] = true;
                }
            }
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_pptx_reader::animation::TransitionSchema;

    fn slides() -> (Vec<u8>, Vec<u8>) {
        (vec![0u8; 4 * 4 * 4], vec![255u8; 4 * 4 * 4])
    }

    fn white_pixels(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|pixel| pixel[0] == 255).count()
    }

    #[test]
    fn transitions_move_between_the_slides() {
        let (from, to) = slides();
        let frame = TransitionFrame {
            from: &from,
            to: &to,
            width: 4,
            height: 4,
        };
        let fade = TransitionEffect::new(TransitionSchema::Base, "fade");
        assert!(compose_transition(frame, &fade, 0.5)[0].abs_diff(128) <= 1);
        let wipe = TransitionEffect::new(TransitionSchema::Base, "wipe").with_attr("dir", "d");
        let half = compose_transition(frame, &wipe, 0.5);
        assert_eq!(white_pixels(&half), 8);
        assert_eq!(half[0], 255, "a downward wipe shows the top first");
        let push = TransitionEffect::new(TransitionSchema::Base, "push").with_attr("dir", "u");
        assert_eq!(white_pixels(&compose_transition(frame, &push, 0.5)), 8);
        let circle = TransitionEffect::new(TransitionSchema::Base, "circle");
        assert_eq!(white_pixels(&compose_transition(frame, &circle, 1.0)), 16);
        assert_eq!(white_pixels(&compose_transition(frame, &circle, 0.0)), 0);
        let vortex = TransitionEffect::new(TransitionSchema::P14, "vortex");
        assert_eq!(white_pixels(&compose_transition(frame, &vortex, 1.0)), 16);
    }
}
