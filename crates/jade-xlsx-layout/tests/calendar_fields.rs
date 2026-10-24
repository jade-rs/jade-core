use jade_xlsx_layout::number_format::{calendar_fields, is_calendar_date_valid};
use jade_xlsx_reader::DateSystem::{Nineteen00, Nineteen04};

#[test]
fn calendar_groups_use_stored_boundaries_without_display_rounding() {
    for (epoch, offset) in [(Nineteen00, 0.0), (Nineteen04, 1462.0)] {
        let day = 45322.0 - offset;
        assert_eq!(
            calendar_fields(day + 86399.9 / 86400.0, epoch),
            Some([2024, 1, 31, 23, 59, 59])
        );
        assert_eq!(
            calendar_fields(day + 1.0, epoch),
            Some([2024, 2, 1, 0, 0, 0])
        );
        // Every exact second and the immediately preceding representable value.
        for second in 1..86400 {
            let boundary = day + f64::from(second) / 86400.0;
            let fields = calendar_fields(boundary, epoch).unwrap();
            assert_eq!(fields[3] * 3600 + fields[4] * 60 + fields[5], second);
            let previous = f64::from_bits(boundary.to_bits() - 1);
            let fields = calendar_fields(previous, epoch).unwrap();
            assert_eq!(fields[3] * 3600 + fields[4] * 60 + fields[5], second - 1);
        }
    }
}

#[test]
fn epochs_phantom_day_and_calendar_limits_are_shared() {
    assert_eq!(
        calendar_fields(0.0, Nineteen04),
        Some([1904, 1, 1, 0, 0, 0])
    );
    assert_eq!(
        calendar_fields(59.0, Nineteen00),
        Some([1900, 2, 28, 0, 0, 0])
    );
    assert_eq!(
        calendar_fields(60.0, Nineteen00),
        Some([1900, 2, 29, 0, 0, 0])
    );
    assert_eq!(
        calendar_fields(61.0, Nineteen00),
        Some([1900, 3, 1, 0, 0, 0])
    );
    for (epoch, limit) in [(Nineteen00, 2958466.0_f64), (Nineteen04, 2957004.0_f64)] {
        assert_eq!(
            calendar_fields(limit - 1.0, epoch).unwrap()[..3],
            [9999, 12, 31]
        );
        for serial in [-1.0, f64::NAN, f64::INFINITY, limit] {
            assert_eq!(calendar_fields(serial, epoch), None);
        }
    }
    assert!(is_calendar_date_valid(1900, 2, 29, Nineteen00));
    assert!(!is_calendar_date_valid(1900, 2, 29, Nineteen04));
    assert!(is_calendar_date_valid(2024, 2, 29, Nineteen00));
    for (year, month, day) in [
        (2023, 2, 29),
        (2024, 4, 31),
        (0, 1, 1),
        (10000, 1, 1),
        (2024, 13, 1),
    ] {
        assert!(!is_calendar_date_valid(year, month, day, Nineteen00));
    }
}

#[test]
fn calendar_serial_round_trips_both_epochs_and_the_1900_boundary() {
    use jade_xlsx_layout::number_format::calendar_serial;
    for (epoch, serial) in [
        (Nineteen00, 0.),
        (Nineteen00, 1.),
        (Nineteen00, 59.),
        (Nineteen00, 60.),
        (Nineteen00, 61.),
        (Nineteen00, 2_958_465.),
        (Nineteen04, 0.),
        (Nineteen04, 59.),
        (Nineteen04, 2_957_003.),
    ] {
        let [y, m, d, ..] = calendar_fields(serial, epoch).unwrap();
        assert_eq!(calendar_serial(y, m, d, epoch), Some(serial));
    }
    assert_eq!(calendar_serial(1900, 2, 29, Nineteen04), None);
    assert_eq!(calendar_serial(2023, 2, 29, Nineteen00), None);
    assert_eq!(calendar_serial(1903, 12, 31, Nineteen04), None);
}
