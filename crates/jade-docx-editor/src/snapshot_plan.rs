/// Utilities for determining which pages should be snapshotted after a render.
///
/// This module is intentionally free of any Canvas / wasm-bindgen specific APIs so it can be
/// unit-tested on the native host.
/// Computes the (0-indexed) page indices that should be snapshotted.
///
/// - When `needs_full_repaint` is true, all pages `0..page_count` are returned.
/// - Otherwise, `dirty_pages` are sorted/deduplicated, filtered to `< page_count`, and returned.
pub(crate) fn pages_to_snapshot(
    needs_full_repaint: bool,
    page_count: usize,
    dirty_pages: impl IntoIterator<Item = usize>,
) -> Vec<usize> {
    if page_count == 0 {
        return Vec::new();
    }
    if needs_full_repaint {
        return (0..page_count).collect();
    }
    let mut pages: Vec<usize> = dirty_pages
        .into_iter()
        .filter(|p| *p < page_count)
        .collect();
    pages.sort_unstable();
    pages.dedup();
    pages
}

/// Filters a set of 0-indexed page indices to those that fall within the given
/// visible page range (1-indexed). If `visible_range` is `None`, returns `pages` unchanged.
pub(crate) fn filter_pages_by_visible_range(
    mut pages: Vec<usize>,
    visible_range: Option<std::ops::RangeInclusive<usize>>,
) -> Vec<usize> {
    let Some(range_1idx) = visible_range else {
        return pages;
    };
    pages.retain(|p0| range_1idx.contains(&(p0.saturating_add(1))));
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_to_snapshot_full_repaint_returns_all_pages() {
        let pages = pages_to_snapshot(true, 5, [0, 2, 4]);
        assert_eq!(pages, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn pages_to_snapshot_dirty_sorts_dedups_and_filters() {
        let pages = pages_to_snapshot(false, 5, [3, 1, 3, 10, 0, 4, 4]);
        assert_eq!(pages, vec![0, 1, 3, 4]);
    }

    #[test]
    fn pages_to_snapshot_dirty_empty_returns_empty() {
        let pages = pages_to_snapshot(false, 5, std::iter::empty::<usize>());
        assert!(pages.is_empty());
    }

    #[test]
    fn pages_to_snapshot_page_count_zero_is_empty() {
        let pages = pages_to_snapshot(false, 0, [0, 1, 2]);
        assert!(pages.is_empty());
        let pages = pages_to_snapshot(true, 0, [0, 1, 2]);
        assert!(pages.is_empty());
    }

    #[test]
    fn filter_pages_by_visible_range_keeps_only_visible_pages() {
        let pages = vec![0, 1, 2, 3];
        let filtered = filter_pages_by_visible_range(pages, Some(2..=3));
        assert_eq!(filtered, vec![1, 2]);
    }

    #[test]
    fn filter_pages_by_visible_range_none_returns_original() {
        let pages = vec![0, 2, 5];
        let filtered = filter_pages_by_visible_range(pages.clone(), None);
        assert_eq!(filtered, pages);
    }
}
