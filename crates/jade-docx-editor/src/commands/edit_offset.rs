/// Maps a byte offset through the replacement of `[start, end)`.
/// The caller must order the edit endpoints first. An insertion attaches an
/// anchor at `start` to the right of the inserted text.
pub(crate) fn rebase_byte_offset(
    position: usize,
    start: usize,
    end: usize,
    replacement_len: usize,
) -> usize {
    let removed_len = end.saturating_sub(start);
    if removed_len == 0 {
        if position >= start {
            position.saturating_add(replacement_len)
        } else {
            position
        }
    } else if position <= start {
        position
    } else if position < end {
        start
    } else {
        position
            .saturating_sub(removed_len)
            .saturating_add(replacement_len)
    }
}

#[cfg(test)]
mod tests {
    use super::rebase_byte_offset;

    #[test]
    fn rebases_insertions_and_replacements_at_boundaries() {
        assert_eq!(rebase_byte_offset(4, 4, 4, 3), 7);
        assert_eq!(rebase_byte_offset(3, 4, 4, 3), 3);
        assert_eq!(rebase_byte_offset(4, 4, 8, 2), 4);
        assert_eq!(rebase_byte_offset(6, 4, 8, 2), 4);
        assert_eq!(rebase_byte_offset(8, 4, 8, 2), 6);
        assert_eq!(rebase_byte_offset(9, 4, 8, 2), 7);
    }
}

#[cfg(kani)]
mod verification {
    use super::rebase_byte_offset;

    #[kani::proof]
    fn rebasing_preserves_offset_order() {
        let first: usize = kani::any();
        let second: usize = kani::any();
        let start: usize = kani::any();
        let end: usize = kani::any();
        let replacement_len: usize = kani::any();
        kani::assume(first <= second && start <= end);

        assert!(
            rebase_byte_offset(first, start, end, replacement_len)
                <= rebase_byte_offset(second, start, end, replacement_len)
        );
    }

    #[kani::proof]
    fn rebasing_respects_edit_boundaries() {
        let position: usize = kani::any();
        let start: usize = kani::any();
        let end: usize = kani::any();
        let replacement_len: usize = kani::any();
        kani::assume(start <= end);

        let rebased = rebase_byte_offset(position, start, end, replacement_len);
        if position < start {
            assert_eq!(rebased, position);
        }
        if start < end && start <= position && position < end {
            assert_eq!(rebased, start);
        }
        if position >= end && start < end {
            assert_eq!(
                rebased,
                (position - (end - start)).saturating_add(replacement_len)
            );
        }
    }
}
