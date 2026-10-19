use crate::{PlacedGeometry, Rect};

pub(crate) struct PresetLayout {
    pub geometry: PlacedGeometry,
    pub text: Rect,
}
impl PresetLayout {
    pub fn height_for_content(&self, width: f64, height: f64, content_height: f64) -> f64 {
        if self.geometry == PlacedGeometry::Rect {
            return content_height;
        }
        let text_height = content_height - (height - self.text.height);
        if let PlacedGeometry::RoundRect { adjust } = self.geometry {
            let margin = 2.0 * adjust * 0.29289;
            let shorter = text_height / (1.0 - margin);
            if shorter <= width {
                shorter
            } else {
                text_height + width * margin
            }
        } else if self.text.height > 0.0 {
            text_height * height / self.text.height
        } else {
            content_height
        }
    }
}

/// DrawingML preset outlines and their `rect` guides use the same adjustments.
pub(crate) fn resolve(
    preset: Option<&str>,
    adjustments: &[(String, i64)],
    w: f64,
    h: f64,
) -> Option<PresetLayout> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let guide = |name: &str, default: f64| {
        adjustments
            .iter()
            .find(|(key, _)| key == name)
            .map_or(default, |(_, value)| *value as f64 / 100_000.0)
    };
    let rect = |l: f64, t: f64, r: f64, b: f64| Rect {
        x: l,
        y: t,
        width: r - l,
        height: b - t,
    };
    let (geometry, text) = match preset? {
        "rect" => (PlacedGeometry::Rect, rect(0.0, 0.0, w, h)),
        "ellipse" => {
            let inset = (1.0 - std::f64::consts::FRAC_1_SQRT_2) / 2.0;
            (
                PlacedGeometry::Ellipse,
                rect(w * inset, h * inset, w * (1.0 - inset), h * (1.0 - inset)),
            )
        }
        "roundRect" => {
            let adjust = guide("adj", 0.16667).clamp(0.0, 0.5);
            let inset = w.min(h) * adjust * 0.29289;
            (
                PlacedGeometry::RoundRect { adjust },
                rect(inset, inset, w - inset, h - inset),
            )
        }
        "triangle" => {
            let apex = guide("adj", 0.5).clamp(0.0, 1.0);
            (
                PlacedGeometry::Polygon(vec![(apex, 0.0), (1.0, 1.0), (0.0, 1.0)]),
                rect(w * apex / 2.0, h / 2.0, w * (apex + 1.0) / 2.0, h),
            )
        }
        "diamond" => (
            PlacedGeometry::Polygon(vec![(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]),
            rect(w / 4.0, h / 4.0, w * 3.0 / 4.0, h * 3.0 / 4.0),
        ),
        "hexagon" => {
            let max = 0.5 * w / w.min(h);
            let adjust = guide("adj", 0.25).clamp(0.0, max);
            let x = w.min(h) * adjust / w;
            let dy = guide("vf", 1.1547) * std::f64::consts::FRAC_PI_3.sin() / 2.0;
            // q1..q8 reduce to these two linear branches around maxAdj/2.
            let inset = if adjust > max / 2.0 {
                1.0 / 24.0 + adjust / (4.0 * max)
            } else {
                1.0 / 12.0 + adjust / (6.0 * max)
            };
            (
                PlacedGeometry::Polygon(vec![
                    (x, 0.5 - dy),
                    (1.0 - x, 0.5 - dy),
                    (1.0, 0.5),
                    (1.0 - x, 0.5 + dy),
                    (x, 0.5 + dy),
                    (0.0, 0.5),
                ]),
                rect(w * inset, h * inset, w * (1.0 - inset), h * (1.0 - inset)),
            )
        }
        // Straight connectors stay `Line`: the renderer gives them their
        // default stroke and line ends.
        "line" | "straightConnector1" => return None,
        name => {
            let preset = crate::preset_shapes::evaluate(name, adjustments, w, h)?;
            (PlacedGeometry::Paths(preset.paths), preset.text)
        }
    };
    Some(PresetLayout { geometry, text })
}

/// Convert a shape-local rectangle into the existing turned text frame.
pub(crate) fn turned(rect: Rect, w: f64, h: f64, rotation: f64) -> Rect {
    match rotation as i32 {
        90 => Rect {
            x: rect.y,
            y: w - rect.x - rect.width,
            width: rect.height,
            height: rect.width,
        },
        270 => Rect {
            x: h - rect.y - rect.height,
            y: rect.x,
            width: rect.height,
            height: rect.width,
        },
        _ => rect,
    }
}
