//! Paragraph-group placement consuming already measured shared layout extents.

/// CSS-pixel tolerance for accumulated line-band coordinates.
pub const FLOW_EPSILON_PX: f64 = 1e-6;

/// An oversized keepNext group starts on a fresh flow container, then flows
/// normally; it must not repeatedly advance at the top of that container.
pub fn keep_next_group_needs_advance(height: f64, remaining: f64, at_top: bool) -> bool {
    height.is_finite() && height > 0.0 && !at_top && height > remaining + FLOW_EPSILON_PX
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_group_fit_and_oversized_flow() {
        assert!(!keep_next_group_needs_advance(60.0, 60.0, false));
        assert!(keep_next_group_needs_advance(60.0, 59.0, false));
        assert!(keep_next_group_needs_advance(600.0, 59.0, false));
        assert!(!keep_next_group_needs_advance(600.0, 200.0, true));
        assert!(!keep_next_group_needs_advance(f64::NAN, 20.0, false));
    }
}
