use super::*;

pub(super) fn parameters(node: &FilterCriterion) -> Result<(bool, bool, f64, f64)> {
    if !node.children.is_empty() {
        return Err(error("rank filters cannot contain children"));
    }
    let top = !node.attributes.contains_key("top") || bool_attribute(node, "top")?;
    let percent = bool_attribute(node, "percent")?;
    let value = node
        .attributes
        .get("val")
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && (1.0..=500.0).contains(value))
        .ok_or_else(|| error("rank filter value must be between 1 and 500"))?;
    let threshold = match node.attributes.get("filterVal") {
        None => 0.0,
        Some(value) => value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| error("rank filter threshold is invalid"))?,
    };
    Ok((top, percent, value, threshold))
}

/// Ranking samples include all numeric cells in the owner's data column,
/// independently of saved/manual visibility and the other column predicates.
pub(super) fn threshold(
    sheet: &Sheet,
    column: u32,
    first: u32,
    end: u32,
    node: &FilterCriterion,
) -> Result<f64> {
    let (top, percent, value, _) = parameters(node)?;
    if value.fract() != 0.0 {
        return Err(error("fractional rank values cannot yet be reapplied"));
    }
    let mut numbers = vec![];
    let start = sheet.rows.partition_point(|row| row.index < first);
    for row in sheet.rows[start..]
        .iter()
        .take_while(|row| row.index <= end)
    {
        if let Some(cell) = find_cell(row, column) {
            if let CellValue::Number(value) = cell.value {
                if !value.is_finite() {
                    return Err(error("non-finite numeric cells cannot be ranked"));
                }
                numbers.push(value);
            }
        }
    }
    if numbers.is_empty() {
        return Ok(0.0);
    }
    numbers.sort_by(f64::total_cmp);
    let count = if percent {
        (numbers.len() as f64 * value / 100.0).ceil() as usize
    } else {
        value as usize
    };
    let count = count.clamp(1, numbers.len());
    Ok(numbers[if top {
        numbers.len() - count
    } else {
        count - 1
    }])
}
