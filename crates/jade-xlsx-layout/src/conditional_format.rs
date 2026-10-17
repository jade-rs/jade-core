//! Evaluate supported conditional rules once through the shared layout path.

use crate::visual_format::{PreparedVisual, ResolvedDataBar};
use crate::{ResolvedFill, ResolvedFont};
use jade_xlsx_reader::{
    conditional_format::{ConditionalRule, DifferentialBorder, DifferentialStyle},
    Border, CellRange, CellRef, CellValue, Sheet, StyleTable,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlankConditionalRule {
    pub ranges: Vec<CellRange>,
    pub style: DifferentialStyle,
    pub stop_if_true: bool,
}

struct PreparedRule<'a> {
    ranges: &'a [CellRange],
    rule: &'a ConditionalRule,
    visual: Option<Result<PreparedVisual, String>>,
}

pub struct ConditionalStyles<'a> {
    rules: Vec<PreparedRule<'a>>,
    styles: &'a StyleTable,
}

impl<'a> ConditionalStyles<'a> {
    pub fn new(sheet: &'a Sheet, styles: &'a StyleTable) -> Self {
        let mut rules: Vec<_> = sheet
            .conditional_formats
            .iter()
            .flat_map(|format| {
                format
                    .rules
                    .iter()
                    .map(|rule| (format.ranges.as_slice(), rule))
            })
            .collect();
        rules.sort_by_key(|(_, rule)| rule.priority);
        let mut samples: Vec<(&[CellRange], Vec<f64>)> = Vec::new();
        let rules = rules
            .into_iter()
            .map(|(ranges, rule)| {
                let visual = rule.visual.as_ref().map(|_| {
                    if let Some(issue) = support_issue(rule, styles) {
                        return Err(issue);
                    }
                    let index = if let Some(index) =
                        samples.iter().position(|(other, _)| *other == ranges)
                    {
                        index
                    } else {
                        let mut values: Vec<f64> = sheet
                            .rows
                            .iter()
                            .flat_map(|row| &row.cells)
                            .filter(|cell| {
                                ranges.iter().any(|range| range.contains(cell.reference))
                            })
                            .filter_map(|cell| match cell.value {
                                CellValue::Number(number) if number.is_finite() => Some(number),
                                _ => None,
                            })
                            .collect();
                        values.sort_by(f64::total_cmp);
                        samples.push((ranges, values));
                        samples.len() - 1
                    };
                    PreparedVisual::new(rule, &samples[index].1)
                });
                PreparedRule {
                    ranges,
                    rule,
                    visual,
                }
            })
            .collect();
        Self { rules, styles }
    }

    pub fn issue(&self, rule: &ConditionalRule) -> Option<String> {
        support_issue(rule, self.styles).or_else(|| {
            self.rules
                .iter()
                .find(|prepared| std::ptr::eq(prepared.rule, rule))
                .and_then(|prepared| prepared.visual.as_ref()?.as_ref().err().cloned())
        })
    }

    pub fn for_cell(&self, reference: CellRef, value: &CellValue) -> DifferentialStyle {
        self.for_cell_with_visual(reference, value).0
    }

    pub(crate) fn for_cell_with_visual(
        &self,
        reference: CellRef,
        value: &CellValue,
    ) -> (DifferentialStyle, Option<ResolvedDataBar>) {
        let mut result = DifferentialStyle::default();
        let mut bar = None;
        for prepared in &self.rules {
            let rule = prepared.rule;
            if !prepared
                .ranges
                .iter()
                .any(|range| range.contains(reference))
            {
                continue;
            }
            let matching = match &prepared.visual {
                Some(Ok(_)) => {
                    Some(matches!(value,CellValue::Number(number) if number.is_finite()))
                }
                Some(Err(_)) => None,
                None => matches_rule(rule, value),
            };
            match matching {
                Some(true) => {}
                None if rule.stop_if_true => break,
                _ => continue,
            }
            if let Some(Ok(visual)) = &prepared.visual {
                let (fill, data_bar) = visual.for_cell(value);
                if result.fill.is_none() {
                    result.fill = fill.map(|color| jade_xlsx_reader::Fill {
                        pattern: Some("solid".into()),
                        foreground: Some(color),
                        background: None,
                    });
                }
                if bar.is_none() {
                    bar = data_bar;
                }
            }
            if let Some(style) = rule
                .differential_style_index
                .and_then(|index| self.styles.differential_styles.get(index as usize))
            {
                inherit_missing(&mut result, style);
            }
            if rule.stop_if_true {
                break;
            }
        }
        (result, bar)
    }

    pub fn blank_rules(&self) -> Vec<BlankConditionalRule> {
        self.rules
            .iter()
            .filter_map(|prepared| {
                let ranges = prepared.ranges;
                let rule = prepared.rule;
                let matches = matches_rule(rule, &CellValue::Empty);
                if matches == Some(false) || (matches.is_none() && !rule.stop_if_true) {
                    return None;
                }
                // A true stop rule without a style still prevents lower rules.
                let style = if matches.is_none() {
                    DifferentialStyle::default()
                } else {
                    rule.differential_style_index
                        .and_then(|index| self.styles.differential_styles.get(index as usize))
                        .cloned()
                        .unwrap_or_default()
                };
                Some(BlankConditionalRule {
                    ranges: ranges.to_vec(),
                    style,
                    stop_if_true: rule.stop_if_true,
                })
            })
            .collect()
    }
}

pub fn blank_style(rules: &[BlankConditionalRule], reference: CellRef) -> DifferentialStyle {
    let mut result = DifferentialStyle::default();
    for rule in rules {
        if !rule.ranges.iter().any(|range| range.contains(reference)) {
            continue;
        }
        inherit_missing(&mut result, &rule.style);
        if rule.stop_if_true {
            break;
        }
    }
    result
}

fn inherit_missing(target: &mut DifferentialStyle, source: &DifferentialStyle) {
    macro_rules! inherit { ($($field:ident),*) => { $(if target.$field.is_none() { target.$field = source.$field.clone(); })* }; }
    inherit!(
        text_color,
        bold,
        italic,
        underline,
        strike,
        fill,
        left,
        right,
        top,
        bottom,
        number_format_code
    );
}

pub fn apply_font(style: &DifferentialStyle, font: &mut ResolvedFont) {
    if let Some(value) = style.bold {
        font.bold = value;
    }
    if let Some(value) = style.italic {
        font.italic = value;
    }
    if let Some(value) = style.underline {
        font.underline = value;
    }
    if let Some(value) = style.strike {
        font.strike = value;
    }
}

pub fn apply_border(style: &DifferentialStyle, border: &mut Border) {
    let resolve = |value: &DifferentialBorder| match value {
        DifferentialBorder::Clear => None,
        DifferentialBorder::Edge(edge) => Some(edge.clone()),
    };
    if let Some(value) = &style.left {
        border.left = resolve(value);
    }
    if let Some(value) = &style.right {
        border.right = resolve(value);
    }
    if let Some(value) = &style.top {
        border.top = resolve(value);
    }
    if let Some(value) = &style.bottom {
        border.bottom = resolve(value);
    }
}

pub fn conditional_fill(style: &DifferentialStyle) -> Option<Option<ResolvedFill>> {
    style.fill.as_ref().map(|fill| {
        let pattern = fill.pattern.as_deref().unwrap_or("solid");
        if pattern == "none" {
            None
        } else {
            Some(ResolvedFill {
                pattern: pattern.to_string(),
                foreground: fill.foreground.or(fill.background),
                background: fill.background,
            })
        }
    })
}

/// None means unsupported, rather than a false evaluation of a known rule.
pub fn matches_rule(rule: &ConditionalRule, value: &CellValue) -> Option<bool> {
    if !rule.issues.is_empty() {
        return None;
    }
    let blank = matches!(value, CellValue::Empty)
        || matches!(value, CellValue::Text(text) if text.trim_matches(' ').is_empty());
    if matches!(rule.rule_type.as_str(), "colorScale" | "dataBar") {
        if rule.visual.is_none() || crate::visual_format::issue(rule).is_some() {
            return None;
        }
        return Some(matches!(value,CellValue::Number(number) if number.is_finite()));
    }
    match rule.rule_type.as_str() {
        "containsBlanks" => Some(blank),
        "notContainsBlanks" => Some(!blank),
        "containsErrors" => Some(matches!(value, CellValue::Error(_))),
        "notContainsErrors" => Some(!matches!(value, CellValue::Error(_))),
        "containsText" | "notContainsText" | "beginsWith" | "endsWith" => {
            let needle = rule.text.as_ref()?.to_lowercase();
            let text = crate::number_format::format_value(
                value,
                None,
                jade_xlsx_reader::DateSystem::Nineteen00,
            )
            .text
            .to_lowercase();
            Some(match rule.rule_type.as_str() {
                "containsText" => wildcard_search(&needle, &text),
                "notContainsText" => !wildcard_search(&needle, &text),
                "beginsWith" => text.starts_with(&needle),
                _ => text.ends_with(&needle),
            })
        }
        "cellIs" => {
            let first = literal(rule.formulas.first()?)?;
            let comparison = compare(value, &first);
            Some(match rule.operator.as_deref()? {
                "equal" => comparison == Some(Ordering::Equal),
                "notEqual" => comparison.is_some_and(|order| order != Ordering::Equal),
                "greaterThan" => comparison == Some(Ordering::Greater),
                "greaterThanOrEqual" => comparison.is_some_and(|order| order != Ordering::Less),
                "lessThan" => comparison == Some(Ordering::Less),
                "lessThanOrEqual" => comparison.is_some_and(|order| order != Ordering::Greater),
                "between" | "notBetween" => {
                    let second = literal(rule.formulas.get(1)?)?;
                    let high = compare(value, &second);
                    let between = comparison.is_some_and(|order| order != Ordering::Less)
                        && high.is_some_and(|order| order != Ordering::Greater);
                    if rule.operator.as_deref() == Some("notBetween") {
                        comparison.is_some() && high.is_some() && !between
                    } else {
                        between
                    }
                }
                _ => return None,
            })
        }
        _ => None,
    }
}

fn wildcard_search(pattern: &str, text: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    let mut matches = vec![true; text.len() + 1];
    let mut chars = pattern.chars();
    while let Some(character) = chars.next() {
        let escaped = character == '~';
        let character = if escaped {
            chars.next().unwrap_or('~')
        } else {
            character
        };
        let mut next = vec![false; text.len() + 1];
        if character == '*' && !escaped {
            next[0] = matches[0];
            for i in 1..=text.len() {
                next[i] = matches[i] || next[i - 1];
            }
        } else {
            for i in 1..=text.len() {
                next[i] =
                    matches[i - 1] && ((character == '?' && !escaped) || text[i - 1] == character);
            }
        }
        matches = next;
    }
    matches.into_iter().any(|value| value)
}

pub fn support_issue(rule: &ConditionalRule, styles: &StyleTable) -> Option<String> {
    if !rule.issues.is_empty() {
        return Some(rule.issues.join(", "));
    }
    if let Some(issue) = crate::visual_format::issue(rule) {
        return Some(issue);
    }
    if matches_rule(rule, &CellValue::Empty).is_none() {
        return Some("rule type, operator, or formula operands are not supported".into());
    }
    if let Some(index) = rule.differential_style_index {
        let Some(style) = styles.differential_styles.get(index as usize) else {
            return Some("differential style index is missing".into());
        };
        if !style.unsupported_properties.is_empty() {
            return Some(format!(
                "unsupported differential properties: {}",
                style.unsupported_properties.join(", ")
            ));
        }
    }
    None
}

fn literal(formula: &str) -> Option<CellValue> {
    let formula = formula.trim().strip_prefix('=').unwrap_or(formula.trim());
    if let Ok(number) = formula.parse::<f64>() {
        return number.is_finite().then_some(CellValue::Number(number));
    }
    if formula.eq_ignore_ascii_case("TRUE") {
        return Some(CellValue::Bool(true));
    }
    if formula.eq_ignore_ascii_case("FALSE") {
        return Some(CellValue::Bool(false));
    }
    let text = formula.strip_prefix('"')?.strip_suffix('"')?;
    // Doubled quotes are the only escape in an Excel string literal.
    let mut decoded = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '"' && chars.next() != Some('"') {
            return None;
        }
        decoded.push(c);
    }
    Some(CellValue::Text(decoded))
}

fn compare(value: &CellValue, operand: &CellValue) -> Option<Ordering> {
    match (value, operand) {
        (CellValue::Number(value), CellValue::Number(operand)) => value.partial_cmp(operand),
        (CellValue::Empty, CellValue::Number(operand)) => 0.0f64.partial_cmp(operand),
        (CellValue::Empty, CellValue::Text(operand)) => {
            Some(String::new().cmp(&operand.to_lowercase()))
        }
        (CellValue::Text(value), CellValue::Text(operand)) => {
            Some(value.to_lowercase().cmp(&operand.to_lowercase()))
        }
        (CellValue::Bool(value), CellValue::Bool(operand)) => Some(value.cmp(operand)),
        _ => None,
    }
}
