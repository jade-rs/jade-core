use jade_xlsx_layout::{
    layout_sheet,
    resize::{auto_fit_columns, ResizeGesture, ResizeHit},
    LayoutOptions, ResolvedFont, TextMeasure,
};
use jade_xlsx_reader::{sheet::parse_sheet, styles::parse_styles, Axis};
use std::rc::Rc;
struct Digits;
impl TextMeasure for Digits {
    fn measure(&self, text: &str, font: &ResolvedFont) -> f64 {
        if font.family == "NormalDigits" {
            text.chars()
                .map(|ch| {
                    if ch == '9' {
                        10.0
                    } else if ch == ' ' {
                        4.0
                    } else {
                        8.0
                    }
                })
                .sum()
        } else {
            text.chars().count() as f64 * 3.0
        }
    }
}
fn styles() -> jade_xlsx_reader::StyleTable {
    parse_styles(r#"<styleSheet><fonts><font><name val="CellFace"/><sz val="11"/></font><font><name val="NormalDigits"/><sz val="22"/></font></fonts><cellStyleXfs><xf fontId="1"/></cellStyleXfs><cellXfs><xf fontId="0"><alignment indent="2"/></xf></cellXfs><cellStyles><cellStyle name="標準" builtinId="0" xfId="0"/></cellStyles></styleSheet>"#).unwrap()
}
#[test]
fn digit_maximum_normal_indent_and_resize_conversion_share_the_selected_font() {
    let styles = styles();
    let options = LayoutOptions {
        measure: Rc::new(Digits),
        min_column_count: 2,
        ..Default::default()
    }
    .with_normal_font_metrics(&styles);
    assert_eq!(options.max_digit_width_px, 10.0);
    let sheet = parse_sheet("Sheet", "sheet.xml", r#"<worksheet><sheetData><row r="1"><c r="A1" s="0" t="inlineStr"><is><t>Text</t></is></c></row></sheetData></worksheet>"#, &[]).unwrap();
    let layout = layout_sheet(&sheet, &styles, &options);
    assert_eq!(layout.max_digit_width_px, 10.0);
    assert_eq!(layout.columns[0].size, 91.0);
    assert_eq!(layout.cells[0].font.family, "CellFace");
    assert_eq!(layout.cells[0].text_inset_px, 24.0);
    let mut gesture = ResizeGesture {
        hit: ResizeHit {
            axis: Axis::Columns,
            index: 0,
            size_px: 91.0,
        },
        pointer_start: 0.0,
        size_px: 91.0,
    };
    gesture.update(20.0, 0.0);
    assert_eq!(
        gesture.stored_size(layout.max_digit_width_px),
        2841.0_f32 / 256.0
    );
    let fit = auto_fit_columns(&sheet, &styles, 0, 0, &options);
    assert_eq!(
        jade_xlsx_layout::units::column_width_chars_to_px(fit[&0], layout.max_digit_width_px),
        42.0
    );
}
#[test]
fn unavailable_or_invalid_measurements_preserve_the_configured_fallback() {
    struct Bad;
    impl TextMeasure for Bad {
        fn measure(&self, _: &str, _: &ResolvedFont) -> f64 {
            f64::NAN
        }
    }
    let options = LayoutOptions {
        max_digit_width_px: 9.0,
        measure: Rc::new(Bad),
        ..Default::default()
    }
    .with_normal_font_metrics(&styles());
    assert_eq!(options.max_digit_width_px, 9.0);
    let default = jade_xlsx_reader::StyleTable::default();
    let options = LayoutOptions {
        measure: Rc::new(Digits),
        ..Default::default()
    }
    .with_normal_font_metrics(&default);
    assert_eq!(options.max_digit_width_px, 7.0);
    assert_eq!(
        options
            .with_measured_normal_font(&default)
            .max_digit_width_px,
        3.0
    );
}
