//! A synchronous calculation uses one clock instant across its volatile cells.

use std::cell::Cell;

thread_local! {
    static INSTANT: Cell<Option<i64>> = const { Cell::new(None) };
}

pub(crate) fn instant() -> Option<i64> {
    INSTANT.with(Cell::get)
}

/// Scope a clock instant for a synchronous calculation, restoring nested scopes
/// on return or unwinding. Independent threads retain their own clock.
pub fn with_milliseconds<T>(milliseconds: i64, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<i64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            INSTANT.with(|instant| instant.set(self.0));
        }
    }
    let _restore = Restore(INSTANT.with(|instant| instant.replace(Some(milliseconds))));
    operation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_scopes_restore_on_return_and_unwind_and_do_not_cross_threads() {
        assert_eq!(instant(), None);
        with_milliseconds(1, || {
            with_milliseconds(2, || assert_eq!(instant(), Some(2)));
            assert_eq!(instant(), Some(1));
            let result = std::panic::catch_unwind(|| with_milliseconds(3, || panic!("unwind")));
            assert!(result.is_err());
            assert_eq!(instant(), Some(1));
            std::thread::spawn(|| assert_eq!(instant(), None))
                .join()
                .unwrap();
        });
        assert_eq!(instant(), None);
    }
}
