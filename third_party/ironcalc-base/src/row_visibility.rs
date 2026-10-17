//! Imported effective row visibility for one synchronous calculation.
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

#[derive(Default)]
pub struct SheetVisibility {
    pub default_hidden: bool,
    /// One-based row indices whose visibility differs from the sheet default.
    pub overrides: HashMap<i32, bool>,
    pub filtered_ranges: Vec<(i32, i32)>,
    pub unknown_filtered_range: bool,
    /// Known manual hides that do not also fail the last applied filter.
    pub manual_hidden_rows: HashSet<i32>,
    pub uncertain_filtered_ranges: bool,
}

pub(crate) enum FilterState {
    Filtered,
    Manual,
    Unknown,
}

pub(crate) fn filter_state(sheet: u32, row: i32) -> Option<FilterState> {
    SHEETS.with(|sheets| {
        let sheets = sheets.borrow();
        let sheet = sheets.as_ref()?.get(sheet as usize)?;
        if sheet.uncertain_filtered_ranges
            && sheet
                .filtered_ranges
                .iter()
                .any(|(first, end)| row >= *first && row <= *end)
        {
            return Some(FilterState::Unknown);
        }
        if sheet.manual_hidden_rows.contains(&row) {
            return Some(FilterState::Manual);
        }
        let end = sheet
            .filtered_ranges
            .partition_point(|(start, _)| *start <= row);
        if end > 0 && row <= sheet.filtered_ranges[end - 1].1 {
            Some(FilterState::Filtered)
        } else if sheet.unknown_filtered_range {
            Some(FilterState::Unknown)
        } else {
            Some(FilterState::Manual)
        }
    })
}

thread_local! {
    static SHEETS: RefCell<Option<Vec<SheetVisibility>>> = const { RefCell::new(None) };
}

pub fn with_visibility<T>(mut sheets: Vec<SheetVisibility>, operation: impl FnOnce() -> T) -> T {
    for sheet in &mut sheets {
        sheet.filtered_ranges.sort_unstable();
        let mut merged: Vec<(i32, i32)> = vec![];
        for (start, end) in sheet.filtered_ranges.drain(..) {
            if let Some(previous) = merged
                .last_mut()
                .filter(|previous| start <= previous.1.saturating_add(1))
            {
                previous.1 = previous.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        sheet.filtered_ranges = merged;
    }
    struct Restore(Option<Vec<SheetVisibility>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SHEETS.with(|sheets| *sheets.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SHEETS.with(|slot| slot.borrow_mut().replace(sheets)));
    operation()
}

pub(crate) fn hidden(sheet: u32, row: i32) -> Option<bool> {
    SHEETS.with(|sheets| {
        let sheets = sheets.borrow();
        let sheet = sheets.as_ref()?.get(sheet as usize)?;
        Some(
            sheet
                .overrides
                .get(&row)
                .copied()
                .unwrap_or(sheet.default_hidden),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Model;

    #[test]
    fn known_manual_rows_and_uncertain_filter_snapshots_have_distinct_subtotal_behavior() {
        let mut model = Model::new_empty("workbook", "en", "UTC", "en").unwrap();
        for (row, value) in [(1, "10"), (2, "20"), (3, "30")] {
            model.set_user_input(0, row, 1, value.into()).unwrap();
        }
        model
            .set_user_input(0, 5, 1, "=SUBTOTAL(9,A1:A3)".into())
            .unwrap();
        model
            .set_user_input(0, 5, 2, "=SUBTOTAL(109,A1:A3)".into())
            .unwrap();
        for uncertain in [false, true] {
            with_visibility(
                vec![SheetVisibility {
                    overrides: HashMap::from([(2, true), (3, true)]),
                    filtered_ranges: vec![(1, 3)],
                    manual_hidden_rows: HashSet::from([2]),
                    uncertain_filtered_ranges: uncertain,
                    ..Default::default()
                }],
                || {
                    model.evaluate();
                    assert_eq!(
                        model.get_formatted_cell_value(0, 5, 1).unwrap(),
                        if uncertain { "#ERROR!" } else { "30" }
                    );
                    assert_eq!(model.get_formatted_cell_value(0, 5, 2).unwrap(), "10");
                },
            );
        }
        assert!(filter_state(0, 2).is_none());
    }

    #[test]
    fn visibility_scopes_restore_nested_panics_and_leave_other_threads_and_model_rows_intact() {
        let mut model = Model::new_empty("workbook", "en", "UTC", "en").unwrap();
        model.set_row_hidden(0, 4, true).unwrap();
        with_visibility(
            vec![SheetVisibility {
                default_hidden: false,
                overrides: HashMap::from([(2, true), (4, false)]),
                ..Default::default()
            }],
            || {
                assert!(model.is_row_hidden(0, 2).unwrap());
                assert!(model.is_row_hidden(0, 0).is_err());
                assert!(model.is_row_hidden(1, 2).is_err());
                assert!(!model.is_row_hidden(0, 4).unwrap());
                with_visibility(
                    vec![SheetVisibility {
                        default_hidden: true,
                        overrides: HashMap::from([(2, false)]),
                        ..Default::default()
                    }],
                    || {
                        assert!(!model.is_row_hidden(0, 2).unwrap());
                        assert!(model.is_row_hidden(0, 1_048_576).unwrap());
                        assert_eq!(model.workbook.worksheets[0].rows.len(), 1);
                    },
                );
                assert!(model.is_row_hidden(0, 2).unwrap());
                let result =
                    std::panic::catch_unwind(|| with_visibility(vec![], || panic!("unwind")));
                assert!(result.is_err());
                assert!(model.is_row_hidden(0, 2).unwrap());
                std::thread::spawn(|| assert_eq!(hidden(0, 2), None))
                    .join()
                    .unwrap();
            },
        );
        assert!(!model.is_row_hidden(0, 2).unwrap());
        assert!(model.is_row_hidden(0, 4).unwrap());
        assert_eq!(hidden(0, 2), None);
    }

    #[test]
    fn subtotal_variance_hidden_modes_exclude_manually_hidden_rows_without_a_scope() {
        let mut model = Model::new_empty("workbook", "en", "UTC", "en").unwrap();
        for (row, value) in [(1, "10"), (2, "20"), (3, "30")] {
            model.set_user_input(0, row, 1, value.into()).unwrap();
        }
        model.set_row_hidden(0, 2, true).unwrap();
        for (column, formula, expected) in [
            (2, "=SUBTOTAL(110,A1:A3)", "200"),
            (3, "=SUBTOTAL(111,A1:A3)", "100"),
            (4, "=SUBTOTAL(10,A1:A3)", "100"),
        ] {
            model.set_user_input(0, 5, column, formula.into()).unwrap();
            model.evaluate();
            assert_eq!(
                model.get_formatted_cell_value(0, 5, column).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn filtered_ranges_are_merged_and_unknown_ranges_do_not_guess_full_subtotals() {
        let mut model = Model::new_empty("workbook", "en", "UTC", "en").unwrap();
        for row in 1..=4 {
            model.set_user_input(0, row, 1, "10".into()).unwrap();
        }
        model
            .set_user_input(0, 6, 1, "=SUBTOTAL(9,A1:A4)".into())
            .unwrap();
        model
            .set_user_input(0, 6, 2, "=SUBTOTAL(109,A1:A4)".into())
            .unwrap();
        with_visibility(
            vec![SheetVisibility {
                overrides: HashMap::from([(2, true), (4, true)]),
                filtered_ranges: vec![(3, 3), (2, 2)],
                ..Default::default()
            }],
            || {
                model.evaluate();
                assert_eq!(model.get_formatted_cell_value(0, 6, 1).unwrap(), "30");
                assert_eq!(model.get_formatted_cell_value(0, 6, 2).unwrap(), "20");
            },
        );
        with_visibility(
            vec![SheetVisibility {
                overrides: HashMap::from([(2, true)]),
                unknown_filtered_range: true,
                ..Default::default()
            }],
            || {
                model.evaluate();
                assert_eq!(model.get_formatted_cell_value(0, 6, 1).unwrap(), "#ERROR!");
                assert_eq!(model.get_formatted_cell_value(0, 6, 2).unwrap(), "30");
            },
        );
    }
}
