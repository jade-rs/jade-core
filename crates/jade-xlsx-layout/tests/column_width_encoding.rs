use jade_xlsx_layout::units::*;
#[test]
fn encoded_widths_match_microsoft_examples_without_adding_padding_twice() {
    assert_eq!(column_width_chars_to_px(8.7109375, 7.0), 61.0);
    assert_eq!(stored_column_width_to_displayed(8.7109375, 7.0), 8.0);
    assert_eq!(column_width_chars_to_px(9.140625, 7.0), 64.0);
    assert_eq!(stored_column_width_to_displayed(9.140625, 7.0), 8.43);
    assert_eq!(displayed_column_width_to_stored(8.0, 7.0), 8.7109375);
}
#[test]
fn sub_character_widths_use_encoded_digit_units_and_zero_stays_zero() {
    assert_eq!(column_width_chars_to_px(0.5, 7.0), 3.0);
    assert_eq!(
        column_width_chars_to_px(displayed_column_width_to_stored(0.5, 7.0), 7.0),
        6.0
    );
    assert_eq!(column_width_chars_to_px(0.0, 7.0), 0.0);
    assert_eq!(column_width_px_to_chars(0.0, 7.0), 0.0);
}
#[test]
fn pixel_sizing_round_trips_across_digit_metrics_and_the_file_limit() {
    for mdw in [5.0, 7.0, 10.0, 16.25, 128.0, 160.0] {
        for pixels in 0..1000 {
            let stored = column_width_px_to_chars(f64::from(pixels), mdw);
            assert_eq!(
                column_width_chars_to_px(stored, mdw),
                f64::from(pixels),
                "{pixels} at {mdw}"
            );
            assert_eq!(stored * 256.0, (stored * 256.0).round());
        }
    }
    assert_eq!(column_width_px_to_chars(1e9, 7.0), 255.0);
    assert_eq!(column_width_chars_to_px(255.0, 7.0), 1785.0);
}
#[test]
fn invalid_inputs_do_not_create_nan_geometry() {
    for value in [f32::NAN, f32::INFINITY, -1.0] {
        assert_eq!(column_width_chars_to_px(value, 7.0), 0.0);
    }
    for mdw in [0.0, f64::NAN, f64::INFINITY, -1.0] {
        assert_eq!(column_width_chars_to_px(9.140625, mdw), 64.0);
    }
    assert_eq!(column_width_px_to_chars(f64::NAN, 7.0), 0.0);
}
