/// Logical byte bounds of source ranges, independent of visual bidi order.
/// Empty input has no range; zero-length and reversed ranges remain representable.
#[inline]
pub fn logical_source_range(
    ranges: impl IntoIterator<Item = (usize, usize)>,
) -> Option<(usize, usize)> {
    let mut ranges = ranges.into_iter();
    let (start, end) = ranges.next()?;
    let mut bounds = (start.min(end), start.max(end));
    for (start, end) in ranges {
        bounds.0 = bounds.0.min(start.min(end));
        bounds.1 = bounds.1.max(start.max(end));
    }
    Some(bounds)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LineHorizontalMetrics {
    pub start_x: f64,
    pub cursor_end_x: f64,
}

impl LineHorizontalMetrics {
    pub fn cursor_end_x(&self) -> f64 {
        self.cursor_end_x.max(self.start_x)
    }

    pub fn cursor_width(&self) -> f64 {
        (self.cursor_end_x() - self.start_x).abs()
    }

    pub fn visual_end_x<I>(&self, extra_ranges: I) -> f64
    where
        I: IntoIterator<Item = (f64, f64)>,
    {
        let mut max_x = self.start_x.max(self.cursor_end_x);
        for (start_x, end_x) in extra_ranges {
            max_x = max_x.max(start_x.max(end_x));
        }
        max_x
    }
}

#[cfg(test)]
mod tests {
    use super::LineHorizontalMetrics;

    #[test]
    fn visual_end_x_uses_cursor_and_extra_ranges() {
        let metrics = LineHorizontalMetrics {
            start_x: 10.0,
            cursor_end_x: 18.0,
        };

        let visual_end = metrics.visual_end_x([(12.0, 24.0), (8.0, 22.0)]);
        assert!((visual_end - 24.0).abs() < f64::EPSILON);
    }
}
