//! Sparklines: the small charts Excel 2010 draws inside cells, saved in the
//! worksheet's `x14:sparklineGroups` extension.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{CellRef, Color};
use jade_opc::local_name;

/// A group of sparklines sharing one look.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SparklineGroup {
    /// `line`, `column` or `stacked` (win/loss).
    pub kind: String,
    pub line_weight_points: f64,
    /// `displayEmptyCellsAs`: `gap`, `zero` or `span`.
    pub empty_cells: String,
    pub markers: bool,
    pub high: bool,
    pub low: bool,
    pub first: bool,
    pub last: bool,
    pub negative: bool,
    pub display_axis: bool,
    pub display_hidden: bool,
    pub right_to_left: bool,
    /// `minAxisType`/`maxAxisType`: `individual`, `group` or `custom`.
    pub min_axis: String,
    pub max_axis: String,
    pub manual_min: Option<f64>,
    pub manual_max: Option<f64>,
    pub color_series: Option<Color>,
    pub color_negative: Option<Color>,
    pub color_axis: Option<Color>,
    pub color_markers: Option<Color>,
    pub color_first: Option<Color>,
    pub color_last: Option<Color>,
    pub color_high: Option<Color>,
    pub color_low: Option<Color>,
    pub sparklines: Vec<Sparkline>,
}

/// One sparkline: the data it plots and the cell it is drawn in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sparkline {
    /// `xm:f`, e.g. `Sheet1!A2:E2`.
    pub formula: String,
    /// `xm:sqref`: the cell.
    pub cell: CellRef,
}

fn attribute(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

fn flag(element: &BytesStart<'_>, key: &[u8]) -> Result<bool> {
    Ok(attribute(element, key)?.is_some_and(|value| value == "1" || value == "true"))
}

/// The sparkline groups of a worksheet's XML.
pub fn parse_sparklines(xml: &str) -> Result<Vec<SparklineGroup>> {
    if !xml.contains("sparklineGroup") {
        return Ok(Vec::new());
    }
    let mut reader = Reader::from_str(xml);
    let mut groups = Vec::new();
    let mut group: Option<SparklineGroup> = None;
    let mut sparkline: Option<Sparkline> = None;
    // Which text is being read: 1 the formula, 2 the cell.
    let mut reading = 0u8;
    let mut text = String::new();
    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf)?;
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name();
                match local_name(qualified.as_ref()) {
                    b"sparklineGroup" => {
                        let number = |key: &[u8]| -> Result<Option<f64>> {
                            Ok(attribute(element, key)?.and_then(|value| value.parse().ok()))
                        };
                        let parsed = SparklineGroup {
                            kind: attribute(element, b"type")?.unwrap_or_else(|| "line".into()),
                            line_weight_points: number(b"lineWeight")?.unwrap_or(0.75),
                            empty_cells: attribute(element, b"displayEmptyCellsAs")?
                                .unwrap_or_else(|| "zero".into()),
                            markers: flag(element, b"markers")?,
                            high: flag(element, b"high")?,
                            low: flag(element, b"low")?,
                            first: flag(element, b"first")?,
                            last: flag(element, b"last")?,
                            negative: flag(element, b"negative")?,
                            display_axis: flag(element, b"displayXAxis")?,
                            display_hidden: flag(element, b"displayHidden")?,
                            right_to_left: flag(element, b"rightToLeft")?,
                            min_axis: attribute(element, b"minAxisType")?
                                .unwrap_or_else(|| "individual".into()),
                            max_axis: attribute(element, b"maxAxisType")?
                                .unwrap_or_else(|| "individual".into()),
                            manual_min: number(b"manualMin")?,
                            manual_max: number(b"manualMax")?,
                            ..Default::default()
                        };
                        if empty {
                            groups.push(parsed);
                        } else {
                            group = Some(parsed);
                        }
                    }
                    name @ (b"colorSeries" | b"colorNegative" | b"colorAxis" | b"colorMarkers"
                    | b"colorFirst" | b"colorLast" | b"colorHigh" | b"colorLow") => {
                        if let Some(group) = group.as_mut() {
                            let color = crate::styles::parse_color(element)?;
                            match name {
                                b"colorSeries" => group.color_series = color,
                                b"colorNegative" => group.color_negative = color,
                                b"colorAxis" => group.color_axis = color,
                                b"colorMarkers" => group.color_markers = color,
                                b"colorFirst" => group.color_first = color,
                                b"colorLast" => group.color_last = color,
                                b"colorHigh" => group.color_high = color,
                                _ => group.color_low = color,
                            }
                        }
                    }
                    b"sparkline" if group.is_some() && !empty => {
                        sparkline = Some(Sparkline {
                            formula: String::new(),
                            cell: CellRef::new(0, 0),
                        });
                    }
                    b"f" if sparkline.is_some() && !empty => {
                        reading = 1;
                        text.clear();
                    }
                    b"sqref" if sparkline.is_some() && !empty => {
                        reading = 2;
                        text.clear();
                    }
                    _ => {}
                }
            }
            Event::Text(value) if reading != 0 => text.push_str(&value.decode()?),
            Event::GeneralRef(reference) if reading != 0 => {
                if let Some(character) = reference.resolve_char_ref()? {
                    text.push(character);
                } else if let Some(value) =
                    quick_xml::escape::resolve_predefined_entity(&reference.decode()?)
                {
                    text.push_str(value);
                }
            }
            Event::End(element) => {
                let qualified = element.name();
                match local_name(qualified.as_ref()) {
                    b"f" if reading == 1 => {
                        if let Some(sparkline) = sparkline.as_mut() {
                            sparkline.formula = text.trim().to_string();
                        }
                        reading = 0;
                    }
                    b"sqref" if reading == 2 => {
                        if let Some(sparkline) = sparkline.as_mut() {
                            if let Ok(cell) = CellRef::parse(text.trim()) {
                                sparkline.cell = cell;
                            }
                        }
                        reading = 0;
                    }
                    b"sparkline" => {
                        if let (Some(group), Some(sparkline)) = (group.as_mut(), sparkline.take()) {
                            if !sparkline.formula.is_empty() {
                                group.sparklines.push(sparkline);
                            }
                        }
                    }
                    b"sparklineGroup" => {
                        if let Some(group) = group.take() {
                            groups.push(group);
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_group_its_colours_and_its_sparklines() {
        let groups = parse_sparklines(
            r#"<worksheet xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><extLst><ext uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}"><x14:sparklineGroups><x14:sparklineGroup type="column" displayEmptyCellsAs="gap" markers="1" high="1" negative="1" displayXAxis="1" lineWeight="1.25"><x14:colorSeries rgb="FF376092"/><x14:colorNegative theme="5"/><x14:colorHigh rgb="FF00B050"/><x14:sparklines><x14:sparkline><xm:f>Data!A2:E2</xm:f><xm:sqref>F2</xm:sqref></x14:sparkline><x14:sparkline><xm:f>Data!A3:E3</xm:f><xm:sqref>F3</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst></worksheet>"#,
        )
        .unwrap();
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.kind, "column");
        assert_eq!(group.empty_cells, "gap");
        assert!(group.markers && group.high && group.negative && group.display_axis);
        assert!(!group.low);
        assert_eq!(group.line_weight_points, 1.25);
        assert_eq!(group.color_series, Some(Color::Argb([255, 55, 96, 146])));
        assert!(matches!(
            group.color_negative,
            Some(Color::Theme { index: 5, .. })
        ));
        assert_eq!(group.sparklines.len(), 2);
        assert_eq!(group.sparklines[1].formula, "Data!A3:E3");
        assert_eq!(group.sparklines[1].cell, CellRef::parse("F3").unwrap());
    }
}
