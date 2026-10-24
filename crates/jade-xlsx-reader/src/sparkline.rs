//! Sparklines: the small charts Excel 2010 draws inside cells, saved in the
//! worksheet's `x14:sparklineGroups` extension.

use quick_xml::events::{BytesStart, Event};
use quick_xml::{name::ResolveResult, NsReader};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{CellRange, CellRef, Color};
use jade_opc::local_name;

use crate::data_validations::{X14_NAMESPACE, XM_NAMESPACE};

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

type Element = (Vec<u8>, Vec<u8>);

fn is_groups_container(stack: &[Element]) -> bool {
    if stack.len() != 4 {
        return false;
    }
    let main = stack[0].0.as_slice();
    matches!(
        main,
        b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
            | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
    ) && stack[0].1 == b"worksheet"
        && stack[1].0 == main
        && stack[1].1 == b"extLst"
        && stack[2].0 == main
        && stack[2].1 == b"ext"
        && stack[3].0 == X14_NAMESPACE
        && stack[3].1 == b"sparklineGroups"
}

struct PendingSparkline {
    depth: usize,
    formula: String,
    cell: Option<CellRef>,
}
struct Leaf {
    depth: usize,
    formula: bool,
    text: String,
}

fn finish_leaf(sparkline: &mut PendingSparkline, formula: bool, text: &str) {
    if formula {
        sparkline.formula = text.trim().to_string();
    } else {
        sparkline.cell = CellRange::parse(text.trim())
            .ok()
            .filter(|range| range.start == range.end)
            .map(|range| range.start);
    }
}

/// The owned x14 groups of a worksheet. Foreign and misplaced lookalikes stay
/// opaque; a missing/invalid display cell never silently becomes A1.
pub fn parse_sparklines(xml: &str) -> Result<Vec<SparklineGroup>> {
    if !xml.contains("sparklineGroup") {
        return Ok(Vec::new());
    }
    let mut reader = NsReader::from_str(xml);
    let mut stack: Vec<Element> = Vec::new();
    let mut groups = Vec::new();
    let mut group: Option<(usize, SparklineGroup)> = None;
    let mut sparkline: Option<PendingSparkline> = None;
    let mut leaf: Option<Leaf> = None;
    loop {
        let event = reader.read_event()?;
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                let empty = matches!(event, Event::Empty(_));
                let namespace = match reader.resolver().resolve_element(element.name()).0 {
                    ResolveResult::Unbound => Vec::new(),
                    ResolveResult::Bound(ns) => ns.as_ref().to_vec(),
                    _ => b"?unknown".to_vec(),
                };
                let name = local_name(element.name().as_ref()).to_vec();
                let depth = stack.len();
                if namespace == X14_NAMESPACE
                    && name == b"sparklineGroup"
                    && is_groups_container(&stack)
                {
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
                        group = Some((depth, parsed));
                    }
                } else if namespace == X14_NAMESPACE
                    && group.as_ref().is_some_and(|(at, _)| at + 1 == depth)
                {
                    if let Some((_, group)) = group.as_mut() {
                        match name.as_slice() {
                            b"colorSeries" => {
                                group.color_series = crate::styles::parse_color(element)?
                            }
                            b"colorNegative" => {
                                group.color_negative = crate::styles::parse_color(element)?
                            }
                            b"colorAxis" => group.color_axis = crate::styles::parse_color(element)?,
                            b"colorMarkers" => {
                                group.color_markers = crate::styles::parse_color(element)?
                            }
                            b"colorFirst" => {
                                group.color_first = crate::styles::parse_color(element)?
                            }
                            b"colorLast" => group.color_last = crate::styles::parse_color(element)?,
                            b"colorHigh" => group.color_high = crate::styles::parse_color(element)?,
                            b"colorLow" => group.color_low = crate::styles::parse_color(element)?,
                            _ => {}
                        }
                    }
                } else if namespace == X14_NAMESPACE
                    && name == b"sparkline"
                    && !empty
                    && group.as_ref().is_some_and(|(at, _)| at + 2 == depth)
                    && stack
                        .last()
                        .is_some_and(|(ns, n)| ns == X14_NAMESPACE && n == b"sparklines")
                {
                    sparkline = Some(PendingSparkline {
                        depth,
                        formula: String::new(),
                        cell: None,
                    });
                } else if namespace == XM_NAMESPACE
                    && matches!(name.as_slice(), b"f" | b"sqref")
                    && sparkline.as_ref().is_some_and(|s| s.depth + 1 == depth)
                {
                    let formula = name == b"f";
                    if empty {
                        finish_leaf(sparkline.as_mut().unwrap(), formula, "");
                    } else {
                        leaf = Some(Leaf {
                            depth,
                            formula,
                            text: String::new(),
                        });
                    }
                }
                if !empty {
                    stack.push((namespace, name));
                }
            }
            Event::Text(value) => {
                if let Some(leaf) = leaf.as_mut().filter(|v| stack.len() == v.depth + 1) {
                    leaf.text.push_str(&value.decode()?);
                }
            }
            Event::CData(value) => {
                if let Some(leaf) = leaf.as_mut().filter(|v| stack.len() == v.depth + 1) {
                    leaf.text.push_str(&value.decode()?);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(leaf) = leaf.as_mut().filter(|v| stack.len() == v.depth + 1) {
                    if let Some(character) = reference.resolve_char_ref()? {
                        leaf.text.push(character);
                    } else if let Some(value) =
                        quick_xml::escape::resolve_predefined_entity(&reference.decode()?)
                    {
                        leaf.text.push_str(value);
                    }
                }
            }
            Event::End(_) => {
                let depth = stack.len().saturating_sub(1);
                if leaf.as_ref().is_some_and(|v| v.depth == depth) {
                    let value = leaf.take().unwrap();
                    if let Some(sparkline) = sparkline.as_mut() {
                        finish_leaf(sparkline, value.formula, &value.text);
                    }
                }
                if sparkline.as_ref().is_some_and(|s| s.depth == depth) {
                    let value = sparkline.take().unwrap();
                    if let (Some((_, group)), Some(cell)) = (group.as_mut(), value.cell) {
                        if !value.formula.is_empty() {
                            group.sparklines.push(Sparkline {
                                formula: value.formula,
                                cell,
                            });
                        }
                    }
                }
                if group.as_ref().is_some_and(|(at, _)| *at == depth) {
                    groups.push(group.take().unwrap().1);
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
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
