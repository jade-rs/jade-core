//! Conditional formats made, listed, deleted and cleared, as Excel's
//! Conditional Formatting menu and its Rules Manager do. Each change is one
//! write and one undo step; a protected sheet refuses them unless its
//! protection allows formatting cells, as Excel's does.

use jade_xlsx_reader::conditional_format::ConditionalRule;
use jade_xlsx_reader::CellRange;
pub use jade_xlsx_writer::{ConditionalStyle, NewConditionalRule};

use crate::{format_reference, parse_range, HeadlessError, HeadlessWorkbook, Result};

/// One rule as the Rules Manager lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalFormatView {
    /// What it applies to: `B2:B4`, or several ranges space-separated.
    pub range: String,
    /// The rule in words, as Excel's Rules Manager puts it.
    pub description: String,
    /// The menu rule it is, when it is one — for a shell to describe it in
    /// its own language.
    pub rule: Option<NewConditionalRule>,
}

const WHAT: &str = "formatting cells is not allowed";

fn range_text(range: CellRange) -> String {
    if range.start == range.end {
        format_reference(range.start)
    } else {
        format!(
            "{}:{}",
            format_reference(range.start),
            format_reference(range.end)
        )
    }
}

/// The menu rule a file's rule is, when it is one.
fn menu_rule(rule: &ConditionalRule) -> Option<NewConditionalRule> {
    let value = |index: usize| rule.formulas.get(index).cloned();
    let ranking = &rule.ranking;
    Some(match (rule.rule_type.as_str(), rule.operator.as_deref()) {
        ("cellIs", Some("greaterThan")) => NewConditionalRule::GreaterThan(value(0)?),
        ("cellIs", Some("lessThan")) => NewConditionalRule::LessThan(value(0)?),
        ("cellIs", Some("between")) => NewConditionalRule::Between(value(0)?, value(1)?),
        ("cellIs", Some("equal")) => NewConditionalRule::EqualTo(value(0)?),
        ("containsText", _) => NewConditionalRule::TextContains(rule.text.clone()?),
        ("duplicateValues", _) => NewConditionalRule::DuplicateValues,
        ("top10", _) if ranking.bottom => NewConditionalRule::Bottom {
            rank: ranking.rank.unwrap_or(10),
            percent: ranking.percent,
        },
        ("top10", _) => NewConditionalRule::Top {
            rank: ranking.rank.unwrap_or(10),
            percent: ranking.percent,
        },
        ("aboveAverage", _) if ranking.std_dev.is_some() || ranking.equal_average => return None,
        ("aboveAverage", _) if ranking.below_average => NewConditionalRule::BelowAverage,
        ("aboveAverage", _) => NewConditionalRule::AboveAverage,
        _ => return None,
    })
}

fn describe(rule: &ConditionalRule) -> String {
    let value = |index: usize| rule.formulas.get(index).cloned().unwrap_or_default();
    match menu_rule(rule) {
        Some(NewConditionalRule::GreaterThan(value)) => format!("Cell Value > {value}"),
        Some(NewConditionalRule::LessThan(value)) => format!("Cell Value < {value}"),
        Some(NewConditionalRule::Between(low, high)) => {
            format!("Cell Value between {low} and {high}")
        }
        Some(NewConditionalRule::EqualTo(value)) => format!("Cell Value = {value}"),
        Some(NewConditionalRule::TextContains(text)) => format!("Cell Value contains \"{text}\""),
        Some(NewConditionalRule::DuplicateValues) => "Duplicate Values".into(),
        Some(NewConditionalRule::Top { rank, percent }) => {
            format!("Top {rank}{}", if percent { "%" } else { "" })
        }
        Some(NewConditionalRule::Bottom { rank, percent }) => {
            format!("Bottom {rank}{}", if percent { "%" } else { "" })
        }
        Some(NewConditionalRule::AboveAverage) => "Above Average".into(),
        Some(NewConditionalRule::BelowAverage) => "Below Average".into(),
        Some(NewConditionalRule::DataBar(_) | NewConditionalRule::ColorScale(_)) => unreachable!(),
        None => match rule.rule_type.as_str() {
            "dataBar" => "Data Bar".into(),
            "colorScale" => "Graded Color Scale".into(),
            "iconSet" => "Icon Set".into(),
            "expression" => format!("Formula: ={}", value(0)),
            "cellIs" => format!(
                "Cell Value {} {}",
                rule.operator.as_deref().unwrap_or_default(),
                rule.formulas.join(" and ")
            ),
            "uniqueValues" => "Unique Values".into(),
            "containsBlanks" => "Blanks".into(),
            "notContainsBlanks" => "No Blanks".into(),
            "containsErrors" => "Errors".into(),
            "notContainsErrors" => "No Errors".into(),
            "timePeriod" => "Dates Occurring".into(),
            "beginsWith" => format!(
                "Cell Value begins with \"{}\"",
                rule.text.clone().unwrap_or_default()
            ),
            "endsWith" => format!(
                "Cell Value ends with \"{}\"",
                rule.text.clone().unwrap_or_default()
            ),
            "notContainsText" => format!(
                "Cell Value does not contain \"{}\"",
                rule.text.clone().unwrap_or_default()
            ),
            other => other.to_string(),
        },
    }
}

impl HeadlessWorkbook {
    /// The sheet's conditional formats as Excel's Rules Manager lists them:
    /// first the one that wins, which is the newest unless reordered.
    pub fn conditional_formats(&self, sheet: &str) -> Result<Vec<ConditionalFormatView>> {
        let index = self.sheet_index(sheet)?;
        let mut rules: Vec<(&[CellRange], &ConditionalRule)> = self.workbook.sheets[index]
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
        Ok(rules
            .into_iter()
            .map(|(ranges, rule)| ConditionalFormatView {
                range: ranges
                    .iter()
                    .map(|range| range_text(*range))
                    .collect::<Vec<_>>()
                    .join(" "),
                description: describe(rule),
                rule: menu_rule(rule).or_else(|| visual_rule(rule)),
            })
            .collect())
    }

    /// Adds `rule` over `range` in `style` as Excel's Conditional
    /// Formatting menu does: it goes first, ahead of the sheet's others.
    pub fn add_conditional_format(
        &mut self,
        sheet: &str,
        range: &str,
        rule: &NewConditionalRule,
        style: ConditionalStyle,
    ) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, WHAT, |p| p.format_cells)?;
        let range = parse_range(range)?;
        match rule {
            NewConditionalRule::ColorScale(colors) if !(2..=3).contains(&colors.len()) => {
                return Err(HeadlessError::BadReference(format!(
                    "a colour scale has 2 or 3 colours, not {}",
                    colors.len()
                )));
            }
            NewConditionalRule::Top { rank, percent }
            | NewConditionalRule::Bottom { rank, percent }
                if *rank == 0 || (*percent && *rank > 100) || *rank > 1000 =>
            {
                return Err(HeadlessError::BadReference(rank.to_string()));
            }
            _ => {}
        }
        let part_name = self.workbook.sheets[index].part_name.clone();
        let (styles_name, styles) = jade_xlsx_writer::styles_part(&self.source)?;
        let (sheet_xml, styles) = jade_xlsx_writer::add_conditional_format(
            &self.sheet_part(&part_name)?,
            &styles,
            range,
            rule,
            style,
        )?;
        // A workbook without a styles part gets one, as a format change gives it.
        self.write_edits_with_parts(
            Vec::new(),
            Some((styles_name, styles)),
            vec![(part_name, sheet_xml)],
        )
    }

    /// Deletes the rule at `index` in [`Self::conditional_formats`]' order,
    /// as the Rules Manager's Delete Rule does.
    pub fn delete_conditional_format(&mut self, sheet: &str, index: usize) -> Result<()> {
        let sheet_index = self.sheet_index(sheet)?;
        self.guard(sheet_index, WHAT, |p| p.format_cells)?;
        let count: usize = self.workbook.sheets[sheet_index]
            .conditional_formats
            .iter()
            .map(|format| format.rules.len())
            .sum();
        if index >= count {
            return Err(HeadlessError::BadReference(format!(
                "conditional format {index}"
            )));
        }
        let part_name = self.workbook.sheets[sheet_index].part_name.clone();
        let patched =
            jade_xlsx_writer::delete_conditional_format(&self.sheet_part(&part_name)?, index)?;
        self.replace_parts(vec![(part_name, patched)])
    }

    /// Clears the sheet's conditional formats — all of them, or, as Clear
    /// Rules from Selected Cells does, only from `range`: a rule keeps the
    /// rest of the cells it applied to. False, and no undo step, when
    /// there was nothing there to clear.
    pub fn clear_conditional_formats(&mut self, sheet: &str, range: Option<&str>) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, WHAT, |p| p.format_cells)?;
        let range = range.map(parse_range).transpose()?;
        let part_name = self.workbook.sheets[index].part_name.clone();
        let source = self.sheet_part(&part_name)?;
        let patched = jade_xlsx_writer::clear_conditional_formats(&source, range)?;
        if patched == source {
            return Ok(false);
        }
        self.replace_parts(vec![(part_name, patched)])?;
        Ok(true)
    }
}

/// A data bar or colour scale as the menu makes them, with its colours.
fn visual_rule(rule: &ConditionalRule) -> Option<NewConditionalRule> {
    use jade_xlsx_reader::conditional_format::ConditionalVisual;
    use jade_xlsx_reader::Color;
    let rgb = |color: &Color| match color {
        Color::Argb([_, r, g, b]) => Some(u32::from_be_bytes([0, *r, *g, *b])),
        _ => None,
    };
    match rule.visual.as_ref()? {
        ConditionalVisual::DataBar { colors, .. } => {
            Some(NewConditionalRule::DataBar(rgb(colors.first()?)?))
        }
        ConditionalVisual::ColorScale { colors, .. } => Some(NewConditionalRule::ColorScale(
            colors.iter().map(rgb).collect::<Option<Vec<_>>>()?,
        )),
        ConditionalVisual::IconSet { .. } => None,
    }
}
