//! Reconcile worksheet extension coverage with a partial grid edit. Work from
//! the original records: the package-wide formula pass has already visited
//! the current tree, and applying another structural translation doubles it.
use crate::{
    cell_shift::{
        aggregate_rule, children_map, head_attr, leaf_text, rule_formula, rule_pieces, text_content,
    },
    patch::{apply, local, Splice},
    shift::{range_text, reference_text},
    xml_edit, xml_edit_ns, Result,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{shift_cells_formula, CellRange, CellShift};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use std::collections::{BTreeMap, BTreeSet};
const X14: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &[u8] = b"http://schemas.microsoft.com/office/excel/2006/main";
const REVISION: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2014/revision";

pub(crate) struct IdPool {
    used: BTreeSet<String>,
    next: u128,
}
impl IdPool {
    pub fn new(package: &OpcPackage) -> Self {
        let mut pool = Self {
            used: BTreeSet::new(),
            next: 0x6c62272e07bb014262b821756295c58d,
        };
        for name in package.part_names() {
            if !name.ends_with(".xml") && !name.ends_with(".rels") {
                continue;
            }
            if let Some(data) = package.part(name) {
                for byte in name.as_bytes().iter().chain(data) {
                    pool.next =
                        (pool.next ^ u128::from(*byte)).wrapping_mul(0x1000000000000000000013b);
                }
                for candidate in data.windows(38) {
                    if candidate[0] == b'{'
                        && candidate[37] == b'}'
                        && candidate[1..37].iter().enumerate().all(|(i, b)| {
                            if [8, 13, 18, 23].contains(&i) {
                                *b == b'-'
                            } else {
                                b.is_ascii_hexdigit()
                            }
                        })
                    {
                        pool.used
                            .insert(String::from_utf8_lossy(candidate).to_ascii_uppercase());
                    }
                }
            }
        }
        pool
    }
    fn fresh(&mut self) -> String {
        loop {
            // Derive stable identities from the package, with UUID version
            // and variant bits. Always check all XML parts for collisions.
            let n = (self.next & !(0xFu128 << 76) & !(3u128 << 62)) | (4u128 << 76) | (2u128 << 62);
            self.next = self.next.wrapping_add(1);
            let id = format!(
                "{{{:08X}-{:04X}-{:04X}-{:04X}-{:012X}}}",
                n >> 96,
                (n >> 80) & 0xFFFF,
                (n >> 64) & 0xFFFF,
                (n >> 48) & 0xFFFF,
                n & 0xFFFFFFFFFFFF
            );
            if self.used.insert(id.clone()) {
                return id;
            }
        }
    }
}
fn map_ns(
    xml: &str,
    name: &[u8],
    ns: &[u8],
    descend: bool,
    mut f: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    let entries = if descend {
        xml_edit_ns::descendants_ns(xml, name, ns)?
    } else {
        xml_edit_ns::children_ns(xml, name, ns)?
    };
    let mut edits = Vec::new();
    for (start, end, c) in entries {
        let moved = f(&c.text)?;
        if moved == c.text {
            continue;
        }
        let text = if moved.is_empty() {
            moved
        } else {
            c.finish(&moved)?
        };
        edits.push(Splice { start, end, text });
    }
    Ok(apply(xml, edits))
}
fn coverage(xml: &str) -> Result<Vec<CellRange>> {
    Ok(xml_edit_ns::children_ns(xml, b"sqref", XM)?
        .first()
        .map(|(_, _, c)| leaf_text(&c.text))
        .transpose()?
        .unwrap_or_default()
        .split_ascii_whitespace()
        .filter_map(|s| CellRange::parse(s).ok())
        .collect())
}
fn set_coverage(xml: &str, ranges: &[CellRange]) -> Result<String> {
    let text = ranges
        .iter()
        .copied()
        .map(range_text)
        .collect::<Vec<_>>()
        .join(" ");
    map_ns(xml, b"sqref", XM, false, |s| text_content(s, &text))
}
fn formula_map(xml: &str, f: impl Fn(&str) -> String) -> Result<String> {
    map_ns(xml, b"f", XM, true, |s| text_content(s, &f(&leaf_text(s)?)))
}
fn structural(xml: &str, change: &CellShift) -> Result<String> {
    formula_map(xml, |f| shift_cells_formula(f, &change.sheet, change))
}
fn retain_nonempty(xml: String, name: &[u8]) -> Result<String> {
    Ok(if xml_edit_ns::children(&xml, name)?.is_empty() {
        String::new()
    } else {
        xml
    })
}
fn paired_ranges(xml: &str) -> Result<BTreeMap<String, Vec<CellRange>>> {
    let mut pairs = BTreeMap::new();
    for (_, _, cf) in xml_edit_ns::children(xml, b"conditionalFormatting")? {
        let ranges = head_attr(&cf.text, b"sqref")?
            .unwrap_or_default()
            .split_ascii_whitespace()
            .filter_map(|s| CellRange::parse(s).ok())
            .collect::<Vec<_>>();
        for (_, _, rule) in xml_edit_ns::children(&cf.text, b"cfRule")? {
            // IDs are only links in a cfRule's extension subtree, never a
            // same-named opaque foreign element.
            for (_, _, exts) in xml_edit_ns::children(&rule.text, b"extLst")? {
                for (_, _, ext) in xml_edit_ns::children(&exts.text, b"ext")? {
                    for (_, _, id) in xml_edit_ns::children_ns(&ext.text, b"id", X14)? {
                        pairs.insert(leaf_text(&id.text)?.to_ascii_uppercase(), ranges.clone());
                    }
                }
            }
        }
    }
    Ok(pairs)
}
fn move_cf(
    xml: &str,
    change: &CellShift,
    old_pairs: &BTreeMap<String, Vec<CellRange>>,
    pairs: &BTreeMap<String, Vec<CellRange>>,
    ids: &mut IdPool,
) -> Result<String> {
    let moved = children_map(xml, b"conditionalFormatting", |cf| {
        let ranges = coverage(cf)?;
        let Some(base) = ranges.first().map(|r| r.start) else {
            return Ok(cf.into());
        };
        let rules = xml_edit_ns::children(cf, b"cfRule")?;
        let mut result = String::new();
        for (index, (_, _, rule)) in rules.iter().enumerate() {
            let mut i = 0;
            let one = children_map(cf, b"cfRule", |r| {
                let keep = i == index;
                i += 1;
                Ok(if keep { r.into() } else { String::new() })
            })?;
            let id = head_attr(&rule.text, b"id")?.map(|s| s.to_ascii_uppercase());
            let standalone = head_attr(&rule.text, b"priority")?.is_some();
            if let Some(id) = id
                .as_ref()
                .filter(|id| old_pairs.contains_key(*id) && !standalone)
            {
                if let Some(targets) = pairs.get(id) {
                    result.push_str(&structural(&set_coverage(&one, targets)?, change)?);
                }
                continue;
            }
            let relative = !aggregate_rule(&rule.text)?;
            if !relative {
                let targets = ranges
                    .iter()
                    .flat_map(|r| change.map_regions(*r))
                    .collect::<Vec<_>>();
                if !targets.is_empty() {
                    result.push_str(&structural(&set_coverage(&one, &targets)?, change)?);
                }
            } else {
                for (copies, piece) in ranges
                    .iter()
                    .flat_map(|r| rule_pieces(*r, change))
                    .enumerate()
                {
                    if copies >= crate::cell_shift::CELL_SHIFT_LIMIT {
                        return Err(crate::WriteError::CellShift(
                            "too many split rule regions".into(),
                        ));
                    }
                    let mut moved = set_coverage(&one, &[piece.target])?;
                    moved = formula_map(&moved, |f| rule_formula(f, base, piece, change))?;
                    if copies > 0 && id.is_some() {
                        let id = ids.fresh();
                        moved = children_map(&moved, b"cfRule", |r| {
                            xml_edit::root_attributes(r, &[("id", Some(id.clone()))])
                        })?;
                    }
                    result.push_str(&moved);
                }
            }
        }
        Ok(result)
    })?;
    retain_nonempty(moved, b"conditionalFormatting")
}
fn move_dv(xml: &str, change: &CellShift) -> Result<String> {
    let moved = children_map(xml, b"dataValidation", |rule| {
        let ranges = coverage(rule)?;
        let Some(base) = ranges.first().map(|r| r.start) else {
            return Ok(rule.into());
        };
        let mut result = String::new();
        for (copies, piece) in ranges
            .iter()
            .flat_map(|r| rule_pieces(*r, change))
            .enumerate()
        {
            if copies >= crate::cell_shift::CELL_SHIFT_LIMIT {
                return Err(crate::WriteError::CellShift(
                    "too many split rule regions".into(),
                ));
            }
            let moved = set_coverage(rule, &[piece.target])?;
            result.push_str(&crate::validation_sources::map_formulas(
                &moved,
                true,
                &|f| rule_formula(f, base, piece, change),
            )?);
        }
        Ok(result)
    })?;
    let count = xml_edit_ns::children(&moved, b"dataValidation")?.len();
    if count == 0 {
        return Ok(String::new());
    }
    xml_edit::root_attributes(&moved, &[("count", Some(count.to_string()))])
}
fn move_sparklines(xml: &str, change: &CellShift) -> Result<String> {
    let moved = children_map(xml, b"sparklineGroup", |group| {
        let group = children_map(group, b"sparklines", |sparklines| {
            let moved = children_map(sparklines, b"sparkline", |sparkline| {
                let ranges = coverage(sparkline)?;
                let Some(location) = ranges.first().filter(|r| r.start == r.end) else {
                    return Ok(sparkline.into());
                };
                let Some(target) = change.map_cell(location.start) else {
                    return Ok(String::new());
                };
                let moved = map_ns(sparkline, b"sqref", XM, false, |s| {
                    text_content(s, &reference_text(target))
                })?;
                structural(&moved, change)
            })?;
            retain_nonempty(moved, b"sparkline")
        })?;
        if xml_edit_ns::children(&group, b"sparklines")?.is_empty() {
            return Ok(String::new());
        }
        // A group's optional date-axis formula has the same structural
        // semantics as a sparkline source, independent of the display cell.
        map_ns(&group, b"f", XM, false, |f| {
            text_content(
                f,
                &shift_cells_formula(&leaf_text(f)?, &change.sheet, change),
            )
        })
    })?;
    retain_nonempty(moved, b"sparklineGroup")
}

pub(crate) fn move_extensions(
    original: &str,
    current: &str,
    change: &CellShift,
    ids: &mut IdPool,
) -> Result<String> {
    let old_pairs = paired_ranges(original)?;
    let pairs = paired_ranges(current)?;
    let old_exts = xml_edit_ns::children(original, b"extLst")?;
    let mut result = current.to_owned();
    if let Some((_, _, old)) = old_exts.first() {
        result = children_map(&result, b"extLst", |exts| {
            children_map(exts, b"ext", |ext| {
                let uri = head_attr(ext, b"uri")?;
                let mut source = None;
                for (_, _, candidate) in xml_edit_ns::children(&old.text, b"ext")? {
                    if head_attr(&candidate.text, b"uri")? == uri {
                        source = Some(candidate);
                        break;
                    }
                }
                let Some(source) = source else {
                    return Ok(ext.into());
                };
                let mut moved = ext.to_owned();
                for name in [
                    b"conditionalFormattings".as_slice(),
                    b"dataValidations",
                    b"sparklineGroups",
                ] {
                    let originals = xml_edit_ns::children_ns(&source.text, name, X14)?;
                    let mut index = 0;
                    moved = map_ns(&moved, name, X14, false, |_| {
                        let Some((_, _, source)) = originals.get(index) else {
                            return Ok(String::new());
                        };
                        index += 1;
                        let changed = match name {
                            b"conditionalFormattings" => {
                                move_cf(&source.text, change, &old_pairs, &pairs, ids)?
                            }
                            b"dataValidations" => move_dv(&source.text, change)?,
                            _ => move_sparklines(&source.text, change)?,
                        };
                        if changed.is_empty() {
                            Ok(changed)
                        } else {
                            source.finish(&changed)
                        }
                    })?;
                }
                Ok(moved)
            })
        })?;
    }
    normalize_priorities_and_uids(&result, ids)
}

// Priority is shared by standard and standalone extended rules. Paired x14
// rules deliberately omit it; adding a priority would change their meaning.
fn normalize_priorities_and_uids(xml: &str, ids: &mut IdPool) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    let mut edits = Vec::new();
    let mut rules = Vec::new();
    let mut seen_uids = BTreeSet::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let end = reader.buffer_position() as usize;
                let (ns, _) = reader.resolver().resolve_element(e.name());
                let known = matches!(ns,ResolveResult::Bound(ref n) if matches!(n.as_ref(),
                    b"http://schemas.openxmlformats.org/spreadsheetml/2006/main" |
                    b"http://purl.oclc.org/ooxml/spreadsheetml/main" |
                    b"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"));
                if !known {
                    continue;
                }
                let mut updates = Vec::new();
                for attr in e.attributes() {
                    let attr = attr?;
                    let value = attr.unescape_value()?.into_owned();
                    if local(e.name().as_ref()) == b"cfRule" && attr.key.as_ref() == b"priority" {
                        if let Ok(priority) = value.parse::<u32>() {
                            rules.push((priority, start, end));
                        }
                    }
                    let (ns, name) = reader.resolver().resolve_attribute(attr.key);
                    if name.as_ref() == b"uid"
                        && matches!(ns,ResolveResult::Bound(n) if n.as_ref()==REVISION)
                        && !seen_uids.insert(value.to_ascii_uppercase())
                    {
                        updates.push((
                            String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                            ids.fresh(),
                        ));
                    }
                }
                if !updates.is_empty() {
                    let attrs = updates
                        .iter()
                        .map(|(k, v)| (k.as_str(), Some(v.clone())))
                        .collect::<Vec<_>>();
                    // root_attributes accepts a detached start tag as an empty element.
                    let head = &xml[start..end];
                    let empty = if head.ends_with("/>") {
                        head.to_owned()
                    } else {
                        format!("{}/>", &head[..head.len() - 1])
                    };
                    let edited = xml_edit::root_attributes(&empty, &attrs)?;
                    let text = if head.ends_with("/>") {
                        edited
                    } else {
                        format!("{}>", &edited[..edited.len() - 2])
                    };
                    edits.push(Splice { start, end, text });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut priorities = BTreeSet::new();
    if rules.iter().any(|(p, _, _)| !priorities.insert(*p)) {
        rules.sort_by_key(|(priority, start, _)| (*priority, *start));
        for (rank, (_, start, end)) in rules.into_iter().enumerate() {
            let old = edits
                .iter()
                .find(|e| e.start == start)
                .map(|e| e.text.clone())
                .unwrap_or_else(|| xml[start..end].into());
            let is_empty = old.ends_with("/>");
            let empty = if is_empty {
                old.clone()
            } else {
                format!("{}/>", &old[..old.len() - 1])
            };
            let edited =
                xml_edit::root_attributes(&empty, &[("priority", Some((rank + 1).to_string()))])?;
            let text = if is_empty {
                edited
            } else {
                format!("{}>", &edited[..edited.len() - 2])
            };
            if let Some(e) = edits.iter_mut().find(|e| e.start == start) {
                e.text = text;
            } else {
                edits.push(Splice { start, end, text });
            }
        }
    }
    Ok(apply(xml, edits))
}
