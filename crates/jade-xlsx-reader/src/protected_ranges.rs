//! Owned standard/x14 editable ranges and their original XML locations.
use crate::{CellRange, CellRef, Result, XlsxError};
use quick_xml::{
    events::{BytesStart, Event},
    name::ResolveResult,
    NsReader,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};
pub const X14: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
pub const XM: &[u8] = b"http://schemas.microsoft.com/office/excel/2006/main";
pub const EXT_URI: &str = "{FC87AEE6-9EDD-4A0A-B7FB-166176984837}";
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangePassword {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spin_count: Option<String>,
}
impl RangePassword {
    pub fn modern(&self) -> bool {
        self.algorithm.is_some()
            || self.hash.is_some()
            || self.salt.is_some()
            || self.spin_count.is_some()
    }
    pub fn requires_password(&self) -> bool {
        self.modern()
            || self
                .legacy
                .as_deref()
                .is_some_and(|s| s.trim_matches([' ', '\t', '\r', '\n']) != "0000")
    }
    pub fn verifier(&self) -> jade_opc::protection::PasswordVerifier {
        jade_opc::protection::PasswordVerifier {
            legacy: self.legacy.clone(),
            algorithm: self.algorithm.clone(),
            hash: self.hash.clone(),
            salt: self.salt.clone(),
            spin_count: self.spin_count.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtectedRange {
    pub name: String,
    pub ranges: Vec<CellRange>,
    pub passwords: Vec<RangePassword>,
    pub security_descriptors: Vec<String>,
    pub ambiguous: bool,
}
impl ProtectedRange {
    pub fn requires_password(&self) -> bool {
        self.ambiguous || self.passwords.iter().any(RangePassword::requires_password)
    }
    pub fn password_verifiers(&self) -> impl Iterator<Item = &RangePassword> {
        let modern = self.passwords.iter().any(RangePassword::modern);
        self.passwords.iter().filter(move |p| {
            if modern {
                p.modern()
            } else {
                p.requires_password()
            }
        })
    }
}
#[derive(Debug, Clone)]
pub struct RangeContainer {
    pub span: Range<usize>,
    pub extended: bool,
}
#[derive(Debug, Clone)]
pub struct RangeRecord {
    pub span: Range<usize>,
    pub container: usize,
    pub value: ProtectedRange,
}
#[derive(Debug, Clone, Default)]
pub struct RangeSources {
    pub unsupported_mce: bool,
    pub containers: Vec<RangeContainer>,
    pub records: Vec<RangeRecord>,
}
fn bad(s: &str) -> XlsxError {
    XlsxError::BadProtectedRange(s.into())
}
fn attr(e: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for a in e.attributes() {
        let a = a?;
        if a.key.as_ref() == key {
            return Ok(Some(a.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
/// XML sqref: local cell rectangles, including absolute markers. No silent partial parsing.
pub fn parse_sqref(text: &str) -> Option<Vec<CellRange>> {
    let ranges = text
        .split([' ', '\t', '\r', '\n'])
        .filter(|s| !s.is_empty())
        .map(|s| {
            let r = CellRange::parse(s).ok()?;
            if r.start.row > r.end.row || r.start.column > r.end.column {
                None
            } else {
                Some(r)
            }
        })
        .collect::<Option<Vec<_>>>()?;
    (!ranges.is_empty()).then_some(ranges)
}
fn begin(e: &BytesStart<'_>) -> Result<ProtectedRange> {
    let name = crate::xstring::decode_xstring(&attr(e, b"name")?.unwrap_or_default());
    let password = RangePassword {
        legacy: attr(e, b"password")?,
        algorithm: attr(e, b"algorithmName")?,
        hash: attr(e, b"hashValue")?,
        salt: attr(e, b"saltValue")?,
        spin_count: attr(e, b"spinCount")?,
    };
    Ok(ProtectedRange {
        name,
        ranges: Vec::new(),
        passwords: vec![password],
        security_descriptors: attr(e, b"securityDescriptor")?.into_iter().collect(),
        ambiguous: false,
    })
}
struct Active {
    start: usize,
    depth: usize,
    container: usize,
    value: ProtectedRange,
    sqrefs: usize,
}
/// Byte spans retain unmodeled records too, so preserving writers never re-index through a filtered list.
pub fn sources(xml: &str) -> Result<RangeSources> {
    let mut r = NsReader::from_str(xml);
    let mut stack: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut main = None;
    let mut out = RangeSources::default();
    let mut container: Option<(usize, usize)> = None;
    let mut active: Option<Active> = None;
    let mut extension = None;
    let mut text: Option<(usize, String)> = None;
    let mut descriptor = None;
    loop {
        let start = r.buffer_position() as usize;
        match r.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let ns = match r.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => Vec::new(),
                    ResolveResult::Bound(n) => n.as_ref().to_vec(),
                    _ => b"?unknown".to_vec(),
                };
                let local = jade_opc::local_name(e.name().as_ref()).to_vec();
                let depth = stack.len();
                if depth == 0 {
                    if local != b"worksheet"
                        || !(ns.is_empty()
                            || matches!(
                                ns.as_slice(),
                                b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                    | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                            ))
                    {
                        return Err(bad("not a SpreadsheetML worksheet"));
                    }
                    main = Some(ns.clone());
                    if !xml.contains("protectedRange") {
                        return Ok(out);
                    }
                }
                let owned = main.as_ref() == Some(&ns);
                if let Some((d, _)) = &text {
                    if depth > *d {
                        if let Some(a) = &mut active {
                            a.value.ambiguous = true;
                        }
                    }
                }
                if matches!(local.as_slice(), b"protectedRange" | b"protectedRanges")
                    && (owned || ns == X14)
                    && stack.iter().any(|(ns, name)| {
                        ns == crate::data_validations::MC_NAMESPACE && name == b"AlternateContent"
                    })
                {
                    out.unsupported_mce = true;
                }

                if depth == 2
                    && owned
                    && local == b"ext"
                    && stack[1].0 == ns
                    && stack[1].1 == b"extLst"
                    && attr(&e, b"uri")?.is_some_and(|uri| uri.eq_ignore_ascii_case(EXT_URI))
                    && !empty
                {
                    extension = Some(depth);
                }
                let standard = depth == 1 && owned && local == b"protectedRanges";
                let extended =
                    depth == 3 && ns == X14 && local == b"protectedRanges" && extension == Some(2);
                if standard || extended {
                    let index = out.containers.len();
                    out.containers.push(RangeContainer {
                        span: start..r.buffer_position() as usize,
                        extended,
                    });
                    if !empty {
                        container = Some((depth, index));
                    }
                } else if let Some((d, index)) = container {
                    let ext = out.containers[index].extended;
                    if depth == d + 1
                        && local == b"protectedRange"
                        && if ext { ns == X14 } else { owned }
                    {
                        let mut value = begin(&e)?;
                        if !ext {
                            value.ranges = attr(&e, b"sqref")?
                                .as_deref()
                                .and_then(parse_sqref)
                                .unwrap_or_default();
                            value.ambiguous = value.ranges.is_empty();
                        }
                        if empty {
                            value.ambiguous |= ext;
                            out.records.push(RangeRecord {
                                span: start..r.buffer_position() as usize,
                                container: index,
                                value,
                            });
                        } else {
                            active = Some(Active {
                                start,
                                depth,
                                container: index,
                                value,
                                sqrefs: 0,
                            });
                        }
                    } else if let Some(a) = &mut active {
                        if depth == a.depth + 1 && ext && ns == XM && local == b"sqref" {
                            a.sqrefs += 1;
                            if empty {
                                a.value.ambiguous = true;
                            } else {
                                text = Some((depth, String::new()));
                            }
                        }
                        if depth == a.depth + 1
                            && ((ext && ns == X14) || (!ext && owned))
                            && local == b"securityDescriptor"
                        {
                            if empty {
                                a.value
                                    .security_descriptors
                                    .push(xml[start..r.buffer_position() as usize].into());
                            } else {
                                descriptor = Some((depth, start));
                            }
                        }
                    }
                }
                if !empty {
                    stack.push((ns, local));
                }
            }
            Event::Text(t) => {
                if let Some((_, value)) = &mut text {
                    value.push_str(&t.xml_content()?);
                }
            }
            Event::CData(t) => {
                if let Some((_, value)) = &mut text {
                    value.push_str(&t.decode()?);
                }
            }
            Event::GeneralRef(t) => {
                if let Some((_, value)) = &mut text {
                    let encoded = format!("&{};", t.decode()?);
                    value.push_str(
                        &quick_xml::escape::unescape(&encoded)
                            .map_err(|_| bad("invalid range reference entity"))?,
                    );
                }
            }
            Event::End(_) => {
                let depth = stack.len().saturating_sub(1);
                if text.as_ref().is_some_and(|(d, _)| *d == depth) {
                    let (_, value) = text.take().unwrap();
                    if let Some(a) = &mut active {
                        a.value.ranges = parse_sqref(&value).unwrap_or_default();
                        a.value.ambiguous |= a.value.ranges.is_empty();
                    }
                }
                if descriptor.is_some_and(|(d, _)| d == depth) {
                    let (_, begin) = descriptor.take().unwrap();
                    if let Some(a) = &mut active {
                        a.value
                            .security_descriptors
                            .push(xml[begin..r.buffer_position() as usize].into());
                    }
                }
                if active.as_ref().is_some_and(|a| a.depth == depth) {
                    let mut a = active.take().unwrap();
                    if out.containers[a.container].extended {
                        a.value.ambiguous |= a.sqrefs != 1;
                    }
                    out.records.push(RangeRecord {
                        span: a.start..r.buffer_position() as usize,
                        container: a.container,
                        value: a.value,
                    });
                }
                if container.is_some_and(|(d, _)| d == depth) {
                    let (_, i) = container.take().unwrap();
                    out.containers[i].span.end = r.buffer_position() as usize;
                }
                if extension == Some(depth) {
                    extension = None;
                }
                stack.pop();
            }
            Event::Eof => {
                if main.is_none() || active.is_some() || container.is_some() {
                    return Err(bad("incomplete range container"));
                }
                return Ok(out);
            }
            _ => {}
        }
    }
}
pub fn subtract(range: CellRange, cut: CellRange) -> Vec<CellRange> {
    let left = range.start.column.max(cut.start.column);
    let right = range.end.column.min(cut.end.column);
    let top = range.start.row.max(cut.start.row);
    let bottom = range.end.row.min(cut.end.row);
    if left > right || top > bottom {
        return vec![range];
    }
    let mut out = Vec::new();
    let mut push = |x1, y1, x2, y2| {
        out.push(CellRange {
            start: CellRef::new(x1, y1),
            end: CellRef::new(x2, y2),
        })
    };
    if range.start.row < top {
        push(
            range.start.column,
            range.start.row,
            range.end.column,
            top - 1,
        )
    }
    if bottom < range.end.row {
        push(
            range.start.column,
            bottom + 1,
            range.end.column,
            range.end.row,
        )
    }
    if range.start.column < left {
        push(range.start.column, top, left - 1, bottom)
    }
    if right < range.end.column {
        push(right + 1, top, range.end.column, bottom)
    }
    out
}
fn covered(a: &[CellRange], b: &[CellRange]) -> bool {
    a.iter().all(|r| {
        let mut remaining = vec![*r];
        for cut in b {
            remaining = remaining
                .into_iter()
                .flat_map(|r| subtract(r, *cut))
                .collect();
            if remaining.is_empty() {
                break;
            }
        }
        remaining.is_empty()
    })
}
/// Merge standard fallback and x14 records by sheet-local name. Modern metadata
/// remains authoritative across the pair; inconsistent/duplicate coverage is unusable.
pub fn protected_ranges(xml: &str) -> Result<Vec<ProtectedRange>> {
    let source = sources(xml)?;
    if source.unsupported_mce {
        return Err(bad(
            "editable ranges inside AlternateContent are unsupported",
        ));
    }
    let mut groups: BTreeMap<String, (ProtectedRange, [bool; 2])> = BTreeMap::new();
    for record in source.records {
        if record.value.name.is_empty() {
            continue;
        }
        let ext = source.containers[record.container].extended as usize;
        let key = record.value.name.to_lowercase();
        if let Some((value, seen)) = groups.get_mut(&key) {
            value.ambiguous |= record.value.ambiguous
                || seen[ext]
                || !covered(&value.ranges, &record.value.ranges)
                || !covered(&record.value.ranges, &value.ranges);
            value.passwords.extend(record.value.passwords);
            value
                .security_descriptors
                .extend(record.value.security_descriptors);
            seen[ext] = true;
        } else {
            let mut seen = [false; 2];
            seen[ext] = true;
            groups.insert(key, (record.value, seen));
        }
    }
    Ok(groups.into_values().map(|(value, _)| value).collect())
}

pub fn protection_stamp(xml: &str) -> Result<String> {
    let mut r = NsReader::from_str(xml);
    let mut main = None;
    let mut stamp = String::new();
    loop {
        let start = r.buffer_position() as usize;
        match r.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let ns = match r.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => Some(Vec::new()),
                    ResolveResult::Bound(n) => Some(n.as_ref().to_vec()),
                    _ => None,
                };
                if main.is_none() {
                    main = ns.clone();
                }
                if ns == main && jade_opc::local_name(e.name().as_ref()) == b"sheetProtection" {
                    if !empty {
                        let name = e.name().as_ref().to_vec();
                        r.read_to_end(quick_xml::name::QName(&name))?;
                    }
                    stamp.push_str(&xml[start..r.buffer_position() as usize]);
                }
            }
            Event::Eof => return Ok(stamp),
            _ => {}
        }
    }
}

pub fn same_coverage(a: &[CellRange], b: &[CellRange]) -> bool {
    covered(a, b) && covered(b, a)
}
