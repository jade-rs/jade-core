//! Workbook structure protection authoring, authenticated removal and preserving splices.
use crate::{xml_edit, xml_edit_ns, Result, WriteError};
use jade_opc::{protection::PasswordVerifier, OpcPackage};
use quick_xml::{events::Event, name::ResolveResult, NsReader, Reader};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkbookProtectionInfo {
    pub structure: bool,
    pub windows: bool,
    pub has_password: bool,
}
impl WorkbookProtectionInfo {
    pub fn configured(self) -> bool {
        self.structure || self.windows || self.has_password
    }
}
struct Record {
    info: WorkbookProtectionInfo,
    passwords: Vec<PasswordVerifier>,
}
fn modern(p: &PasswordVerifier) -> bool {
    p.algorithm.is_some() || p.hash.is_some() || p.salt.is_some() || p.spin_count.is_some()
}
fn bad(message: &str) -> WriteError {
    WriteError::WorkbookProtection(message.into())
}
fn record(xml: &str) -> Result<Option<Record>> {
    let mut root = NsReader::from_str(xml);
    loop {
        match root.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let owned = match root.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => true,
                    ResolveResult::Bound(ns) => matches!(
                        ns.as_ref(),
                        b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                            | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                    ),
                    _ => false,
                };
                if !owned || jade_opc::local_name(e.name().as_ref()) != b"workbook" {
                    return Err(bad("not a SpreadsheetML workbook"));
                }
                break;
            }
            Event::Eof => return Err(bad("missing workbook root")),
            _ => {}
        }
    }
    let records = xml_edit_ns::children(xml, b"workbookProtection")?;
    if records.len() > 1 {
        return Err(bad("multiple protection records cannot be edited"));
    }
    let Some((_, _, context)) = records.into_iter().next() else {
        return Ok(None);
    };
    let mut reader = Reader::from_str(&context.text);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let mut info = WorkbookProtectionInfo::default();
                let mut password = PasswordVerifier::default();
                let mut historical = PasswordVerifier::default();
                let mut revision = false;
                let mut revision_credentials = false;
                for a in e.attributes() {
                    let a = a?;
                    let value = a.unescape_value()?.into_owned();
                    match a.key.as_ref() {
                        b"lockStructure" | b"lockWindows" | b"lockRevision" => {
                            let flag = match value.trim_matches([' ', '\t', '\r', '\n']) {
                                "1" | "true" => true,
                                "0" | "false" => false,
                                _ => return Err(bad("invalid lock boolean")),
                            };
                            if a.key.as_ref() == b"lockStructure" {
                                info.structure = flag;
                            } else if a.key.as_ref() == b"lockWindows" {
                                info.windows = flag;
                            } else {
                                revision = flag;
                            }
                        }
                        b"workbookPassword" => password.legacy = Some(value),
                        b"workbookAlgorithmName" => password.algorithm = Some(value),
                        b"workbookHashValue" => password.hash = Some(value),
                        b"workbookSaltValue" => password.salt = Some(value),
                        b"workbookSpinCount" => password.spin_count = Some(value),
                        b"algorithmName" => historical.algorithm = Some(value),
                        b"hashValue" => historical.hash = Some(value),
                        b"saltValue" => historical.salt = Some(value),
                        b"spinCount" => historical.spin_count = Some(value),
                        b"revisionsPassword"
                        | b"revisionsAlgorithmName"
                        | b"revisionsHashValue"
                        | b"revisionsSaltValue"
                        | b"revisionsSpinCount" => revision_credentials = true,
                        _ => {}
                    }
                }
                // MS-OE376 also describes generic ISO names. Their role is
                // ambiguous when revision protection shares this record.
                if modern(&historical) && (revision || revision_credentials) {
                    return Err(bad("historical ISO password has ambiguous revision scope"));
                }
                let mut passwords = Vec::new();
                if modern(&password) || modern(&historical) {
                    if modern(&password) {
                        passwords.push(password);
                    }
                    if modern(&historical) {
                        passwords.push(historical);
                    }
                } else if password.is_present() {
                    passwords.push(password);
                }
                info.has_password = !passwords.is_empty();
                return Ok(Some(Record { info, passwords }));
            }
            Event::Eof => return Err(bad("missing protection record")),
            _ => {}
        }
    }
}
fn open(original: &[u8]) -> Result<(OpcPackage, String, String)> {
    let package = OpcPackage::open(original)?;
    let name = crate::workbook_part_name(&package)?;
    let xml = package.required_part_str(&name)?;
    Ok((package, name, xml))
}
pub fn workbook_protection(original: &[u8]) -> Result<Option<WorkbookProtectionInfo>> {
    let (_, _, xml) = open(original)?;
    Ok(record(&xml)?.map(|r| r.info))
}
const FOLLOWING: &[&[u8]] = &[
    b"bookViews",
    b"sheets",
    b"functionGroups",
    b"externalReferences",
    b"definedNames",
    b"calcPr",
    b"oleSize",
    b"customWorkbookViews",
    b"pivotCaches",
    b"smartTagPr",
    b"smartTagTypes",
    b"webPublishing",
    b"fileRecoveryPr",
    b"webPublishObjects",
    b"extLst",
];
fn edit(original: &[u8], updates: &[(&str, Option<String>)]) -> Result<Vec<u8>> {
    let (mut package, name, xml) = open(original)?;
    let updated = xml_edit_ns::edit_child(&xml, b"workbookProtection", FOLLOWING, |child| {
        xml_edit::root_attributes(child, updates)
    })?;
    if updated == xml {
        return Ok(original.to_vec());
    }
    package.replace(&name, updated.into_bytes());
    Ok(package.write()?)
}
/// Protect structure with an optional password. Caller supplies fresh CSPRNG salt.
pub fn protect_workbook(original: &[u8], password: &str, salt: &[u8; 16]) -> Result<Vec<u8>> {
    if workbook_protection(original)?.is_some_and(|r| r.configured()) {
        return Err(bad(
            "unprotect the existing workbook before changing its password",
        ));
    }
    let p = if password.is_empty() {
        PasswordVerifier::default()
    } else {
        PasswordVerifier::new(password, salt)?
    };
    edit(
        original,
        &[
            ("lockStructure", Some("1".into())),
            ("workbookPassword", None),
            ("workbookAlgorithmName", p.algorithm),
            ("workbookHashValue", p.hash),
            ("workbookSaltValue", p.salt),
            ("workbookSpinCount", p.spin_count),
        ],
    )
}
/// Authenticate structure/windows removal; revision and unmodeled attributes survive.
pub fn unprotect_workbook(original: &[u8], password: &str) -> Result<Vec<u8>> {
    let (_, _, xml) = open(original)?;
    let Some(r) = record(&xml)? else {
        return Ok(original.to_vec());
    };
    if !r.info.configured() {
        return Ok(original.to_vec());
    }
    for verifier in &r.passwords {
        if !verifier.verifies(password)? {
            return Err(WriteError::IncorrectPassword);
        }
    }
    edit(
        original,
        &[
            ("lockStructure", None),
            ("lockWindows", None),
            ("workbookPassword", None),
            ("workbookAlgorithmName", None),
            ("workbookHashValue", None),
            ("workbookSaltValue", None),
            ("workbookSpinCount", None),
            ("algorithmName", None),
            ("hashValue", None),
            ("saltValue", None),
            ("spinCount", None),
        ],
    )
}
