use super::{
    super::{
        bad, cache, edit, report, source, xml, PivotLayout, PivotReportLayout, PivotSource,
        PivotUpdate, Result, VerifiedPivotCells,
    },
    package,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    pivot::{CacheField, PivotCache},
    CellRange, CellRef, CellValue, Workbook,
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PivotDestination {
    Existing {
        sheet: String,
        origin: CellRef,
    },
    /// The worksheet and report are created together; failure returns no edit.
    NewSheet {
        name: String,
        after: Option<String>,
        origin: CellRef,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub struct PivotCreate {
    pub name: Option<String>,
    pub source: PivotSource,
    pub destination: PivotDestination,
    /// Optional initial settings. Source and origin are supplied above.
    pub settings: PivotUpdate,
}
#[derive(Debug, Clone)]
pub struct PivotCreateResult {
    pub bytes: Vec<u8>,
    pub name: String,
    pub sheet: String,
    pub cache_id: u32,
    pub reference: CellRange,
    pub source_rows: usize,
}
fn chosen_name(book: &Workbook, requested: Option<&str>) -> Result<String> {
    let used = |name: &str| {
        book.pivots
            .tables
            .iter()
            .any(|t| t.name.to_uppercase() == name.to_uppercase())
    };
    let name = if let Some(name) = requested {
        name.to_string()
    } else {
        (1u32..)
            .map(|n| format!("PivotTable{n}"))
            .find(|n| !used(n))
            .ok_or_else(|| bad("pivot name overflow"))?
    };
    if name.trim().is_empty()
        || name.encode_utf16().count() > 255
        || name.chars().any(char::is_control)
        || used(&name)
    {
        return Err(bad("invalid or duplicate pivot name"));
    }
    Ok(name)
}
fn new_cache(book: &Workbook, source: &PivotSource) -> Result<PivotCache> {
    let mut cache = PivotCache {
        id: 0,
        part_name: String::new(),
        records_part: None,
        source: edit::cache_source(source)?,
        fields: Vec::new(),
        records: None,
        attributes: BTreeMap::new(),
        unsupported: Vec::new(),
    };
    let (sheet, range) = source::location(book, &cache)?;
    if range.start.column > range.end.column
        || range.start.row >= range.end.row
        || range.end.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
        || range.end.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
        || (u64::from(range.end.column - range.start.column) + 1)
            * (u64::from(range.end.row - range.start.row) + 1)
            > 1_000_000
    {
        return Err(bad(
            "pivot source needs a bounded header and at least one data row",
        ));
    }
    let mut headers = BTreeMap::new();
    for row in sheet.rows.iter().filter(|r| r.index == range.start.row) {
        for cell in row
            .cells
            .iter()
            .filter(|c| c.reference.row == range.start.row && range.contains(c.reference))
        {
            if headers.insert(cell.reference.column, &cell.value).is_some() {
                return Err(bad("duplicate pivot header cell"));
            }
        }
    }
    let mut names = BTreeSet::new();
    for column in range.start.column..=range.end.column {
        let Some(CellValue::Text(name)) = headers.get(&column).copied() else {
            return Err(bad("pivot source needs nonempty text headers"));
        };
        if name.trim().is_empty()
            || name.encode_utf16().count() > 32767
            || !names.insert(name.to_uppercase())
        {
            return Err(bad("invalid or duplicate pivot source header"));
        }
        cache.fields.push(CacheField {
            name: name.clone(),
            shared_items: Vec::new(),
            grouping: None,
            attributes: BTreeMap::new(),
        });
    }
    Ok(cache)
}
/// Create a fresh worksheet-backed cache and report. Existing caches/reports
/// remain independent; requested initial options are applied before rendering.
pub fn create_pivot(
    original: &[u8],
    options: &PivotCreate,
    verified: &VerifiedPivotCells,
) -> Result<PivotCreateResult> {
    let original_book = jade_xlsx_reader::read_xlsx(original)?;
    if !original_book.pivots.issues.is_empty() {
        return Err(bad("creating a pivot requires an unambiguous catalog"));
    }
    if options.settings.source.is_some() {
        return Err(bad("initial pivot source belongs in the creation source"));
    }
    let name = chosen_name(&original_book, options.name.as_deref())?;
    let mut cache = new_cache(&original_book, &options.source)?;
    let source = source::read(&original_book, &cache, verified)?;
    let rebuilt = cache::rebuild(&cache, &source)?;
    for (field, items) in cache.fields.iter_mut().zip(&rebuilt.fields) {
        field.shared_items = items.clone();
    }
    let bytes = match &options.destination {
        PivotDestination::Existing { .. } => original.to_vec(),
        PivotDestination::NewSheet { name, after, .. } => {
            crate::sheets::add_sheet(original, name, after.as_deref())?.0
        }
    };
    let book = jade_xlsx_reader::read_xlsx(&bytes)?;
    let (sheet_name, origin) = match &options.destination {
        PivotDestination::Existing { sheet, origin } => (sheet, origin),
        PivotDestination::NewSheet { name, origin, .. } => (name, origin),
    };
    let matches = book
        .sheets
        .iter()
        .filter(|s| jade_xlsx_reader::formula::same_sheet_name(&s.name, sheet_name))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(bad("pivot destination worksheet is missing or ambiguous"));
    }
    let owner = matches[0];
    if owner.protection.as_ref().is_some_and(|p| !p.pivot_tables) {
        return Err(bad("creating pivots is not allowed on the protected sheet"));
    }
    if origin.row >= jade_xlsx_reader::model::MAX_ROW_COUNT - 1
        || origin.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT - 1
    {
        return Err(bad("pivot destination has no room for a report"));
    }
    let mut package = OpcPackage::open(&bytes)?;
    let workbook = crate::workbook_part_name(&package)?;
    let workbook_xml = package.required_part_str(&workbook)?;
    let strict = xml::strict(&workbook_xml)?;
    let main = if strict {
        "http://purl.oclc.org/ooxml/spreadsheetml/main"
    } else {
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    };
    let rel = if strict {
        "http://purl.oclc.org/ooxml/officeDocument/relationships"
    } else {
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    };
    let dir = workbook
        .rsplit_once('/')
        .map_or(String::new(), |(d, _)| format!("{d}/"));
    // Reserve IDs in retained markup too, including non-selected MCE
    // branches. Reserving an ID does not promote its declaration into a cache.
    let mut reserved = book
        .pivots
        .caches
        .iter()
        .map(|c| c.id)
        .collect::<BTreeSet<_>>();
    let mut reader = quick_xml::NsReader::from_str(&workbook_xml);
    loop {
        match reader.read_event()? {
            quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                if e.local_name().as_ref() == b"pivotCache"
                    && matches!(reader.resolver().resolve_element(e.name()).0,quick_xml::name::ResolveResult::Bound(n) if n.as_ref()==main.as_bytes())
                {
                    for a in e.attributes() {
                        let a = a?;
                        if a.key.as_ref() == b"cacheId" {
                            if let Ok(id) = a.unescape_value()?.trim().parse::<u32>() {
                                reserved.insert(id);
                            }
                        }
                    }
                }
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    cache.id = (0u32..)
        .find(|id| !reserved.contains(id))
        .ok_or_else(|| bad("pivot cache identity overflow"))?;
    cache.part_name =
        package::free_part(&package, &format!("{dir}pivotCache/pivotCacheDefinition"))?;
    let records = package::free_part(&package, &format!("{dir}pivotCache/pivotCacheRecords"))?;
    cache.records_part = Some(records.clone());
    let table_part = package::free_part(&package, &format!("{dir}pivotTables/pivotTable"))?;
    let cache_relationship = package::add_relationship(
        &mut package,
        &cache.part_name,
        "pivotCacheRecords",
        &records,
        strict,
    )?;
    let source_attributes = match &cache.source.defined_name {
        Some(name) => format!(
            "name=\"{}\"{}",
            xml::text_attr(name),
            cache
                .source
                .sheet
                .as_deref()
                .map(|s| format!(" sheet=\"{}\"", xml::text_attr(s)))
                .unwrap_or_default()
        ),
        None => format!(
            "sheet=\"{}\" ref=\"{}\"",
            xml::text_attr(cache.source.sheet.as_deref().unwrap()),
            crate::shift::range_text(cache.source.reference.unwrap())
        ),
    };
    let fields = cache
        .fields
        .iter()
        .map(|f| {
            format!(
                "<cacheField name=\"{}\"><sharedItems count=\"0\"/></cacheField>",
                xml::text_attr(&f.name)
            )
        })
        .collect::<String>();
    package.replace(&cache.part_name,format!("<pivotCacheDefinition xmlns=\"{main}\" xmlns:r=\"{rel}\" r:id=\"{cache_relationship}\" recordCount=\"0\" saveData=\"1\" refreshOnLoad=\"0\"><cacheSource type=\"worksheet\"><worksheetSource {source_attributes}/></cacheSource><cacheFields count=\"{}\">{fields}</cacheFields></pivotCacheDefinition>",cache.fields.len()).into_bytes());
    package.replace(
        &records,
        format!("<pivotCacheRecords xmlns=\"{main}\" count=\"0\"/>").into_bytes(),
    );
    for (part, kind) in [
        (&cache.part_name, "pivotCacheDefinition"),
        (&records, "pivotCacheRecords"),
        (&table_part, "pivotTable"),
    ] {
        package::add_type(&mut package, part, kind)?;
    }
    let numeric = (0..cache.fields.len()).find(|i| {
        source
            .rows
            .iter()
            .any(|r| matches!(r[*i], CellValue::Number(_)))
            && source
                .rows
                .iter()
                .all(|r| matches!(r[*i], CellValue::Empty | CellValue::Number(_)))
            && !source.dates.iter().any(|r| r[*i])
    });
    let value_field = numeric.unwrap_or(cache.fields.len() - 1);
    let aggregation = if numeric.is_some() { "sum" } else { "count" };
    let row_field = (0..cache.fields.len()).find(|i| *i != value_field);
    let reference = CellRange {
        start: *origin,
        end: CellRef::new(origin.column + 1, origin.row + 1),
    };
    let definition=format!("<pivotTableDefinition xmlns=\"{main}\" name=\"{}\" cacheId=\"{}\" dataCaption=\"Values\" compact=\"0\" outline=\"0\"><location ref=\"{}\" firstHeaderRow=\"0\" firstDataRow=\"1\" firstDataCol=\"1\"/><pivotFields count=\"{}\">{}</pivotFields><dataFields count=\"1\"><dataField fld=\"{value_field}\" subtotal=\"{aggregation}\" name=\"Total\"/></dataFields><pivotTableStyleInfo name=\"PivotStyleMedium9\" showRowHeaders=\"1\" showColHeaders=\"1\"/></pivotTableDefinition>",xml::text_attr(&name),cache.id,crate::shift::range_text(reference),cache.fields.len(),cache.fields.iter().map(|f|format!("<pivotField compact=\"0\" outline=\"0\"><items count=\"{}\">{}</items></pivotField>",f.shared_items.len(),(0..f.shared_items.len()).map(|i|format!("<item x=\"{i}\"/>")).collect::<String>())).collect::<String>());
    package.replace(&table_part, definition.clone().into_bytes());
    let table =
        jade_xlsx_reader::pivot::parse_pivot_table(&owner.part_name, &table_part, &definition)?;
    let mut update = options.settings.clone();
    let layout = update.layout.get_or_insert(PivotLayout {
        rows: row_field.map(|i| vec![i as i32]).unwrap_or_default(),
        columns: vec![-2],
        pages: Vec::new(),
        report_layout: PivotReportLayout::Tabular,
        origin: Some(*origin),
        row_grand_totals: true,
        column_grand_totals: true,
    });
    if layout.origin.is_some_and(|at| at != *origin) {
        return Err(bad(
            "initial report origin differs from its creation destination",
        ));
    }
    layout.origin = Some(*origin);
    let edited = edit::prepare(&mut package, &book, &table, &cache, &update)?;
    let plan = report::build_new(
        &book,
        &edited.table,
        &edited.cache,
        &rebuilt,
        &source,
        &package.required_part_str(&table_part)?,
    )?;
    report::check_destinations(&book, std::slice::from_ref(&plan), &source)?;
    cache::write(&mut package, &edited.cache, &rebuilt)?;
    report::write(&mut package, &plan)?;
    package::add_relationship(
        &mut package,
        &table_part,
        "pivotCacheDefinition",
        &cache.part_name,
        strict,
    )?;
    package::add_relationship(
        &mut package,
        &owner.part_name,
        "pivotTable",
        &table_part,
        strict,
    )?;
    let id = package::add_relationship(
        &mut package,
        &workbook,
        "pivotCacheDefinition",
        &cache.part_name,
        strict,
    )?;
    package::add_declaration(&mut package, &workbook, cache.id, &id, rel)?;
    let body = package.required_part_str(&workbook)?;
    package.replace(
        &workbook,
        crate::request_full_recalculation(&body).into_bytes(),
    );
    crate::shift::drop_calc_chain(&mut package, &workbook)?;
    crate::chart_cache::refresh_package(&mut package, &crate::VerifiedChartCells::new())?;
    let bytes = package.write()?;
    let after = jade_xlsx_reader::read_xlsx(&bytes)?;
    if !after.pivots.issues.is_empty()
        || after
            .pivots
            .tables
            .iter()
            .filter(|t| t.part_name == table_part && t.cache_id == cache.id)
            .count()
            != 1
        || after
            .pivots
            .caches
            .iter()
            .filter(|c| c.id == cache.id)
            .count()
            != 1
    {
        return Err(bad("created pivot relationship graph is inconsistent"));
    }
    Ok(PivotCreateResult {
        bytes,
        name,
        sheet: owner.name.clone(),
        cache_id: cache.id,
        reference: plan.range,
        source_rows: source.rows.len(),
    })
}
