//! Input-rule authoring by preserving splices. A change to one rule retains
//! sibling rules, container settings, unknown attributes and extensions.
use crate::{
    patch::{apply, escape, namespace_prefix, Splice},
    xml_edit,
    xml_edit_ns::set_child,
    Result, WriteError,
};
use jade_xlsx_reader::{xstring::encode_xstring, CellRange, DataValidation};
use quick_xml::{events::Event, Reader};

const FOLLOWING: &[&[u8]] = &[
    b"hyperlinks",
    b"printOptions",
    b"pageMargins",
    b"pageSetup",
    b"headerFooter",
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];

/// Creates a standard rule (`index=None`) or replaces a standard/x14 rule at `index`,
/// in reader order. Formulas are written for the first range's first cell.
pub fn set_data_validation(
    xml: &str,
    index: Option<usize>,
    rule: &DataValidation,
) -> Result<String> {
    validate(rule)?;
    if let Some(index) = index {
        return edit_record(xml, index, Some(rule));
    }
    crate::xml_edit_ns::edit_child(xml, b"dataValidations", FOLLOWING, |list| {
        let (total, _) = slots(list)?;
        let child = crate::xml_edit_ns::new_child(list, b"dataValidation")?;
        let rule_xml = child.finish(&rule_xml(&child.text, rule)?)?;
        let changed = xml_edit::append_child(list, &rule_xml)?;
        xml_edit::root_attributes(&changed, &[("count", Some((total + 1).to_string()))])
    })
}

/// The reader index at which a newly appended standard rule will appear.
pub fn data_validation_insertion_index(xml: &str) -> Result<usize> {
    let records = jade_xlsx_reader::data_validations::validation_records(xml)?;
    let first = crate::xml_edit_ns::children(xml, b"dataValidations")?
        .first()
        .map(|(start, _, _)| *start);
    Ok(records
        .iter()
        .filter(|r| !r.extended && Some(r.container.start) == first)
        .count())
}

pub fn delete_data_validation(xml: &str, index: usize) -> Result<String> {
    edit_record(xml, index, None)
}

fn edit_record(xml: &str, index: usize, rule: Option<&DataValidation>) -> Result<String> {
    let record = jade_xlsx_reader::data_validations::validation_records(xml)?
        .into_iter()
        .nth(index)
        .ok_or_else(|| bad("rule index is outside the worksheet"))?;
    let list = crate::xml_edit_ns::fragment(xml, record.container.clone())?;
    let entries = crate::xml_edit_ns::children(&list.text, b"dataValidation")?;
    let total = entries.len();
    let (start, end, context) = entries
        .into_iter()
        .nth(record.index)
        .ok_or_else(|| bad("rule index is outside the container"))?;
    let replacement = match rule {
        Some(rule) if record.extended => extended_rule_xml(&context.text, rule)?,
        Some(rule) => rule_xml(&context.text, rule)?,
        None => String::new(),
    };
    let replacement = if replacement.is_empty() {
        replacement
    } else {
        context.finish(&replacement)?
    };
    let changed = apply(
        &list.text,
        vec![Splice {
            start,
            end,
            text: replacement,
        }],
    );
    let changed = xml_edit::root_attributes(
        &changed,
        &[(
            "count",
            Some((total - usize::from(rule.is_none())).to_string()),
        )],
    )?;
    let changed = if rule.is_none() && total == 1 {
        String::new()
    } else {
        list.finish(&changed)?
    };
    Ok(apply(
        xml,
        vec![Splice {
            start: record.container.start,
            end: record.container.end,
            text: changed,
        }],
    ))
}

fn slots(xml: &str) -> Result<(usize, String)> {
    let total = crate::xml_edit_ns::children(xml, b"dataValidation")?.len();
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                return Ok((total, namespace_prefix(e.name().as_ref())))
            }
            Event::Eof => return Err(WriteError::NoRoot("dataValidations")),
            _ => {}
        }
    }
}

fn rule_attributes(existing: &str, rule: &DataValidation, extended: bool) -> Result<String> {
    let string = |s: &Option<String>| {
        s.as_ref().map(|s| {
            encode_xstring(s)
                .replace('\n', "_x000A_")
                .replace('\t', "_x0009_")
        })
    };
    let ranges = rule
        .ranges
        .iter()
        .map(|r| crate::shift::range_text(*r))
        .collect::<Vec<_>>()
        .join(" ");
    xml_edit::root_attributes(
        existing,
        &[
            ("sqref", if extended { None } else { Some(ranges) }),
            ("type", rule.validation_type.clone()),
            ("operator", rule.operator.clone()),
            ("allowBlank", Some(u8::from(rule.allow_blank).to_string())),
            (
                "showInputMessage",
                Some(u8::from(rule.show_input_message).to_string()),
            ),
            (
                "showErrorMessage",
                Some(u8::from(rule.show_error_message).to_string()),
            ),
            (
                "showDropDown",
                Some(
                    u8::from(if extended {
                        !rule.suppress_dropdown
                    } else {
                        rule.suppress_dropdown
                    })
                    .to_string(),
                ),
            ),
            ("errorStyle", rule.error_style.clone()),
            ("imeMode", rule.ime_mode.clone()),
            ("promptTitle", string(&rule.prompt_title)),
            ("prompt", string(&rule.prompt)),
            ("errorTitle", string(&rule.error_title)),
            ("error", string(&rule.error)),
        ],
    )
}
fn rule_xml(existing: &str, rule: &DataValidation) -> Result<String> {
    let xml = rule_attributes(existing, rule, false)?;
    let source = crate::validation_sources::rewrite(&xml, rule, false)?;
    let xml = source.xml;
    let xml = if source.first {
        xml
    } else {
        set_child(
            &xml,
            b"formula1",
            |p| formula_xml(p, "formula1", rule.formula1.as_deref()),
            &[b"formula2", b"extLst"],
        )?
    };
    if source.second {
        return Ok(xml);
    }
    set_child(
        &xml,
        b"formula2",
        |p| formula_xml(p, "formula2", rule.formula2.as_deref()),
        &[b"extLst"],
    )
}
fn extended_rule_xml(existing: &str, rule: &DataValidation) -> Result<String> {
    use jade_xlsx_reader::data_validations::XM_NAMESPACE;
    let xml = rule_attributes(existing, rule, true)?;
    let source = crate::validation_sources::rewrite(&xml, rule, true)?;
    let mut xml = source.xml;
    for (name, value, following) in [
        (
            b"formula1".as_slice(),
            rule.formula1.as_deref(),
            vec![b"formula2".as_slice(), b"sqref", b"extLst"],
        ),
        (
            b"formula2".as_slice(),
            rule.formula2.as_deref(),
            vec![b"sqref".as_slice(), b"extLst"],
        ),
    ] {
        if (name == b"formula1" && source.first) || (name == b"formula2" && source.second) {
            continue;
        }
        if let Some(value) = value {
            xml = crate::xml_edit_ns::edit_child(&xml, name, &following, |wrapper| {
                crate::validation_sources::set_formula_value(wrapper, value)
            })?;
            let wrappers = crate::xml_edit_ns::children(&xml, name)?;
            let coverage = crate::xml_edit_ns::children_ns(&xml, b"sqref", XM_NAMESPACE)?;
            if let (Some((start, end, _)), Some((at, _, _))) = (wrappers.first(), coverage.first())
            {
                if start > at {
                    xml = apply(
                        &xml,
                        vec![
                            Splice {
                                start: *at,
                                end: *at,
                                text: xml[*start..*end].to_owned(),
                            },
                            Splice {
                                start: *start,
                                end: *end,
                                text: String::new(),
                            },
                        ],
                    );
                }
            }
        } else {
            xml = set_child(&xml, name, |_| String::new(), &following)?;
        }
    }
    let ranges = rule
        .ranges
        .iter()
        .copied()
        .map(crate::shift::range_text)
        .collect::<Vec<_>>()
        .join(" ");
    let leaves = crate::xml_edit_ns::children_ns(&xml, b"sqref", XM_NAMESPACE)?;
    if let Some((start, end, leaf)) = leaves.first() {
        let changed = crate::cell_shift::text_content(&leaf.text, &ranges)?;
        xml = apply(
            &xml,
            vec![Splice {
                start: *start,
                end: *end,
                text: leaf.finish(&changed)?,
            }],
        );
    } else {
        xml = xml_edit::append_child(
            &xml,
            &format!(
                "<sqref xmlns=\"{}\">{}</sqref>",
                String::from_utf8_lossy(XM_NAMESPACE),
                ranges
            ),
        )?;
    }
    Ok(xml)
}

fn formula_xml(prefix: &str, name: &str, formula: Option<&str>) -> String {
    formula
        .map(|f| {
            format!(
                "<{prefix}{name}>{}</{prefix}{name}>",
                escape(f.strip_prefix('=').unwrap_or(f))
            )
        })
        .unwrap_or_default()
}
fn bad(message: &str) -> WriteError {
    WriteError::DataValidation(message.into())
}

pub fn validate(rule: &DataValidation) -> Result<()> {
    if rule.ranges.is_empty() || rule.ranges.iter().any(|r| !valid_range(*r)) {
        return Err(bad("a rule needs valid worksheet ranges"));
    }
    let kind = rule.validation_type.as_deref().unwrap_or("none");
    if !matches!(
        kind,
        "none" | "whole" | "decimal" | "list" | "date" | "time" | "textLength" | "custom"
    ) {
        return Err(bad("unknown validation type"));
    }
    let operator = rule.operator.as_deref().unwrap_or("between");
    if !matches!(
        operator,
        "between"
            | "notBetween"
            | "equal"
            | "notEqual"
            | "lessThan"
            | "lessThanOrEqual"
            | "greaterThan"
            | "greaterThanOrEqual"
    ) {
        return Err(bad("unknown comparison operator"));
    }
    if rule
        .error_style
        .as_deref()
        .is_some_and(|s| !matches!(s, "stop" | "warning" | "information"))
    {
        return Err(bad("unknown error-alert style"));
    }
    if rule.ime_mode.as_deref().is_some_and(|s| {
        !matches!(
            s,
            "noControl"
                | "off"
                | "on"
                | "disabled"
                | "hiragana"
                | "fullKatakana"
                | "halfKatakana"
                | "fullAlpha"
                | "halfAlpha"
                | "fullHangul"
                | "halfHangul"
        )
    }) {
        return Err(bad("unknown IME mode"));
    }
    if let Some(list) = &rule.literal_list {
        if kind != "list" || rule.formula1.is_some() || rule.formula2.is_some() {
            return Err(bad(
                "a literal candidate list must be the sole source of a list rule",
            ));
        }
        if jade_xlsx_reader::data_validations::parse_literal_list(list).is_none() {
            return Err(bad("the candidate list has invalid CSV quoting"));
        }
    }
    let has_formula = |f: &Option<String>| {
        f.as_ref()
            .is_some_and(|f| !f.trim_start_matches('=').trim().is_empty())
    };
    if kind != "none" && rule.literal_list.is_none() && !has_formula(&rule.formula1) {
        return Err(bad("the rule needs a source or first formula"));
    }
    if !matches!(kind, "none" | "list" | "custom")
        && matches!(operator, "between" | "notBetween")
        && !has_formula(&rule.formula2)
    {
        return Err(bad("a between rule needs both bounds"));
    }
    for formula in [&rule.formula1, &rule.formula2].into_iter().flatten() {
        if formula
            .chars()
            .any(|c| !crate::patch::is_xml_10_character(c))
        {
            return Err(bad("the formula contains an invalid XML character"));
        }
    }
    Ok(())
}
pub(crate) fn valid_range(range: CellRange) -> bool {
    range.start.column <= range.end.column
        && range.start.row <= range.end.row
        && range.end.column < jade_xlsx_reader::model::MAX_COLUMN_COUNT
        && range.end.row < jade_xlsx_reader::model::MAX_ROW_COUNT
}

/// Clears all standard/x14 rules, or only the portion covering the selected cells.
/// When a remaining range has a new anchor, relative formulas follow it.
pub fn clear_data_validations(xml: &str, cut: Option<CellRange>) -> Result<String> {
    if cut.is_some_and(|r| !valid_range(r)) {
        return Err(bad("invalid selection"));
    }
    let rules = jade_xlsx_reader::data_validations::parse_data_validations(xml)?;
    let mut updated = xml.to_string();
    for (index, rule) in rules.into_iter().enumerate().rev() {
        let remaining = match cut {
            None => Vec::new(),
            Some(cut) => rule
                .ranges
                .iter()
                .flat_map(|r| crate::conditional_formats::subtract(*r, cut))
                .collect(),
        };
        if remaining == rule.ranges {
            continue;
        }
        if remaining.is_empty() {
            updated = delete_data_validation(&updated, index)?;
        } else {
            let mut new = rule.clone();
            new.ranges = remaining;
            let origin = rule.ranges[0].start;
            let target = new.ranges[0].start;
            new.formula1 = rule
                .formula1
                .as_deref()
                .map(|f| jade_xlsx_reader::translate_formula(f, origin, target));
            new.formula2 = rule
                .formula2
                .as_deref()
                .map(|f| jade_xlsx_reader::translate_formula(f, origin, target));
            // Coverage-only clearing also applies to imported rules whose
            // unsupported type/formula cannot be authored through the dialog.
            updated = edit_record(&updated, index, Some(&new))?;
        }
    }
    Ok(updated)
}
