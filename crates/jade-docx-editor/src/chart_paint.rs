//! A Word chart drawn as its chart part describes it — plot, axes and their
//! labels, title, legend — by the chart layout the spreadsheet renderer
//! uses, from the values cached in the part (a Word chart's data lives in
//! its embedded workbook, which the cache mirrors).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use jade_painter::Painter;
use jade_xlsx_layout::chart::{layout_chart, ChartOp};
use jade_xlsx_layout::{LayoutOptions, Rect};
use jade_xlsx_reader::chart::Chart;
use jade_xlsx_reader::{Sheet, StyleTable};

thread_local! {
    static CHARTS: RefCell<HashMap<u64, Option<Rc<Chart>>>> = RefCell::new(HashMap::new());
    // A sheet no chart formula names, so every series reads its cache.
    static EMPTY_SHEET: Option<Sheet> = jade_xlsx_reader::sheet::parse_sheet(
        "\u{1}jade-chart-cache",
        "xl/worksheets/sheet1.xml",
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
        &[],
    )
    .ok();
}

fn parsed(xml: &str) -> Option<Rc<Chart>> {
    let mut hasher = rustc_hash::FxHasher::default();
    xml.hash(&mut hasher);
    let key = hasher.finish();
    CHARTS.with(|charts| {
        if let Some(found) = charts.borrow().get(&key) {
            return found.clone();
        }
        let chart = jade_xlsx_reader::chart::parse_chart(xml).ok().map(Rc::new);
        let mut charts = charts.borrow_mut();
        if charts.len() > 64 {
            charts.clear();
        }
        charts.insert(key, chart.clone());
        chart
    })
}

/// The drawing operations for the chart part `xml` in the box `frame`;
/// `None` when the layout cannot draw the chart's kind.
pub fn chart_ops(xml: &str, (x, y, width, height): (f64, f64, f64, f64)) -> Option<Vec<ChartOp>> {
    let chart = parsed(xml)?;
    EMPTY_SHEET.with(|sheet| {
        let sheet = sheet.as_ref()?;
        let placed = layout_chart(
            &chart,
            Rect {
                x,
                y,
                width,
                height,
            },
            sheet,
            None,
            &LayoutOptions::default(),
            &StyleTable::default(),
        );
        (!placed.ops.is_empty()).then_some(placed.ops)
    })
}

/// Draws the chart; returns whether it could.
pub fn paint_chart<P: Painter>(painter: &P, xml: &str, frame: (f64, f64, f64, f64)) -> bool {
    let Some(ops) = chart_ops(xml, frame) else {
        return false;
    };
    let (x, y, width, height) = frame;
    painter.save();
    // The chart area: white, as Word's default chart is.
    painter.set_fill_style_str("#ffffff");
    painter.fill_rect(x, y, width, height);
    painter.begin_path();
    painter.rect(x, y, width, height);
    painter.clip();
    for op in &ops {
        match op {
            ChartOp::Clip { rect } => {
                painter.save();
                painter.begin_path();
                painter.rect(rect.x, rect.y, rect.width, rect.height);
                painter.clip();
            }
            ChartOp::Restore => painter.restore(),
            ChartOp::Rectangle { rect, fill, stroke } => {
                if let Some(fill) = fill {
                    painter.set_fill_style_str(fill);
                    painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
                }
                if let Some(stroke) = stroke {
                    painter.set_stroke_style_str(stroke);
                    painter.set_line_width(1.0);
                    painter.set_line_dash(&[]);
                    painter.stroke_rect(rect.x, rect.y, rect.width, rect.height);
                }
            }
            ChartOp::Path {
                points,
                closed,
                fill,
                stroke,
                width,
                dash,
            } => {
                painter.begin_path();
                if let Some((px, py)) = points.first() {
                    painter.move_to(*px, *py);
                }
                for (px, py) in points.iter().skip(1) {
                    painter.line_to(*px, *py);
                }
                if *closed {
                    painter.close_path();
                }
                if let Some(fill) = fill {
                    painter.set_fill_style_str(fill);
                    painter.fill();
                }
                if let Some(stroke) = stroke {
                    painter.set_stroke_style_str(stroke);
                    painter.set_line_width(*width);
                    painter.set_line_dash(dash);
                    painter.stroke();
                    painter.set_line_dash(&[]);
                }
            }
            ChartOp::Text {
                text,
                x,
                y,
                font,
                color,
                align: _,
                rotation,
            } => {
                let mut shorthand = String::new();
                if font.bold {
                    shorthand.push_str("bold ");
                }
                if font.italic {
                    shorthand.push_str("italic ");
                }
                shorthand.push_str(&format!("{:.2}pt '{}'", font.size_points, font.family));
                painter.set_font(&shorthand);
                painter.set_text_baseline("alphabetic");
                painter.set_fill_style_str(color);
                if *rotation == 0.0 {
                    let _ = painter.fill_text(text, *x, *y);
                } else {
                    // A value axis title, reading upwards.
                    painter.save();
                    let _ = painter.translate(*x, *y);
                    let _ = painter.rotate(*rotation);
                    let _ = painter.fill_text(text, 0.0, 0.0);
                    painter.restore();
                }
            }
        }
    }
    painter.restore();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_chart_lays_out_with_its_title_legend_and_bars() {
        let chart = crate::document::RunChart {
            kind: crate::document::ChartKind::Bar,
            title: "売上".into(),
            categories: vec!["A".into(), "B".into()],
            series: vec![crate::document::ChartSeries {
                name: "系列 1".into(),
                values: vec![Some(1.0), Some(3.0)],
            }],
            legend: Some("b".into()),
            xml: None,
        };
        let xml = crate::chart::build_chart_xml(&chart, None).unwrap();
        let ops = chart_ops(&xml, (0.0, 0.0, 400.0, 300.0)).expect("drawn");
        let texts: Vec<&str> = ops
            .iter()
            .filter_map(|op| match op {
                ChartOp::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"売上"), "{texts:?}");
        assert!(texts.contains(&"系列 1"), "the legend: {texts:?}");
        assert!(texts.contains(&"A") && texts.contains(&"B"), "{texts:?}");
    }
}
