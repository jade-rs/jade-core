//! Sparklines written back as Excel 2010 saves them: the worksheet's
//! `x14:sparklineGroups` extension, every group in it, replaced whole.

use jade_xlsx_reader::model::Color;
use jade_xlsx_reader::sparkline::SparklineGroup;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::patch::{apply, attribute, escape, local, namespace_prefix, Splice};
use crate::{Result, WriteError};

/// The extension's `uri`, as Excel names the sparkline one.
pub const SPARKLINE_EXTENSION: &str = "{05C60535-1F16-4fd2-B633-F4F36F0B64E0}";
const X14_NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM_NS: &str = "http://schemas.microsoft.com/office/excel/2006/main";

fn color(name: &str, color: Option<&Color>) -> String {
    let Some(color) = color else {
        return String::new();
    };
    let attributes = match color {
        Color::Argb(argb) => format!(
            " rgb=\"{:02X}{:02X}{:02X}{:02X}\"",
            argb[0], argb[1], argb[2], argb[3]
        ),
        Color::Indexed(index) => format!(" indexed=\"{index}\""),
        Color::Theme { index, tint, .. } => {
            if *tint == 0.0 {
                format!(" theme=\"{index}\"")
            } else {
                format!(" theme=\"{index}\" tint=\"{tint}\"")
            }
        }
        Color::Auto => " auto=\"1\"".to_string(),
    };
    format!("<x14:{name}{attributes}/>")
}

fn a1(cell: jade_xlsx_reader::CellRef) -> String {
    let mut label = Vec::new();
    let mut remaining = u64::from(cell.column) + 1;
    while remaining > 0 {
        label.push(b'A' + ((remaining - 1) % 26) as u8);
        remaining = (remaining - 1) / 26;
    }
    label.reverse();
    format!(
        "{}{}",
        String::from_utf8(label).expect("ASCII"),
        cell.row + 1
    )
}

fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// One group as `x14:sparklineGroup`, its attributes only where they are
/// not the schema's defaults.
fn group_xml(group: &SparklineGroup) -> String {
    let mut head = String::from("<x14:sparklineGroup");
    if let Some(max) = group.manual_max {
        head.push_str(&format!(" manualMax=\"{}\"", number(max)));
    }
    if let Some(min) = group.manual_min {
        head.push_str(&format!(" manualMin=\"{}\"", number(min)));
    }
    if (group.line_weight_points - 0.75).abs() > f64::EPSILON {
        head.push_str(&format!(
            " lineWeight=\"{}\"",
            number(group.line_weight_points)
        ));
    }
    if group.kind != "line" {
        head.push_str(&format!(" type=\"{}\"", escape(&group.kind)));
    }
    if group.date_axis {
        head.push_str(" dateAxis=\"1\"");
    }
    head.push_str(&format!(
        " displayEmptyCellsAs=\"{}\"",
        escape(&group.empty_cells)
    ));
    for (name, on) in [
        ("markers", group.markers),
        ("high", group.high),
        ("low", group.low),
        ("first", group.first),
        ("last", group.last),
        ("negative", group.negative),
        ("displayXAxis", group.display_axis),
        ("displayHidden", group.display_hidden),
    ] {
        if on {
            head.push_str(&format!(" {name}=\"1\""));
        }
    }
    if group.min_axis != "individual" {
        head.push_str(&format!(" minAxisType=\"{}\"", escape(&group.min_axis)));
    }
    if group.max_axis != "individual" {
        head.push_str(&format!(" maxAxisType=\"{}\"", escape(&group.max_axis)));
    }
    if group.right_to_left {
        head.push_str(" rightToLeft=\"1\"");
    }
    head.push('>');
    for (name, value) in [
        ("colorSeries", &group.color_series),
        ("colorNegative", &group.color_negative),
        ("colorAxis", &group.color_axis),
        ("colorMarkers", &group.color_markers),
        ("colorFirst", &group.color_first),
        ("colorLast", &group.color_last),
        ("colorHigh", &group.color_high),
        ("colorLow", &group.color_low),
    ] {
        head.push_str(&color(name, value.as_ref()));
    }
    if let Some(range) = group.date_range.as_deref().filter(|_| group.date_axis) {
        head.push_str(&format!("<xm:f>{}</xm:f>", escape(range)));
    }
    head.push_str("<x14:sparklines>");
    for sparkline in &group.sparklines {
        head.push_str(&format!(
            "<x14:sparkline><xm:f>{}</xm:f><xm:sqref>{}</xm:sqref></x14:sparkline>",
            escape(&sparkline.formula),
            a1(sparkline.cell)
        ));
    }
    head.push_str("</x14:sparklines></x14:sparklineGroup>");
    head
}

/// The sheet's XML with its sparklines `groups`, and no others: the
/// sparkline extension replaced, made, or — for none — taken away.
pub fn set_sparkline_groups(xml: &str, groups: &[SparklineGroup]) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut depth = 0usize;
    let mut prefix = String::new();
    let mut ext_list: Option<(usize, usize)> = None;
    let mut ext_list_empty: Option<(usize, usize)> = None;
    let mut sparkline_ext: Option<(usize, usize)> = None;
    let mut ext_start: Option<(usize, bool)> = None;
    let mut root_close = None;
    let mut ext_list_start = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) => {
                let name = element.name();
                if depth == 0 {
                    prefix = namespace_prefix(name.as_ref());
                }
                if depth == 1 && local(name.as_ref()) == b"extLst" {
                    ext_list_start = Some(start);
                }
                if depth == 2 && ext_list_start.is_some() && local(name.as_ref()) == b"ext" {
                    let ours = attribute(element, b"uri")?.as_deref() == Some(SPARKLINE_EXTENSION);
                    ext_start = Some((start, ours));
                }
                depth += 1;
            }
            Event::Empty(ref element) => {
                if depth == 1 && local(element.name().as_ref()) == b"extLst" {
                    ext_list_empty = Some((start, end));
                }
            }
            Event::End(ref element) => {
                depth -= 1;
                let name = local(element.name().as_ref()).to_vec();
                if depth == 2 && name == b"ext" {
                    if let Some((at, true)) = ext_start.take() {
                        sparkline_ext = Some((at, end));
                    }
                }
                if depth == 1 && name == b"extLst" {
                    ext_list = ext_list_start.map(|at| (at, start));
                }
                if depth == 0 {
                    root_close = Some(start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    // `namespace_prefix` keeps its colon: `x:` or nothing.
    let p = prefix;
    let content = if groups.is_empty() {
        String::new()
    } else {
        let body: String = groups.iter().map(group_xml).collect();
        format!(
            "<{p}ext uri=\"{SPARKLINE_EXTENSION}\" xmlns:x14=\"{X14_NS}\"><x14:sparklineGroups xmlns:xm=\"{XM_NS}\">{body}</x14:sparklineGroups></{p}ext>"
        )
    };
    let splice = if let Some((start, end)) = sparkline_ext {
        Splice {
            start,
            end,
            text: content,
        }
    } else if content.is_empty() {
        return Ok(xml.to_string());
    } else if let Some((_, close)) = ext_list {
        Splice {
            start: close,
            end: close,
            text: content,
        }
    } else if let Some((start, end)) = ext_list_empty {
        Splice {
            start,
            end,
            text: format!("<{p}extLst>{content}</{p}extLst>"),
        }
    } else {
        let at = root_close.ok_or_else(|| WriteError::MissingPart("worksheet".into()))?;
        Splice {
            start: at,
            end: at,
            text: format!("<{p}extLst>{content}</{p}extLst>"),
        }
    };
    Ok(apply(xml, vec![splice]))
}
