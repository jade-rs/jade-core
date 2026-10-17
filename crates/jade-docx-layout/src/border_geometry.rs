//! Shared geometry of parallel Word border rules.

use crate::line_metrics::WordLayoutPlatform;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoubleBorderMetrics {
    pub stripe_width: f64,
    pub stripe_offset: f64,
    pub layout_width: f64,
}

/// Word for Mac uses `w:sz` for each rule of a double border, with a gap of
/// the same size. The unchanged Word PDFs for `table-border-conflicts` and
/// `table-cell-spacing` show 1pt, 2pt and 3pt rules, and 3pt, 6pt and 9pt
/// complete bands respectively. Windows retains the existing policy.
pub fn double_border_metrics(width: f64, platform: WordLayoutPlatform) -> DoubleBorderMetrics {
    if platform == WordLayoutPlatform::Mac {
        DoubleBorderMetrics {
            stripe_width: width,
            stripe_offset: width,
            layout_width: 3.0 * width,
        }
    } else {
        let stripe_width = (width / 3.0).max(0.5);
        DoubleBorderMetrics {
            stripe_width,
            stripe_offset: (width / 2.0).max(stripe_width),
            layout_width: width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_double_border_stripes_and_gap_share_the_authored_width() {
        for width in [0.25, 0.5, 1.0, 2.0, 3.0] {
            let metrics = double_border_metrics(width, WordLayoutPlatform::Mac);
            assert_eq!(metrics.stripe_width, width);
            assert_eq!(metrics.stripe_offset, width);
            assert_eq!(metrics.layout_width, 3.0 * width);
            assert_eq!(
                2.0 * metrics.stripe_offset + metrics.stripe_width,
                metrics.layout_width
            );
        }
    }

    #[test]
    fn windows_double_border_keeps_legacy_rule_geometry_and_layout_budget() {
        let metrics = double_border_metrics(3.0, WordLayoutPlatform::Windows);
        assert_eq!(
            metrics,
            DoubleBorderMetrics {
                stripe_width: 1.0,
                stripe_offset: 1.5,
                layout_width: 3.0
            }
        );
        assert_eq!(
            double_border_metrics(1.0, WordLayoutPlatform::Windows).stripe_width,
            0.5
        );
    }
}
