#![allow(clippy::unwrap_used)]

//! FORECAST.ETS and its family against what Excel computes for the same
//! series. Excel does not publish how it starts the smoothing or picks its
//! parameters, so forecasts are held to within 3% of Excel's.

use crate::cell::CellValue;
use crate::test::util::new_empty_model;
use crate::Model;

/// Timeline, values, three target dates, Excel's forecasts for them and
/// the seasonality Excel detects: a season of 4 with a trend, one of 12,
/// daily dates with a weekly pattern, steps of 5 with gaps and two points
/// out of order, the first of each month, and a timeline with repeats.
/// A negative season is one Excel finds in what is only noise around a
/// line, which detection here does not.
type Case = (&'static [f64], &'static [f64], [f64; 3], [f64; 3], i32);

const CASES: &[Case] = &[
    (
        &[
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
            17.0, 18.0, 19.0, 20.0, 21.0, 22.0, 23.0, 24.0,
        ],
        &[
            12.51, 9.344, 6.739, 13.173, 15.163, 11.394, 8.852, 15.202, 17.314, 13.813, 11.012,
            17.255, 18.767, 15.141, 13.509, 19.104, 20.653, 17.643, 14.992, 21.926, 23.519, 20.011,
            17.026, 23.277,
        ],
        [25.0, 27.0, 36.0],
        [25.360419401226, 19.232371604423, 29.77415363446],
        4,
    ),
    (
        &[
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
            17.0, 18.0, 19.0, 20.0, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 29.0, 30.0,
            31.0, 32.0, 33.0, 34.0, 35.0, 36.0,
        ],
        &[
            97.634, 110.976, 118.937, 121.603, 123.642, 115.302, 111.922, 101.784, 91.689, 94.999,
            96.411, 105.711, 116.894, 126.56, 136.582, 143.553, 142.049, 136.898, 127.997, 118.189,
            115.127, 112.785, 118.546, 122.786, 138.371, 148.836, 156.239, 158.723, 157.967,
            155.176, 145.719, 134.708, 130.61, 126.501, 133.199, 141.434,
        ],
        [37.0, 39.0, 48.0],
        [153.51791908066, 173.001790966583, 159.486300591668],
        12,
    ),
    (
        &[
            45000.0, 45001.0, 45002.0, 45003.0, 45004.0, 45005.0, 45006.0, 45007.0, 45008.0,
            45009.0, 45010.0, 45011.0, 45012.0, 45013.0, 45014.0, 45015.0, 45016.0, 45017.0,
            45018.0, 45019.0, 45020.0, 45021.0, 45022.0, 45023.0, 45024.0, 45025.0, 45026.0,
            45027.0, 45028.0, 45029.0, 45030.0, 45031.0, 45032.0, 45033.0, 45034.0, 45035.0,
            45036.0, 45037.0, 45038.0, 45039.0, 45040.0, 45041.0,
        ],
        &[
            210.225, 212.923, 212.046, 213.141, 192.996, 182.251, 183.521, 211.826, 213.087,
            217.601, 211.487, 196.411, 183.285, 181.002, 211.479, 219.24, 217.958, 211.966,
            196.557, 189.301, 188.756, 218.773, 217.712, 220.388, 215.29, 201.78, 185.071, 187.797,
            222.174, 224.663, 226.607, 222.03, 202.949, 188.596, 191.764, 217.425, 221.912,
            229.584, 224.177, 203.761, 192.211, 194.743,
        ],
        [45042.0, 45044.0, 45053.0],
        [223.407846927872, 228.621291676703, 209.373568855936],
        7,
    ),
    (
        &[
            5.0, 10.0, 20.0, 15.0, 25.0, 30.0, 35.0, 45.0, 50.0, 55.0, 65.0, 70.0, 75.0, 80.0,
        ],
        &[
            32.925, 31.447, 33.068, 31.107, 37.468, 40.257, 42.779, 48.009, 46.251, 52.217, 56.094,
            58.636, 56.475, 62.507,
        ],
        [85.0, 95.0, 140.0],
        [64.550665452283, 67.961033783199, 87.332605282768],
        -5,
    ),
    (
        &[
            44927.0, 44958.0, 44986.0, 45017.0, 45047.0, 45078.0, 45108.0, 45139.0, 45170.0,
            45200.0, 45231.0, 45261.0, 45292.0, 45323.0, 45352.0, 45383.0, 45413.0, 45444.0,
            45474.0, 45505.0, 45536.0, 45566.0, 45597.0, 45627.0, 45658.0, 45689.0, 45717.0,
            45748.0, 45778.0, 45809.0, 45839.0, 45870.0, 45901.0, 45931.0, 45962.0, 45992.0,
        ],
        &[
            551.222, 547.038, 547.043, 507.415, 483.909, 471.827, 471.461, 476.609, 510.391,
            545.352, 561.471, 605.111, 608.999, 609.247, 585.07, 555.659, 540.952, 524.322,
            509.224, 533.869, 552.171, 590.751, 619.685, 634.697, 652.256, 649.549, 631.976,
            599.759, 585.168, 556.828, 560.257, 564.428, 603.122, 636.22, 661.023, 689.299,
        ],
        [46023.0, 46174.0, 46357.0],
        [698.72669069639, 609.968752839021, 736.126473790771],
        12,
    ),
    (
        &[1.0, 2.0, 2.0, 3.0, 4.0, 5.0, 6.0, 6.0, 7.0, 8.0, 9.0, 10.0],
        &[
            10.0, 12.0, 14.0, 15.0, 15.0, 18.0, 20.0, 22.0, 23.0, 25.0, 26.0, 29.0,
        ],
        [11.0, 13.0, 22.0],
        [30.564188158881, 34.678237641994, 53.191460316005],
        0,
    ),
];

fn number(model: &Model, cell: &str) -> f64 {
    let reference = model._parse_reference(cell);
    match model
        .get_cell_value_by_index(reference.sheet, reference.row, reference.column)
        .unwrap()
    {
        CellValue::Number(value) => value,
        other => panic!("{cell} is {other:?}"),
    }
}

fn model_with(timeline: &[f64], values: &[f64]) -> Model<'static> {
    let mut model = new_empty_model();
    for (row, (time, value)) in timeline.iter().zip(values).enumerate() {
        model._set(&format!("A{}", row + 1), &time.to_string());
        model._set(&format!("B{}", row + 1), &value.to_string());
    }
    model
}

#[test]
fn forecasts_are_within_three_percent_of_excel() {
    for (timeline, values, targets, expected, _) in CASES {
        let n = timeline.len();
        let mut model = model_with(timeline, values);
        for (index, target) in targets.iter().enumerate() {
            model._set(
                &format!("C{}", index + 1),
                &format!("=FORECAST.ETS({target}, B1:B{n}, A1:A{n})"),
            );
        }
        model.evaluate();
        for (index, excel) in expected.iter().enumerate() {
            let value = number(&model, &format!("C{}", index + 1));
            let off = ((value - excel) / excel).abs();
            assert!(
                off < 0.03,
                "{value} is {:.2}% from Excel's {excel}",
                off * 100.0
            );
        }
    }
}

#[test]
fn seasonality_is_detected_as_excel_detects_it() {
    for (timeline, values, _, _, season) in CASES {
        if *season < 0 {
            continue;
        }
        let n = timeline.len();
        let mut model = model_with(timeline, values);
        model._set(
            "C1",
            &format!("=FORECAST.ETS.SEASONALITY(B1:B{n}, A1:A{n})"),
        );
        model.evaluate();
        assert_eq!(number(&model, "C1"), *season as f64, "{timeline:?}");
    }
}

#[test]
fn stat_and_confint_as_excel_reports_them() {
    // Steps of 5 with gaps (Excel: step 5), the first of each month (31).
    for (case, step) in [(&CASES[3], 5.0), (&CASES[4], 31.0)] {
        let (timeline, values, targets, _, _) = case;
        let n = timeline.len();
        let mut model = model_with(timeline, values);
        model._set("C1", &format!("=FORECAST.ETS.STAT(B1:B{n}, A1:A{n}, 8)"));
        let confint = |target: f64, level: &str| {
            format!("=FORECAST.ETS.CONFINT({target}, B1:B{n}, A1:A{n}{level})")
        };
        model._set("C2", &confint(targets[0], ""));
        model._set("C3", &confint(targets[2], ""));
        model._set("C4", &confint(targets[0], ", 0.5"));
        model._set("C5", &format!("=FORECAST.ETS.STAT(B1:B{n}, A1:A{n}, 1)"));
        model.evaluate();
        assert_eq!(number(&model, "C1"), step);
        // Wider further out, narrower for a lower confidence.
        assert!(number(&model, "C3") > number(&model, "C2"));
        assert!(number(&model, "C4") < number(&model, "C2"));
        let alpha = number(&model, "C5");
        assert!((0.001..=0.999).contains(&alpha));
    }
}
