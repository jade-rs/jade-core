//! DrawingML shape guides: the `fmla` language of `<a:gd>` that preset and
//! custom geometries compute their points from (ECMA-376 §20.1.9.11).

use std::collections::HashMap;

/// Guide values by name, with the built-in shape variables.
pub struct ShapeGuides {
    w: f64,
    h: f64,
    values: HashMap<String, f64>,
}

impl ShapeGuides {
    /// Guides for a shape `w` × `h` in whatever unit its formulas use.
    pub fn new(w: f64, h: f64) -> Self {
        Self {
            w,
            h,
            values: HashMap::new(),
        }
    }

    pub fn set(&mut self, name: &str, value: f64) {
        self.values.insert(name.to_string(), value);
    }

    /// Evaluates `fmla` and stores it as `name`.
    pub fn define(&mut self, name: &str, fmla: &str) {
        let value = self.eval(fmla);
        self.set(name, value);
    }

    fn builtin(&self, name: &str) -> Option<f64> {
        let (w, h) = (self.w, self.h);
        let ss = w.min(h);
        let divided = |prefix: &str, base: f64| {
            name.strip_prefix(prefix)
                .and_then(|n| n.parse::<f64>().ok())
                .filter(|n| *n > 0.0)
                .map(|n| base / n)
        };
        Some(match name {
            "w" | "r" => w,
            "h" | "b" => h,
            "l" | "t" => 0.0,
            "hc" => w / 2.0,
            "vc" => h / 2.0,
            "ss" => ss,
            "ls" => w.max(h),
            "cd2" => 10_800_000.0,
            "cd4" => 5_400_000.0,
            "cd8" => 2_700_000.0,
            "3cd4" => 16_200_000.0,
            "3cd8" => 8_100_000.0,
            "5cd8" => 13_500_000.0,
            "7cd8" => 18_900_000.0,
            _ => {
                return divided("wd", w)
                    .or_else(|| divided("hd", h))
                    .or_else(|| divided("ssd", ss))
            }
        })
    }

    /// A formula argument: a guide, a literal or a built-in variable.
    pub fn arg(&self, token: &str) -> f64 {
        if let Some(value) = self.values.get(token) {
            return *value;
        }
        if let Ok(value) = token.parse::<f64>() {
            return value;
        }
        self.builtin(token).unwrap_or(0.0)
    }

    /// Evaluates one `fmla` (`*/ w adj 100000`, `val 5`, …).
    pub fn eval(&self, fmla: &str) -> f64 {
        let mut parts = fmla.split_whitespace();
        let op = parts.next().unwrap_or("val");
        let args: Vec<f64> = parts.map(|token| self.arg(token)).collect();
        let a = |i: usize| args.get(i).copied().unwrap_or(0.0);
        let rad = |v: f64| (v / 60_000.0).to_radians();
        let deg = |r: f64| r.to_degrees() * 60_000.0;
        match op {
            "val" => a(0),
            "*/" => {
                if a(2) == 0.0 {
                    0.0
                } else {
                    a(0) * a(1) / a(2)
                }
            }
            "+-" => a(0) + a(1) - a(2),
            "+/" => {
                if a(2) == 0.0 {
                    0.0
                } else {
                    (a(0) + a(1)) / a(2)
                }
            }
            "?:" => {
                if a(0) > 0.0 {
                    a(1)
                } else {
                    a(2)
                }
            }
            "abs" => a(0).abs(),
            "at2" => deg(a(1).atan2(a(0))),
            "cat2" => a(0) * a(2).atan2(a(1)).cos(),
            "sat2" => a(0) * a(2).atan2(a(1)).sin(),
            "cos" => a(0) * rad(a(1)).cos(),
            "sin" => a(0) * rad(a(1)).sin(),
            "tan" => a(0) * rad(a(1)).tan(),
            "max" => a(0).max(a(1)),
            "min" => a(0).min(a(1)),
            "mod" => (a(0) * a(0) + a(1) * a(1) + a(2) * a(2)).sqrt(),
            "pin" => {
                if a(1) < a(0) {
                    a(0)
                } else if a(1) > a(2) {
                    a(2)
                } else {
                    a(1)
                }
            }
            "sqrt" => a(0).max(0.0).sqrt(),
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_see_builtins_and_earlier_guides() {
        let mut guides = ShapeGuides::new(200.0, 100.0);
        guides.define("adj", "val 25000");
        guides.define("x1", "*/ ss adj 100000");
        guides.define("x2", "+- r 0 x1");
        assert_eq!(guides.arg("x1"), 25.0);
        assert_eq!(guides.arg("x2"), 175.0);
        assert_eq!(guides.eval("pin 0 x2 hc"), 100.0);
        assert_eq!(guides.eval("?: -1 wd2 hd2"), 50.0);
        assert!((guides.eval("cos 100 cd4")).abs() < 1e-9);
        assert!((guides.eval("at2 0 10") - 5_400_000.0).abs() < 1e-6);
    }
}
