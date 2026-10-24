//! `xl/workbook.xml` — the sheet list, date epoch, and defined names.

use jade_opc::local_name;
use quick_xml::events::Event;
use quick_xml::{name::ResolveResult, NsReader};

use crate::error::Result;
use crate::model::{CalculationMode, DateSystem, DefinedName, SheetVisibility};

/// A `<sheet>` entry, before its relationship is resolved to a part name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetEntry {
    pub name: String,
    pub relationship_id: Option<String>,
    pub visibility: SheetVisibility,
}

/// What `workbook.xml` says: sheet order, names, and the date epoch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkbookPart {
    /// In workbook order, which is the order Excel shows tabs.
    pub sheets: Vec<SheetEntry>,
    pub defined_names: Vec<DefinedName>,
    pub date_system: DateSystem,
    /// The saved tab from the first workbook view, in workbook order.
    pub active_sheet_index: Option<usize>,
    /// Whether cached formula results must be recalculated when opened.
    pub full_calculation_on_load: bool,
    pub calculation_mode: CalculationMode,
    pub calculate_on_save: Option<bool>,
    /// `workbookProtection/@lockStructure`; password metadata alone does not lock it.
    pub structure_protected: bool,
}

/// Reads `xl/workbook.xml`.
pub fn parse_workbook(xml: &str) -> Result<WorkbookPart> {
    let mut reader = NsReader::from_str(xml);
    // Defined-name formulas can contain significant spaces on either side of
    // an XML entity (for example `"A &amp; B"`). Trimming each text event
    // independently would silently turn that into `"A&B"`.
    reader.config_mut().trim_text(false);
    let mut part = WorkbookPart::default();
    let mut defined_name: Option<DefinedName> = None;
    let mut workbook_view_seen = false;
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut root_namespace: Option<Vec<u8>> = None;
    loop {
        let event = reader.read_event_into(&mut buf)?;
        let event_depth = depth;
        if matches!(event, Event::Start(_)) {
            depth += 1;
        }
        if matches!(event, Event::End(_)) {
            depth = depth.saturating_sub(1);
        }
        if let Event::Start(ref element) | Event::Empty(ref element) = event {
            if event_depth == 0 && local_name(element.name().as_ref()) == b"workbook" {
                root_namespace = match reader.resolver().resolve_element(element.name()).0 {
                    ResolveResult::Unbound => Some(Vec::new()),
                    ResolveResult::Bound(namespace)
                        if matches!(
                            namespace.as_ref(),
                            b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                        ) =>
                    {
                        Some(namespace.as_ref().to_vec())
                    }
                    _ => None,
                };
            }
        }
        let owned_direct = event_depth == 1
            && match &event {
                Event::Start(element) | Event::Empty(element) => {
                    root_namespace.as_ref().is_some_and(|root| {
                        match reader.resolver().resolve_element(element.name()).0 {
                            ResolveResult::Unbound => root.is_empty(),
                            ResolveResult::Bound(namespace) => {
                                namespace.as_ref() == root.as_slice()
                            }
                            _ => false,
                        }
                    })
                }
                _ => false,
            };
        match event {
            Event::Start(ref element) | Event::Empty(ref element)
                if owned_direct && local_name(element.name().as_ref()) == b"workbookProtection" =>
            {
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    if attribute.key.as_ref() == b"lockStructure" {
                        let value = attribute.unescape_value()?;
                        // XML Schema boolean whitespace is collapsed. Do not silently
                        // interpret malformed protection flags as an unlocked workbook.
                        part.structure_protected |=
                            match value.trim_matches([' ', '\t', '\r', '\n']) {
                                "1" | "true" => true,
                                "0" | "false" => false,
                                _ => {
                                    return Err(crate::XlsxError::BadWorkbookProtection(
                                        value.into_owned(),
                                    ))
                                }
                            };
                    }
                }
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if local_name(element.name().as_ref()) == b"workbookPr" =>
            {
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    if local_name(attribute.key.as_ref()) == b"date1904" {
                        let value = attribute.unescape_value()?;
                        if value != "0" && value != "false" {
                            part.date_system = DateSystem::Nineteen04;
                        }
                    }
                }
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if local_name(element.name().as_ref()) == b"workbookView" =>
            {
                if !workbook_view_seen {
                    for attribute in element.attributes() {
                        let attribute = attribute?;
                        if local_name(attribute.key.as_ref()) == b"activeTab" {
                            part.active_sheet_index = attribute.unescape_value()?.parse().ok();
                        }
                    }
                }
                workbook_view_seen = true;
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if owned_direct && local_name(element.name().as_ref()) == b"calcPr" =>
            {
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    let value = attribute.unescape_value()?;
                    match attribute.key.as_ref() {
                        b"fullCalcOnLoad" => {
                            part.full_calculation_on_load = value != "0" && value != "false"
                        }
                        b"calcMode" => {
                            part.calculation_mode =
                                CalculationMode::parse(&value).ok_or_else(|| {
                                    crate::XlsxError::BadCalculationMode(value.into_owned())
                                })?
                        }
                        b"calcOnSave" => {
                            part.calculate_on_save = Some(value != "0" && value != "false")
                        }
                        _ => {}
                    }
                }
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if local_name(element.name().as_ref()) == b"sheet" =>
            {
                let mut name = String::new();
                let mut relationship_id = None;
                let mut visibility = SheetVisibility::Visible;
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    match local_name(attribute.key.as_ref()) {
                        b"name" => {
                            name = crate::xstring::decode_xstring(&attribute.unescape_value()?)
                        }
                        b"id" => relationship_id = Some(attribute.unescape_value()?.into_owned()),
                        b"state" => {
                            visibility = match attribute.unescape_value()?.as_ref() {
                                "hidden" => SheetVisibility::Hidden,
                                "veryHidden" => SheetVisibility::VeryHidden,
                                _ => SheetVisibility::Visible,
                            }
                        }
                        _ => {}
                    }
                }
                part.sheets.push(SheetEntry {
                    name,
                    relationship_id,
                    visibility,
                });
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if local_name(element.name().as_ref()) == b"definedName" =>
            {
                let empty = matches!(event, Event::Empty(_));
                let mut parsed = DefinedName {
                    name: String::new(),
                    formula: String::new(),
                    local_sheet_index: None,
                    hidden: false,
                };
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    let value = attribute.unescape_value()?;
                    match local_name(attribute.key.as_ref()) {
                        b"name" => parsed.name = crate::xstring::decode_xstring(&value),
                        b"localSheetId" => parsed.local_sheet_index = value.parse().ok(),
                        b"hidden" => parsed.hidden = value != "0" && value != "false",
                        _ => {}
                    }
                }
                if empty {
                    part.defined_names.push(parsed);
                } else {
                    defined_name = Some(parsed);
                }
            }
            Event::Text(ref text) if defined_name.is_some() => {
                if let Some(name) = defined_name.as_mut() {
                    name.formula.push_str(&text.decode()?);
                }
            }
            Event::GeneralRef(ref reference) if defined_name.is_some() => {
                if let Some(decoded) = decoded_reference(reference) {
                    if let Some(name) = defined_name.as_mut() {
                        name.formula.push_str(&decoded);
                    }
                }
            }
            Event::End(ref element) if local_name(element.name().as_ref()) == b"definedName" => {
                if let Some(name) = defined_name.take() {
                    part.defined_names.push(name);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(part)
}

fn decoded_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .ok()
        .map(|text| text.into_owned())
}
