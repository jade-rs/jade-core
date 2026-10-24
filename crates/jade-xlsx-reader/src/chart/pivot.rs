use super::Node;
use serde::{Deserialize, Serialize};
/// The standard chartSpace/pivotSource binding. The name is XML text, not an
/// ST_Xstring value; literal `_xHHHH_` sequences must keep their identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PivotSource {
    pub name: String,
    pub format_id: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotSourceName {
    pub workbook: Option<String>,
    pub sheet: String,
    pub pivot: String,
}
fn unquote(s: &str) -> Option<String> {
    if s.starts_with('\'') {
        let inner = s.strip_prefix('\'')?.strip_suffix('\'')?;
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\'' && chars.next() != Some('\'') {
                return None;
            }
            out.push(c);
        }
        Some(out)
    } else {
        Some(s.to_string())
    }
}
impl PivotSource {
    /// Office's standard [filename]sheet!pivottable spelling. An unqualified
    /// sheet!pivot is also retained for local producers. No file is resolved.
    pub fn qualified_name(&self) -> Option<PivotSourceName> {
        let (owner, pivot) = self.name.rsplit_once('!')?;
        if pivot.is_empty() {
            return None;
        }
        let owner = unquote(owner)?;
        let (workbook, sheet) = if let Some(rest) = owner.strip_prefix('[') {
            let (book, sheet) = rest.split_once(']')?;
            if book.is_empty() {
                return None;
            }
            (Some(book.to_string()), unquote(sheet)?)
        } else {
            (None, owner)
        };
        if sheet.is_empty() || sheet.contains(['[', ']']) {
            return None;
        }
        Some(PivotSourceName {
            workbook,
            sheet,
            pivot: pivot.to_string(),
        })
    }
}
pub(super) fn parse(space: &Node, issues: &mut Vec<String>) -> Option<PivotSource> {
    let nodes = space
        .children
        .iter()
        .filter(|n| n.main && n.name == "pivotSource")
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        return None;
    }
    let read = || {
        if nodes.len() != 1 {
            return None;
        }
        let n = nodes[0];
        let children = |name: &str| {
            n.children
                .iter()
                .filter(|c| c.main && c.name == name)
                .collect::<Vec<_>>()
        };
        let names = children("name");
        let formats = children("fmtId");
        if names.len() != 1 || formats.len() != 1 || !names[0].children.is_empty() {
            return None;
        }
        let name = names[0].text.clone();
        if name.is_empty() {
            return None;
        }
        let format_id = formats[0].attrs.get("val")?.trim().parse::<u32>().ok()?;
        Some(PivotSource { name, format_id })
    };
    let value = read();
    if value.is_none() {
        issues.push("invalid pivot chart source".into());
    } else {
        issues.push("pivot chart controls".into());
    }
    value
}
