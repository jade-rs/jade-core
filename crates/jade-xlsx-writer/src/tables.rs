//! Excel table authoring on the original OPC package. Definitions and headers
//! are changed together; existing column/extension metadata is kept by ID.
use crate::{
    patch::{apply, escape, namespace_prefix, Splice},
    xml_edit, xml_edit_ns, CellEdit, NewValue, Result, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::table_reference::column_selectors as selector_columns;
use jade_xlsx_reader::{CellRange, CellRef, CellValue, Table, TableColumn, Workbook};
use quick_xml::{events::Event, name::ResolveResult, NsReader, Reader};
use std::collections::{BTreeMap, HashSet};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT_MAIN: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const STRICT_REL: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const TABLE_TYPE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableStyle {
    pub name: Option<String>,
    pub first_column: bool,
    pub last_column: bool,
    pub row_stripes: bool,
    pub column_stripes: bool,
}
impl Default for TableStyle {
    fn default() -> Self {
        Self {
            name: Some("TableStyleMedium2".into()),
            first_column: false,
            last_column: false,
            row_stripes: true,
            column_stripes: false,
        }
    }
}
impl TableStyle {
    pub fn from_table(t: &Table) -> Self {
        Self {
            name: t.style_name.clone(),
            first_column: t.show_first_column,
            last_column: t.show_last_column,
            row_stripes: t.show_row_stripes,
            column_stripes: t.show_column_stripes,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableOptions {
    pub name: Option<String>,
    pub has_headers: bool,
    pub style: TableStyle,
}
impl Default for TableOptions {
    fn default() -> Self {
        Self {
            name: None,
            has_headers: true,
            style: TableStyle::default(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotalFunction {
    Average,
    Count,
    CountNumbers,
    Max,
    Min,
    Sum,
    StandardDeviation,
    Variance,
}
impl TotalFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Average => "average",
            Self::Count => "count",
            Self::CountNumbers => "countNums",
            Self::Max => "max",
            Self::Min => "min",
            Self::Sum => "sum",
            Self::StandardDeviation => "stdDev",
            Self::Variance => "var",
        }
    }
    pub fn code(self) -> u32 {
        match self {
            Self::Average => 101,
            Self::Count => 103,
            Self::CountNumbers => 102,
            Self::Max => 104,
            Self::Min => 105,
            Self::Sum => 109,
            Self::StandardDeviation => 107,
            Self::Variance => 110,
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        [
            Self::Average,
            Self::Count,
            Self::CountNumbers,
            Self::Max,
            Self::Min,
            Self::Sum,
            Self::StandardDeviation,
            Self::Variance,
        ]
        .into_iter()
        .find(|f| f.name() == name)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableTotal {
    None,
    Function(TotalFunction),
    Label(String),
    Formula(String),
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableUpdate {
    pub name: Option<String>,
    pub reference: Option<CellRange>,
    pub style: Option<TableStyle>,
    pub header_row: Option<bool>,
    pub totals_row: Option<bool>,
    pub filter_buttons: Option<bool>,
    /// Zero-based column index and replacement. Formula None removes only
    /// calculated-column metadata; it leaves existing cell formulas intact.
    pub calculated_columns: Vec<(usize, Option<String>)>,
    pub totals: Vec<(usize, TableTotal)>,
}
#[derive(Debug, Clone)]
pub struct TableResult {
    pub bytes: Vec<u8>,
    pub name: String,
    pub reference: CellRange,
}
fn bad(message: &str) -> WriteError {
    WriteError::Table(message.into())
}
pub(crate) fn attr(xml: &str, key: &[u8]) -> Result<Option<String>> {
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                for a in e.attributes() {
                    let a = a?;
                    if a.key.as_ref() == key {
                        return Ok(Some(a.unescape_value()?.into_owned()));
                    }
                }
                return Ok(None);
            }
            Event::Eof => return Err(WriteError::NoRoot("table")),
            _ => {}
        }
    }
}
fn prefix(xml: &str) -> Result<String> {
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => return Ok(namespace_prefix(e.name().as_ref())),
            Event::Eof => return Err(WriteError::NoRoot("table")),
            _ => {}
        }
    }
}
fn main_namespace(xml: &str) -> Result<String> {
    let mut r = NsReader::from_str(xml);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                return Ok(match r.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Bound(n) if n.as_ref() == STRICT_MAIN.as_bytes() => STRICT_MAIN,
                    _ => MAIN,
                }
                .into())
            }
            Event::Eof => return Err(WriteError::NoRoot("worksheet")),
            _ => {}
        }
    }
}
pub(crate) fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.row <= b.end.row
        && b.start.row <= a.end.row
        && a.start.column <= b.end.column
        && b.start.column <= a.end.column
}
pub(crate) fn validate_range(r: CellRange) -> Result<()> {
    if r.start.column > r.end.column
        || r.start.row > r.end.row
        || r.end.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
        || r.end.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
    {
        return Err(bad("invalid table range"));
    }
    Ok(())
}
pub fn validate_table_name(book: &Workbook, name: &str, own: Option<&str>) -> Result<()> {
    crate::defined_names::validate_name(name).map_err(|_| bad("invalid table name"))?;
    let same = |s: &str| s.to_lowercase() == name.to_lowercase();
    if book.defined_names.iter().any(|n| same(&n.name))
        || book
            .sheets
            .iter()
            .flat_map(|s| &s.tables)
            .any(|t| own.is_none_or(|o| o != t.name) && (same(&t.name) || same(&t.display_name)))
    {
        return Err(bad("the table name is already in use"));
    }
    Ok(())
}
fn validate_style(book: &Workbook, style: &TableStyle) -> Result<()> {
    if let Some(name) = &style.name {
        let builtin = [
            ("TableStyleLight", 21),
            ("TableStyleMedium", 28),
            ("TableStyleDark", 11),
        ]
        .iter()
        .any(|(p, max)| {
            name.strip_prefix(p)
                .and_then(|s| s.parse::<u32>().ok())
                .is_some_and(|n| n >= 1 && n <= *max)
        });
        if !builtin && !book.styles.table_styles.iter().any(|s| s.name == *name) {
            return Err(bad("unknown table style"));
        }
    }
    Ok(())
}
fn set_style(t: &mut Table, s: &TableStyle) {
    t.style_name = s.name.clone();
    t.show_first_column = s.first_column;
    t.show_last_column = s.last_column;
    t.show_row_stripes = s.row_stripes;
    t.show_column_stripes = s.column_stripes;
}
fn check_area(sheet: &jade_xlsx_reader::Sheet, range: CellRange, own: Option<&str>) -> Result<()> {
    validate_range(range)?;
    if sheet
        .tables
        .iter()
        .any(|t| own.is_none_or(|name| name != t.name) && overlaps(t.reference, range))
    {
        return Err(bad("tables cannot overlap"));
    }
    if sheet.merges.iter().any(|r| overlaps(*r, range)) {
        return Err(bad("tables cannot contain merged cells"));
    }
    if sheet
        .array_formulas
        .iter()
        .any(|a| overlaps(a.range, range))
    {
        return Err(bad("tables cannot contain array or spill formulas"));
    }
    Ok(())
}
fn cell(sheet: &jade_xlsx_reader::Sheet, at: CellRef) -> Option<&jade_xlsx_reader::Cell> {
    sheet
        .rows
        .binary_search_by_key(&at.row, |r| r.index)
        .ok()
        .and_then(|i| {
            let r = &sheet.rows[i];
            r.cells
                .binary_search_by_key(&at.column, |c| c.reference.column)
                .ok()
                .map(|i| &r.cells[i])
        })
}
fn blank_row(sheet: &jade_xlsx_reader::Sheet, row: u32, first: u32, last: u32) -> bool {
    !sheet
        .rows
        .iter()
        .filter(|r| r.index == row)
        .flat_map(|r| &r.cells)
        .any(|c| {
            (first..=last).contains(&c.reference.column)
                && (c.formula.is_some() || c.value != CellValue::Empty)
        })
}
fn heading(book: &Workbook, sheet: &jade_xlsx_reader::Sheet, at: CellRef) -> String {
    let Some(c) = cell(sheet, at) else {
        return String::new();
    };
    let format = book
        .styles
        .cell_format(sheet.effective_style_index(at, c.style_index))
        .and_then(|f| book.styles.number_format_code(f));
    jade_xlsx_layout::number_format::format_value(&c.value, format, book.date_system).text
}
fn names_unique(wanted: Vec<String>) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for (i, s) in wanted.into_iter().enumerate() {
        let base = if s.trim().is_empty() {
            format!("Column{}", i + 1)
        } else {
            s
        };
        if base.encode_utf16().count() > 255 {
            return Err(bad("a table header exceeds 255 characters"));
        }
        let mut name = base.clone();
        let mut n = 2;
        while out.iter().any(|s| s.to_lowercase() == name.to_lowercase()) {
            let suffix = n.to_string();
            let max = 255 - suffix.encode_utf16().count();
            let mut base_short = String::new();
            for c in base.chars() {
                if base_short.encode_utf16().count() + c.len_utf16() > max {
                    break;
                }
                base_short.push(c);
            }
            name = format!("{base_short}{n}");
            n += 1;
        }
        out.push(name);
    }
    Ok(out)
}
fn text(s: &str) -> String {
    jade_xlsx_reader::xstring::encode_xstring(s)
        .replace('\n', "_x000A_")
        .replace('\t', "_x0009_")
}
fn formula_valid(f: &str) -> Result<String> {
    let f = f.strip_prefix('=').unwrap_or(f);
    if f.trim().is_empty() || f.chars().any(|c| !crate::patch::is_xml_10_character(c)) {
        return Err(bad("invalid table formula"));
    }
    Ok(f.into())
}
fn edit_formula(
    xml: &str,
    kind: &[u8],
    value: Option<&str>,
    following: &[&[u8]],
) -> Result<String> {
    if let Some(value) = value {
        let value = formula_valid(value)?;
        xml_edit_ns::edit_child(xml, kind, following, |node| {
            let mut r = Reader::from_str(node);
            loop {
                let start = r.buffer_position() as usize;
                match r.read_event()? {
                    Event::Start(e) => {
                        let head = r.buffer_position() as usize;
                        let name = e.name().as_ref().to_vec();
                        r.read_to_end(quick_xml::name::QName(&name))?;
                        let end = r.buffer_position() as usize;
                        let close = end - String::from_utf8_lossy(&name).len() - 3;
                        return Ok(apply(
                            node,
                            vec![Splice {
                                start: head,
                                end: close,
                                text: escape(&value),
                            }],
                        ));
                    }
                    Event::Empty(e) => {
                        let end = r.buffer_position() as usize;
                        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        return Ok(apply(
                            node,
                            vec![Splice {
                                start,
                                end,
                                text: format!(
                                    "{}>{}</{name}>",
                                    &node[start..end - 2],
                                    escape(&value)
                                ),
                            }],
                        ));
                    }
                    Event::Eof => return Err(WriteError::NoRoot("formula")),
                    _ => {}
                }
            }
        })
    } else {
        xml_edit_ns::set_child(xml, kind, |_| String::new(), following)
    }
}
/// Change only affected calculated formulas, retaining their XML metadata.
pub(crate) fn rebase_calculated_formulas(
    xml: &str,
    formulas: &BTreeMap<u32, String>,
) -> Result<String> {
    let mut changes = Vec::new();
    for (start, end, container) in xml_edit_ns::children(xml, b"tableColumns")? {
        let mut columns = Vec::new();
        for (a, b, column) in xml_edit_ns::children(&container.text, b"tableColumn")? {
            let Some(formula) = attr(&column.text, b"id")?
                .and_then(|s| s.parse::<u32>().ok())
                .and_then(|id| formulas.get(&id))
            else {
                continue;
            };
            let updated = edit_formula(
                &column.text,
                b"calculatedColumnFormula",
                Some(formula),
                &[b"totalsRowFormula", b"xmlColumnPr", b"extLst"],
            )?;
            if updated != column.text {
                columns.push(Splice {
                    start: a,
                    end: b,
                    text: column.finish(&updated)?,
                });
            }
        }
        if !columns.is_empty() {
            changes.push(Splice {
                start,
                end,
                text: container.finish(&apply(&container.text, columns))?,
            });
        }
    }
    Ok(apply(xml, changes))
}

/// Update a modeled table while retaining unknown root, column and extension XML.
pub fn edit_definition(xml: &str, table: &Table) -> Result<String> {
    let mut updated = xml_edit::root_attributes(
        xml,
        &[
            ("name", Some(table.name.clone())),
            ("displayName", Some(table.display_name.clone())),
            ("ref", Some(crate::shift::range_text(table.reference))),
            ("headerRowCount", Some(table.header_row_count.to_string())),
            ("totalsRowCount", Some(table.totals_row_count.to_string())),
            (
                "totalsRowShown",
                Some(u8::from(table.totals_row_count > 0).to_string()),
            ),
        ],
    )?;
    updated = xml_edit_ns::edit_child(
        &updated,
        b"tableColumns",
        &[b"tableStyleInfo", b"extLst"],
        |container| {
            let entries = xml_edit_ns::children(container, b"tableColumn")?;
            let mut by_id = BTreeMap::new();
            for (start, end, entry) in &entries {
                if let Some(id) = attr(&entry.text, b"id")?.and_then(|s| s.parse::<u32>().ok()) {
                    by_id.insert(id, (*start, *end, entry));
                }
            }
            let p = prefix(container)?;
            let mut replacement = String::new();
            for (i, name) in table.columns.iter().enumerate() {
                let properties = table
                    .column_properties
                    .get(i)
                    .cloned()
                    .unwrap_or(TableColumn {
                        id: i as u32 + 1,
                        ..Default::default()
                    });
                let existing = by_id.get(&properties.id).map(|(_, _, c)| *c);
                let base = existing
                    .map(|c| c.text.clone())
                    .unwrap_or_else(|| format!("<{p}tableColumn/>"));
                let mut column = xml_edit::root_attributes(
                    &base,
                    &[
                        ("id", Some(properties.id.to_string())),
                        ("name", Some(text(name))),
                        ("totalsRowFunction", properties.totals_function.clone()),
                        (
                            "totalsRowLabel",
                            properties.totals_label.as_deref().map(text),
                        ),
                    ],
                )?;
                column = edit_formula(
                    &column,
                    b"calculatedColumnFormula",
                    properties.calculated_formula.as_deref(),
                    &[b"totalsRowFormula", b"xmlColumnPr", b"extLst"],
                )?;
                column = edit_formula(
                    &column,
                    b"totalsRowFormula",
                    properties.totals_formula.as_deref(),
                    &[b"xmlColumnPr", b"extLst"],
                )?;
                replacement.push_str(&if let Some(existing) = existing {
                    existing.finish(&column)?
                } else {
                    column
                });
            }
            let mut splices: Vec<_> = entries
                .iter()
                .map(|(start, end, _)| Splice {
                    start: *start,
                    end: *end,
                    text: String::new(),
                })
                .collect();
            if let Some(first) = splices.first_mut() {
                first.text = replacement;
                let changed = apply(container, splices);
                xml_edit::root_attributes(
                    &changed,
                    &[("count", Some(table.columns.len().to_string()))],
                )
            } else {
                let changed = xml_edit::append_child(container, &replacement)?;
                xml_edit::root_attributes(
                    &changed,
                    &[("count", Some(table.columns.len().to_string()))],
                )
            }
        },
    )?;
    updated = xml_edit_ns::edit_child(&updated, b"tableStyleInfo", &[b"extLst"], |node| {
        xml_edit::root_attributes(
            node,
            &[
                ("name", table.style_name.clone()),
                (
                    "showFirstColumn",
                    Some(u8::from(table.show_first_column).to_string()),
                ),
                (
                    "showLastColumn",
                    Some(u8::from(table.show_last_column).to_string()),
                ),
                (
                    "showRowStripes",
                    Some(u8::from(table.show_row_stripes).to_string()),
                ),
                (
                    "showColumnStripes",
                    Some(u8::from(table.show_column_stripes).to_string()),
                ),
            ],
        )
    })?;
    if table.auto_filter && table.header_row_count > 0 {
        let mut range = table.reference;
        range.end.row -= table.totals_row_count;
        updated = xml_edit_ns::edit_child(
            &updated,
            b"autoFilter",
            &[b"sortState", b"tableColumns", b"tableStyleInfo", b"extLst"],
            |node| {
                xml_edit::root_attributes(node, &[("ref", Some(crate::shift::range_text(range)))])
            },
        )?;
    } else {
        updated = xml_edit_ns::set_child(&updated, b"autoFilter", |_| String::new(), &[])?;
    }
    Ok(updated)
}
fn write_cells(
    package: &mut OpcPackage,
    part: &str,
    edits: BTreeMap<CellRef, CellEdit>,
) -> Result<()> {
    if !edits.is_empty() {
        let xml = package.required_part_str(part)?;
        package.replace(
            part,
            crate::patch_sheet(&xml, &edits.into_values().collect::<Vec<_>>())?.into_bytes(),
        );
    }
    Ok(())
}
fn put(edits: &mut BTreeMap<CellRef, CellEdit>, at: CellRef, value: NewValue, style: Option<u32>) {
    edits.insert(
        at,
        CellEdit {
            reference: at,
            value,
            style_index: style,
        },
    );
}
fn ask_recalc(package: &mut OpcPackage) -> Result<()> {
    let part = crate::workbook_part_name(package)?;
    let xml = package.required_part_str(&part)?;
    package.replace(&part, crate::request_full_recalculation(&xml).into_bytes());
    Ok(())
}
fn add_content_type(package: &mut OpcPackage, part: &str) -> Result<()> {
    let xml = package.required_part_str("[Content_Types].xml")?;
    let p = prefix(&xml)?;
    let entry = format!(
        "<{p}Override PartName=\"/{}\" ContentType=\"{TABLE_TYPE}\"/>",
        escape(part)
    );
    package.replace(
        "[Content_Types].xml",
        xml_edit::append_child(&xml, &entry)?.into_bytes(),
    );
    Ok(())
}
fn add_table_part(
    package: &mut OpcPackage,
    sheet: &str,
    part: &str,
    namespace: &str,
) -> Result<()> {
    let rel_ns = if namespace == STRICT_MAIN {
        STRICT_REL
    } else {
        REL
    };
    let rels_part = jade_opc::rels_part_name(sheet);
    let rels=package.part_str(&rels_part)?.unwrap_or_else(||"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into());
    let parsed = jade_opc::parse_relationships(&rels)?;
    let id = (1u32..)
        .map(|i| format!("rId{i}"))
        .find(|id| parsed.relationship(id).is_none())
        .ok_or_else(|| bad("no available relationship ID"))?;
    let p = prefix(&rels)?;
    let entry = format!(
        "<{p}Relationship Id=\"{id}\" Type=\"{rel_ns}/table\" Target=\"{}\"/>",
        escape(&crate::notes::relative_target(sheet, part))
    );
    let updated = xml_edit::append_child(&rels, &entry)?;
    package.replace(&rels_part, updated.into_bytes());
    let xml = package.required_part_str(sheet)?;
    let updated = xml_edit_ns::edit_child(&xml, b"tableParts", &[b"extLst"], |node| {
        let p = prefix(node)?;
        let count = xml_edit_ns::children(node, b"tablePart")?.len() + 1;
        let entry = format!("<{p}tablePart xmlns:jadeTable=\"{rel_ns}\" jadeTable:id=\"{id}\"/>");
        let updated = xml_edit::append_child(node, &entry)?;
        xml_edit::root_attributes(&updated, &[("count", Some(count.to_string()))])
    })?;
    package.replace(sheet, updated.into_bytes());
    Ok(())
}
/// Create a table with synchronized headers, table part and sheet relationship.
pub fn create_table(
    original: &[u8],
    sheet_name: &str,
    reference: CellRange,
    options: &TableOptions,
) -> Result<TableResult> {
    let book = jade_xlsx_reader::read_xlsx(original)?;
    let sheet = book
        .sheets
        .iter()
        .find(|s| s.name == sheet_name)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    validate_style(&book, &options.style)?;
    check_area(sheet, reference, None)?;
    let name = if let Some(name) = &options.name {
        validate_table_name(&book, name, None)?;
        name.clone()
    } else {
        (1u32..)
            .map(|i| format!("Table{i}"))
            .find(|name| validate_table_name(&book, name, None).is_ok())
            .ok_or_else(|| bad("no available table name"))?
    };
    let mut range = reference;
    let mut edits = BTreeMap::new();
    if !options.has_headers {
        range.end.row = range
            .end
            .row
            .checked_add(1)
            .filter(|row| *row < jade_xlsx_reader::model::MAX_ROW_COUNT)
            .ok_or_else(|| bad("table exceeds worksheet bounds"))?;
        let header = CellRange {
            start: reference.start,
            end: CellRef::new(reference.end.column, reference.start.row),
        };
        let bytes = crate::cell_shift::shift_cells(
            original,
            &book,
            &jade_xlsx_reader::CellShift {
                sheet: sheet.name.clone(),
                range: header,
                axis: jade_xlsx_reader::Axis::Rows,
                inserted: true,
            },
        )?;
        let mut next = options.clone();
        next.has_headers = true;
        next.name = Some(name);
        return create_table(&bytes, sheet_name, range, &next);
    } else if range.start.row == range.end.row {
        range.end.row += 1;
        check_area(sheet, range, None)?;
        if !blank_row(sheet, range.end.row, range.start.column, range.end.column) {
            return Err(bad("a header-only selection needs an empty data row"));
        }
    }
    let wanted = (range.start.column..=range.end.column)
        .map(|col| {
            if options.has_headers {
                heading(&book, sheet, CellRef::new(col, range.start.row))
            } else {
                String::new()
            }
        })
        .collect();
    let columns = names_unique(wanted)?;
    for (i, name) in columns.iter().enumerate() {
        put(
            &mut edits,
            CellRef::new(range.start.column + i as u32, range.start.row),
            NewValue::Text(name.clone()),
            None,
        );
    }
    let mut package = OpcPackage::open(original)?;
    let workbook_part = crate::workbook_part_name(&package)?;
    let directory = workbook_part
        .rsplit_once('/')
        .map(|(d, _)| format!("{d}/tables/table"))
        .unwrap_or_else(|| "tables/table".into());
    let part = crate::notes::free_part_name(&package, &directory, ".xml");
    let mut used_ids = HashSet::new();
    for t in book.sheets.iter().flat_map(|s| &s.tables) {
        if let Some(id) = attr(&package.required_part_str(&t.part_name)?, b"id")?
            .and_then(|s| s.parse::<u32>().ok())
        {
            used_ids.insert(id);
        }
    }
    let id = (1u32..)
        .find(|id| !used_ids.contains(id))
        .ok_or_else(|| bad("no available table ID"))?;
    let namespace = main_namespace(&package.required_part_str(&sheet.part_name)?)?;
    let mut table = Table {
        part_name: part.clone(),
        name: name.clone(),
        display_name: name.clone(),
        reference: range,
        header_row_count: 1,
        totals_row_count: 0,
        column_properties: (0..columns.len())
            .map(|i| TableColumn {
                id: i as u32 + 1,
                ..Default::default()
            })
            .collect(),
        columns,
        style_name: None,
        show_first_column: false,
        show_last_column: false,
        show_row_stripes: false,
        show_column_stripes: false,
        auto_filter: true,
        filter: None,
    };
    set_style(&mut table, &options.style);
    let xml = edit_definition(
        &format!("<table xmlns=\"{namespace}\" id=\"{id}\"/>"),
        &table,
    )?;
    package.replace(&part, xml.into_bytes());
    write_cells(&mut package, &sheet.part_name, edits)?;
    add_table_part(&mut package, &sheet.part_name, &part, &namespace)?;
    add_content_type(&mut package, &part)?;
    ask_recalc(&mut package)?;
    Ok(TableResult {
        bytes: package.write()?,
        name,
        reference: range,
    })
}

fn escaped_column(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if matches!(c, '[' | ']' | '#' | '\'' | '@') {
            out.push('\'');
        }
        out.push(c);
    }
    out
}
fn totals_value(table: &Table, index: usize) -> Result<NewValue> {
    let c = &table.column_properties[index];
    if let Some(f) = &c.totals_formula {
        return Ok(NewValue::Formula(formula_valid(f)?));
    }
    if let Some(function) = c
        .totals_function
        .as_deref()
        .and_then(TotalFunction::from_name)
    {
        return Ok(NewValue::Formula(format!(
            "SUBTOTAL({},{}[{}])",
            function.code(),
            table.name,
            escaped_column(&table.columns[index])
        )));
    }
    if let Some(label) = &c.totals_label {
        return Ok(NewValue::Text(label.clone()));
    }
    Ok(NewValue::Blank)
}
fn clear_row(edits: &mut BTreeMap<CellRef, CellEdit>, row: u32, first: u32, last: u32) {
    for column in first..=last {
        put(edits, CellRef::new(column, row), NewValue::Blank, None);
    }
}
fn remap_filter(xml: &str, old: &Table, new: &Table) -> Result<String> {
    let Some((_, _, container)) = xml_edit_ns::children(xml, b"autoFilter")?
        .into_iter()
        .next()
    else {
        return Ok(xml.into());
    };
    let mut splices = Vec::new();
    for (start, end, entry) in xml_edit_ns::children(&container.text, b"filterColumn")? {
        if let Some(old_id) = attr(&entry.text, b"colId")?.and_then(|s| s.parse::<u32>().ok()) {
            let absolute = old.reference.start.column + old_id;
            let text =
                if absolute < new.reference.start.column || absolute > new.reference.end.column {
                    String::new()
                } else {
                    entry.finish(&xml_edit::root_attributes(
                        &entry.text,
                        &[(
                            "colId",
                            Some((absolute - new.reference.start.column).to_string()),
                        )],
                    )?)?
                };
            splices.push(Splice { start, end, text });
        }
    }
    let changed = container.finish(&apply(&container.text, splices))?;
    xml_edit_ns::set_child(xml, b"autoFilter", |_| changed, &[])
}
/// Edit size/name/style/headers/totals and column formulas as one package write.
/// A resized range contains the header and totals rows currently displayed;
/// header/totals toggles then add or remove their own row.
pub fn update_table(
    original: &[u8],
    sheet_name: &str,
    table_name: &str,
    update: &TableUpdate,
) -> Result<TableResult> {
    let book = jade_xlsx_reader::read_xlsx(original)?;
    let sheet = book
        .sheets
        .iter()
        .find(|s| s.name == sheet_name)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    let old = sheet
        .tables
        .iter()
        .find(|t| t.name.to_lowercase() == table_name.to_lowercase())
        .ok_or_else(|| bad("no such table"))?;
    let mut table = old.clone();
    let mut edits = BTreeMap::new();
    if let Some(name) = &update.name {
        validate_table_name(&book, name, Some(&old.name))?;
        table.name = name.clone();
        table.display_name = name.clone();
    }
    if let Some(style) = &update.style {
        validate_style(&book, style)?;
        set_style(&mut table, style);
    }
    if let Some(range) = update.reference {
        validate_range(range)?;
        if range.start.row != old.reference.start.row
            || range.start.column > old.reference.end.column
            || range.end.column < old.reference.start.column
        {
            return Err(bad(
                "a resized table must keep its first row and overlap its original columns",
            ));
        }
        table.reference = range;
    }
    let mut properties = Vec::new();
    let mut wanted = Vec::new();
    let mut next_id = old
        .column_properties
        .iter()
        .map(|p| p.id)
        .max()
        .unwrap_or(old.columns.len() as u32);
    for column in table.reference.start.column..=table.reference.end.column {
        if (old.reference.start.column..=old.reference.end.column).contains(&column) {
            let index = (column - old.reference.start.column) as usize;
            wanted.push(old.columns[index].clone());
            properties.push(
                old.column_properties
                    .get(index)
                    .cloned()
                    .unwrap_or(TableColumn {
                        id: index as u32 + 1,
                        ..Default::default()
                    }),
            );
        } else {
            wanted.push(if table.header_row_count > 0 {
                heading(
                    &book,
                    sheet,
                    CellRef::new(column, table.reference.start.row),
                )
            } else {
                String::new()
            });
            next_id = next_id
                .checked_add(1)
                .ok_or_else(|| bad("table column ID overflow"))?;
            properties.push(TableColumn {
                id: next_id,
                ..Default::default()
            });
        }
    }
    table.columns = names_unique(wanted)?;
    table.column_properties = properties;
    if let Some(header) = update.header_row {
        if header != (table.header_row_count > 0) {
            if header {
                let row = table
                    .reference
                    .start
                    .row
                    .checked_sub(1)
                    .ok_or_else(|| bad("there is no room for a header row"))?;
                if !blank_row(
                    sheet,
                    row,
                    table.reference.start.column,
                    table.reference.end.column,
                ) {
                    return Err(bad("the cells above the table are occupied"));
                }
                table.reference.start.row = row;
                table.header_row_count = 1;
                table.auto_filter = true;
            } else {
                clear_row(
                    &mut edits,
                    table.reference.start.row,
                    table.reference.start.column,
                    table.reference.end.column,
                );
                table.reference.start.row += 1;
                table.header_row_count = 0;
                table.auto_filter = false;
            }
        }
    }
    if let Some(totals) = update.totals_row {
        if totals != (table.totals_row_count > 0) {
            if totals {
                table.reference.end.row = table
                    .reference
                    .end
                    .row
                    .checked_add(1)
                    .ok_or_else(|| bad("there is no room for a totals row"))?;
                if !blank_row(
                    sheet,
                    table.reference.end.row,
                    table.reference.start.column,
                    table.reference.end.column,
                ) {
                    return Err(bad("the cells below the table are occupied"));
                }
                table.totals_row_count = 1;
                if table.column_properties.iter().all(|c| {
                    c.totals_function.is_none()
                        && c.totals_label.is_none()
                        && c.totals_formula.is_none()
                }) {
                    let last = table.columns.len() - 1;
                    let numbers = sheet.rows.iter().flat_map(|r| &r.cells).any(|c| {
                        c.reference.column == table.reference.end.column
                            && c.reference.row >= table.reference.start.row + table.header_row_count
                            && c.reference.row < table.reference.end.row
                            && matches!(c.value, CellValue::Number(_))
                    });
                    table.column_properties[last].totals_function =
                        Some(if numbers { "sum" } else { "count" }.into());
                    if last > 0 {
                        table.column_properties[0].totals_label = Some("Total".into());
                    }
                }
            } else {
                clear_row(
                    &mut edits,
                    table.reference.end.row,
                    table.reference.start.column,
                    table.reference.end.column,
                );
                table.reference.end.row = table
                    .reference
                    .end
                    .row
                    .checked_sub(1)
                    .ok_or_else(|| bad("invalid table size"))?;
                table.totals_row_count = 0;
            }
        }
    }
    if let Some(filter) = update.filter_buttons {
        table.auto_filter = filter && table.header_row_count > 0;
    }
    check_area(sheet, table.reference, Some(&old.name))?;
    if table.reference.end.row - table.reference.start.row
        < table.header_row_count + table.totals_row_count
    {
        return Err(bad("a table needs at least one data row"));
    }
    if old.totals_row_count > 0
        && table.totals_row_count > 0
        && table.reference.end.row != old.reference.end.row
    {
        if !blank_row(
            sheet,
            table.reference.end.row,
            table.reference.start.column,
            table.reference.end.column,
        ) {
            return Err(bad("the new totals row is occupied"));
        }
        clear_row(
            &mut edits,
            old.reference.end.row,
            old.reference.start.column.max(table.reference.start.column),
            old.reference.end.column.min(table.reference.end.column),
        );
    }
    for (column, setting) in &update.totals {
        let Some(c) = table.column_properties.get_mut(*column) else {
            return Err(bad("total column is outside the table"));
        };
        if table.totals_row_count == 0 {
            return Err(bad("enable the totals row before setting a total"));
        }
        c.totals_function = None;
        c.totals_label = None;
        c.totals_formula = None;
        match setting {
            TableTotal::None => {}
            TableTotal::Function(f) => c.totals_function = Some(f.name().into()),
            TableTotal::Label(s) => c.totals_label = Some(s.clone()),
            TableTotal::Formula(f) => {
                c.totals_function = Some("custom".into());
                c.totals_formula = Some(formula_valid(f)?);
            }
        }
    }
    for (column, formula) in &update.calculated_columns {
        let Some(c) = table.column_properties.get_mut(*column) else {
            return Err(bad("calculated column is outside the table"));
        };
        c.calculated_formula = formula.as_deref().map(formula_valid).transpose()?;
        if let Some(formula) = &c.calculated_formula {
            let first = CellRef::new(
                table.reference.start.column + *column as u32,
                table.reference.start.row + table.header_row_count,
            );
            let end = table.reference.end.row - table.totals_row_count;
            if u64::from(end - first.row + 1).saturating_mul(update.calculated_columns.len() as u64)
                > crate::clipboard::CLIPBOARD_CELL_LIMIT
            {
                return Err(bad("calculated-column edit exceeds the cell limit"));
            }
            for row in first.row..=end {
                let at = CellRef::new(first.column, row);
                put(
                    &mut edits,
                    at,
                    NewValue::Formula(jade_xlsx_reader::translate_formula(formula, first, at)),
                    None,
                );
            }
        }
    }
    if table.header_row_count > 0 {
        for (i, name) in table.columns.iter().enumerate() {
            put(
                &mut edits,
                CellRef::new(
                    table.reference.start.column + i as u32,
                    table.reference.start.row,
                ),
                NewValue::Text(name.clone()),
                None,
            );
        }
    }
    if table.totals_row_count > 0 {
        for i in 0..table.columns.len() {
            let at = CellRef::new(
                table.reference.start.column + i as u32,
                table.reference.end.row,
            );
            let explicit = update.totals.iter().any(|(column, _)| *column == i);
            if explicit || old.totals_row_count == 0 {
                put(&mut edits, at, totals_value(&table, i)?, None);
            } else if old.reference.end.row != table.reference.end.row {
                let source = CellRef::new(at.column, old.reference.end.row);
                let known =
                    (old.reference.start.column..=old.reference.end.column).contains(&at.column);
                let old_cell = known.then(|| cell(sheet, source)).flatten();
                let value = match old_cell {
                    Some(c) if c.formula.is_some() => NewValue::FormulaCached {
                        formula: c.formula.clone().unwrap(),
                        cached: c.value.clone(),
                    },
                    Some(c) => match &c.value {
                        CellValue::Number(n) => NewValue::Number(*n),
                        CellValue::Text(s) => NewValue::Text(s.clone()),
                        CellValue::Bool(b) => NewValue::Bool(*b),
                        CellValue::Error(e) => NewValue::Error(e.clone()),
                        CellValue::Empty => NewValue::Blank,
                    },
                    None => NewValue::Blank,
                };
                put(
                    &mut edits,
                    at,
                    value,
                    old_cell.and_then(|c| sheet.effective_style_index(source, c.style_index)),
                );
            }
        }
    }
    // Fill recorded calculated columns into newly added rows only; exceptions
    // already in the old data body remain untouched.
    let old_data_end = old.reference.end.row.saturating_sub(old.totals_row_count);
    let new_data_end = table.reference.end.row - table.totals_row_count;
    if new_data_end > old_data_end {
        for (i, c) in table.column_properties.iter().enumerate() {
            if update.calculated_columns.iter().any(|(col, _)| *col == i) {
                continue;
            }
            if let Some(formula) = &c.calculated_formula {
                let first = CellRef::new(
                    table.reference.start.column + i as u32,
                    table.reference.start.row + table.header_row_count,
                );
                if u64::from(new_data_end - old_data_end) > crate::clipboard::CLIPBOARD_CELL_LIMIT {
                    return Err(bad("calculated-column edit exceeds the cell limit"));
                }
                for row in old_data_end + 1..=new_data_end {
                    let at = CellRef::new(first.column, row);
                    if cell(sheet, at)
                        .is_none_or(|c| c.value == CellValue::Empty && c.formula.is_none())
                        || old.totals_row() == Some(row)
                    {
                        put(
                            &mut edits,
                            at,
                            NewValue::Formula(jade_xlsx_reader::translate_formula(
                                formula, first, at,
                            )),
                            None,
                        );
                    }
                }
            }
        }
    }
    let mut package = OpcPackage::open(original)?;
    let xml = package.required_part_str(&old.part_name)?;
    let xml = remap_filter(&edit_definition(&xml, &table)?, old, &table)?;
    package.replace(&old.part_name, xml.into_bytes());
    write_cells(&mut package, &sheet.part_name, edits)?;
    if old.name != table.name {
        rename_references(&mut package, &book, &old.name, &table.name)?;
    }
    ask_recalc(&mut package)?;
    Ok(TableResult {
        bytes: follow_column_names(original, &package.write()?)?,
        name: table.name,
        reference: table.reference,
    })
}
fn rename_references(
    package: &mut OpcPackage,
    book: &Workbook,
    old: &str,
    new: &str,
) -> Result<()> {
    crate::table_formulas::rewrite_package(package, book, &|formula, _, _| {
        jade_xlsx_reader::rewrite_table_references(formula, |name, selector| {
            name.filter(|n| n.to_lowercase() == old.to_lowercase())
                .map(|_| format!("{new}{}", selector.unwrap_or_default()))
        })
    })
}
fn changed_selector(selector: &str, changes: &[(String, Option<String>)]) -> Option<String> {
    let mut splices = Vec::new();
    for (start, end, column) in selector_columns(selector) {
        if let Some((_, new)) = changes
            .iter()
            .find(|(name, _)| name.to_lowercase() == column.to_lowercase())
        {
            let Some(new) = new else {
                return Some("#REF!".into());
            };
            let at = selector[start..end].starts_with('@');
            splices.push(Splice {
                start,
                end,
                text: format!("{}{}", if at { "@" } else { "" }, escaped_column(new)),
            });
        }
    }
    (!splices.is_empty()).then(|| apply(selector, splices))
}
/// Make formulas follow header changes, including current-row references in
/// calculated columns, names, conditional rules, validations and chart sources.
pub fn follow_column_names(original: &[u8], updated: &[u8]) -> Result<Vec<u8>> {
    let old = jade_xlsx_reader::read_xlsx(original)?;
    let new = jade_xlsx_reader::read_xlsx(updated)?;
    let mut changes = Vec::new();
    for table in new.sheets.iter().flat_map(|s| &s.tables) {
        let Some(before) = old
            .sheets
            .iter()
            .flat_map(|s| &s.tables)
            .find(|t| t.part_name == table.part_name)
        else {
            continue;
        };
        let mut columns = Vec::new();
        for (i, name) in before.columns.iter().enumerate() {
            let id = before
                .column_properties
                .get(i)
                .map(|c| c.id)
                .unwrap_or(i as u32 + 1);
            let next = table
                .column_properties
                .iter()
                .position(|c| c.id == id)
                .and_then(|index| table.columns.get(index));
            if next != Some(name) {
                columns.push((name.clone(), next.cloned()));
            }
        }
        if !columns.is_empty() {
            changes.push((table.name.clone(), columns));
        }
    }
    if changes.is_empty() {
        return Ok(updated.to_vec());
    }
    let mut package = OpcPackage::open(updated)?;
    crate::table_formulas::rewrite_package(&mut package, &new, &|formula, context, _| {
        jade_xlsx_reader::rewrite_table_references(formula, |name, selector| {
            let table = name.or_else(|| context.map(|t| t.name.as_str()))?;
            let (_, columns) = changes
                .iter()
                .find(|(n, _)| n.to_lowercase() == table.to_lowercase())?;
            let changed = changed_selector(selector?, columns)?;
            Some(if changed == "#REF!" {
                changed
            } else {
                format!("{}{changed}", name.unwrap_or_default())
            })
        })
    })?;
    Ok(package.write()?)
}

/// A table selector as fixed A1 areas. Current-row selectors keep a relative
/// row so filling an existing calculated formula still follows each row.
pub use jade_xlsx_reader::table_reference::to_a1 as structured_a1;

fn rgb(color: jade_xlsx_reader::Color) -> Option<String> {
    jade_xlsx_layout::color::rgba(color).map(|[r, g, b, _]| format!("{r:02X}{g:02X}{b:02X}"))
}
fn materialize_style(
    package: &mut OpcPackage,
    book: &Workbook,
    sheet: &jade_xlsx_reader::Sheet,
    table: &Table,
) -> Result<()> {
    let Some(styled) = jade_xlsx_layout::table_style::styled_tables(sheet, &book.styles)
        .into_iter()
        .find(|t| t.range == table.reference)
    else {
        return Ok(());
    };
    let count = u64::from(table.reference.end.row - table.reference.start.row + 1)
        * u64::from(table.reference.end.column - table.reference.start.column + 1);
    if count > crate::clipboard::CLIPBOARD_CELL_LIMIT {
        return Err(bad("converting this table style exceeds the cell limit"));
    }
    let bytes = package.write()?;
    let (styles_part, mut styles_xml) = crate::styles_part(&bytes)?;
    let mut edits = BTreeMap::new();
    let mut styles = BTreeMap::new();
    for row in table.reference.start.row..=table.reference.end.row {
        for column in table.reference.start.column..=table.reference.end.column {
            let at = CellRef::new(column, row);
            let own = cell(sheet, at);
            let base = sheet
                .effective_style_index(at, own.and_then(|c| c.style_index))
                .unwrap_or(0);
            let format = book.styles.cell_format(Some(base));
            let font = format.and_then(|f| book.styles.font(f));
            let fill = format.and_then(|f| book.styles.fill(f));
            let border = format.and_then(|f| book.styles.border(f));
            let look = styled.style_at(at).unwrap();
            let normal = book.styles.normal_font().and_then(|f| f.color);
            let own_color = font.and_then(|f| f.color);
            let edge = |table: &Option<jade_xlsx_reader::BorderEdge>,
                        own: Option<&jade_xlsx_reader::BorderEdge>| {
                if own.is_some() {
                    None
                } else {
                    table.as_ref().and_then(|edge| {
                        crate::BorderStyle::parse(&edge.style).map(|style| crate::BorderLine {
                            style,
                            color: edge.color.and_then(rgb),
                        })
                    })
                }
            };
            let change = crate::FormatChange {
                bold: (look.bold && !font.is_some_and(|f| f.bold)).then_some(true),
                italic: (look.italic && !font.is_some_and(|f| f.italic)).then_some(true),
                font_color: if own_color.is_none()
                    || own_color == normal
                    || own_color == Some(jade_xlsx_reader::Color::Auto)
                {
                    look.text_color.and_then(rgb)
                } else {
                    None
                },
                fill_color: if fill
                    .is_none_or(|f| matches!(f.pattern.as_deref(), None | Some("none" | "gray125")))
                {
                    look.fill.as_ref().and_then(|f| f.foreground).and_then(rgb)
                } else {
                    None
                },
                border_left: edge(&look.left, border.and_then(|b| b.left.as_ref())),
                border_right: edge(&look.right, border.and_then(|b| b.right.as_ref())),
                border_top: edge(&look.top, border.and_then(|b| b.top.as_ref())),
                border_bottom: edge(&look.bottom, border.and_then(|b| b.bottom.as_ref())),
                ..Default::default()
            };
            if change.is_empty() {
                continue;
            }
            let key = format!("{base}:{change:?}");
            let index = if let Some(index) = styles.get(&key) {
                *index
            } else {
                let (next, index) = crate::add_cell_format(&styles_xml, Some(base), &change)?;
                styles_xml = next;
                styles.insert(key, index);
                index
            };
            put(&mut edits, at, NewValue::Unchanged, Some(index));
        }
    }
    crate::styles::install_styles_relationship(package, &styles_part)?;
    package.replace(&styles_part, styles_xml.into_bytes());
    write_cells(package, &sheet.part_name, edits)
}
fn relationship_id(xml: &str) -> Result<Option<String>> {
    Ok(jade_xlsx_reader::table::table_part_id(xml)?)
}

fn remove_definition(
    package: &mut OpcPackage,
    sheet: &jade_xlsx_reader::Sheet,
    table: &Table,
) -> Result<()> {
    let rels_part = jade_opc::rels_part_name(&sheet.part_name);
    let rels = package.required_part_str(&rels_part)?;
    let parsed = jade_opc::parse_relationships(&rels)?;
    let ids: HashSet<_> = parsed
        .by_type_suffix("/table")
        .into_iter()
        .filter(|r| jade_opc::resolve_target(&sheet.part_name, &r.target) == table.part_name)
        .map(|r| r.id.clone())
        .collect();
    let mut splices = Vec::new();
    for (start, end, entry) in xml_edit_ns::children(&rels, b"Relationship")? {
        if attr(&entry.text, b"Id")?.is_some_and(|id| ids.contains(&id)) {
            splices.push(Splice {
                start,
                end,
                text: String::new(),
            });
        }
    }
    package.replace(&rels_part, apply(&rels, splices).into_bytes());
    let xml = package.required_part_str(&sheet.part_name)?;
    if let Some((_, _, container)) = xml_edit_ns::children(&xml, b"tableParts")?
        .into_iter()
        .next()
    {
        let entries = xml_edit_ns::children(&container.text, b"tablePart")?;
        let mut splices = Vec::new();
        let mut remaining = entries.len();
        for (start, end, entry) in entries {
            if relationship_id(&entry.text)?.is_some_and(|id| ids.contains(&id)) {
                remaining -= 1;
                splices.push(Splice {
                    start,
                    end,
                    text: String::new(),
                });
            }
        }
        let changed = if remaining == 0 {
            String::new()
        } else {
            container.finish(&xml_edit::root_attributes(
                &apply(&container.text, splices),
                &[("count", Some(remaining.to_string()))],
            )?)?
        };
        package.replace(
            &sheet.part_name,
            xml_edit_ns::set_child(&xml, b"tableParts", |_| changed, &[])?.into_bytes(),
        );
    }
    let mut shared = false;
    for part in package.part_names() {
        if let Some(source) = crate::sheets::source_of_rels(part) {
            if let Some(xml) = package.part_str(part)? {
                if jade_opc::parse_relationships(&xml)?.all().iter().any(|r| {
                    !r.external && jade_opc::resolve_target(&source, &r.target) == table.part_name
                }) {
                    shared = true;
                    break;
                }
            }
        }
    }
    if !shared {
        package.remove(&table.part_name);
        package.remove(&jade_opc::rels_part_name(&table.part_name));
        let types = package.required_part_str("[Content_Types].xml")?;
        let mut splices = Vec::new();
        for (start, end, entry) in xml_edit_ns::children(&types, b"Override")? {
            if attr(&entry.text, b"PartName")?.as_deref() == Some(&format!("/{}", table.part_name))
            {
                splices.push(Splice {
                    start,
                    end,
                    text: String::new(),
                });
            }
        }
        package.replace("[Content_Types].xml", apply(&types, splices).into_bytes());
    }
    Ok(())
}
/// Convert to a normal range, preserving the displayed table style and turning
/// structured references into equivalent fixed/current-row A1 references.
pub fn convert_to_range(original: &[u8], sheet_name: &str, table_name: &str) -> Result<Vec<u8>> {
    let book = jade_xlsx_reader::read_xlsx(original)?;
    let sheet = book
        .sheets
        .iter()
        .find(|s| s.name == sheet_name)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet_name.into()))?;
    let table = sheet
        .tables
        .iter()
        .find(|t| t.name.to_lowercase() == table_name.to_lowercase())
        .ok_or_else(|| bad("no such table"))?;
    let mut package = OpcPackage::open(original)?;
    materialize_style(&mut package, &book, sheet, table)?;
    crate::table_formulas::rewrite_package(&mut package, &book, &|formula, context, at| {
        jade_xlsx_reader::rewrite_table_references(formula, |name, selector| {
            let name = name.or_else(|| context.map(|t| t.name.as_str()))?;
            if name.to_lowercase() == table.name.to_lowercase()
                || name.to_lowercase() == table.display_name.to_lowercase()
            {
                Some(structured_a1(table, &sheet.name, selector, at))
            } else {
                None
            }
        })
    })?;
    remove_definition(&mut package, sheet, table)?;
    ask_recalc(&mut package)?;
    Ok(package.write()?)
}

/// Table auto-expansion and calculated-column fill after actual user cell
/// changes. Formatting/recalculation and protected sheets do not trigger it.
pub fn after_cell_entries(
    before: &Workbook,
    written: &[u8],
    sheets: &[crate::SheetEdits],
) -> Result<Vec<u8>> {
    let relevant: Vec<_> = sheets
        .iter()
        .filter_map(|edits| {
            let sheet = before
                .sheets
                .iter()
                .find(|s| s.part_name == edits.part_name)?;
            if sheet.protection.is_some() || sheet.tables.is_empty() {
                return None;
            }
            let cells: Vec<_> = edits
                .cells
                .iter()
                .filter(|e| {
                    !matches!(
                        e.value,
                        NewValue::Unchanged | NewValue::Blank | NewValue::Remove
                    )
                })
                .collect();
            (!cells.is_empty()).then_some((sheet, cells))
        })
        .collect();
    if relevant.is_empty() {
        return Ok(written.to_vec());
    }
    let mut bytes = written.to_vec();
    for (original, entries) in relevant {
        for initial in &original.tables {
            let candidate = entries.iter().any(|e| {
                let r = initial.reference;
                let at = e.reference;
                (r.contains(at)
                    && matches!(
                        e.value,
                        NewValue::Formula(_) | NewValue::FormulaCached { .. }
                    ))
                    || (initial.totals_row_count == 0
                        && at.row == r.end.row.saturating_add(1)
                        && (r.start.column..=r.end.column).contains(&at.column))
                    || (at.column == r.end.column.saturating_add(1)
                        && (r.start.row..=r.end.row).contains(&at.row))
            });
            if !candidate {
                continue;
            }
            let book = jade_xlsx_reader::read_xlsx(&bytes)?;
            let sheet = book
                .sheets
                .iter()
                .find(|s| s.part_name == original.part_name)
                .unwrap();
            let Some(table) = sheet
                .tables
                .iter()
                .find(|t| t.part_name == initial.part_name)
            else {
                continue;
            };
            let mut reference = table.reference;
            // Extend through consecutive populated entry rows/columns, including
            // a rectangular paste beginning immediately next to the table.
            loop {
                let mut next = reference;
                if table.totals_row_count == 0
                    && reference.end.row + 1 < jade_xlsx_reader::model::MAX_ROW_COUNT
                    && entries.iter().any(|e| {
                        e.reference.row == reference.end.row + 1
                            && (reference.start.column..=reference.end.column)
                                .contains(&e.reference.column)
                    })
                {
                    next.end.row += 1;
                }
                if reference.end.column + 1 < jade_xlsx_reader::model::MAX_COLUMN_COUNT
                    && entries.iter().any(|e| {
                        e.reference.column == reference.end.column + 1
                            && (reference.start.row..=next.end.row).contains(&e.reference.row)
                    })
                {
                    next.end.column += 1;
                }
                if next == reference || check_area(sheet, next, Some(&table.name)).is_err() {
                    break;
                }
                reference = next;
            }
            let data_first = reference.start.row + table.header_row_count;
            let data_last = reference.end.row - table.totals_row_count;
            let mut calculated = Vec::new();
            for column in reference.start.column..=reference.end.column {
                let formulas: Vec<_> = entries
                    .iter()
                    .filter(|e| {
                        e.reference.column == column
                            && (data_first..=data_last).contains(&e.reference.row)
                            && matches!(
                                e.value,
                                NewValue::Formula(_) | NewValue::FormulaCached { .. }
                            )
                    })
                    .collect();
                if formulas.len() != 1 {
                    continue;
                }
                let at = formulas[0].reference;
                let other_content = sheet.rows.iter().flat_map(|r| &r.cells).any(|c| {
                    c.reference.column == column
                        && c.reference != at
                        && (data_first..=data_last).contains(&c.reference.row)
                        && (c.formula.is_some() || c.value != CellValue::Empty)
                });
                if other_content {
                    continue;
                }
                let Some(formula) = cell(sheet, at).and_then(|c| c.formula.as_ref()) else {
                    continue;
                };
                let origin = CellRef::new(column, data_first);
                calculated.push((
                    (column - reference.start.column) as usize,
                    Some(jade_xlsx_reader::translate_formula(formula, at, origin)),
                ));
            }
            if reference == table.reference && calculated.is_empty() {
                continue;
            }
            let update = TableUpdate {
                reference: (reference != table.reference).then_some(reference),
                calculated_columns: calculated,
                ..Default::default()
            };
            bytes = update_table(&bytes, &sheet.name, &table.name, &update)?.bytes;
        }
    }
    Ok(bytes)
}

/// Keep per-column totals metadata in step with direct cell edits, including
/// clearing a total or entering a numeric constant. The literal cell is kept
/// exactly as typed rather than being regenerated from metadata.
pub fn follow_total_cells(
    before: &Workbook,
    written: &[u8],
    sheets: &[crate::SheetEdits],
) -> Result<Vec<u8>> {
    let mut touched = Vec::new();
    for edits in sheets {
        if let Some(sheet) = before
            .sheets
            .iter()
            .find(|s| s.part_name == edits.part_name)
        {
            for table in &sheet.tables {
                if let Some(row) = table.totals_row() {
                    let cells: Vec<_> = edits
                        .cells
                        .iter()
                        .filter(|e| {
                            e.reference.row == row
                                && e.reference.column >= table.reference.start.column
                                && !matches!(e.value, NewValue::Unchanged)
                        })
                        .map(|e| e.reference)
                        .collect();
                    if !cells.is_empty() {
                        touched.push((sheet.part_name.clone(), table.part_name.clone(), cells));
                    }
                }
            }
        }
    }
    if touched.is_empty() {
        return Ok(written.to_vec());
    }
    let book = jade_xlsx_reader::read_xlsx(written)?;
    let mut package = OpcPackage::open(written)?;
    for (sheet_part, part, cells) in touched {
        let Some(sheet) = book.sheets.iter().find(|s| s.part_name == sheet_part) else {
            continue;
        };
        let Some(original) = sheet.tables.iter().find(|t| t.part_name == part) else {
            continue;
        };
        let mut table = original.clone();
        for at in cells {
            if !table.reference.contains(at) {
                continue;
            }
            let i = (at.column - table.reference.start.column) as usize;
            let Some(properties) = table.column_properties.get_mut(i) else {
                continue;
            };
            properties.totals_function = None;
            properties.totals_label = None;
            properties.totals_formula = None;
            if let Some(c) = cell(sheet, at) {
                if let Some(f) = &c.formula {
                    properties.totals_function = Some("custom".into());
                    properties.totals_formula = Some(f.clone());
                } else if let CellValue::Text(text) = &c.value {
                    properties.totals_label = Some(text.clone());
                }
            }
        }
        let xml = package.required_part_str(&part)?;
        package.replace(&part, edit_definition(&xml, &table)?.into_bytes());
    }
    Ok(package.write()?)
}
