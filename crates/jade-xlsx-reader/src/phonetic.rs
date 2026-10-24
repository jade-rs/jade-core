//! Read saved phonetic sort keys without changing cell values or package bytes.
use crate::{CellRange, CellRef, OpcPackage, Result};
use quick_xml::{
    events::{BytesStart, Event},
    name::{QName, ResolveResult},
    NsReader,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneticReading {
    pub base: String,
    pub key: Option<String>,
    pub issue: Option<String>,
}
#[derive(Default)]
struct Item {
    base: String,
    runs: Vec<(Option<usize>, Option<usize>, String)>,
}
impl Item {
    fn finish(self) -> PhoneticReading {
        let base = crate::xstring::decode_xstring(&self.base);
        let bad = || PhoneticReading {
            base: base.clone(),
            key: None,
            issue: Some("invalid phonetic guide offsets or text".into()),
        };
        if self.runs.is_empty() {
            return PhoneticReading {
                base,
                key: None,
                issue: None,
            };
        }
        let units: Vec<_> = base.encode_utf16().collect();
        let mut end = 0;
        let mut key = String::new();
        for (start, stop, reading) in self.runs {
            let (Some(start), Some(stop)) = (start, stop) else {
                return bad();
            };
            let reading = crate::xstring::decode_xstring(&reading);
            if start < end || start >= stop || stop > units.len() || reading.is_empty() {
                return bad();
            }
            let Ok(before) = String::from_utf16(&units[end..start]) else {
                return bad();
            };
            if String::from_utf16(&units[start..stop]).is_err() {
                return bad();
            }
            key.push_str(&before);
            key.push_str(&reading);
            end = stop;
        }
        let Ok(tail) = String::from_utf16(&units[end..]) else {
            return bad();
        };
        key.push_str(&tail);
        PhoneticReading {
            base,
            key: Some(key),
            issue: None,
        }
    }
}
fn attr(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attr in element.attributes() {
        let attr = attr?;
        if attr.key.as_ref() == key {
            return Ok(Some(attr.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
fn namespace(ns: ResolveResult<'_>) -> Option<Vec<u8>> {
    match ns {
        ResolveResult::Unbound => Some(vec![]),
        ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
        _ => None,
    }
}
fn accepted(ns: &Option<Vec<u8>>) -> bool {
    ns.as_ref().is_some_and(|ns| {
        ns.is_empty()
            || matches!(
                ns.as_slice(),
                b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                    | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
            )
    })
}
enum Select<'a> {
    Shared(&'a BTreeSet<usize>),
    Inline(CellRange, &'a BTreeSet<u32>),
}
#[derive(Default)]
struct Scanned {
    items: BTreeMap<usize, PhoneticReading>,
    inline: BTreeMap<CellRef, PhoneticReading>,
    shared: BTreeMap<CellRef, usize>,
}
#[derive(Clone, Copy)]
enum Text {
    Base,
    Guide(usize),
    Index,
}
fn scan(xml: &str, select: Select<'_>) -> Result<Scanned> {
    let content = match &select {
        Select::Shared(_) => crate::cell_content::shared_strings(xml)?,
        Select::Inline(_, _) => crate::cell_content::worksheet(xml)?,
    };
    let mut reader = NsReader::from_str(&content);
    let mut stack = Vec::<(String, bool)>::new();
    let mut root_ns = None;
    let mut next_row = 0;
    let mut row = 0;
    let mut column = 0;
    let mut at = None;
    let mut cell_type = None;
    let mut item = None::<(usize, Item)>;
    let mut item_index = 0;
    let mut item_cell = None;
    let mut text = None::<(usize, Text)>;
    let mut raw_index = String::new();
    let mut result = Scanned::default();
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name();
                let (ns, local) = reader.resolver().resolve_element(name);
                let ns = namespace(ns);
                let local = String::from_utf8_lossy(local.as_ref()).into_owned();
                if stack.is_empty() {
                    root_ns = ns;
                }
                let owned = namespace(reader.resolver().resolve_element(name).0) == root_ns
                    && accepted(&root_ns);
                if owned
                    && stack.len() == 2
                    && stack[1] == ("sheetData".into(), true)
                    && local == "row"
                {
                    row = attr(element, b"r")?
                        .and_then(|v| v.parse::<u32>().ok())
                        .and_then(|v| v.checked_sub(1))
                        .unwrap_or(next_row);
                    next_row = row.saturating_add(1);
                    column = 0;
                }
                if owned
                    && stack.len() == 3
                    && stack[1] == ("sheetData".into(), true)
                    && stack[2] == ("row".into(), true)
                    && local == "c"
                {
                    let parsed = attr(element, b"r")?
                        .and_then(|v| CellRef::parse(&v).ok())
                        .unwrap_or(CellRef::new(column, row));
                    column = parsed.column.saturating_add(1);
                    at = match &select {
                        Select::Inline(range, columns)
                            if range.contains(parsed) && columns.contains(&parsed.column) =>
                        {
                            Some(parsed)
                        }
                        _ => None,
                    };
                    cell_type = attr(element, b"t")?;
                    raw_index.clear();
                }
                let selected_shared =
                    owned && stack.len() == 1 && stack[0].0 == "sst" && local == "si";
                if selected_shared {
                    let index = item_index;
                    item_index += 1;
                    let keep = matches!(&select,Select::Shared(needed) if needed.contains(&index));
                    if !keep {
                        if !empty {
                            reader.read_to_end(QName(name.as_ref()))?;
                        }
                        continue;
                    }
                    item_index = index + 1;
                    item = Some((stack.len(), Item::default()));
                    item_cell = None;
                    if empty {
                        result.items.insert(index, Item::default().finish());
                        item = None;
                    }
                } else if owned
                    && local == "is"
                    && stack.len() == 4
                    && stack[3] == ("c".into(), true)
                    && at.is_some()
                    && cell_type.as_deref() == Some("inlineStr")
                {
                    item = Some((stack.len(), Item::default()));
                    item_cell = at;
                    if empty {
                        item = None;
                        item_cell = None;
                    }
                } else if let Some((depth, item)) = &mut item {
                    if owned && local == "rPh" && stack.len() == *depth + 1 {
                        item.runs.push((
                            attr(element, b"sb")?.and_then(|v| v.parse().ok()),
                            attr(element, b"eb")?.and_then(|v| v.parse().ok()),
                            String::new(),
                        ));
                    } else if owned && local == "t" && !empty {
                        let slot = if stack.len() == *depth + 1
                            || (stack.len() == *depth + 2
                                && stack.last() == Some(&("r".into(), true)))
                        {
                            Some(Text::Base)
                        } else if stack.len() == *depth + 2
                            && stack.last() == Some(&("rPh".into(), true))
                        {
                            item.runs.len().checked_sub(1).map(Text::Guide)
                        } else {
                            None
                        };
                        text = slot.map(|slot| (stack.len() + 1, slot));
                    }
                } else if owned
                    && local == "v"
                    && !empty
                    && stack.len() == 4
                    && stack[3] == ("c".into(), true)
                    && at.is_some()
                    && cell_type.as_deref() == Some("s")
                {
                    text = Some((stack.len() + 1, Text::Index));
                }
                if !empty {
                    stack.push((local, owned));
                } else if local == "c" {
                    at = None;
                }
            }
            Event::Text(ref value) => {
                if let Some((depth, slot)) = text.filter(|(depth, _)| *depth == stack.len()) {
                    let _ = depth;
                    let value = value.decode()?;
                    match slot {
                        Text::Base => {
                            if let Some((_, item)) = &mut item {
                                item.base.push_str(&value);
                            }
                        }
                        Text::Guide(index) => {
                            if let Some((_, item)) = &mut item {
                                item.runs[index].2.push_str(&value);
                            }
                        }
                        Text::Index => raw_index.push_str(&value),
                    }
                }
            }
            Event::CData(ref value) => {
                if let Some((depth, slot)) = text.filter(|(depth, _)| *depth == stack.len()) {
                    let _ = depth;
                    let value = value.decode()?;
                    match slot {
                        Text::Base => {
                            if let Some((_, item)) = &mut item {
                                item.base.push_str(&value);
                            }
                        }
                        Text::Guide(index) => {
                            if let Some((_, item)) = &mut item {
                                item.runs[index].2.push_str(&value);
                            }
                        }
                        Text::Index => raw_index.push_str(&value),
                    }
                }
            }
            Event::GeneralRef(ref value) => {
                if let Some((_, slot)) = text.filter(|(depth, _)| *depth == stack.len()) {
                    let decoded = quick_xml::escape::unescape(&format!("&{};", value.decode()?))
                        .map_err(quick_xml::Error::from)?
                        .into_owned();
                    match slot {
                        Text::Base => {
                            if let Some((_, item)) = &mut item {
                                item.base.push_str(&decoded);
                            }
                        }
                        Text::Guide(index) => {
                            if let Some((_, item)) = &mut item {
                                item.runs[index].2.push_str(&decoded);
                            }
                        }
                        Text::Index => raw_index.push_str(&decoded),
                    }
                }
            }
            Event::End(_) => {
                if item
                    .as_ref()
                    .is_some_and(|(depth, _)| stack.len() == *depth + 1)
                {
                    let (_, parsed) = item.take().unwrap();
                    if let Some(at) = item_cell {
                        result.inline.insert(at, parsed.finish());
                    } else {
                        result.items.insert(item_index - 1, parsed.finish());
                    }
                }
                if stack.last() == Some(&("c".into(), true)) {
                    if cell_type.as_deref() == Some("s") {
                        if let (Some(at), Ok(index)) = (at, raw_index.trim().parse()) {
                            result.shared.insert(at, index);
                        }
                    }
                    at = None;
                }
                if text.is_some_and(|(depth, _)| depth == stack.len()) {
                    text = None;
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(result)
}
/// Only selected key cells are materialized. Shared-string indexes resolve via
/// workbook relationships, including Strict and relocated workbooks.
pub fn sort_readings(
    bytes: &[u8],
    sheet_part: &str,
    range: CellRange,
    columns: &BTreeSet<u32>,
) -> Result<BTreeMap<CellRef, PhoneticReading>> {
    package_readings(&OpcPackage::open(bytes)?, sheet_part, range, columns)
}

/// [`sort_readings`] in a package already open.
pub fn package_readings(
    package: &OpcPackage,
    sheet_part: &str,
    range: CellRange,
    columns: &BTreeSet<u32>,
) -> Result<BTreeMap<CellRef, PhoneticReading>> {
    let sheet = package.required_part_str(sheet_part)?;
    let mut scanned = scan(&sheet, Select::Inline(range, columns))?;
    if scanned.shared.is_empty() {
        return Ok(scanned.inline);
    }
    let workbook = package
        .part_str("_rels/.rels")?
        .map(|xml| jade_opc::parse_relationships(&xml))
        .transpose()?
        .and_then(|rels| {
            rels.target_by_type_suffix("/officeDocument")
                .map(|target| jade_opc::resolve_target("", target))
        })
        .unwrap_or_else(|| "xl/workbook.xml".into());
    let rels = package
        .part_str(&jade_opc::rels_part_name(&workbook))?
        .map(|xml| jade_opc::parse_relationships(&xml))
        .transpose()?
        .unwrap_or_default();
    if let Some(target) = rels.target_by_type_suffix("/sharedStrings") {
        let part = jade_opc::resolve_target(&workbook, target);
        let needed = scanned.shared.values().copied().collect();
        let shared = scan(&package.required_part_str(&part)?, Select::Shared(&needed))?;
        for (at, index) in scanned.shared {
            if let Some(reading) = shared.items.get(&index) {
                scanned.inline.insert(at, reading.clone());
            }
        }
    }
    Ok(scanned.inline)
}

#[cfg(test)]
mod tests {
    use super::*;
    const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    fn shared(body: &str) -> BTreeMap<usize, PhoneticReading> {
        let needed = BTreeSet::from([0, 1, 2]);
        scan(
            &format!("<sst xmlns=\"{NS}\">{body}</sst>"),
            Select::Shared(&needed),
        )
        .unwrap()
        .items
    }
    #[test]
    fn partial_guides_preserve_base_gaps_and_decode_xstrings_entities_and_cdata() {
        let items = shared(
            r#"<si><r><t>東京 &amp; </t></r><r><t>大阪_x0021_</t></r><rPh sb="0" eb="2"><t><![CDATA[トウキョウ]]></t></rPh><rPh sb="5" eb="7"><t>オオサカ</t></rPh></si>"#,
        );
        assert_eq!(items[&0].base, "東京 & 大阪!");
        assert_eq!(items[&0].key.as_deref(), Some("トウキョウ & オオサカ!"));
    }
    #[test]
    fn utf16_offsets_do_not_split_surrogate_pairs() {
        let items = shared(
            r#"<si><t>😀東京A</t><rPh sb="2" eb="4"><t>トウキョウ</t></rPh></si><si><t>😀東京</t><rPh sb="1" eb="3"><t>bad</t></rPh></si>"#,
        );
        assert_eq!(items[&0].key.as_deref(), Some("😀トウキョウA"));
        assert!(items[&1].issue.is_some());
    }
    #[test]
    fn overlapping_outside_missing_and_empty_guide_ranges_are_reported() {
        for guides in [
            r#"<rPh sb="0" eb="2"><t>aa</t></rPh><rPh sb="1" eb="2"><t>bb</t></rPh>"#,
            r#"<rPh sb="0" eb="9"><t>aa</t></rPh>"#,
            r#"<rPh eb="2"><t>aa</t></rPh>"#,
            r#"<rPh sb="0" eb="2"><t/></rPh>"#,
        ] {
            assert!(shared(&format!("<si><t>東京</t>{guides}</si>"))[&0]
                .issue
                .is_some());
        }
    }
    #[test]
    fn foreign_runs_and_foreign_parents_do_not_supply_readings() {
        let items = shared(
            r#"<si xmlns:f="urn:test"><t>東京</t><f:rPh sb="0" eb="2"><t>foreign</t></f:rPh><f:parent><rPh sb="0" eb="2"><t>foreign</t></rPh></f:parent><rPh sb="0" eb="2"><f:t>foreign</f:t><t>トウキョウ</t></rPh></si>"#,
        );
        assert_eq!(items[&0].base, "東京");
        assert_eq!(items[&0].key.as_deref(), Some("トウキョウ"));
    }
    #[test]
    fn omitted_sheet_cell_indexes_and_strict_namespace_are_supported() {
        let range = CellRange::parse("A1:C4").unwrap();
        let columns = BTreeSet::from([0]);
        let xml = r#"<s:worksheet xmlns:s="http://purl.oclc.org/ooxml/spreadsheetml/main"><s:sheetData><s:row><s:c t="inlineStr"><s:is><s:t>東京</s:t><s:rPh sb="0" eb="2"><s:t>トウキョウ</s:t></s:rPh></s:is></s:c><s:c t="inlineStr"><s:is><s:t>ignored</s:t></s:is></s:c></s:row><s:row><s:c t="s"><s:v>2</s:v></s:c></s:row></s:sheetData></s:worksheet>"#;
        let parsed = scan(xml, Select::Inline(range, &columns)).unwrap();
        assert_eq!(
            parsed.inline[&CellRef::new(0, 0)].key.as_deref(),
            Some("トウキョウ")
        );
        assert_eq!(parsed.shared[&CellRef::new(0, 1)], 2);
        assert_eq!(parsed.inline.len(), 1);
    }
}
