//! Source-preserving worksheet protection and authenticated removal.
use crate::{xml_edit, xml_edit_ns, Result, WriteError};
use jade_opc::{protection::PasswordVerifier, OpcPackage};
use jade_xlsx_reader::SheetProtection;
use quick_xml::{events::Event, name::ResolveResult, NsReader, Reader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetProtectionInfo {
    pub protected: bool,
    pub has_password: bool,
    pub allowed: SheetProtection,
}
impl SheetProtectionInfo {
    pub fn configured(self) -> bool {
        self.protected || self.has_password || !self.allowed.objects || !self.allowed.scenarios
    }
}
fn bad(message: &str) -> WriteError {
    WriteError::SheetProtection(message.into())
}
fn root(xml: &str) -> Result<()> {
    let mut r = NsReader::from_str(xml);
    let mut depth = 0;
    let mut found = false;
    let mut alternate = false;
    loop {
        match r.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let ns = r.resolver().resolve_element(e.name()).0;
                let owned = match &ns {
                    ResolveResult::Unbound => true,
                    ResolveResult::Bound(ns) => matches!(
                        ns.as_ref(),
                        b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                            | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                    ),
                    _ => false,
                };
                let local = jade_opc::local_name(e.name().as_ref()).to_vec();
                if depth == 0 {
                    if !owned || local != b"worksheet" {
                        return Err(bad("not a SpreadsheetML worksheet"));
                    }
                    found = true;
                }
                if depth == 1
                    && local == b"AlternateContent"
                    && matches!(&ns, ResolveResult::Bound(ns) if ns.as_ref() == b"http://schemas.openxmlformats.org/markup-compatibility/2006")
                {
                    alternate = !empty;
                }
                if alternate && depth > 1 && owned && local == b"sheetProtection" {
                    return Err(bad("protection inside AlternateContent cannot be edited"));
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 1 {
                    alternate = false;
                }
            }
            Event::Eof => {
                return if found {
                    Ok(())
                } else {
                    Err(bad("missing worksheet root"))
                }
            }
            _ => {}
        }
    }
}
fn record(xml: &str) -> Result<Option<(SheetProtectionInfo, PasswordVerifier)>> {
    root(xml)?;
    let records = xml_edit_ns::children(xml, b"sheetProtection")?;
    if records.len() > 1 {
        return Err(bad("multiple protection records cannot be edited"));
    }
    let Some((_, _, context)) = records.into_iter().next() else {
        return Ok(None);
    };
    let mut r = Reader::from_str(&context.text);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let mut password = PasswordVerifier::default();
                for a in e.attributes() {
                    let a = a?;
                    let value = a.unescape_value()?.into_owned();
                    match a.key.as_ref() {
                        b"password" => password.legacy = Some(value),
                        b"algorithmName" => password.algorithm = Some(value),
                        b"hashValue" => password.hash = Some(value),
                        b"saltValue" => password.salt = Some(value),
                        b"spinCount" => password.spin_count = Some(value),
                        _ => {}
                    }
                }
                let (protected, allowed) = jade_xlsx_reader::sheet_protection::flags(&e)?;
                return Ok(Some((
                    SheetProtectionInfo {
                        protected,
                        has_password: password.is_present(),
                        allowed,
                    },
                    password,
                )));
            }
            Event::Eof => return Err(bad("missing protection record")),
            _ => {}
        }
    }
}
const FOLLOWING: &[&[u8]] = &[
    b"protectedRanges",
    b"scenarios",
    b"autoFilter",
    b"sortState",
    b"dataConsolidate",
    b"customSheetViews",
    b"mergeCells",
    b"phoneticPr",
    b"conditionalFormatting",
    b"dataValidations",
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
fn open(original: &[u8], part: &str) -> Result<(OpcPackage, String)> {
    let p = OpcPackage::open(original)?;
    let xml = p.required_part_str(part)?;
    Ok((p, xml))
}
pub fn sheet_protection(original: &[u8], part: &str) -> Result<Option<SheetProtectionInfo>> {
    let (_, xml) = open(original, part)?;
    Ok(record(&xml)?.map(|r| r.0))
}
fn edit(original: &[u8], part: &str, updates: &[(&str, Option<String>)]) -> Result<Vec<u8>> {
    let (mut p, xml) = open(original, part)?;
    let updated = xml_edit_ns::edit_child(&xml, b"sheetProtection", FOLLOWING, |child| {
        xml_edit::root_attributes(child, updates)
    })?;
    if updated == xml {
        return Ok(original.to_vec());
    }
    p.replace(part, updated.into_bytes());
    Ok(p.write()?)
}
/// Protect a worksheet. The caller supplies a fresh CSPRNG salt for a nonempty password.
pub fn protect_sheet(
    original: &[u8],
    part: &str,
    allowed: SheetProtection,
    password: &str,
    salt: &[u8; 16],
) -> Result<Vec<u8>> {
    if sheet_protection(original, part)?.is_some_and(|r| r.configured()) {
        return Err(bad(
            "unprotect the existing sheet before changing its password",
        ));
    }
    let p = if password.is_empty() {
        PasswordVerifier::default()
    } else {
        PasswordVerifier::new(password, salt)?
    };
    let mut updates = vec![
        ("sheet", Some("1".into())),
        ("password", None),
        ("algorithmName", p.algorithm),
        ("hashValue", p.hash),
        ("saltValue", p.salt),
        ("spinCount", p.spin_count),
    ];
    for (key, permits) in [
        ("selectLockedCells", allowed.select_locked_cells),
        ("selectUnlockedCells", allowed.select_unlocked_cells),
        ("formatCells", allowed.format_cells),
        ("formatColumns", allowed.format_columns),
        ("formatRows", allowed.format_rows),
        ("insertColumns", allowed.insert_columns),
        ("insertRows", allowed.insert_rows),
        ("insertHyperlinks", allowed.insert_hyperlinks),
        ("deleteColumns", allowed.delete_columns),
        ("deleteRows", allowed.delete_rows),
        ("sort", allowed.sort),
        ("autoFilter", allowed.auto_filter),
        ("pivotTables", allowed.pivot_tables),
        ("objects", allowed.objects),
        ("scenarios", allowed.scenarios),
    ] {
        updates.push((key, Some(if permits { "0" } else { "1" }.into())));
    }
    edit(original, part, &updates)
}
/// Clear sheet/object/scenario locks and credentials after authentication. Other XML survives.
pub fn unprotect_sheet(original: &[u8], part: &str, password: &str) -> Result<Vec<u8>> {
    let (_, xml) = open(original, part)?;
    let Some((info, verifier)) = record(&xml)? else {
        return Ok(original.to_vec());
    };
    if !info.configured() {
        return Ok(original.to_vec());
    }
    if !verifier.verifies(password)? {
        return Err(WriteError::IncorrectPassword);
    }
    edit(
        original,
        part,
        &[
            ("sheet", None),
            ("objects", None),
            ("scenarios", None),
            ("password", None),
            ("algorithmName", None),
            ("hashValue", None),
            ("saltValue", None),
            ("spinCount", None),
        ],
    )
}
