use super::{bad, source::Source, xml, Result};
use crate::{
    patch::{apply, escape, Splice},
    xml_edit, xml_edit_ns,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    pivot::{PivotCache, PivotScalar, PivotValue},
    CellValue,
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Missing,
    Number(u64),
    Text(String),
    Bool(bool),
    Error(String),
    Date(String),
}
pub(super) fn key(value: &PivotScalar) -> Option<Key> {
    Some(match value {
        PivotScalar::Missing => Key::Missing,
        PivotScalar::Number(n) if n.is_finite() => Key::Number(if *n == 0. {
            0
        } else {
            format!("{n:.14e}").parse::<f64>().ok()?.to_bits()
        }),
        PivotScalar::Text(s) => Key::Text(s.to_uppercase()),
        PivotScalar::Bool(b) => Key::Bool(*b),
        PivotScalar::Error(e) => Key::Error(e.clone()),
        PivotScalar::Date(d) => Key::Date(d.trim_end_matches('Z').into()),
        _ => return None,
    })
}
fn index_key(value: &PivotScalar, system: jade_xlsx_reader::DateSystem) -> Option<Key> {
    if let PivotScalar::Date(raw) = value {
        let raw = raw.strip_suffix('Z').unwrap_or(raw);
        let (date, time) = raw.split_once('T').unwrap_or((raw, "00:00:00"));
        let parts = date
            .split('-')
            .map(str::parse::<u32>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .ok()?;
        if parts.len() != 3
            || !jade_xlsx_layout::number_format::is_calendar_date_valid(
                parts[0],
                parts[1],
                parts[2],
                jade_xlsx_reader::DateSystem::Nineteen04,
            )
        {
            return None;
        }
        let time = time.split(':').collect::<Vec<_>>();
        if time.len() != 3 || !time[2].chars().all(|c| c.is_ascii_digit() || c == '.') {
            return None;
        }
        let h = time[0].parse::<u32>().ok()?;
        let m = time[1].parse::<u32>().ok()?;
        let s = time[2].parse::<f64>().ok()?;
        if h > 23 || m > 59 || !(0.0..60.).contains(&s) {
            return None;
        }
        let serial =
            jade_xlsx_layout::number_format::calendar_serial(parts[0], parts[1], parts[2], system)?
                + (f64::from(h * 3600 + m * 60) + s) / 86400.;
        return Some(Key::Date(format!("{serial:.14e}")));
    }
    key(value)
}
fn scalar(value: &CellValue, date: bool, system: jade_xlsx_reader::DateSystem) -> PivotScalar {
    match value {
        CellValue::Empty => PivotScalar::Missing,
        CellValue::Text(s) => PivotScalar::Text(s.clone()),
        CellValue::Bool(b) => PivotScalar::Bool(*b),
        CellValue::Error(e) => PivotScalar::Error(e.clone()),
        CellValue::Number(n) => {
            if date {
                if let Some([y, m, d, h, min, s]) =
                    jade_xlsx_layout::number_format::calendar_fields(*n, system)
                {
                    // Excel's fictional 1900 leap day cannot be an XML Schema date.
                    // Retain its numeric serial/format rather than write invalid d.
                    if !(y == 1900 && m == 2 && d == 29) {
                        let base = n.floor() + f64::from(h * 3600 + min * 60 + s) / 86400.;
                        let nanos = (((n - base) * 86400.) * 1_000_000_000.)
                            .floor()
                            .clamp(0., 999_999_999.) as u32;
                        let frac = if nanos > 0 {
                            format!(".{nanos:09}").trim_end_matches('0').to_string()
                        } else {
                            String::new()
                        };
                        return PivotScalar::Date(format!(
                            "{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}{frac}"
                        ));
                    }
                }
            }
            PivotScalar::Number(*n)
        }
    }
}
pub(super) struct Rebuilt {
    pub fields: Vec<Vec<PivotValue>>,
    pub rows: Vec<Vec<u32>>,
    pub active: Vec<BTreeSet<u32>>,
    pub labels: Vec<Vec<String>>,
    pub native: Vec<Vec<CellValue>>,
}
pub(super) fn rebuild(cache: &PivotCache, source: &Source) -> Result<Rebuilt> {
    let mut fields = cache
        .fields
        .iter()
        .map(|f| f.shared_items.clone())
        .collect::<Vec<_>>();
    let mut item_count = fields.iter().map(Vec::len).sum::<usize>();
    let item_budget = jade_xlsx_reader::pivot::MAX_CATALOG_VALUES
        .saturating_sub(source.rows.len().saturating_mul(fields.len()));
    if item_count > item_budget {
        return Err(bad(
            "retained cache indices exceed the refresh value budget",
        ));
    }
    let mut indexes = fields
        .iter()
        .map(|items| {
            let mut map = BTreeMap::new();
            for (i, v) in items.iter().enumerate() {
                let k = index_key(&v.value, source.date_system)
                    .ok_or_else(|| bad("unknown shared cache value cannot be refreshed"))?;
                map.entry(k).or_insert(i as u32);
            }
            Ok(map)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut active = vec![BTreeSet::new(); fields.len()];
    let mut rows = Vec::with_capacity(source.rows.len());
    let mut originals = BTreeMap::new();
    for (row_index, row) in source.rows.iter().enumerate() {
        let mut record = Vec::with_capacity(fields.len());
        for (i, v) in row.iter().enumerate() {
            let value = scalar(v, source.dates[row_index][i], source.date_system);
            let k = index_key(&value, source.date_system)
                .ok_or_else(|| bad("uninterpreted source value"))?;
            let index = if let Some(old) = indexes[i].get(&k) {
                *old
            } else {
                if item_count >= item_budget {
                    return Err(bad("refreshed cache exceeds the interpreted value budget"));
                }
                item_count += 1;
                let index =
                    u32::try_from(fields[i].len()).map_err(|_| bad("too many cache members"))?;
                fields[i].push(PivotValue {
                    value,
                    unused: false,
                    caption: None,
                    attributes: Default::default(),
                });
                indexes[i].insert(k, index);
                index
            };
            originals.entry((i, index)).or_insert_with(|| v.clone());
            active[i].insert(index);
            record.push(index);
        }
        rows.push(record);
    }
    if fields
        .iter()
        .map(Vec::len)
        .sum::<usize>()
        .saturating_add(rows.len().saturating_mul(fields.len()))
        > jade_xlsx_reader::pivot::MAX_CATALOG_VALUES
    {
        return Err(bad("refreshed cache exceeds the interpreted value budget"));
    }
    for (i, items) in fields.iter_mut().enumerate() {
        for (j, v) in items.iter_mut().enumerate() {
            v.unused = !active[i].contains(&(j as u32));
        }
    }
    let labels = fields
        .iter()
        .enumerate()
        .map(|(i, items)| {
            items
                .iter()
                .map(|v| {
                    if let Some(c) = &v.caption {
                        return c.clone();
                    }
                    match &v.value {
                        PivotScalar::Missing => "(blank)".into(),
                        PivotScalar::Text(s) | PivotScalar::Error(s) => s.clone(),
                        PivotScalar::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
                        PivotScalar::Number(n) => {
                            jade_xlsx_layout::number_format::format_value(
                                &CellValue::Number(*n),
                                source.formats[i].as_deref(),
                                source.date_system,
                            )
                            .text
                        }
                        PivotScalar::Date(d) => d.replace('T', " "),
                        _ => String::new(),
                    }
                })
                .collect()
        })
        .collect();
    let native = fields
        .iter()
        .enumerate()
        .map(|(i, items)| {
            items
                .iter()
                .enumerate()
                .map(|(j, v)| match &v.value {
                    PivotScalar::Missing => CellValue::Text("(blank)".into()),
                    PivotScalar::Text(s) => CellValue::Text(s.clone()),
                    PivotScalar::Number(n) => CellValue::Number(*n),
                    PivotScalar::Bool(b) => CellValue::Bool(*b),
                    PivotScalar::Error(e) => CellValue::Error(e.clone()),
                    PivotScalar::Date(_) => originals
                        .get(&(i, j as u32))
                        .cloned()
                        .unwrap_or(CellValue::Empty),
                    _ => CellValue::Empty,
                })
                .collect()
        })
        .collect();
    Ok(Rebuilt {
        native,
        fields,
        rows,
        active,
        labels,
    })
}
fn value_xml(value: &PivotValue, p: &str) -> Result<String> {
    let (tag, v) = match &value.value {
        PivotScalar::Missing => ("m", None),
        PivotScalar::Number(n) => ("n", Some(n.to_string())),
        PivotScalar::Text(s) => ("s", Some(xml::xstring_attribute(s))),
        PivotScalar::Bool(b) => ("b", Some(u8::from(*b).to_string())),
        PivotScalar::Error(e) => ("e", Some(e.clone())),
        PivotScalar::Date(d) => ("d", Some(d.clone())),
        _ => return Err(bad("cannot write an uninterpreted cache value")),
    };
    let mut attrs = value
        .attributes
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "v" | "u" | "c"))
        .map(|(k, v)| format!(" {k}=\"{}\"", escape(v)))
        .collect::<String>();
    if let Some(v) = v {
        attrs.push_str(&format!(" v=\"{}\"", escape(&v)));
    }
    if let Some(c) = &value.caption {
        attrs.push_str(&format!(" c=\"{}\"", xml::text_attr(c)));
    }
    if value.unused {
        attrs.push_str(" u=\"1\"");
    }
    Ok(format!("<{p}{tag}{attrs}/>"))
}
fn shared(xml: &str, values: &[PivotValue]) -> Result<String> {
    let p = xml::prefix(xml)?;
    let mut old = ["m", "n", "s", "b", "d", "e"]
        .iter()
        .map(|tag| xml_edit_ns::children(xml, tag.as_bytes()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    old.sort_by_key(|s| s.0);
    if old.len() > values.len() {
        return Err(bad("shared cache values lost their positions"));
    }
    let body = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if let Some((_, _, c)) = old.get(i) {
                let updated =
                    xml_edit::root_attributes(&c.text, &[("u", v.unused.then(|| "1".into()))])?;
                c.finish(&updated)
            } else {
                value_xml(v, &p)
            }
        })
        .collect::<Result<Vec<_>>>()?
        .join("");
    let updated = if old.is_empty() {
        xml_edit::append_child(xml, &body)?
    } else {
        apply(
            xml,
            old.into_iter()
                .enumerate()
                .map(|(i, (start, end, _))| Splice {
                    start,
                    end,
                    text: if i == 0 { body.clone() } else { String::new() },
                })
                .collect(),
        )
    };
    let types = values
        .iter()
        .filter_map(|v| key(&v.value))
        .map(|v| match v {
            Key::Missing => 0,
            Key::Number(_) => 1,
            Key::Text(_) => 2,
            Key::Bool(_) => 3,
            Key::Error(_) => 4,
            Key::Date(_) => 5,
        })
        .filter(|i| *i != 0)
        .collect::<BTreeSet<_>>();
    let flag = |test: bool| Some(u8::from(test).to_string());
    xml_edit::root_attributes(
        &updated,
        &[
            ("count", Some(values.len().to_string())),
            (
                "containsBlank",
                flag(
                    values
                        .iter()
                        .any(|v| matches!(v.value, PivotScalar::Missing)),
                ),
            ),
            ("containsDate", flag(types.contains(&5))),
            ("containsNumber", flag(types.contains(&1))),
            ("containsString", flag(types.contains(&2))),
            ("containsNonDate", flag(types.iter().any(|i| *i != 5))),
            ("containsMixedTypes", flag(types.len() > 1)),
            ("containsSemiMixedTypes", flag(types.contains(&2))),
            (
                "containsInteger",
                flag(
                    types == BTreeSet::from([1])
                        && values
                            .iter()
                            .all(|v| !matches!(v.value,PivotScalar::Number(n) if n.fract()!=0.)),
                ),
            ),
            (
                "longText",
                flag(values.iter().any(
                    |v| matches!(&v.value,PivotScalar::Text(s) if s.encode_utf16().count()>255),
                )),
            ),
            ("minValue", None),
            ("maxValue", None),
            ("minDate", None),
            ("maxDate", None),
        ],
    )
}
pub(super) fn write(package: &mut OpcPackage, cache: &PivotCache, new: &Rebuilt) -> Result<()> {
    let mut xml = package.required_part_str(&cache.part_name)?;
    let container = xml::child(&xml, "cacheFields")?.ok_or_else(|| bad("missing cacheFields"))?;
    let children = xml_edit_ns::children(&container.text, b"cacheField")?;
    if children.len() != new.fields.len() {
        return Err(bad("cache field ownership mismatch"));
    }
    let mut splices = Vec::new();
    for (i, (start, end, c)) in children.into_iter().enumerate() {
        let existing = xml::child(&c.text, "sharedItems")?;
        let base = existing
            .as_ref()
            .map(|x| x.text.clone())
            .unwrap_or(format!("<{}sharedItems/>", xml::prefix(&c.text)?));
        let values = shared(&base, &new.fields[i])?;
        let values = if let Some(e) = existing {
            e.finish(&values)?
        } else {
            values
        };
        let changed = xml_edit_ns::set_child(
            &c.text,
            b"sharedItems",
            |_| values,
            &[b"fieldGroup", b"mpMap", b"extLst"],
        )?;
        splices.push(Splice {
            start,
            end,
            text: c.finish(&changed)?,
        });
    }
    let fields = container.finish(&apply(&container.text, splices))?;
    xml = xml_edit_ns::set_child(&xml, b"cacheFields", |_| fields, &[])?;
    let records_part = records_part(package, cache, &mut xml)?;
    let old = package.part_str(&records_part)?;
    let strict = xml::strict(&xml)?;
    let base = old.unwrap_or_else(|| {
        format!(
            "<pivotCacheRecords xmlns=\"{}\"/>",
            if strict {
                "http://purl.oclc.org/ooxml/spreadsheetml/main"
            } else {
                "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
            }
        )
    });
    xml::flat_entries(
        &base,
        "r",
        &[],
        &["x", "s", "n", "d", "e", "b", "m"],
        &["v"],
    )?;
    let p = xml::prefix(&base)?;
    let rows = new
        .rows
        .iter()
        .map(|r| {
            format!(
                "<{p}r>{}</{p}r>",
                r.iter()
                    .map(|i| format!("<{p}x v=\"{i}\"/>"))
                    .collect::<String>()
            )
        })
        .collect::<String>();
    let records = xml::replace_entries(&base, "r", &rows)?;
    let records =
        xml_edit::root_attributes(&records, &[("count", Some(new.rows.len().to_string()))])?;
    package.replace(&records_part, records.into_bytes());
    xml = xml_edit::root_attributes(
        &xml,
        &[
            ("recordCount", Some(new.rows.len().to_string())),
            ("refreshOnLoad", Some("0".into())),
            ("invalid", Some("0".into())),
            ("saveData", Some("1".into())),
        ],
    )?;
    package.replace(&cache.part_name, xml.into_bytes());
    Ok(())
}
fn records_part(
    package: &mut OpcPackage,
    cache: &PivotCache,
    definition: &mut String,
) -> Result<String> {
    if let Some(p) = &cache.records_part {
        return Ok(p.clone());
    }
    let parent = cache.part_name.rsplit_once('/').map_or("", |(p, _)| p);
    let mut i = 1;
    let part = loop {
        let p = format!("{parent}/jade-records{i}.xml");
        if package.part(&p).is_none() {
            break p;
        }
        i += 1;
    };
    let relpart = jade_opc::rels_part_name(&cache.part_name);
    let mut rels=package.part_str(&relpart)?.unwrap_or_else(||"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into());
    let strict = xml::strict(definition)?;
    let ns = if strict {
        "http://purl.oclc.org/ooxml/officeDocument/relationships"
    } else {
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    };
    let old_id = cache.attributes.get("@relationship");
    let mut used = BTreeSet::new();
    let mut existing = None;
    for (start, end, c) in xml_edit_ns::children(&rels, b"Relationship")? {
        let id = xml::attr(&c.text, "Id")?.unwrap_or_default();
        if !used.insert(id.clone()) {
            return Err(bad("duplicate cache relationship identity"));
        }
        if old_id == Some(&id) {
            if !matches!(
                xml::attr(&c.text, "TargetMode")?.as_deref(),
                None | Some("Internal")
            ) || xml::attr(&c.text, "Type")?.as_deref()
                != Some(format!("{ns}/pivotCacheRecords").as_str())
            {
                return Err(bad("cache records relationship is not owned/internal"));
            }
            if let Some(target) = xml::attr(&c.text, "Target")? {
                let resolved = jade_opc::resolve_target(&cache.part_name, &target);
                if package.part(&resolved).is_some() {
                    return Err(bad(
                        "uninterpreted existing cache records cannot be replaced",
                    ));
                }
            }
            let updated = xml_edit::root_attributes(
                &c.text,
                &[("Target", Some(format!("jade-records{i}.xml")))],
            )?;
            existing = Some((
                id,
                Splice {
                    start,
                    end,
                    text: c.finish(&updated)?,
                },
            ));
        }
    }
    let id = if let Some((id, splice)) = existing {
        rels = apply(&rels, vec![splice]);
        id
    } else {
        if old_id.is_some() {
            return Err(bad("declared cache records relationship is missing"));
        }
        let mut n = 1;
        let id = loop {
            let id = format!("jadeRecords{n}");
            if !used.contains(&id) {
                break id;
            }
            n += 1;
        };
        let p = xml::prefix(&rels)?;
        rels=xml_edit::append_child(&rels,&format!("<{p}Relationship Id=\"{id}\" Type=\"{ns}/pivotCacheRecords\" Target=\"jade-records{i}.xml\"/>"))?;
        *definition = xml_edit::root_attributes(
            definition,
            &[
                ("xmlns:jaderel", Some(ns.into())),
                ("jaderel:id", Some(id.clone())),
            ],
        )?;
        id
    };
    let _ = id;
    package.replace(&relpart, rels.into_bytes());
    let types = package.required_part_str("[Content_Types].xml")?;
    let p = xml::prefix(&types)?;
    let types=xml_edit::append_child(&types,&format!("<{p}Override PartName=\"/{}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml\"/>",escape(&part)))?;
    package.replace("[Content_Types].xml", types.into_bytes());
    Ok(part)
}
