use super::{xml::*, *};
use crate::{xstring::decode_xstring, OpcPackage, Result};
use quick_xml::{events::Event, NsReader};
use std::collections::{BTreeMap, BTreeSet};
fn list<'a>(node: &'a Node, container: &str, child: &str) -> Result<Vec<&'a Node>> {
    match node.optional(container)? {
        Some(n) => {
            let items = n.children(child).collect::<Vec<_>>();
            check_count(n, items.len())?;
            Ok(items)
        }
        None => Ok(Vec::new()),
    }
}
fn check_count(n: &Node, actual: usize) -> Result<()> {
    if n.u32("count")?.is_some_and(|v| v as usize != actual) {
        Err(bad(format!("{} count disagrees with children", n.name)))
    } else {
        Ok(())
    }
}
fn scalar(n: &Node, record: bool) -> Result<PivotValue> {
    let v = n.attr("v");
    let value = match (n.own, n.name.as_str()) {
        (true, "m") => PivotScalar::Missing,
        (true, "s") if v.is_some() => PivotScalar::Text(decode_xstring(v.unwrap())),
        (true, "n") => v
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|n| n.is_finite())
            .map(PivotScalar::Number)
            .unwrap_or_else(|| PivotScalar::Unknown {
                element: n.name.clone(),
                value: v.map(str::to_string),
            }),
        (true, "b") => match v.map(str::trim) {
            Some("1" | "true") => PivotScalar::Bool(true),
            Some("0" | "false") => PivotScalar::Bool(false),
            _ => PivotScalar::Unknown {
                element: n.name.clone(),
                value: v.map(str::to_string),
            },
        },
        (true, "e") if v.is_some() => PivotScalar::Error(v.unwrap().into()),
        (true, "d") if v.is_some() => PivotScalar::Date(v.unwrap().into()),
        (true, "x") if record => PivotScalar::Shared(n.required_u32("v")?),
        _ => PivotScalar::Unknown {
            element: n.name.clone(),
            value: v.map(str::to_string),
        },
    };
    Ok(PivotValue {
        value,
        unused: n.boolean("u", false)?,
        caption: n.attr("c").map(decode_xstring),
        attributes: n.attrs.clone(),
    })
}
fn values(n: &Node) -> Result<Vec<PivotValue>> {
    let result = n
        .children
        .iter()
        .map(|n| scalar(n, false))
        .collect::<Result<Vec<_>>>()?;
    check_count(n, result.len())?;
    Ok(result)
}
fn unknown_children(n: &Node, known: &[&str]) -> Vec<String> {
    n.children
        .iter()
        .filter(|c| !c.own || !known.contains(&c.name.as_str()))
        .map(|c| format!("{}/{}", n.name, c.name))
        .collect()
}
/// Read one definition without resolving any external connection or records.
pub fn parse_cache_definition(id: u32, part_name: &str, xml: &str) -> Result<PivotCache> {
    let root = document(xml, "pivotCacheDefinition", MAIN)?;
    let src = root.required("cacheSource")?;
    let mut source = CacheSource {
        kind: src.text("type")?,
        connection_id: src.u32("connectionId")?,
        ..Default::default()
    };
    if let Some(sheet) = src.optional("worksheetSource")? {
        source.sheet = sheet.attr("sheet").map(decode_xstring);
        source.reference = sheet.attr("ref").map(crate::CellRange::parse).transpose()?;
        source.defined_name = sheet.attr("name").map(decode_xstring);
        source.relationship_id = sheet.attr("@relationship").map(str::to_string);
    }
    if source
        .reference
        .is_some_and(|r| r.start.row > r.end.row || r.start.column > r.end.column)
    {
        return Err(bad("inverted pivot source range"));
    }
    let mut unsupported = unknown_children(&root, &["cacheSource", "cacheFields"]);
    if source.kind != "worksheet" {
        unsupported.push(format!("cache source {}", source.kind));
    }
    root.required("cacheFields")?;
    let mut fields = Vec::new();
    for field in list(&root, "cacheFields", "cacheField")? {
        let grouping = field
            .optional("fieldGroup")?
            .map(|g| -> Result<FieldGrouping> {
                let discrete = match g.optional("discretePr")? {
                    Some(d) => {
                        let values = d
                            .children("x")
                            .map(|n| n.required_u32("v"))
                            .collect::<Result<Vec<_>>>()?;
                        check_count(d, values.len())?;
                        values
                    }
                    None => Vec::new(),
                };
                Ok(FieldGrouping {
                    base: g.u32("base")?,
                    parent: g.u32("par")?,
                    range: g.optional("rangePr")?.map(|r| r.attrs.clone()),
                    discrete,
                    items: g
                        .optional("groupItems")?
                        .map(values)
                        .transpose()?
                        .unwrap_or_default(),
                })
            })
            .transpose()?;
        unsupported.extend(unknown_children(field, &["sharedItems", "fieldGroup"]));
        if field.attr("formula").is_some() {
            unsupported.push(format!("calculated cache field {}", field.text("name")?));
        }
        fields.push(CacheField {
            name: decode_xstring(&field.text("name")?),
            shared_items: field
                .optional("sharedItems")?
                .map(values)
                .transpose()?
                .unwrap_or_default(),
            grouping,
            attributes: field.attrs.clone(),
        });
    }
    if fields.is_empty() {
        return Err(bad("cacheFields must contain a field"));
    }
    Ok(PivotCache {
        id,
        part_name: part_name.into(),
        records_part: None,
        source,
        fields,
        records: None,
        attributes: root.attrs,
        unsupported,
    })
}
/// Cache records are streamed and bounded by actual values, never count attrs.
/// Unknown entries keep their column position; shared indices resolve per field.
pub fn parse_cache_records(xml: &str, field_count: usize) -> Result<PivotRecords> {
    let mut reader = NsReader::from_str(xml);
    let mut stack: Vec<(String, bool)> = Vec::new();
    let mut root_ns = None;
    let mut records = PivotRecords::default();
    let mut row = None;
    let mut value_count = 0usize;
    let mut root_seen = false;
    loop {
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let ns = namespace(&reader, &e);
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if stack.is_empty() {
                    if root_seen
                        || name != "pivotCacheRecords"
                        || !ns
                            .as_ref()
                            .is_some_and(|n| n.is_empty() || MAIN.contains(&n.as_str()))
                    {
                        return Err(bad("invalid cache records root"));
                    }
                    root_seen = true;
                    root_ns = ns.clone();
                    records.declared_count = number(
                        attributes(&reader, &e)?.get("count").map(String::as_str),
                        "count",
                    )?;
                }
                let own = ns.is_some() && ns == root_ns && stack.last().is_none_or(|n| n.1);
                if stack.len() == 1 && !(own && matches!(name.as_str(), "r" | "extLst")) {
                    return Err(bad("unsupported cache records child"));
                }
                if own && stack.len() == 1 && name == "extLst" {
                    records.unsupported.push("pivotCacheRecords/extLst".into());
                }
                if own && stack.len() == 1 && name == "r" {
                    row = Some(Vec::new());
                    if empty {
                        finish_record(&mut records, &mut row, field_count)?;
                    }
                }
                if stack.len() == 2 && stack[1].0 == "r" {
                    value_count += 1;
                    if value_count > 1_000_000 {
                        return Err(bad("pivot record value limit"));
                    }
                    let n = Node {
                        name: name.clone(),
                        own,
                        attrs: attributes(&reader, &e)?,
                        children: Vec::new(),
                    };
                    if let Some(row) = row.as_mut() {
                        row.push(scalar(&n, true)?);
                    }
                }
                if !empty {
                    if stack.len() > 128 {
                        return Err(bad("pivot records depth limit"));
                    }
                    stack.push((name, own));
                }
            }
            Event::End(_) => {
                if let Some((name, own)) = stack.pop() {
                    if stack.len() == 1 && own && name == "r" {
                        finish_record(&mut records, &mut row, field_count)?;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || !stack.is_empty() {
        return Err(bad("incomplete pivot records"));
    }
    if records
        .declared_count
        .is_some_and(|n| n as usize != records.rows.len())
    {
        return Err(bad("pivot record count disagrees with rows"));
    }
    Ok(records)
}
fn finish_record(
    records: &mut PivotRecords,
    row: &mut Option<Vec<PivotValue>>,
    field_count: usize,
) -> Result<()> {
    let row = row.take().ok_or_else(|| bad("missing pivot record"))?;
    if row.len() != field_count {
        return Err(bad("pivot record field count mismatch"));
    }
    if records.rows.len() >= 1_000_000 {
        return Err(bad("pivot record row limit"));
    }
    records.rows.push(row);
    Ok(())
}
fn indices(root: &Node, container: &str) -> Result<Vec<i32>> {
    list(root, container, "field")?
        .into_iter()
        .map(|n| {
            n.i32("x")?
                .filter(|n| *n >= 0 || *n == -2)
                .ok_or_else(|| bad("invalid pivot axis field"))
        })
        .collect()
}
fn axis_items(root: &Node, container: &str) -> Result<Vec<AxisItem>> {
    list(root, container, "i")?
        .into_iter()
        .map(|n| {
            Ok(AxisItem {
                kind: n.attr("t").unwrap_or("data").into(),
                repeat: n.u32("r")?.unwrap_or(0),
                data_index: n.u32("i")?.unwrap_or(0),
                items: n
                    .children("x")
                    .map(|x| Ok(x.u32("v")?.unwrap_or(0)))
                    .collect::<Result<_>>()?,
            })
        })
        .collect()
}
pub fn parse_pivot_table(sheet_part: &str, part_name: &str, xml: &str) -> Result<PivotTable> {
    let root = document(xml, "pivotTableDefinition", MAIN)?;
    let loc = root.required("location")?;
    let reference = crate::CellRange::parse(&loc.text("ref")?)?;
    if reference.start.row > reference.end.row || reference.start.column > reference.end.column {
        return Err(bad("inverted pivot location"));
    }
    let location = PivotLocation {
        reference,
        first_header_row: loc.required_u32("firstHeaderRow")?,
        first_data_row: loc.required_u32("firstDataRow")?,
        first_data_column: loc.required_u32("firstDataCol")?,
        row_page_count: loc.u32("rowPageCount")?,
        column_page_count: loc.u32("colPageCount")?,
    };
    if location.first_header_row > reference.end.row - reference.start.row
        || location.first_data_row > reference.end.row - reference.start.row
        || location.first_data_column > reference.end.column - reference.start.column
    {
        return Err(bad("pivot location offset outside ref"));
    }
    root.required("pivotFields")?;
    let fields = list(&root, "pivotFields", "pivotField")?
        .into_iter()
        .map(|n| {
            Ok(PivotField {
                axis: n.attr("axis").map(str::to_string),
                caption: n.attr("name").map(decode_xstring),
                attributes: n.attrs.clone(),
                items: list(n, "items", "item")?
                    .into_iter()
                    .map(|i| {
                        Ok(PivotItem {
                            shared_index: i.u32("x")?,
                            kind: i.attr("t").unwrap_or("data").into(),
                            hidden: i.boolean("h", false)?,
                            caption: i.attr("n").map(decode_xstring),
                            attributes: i.attrs.clone(),
                        })
                    })
                    .collect::<Result<_>>()?,
            })
        })
        .collect::<Result<_>>()?;
    let page_fields = list(&root, "pageFields", "pageField")?
        .into_iter()
        .map(|n| {
            Ok(PageField {
                field: n.required_u32("fld")?,
                item: n.u32("item")?,
                hierarchy: n.i32("hier")?,
                caption: n.attr("name").map(decode_xstring),
            })
        })
        .collect::<Result<_>>()?;
    let data_fields = list(&root, "dataFields", "dataField")?
        .into_iter()
        .map(|n| {
            Ok(DataField {
                field: n.required_u32("fld")?,
                name: n.attr("name").map(decode_xstring),
                aggregation: n.attr("subtotal").unwrap_or("sum").into(),
                show_as: n.attr("showDataAs").unwrap_or("normal").into(),
                base_field: n.u32("baseField")?,
                base_item: n.u32("baseItem")?,
                number_format_id: n.u32("numFmtId")?,
            })
        })
        .collect::<Result<_>>()?;
    let mut filters = Vec::new();
    for n in list(&root, "filters", "filter")? {
        let mut conditions = Vec::new();
        if let Some(a) = n.optional("autoFilter")? {
            for column in a.children("filterColumn") {
                for c in &column.children {
                    if c.own {
                        conditions.push(FilterCondition {
                            element: c.name.clone(),
                            attributes: c.attrs.clone(),
                        });
                        for leaf in &c.children {
                            if leaf.own {
                                conditions.push(FilterCondition {
                                    element: leaf.name.clone(),
                                    attributes: leaf.attrs.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
        filters.push(PivotFilter {
            attributes: n.attrs.clone(),
            conditions,
        });
    }
    Ok(PivotTable {
        part_name: part_name.into(),
        sheet_part: sheet_part.into(),
        name: decode_xstring(&root.text("name")?),
        cache_id: root.required_u32("cacheId")?,
        location,
        fields,
        row_fields: indices(&root, "rowFields")?,
        column_fields: indices(&root, "colFields")?,
        row_items: axis_items(&root, "rowItems")?,
        column_items: axis_items(&root, "colItems")?,
        page_fields,
        data_fields,
        style: root
            .optional("pivotTableStyleInfo")?
            .map(|n| n.attrs.clone()),
        filters,
        unsupported: unknown_children(
            &root,
            &[
                "location",
                "pivotFields",
                "rowFields",
                "rowItems",
                "colFields",
                "colItems",
                "pageFields",
                "dataFields",
                "pivotTableStyleInfo",
                "filters",
            ],
        ),
        attributes: root.attrs,
    })
}
type RelationshipMap = BTreeMap<String, Option<BTreeMap<String, String>>>;
struct PartRelationships<'a> {
    package: &'a OpcPackage,
    owners: BTreeMap<String, std::result::Result<RelationshipMap, String>>,
}
impl<'a> PartRelationships<'a> {
    fn new(package: &'a OpcPackage) -> Self {
        Self {
            package,
            owners: BTreeMap::new(),
        }
    }
    /// Pivot tables are implicitly owned by a worksheet's typed package
    /// relationships. CT_Worksheet has no pivotTableParts declaration list.
    fn related_ids(&self, owner: &str, kind: &str) -> Result<Vec<String>> {
        let part = jade_opc::rels_part_name(owner);
        if self.package.part(&part).is_none() {
            return Ok(Vec::new());
        }
        let xml = self.package.required_part_str(&part)?;
        let root = document(&xml, "Relationships", PKG)?;
        let mut ids = Vec::new();
        for node in root.children("Relationship") {
            if RELS
                .iter()
                .any(|base| node.attr("Type") == Some(format!("{base}/{kind}").as_str()))
            {
                ids.push(node.text("Id")?);
                if ids.len() > 4096 {
                    return Err(bad("too many pivot relationships"));
                }
            }
        }
        Ok(ids)
    }
    fn related(&mut self, owner: &str, id: &str, kind: &str) -> Result<String> {
        let relationships = self.owners.entry(owner.to_string()).or_insert_with(|| {
            let read = || -> Result<BTreeMap<String, Option<BTreeMap<String, String>>>> {
                let xml = self
                    .package
                    .required_part_str(&jade_opc::rels_part_name(owner))?;
                let root = document(&xml, "Relationships", PKG)?;
                let mut map = BTreeMap::new();
                for node in root.children("Relationship") {
                    if let Some(id) = node.attr("Id") {
                        if map.contains_key(id) {
                            map.insert(id.to_string(), None);
                        } else {
                            map.insert(id.to_string(), Some(node.attrs.clone()));
                        }
                    }
                }
                Ok(map)
            };
            read().map_err(|e| e.to_string())
        });
        let map = relationships.as_ref().map_err(|e| bad(e.clone()))?;
        let attrs = map
            .get(id)
            .and_then(Option::as_ref)
            .ok_or_else(|| bad(format!("missing or duplicate relationship {id}")))?;
        if !RELS.iter().any(|base| {
            attrs.get("Type").map(String::as_str) == Some(format!("{base}/{kind}").as_str())
        }) {
            return Err(bad(format!("relationship {id} is not {kind}")));
        }
        if !matches!(
            attrs.get("TargetMode").map(String::as_str),
            None | Some("Internal")
        ) {
            return Err(bad(format!("external or unsupported relationship {id}")));
        }
        let target = attrs
            .get("Target")
            .ok_or_else(|| bad("missing pivot part target"))?;
        if target.is_empty() || target.contains([':', '\\', '#', '?', '\0']) {
            return Err(bad("unsupported pivot part URI"));
        }
        let mut segments = if target.starts_with('/') {
            Vec::new()
        } else {
            owner
                .rsplit_once('/')
                .map(|(dir, _)| dir.split('/').collect())
                .unwrap_or_default()
        };
        for segment in target.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    if segments.pop().is_none() {
                        return Err(bad("pivot part target escapes package"));
                    }
                }
                s => segments.push(s),
            }
        }
        if segments.is_empty() {
            return Err(bad("empty pivot part target"));
        }
        Ok(segments.join("/"))
    }
}
fn issue(catalog: &mut PivotCatalog, part: &str, error: impl std::fmt::Display) {
    catalog.issues.push(PivotIssue {
        part_name: part.into(),
        reason: error.to_string(),
    });
}
/// Cache declarations belong to the workbook; pivot tables belong to the
/// worksheet through exact typed relationships, as specified by CT_Worksheet.
/// Filename-only orphan parts never enter the view. Bad/ambiguous pivot metadata is reported independently
/// of ordinary cells; the preserving package keeps all unsupported parts.
pub fn read_catalog(
    package: &OpcPackage,
    workbook_part: &str,
    workbook_xml: &str,
    sheets: &[crate::Sheet],
) -> PivotCatalog {
    let mut catalog = PivotCatalog::default();
    let mut values_read = 0usize;
    let mut fields_read = 0usize;
    let mut resolver = PartRelationships::new(package);
    let bindings = match references(workbook_xml, "workbook", "pivotCaches", "pivotCache") {
        Ok(v) => v,
        Err(e) => {
            issue(&mut catalog, workbook_part, e);
            Vec::new()
        }
    };
    let mut ids = BTreeMap::new();
    for binding in &bindings {
        if let Some(id) = binding
            .get("cacheId")
            .and_then(|v| v.trim().parse::<u32>().ok())
        {
            *ids.entry(id).or_insert(0usize) += 1;
        }
    }
    for binding in bindings {
        let mut read = || -> Result<PivotCache> {
            let id = number::<u32>(binding.get("cacheId").map(String::as_str), "cacheId")?
                .ok_or_else(|| bad("missing cacheId"))?;
            if ids.get(&id) != Some(&1) {
                return Err(bad(format!("ambiguous cacheId {id}")));
            }
            let rid = binding
                .get("@relationship")
                .ok_or_else(|| bad("cache missing relationship id"))?;
            let part = resolver.related(workbook_part, rid, "pivotCacheDefinition")?;
            parse_cache_definition(id, &part, &package.required_part_str(&part)?)
        };
        match read() {
            Ok(mut cache) => {
                if cache.fields.len() > 65_536usize.saturating_sub(fields_read) {
                    issue(&mut catalog, workbook_part, "workbook pivot field limit");
                    continue;
                }
                fields_read += cache.fields.len();
                let metadata_values = cache
                    .fields
                    .iter()
                    .map(|f| {
                        f.shared_items.len() + f.grouping.as_ref().map_or(0, |g| g.items.len())
                    })
                    .sum::<usize>();
                if metadata_values > MAX_CATALOG_VALUES.saturating_sub(values_read) {
                    issue(&mut catalog, workbook_part, "workbook pivot value limit");
                    continue;
                }
                values_read += metadata_values;

                if let Some(rid) = cache.attributes.get("@relationship") {
                    match resolver
                        .related(&cache.part_name, rid, "pivotCacheRecords")
                        .and_then(|part| {
                            Ok((
                                part.clone(),
                                parse_cache_records(
                                    &package.required_part_str(&part)?,
                                    cache.fields.len(),
                                )?,
                            ))
                        }) {
                        Ok((part, records)) => {
                            let record_values = records.rows.iter().map(Vec::len).sum::<usize>();
                            if record_values > MAX_CATALOG_VALUES.saturating_sub(values_read) {
                                issue(&mut catalog, &cache.part_name, "workbook pivot value limit");
                                cache.records_part = Some(part);
                                catalog.caches.push(Arc::new(cache));
                                continue;
                            }
                            values_read += record_values;
                            let invalid=records.rows.iter().any(|row|row.iter().enumerate().any(|(field,v)|matches!(v.value,PivotScalar::Shared(i) if i as usize>=cache.fields[field].shared_items.len())));
                            cache.records_part = Some(part);
                            let wrong_count =
                                cache.attributes.get("recordCount").is_some_and(|n| {
                                    n.trim().parse::<usize>().ok() != Some(records.rows.len())
                                });
                            if wrong_count {
                                issue(
                                    &mut catalog,
                                    &cache.part_name,
                                    "cache definition recordCount disagrees with records",
                                );
                            }
                            if invalid || wrong_count {
                                if invalid {
                                    issue(
                                        &mut catalog,
                                        &cache.part_name,
                                        "cache record shared item index out of bounds",
                                    );
                                }
                            } else {
                                cache.records = Some(records);
                            }
                        }
                        Err(e) => issue(&mut catalog, &cache.part_name, e),
                    }
                }
                catalog.caches.push(Arc::new(cache));
            }
            Err(e) => issue(&mut catalog, workbook_part, e),
        }
    }
    let mut table_parts = BTreeMap::new();
    let mut table_count = 0usize;
    for sheet in sheets {
        let declarations = resolver.related_ids(&sheet.part_name, "pivotTable");
        let declarations = match declarations {
            Ok(v) => v,
            Err(e) => {
                issue(&mut catalog, &sheet.part_name, e);
                continue;
            }
        };
        table_count += declarations.len();
        if table_count > 4096 {
            issue(&mut catalog, &sheet.part_name, "workbook pivot table limit");
            continue;
        }
        for declaration in declarations {
            let mut read = || -> Result<PivotTable> {
                let part = resolver.related(&sheet.part_name, &declaration, "pivotTable")?;
                parse_pivot_table(&sheet.part_name, &part, &package.required_part_str(&part)?)
            };
            match read() {
                Ok(table) => {
                    if table.fields.len() > 65_536usize.saturating_sub(fields_read) {
                        issue(&mut catalog, &sheet.part_name, "workbook pivot field limit");
                        continue;
                    }
                    fields_read += table.fields.len();

                    *table_parts.entry(table.part_name.clone()).or_insert(0usize) += 1;
                    if !catalog.caches.iter().any(|c| c.id == table.cache_id) {
                        issue(
                            &mut catalog,
                            &table.part_name,
                            format!("unresolved cacheId {}", table.cache_id),
                        );
                        issue(
                            &mut catalog,
                            workbook_part,
                            "unresolved source cache for an owned pivot table",
                        );
                    }
                    // Workbook cacheId and the table's required cache-part
                    // relationship must identify the same cache. Missing links
                    // in legacy files are tolerated for display and repaired by
                    // a preserving rebuild; an explicit conflicting link must
                    // never silently choose a different source.
                    let links = (|| -> Result<()> {
                        let ids = resolver.related_ids(&table.part_name, "pivotCacheDefinition")?;
                        if ids.is_empty() {
                            return Ok(());
                        }
                        if ids.len() != 1 {
                            return Err(bad("ambiguous pivot table cache relationships"));
                        }
                        let part =
                            resolver.related(&table.part_name, &ids[0], "pivotCacheDefinition")?;
                        if !catalog
                            .caches
                            .iter()
                            .any(|c| c.id == table.cache_id && c.part_name == part)
                        {
                            return Err(bad(
                                "pivot table cache relationship contradicts workbook cacheId",
                            ));
                        }
                        Ok(())
                    })();
                    if let Err(e) = links {
                        issue(&mut catalog, &table.part_name, e);
                        issue(
                            &mut catalog,
                            workbook_part,
                            "unresolved source cache relationship for an owned pivot table",
                        );
                    }
                    catalog.tables.push(Arc::new(table));
                }
                Err(e) => issue(&mut catalog, &sheet.part_name, e),
            }
        }
    }
    let duplicate_parts = table_parts
        .into_iter()
        .filter_map(|(part, n)| (n > 1).then_some(part))
        .collect::<BTreeSet<_>>();
    let ambiguous_owners = catalog
        .tables
        .iter()
        .filter(|t| duplicate_parts.contains(&t.part_name))
        .map(|t| t.sheet_part.clone())
        .collect::<BTreeSet<_>>();
    for owner in ambiguous_owners {
        issue(&mut catalog, &owner, "ambiguous pivot table ownership");
    }
    catalog
        .tables
        .retain(|t| !duplicate_parts.contains(&t.part_name));
    for part in duplicate_parts {
        issue(&mut catalog, &part, "ambiguous pivot table ownership");
    }
    catalog
}
