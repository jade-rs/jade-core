//! Scoped date semantics for consumers projecting 1900/1904 workbooks.
//!
//! Date conversion is shared by evaluation, text coercion, formatting and
//! financial functions. Scope the whole operation so those paths agree.

use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DateSystem {
    #[default]
    Excel1900,
    Excel1904,
}

thread_local! {
    static ACTIVE: Cell<DateSystem> = const { Cell::new(DateSystem::Excel1900) };
}

/// Runs a synchronous operation with the selected epoch. Nested operations,
/// panics and concurrent threads restore/isolate their own previous setting.
pub fn with_date_system<T>(system: DateSystem, operation: impl FnOnce() -> T) -> T {
    struct Restore(DateSystem);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.set(self.0));
        }
    }
    let _restore = Restore(ACTIVE.with(|active| active.replace(system)));
    operation()
}

pub(crate) fn is_1904() -> bool {
    ACTIVE.with(|active| active.get() == DateSystem::Excel1904)
}

pub(crate) fn base() -> i32 {
    crate::constants::EXCEL_DATE_BASE + if is_1904() { 1462 } else { 0 }
}

pub(crate) fn minimum_serial() -> i32 {
    if is_1904() {
        0
    } else {
        crate::constants::MINIMUM_DATE_SERIAL_NUMBER
    }
}

pub(crate) fn maximum_serial() -> i32 {
    crate::constants::MAXIMUM_DATE_SERIAL_NUMBER - if is_1904() { 1462 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_contexts_and_panics_restore_the_previous_epoch() {
        assert!(!is_1904());
        with_date_system(DateSystem::Excel1904, || {
            assert!(is_1904());
            with_date_system(DateSystem::Excel1900, || assert!(!is_1904()));
            assert!(is_1904());
            let result = std::panic::catch_unwind(|| {
                with_date_system(DateSystem::Excel1900, || panic!("test unwind"));
            });
            assert!(result.is_err());
            assert!(is_1904());
        });
        assert!(!is_1904());
    }

    #[test]
    fn threads_do_not_share_date_context() {
        with_date_system(DateSystem::Excel1904, || {
            std::thread::spawn(|| assert!(!is_1904())).join().unwrap();
            assert!(is_1904());
        });
    }
}
