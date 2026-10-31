//! Adding, renaming, deleting and moving sheets.
//!
//! A sheet is a `<sheet>` entry in `workbook.xml`, a relationship from the
//! workbook part to its worksheet part, a content type for that part and a
//! title in `docProps/app.xml`. Beyond those, the workbook refers to sheets
//! by position — a defined name's `localSheetId`, the workbook view's
//! `activeTab` and `firstSheet` — and formulas refer to them by name. Each
//! operation here keeps all of that consistent, and, as everywhere in this
//! crate, patches the parts as text so whatever it does not touch comes
//! back exactly as it was.
//!
//! What Excel does, and so what this does:
//!
//! - **Rename** rewrites every formula, defined name, conditional format,
//!   validation, table formula and chart series that names the sheet, and
//!   a pivot cache's source sheet. Text in string literals is not a
//!   reference (`INDIRECT("Sheet1!A1")` keeps its text), nor is a
//!   hyperlink's location, which Excel does not follow either.
//! - **Delete** turns every reference into the sheet into `#REF!`
//!   (`Sheet2!A1` → `#REF!A1`), and every structured reference into one of
//!   its tables into `#REF!`; removes the names scoped to it; and removes
//!   its worksheet part with every part only it reached — its tables,
//!   drawing, charts, comments. The last visible sheet cannot be deleted.
//!   The calculation chain, which lists cells by sheet, is dropped for
//!   Excel to rebuild, and the workbook asks to be recalculated.
//! - **Move** changes no formula: references are by name.

use std::collections::{BTreeSet, HashSet};

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use jade_opc::OpcPackage;
use jade_xlsx_reader::formula::same_sheet_name;
use jade_xlsx_reader::xstring::{decode_xstring, encode_xstring};
use jade_xlsx_reader::{delete_sheet_in_formula, quote_sheet_name, rename_sheet_in_formula};

use crate::patch::{apply, attribute, escape, local, namespace_prefix, Splice};
use crate::shift::{remove_elements, rewrite};
use crate::{Result, WriteError};

/// Changes a tab's visibility, retaining at least one visible worksheet and
/// moving the saved active tab away from a newly hidden sheet.
pub fn set_sheet_visibility(
    original: &[u8],
    name: &str,
    visibility: jade_xlsx_reader::SheetVisibility,
) -> Result<Vec<u8>> {
    use jade_xlsx_reader::SheetVisibility;
    let mut parts = WorkbookParts::open(original)?;
    let index = parts.position(name)?;
    let parsed = jade_xlsx_reader::workbook_part::parse_workbook(&parts.workbook_xml)?;
    if parsed.sheets[index].visibility == visibility {
        return Ok(original.to_vec());
    }
    parts.ensure_structure_unlocked()?;
    if visibility != SheetVisibility::Visible
        && !parts
            .sheets
            .iter()
            .enumerate()
            .any(|(i, s)| i != index && s.visible)
    {
        return Err(WriteError::LastVisibleSheet(name.to_string()));
    }
    let entry = &parts.sheets[index];
    let state = match visibility {
        SheetVisibility::Visible => None,
        SheetVisibility::Hidden => Some("hidden".to_string()),
        SheetVisibility::VeryHidden => Some("veryHidden".to_string()),
    };
    let mut reader = Reader::from_str(&parts.workbook_xml[entry.start..entry.end]);
    let event = reader.read_event()?;
    let empty = matches!(event, Event::Empty(_));
    let head = match event {
        Event::Start(ref element) | Event::Empty(ref element) => {
            rewrite(element, &[("state", state)], empty)?
        }
        _ => unreachable!(),
    };
    let mut xml = apply(
        &parts.workbook_xml,
        vec![Splice {
            start: entry.start,
            end: entry.start + reader.buffer_position() as usize,
            text: head,
        }],
    );
    if visibility != SheetVisibility::Visible {
        let next = (index + 1..parts.sheets.len())
            .chain(0..index)
            .find(|i| parts.sheets[*i].visible)
            .unwrap();
        let mut views = Reader::from_str(&xml);
        let mut splices = Vec::new();
        let mut seen = false;
        let mut buf = Vec::new();
        loop {
            let start = views.buffer_position() as usize;
            let event = views.read_event_into(&mut buf)?;
            match event {
                Event::Start(ref e) | Event::Empty(ref e)
                    if local(e.name().as_ref()) == b"workbookView" =>
                {
                    seen = true;
                    let mut updates = Vec::new();
                    for key in ["activeTab", "firstSheet"] {
                        let current = attribute(e, key.as_bytes())?
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        if current == index {
                            updates.push((key, Some(next.to_string())));
                        }
                    }
                    if !updates.is_empty() {
                        splices.push(Splice {
                            start,
                            end: views.buffer_position() as usize,
                            text: rewrite(e, &updates, matches!(event, Event::Empty(_)))?,
                        });
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        xml = apply(&xml, splices);
        if !seen && index == 0 {
            xml = crate::header_footer::set_child(
                &xml,
                b"bookViews",
                |p| {
                    format!("<{p}bookViews><{p}workbookView activeTab=\"{next}\" firstSheet=\"{next}\"/></{p}bookViews>")
                },
                &[
                    b"sheets",
                    b"functionGroups",
                    b"externalReferences",
                    b"definedNames",
                    b"calcPr",
                    b"extLst",
                ],
            )?;
        }
        if let Some(part) = parts.sheet_parts()?.get(index).and_then(Clone::clone) {
            if let Some(sheet_xml) = parts.package.part_str(&part)? {
                let sheet_xml = rewrite_selected_tabs(&sheet_xml)?;
                parts.package.replace(&part, sheet_xml.into_bytes());
            }
        }
    }
    parts.finish(xml)
}

fn rewrite_selected_tabs(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut splices = Vec::new();
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e)
                if local(e.name().as_ref()) == b"sheetView" =>
            {
                splices.push(Splice {
                    start,
                    end: reader.buffer_position() as usize,
                    text: rewrite(
                        e,
                        &[("tabSelected", Some("0".into()))],
                        matches!(event, Event::Empty(_)),
                    )?,
                });
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

/// Sets or clears a sheet tab's sRGB colour without replacing sheetPr.
pub fn set_tab_color(original: &[u8], name: &str, color: Option<&str>) -> Result<Vec<u8>> {
    if color.is_some_and(|color| color.len() != 6 || !color.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(WriteError::BadColor {
            value: color.unwrap().into(),
        });
    }
    let mut parts = WorkbookParts::open(original)?;
    let index = parts.position(name)?;
    let part = parts
        .sheet_parts()?
        .get(index)
        .and_then(Clone::clone)
        .ok_or_else(|| WriteError::NoSuchSheet(name.to_string()))?;
    let xml = parts.package.required_part_str(&part)?;
    if color.is_none() && crate::xml_edit::child_xml(&xml, b"sheetPr")?.is_none() {
        return Ok(original.to_vec());
    }
    let patched = crate::xml_edit::edit_child(
        &xml,
        b"sheetPr",
        &[
            b"dimension",
            b"sheetViews",
            b"sheetFormatPr",
            b"cols",
            b"sheetData",
            b"extLst",
        ],
        |property| {
            crate::header_footer::set_child(
                property,
                b"tabColor",
                |p| {
                    color
                        .map(|color| {
                            format!("<{p}tabColor rgb=\"FF{}\"/>", color.to_ascii_uppercase())
                        })
                        .unwrap_or_default()
                },
                &[b"outlinePr", b"pageSetUpPr"],
            )
        },
    )?;
    parts.package.replace(&part, patched.into_bytes());
    parts.package.write().map_err(Into::into)
}

const WORKSHEET_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const WORKSHEET_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const TRANSITIONAL_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const TRANSITIONAL_RELATIONSHIPS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const STRICT_MAIN: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const STRICT_RELATIONSHIPS: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";

/// Excel's longest sheet name, in UTF-16 code units — the unit Excel
/// counts in.
pub const MAX_SHEET_NAME_LENGTH: usize = 31;

/// Checks `name` against Excel's rules for a sheet name: 1 to 31
/// characters (UTF-16 code units), none of `[ ] : * ? / \` or a control
/// character, no apostrophe at either end, not `History` (which Excel
/// reserves), and not the name of one of the `others` sheets, compared
/// without regard to case.
pub fn validate_sheet_name(name: &str, others: &[&str]) -> Result<()> {
    let invalid = |reason| {
        Err(WriteError::InvalidSheetName {
            name: name.to_string(),
            reason,
        })
    };
    if name.is_empty() {
        return invalid("a sheet name cannot be empty");
    }
    if name.encode_utf16().count() > MAX_SHEET_NAME_LENGTH {
        return invalid("a sheet name is at most 31 characters");
    }
    if name
        .chars()
        .any(|c| matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
    {
        return invalid("a sheet name cannot contain [ ] : * ? / \\");
    }
    if name.chars().any(char::is_control) {
        return invalid("a sheet name cannot contain control characters");
    }
    if name.starts_with('\'') || name.ends_with('\'') {
        return invalid("a sheet name cannot begin or end with an apostrophe");
    }
    if name.eq_ignore_ascii_case("history") {
        return invalid("Excel reserves the name History");
    }
    if others.iter().any(|other| same_sheet_name(other, name)) {
        return Err(WriteError::DuplicateSheetName(name.to_string()));
    }
    Ok(())
}

/// One `<sheet>` entry of `workbook.xml`.
#[derive(Debug, Clone)]
struct SheetEntry {
    /// Where the element is in the workbook part.
    start: usize,
    end: usize,
    name: String,
    relationship_id: Option<String>,
    visible: bool,
    sheet_id: Option<u32>,
}

/// The workbook part and what it says about its sheets.
struct WorkbookParts {
    package: OpcPackage,
    workbook_part: String,
    workbook_xml: String,
    sheets: Vec<SheetEntry>,
    /// The `<sheets>` element: where its content starts and ends.
    sheets_content: (usize, usize),
    /// The sheet element's prefix (`x:` in a prefixed part).
    prefix: String,
    /// The qualified name the sheets use for their relationship id (`r:id`).
    id_attribute: Option<String>,
}

impl WorkbookParts {
    fn ensure_structure_unlocked(&self) -> Result<()> {
        if jade_xlsx_reader::workbook_part::parse_workbook(&self.workbook_xml)?.structure_protected
        {
            return Err(WriteError::WorkbookStructureProtected);
        }
        Ok(())
    }

    fn open(original: &[u8]) -> Result<Self> {
        let package = OpcPackage::open(original)?;
        let workbook_part = crate::workbook_part_name(&package)?;
        let workbook_xml = package
            .part_str(&workbook_part)?
            .ok_or_else(|| WriteError::MissingPart(workbook_part.clone()))?;

        let mut reader = Reader::from_str(&workbook_xml);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut sheets = Vec::new();
        let mut sheets_content = None;
        let mut content_start = None;
        let mut prefix = String::new();
        let mut id_attribute = None;
        loop {
            let start = reader.buffer_position() as usize;
            let event = reader.read_event_into(&mut buf)?;
            match event {
                Event::Start(ref element) if local(element.name().as_ref()) == b"sheets" => {
                    content_start = Some(reader.buffer_position() as usize);
                }
                Event::Empty(ref element) if local(element.name().as_ref()) == b"sheets" => {
                    let at = reader.buffer_position() as usize;
                    sheets_content = Some((at, at));
                }
                Event::End(ref element) if local(element.name().as_ref()) == b"sheets" => {
                    if let Some(content) = content_start.take() {
                        sheets_content = Some((content, start));
                    }
                }
                Event::Start(ref element) | Event::Empty(ref element)
                    if content_start.is_some() && local(element.name().as_ref()) == b"sheet" =>
                {
                    let empty = matches!(event, Event::Empty(_));
                    let qualified = element.name().as_ref().to_vec();
                    if !empty {
                        reader.read_to_end_into(QName(&qualified), &mut Vec::new())?;
                    }
                    prefix = namespace_prefix(&qualified);
                    let mut name = String::new();
                    let mut relationship_id = None;
                    let mut visible = true;
                    let mut sheet_id = None;
                    for attribute in element.attributes() {
                        let attribute = attribute?;
                        let key = attribute.key.as_ref();
                        let value = attribute.unescape_value()?.into_owned();
                        match local(key) {
                            b"name" => name = decode_xstring(&value),
                            b"sheetId" => sheet_id = value.parse().ok(),
                            b"state" => visible = value == "visible",
                            b"id" if key.contains(&b':') => {
                                if id_attribute.is_none() {
                                    id_attribute = Some(String::from_utf8_lossy(key).into_owned());
                                }
                                relationship_id = Some(value);
                            }
                            _ => {}
                        }
                    }
                    sheets.push(SheetEntry {
                        start,
                        end: reader.buffer_position() as usize,
                        name,
                        relationship_id,
                        visible,
                        sheet_id,
                    });
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        let sheets_content = sheets_content
            .ok_or_else(|| WriteError::MissingPart(format!("{workbook_part} <sheets>")))?;
        Ok(Self {
            package,
            workbook_part,
            workbook_xml,
            sheets,
            sheets_content,
            prefix,
            id_attribute,
        })
    }

    /// The position of the sheet called `name`: its exact name first, then
    /// without regard to case.
    fn position(&self, name: &str) -> Result<usize> {
        self.sheets
            .iter()
            .position(|sheet| sheet.name == name)
            .or_else(|| {
                self.sheets
                    .iter()
                    .position(|sheet| same_sheet_name(&sheet.name, name))
            })
            .ok_or_else(|| WriteError::NoSuchSheet(name.to_string()))
    }

    fn names(&self) -> Vec<String> {
        self.sheets.iter().map(|sheet| sheet.name.clone()).collect()
    }

    fn workbook_rels_name(&self) -> String {
        jade_opc::rels_part_name(&self.workbook_part)
    }

    fn relationships(&self) -> Result<jade_opc::Relationships> {
        Ok(match self.package.part_str(&self.workbook_rels_name())? {
            Some(xml) => jade_opc::parse_relationships(&xml)?,
            None => Default::default(),
        })
    }

    /// The part each sheet's relationship points at, in sheet order.
    fn sheet_parts(&self) -> Result<Vec<Option<String>>> {
        let relationships = self.relationships()?;
        Ok(self
            .sheets
            .iter()
            .map(|sheet| {
                sheet
                    .relationship_id
                    .as_deref()
                    .and_then(|id| relationships.relationship(id))
                    .filter(|relationship| !relationship.external)
                    .map(|relationship| {
                        jade_opc::resolve_target(&self.workbook_part, &relationship.target)
                    })
            })
            .collect())
    }

    /// The names of the sheets that are worksheets, in order: what the
    /// `Worksheets` titles of `docProps/app.xml` list.
    fn worksheet_names(&self) -> Result<Vec<String>> {
        let relationships = self.relationships()?;
        Ok(self
            .sheets
            .iter()
            .filter(|sheet| {
                sheet
                    .relationship_id
                    .as_deref()
                    .and_then(|id| relationships.relationship(id))
                    .is_some_and(|relationship| {
                        relationship.relationship_type.ends_with("/worksheet")
                    })
            })
            .map(|sheet| sheet.name.clone())
            .collect())
    }

    fn finish(mut self, workbook_xml: String) -> Result<Vec<u8>> {
        self.package
            .replace(&self.workbook_part, workbook_xml.into_bytes());
        Ok(self.package.write()?)
    }
}

/// How positions in the sheet list change, for what refers to sheets by
/// position.
struct Positions<'a> {
    /// Where the sheet at each old position goes; `None` when it is deleted.
    map: &'a dyn Fn(usize) -> Option<usize>,
    /// The active tab when the active sheet is the one deleted.
    fallback: usize,
}

/// `workbook.xml` with its `<sheets>` content replaced by `sheets` (when
/// given), positions remapped, and every defined name's formula passed
/// through `formula`. A name scoped to a deleted sheet goes with it.
fn rewrite_workbook(
    xml: &str,
    sheets: Option<(usize, usize, String)>,
    positions: &Positions<'_>,
    formula: &dyn Fn(&str) -> String,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices = Vec::new();
    if let Some((start, end, text)) = sheets {
        splices.push(Splice { start, end, text });
    }
    let map_index = |value: Option<String>| -> Option<Option<usize>> {
        value
            .and_then(|value| value.parse::<usize>().ok())
            .map(|index| (positions.map)(index))
    };
    // (start, names kept, names removed) of the `<definedNames>` element.
    let mut names: Option<(usize, usize, usize)> = None;
    let mut content: Option<usize> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let head_end = reader.buffer_position() as usize;
                match local(element.name().as_ref()) {
                    b"workbookView" => {
                        let mut updates = Vec::new();
                        if let Some(active) = map_index(attribute(element, b"activeTab")?) {
                            updates.push((
                                "activeTab",
                                Some(active.unwrap_or(positions.fallback).to_string()),
                            ));
                        }
                        if let Some(first) = map_index(attribute(element, b"firstSheet")?) {
                            updates.push((
                                "firstSheet",
                                Some(first.unwrap_or(positions.fallback).to_string()),
                            ));
                        }
                        if !updates.is_empty() {
                            splices.push(Splice {
                                start,
                                end: head_end,
                                text: rewrite(element, &updates, empty)?,
                            });
                        }
                    }
                    b"definedNames" if !empty => names = Some((start, 0, 0)),
                    b"definedName" => match map_index(attribute(element, b"localSheetId")?) {
                        Some(None) => {
                            let name = element.name().as_ref().to_vec();
                            if !empty {
                                reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                            }
                            splices.push(Splice {
                                start,
                                end: reader.buffer_position() as usize,
                                text: String::new(),
                            });
                            if let Some(names) = names.as_mut() {
                                names.2 += 1;
                            }
                        }
                        mapped => {
                            if let Some(Some(index)) = mapped {
                                splices.push(Splice {
                                    start,
                                    end: head_end,
                                    text: rewrite(
                                        element,
                                        &[("localSheetId", Some(index.to_string()))],
                                        empty,
                                    )?,
                                });
                            }
                            if let Some(names) = names.as_mut() {
                                names.1 += 1;
                            }
                            if !empty {
                                content = Some(head_end);
                            }
                        }
                    },
                    _ => {}
                }
            }
            Event::End(ref element) => match local(element.name().as_ref()) {
                b"definedName" => {
                    if let Some(content_start) = content.take() {
                        if let Some(splice) = rewritten_content(xml, content_start, start, formula)?
                        {
                            splices.push(splice);
                        }
                    }
                }
                b"definedNames" => {
                    if let Some((names_start, kept, removed)) = names.take() {
                        if kept == 0 && removed > 0 {
                            splices.push(Splice {
                                start: names_start,
                                end: reader.buffer_position() as usize,
                                text: String::new(),
                            });
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

/// The element content between `start` and `end` passed through `formula`,
/// as a splice — or nothing when it does not change.
fn rewritten_content(
    xml: &str,
    start: usize,
    end: usize,
    formula: &dyn Fn(&str) -> String,
) -> Result<Option<Splice>> {
    let raw = &xml[start..end];
    let text = quick_xml::escape::unescape(raw).map_err(quick_xml::Error::from)?;
    let rewritten = formula(&text);
    if rewritten == text {
        return Ok(None);
    }
    Ok(Some(Splice {
        start,
        end,
        text: escape_text(&rewritten),
    }))
}

/// `text` as element content: only `&`, `<` and `>` need escaping there, so
/// a formula keeps its apostrophes and quotes as Excel writes them.
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A part with the text of every formula element — a cell's `<f>`, a
/// conditional format's `<formula>`, a validation's `<formula1>` and
/// `<formula2>`, an extension's `<xm:f>`, a chart series' `<c:f>`, a table
/// column's calculated and totals formulas — passed through `formula`.
pub(crate) fn rewrite_formula_elements(
    xml: &str,
    formula: &dyn Fn(&str) -> String,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices = Vec::new();
    let mut content: Option<usize> = None;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) => {
                // Only an element holding text is a formula; `<formula1>`
                // around an `<xm:f>` is not.
                content = is_formula_element(local(element.name().as_ref()))
                    .then_some(reader.buffer_position() as usize);
            }
            Event::Empty(_) => content = None,
            Event::End(ref element) => {
                if let Some(content_start) = content.take() {
                    if is_formula_element(local(element.name().as_ref())) {
                        if let Some(splice) = rewritten_content(xml, content_start, start, formula)?
                        {
                            splices.push(splice);
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

fn is_formula_element(name: &[u8]) -> bool {
    matches!(
        name,
        b"f" | b"formula"
            | b"formula1"
            | b"formula2"
            | b"calculatedColumnFormula"
            | b"totalsRowFormula"
    )
}

/// Every part a formula can name a sheet in: the worksheets, the tables
/// hung off them, and every chart.
pub(crate) fn formula_parts(package: &OpcPackage) -> Result<BTreeSet<String>> {
    let mut parts = BTreeSet::new();
    let mut active_tables = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    for (source, relationship) in all_relationships(package)? {
        let kind = &relationship.relationship_type;
        if kind.ends_with("/table") {
            if !active_tables.contains_key(&source) {
                let active = match package.part_str(&source)? {
                    Some(sheet_xml) => {
                        let rels = package
                            .part_str(&jade_opc::rels_part_name(&source))?
                            .map(|x| jade_opc::parse_relationships(&x))
                            .transpose()?
                            .unwrap_or_default();
                        jade_xlsx_reader::table::referenced_table_parts(&sheet_xml, &rels, &source)?
                            .into_iter()
                            .collect()
                    }
                    None => BTreeSet::new(),
                };
                active_tables.insert(source.clone(), active);
            }
            let target = jade_opc::resolve_target(&source, &relationship.target);
            if !active_tables[&source].contains(&target) {
                continue;
            }
        }
        if ["/worksheet", "/table", "/chart", "/chartEx"]
            .iter()
            .any(|suffix| kind.ends_with(suffix))
        {
            let part = jade_opc::resolve_target(&source, &relationship.target);
            if package.part(&part).is_some() {
                parts.insert(part);
            }
        }
    }
    Ok(parts)
}

/// Every internal relationship in the package, with the part it is from
/// (`""` for the package itself).
fn all_relationships(package: &OpcPackage) -> Result<Vec<(String, jade_opc::Relationship)>> {
    let mut found = Vec::new();
    for rels in package.part_names() {
        let Some(source) = source_of_rels(rels) else {
            continue;
        };
        let Some(xml) = package.part_str(rels)? else {
            continue;
        };
        for relationship in jade_opc::parse_relationships(&xml)?.all() {
            if !relationship.external {
                found.push((source.clone(), relationship.clone()));
            }
        }
    }
    Ok(found)
}

/// The part a `.rels` part describes: `xl/_rels/workbook.xml.rels` →
/// `xl/workbook.xml`, `_rels/.rels` → the package (`""`).
pub(crate) fn source_of_rels(rels: &str) -> Option<String> {
    let file = rels.strip_suffix(".rels")?;
    let (directory, name) = match file.rsplit_once('/') {
        Some((directory, name)) => (directory, name),
        None => return None,
    };
    let parent = if directory == "_rels" {
        ""
    } else {
        directory.strip_suffix("/_rels")?
    };
    Some(if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    })
}

/// Passes every formula in the package through `formula`.
pub(crate) fn rewrite_package_formulas(
    package: &mut OpcPackage,
    formula: &dyn Fn(&str) -> String,
) -> Result<()> {
    for part in formula_parts(package)? {
        let Some(xml) = package.part_str(&part)? else {
            continue;
        };
        let rewritten = rewrite_formula_elements(&xml, formula)?;
        if rewritten != xml {
            package.replace(&part, rewritten.into_bytes());
        }
    }
    Ok(())
}

/// The parts reachable from `roots` by following relationships.
fn reachable(package: &OpcPackage, roots: &[String]) -> Result<HashSet<String>> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = roots.to_vec();
    while let Some(part) = queue.pop() {
        if !seen.insert(part.clone()) {
            continue;
        }
        let Some(xml) = package.part_str(&jade_opc::rels_part_name(&part))? else {
            continue;
        };
        for relationship in jade_opc::parse_relationships(&xml)?.all() {
            if relationship.external {
                continue;
            }
            let target = jade_opc::resolve_target(&part, &relationship.target);
            if package.part(&target).is_some() && !seen.contains(&target) {
                queue.push(target);
            }
        }
    }
    Ok(seen)
}

/// `xml` with `text` inserted just before the root element's end tag.
fn insert_before_root_end(xml: &str, text: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf)? {
            Event::Start(_) => depth += 1,
            Event::Empty(ref element) if depth == 0 => {
                let name = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                let end = reader.buffer_position() as usize;
                let head = &xml[start..end];
                let open = format!("{}>", head.trim_end_matches("/>").trim_end());
                return Ok(apply(
                    xml,
                    vec![Splice {
                        start,
                        end,
                        text: format!("{open}{text}</{name}>"),
                    }],
                ));
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(apply(
                        xml,
                        vec![Splice {
                            start,
                            end: start,
                            text: text.to_string(),
                        }],
                    ));
                }
            }
            Event::Eof => return Ok(format!("{xml}{text}")),
            _ => {}
        }
        buf.clear();
    }
}

/// Removes `parts` from the package along with their own relationships and
/// content type overrides.
fn remove_parts(package: &mut OpcPackage, parts: &HashSet<String>) -> Result<()> {
    for part in parts {
        package.remove(part);
        package.remove(&jade_opc::rels_part_name(part));
    }
    if let Some(types) = package.part_str("[Content_Types].xml")? {
        let doomed: HashSet<String> = parts.iter().map(|part| format!("/{part}")).collect();
        let types = remove_elements(&types, |element| {
            local(element.name().as_ref()) == b"Override"
                && attribute(element, b"PartName")
                    .ok()
                    .flatten()
                    .is_some_and(|name| doomed.contains(&name))
        })?;
        package.replace("[Content_Types].xml", types.into_bytes());
    }
    Ok(())
}

/// The `docProps/app.xml` part, when the package has one.
fn app_part(package: &OpcPackage) -> Result<Option<String>> {
    let Some(xml) = package.part_str("_rels/.rels")? else {
        return Ok(None);
    };
    Ok(jade_opc::parse_relationships(&xml)?
        .target_by_type_suffix("/extended-properties")
        .map(|target| jade_opc::resolve_target("", target))
        .filter(|part| package.part(part).is_some()))
}

/// What becomes of a title in `docProps/app.xml` that is not a worksheet's.
enum Title {
    Keep,
    Replace(String),
    Remove,
}

/// The sheet a title such as `Notes!Print_Area` belongs to.
fn title_sheet(title: &str) -> Option<(String, &str)> {
    let (sheet, rest) = title.rsplit_once('!')?;
    let sheet = match sheet
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
    {
        Some(inner) => inner.replace("''", "'"),
        None => sheet.to_string(),
    };
    Some((sheet, rest))
}

/// Updates the titles `docProps/app.xml` lists: the worksheet titles become
/// `worksheets`, and every other title goes through `other`. The worksheet
/// titles are found as the heading whose titles are exactly `before` —
/// the headings are localised (`Worksheets`, `ワークシート`), so a name
/// cannot find them — and when none matches the part is left as it was:
/// it is a cache Excel rewrites on save, not something it reads.
fn update_app_titles(
    package: &mut OpcPackage,
    before: &[String],
    worksheets: &[String],
    mut other: impl FnMut(&str) -> Title,
) -> Result<()> {
    let Some(part) = app_part(package)? else {
        return Ok(());
    };
    let Some(xml) = package.part_str(&part)? else {
        return Ok(());
    };
    let Some(app) = AppTitles::parse(&xml)? else {
        return Ok(());
    };
    let mut offset = 0usize;
    let mut worksheet_heading = None;
    for (index, (_, count)) in app.headings.iter().enumerate() {
        if app.titles.get(offset..offset + count) == Some(before) {
            worksheet_heading = Some(index);
            break;
        }
        offset += count;
    }
    let Some(worksheet_heading) = worksheet_heading else {
        return Ok(());
    };

    let mut headings = Vec::new();
    let mut titles = Vec::new();
    let mut offset = 0usize;
    for (index, (heading, count)) in app.headings.iter().enumerate() {
        let slice = &app.titles[offset..offset + count];
        offset += count;
        let kept: Vec<String> = if index == worksheet_heading {
            worksheets.to_vec()
        } else {
            slice
                .iter()
                .filter_map(|title| match other(title) {
                    Title::Keep => Some(title.clone()),
                    Title::Replace(text) => Some(text),
                    Title::Remove => None,
                })
                .collect()
        };
        if kept.is_empty() {
            continue;
        }
        headings.push((heading.clone(), kept.len()));
        titles.extend(kept);
    }
    let rewritten = app.render(&xml, &headings, &titles);
    if rewritten != xml {
        package.replace(&part, rewritten.into_bytes());
    }
    Ok(())
}

/// The `<HeadingPairs>` and `<TitlesOfParts>` of `docProps/app.xml`.
struct AppTitles {
    headings: Vec<(String, usize)>,
    titles: Vec<String>,
    /// The `vt:` prefix the vectors use.
    prefix: String,
    /// The content of `<HeadingPairs>` and of `<TitlesOfParts>`.
    heading_content: (usize, usize),
    title_content: (usize, usize),
}

impl AppTitles {
    fn parse(xml: &str) -> Result<Option<Self>> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut section: Option<(&'static str, usize)> = None;
        let mut heading_content = None;
        let mut title_content = None;
        let mut values: Vec<(Vec<u8>, String)> = Vec::new();
        let mut heading_values = Vec::new();
        let mut text: Option<(Vec<u8>, String)> = None;
        let mut prefix = String::new();
        loop {
            let start = reader.buffer_position() as usize;
            match reader.read_event_into(&mut buf)? {
                Event::Start(ref element) => {
                    let name = element.name().as_ref().to_vec();
                    match local(&name) {
                        b"HeadingPairs" => {
                            section = Some(("headings", reader.buffer_position() as usize))
                        }
                        b"TitlesOfParts" => {
                            section = Some(("titles", reader.buffer_position() as usize))
                        }
                        b"vector" if section.is_some() => prefix = namespace_prefix(&name),
                        b"lpstr" | b"i4" if section.is_some() => {
                            text = Some((local(&name).to_vec(), String::new()))
                        }
                        _ => {}
                    }
                }
                Event::Text(ref value) => {
                    if let Some((_, text)) = text.as_mut() {
                        text.push_str(&value.decode()?);
                    }
                }
                Event::GeneralRef(ref reference) => {
                    if let Some((_, text)) = text.as_mut() {
                        let name = reference.decode()?;
                        if let Ok(decoded) = quick_xml::escape::unescape(&format!("&{name};")) {
                            text.push_str(&decoded);
                        }
                    }
                }
                Event::End(ref element) => match local(element.name().as_ref()) {
                    b"lpstr" | b"i4" => {
                        if let Some(value) = text.take() {
                            values.push(value);
                        }
                    }
                    b"HeadingPairs" => {
                        if let Some((_, content)) = section.take() {
                            heading_content = Some((content, start));
                            heading_values = std::mem::take(&mut values);
                        }
                    }
                    b"TitlesOfParts" => {
                        if let Some((_, content)) = section.take() {
                            title_content = Some((content, start));
                        }
                    }
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        let (Some(heading_content), Some(title_content)) = (heading_content, title_content) else {
            return Ok(None);
        };
        let mut headings = Vec::new();
        for pair in heading_values.chunks(2) {
            match pair {
                [(kind, heading), (count_kind, count)]
                    if kind == b"lpstr" && count_kind == b"i4" =>
                {
                    let Ok(count) = count.trim().parse() else {
                        return Ok(None);
                    };
                    headings.push((heading.clone(), count));
                }
                _ => return Ok(None),
            }
        }
        let titles: Vec<String> = values
            .into_iter()
            .filter(|(kind, _)| kind == b"lpstr")
            .map(|(_, title)| title)
            .collect();
        if headings.iter().map(|(_, count)| count).sum::<usize>() != titles.len() {
            return Ok(None);
        }
        Ok(Some(Self {
            headings,
            titles,
            prefix,
            heading_content,
            title_content,
        }))
    }

    fn render(&self, xml: &str, headings: &[(String, usize)], titles: &[String]) -> String {
        let vt = &self.prefix;
        let mut heading_xml = format!(
            "<{vt}vector size=\"{}\" baseType=\"variant\">",
            headings.len() * 2
        );
        for (heading, count) in headings {
            heading_xml.push_str(&format!(
                "<{vt}variant><{vt}lpstr>{}</{vt}lpstr></{vt}variant><{vt}variant><{vt}i4>{count}</{vt}i4></{vt}variant>",
                escape_text(heading)
            ));
        }
        heading_xml.push_str(&format!("</{vt}vector>"));
        let mut title_xml = format!("<{vt}vector size=\"{}\" baseType=\"lpstr\">", titles.len());
        for title in titles {
            title_xml.push_str(&format!("<{vt}lpstr>{}</{vt}lpstr>", escape_text(title)));
        }
        title_xml.push_str(&format!("</{vt}vector>"));
        apply(
            xml,
            vec![
                Splice {
                    start: self.heading_content.0,
                    end: self.heading_content.1,
                    text: heading_xml,
                },
                Splice {
                    start: self.title_content.0,
                    end: self.title_content.1,
                    text: title_xml,
                },
            ],
        )
    }
}

/// Adds an empty worksheet called `name` after the sheet `after`, or after
/// the last sheet. Returns the new bytes and the new sheet's position in
/// the tab order.
///
/// The worksheet part is new — `sheetN.xml` beside the existing ones — and
/// gets its relationship, content type and `docProps/app.xml` title; the
/// default row height is the first sheet's, so the new sheet looks like the
/// workbook's others. Names scoped to later sheets and the active tab keep
/// pointing at the same sheets.
pub fn add_sheet(original: &[u8], name: &str, after: Option<&str>) -> Result<(Vec<u8>, usize)> {
    let mut parts = WorkbookParts::open(original)?;
    parts.ensure_structure_unlocked()?;
    let names = parts.names();
    validate_sheet_name(name, &names.iter().map(String::as_str).collect::<Vec<_>>())?;
    let at = match after {
        Some(after) => parts.position(after)? + 1,
        None => parts.sheets.len(),
    };
    let before_worksheets = parts.worksheet_names()?;
    let relationships = parts.relationships()?;
    let sheet_parts = parts.sheet_parts()?;

    // The new part, beside the existing worksheets.
    let directory = sheet_parts
        .iter()
        .flatten()
        .find_map(|part| {
            part.rsplit_once('/')
                .map(|(directory, _)| directory.to_string())
        })
        .unwrap_or_else(|| match parts.workbook_part.rsplit_once('/') {
            Some((directory, _)) => format!("{directory}/worksheets"),
            None => "worksheets".to_string(),
        });
    let part_name = (1..)
        .map(|number| format!("{directory}/sheet{number}.xml"))
        .find(|candidate| parts.package.part(candidate).is_none())
        .expect("an unused part name");
    let relationship_id = (1..)
        .map(|number| format!("rId{number}"))
        .find(|candidate| relationships.relationship(candidate).is_none())
        .expect("an unused relationship id");
    let sheet_id = parts
        .sheets
        .iter()
        .filter_map(|sheet| sheet.sheet_id)
        .max()
        .unwrap_or(0)
        + 1;
    let relationship_type = relationships
        .by_type_suffix("/worksheet")
        .first()
        .map(|relationship| relationship.relationship_type.clone())
        .unwrap_or_else(|| WORKSHEET_RELATIONSHIP.to_string());
    let strict = parts.workbook_xml.contains(STRICT_MAIN);
    let (main, relationships_namespace) = if strict {
        (STRICT_MAIN, STRICT_RELATIONSHIPS)
    } else {
        (TRANSITIONAL_MAIN, TRANSITIONAL_RELATIONSHIPS)
    };

    let row_height = sheet_parts
        .iter()
        .flatten()
        .next()
        .map(|part| parts.package.part_str(part))
        .transpose()?
        .flatten()
        .and_then(|xml| default_row_height(&xml))
        .unwrap_or_else(|| "15".to_string());
    let worksheet = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"{main}\" xmlns:r=\"{relationships_namespace}\"><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"{row_height}\"/><sheetData/><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>"
    );
    parts.package.replace(&part_name, worksheet.into_bytes());

    let workbook_directory = parts
        .workbook_part
        .rsplit_once('/')
        .map(|(directory, _)| format!("{directory}/"))
        .unwrap_or_default();
    let target = part_name
        .strip_prefix(&workbook_directory)
        .map(str::to_string)
        .unwrap_or_else(|| format!("/{part_name}"));
    let rels_name = parts.workbook_rels_name();
    let rels = parts.package.part_str(&rels_name)?.unwrap_or_else(|| {
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>"
            .to_string()
    });
    let rels = insert_before_root_end(
        &rels,
        &format!(
            "<Relationship Id=\"{relationship_id}\" Type=\"{}\" Target=\"{}\"/>",
            escape(&relationship_type),
            escape(&target)
        ),
    )?;
    parts.package.replace(&rels_name, rels.into_bytes());
    if let Some(types) = parts.package.part_str("[Content_Types].xml")? {
        let types = insert_before_root_end(
            &types,
            &format!(
                "<Override PartName=\"/{}\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>",
                escape(&part_name)
            ),
        )?;
        parts
            .package
            .replace("[Content_Types].xml", types.into_bytes());
    }

    let prefix = &parts.prefix;
    let id_attribute = match &parts.id_attribute {
        Some(attribute) => format!("{attribute}=\"{relationship_id}\""),
        None => format!("xmlns:r=\"{relationships_namespace}\" r:id=\"{relationship_id}\""),
    };
    let element = format!(
        "<{prefix}sheet name=\"{}\" sheetId=\"{sheet_id}\" {id_attribute}/>",
        escape(&encode_xstring(name))
    );
    let insert_at = parts
        .sheets
        .get(at)
        .map(|sheet| sheet.start)
        .unwrap_or(parts.sheets_content.1);
    let map = |index: usize| Some(if index >= at { index + 1 } else { index });
    let workbook_xml = rewrite_workbook(
        &parts.workbook_xml,
        Some((insert_at, insert_at, element)),
        &Positions {
            map: &map,
            fallback: 0,
        },
        &|formula| formula.to_string(),
    )?;

    let mut worksheets = before_worksheets.clone();
    let worksheet_at = names[..at]
        .iter()
        .filter(|name| before_worksheets.contains(name))
        .count();
    worksheets.insert(worksheet_at, name.to_string());
    update_app_titles(&mut parts.package, &before_worksheets, &worksheets, |_| {
        Title::Keep
    })?;
    Ok((parts.finish(workbook_xml)?, at))
}

/// Move or Copy ▸ Create a copy: the sheet `source` copied, as `name`,
/// right after it — its cells, formats, merges, view and page setup, and
/// its external hyperlinks. A sheet with parts of its own (drawings,
/// charts, tables, comments) is refused rather than copied in part. Returns
/// the package and the copy's position.
pub fn copy_sheet(original: &[u8], source: &str, name: &str) -> Result<(Vec<u8>, usize)> {
    let parts = WorkbookParts::open(original)?;
    let from = parts.position(source)?;
    let source_name = parts.sheets[from].name.clone();
    let source_part = parts.sheet_parts()?[from]
        .clone()
        .ok_or_else(|| WriteError::NoSuchSheet(source.to_string()))?;
    let package = &parts.package;
    let source_rels = package.part_str(&jade_opc::rels_part_name(&source_part))?;
    if let Some(rels) = &source_rels {
        if jade_opc::parse_relationships(rels)?
            .all()
            .iter()
            .any(|relationship| !relationship.external)
        {
            return Err(WriteError::CopySheet(
                "a sheet with drawings, charts, tables or comments cannot be copied yet".into(),
            ));
        }
    }
    let xml = package.required_part_str(&source_part)?;
    let (added, at) = add_sheet(original, name, Some(&source_name))?;
    let mut parts = WorkbookParts::open(&added)?;
    let copy_part = parts.sheet_parts()?[at]
        .clone()
        .ok_or_else(|| WriteError::NoSuchSheet(name.to_string()))?;
    // The copy is not the selected tab.
    parts
        .package
        .replace(&copy_part, rewrite_selected_tabs(&xml)?.into_bytes());
    if let Some(rels) = source_rels {
        parts
            .package
            .replace(&jade_opc::rels_part_name(&copy_part), rels.into_bytes());
    }
    let workbook_xml = parts.workbook_xml.clone();
    Ok((parts.finish(workbook_xml)?, at))
}

/// The `defaultRowHeight` of a worksheet's `sheetFormatPr`, unless it is a
/// custom height a new sheet should not inherit.
fn default_row_height(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"sheetFormatPr" =>
            {
                let custom = attribute(element, b"customHeight")
                    .ok()
                    .flatten()
                    .is_some_and(|value| value == "1" || value == "true");
                return attribute(element, b"defaultRowHeight")
                    .ok()
                    .flatten()
                    .filter(|value| !custom && value.parse::<f64>().is_ok_and(|v| v > 0.0));
            }
            Event::Start(ref element) if local(element.name().as_ref()) == b"sheetData" => {
                return None
            }
            Event::Eof => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// Renames the sheet `old` to `new`, rewriting every formula, defined
/// name, chart series and table formula that names it, a pivot cache's
/// source sheet, and its `docProps/app.xml` titles. Renaming a sheet to
/// exactly the name it has returns `original` unchanged; a change of case
/// alone is a rename.
pub fn rename_sheet(original: &[u8], old: &str, new: &str) -> Result<Vec<u8>> {
    let mut parts = WorkbookParts::open(original)?;
    let at = parts.position(old)?;
    let current = parts.sheets[at].name.clone();
    if current == new {
        return Ok(original.to_vec());
    }
    parts.ensure_structure_unlocked()?;
    let others: Vec<&str> = parts
        .sheets
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != at)
        .map(|(_, sheet)| sheet.name.as_str())
        .collect();
    validate_sheet_name(new, &others)?;
    let before_worksheets = parts.worksheet_names()?;

    let transform = |formula: &str| rename_sheet_in_formula(formula, &current, new);
    rewrite_package_formulas(&mut parts.package, &transform)?;
    let before = jade_xlsx_reader::read_xlsx(original)?;
    crate::pivots::rename_sources(&mut parts.package, &before, &current, new)?;
    crate::pivot_chart::rename_sheet_sources(&mut parts.package, &before, &current, new)?;

    let entry = &parts.sheets[at];
    let element = renamed_sheet_element(&parts.workbook_xml[entry.start..entry.end], new)?;
    let identity = |index: usize| Some(index);
    let workbook_xml = rewrite_workbook(
        &parts.workbook_xml,
        Some((entry.start, entry.end, element)),
        &Positions {
            map: &identity,
            fallback: 0,
        },
        &transform,
    )?;

    let worksheets: Vec<String> = before_worksheets
        .iter()
        .map(|name| {
            if *name == current {
                new.to_string()
            } else {
                name.clone()
            }
        })
        .collect();
    update_app_titles(
        &mut parts.package,
        &before_worksheets,
        &worksheets,
        |title| match title_sheet(title) {
            Some((sheet, rest)) if same_sheet_name(&sheet, &current) => {
                Title::Replace(format!("{}!{rest}", quote_sheet_name(new)))
            }
            _ => Title::Keep,
        },
    )?;
    parts.finish(workbook_xml)
}

/// A `<sheet>` element with its `name` set to `name`.
fn renamed_sheet_element(element_xml: &str, name: &str) -> Result<String> {
    let mut reader = Reader::from_str(element_xml);
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let head_end = reader.buffer_position() as usize;
                let head = rewrite(element, &[("name", Some(encode_xstring(name)))], empty)?;
                return Ok(apply(
                    element_xml,
                    vec![Splice {
                        start,
                        end: head_end,
                        text: head,
                    }],
                ));
            }
            Event::Eof => return Ok(element_xml.to_string()),
            _ => {}
        }
        buf.clear();
    }
}

/// Deletes the sheet `name` with every part only it reached, turning
/// references into it into `#REF!` and removing the names scoped to it.
/// The last visible sheet cannot be deleted.
pub fn delete_sheet(original: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut parts = WorkbookParts::open(original)?;
    let at = parts.position(name)?;
    parts.ensure_structure_unlocked()?;
    let deleted = parts.sheets[at].clone();
    if !parts
        .sheets
        .iter()
        .enumerate()
        .any(|(index, sheet)| index != at && sheet.visible)
    {
        return Err(WriteError::LastVisibleSheet(deleted.name));
    }
    let names = parts.names();
    let before_worksheets = parts.worksheet_names()?;
    let sheet_part = parts.sheet_parts()?[at].clone();

    // The tables whose structured references go with the sheet.
    let mut tables = Vec::new();
    if let Some(sheet_part) = &sheet_part {
        if let Some(rels) = parts
            .package
            .part_str(&jade_opc::rels_part_name(sheet_part))?
        {
            for relationship in jade_opc::parse_relationships(&rels)?.by_type_suffix("/table") {
                let part = jade_opc::resolve_target(sheet_part, &relationship.target);
                if let Some(xml) = parts.package.part_str(&part)? {
                    if let Some(table) = jade_xlsx_reader::table::parse_table(&part, &xml)? {
                        tables.push(table.name.clone());
                        if table.display_name != table.name {
                            tables.push(table.display_name);
                        }
                    }
                }
            }
        }
    }

    // The relationship goes, and with it every part nothing else reaches.
    let rels_name = parts.workbook_rels_name();
    if let Some(id) = &deleted.relationship_id {
        if let Some(rels) = parts.package.part_str(&rels_name)? {
            let rels = remove_elements(&rels, |element| {
                attribute(element, b"Id")
                    .ok()
                    .flatten()
                    .is_some_and(|candidate| candidate == *id)
            })?;
            parts.package.replace(&rels_name, rels.into_bytes());
        }
    }
    if let Some(sheet_part) = &sheet_part {
        let from_sheet = reachable(&parts.package, std::slice::from_ref(sheet_part))?;
        let still = reachable(&parts.package, &[String::new()])?;
        let doomed: HashSet<String> = from_sheet.difference(&still).cloned().collect();
        remove_parts(&mut parts.package, &doomed)?;
    }

    let order: Vec<&str> = names.iter().map(String::as_str).collect();
    let transform =
        |formula: &str| delete_sheet_in_formula(formula, &deleted.name, &order, &tables);
    rewrite_package_formulas(&mut parts.package, &transform)?;

    let visible_after: Vec<bool> = parts
        .sheets
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != at)
        .map(|(_, sheet)| sheet.visible)
        .collect();
    let fallback = (at..visible_after.len())
        .find(|index| visible_after[*index])
        .or_else(|| {
            (0..at.min(visible_after.len()))
                .rev()
                .find(|index| visible_after[*index])
        })
        .unwrap_or(0);
    let map = |index: usize| match index.cmp(&at) {
        std::cmp::Ordering::Less => Some(index),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(index - 1),
    };
    let workbook_xml = rewrite_workbook(
        &parts.workbook_xml,
        Some((deleted.start, deleted.end, String::new())),
        &Positions {
            map: &map,
            fallback,
        },
        &transform,
    )?;
    let workbook_xml = crate::request_full_recalculation(&workbook_xml);
    crate::shift::drop_calc_chain(&mut parts.package, &parts.workbook_part)?;

    let worksheets: Vec<String> = before_worksheets
        .iter()
        .filter(|name| **name != deleted.name)
        .cloned()
        .collect();
    update_app_titles(
        &mut parts.package,
        &before_worksheets,
        &worksheets,
        |title| match title_sheet(title) {
            Some((sheet, _)) if same_sheet_name(&sheet, &deleted.name) => Title::Remove,
            _ => Title::Keep,
        },
    )?;
    parts.finish(workbook_xml)
}

/// Moves the sheet `name` to position `to_index` of the tab order (0 is
/// the first tab). Formulas are untouched — they name sheets — while the
/// names scoped to sheets and the active tab follow their sheets. Moving a
/// sheet to where it is returns `original` unchanged.
pub fn move_sheet(original: &[u8], name: &str, to_index: usize) -> Result<Vec<u8>> {
    let mut parts = WorkbookParts::open(original)?;
    let from = parts.position(name)?;
    let count = parts.sheets.len();
    if to_index >= count {
        return Err(WriteError::SheetPositionOutOfRange {
            index: to_index,
            count,
        });
    }
    if from == to_index {
        return Ok(original.to_vec());
    }
    parts.ensure_structure_unlocked()?;
    let before_worksheets = parts.worksheet_names()?;
    let mut order: Vec<usize> = (0..count).collect();
    let moved = order.remove(from);
    order.insert(to_index, moved);

    let content: String = order
        .iter()
        .map(|index| {
            let sheet = &parts.sheets[*index];
            &parts.workbook_xml[sheet.start..sheet.end]
        })
        .collect();
    let first = parts.sheets.first().map(|sheet| sheet.start).unwrap_or(0);
    let last = parts.sheets.last().map(|sheet| sheet.end).unwrap_or(0);
    let map = |index: usize| order.iter().position(|old| *old == index);
    let workbook_xml = rewrite_workbook(
        &parts.workbook_xml,
        Some((first, last, content)),
        &Positions {
            map: &map,
            fallback: 0,
        },
        &|formula| formula.to_string(),
    )?;

    let worksheets: Vec<String> = order
        .iter()
        .map(|index| parts.sheets[*index].name.clone())
        .filter(|name| before_worksheets.contains(name))
        .collect();
    update_app_titles(&mut parts.package, &before_worksheets, &worksheets, |_| {
        Title::Keep
    })?;
    parts.finish(workbook_xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rels_part_names_the_part_it_describes() {
        assert_eq!(source_of_rels("_rels/.rels").as_deref(), Some(""));
        assert_eq!(
            source_of_rels("xl/_rels/workbook.xml.rels").as_deref(),
            Some("xl/workbook.xml")
        );
        assert_eq!(
            source_of_rels("xl/worksheets/_rels/sheet1.xml.rels").as_deref(),
            Some("xl/worksheets/sheet1.xml")
        );
        assert_eq!(source_of_rels("xl/workbook.xml"), None);
    }

    #[test]
    fn formula_elements_holding_other_elements_are_left_to_their_children() {
        let xml = r#"<x14:dataValidation><x14:formula1><xm:f>Old!$A$1:$A$3</xm:f></x14:formula1></x14:dataValidation><c r="A1"><f>Old!A1&amp;"Old!"</f></c><cfRule><formula>'Old'!B2</formula></cfRule>"#;
        let rewritten = rewrite_formula_elements(xml, &|formula| {
            rename_sheet_in_formula(formula, "Old", "New Name")
        })
        .unwrap();
        assert_eq!(
            rewritten,
            r#"<x14:dataValidation><x14:formula1><xm:f>'New Name'!$A$1:$A$3</xm:f></x14:formula1></x14:dataValidation><c r="A1"><f>'New Name'!A1&amp;"Old!"</f></c><cfRule><formula>'New Name'!B2</formula></cfRule>"#
        );
    }

    #[test]
    fn an_empty_root_gets_its_first_child() {
        assert_eq!(
            insert_before_root_end(r#"<Types a="1"/>"#, "<X/>").unwrap(),
            r#"<Types a="1"><X/></Types>"#
        );
        assert_eq!(
            insert_before_root_end("<Types><A/></Types>", "<X/>").unwrap(),
            "<Types><A/><X/></Types>"
        );
    }
}
