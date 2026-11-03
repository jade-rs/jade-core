//! Motion paths (`<p:animMotion path>`): `M`, `L`, `C`, `Z`, `E` commands
//! in fractions of the slide, walked by arc length.

/// A path flattened to a polyline with cumulative lengths.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MotionPath {
    points: Vec<(f64, f64)>,
    lengths: Vec<f64>,
}

impl MotionPath {
    /// Parses PowerPoint's path syntax; relative (`m`, `l`, `c`) commands
    /// are accepted too.
    pub fn parse(path: &str) -> Self {
        let mut tokens = path
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .peekable();
        let mut points: Vec<(f64, f64)> = Vec::new();
        let mut current = (0.0, 0.0);
        let mut start = (0.0, 0.0);
        let mut command = 'M';
        let number = |tokens: &mut std::iter::Peekable<_>| -> Option<f64> {
            let token: &str = tokens.next()?;
            token.parse::<f64>().ok()
        };
        loop {
            let Some(&token) = tokens.peek() else {
                break;
            };
            if let Some(c) = token.chars().next().filter(|c| c.is_ascii_alphabetic()) {
                tokens.next();
                command = c;
                if matches!(c, 'Z' | 'z') {
                    points.push(start);
                    current = start;
                    continue;
                }
                if matches!(c, 'E' | 'e') {
                    break;
                }
                // Glued to its number: "M0".
                if token.len() > 1 {
                    continue;
                }
            }
            let relative = command.is_ascii_lowercase();
            let base = if relative { current } else { (0.0, 0.0) };
            match command.to_ascii_uppercase() {
                'M' | 'L' => {
                    let (Some(x), Some(y)) = (number(&mut tokens), number(&mut tokens)) else {
                        break;
                    };
                    current = (base.0 + x, base.1 + y);
                    points.push(current);
                    if command.eq_ignore_ascii_case(&'M') {
                        start = current;
                        // Numbers after a move are lines.
                        command = if relative { 'l' } else { 'L' };
                    }
                }
                'C' => {
                    let values: Option<Vec<f64>> = (0..6).map(|_| number(&mut tokens)).collect();
                    let Some(v) = values else {
                        break;
                    };
                    let p0 = current;
                    let p1 = (base.0 + v[0], base.1 + v[1]);
                    let p2 = (base.0 + v[2], base.1 + v[3]);
                    let p3 = (base.0 + v[4], base.1 + v[5]);
                    for step in 1..=16 {
                        let t = f64::from(step) / 16.0;
                        let u = 1.0 - t;
                        points.push((
                            u * u * u * p0.0
                                + 3.0 * u * u * t * p1.0
                                + 3.0 * u * t * t * p2.0
                                + t * t * t * p3.0,
                            u * u * u * p0.1
                                + 3.0 * u * u * t * p1.1
                                + 3.0 * u * t * t * p2.1
                                + t * t * t * p3.1,
                        ));
                    }
                    current = p3;
                }
                _ => {
                    tokens.next();
                }
            }
        }
        if points.is_empty() {
            points.push((0.0, 0.0));
        }
        let mut lengths = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                let previous = points[index - 1];
                total += ((point.0 - previous.0).powi(2) + (point.1 - previous.1).powi(2)).sqrt();
            }
            lengths.push(total);
        }
        Self { points, lengths }
    }

    /// The point `progress` (0..=1) of the way along the path.
    pub fn point_at(&self, progress: f64) -> (f64, f64) {
        let total = *self.lengths.last().unwrap_or(&0.0);
        if total <= 0.0 || self.points.len() < 2 {
            return self.points.first().copied().unwrap_or((0.0, 0.0));
        }
        let target = progress.clamp(0.0, 1.0) * total;
        let index = self
            .lengths
            .partition_point(|&length| length < target)
            .clamp(1, self.points.len() - 1);
        let (l0, l1) = (self.lengths[index - 1], self.lengths[index]);
        let t = if l1 > l0 {
            (target - l0) / (l1 - l0)
        } else {
            0.0
        };
        let (a, b) = (self.points[index - 1], self.points[index]);
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    }

    /// The flattened points, for drawing the path in the editor.
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_lines_and_curves_by_length() {
        let path = MotionPath::parse("M 0 0 L 0 0.25 E");
        assert_eq!(path.point_at(0.5), (0.0, 0.125));
        let path = MotionPath::parse("M 0 0 L 0.1 0 L 0.1 0.1 Z");
        let end = path.point_at(1.0);
        assert!(end.0.abs() < 1e-9 && end.1.abs() < 1e-9);
        let curve = MotionPath::parse("M 0 0 C 0 0.1 0.25 0.1 0.25 0 E");
        let middle = curve.point_at(0.5);
        assert!((middle.0 - 0.125).abs() < 0.01 && middle.1 > 0.07);
        let exponent = MotionPath::parse("M -4.16667E-6 -4.07407E-6 L -4.16667E-6 0.25 E");
        assert!((exponent.point_at(1.0).1 - 0.25).abs() < 1e-6);
    }
}
