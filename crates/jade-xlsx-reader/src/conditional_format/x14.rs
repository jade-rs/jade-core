//! Resolve namespace-owned x14 definitions and GUID links without copying cell XML.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
const X14: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &[u8] = b"http://schemas.microsoft.com/office/excel/2006/main";
const RULE_EXT: &str = "{B025F937-C7B1-47D3-B67F-A62EFF666E3E}";
const SHEET_EXT: &str = "{78C0D931-6437-407D-A8EE-F0AAD7539E65}";
const EXT_ISSUE: &str = "conditional formatting extensions are not supported";
#[derive(Clone, Copy, PartialEq)]
enum Ns {
    Main,
    X14,
    Xm,
    Other,
}
#[derive(Clone)]
struct Node {
    ns: Ns,
    name: Vec<u8>,
    attributes: BTreeMap<String, String>,
    children: Vec<Node>,
    text: String,
}
impl Node {
    fn is(&self, ns: Ns, name: &[u8]) -> bool {
        self.ns == ns && self.name == name
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }
    fn children(&self, ns: Ns, name: &[u8]) -> Vec<&Node> {
        self.children
            .iter()
            .filter(|node| node.is(ns, name))
            .collect()
    }
}
fn node(element: &BytesStart<'_>, ns: Ns) -> Result<Node> {
    let mut attributes = BTreeMap::new();
    for value in element.attributes() {
        let value = value?;
        // Namespaced attributes cannot impersonate an unqualified setting.
        if !value.key.as_ref().contains(&b':') {
            attributes.insert(
                String::from_utf8_lossy(value.key.as_ref()).into_owned(),
                value.unescape_value()?.into_owned(),
            );
        }
    }
    Ok(Node {
        ns,
        name: local_name(element.name().as_ref()).to_vec(),
        attributes,
        children: Vec::new(),
        text: String::new(),
    })
}
fn guid(value: &str) -> Option<String> {
    let body = value.trim().strip_prefix('{')?.strip_suffix('}')?;
    if body.len() != 36
        || !body.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return None;
    }
    Some(format!("{{{}}}", body.to_ascii_uppercase()))
}
fn uri(node: &Node, expected: &str) -> bool {
    node.attr("uri").and_then(guid).as_deref() == Some(expected)
}
fn boolean(node: &Node, name: &str, default: bool, issues: &mut Vec<String>) -> bool {
    match node.attr(name) {
        None => default,
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => {
            issues.push(format!("invalid x14 {name}"));
            default
        }
    }
}
fn length(node: &Node, name: &str, default: u32, issues: &mut Vec<String>) -> u32 {
    match node.attr(name) {
        None => default,
        Some(value) => value.parse().unwrap_or_else(|_| {
            issues.push(format!("invalid x14 {name}"));
            u32::MAX
        }),
    }
}
fn color(node: &Node, name: &[u8], issues: &mut Vec<String>) -> Result<Option<Color>> {
    let children = node.children(Ns::X14, name);
    if children.len() > 1 {
        issues.push("duplicate x14 bar color".into());
    }
    let Some(child) = children.first() else {
        return Ok(None);
    };
    // Reuse the standard color parser with reconstructed unqualified attributes.
    let mut element = BytesStart::new("color");
    for (name, value) in &child.attributes {
        element.push_attribute((name.as_str(), quick_xml::escape::escape(value).as_ref()));
    }
    if child.attr("tint").is_some_and(|value| {
        value.parse::<f32>().map_or(true, |value| {
            !value.is_finite() || !(-1.0..=1.0).contains(&value)
        })
    }) {
        issues.push("invalid x14 color tint".into());
    }
    let parsed = crate::styles::parse_color(&element)?;
    if parsed.is_none() || !child.children.is_empty() {
        issues.push("invalid x14 bar color".into());
    }
    Ok(parsed)
}
fn ranges(node: &Node) -> Option<Vec<CellRange>> {
    let refs = node.children(Ns::Xm, b"sqref");
    if refs.len() != 1 || !refs[0].children.is_empty() {
        return None;
    }
    let ranges = refs[0]
        .text
        .split_ascii_whitespace()
        .map(CellRange::parse)
        .collect::<Result<Vec<_>>>()
        .ok()?;
    (!ranges.is_empty()).then_some(ranges)
}
fn equal_ranges(left: &[CellRange], right: &[CellRange]) -> bool {
    let keys = |ranges: &[CellRange]| {
        ranges
            .iter()
            .map(|r| (r.start.column, r.start.row, r.end.column, r.end.row))
            .collect::<BTreeSet<_>>()
    };
    keys(left) == keys(right)
}
fn linked_id(node: &Node) -> Option<String> {
    if node.children.len() != 1 {
        return None;
    }
    let ext = &node.children[0];
    if !ext.is(Ns::Main, b"ext") || !uri(ext, RULE_EXT) || ext.children.len() != 1 {
        return None;
    }
    let id = &ext.children[0];
    if !id.is(Ns::X14, b"id") || !id.children.is_empty() {
        return None;
    }
    guid(&id.text)
}
fn visual(
    rule: &Node,
    base: Option<&ConditionalVisual>,
    right_to_left: bool,
    id: Option<String>,
    issues: &mut Vec<String>,
) -> Result<Option<ConditionalVisual>> {
    if rule.attr("type") == Some("iconSet") && base.is_none() {
        return Ok(icon_set(rule, issues));
    }
    if rule.attr("type") != Some("dataBar") {
        issues.push("unsupported x14 conditional rule type".into());
        return Ok(None);
    }
    if !rule.children(Ns::Xm, b"f").is_empty() || boolean(rule, "activePresent", false, issues) {
        issues.push("x14 rule predicate formulas are not supported".into());
    }
    let bars = rule.children(Ns::X14, b"dataBar");
    if bars.len() != 1 {
        issues.push("x14 data bar definition is missing or duplicated".into());
        return Ok(None);
    }
    let bar = bars[0];
    let mut thresholds = Vec::new();
    for threshold in bar.children(Ns::X14, b"cfvo") {
        let formulas = threshold.children(Ns::Xm, b"f");
        if formulas.len() > 1
            || threshold.children.len() != formulas.len()
            || threshold.attr("val").is_some()
        {
            issues.push("invalid or extended x14 threshold".into());
        }
        thresholds.push(ConditionalThreshold {
            kind: threshold.attr("type").unwrap_or_default().into(),
            value: formulas.first().map(|formula| formula.text.clone()),
            gte: true,
        });
    }
    let gradient = boolean(bar, "gradient", true, issues);
    let border = boolean(bar, "border", false, issues);
    let negative_same_color = boolean(bar, "negativeBarColorSameAsPositive", false, issues);
    let negative_same_border = boolean(bar, "negativeBarBorderColorSameAsPositive", true, issues);
    let direction = match bar.attr("direction").unwrap_or("context") {
        "context" => DataBarDirection::Context,
        "leftToRight" => DataBarDirection::LeftToRight,
        "rightToLeft" => DataBarDirection::RightToLeft,
        _ => {
            issues.push("invalid x14 data bar direction".into());
            DataBarDirection::Context
        }
    };
    let axis_position = match bar.attr("axisPosition").unwrap_or("automatic") {
        "automatic" => DataBarAxisPosition::Automatic,
        "middle" => DataBarAxisPosition::Middle,
        "none" => DataBarAxisPosition::None,
        _ => {
            issues.push("invalid x14 data bar axis position".into());
            DataBarAxisPosition::Automatic
        }
    };
    let fill = color(bar, b"fillColor", issues)?;
    let colors = if base.is_some() {
        if fill.is_some() {
            issues.push("linked x14 data bars must inherit their positive fill".into());
        }
        match base {
            Some(ConditionalVisual::DataBar { colors, .. }) => colors.clone(),
            _ => {
                issues.push("x14 link target is not a data bar".into());
                Vec::new()
            }
        }
    } else {
        fill.into_iter().collect()
    };
    let border_color = color(bar, b"borderColor", issues)?;
    let negative_color = color(bar, b"negativeFillColor", issues)?;
    let negative_border_color = color(bar, b"negativeBorderColor", issues)?;
    let axis_color = color(bar, b"axisColor", issues)?;
    if border && border_color.is_none() {
        issues.push("x14 border color is missing".into());
    }
    if !negative_same_color && negative_color.is_none() {
        issues.push("x14 negative fill color is missing".into());
    }
    if border && !negative_same_border && negative_border_color.is_none() {
        issues.push("x14 negative border color is missing".into());
    }
    if axis_position != DataBarAxisPosition::None && axis_color.is_none() {
        issues.push("x14 axis color is missing".into());
    }
    for child in &bar.children {
        if child.ns != Ns::X14
            || !matches!(
                &child.name[..],
                b"cfvo"
                    | b"fillColor"
                    | b"borderColor"
                    | b"negativeFillColor"
                    | b"negativeBorderColor"
                    | b"axisColor"
            )
        {
            issues.push("unsupported x14 data bar child".into());
        }
    }
    if rule
        .children
        .iter()
        .any(|child| !child.is(Ns::X14, b"dataBar") && !child.is(Ns::Xm, b"f"))
    {
        issues.push("unsupported x14 conditional rule child".into());
    }
    Ok(Some(ConditionalVisual::DataBar {
        thresholds,
        colors,
        min_length: length(bar, "minLength", 10, issues),
        max_length: length(bar, "maxLength", 90, issues),
        show_value: boolean(bar, "showValue", true, issues),
        options: Some(Box::new(DataBarOptions {
            linked_id: id,
            gradient,
            border_color: if border { border_color } else { None },
            negative_color: if negative_same_color {
                None
            } else {
                negative_color
            },
            negative_border_color: if border && !negative_same_border {
                negative_border_color
            } else {
                None
            },
            axis_color: if axis_position == DataBarAxisPosition::None {
                None
            } else {
                axis_color
            },
            negative_same_color,
            negative_same_border,
            axis_position,
            direction,
            context_right_to_left: right_to_left,
        })),
    }))
}

/// An x14 icon set: the 2010 sets (`3Stars`, `3Triangles`, `5Boxes`) and
/// custom sets that borrow each band's icon from another set.
fn icon_set(rule: &Node, issues: &mut Vec<String>) -> Option<ConditionalVisual> {
    if !rule.children(Ns::Xm, b"f").is_empty() {
        issues.push("x14 rule predicate formulas are not supported".into());
    }
    let sets = rule.children(Ns::X14, b"iconSet");
    if sets.len() != 1 {
        issues.push("x14 icon set definition is missing or duplicated".into());
        return None;
    }
    let set = sets[0];
    let mut thresholds = Vec::new();
    for threshold in set.children(Ns::X14, b"cfvo") {
        let formulas = threshold.children(Ns::Xm, b"f");
        if formulas.len() > 1 {
            issues.push("invalid or extended x14 threshold".into());
        }
        thresholds.push(ConditionalThreshold {
            kind: threshold.attr("type").unwrap_or_default().into(),
            value: formulas
                .first()
                .map(|formula| formula.text.clone())
                .or_else(|| threshold.attr("val").map(str::to_string)),
            gte: boolean(threshold, "gte", true, issues),
        });
    }
    let custom = if boolean(set, "custom", false, issues) {
        set.children(Ns::X14, b"cfIcon")
            .into_iter()
            .map(|icon| {
                let name = icon.attr("iconSet").unwrap_or("NoIcons");
                (name != "NoIcons").then(|| CustomIcon {
                    icon_set: name.to_string(),
                    index: icon
                        .attr("iconId")
                        .and_then(|id| id.parse().ok())
                        .unwrap_or(0),
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(ConditionalVisual::IconSet {
        icon_set: set.attr("iconSet").unwrap_or("3TrafficLights1").to_string(),
        thresholds,
        reverse: boolean(set, "reverse", false, issues),
        show_value: boolean(set, "showValue", true, issues),
        custom,
    })
}

pub(super) fn reconcile(xml: &str, formats: &mut Vec<ConditionalFormatting>) -> Result<()> {
    if !xml.contains("http://schemas.microsoft.com/office/spreadsheetml/2009/9/main") {
        return Ok(());
    }
    let mut reader = NsReader::from_str(xml);
    let mut buffer = Vec::new();
    let mut path: Vec<Node> = Vec::new();
    let mut capture: Vec<Node> = Vec::new();
    let mut capture_base = None;
    let mut captures = Vec::new();
    let mut ordinal = 0usize;
    let mut current_rule = None;
    let mut right_to_left = false;
    loop {
        let (namespace, event) = reader.read_resolved_event_into(&mut buffer)?;
        let ns = match namespace {
            ResolveResult::Bound(ref namespace) if namespace.as_ref() == X14 => Ns::X14,
            ResolveResult::Bound(ref namespace) if namespace.as_ref() == XM => Ns::Xm,
            _ if spreadsheet_namespace(namespace) => Ns::Main,
            _ => Ns::Other,
        };
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let info = node(element, ns)?;
                let main_rule = path.len() == 2
                    && path[0].is(Ns::Main, b"worksheet")
                    && path[1].is(Ns::Main, b"conditionalFormatting")
                    && info.is(Ns::Main, b"cfRule");
                if main_rule {
                    current_rule = Some(ordinal);
                    ordinal += 1;
                }
                if path.len() == 2
                    && path[0].is(Ns::Main, b"worksheet")
                    && path[1].is(Ns::Main, b"sheetViews")
                    && info.is(Ns::Main, b"sheetView")
                    && matches!(info.attr("workbookViewId"), None | Some("0"))
                {
                    right_to_left = info
                        .attr("rightToLeft")
                        .is_some_and(|value| matches!(value, "1" | "true"));
                }
                let base = path.len() == 3
                    && current_rule.is_some()
                    && path[0].is(Ns::Main, b"worksheet")
                    && path[1].is(Ns::Main, b"conditionalFormatting")
                    && path[2].is(Ns::Main, b"cfRule")
                    && info.is(Ns::Main, b"extLst");
                let ext = path.len() == 3
                    && path[0].is(Ns::Main, b"worksheet")
                    && path[1].is(Ns::Main, b"extLst")
                    && path[2].is(Ns::Main, b"ext")
                    && uri(&path[2], SHEET_EXT)
                    && info.is(Ns::X14, b"conditionalFormattings");
                if capture.is_empty() && (base || ext) {
                    capture_base = if base { current_rule } else { None };
                }
                if !capture.is_empty() || base || ext {
                    if empty {
                        if let Some(parent) = capture.last_mut() {
                            parent.children.push(info.clone());
                        } else {
                            captures.push((capture_base, info.clone()));
                        }
                    } else {
                        capture.push(info.clone());
                    }
                }
                if !empty {
                    path.push(info);
                } else if main_rule {
                    current_rule = None;
                }
            }
            Event::Text(text) => {
                if let Some(node) = capture.last_mut() {
                    node.text.push_str(&text.decode()?);
                }
            }
            Event::CData(text) => {
                if let Some(node) = capture.last_mut() {
                    node.text.push_str(&text.decode()?);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(node) = capture.last_mut() {
                    if let Some(character) = reference.resolve_char_ref()? {
                        node.text.push(character);
                    } else if let Some(value) =
                        quick_xml::escape::resolve_predefined_entity(&reference.decode()?)
                    {
                        node.text.push_str(value);
                    }
                }
            }
            Event::End(_) => {
                if let Some(node) = capture.pop() {
                    if let Some(parent) = capture.last_mut() {
                        parent.children.push(node);
                    } else {
                        captures.push((capture_base, node));
                    }
                }
                if path.last().is_some_and(|node| node.is(Ns::Main, b"cfRule")) && path.len() == 3 {
                    current_rule = None;
                }
                path.pop();
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    let mut links: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut records: BTreeMap<String, Vec<(&Node, Vec<CellRange>)>> = BTreeMap::new();
    let mut standalone = Vec::new();
    for (base, node) in &captures {
        if let Some(base) = base {
            if let Some(id) = linked_id(node) {
                links.entry(id).or_default().push(*base);
            }
        } else {
            for format in node.children(Ns::X14, b"conditionalFormatting") {
                let ranges = ranges(format).unwrap_or_default();
                for rule in format.children(Ns::X14, b"cfRule") {
                    if rule.attr("priority").is_some() {
                        standalone.push((ranges.clone(), rule));
                    } else if let Some(id) = rule.attr("id").and_then(guid) {
                        records.entry(id).or_default().push((rule, ranges.clone()));
                    }
                }
            }
        }
    }
    let mut rules: Vec<_> = formats
        .iter_mut()
        .flat_map(|format| format.rules.iter_mut().map(|rule| (&format.ranges, rule)))
        .collect();
    for (id, owners) in links {
        for index in &owners {
            let Some((ranges, rule)) = rules.get_mut(*index) else {
                continue;
            };
            let mut issues = Vec::new();
            let record = records.get(&id);
            if owners.len() != 1 || record.is_some_and(|records| records.len() != 1) {
                issues.push("duplicate x14 data bar link".into());
            }
            let Some((record, extent)) = record.and_then(|records| records.first()) else {
                rule.issues
                    .push("x14 data bar counterpart is missing".into());
                continue;
            };
            if !equal_ranges(ranges, extent) {
                issues.push("linked x14 data bar ranges disagree".into());
            }
            let updated = visual(
                record,
                rule.visual.as_ref(),
                right_to_left,
                Some(id.clone()),
                &mut issues,
            )?;
            if let (
                Some(ConditionalVisual::DataBar {
                    min_length: old_min,
                    max_length: old_max,
                    ..
                }),
                Some(ConditionalVisual::DataBar {
                    min_length: new_min,
                    max_length: new_max,
                    ..
                }),
            ) = (rule.visual.as_ref(), updated.as_ref())
            {
                let expected = if (*new_min, *new_max) == (0, 100) {
                    (10, 90)
                } else {
                    (*new_min, *new_max)
                };
                if (*old_min, *old_max) != expected {
                    issues.push("x14 fallback length metadata disagrees".into());
                }
            }
            if issues.is_empty() {
                if rule
                    .issues
                    .iter()
                    .filter(|issue| issue.as_str() == EXT_ISSUE)
                    .count()
                    != 1
                {
                    rule.issues
                        .push("additional conditional extensions are not supported".into());
                    continue;
                }
                rule.visual = updated;
                rule.issues.retain(|issue| issue != EXT_ISSUE);
            } else {
                rule.issues.extend(issues);
            }
        }
    }
    drop(rules);
    let mut priorities: BTreeSet<_> = formats
        .iter()
        .flat_map(|format| format.rules.iter().map(|rule| rule.priority))
        .collect();
    for (ranges, node) in standalone {
        let mut issues = Vec::new();
        let priority = node
            .attr("priority")
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value > 0 && *value <= i32::MAX as u32)
            .unwrap_or_else(|| {
                issues.push("invalid x14 priority".into());
                u32::MAX
            });
        if !priorities.insert(priority) {
            issues.push("duplicate x14 priority".into());
        }
        if ranges.is_empty() {
            issues.push("x14 conditional ranges are missing or invalid".into());
        }
        let stop_if_true = boolean(node, "stopIfTrue", false, &mut issues);
        let visual = visual(node, None, right_to_left, None, &mut issues)?;
        formats.push(ConditionalFormatting {
            ranges,
            rules: vec![ConditionalRule {
                rule_type: node.attr("type").unwrap_or_default().into(),
                priority,
                differential_style_index: None,
                stop_if_true,
                operator: None,
                text: None,
                formulas: Vec::new(),
                visual,
                issues,
                ranking: Default::default(),
                time_period: None,
            }],
        });
    }
    Ok(())
}
