//! The furigana Excel saved with cells' text (`<rPh>`), for PHONETIC during
//! one synchronous calculation. The engine's own cells hold only the text.
use std::{cell::RefCell, collections::HashMap};

/// A reading by sheet index, then one-based row and column.
pub type Readings = HashMap<(u32, i32, i32), String>;

thread_local! {
    static READINGS: RefCell<Option<Readings>> = const { RefCell::new(None) };
}

/// Runs `operation` with `readings` visible to PHONETIC, restoring what was
/// there before, however `operation` ends.
pub fn with_readings<T>(readings: Readings, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<Readings>);
    impl Drop for Restore {
        fn drop(&mut self) {
            READINGS.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(READINGS.with(|slot| slot.borrow_mut().replace(readings)));
    operation()
}

pub(crate) fn reading(sheet: u32, row: i32, column: i32) -> Option<String> {
    READINGS.with(|slot| slot.borrow().as_ref()?.get(&(sheet, row, column)).cloned())
}
