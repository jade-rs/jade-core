//! Pivot metadata and typed cache records. The original package remains the
//! authority for saving; these views never serialize or fetch external sources.
mod read;
pub(crate) mod xml;
use crate::{CellRange, CellRef};
pub use read::{parse_cache_definition, parse_cache_records, parse_pivot_table, read_catalog};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PivotCatalog {
    pub caches: Vec<Arc<PivotCache>>,
    pub tables: Vec<Arc<PivotTable>>,
    pub issues: Vec<PivotIssue>,
}
impl PivotCatalog {
    pub fn is_empty(&self) -> bool {
        self.caches.is_empty() && self.tables.is_empty() && self.issues.is_empty()
    }
}
/// Bound total interpreted cache values across the workbook. Original parts
/// remain preserved when a view exceeds this limit.
pub const MAX_CATALOG_VALUES: usize = 1_000_000;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PivotIssue {
    pub part_name: String,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PivotCache {
    pub id: u32,
    pub part_name: String,
    pub records_part: Option<String>,
    pub source: CacheSource,
    pub fields: Vec<CacheField>,
    pub records: Option<PivotRecords>,
    pub attributes: BTreeMap<String, String>,
    /// Features outside the interpreted view, including OLAP and calculated
    /// member metadata. Their XML is retained by preserving workbook writes.
    pub unsupported: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CacheSource {
    pub kind: String,
    pub connection_id: Option<u32>,
    pub sheet: Option<String>,
    pub reference: Option<CellRange>,
    pub defined_name: Option<String>,
    pub relationship_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheField {
    pub name: String,
    pub shared_items: Vec<PivotValue>,
    pub grouping: Option<FieldGrouping>,
    pub attributes: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldGrouping {
    pub base: Option<u32>,
    pub parent: Option<u32>,
    pub range: Option<BTreeMap<String, String>>,
    pub discrete: Vec<u32>,
    pub items: Vec<PivotValue>,
}
/// Keep shared and record indices intact, including unavailable/unknown values.
/// Dates retain their exact ISO spelling; interpretation uses workbook epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PivotValue {
    pub value: PivotScalar,
    pub unused: bool,
    pub caption: Option<String>,
    pub attributes: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PivotScalar {
    Missing,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    Date(String),
    Shared(u32),
    Unknown {
        element: String,
        value: Option<String>,
    },
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PivotRecords {
    pub rows: Vec<Vec<PivotValue>>,
    pub declared_count: Option<u32>,
    pub unsupported: Vec<String>,
}
impl PivotCache {
    pub fn record_value(&self, row: usize, field: usize) -> Option<&PivotScalar> {
        let value = &self.records.as_ref()?.rows.get(row)?.get(field)?.value;
        match value {
            PivotScalar::Shared(index) => Some(
                &self
                    .fields
                    .get(field)?
                    .shared_items
                    .get(*index as usize)?
                    .value,
            ),
            v => Some(v),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PivotTable {
    pub part_name: String,
    pub sheet_part: String,
    pub name: String,
    pub cache_id: u32,
    pub location: PivotLocation,
    pub fields: Vec<PivotField>,
    /// -2 denotes the values pseudo-field; nonnegative indices address cache fields.
    pub row_fields: Vec<i32>,
    pub column_fields: Vec<i32>,
    pub row_items: Vec<AxisItem>,
    pub column_items: Vec<AxisItem>,
    pub page_fields: Vec<PageField>,
    pub data_fields: Vec<DataField>,
    pub style: Option<BTreeMap<String, String>>,
    pub filters: Vec<PivotFilter>,
    pub attributes: BTreeMap<String, String>,
    pub unsupported: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PivotLocation {
    pub reference: CellRange,
    pub first_header_row: u32,
    pub first_data_row: u32,
    pub first_data_column: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_page_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_page_count: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PivotField {
    pub axis: Option<String>,
    pub caption: Option<String>,
    pub items: Vec<PivotItem>,
    pub attributes: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PivotItem {
    pub shared_index: Option<u32>,
    pub kind: String,
    pub hidden: bool,
    pub caption: Option<String>,
    pub attributes: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisItem {
    pub kind: String,
    pub repeat: u32,
    pub data_index: u32,
    pub items: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageField {
    pub field: u32,
    pub item: Option<u32>,
    pub hierarchy: Option<i32>,
    pub caption: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataField {
    pub field: u32,
    pub name: Option<String>,
    pub aggregation: String,
    pub show_as: String,
    pub base_field: Option<u32>,
    pub base_item: Option<u32>,
    pub number_format_id: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PivotFilter {
    pub attributes: BTreeMap<String, String>,
    pub conditions: Vec<FilterCondition>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterCondition {
    pub element: String,
    pub attributes: BTreeMap<String, String>,
}

impl PivotTable {
    /// Label/value cells for report filters above the saved report location.
    /// Returns None when saved page dimensions or coordinates are ambiguous.
    pub fn page_cells(&self) -> Option<Vec<(CellRef, CellRef)>> {
        let count = u32::try_from(self.page_fields.len()).ok()?;
        if count == 0 {
            return Some(Vec::new());
        }
        if count > 4096 {
            return None;
        }
        let wrap = self
            .attributes
            .get("pageWrap")
            .map(|v| v.trim().parse::<u32>())
            .transpose()
            .ok()?
            .filter(|n| *n > 0)
            .unwrap_or(count)
            .min(count);
        let over = match self.attributes.get("pageOverThenDown").map(|v| v.trim()) {
            None | Some("0" | "false") => false,
            Some("1" | "true") => true,
            _ => return None,
        };
        let (rows, columns) = if over {
            (count.div_ceil(wrap), wrap)
        } else {
            (wrap, count.div_ceil(wrap))
        };
        if self
            .location
            .row_page_count
            .is_some_and(|n| n != 0 && n != rows)
            || self
                .location
                .column_page_count
                .is_some_and(|n| n != 0 && n != columns)
        {
            return None;
        }
        let top = self
            .location
            .reference
            .start
            .row
            .checked_sub(rows.checked_add(1)?)?;
        (0..count)
            .map(|i| {
                let (r, c) = if over {
                    (i / wrap, i % wrap)
                } else {
                    (i % wrap, i / wrap)
                };
                let column = self
                    .location
                    .reference
                    .start
                    .column
                    .checked_add(c.checked_mul(3)?)?;
                let row = top.checked_add(r)?;
                let value = column.checked_add(1)?;
                (value < crate::model::MAX_COLUMN_COUNT)
                    .then_some((CellRef::new(column, row), CellRef::new(value, row)))
            })
            .collect()
    }
}
