//! Preserving edits of the interpreted worksheet-pivot definition. Keep the
//! original workbook as the report/chart identity oracle throughout rebuild.
use super::{aggregate::Aggregate, bad, xml, Result};
use crate::{
    patch::{apply, Splice},
    xml_edit, xml_edit_ns,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    pivot::{CacheSource, DataField, PageField, PivotCache, PivotTable},
    CellRange, CellRef, Workbook,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub enum PivotSource {
    Range { sheet: String, range: CellRange },
    Name { sheet: Option<String>, name: String },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PivotReportLayout {
    Compact,
    Outline,
    Tabular,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PivotFilterSelection {
    All,
    /// A stable shared-cache index, not the ordinal of a pivotField item.
    Item(u32),
    Items(BTreeSet<u32>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotPage {
    pub field: u32,
    pub selection: PivotFilterSelection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotLayout {
    /// -2 is the Values pseudo-field; other indices address cache fields.
    pub rows: Vec<i32>,
    pub columns: Vec<i32>,
    pub pages: Vec<PivotPage>,
    pub report_layout: PivotReportLayout,
    pub origin: Option<CellRef>,
    pub row_grand_totals: bool,
    pub column_grand_totals: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotMeasure {
    /// Retain this original measure's series identity and entry metadata.
    /// None creates a new measure; an original index can occur only once.
    pub existing: Option<u32>,
    pub field: u32,
    pub aggregation: String,
    pub caption: String,
    pub number_format_id: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotMemberFilter {
    pub field: u32,
    pub visible: Option<BTreeSet<u32>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotFieldOptions {
    pub field: u32,
    pub caption: Option<String>,
    pub sort: Option<String>,
    pub subtotals: Option<bool>,
    pub subtotals_at_top: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotStyle {
    pub name: Option<String>,
    pub row_headers: bool,
    pub column_headers: bool,
    pub row_stripes: bool,
    pub column_stripes: bool,
    pub last_column: bool,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PivotUpdate {
    /// Shared-cache changes rebuild every report that uses that cache. The
    /// current source schema must keep the same field headers/order.
    pub source: Option<PivotSource>,
    pub layout: Option<PivotLayout>,
    pub measures: Option<Vec<PivotMeasure>>,
    pub member_filters: Vec<PivotMemberFilter>,
    pub field_options: Vec<PivotFieldOptions>,
    pub style: Option<PivotStyle>,
    pub fit_columns: Option<bool>,
}
pub(super) struct Prepared {
    pub cache: PivotCache,
    pub table: PivotTable,
    pub measure_keys: Vec<u32>,
}
/// Current logical settings for an editor. Preserve the original measure
/// indices when reordering the returned measures. Submit only changed groups
/// so unrelated options retain their original XML/defaults.
pub fn pivot_configuration(book: &Workbook, table: &PivotTable) -> Result<PivotUpdate> {
    let caches = book
        .pivots
        .caches
        .iter()
        .filter(|c| c.id == table.cache_id)
        .collect::<Vec<_>>();
    if caches.len() != 1 {
        return Err(bad("missing or ambiguous pivot source cache"));
    }
    let cache = caches[0];
    if table.fields.len() != cache.fields.len()
        || table
            .data_fields
            .iter()
            .any(|m| m.field as usize >= cache.fields.len())
    {
        return Err(bad("pivot fields do not match their cache"));
    }
    if cache.source.kind != "worksheet"
        || cache.source.relationship_id.is_some()
        || cache.fields.iter().any(|f| f.grouping.is_some())
    {
        return Err(bad(
            "editing this connection-backed or grouped pivot is not implemented",
        ));
    }
    if !table.filters.is_empty() || table.data_fields.iter().any(|f| f.show_as != "normal") {
        return Err(bad(
            "editing these advanced pivot filters/show-as settings is not implemented",
        ));
    }
    let owner = book
        .sheets
        .iter()
        .find(|s| s.part_name == table.sheet_part)
        .ok_or_else(|| bad("missing pivot owner"))?;
    let named = super::report::named_measures(owner, table, cache)?;
    let selected = |field: u32| -> Result<Option<BTreeSet<u32>>> {
        let f = table
            .fields
            .get(field as usize)
            .ok_or_else(|| bad("invalid pivot field"))?;
        let hidden = f.items.iter().any(|i| i.kind == "data" && i.hidden)
            || super::report::flag(&f.attributes, "hideNewItems", false)?;
        Ok(hidden.then(|| {
            f.items
                .iter()
                .filter(|i| i.kind == "data" && !i.hidden)
                .filter_map(|i| i.shared_index)
                .collect()
        }))
    };
    let pages = table
        .page_fields
        .iter()
        .map(|page| {
            Ok(PivotPage {
                field: page.field,
                selection: if let Some(item) = page.item {
                    PivotFilterSelection::Item(item)
                } else if let Some(items) = selected(page.field)? {
                    PivotFilterSelection::Items(items)
                } else {
                    PivotFilterSelection::All
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let compact = super::report::flag(&table.attributes, "compact", true)?;
    let outline = super::report::flag(&table.attributes, "outline", true)?;
    let report_layout = if compact {
        PivotReportLayout::Compact
    } else if outline {
        PivotReportLayout::Outline
    } else {
        PivotReportLayout::Tabular
    };
    let source = if let Some(name) = &cache.source.defined_name {
        PivotSource::Name {
            sheet: cache.source.sheet.clone(),
            name: name.clone(),
        }
    } else {
        PivotSource::Range {
            sheet: cache
                .source
                .sheet
                .clone()
                .ok_or_else(|| bad("missing pivot source sheet"))?,
            range: cache
                .source
                .reference
                .ok_or_else(|| bad("missing pivot source range"))?,
        }
    };
    let mut field_options = Vec::new();
    let mut member_filters = Vec::new();
    for (i, field) in table.fields.iter().enumerate() {
        field_options.push(PivotFieldOptions {
            field: i as u32,
            caption: Some(
                field
                    .caption
                    .clone()
                    .unwrap_or_else(|| cache.fields[i].name.clone()),
            ),
            sort: Some(
                field
                    .attributes
                    .get("sortType")
                    .cloned()
                    .unwrap_or_else(|| "manual".into()),
            ),
            subtotals: Some(super::report::flag(
                &field.attributes,
                "defaultSubtotal",
                true,
            )?),
            subtotals_at_top: Some(super::report::flag(&field.attributes, "subtotalTop", true)?),
        });
        if !pages.iter().any(|p| p.field == i as u32) {
            member_filters.push(PivotMemberFilter {
                field: i as u32,
                visible: selected(i as u32)?,
            });
        }
    }
    let style = table
        .style
        .as_ref()
        .map(|s| {
            Ok::<_, crate::WriteError>(PivotStyle {
                name: s.get("name").cloned(),
                row_headers: super::report::flag(s, "showRowHeaders", true)?,
                column_headers: super::report::flag(s, "showColHeaders", true)?,
                row_stripes: super::report::flag(s, "showRowStripes", false)?,
                column_stripes: super::report::flag(s, "showColStripes", false)?,
                last_column: super::report::flag(s, "showLastColumn", false)?,
            })
        })
        .transpose()?;
    Ok(PivotUpdate {
        source: Some(source),
        layout: Some(PivotLayout {
            rows: table.row_fields.clone(),
            columns: table.column_fields.clone(),
            pages,
            report_layout,
            origin: Some(table.location.reference.start),
            row_grand_totals: super::report::flag(&table.attributes, "rowGrandTotals", true)?,
            column_grand_totals: super::report::flag(&table.attributes, "colGrandTotals", true)?,
        }),
        measures: Some(
            named
                .data_fields
                .iter()
                .enumerate()
                .map(|(i, m)| PivotMeasure {
                    existing: Some(i as u32),
                    field: m.field,
                    aggregation: m.aggregation.clone(),
                    caption: m.name.clone().unwrap(),
                    number_format_id: m.number_format_id,
                })
                .collect(),
        ),
        member_filters,
        field_options,
        style,
        fit_columns: Some(super::report::flag(
            &table.attributes,
            "applyWidthHeightFormats",
            true,
        )?),
    })
}
fn boolean(value: bool) -> Option<String> {
    Some(u8::from(value).to_string())
}
fn selection(
    table: &mut PivotTable,
    cache: &PivotCache,
    field: u32,
    chosen: Option<&BTreeSet<u32>>,
) -> Result<()> {
    let i = field as usize;
    let f = table
        .fields
        .get_mut(i)
        .ok_or_else(|| bad("pivot filter field is outside the cache"))?;
    if chosen.is_some_and(|s| {
        s.iter()
            .any(|n| *n as usize >= cache.fields[i].shared_items.len())
    }) {
        return Err(bad("pivot filter member is outside the shared cache"));
    }
    for item in &mut f.items {
        if item.kind == "data" {
            item.hidden = chosen.is_some_and(|s| item.shared_index.is_none_or(|n| !s.contains(&n)));
        }
    }
    f.attributes.insert(
        "hideNewItems".into(),
        u8::from(chosen.is_some()).to_string(),
    );
    f.attributes.insert(
        "includeNewItemsInFilter".into(),
        u8::from(chosen.is_none()).to_string(),
    );
    Ok(())
}
pub(super) fn cache_source(source: &PivotSource) -> Result<CacheSource> {
    Ok(match source {
        PivotSource::Range { sheet, range } => CacheSource {
            kind: "worksheet".into(),
            sheet: Some(sheet.clone()),
            reference: Some(*range),
            ..Default::default()
        },
        PivotSource::Name { sheet, name } => {
            let name = name.trim().trim_start_matches('=');
            if name.is_empty() || name.contains('!') || CellRange::parse(name).is_ok() {
                return Err(bad(
                    "use a declared name/table or an explicit worksheet range",
                ));
            }
            CacheSource {
                kind: "worksheet".into(),
                sheet: sheet.clone(),
                defined_name: Some(name.to_string()),
                ..Default::default()
            }
        }
    })
}
pub(super) fn prepare(
    package: &mut OpcPackage,
    book: &Workbook,
    original: &PivotTable,
    original_cache: &PivotCache,
    update: &PivotUpdate,
) -> Result<Prepared> {
    let mut table = original.clone();
    let mut cache = original_cache.clone();
    if table.fields.len() != cache.fields.len() {
        return Err(bad("pivot field ownership is inconsistent"));
    }
    if !table.filters.is_empty() {
        return Err(bad(
            "advanced pivot filters require their own authoring semantics",
        ));
    }
    let mut definition = package.required_part_str(&table.part_name)?;
    if let Some(source) = &update.source {
        if cache.source.kind != "worksheet" || cache.source.relationship_id.is_some() {
            return Err(bad(
                "changing an external pivot source requires explicit connection conversion",
            ));
        }
        cache.source = cache_source(source)?;
        let part = package.required_part_str(&cache.part_name)?;
        let parent =
            xml::child(&part, "cacheSource")?.ok_or_else(|| bad("missing cache source"))?;
        let source = xml::child(&parent.text, "worksheetSource")?
            .ok_or_else(|| bad("missing worksheet source"))?;
        let changed = xml_edit::root_attributes(
            &source.text,
            &[
                (
                    "sheet",
                    cache.source.sheet.as_deref().map(xml::xstring_attribute),
                ),
                (
                    "name",
                    cache
                        .source
                        .defined_name
                        .as_deref()
                        .map(xml::xstring_attribute),
                ),
                ("ref", cache.source.reference.map(crate::shift::range_text)),
            ],
        )?;
        let child = source.finish(&changed)?;
        let changed = xml_edit_ns::set_child(&parent.text, b"worksheetSource", |_| child, &[])?;
        let child = parent.finish(&changed)?;
        package.replace(
            &cache.part_name,
            xml_edit_ns::set_child(&part, b"cacheSource", |_| child, &[])?.into_bytes(),
        );
    }
    let mut measure_keys = (0..table.data_fields.len() as u32).collect::<Vec<_>>();
    if let Some(measures) = &update.measures {
        if measures.is_empty() || measures.len() > 4096 {
            return Err(bad("pivot needs one to 4096 measures"));
        }
        let raw = xml::child(&definition, "dataFields")?
            .map(|c| xml_edit_ns::children(&c.text, b"dataField"))
            .transpose()?
            .unwrap_or_default();
        if raw.len() != original.data_fields.len() {
            return Err(bad("pivot measure XML ownership mismatch"));
        }
        let mut existing = BTreeSet::new();
        let mut captions = BTreeSet::new();
        let mut fields = Vec::new();
        let mut keys = Vec::new();
        let mut fragments = Vec::new();
        for (i, m) in measures.iter().enumerate() {
            if m.field as usize >= cache.fields.len()
                || m.caption.trim().is_empty()
                || m.caption.encode_utf16().count() > 32767
                || !captions.insert(m.caption.to_uppercase())
            {
                return Err(bad("invalid or duplicate pivot measure caption/field"));
            }
            Aggregate::default().value(&m.aggregation)?;
            if let Some(index) = m.existing {
                if index as usize >= raw.len() || !existing.insert(index) {
                    return Err(bad("invalid or duplicate original measure identity"));
                }
                keys.push(index);
            } else {
                keys.push(
                    u32::try_from(original.data_fields.len() + i)
                        .map_err(|_| bad("measure identity overflow"))?,
                );
            }
            let updates = [
                ("fld", Some(m.field.to_string())),
                ("name", Some(xml::xstring_attribute(&m.caption))),
                ("subtotal", Some(m.aggregation.clone())),
                ("showDataAs", Some("normal".into())),
                ("baseField", None),
                ("baseItem", None),
                ("numFmtId", m.number_format_id.map(|n| n.to_string())),
            ];
            let fragment = if let Some(index) = m.existing {
                let c = &raw[index as usize].2;
                c.finish(&xml_edit::root_attributes(&c.text, &updates)?)?
            } else {
                let p = xml::prefix(&definition)?;
                xml_edit::root_attributes(&format!("<{p}dataField/>"), &updates)?
            };
            fragments.push(fragment);
            fields.push(DataField {
                field: m.field,
                name: Some(m.caption.clone()),
                aggregation: m.aggregation.clone(),
                show_as: "normal".into(),
                base_field: None,
                base_item: None,
                number_format_id: m.number_format_id,
            });
        }
        definition = xml::list(&definition, "dataFields", "dataField", |_| Ok(fragments))?;
        table.data_fields = fields;
        measure_keys = keys;
    }
    let mut filtered = BTreeSet::new();
    for f in &update.member_filters {
        if !filtered.insert(f.field) {
            return Err(bad("duplicate pivot member filter"));
        }
        selection(&mut table, &cache, f.field, f.visible.as_ref())?;
    }
    if let Some(layout) = &update.layout {
        let mut used = BTreeSet::new();
        let mut values = 0;
        for f in layout.rows.iter().chain(&layout.columns) {
            if *f == -2 {
                values += 1;
            } else if *f < 0 || *f as usize >= cache.fields.len() || !used.insert(*f as u32) {
                return Err(bad("duplicate or invalid pivot axis field"));
            }
        }
        if values > 1 {
            return Err(bad("Values can appear on only one pivot axis"));
        }
        table.row_fields = layout.rows.clone();
        table.column_fields = layout.columns.clone();
        if values == 0 && table.data_fields.len() > 1 {
            table.column_fields.push(-2);
        }
        table.page_fields.clear();
        for page in &layout.pages {
            if page.field as usize >= cache.fields.len()
                || !used.insert(page.field)
                || filtered.contains(&page.field)
            {
                return Err(bad("duplicate or invalid pivot page field/filter"));
            }
            let (item, visible, multiple) = match &page.selection {
                PivotFilterSelection::All => (None, None, false),
                PivotFilterSelection::Item(item) => {
                    if *item as usize >= cache.fields[page.field as usize].shared_items.len() {
                        return Err(bad("page member is outside the shared cache"));
                    }
                    (Some(*item), None, false)
                }
                PivotFilterSelection::Items(items) => (None, Some(items), true),
            };
            selection(&mut table, &cache, page.field, visible)?;
            table.fields[page.field as usize].attributes.insert(
                "multipleItemSelectionAllowed".into(),
                u8::from(multiple).to_string(),
            );
            table.page_fields.push(PageField {
                field: page.field,
                item,
                hierarchy: None,
                caption: original
                    .page_fields
                    .iter()
                    .find(|p| p.field == page.field)
                    .and_then(|p| p.caption.clone()),
            });
        }
        let compact = layout.report_layout == PivotReportLayout::Compact;
        let outline = layout.report_layout != PivotReportLayout::Tabular;
        table.location.first_data_column = if compact {
            1
        } else {
            table.row_fields.len().max(1) as u32
        };
        table.location.first_data_row = table.column_fields.len().max(1) as u32;
        if let Some(origin) = layout.origin {
            let r = table.location.reference;
            table.location.reference = CellRange {
                start: origin,
                end: CellRef::new(
                    origin
                        .column
                        .checked_add(r.end.column - r.start.column)
                        .ok_or_else(|| bad("pivot column overflow"))?,
                    origin
                        .row
                        .checked_add(r.end.row - r.start.row)
                        .ok_or_else(|| bad("pivot row overflow"))?,
                ),
            };
        }
        table.location.row_page_count = None;
        table.location.column_page_count = None;
        let positions = table
            .page_cells()
            .ok_or_else(|| bad("not enough room above the report for its page filters"))?;
        if let Some((first, _)) = positions.first() {
            table.location.row_page_count =
                Some(positions.iter().map(|(p, _)| p.row).max().unwrap() - first.row + 1);
            table.location.column_page_count = Some(
                (positions.iter().map(|(p, _)| p.column).max().unwrap() - first.column) / 3 + 1,
            );
        }
        for (k, v) in [
            ("rowGrandTotals", layout.row_grand_totals),
            ("colGrandTotals", layout.column_grand_totals),
            ("compact", compact),
            ("compactData", compact),
            ("outline", outline),
            ("outlineData", outline),
        ] {
            table.attributes.insert(k.into(), u8::from(v).to_string());
            definition = xml_edit::root_attributes(&definition, &[(k, boolean(v))])?;
        }
        for f in &mut table.fields {
            f.attributes
                .insert("compact".into(), u8::from(compact).to_string());
            f.attributes
                .insert("outline".into(), u8::from(outline).to_string());
        }
        let raw_pages = xml::child(&definition, "pageFields")?
            .map(|c| xml_edit_ns::children(&c.text, b"pageField"))
            .transpose()?
            .unwrap_or_default();
        definition = xml::list(&definition, "pageFields", "pageField", |p| {
            table
                .page_fields
                .iter()
                .map(|page| {
                    let previous = original
                        .page_fields
                        .iter()
                        .position(|old| old.field == page.field)
                        .and_then(|i| raw_pages.get(i));
                    let updates = [
                        ("fld", Some(page.field.to_string())),
                        ("item", page.item.map(|n| n.to_string())),
                        ("hier", None),
                        ("name", page.caption.as_deref().map(xml::xstring_attribute)),
                    ];
                    if let Some((_, _, c)) = previous {
                        c.finish(&xml_edit::root_attributes(&c.text, &updates)?)
                    } else {
                        xml_edit::root_attributes(&format!("<{p}pageField/>"), &updates)
                    }
                })
                .collect()
        })?;
    }
    let mut configured = BTreeSet::new();
    for options in &update.field_options {
        if !configured.insert(options.field) {
            return Err(bad("duplicate pivot field options"));
        }
        let field = table
            .fields
            .get_mut(options.field as usize)
            .ok_or_else(|| bad("pivot field options are outside the cache"))?;
        if let Some(caption) = &options.caption {
            if caption.trim().is_empty() {
                return Err(bad("pivot field caption cannot be empty"));
            }
            field.caption = Some(caption.clone());
        }
        if let Some(sort) = &options.sort {
            if !matches!(sort.as_str(), "manual" | "ascending" | "descending") {
                return Err(bad("unsupported pivot field sort"));
            }
            field.attributes.insert("sortType".into(), sort.clone());
        }
        if options.subtotals.is_some() {
            for key in [
                "sumSubtotal",
                "countASubtotal",
                "avgSubtotal",
                "maxSubtotal",
                "minSubtotal",
                "productSubtotal",
                "countSubtotal",
                "stdDevSubtotal",
                "stdDevPSubtotal",
                "varSubtotal",
                "varPSubtotal",
            ] {
                field.attributes.insert(key.into(), "0".into());
            }
        }
        for (k, v) in [
            ("defaultSubtotal", options.subtotals),
            ("subtotalTop", options.subtotals_at_top),
        ] {
            if let Some(v) = v {
                field.attributes.insert(k.into(), u8::from(v).to_string());
            }
        }
    }
    if let Some(enabled) = update.fit_columns {
        definition = xml_edit::root_attributes(
            &definition,
            &[("applyWidthHeightFormats", boolean(enabled))],
        )?;
        table.attributes.insert(
            "applyWidthHeightFormats".into(),
            u8::from(enabled).to_string(),
        );
    }
    let raw = xml::child(&definition, "pivotFields")?.ok_or_else(|| bad("missing pivot fields"))?;
    let entries = xml_edit_ns::children(&raw.text, b"pivotField")?;
    if entries.len() != table.fields.len() {
        return Err(bad("pivot field XML ownership mismatch"));
    }
    let mut splices = Vec::new();
    for (i, (start, end, c)) in entries.into_iter().enumerate() {
        let field = &table.fields[i];
        let mut updates = Vec::new();
        for name in [
            "compact",
            "outline",
            "sortType",
            "defaultSubtotal",
            "subtotalTop",
            "hideNewItems",
            "includeNewItemsInFilter",
            "multipleItemSelectionAllowed",
            "sumSubtotal",
            "countASubtotal",
            "avgSubtotal",
            "maxSubtotal",
            "minSubtotal",
            "productSubtotal",
            "countSubtotal",
            "stdDevSubtotal",
            "stdDevPSubtotal",
            "varSubtotal",
            "varPSubtotal",
        ] {
            if let Some(v) = field.attributes.get(name) {
                updates.push((name, Some(v.clone())));
            }
        }
        if let Some(caption) = &field.caption {
            updates.push(("name", Some(xml::xstring_attribute(caption))));
        }
        let mut text = xml_edit::root_attributes(&c.text, &updates)?;
        if update
            .field_options
            .iter()
            .any(|o| o.field as usize == i && o.sort.is_some())
        {
            text = xml_edit_ns::set_child(&text, b"autoSortScope", |_| String::new(), &[])?;
        }
        // report::write retains item metadata and writes their current hidden flags.
        splices.push(Splice {
            start,
            end,
            text: c.finish(&text)?,
        });
    }
    let changed = raw.finish(&apply(&raw.text, splices))?;
    definition = xml_edit_ns::set_child(&definition, b"pivotFields", |_| changed, &[])?;
    if let Some(style) = &update.style {
        if let Some(name) = &style.name {
            if !jade_xlsx_layout::pivot_style::style_names(&book.styles)
                .iter()
                .any(|n| n.eq_ignore_ascii_case(name))
                || book
                    .styles
                    .table_styles
                    .iter()
                    .filter(|d| d.name.eq_ignore_ascii_case(name))
                    .count()
                    > 1
            {
                return Err(bad("unknown or ambiguous pivot style name"));
            }
        }
        let old = xml::child(&definition, "pivotTableStyleInfo")?;
        let p = xml::prefix(&definition)?;
        let head = old
            .as_ref()
            .map_or_else(|| format!("<{p}pivotTableStyleInfo/>"), |c| c.text.clone());
        let changed = xml_edit::root_attributes(
            &head,
            &[
                ("name", style.name.as_deref().map(xml::xstring_attribute)),
                ("showRowHeaders", boolean(style.row_headers)),
                ("showColHeaders", boolean(style.column_headers)),
                ("showRowStripes", boolean(style.row_stripes)),
                ("showColStripes", boolean(style.column_stripes)),
                ("showLastColumn", boolean(style.last_column)),
            ],
        )?;
        let changed = if let Some(old) = old {
            old.finish(&changed)?
        } else {
            changed
        };
        definition = xml_edit_ns::set_child(
            &definition,
            b"pivotTableStyleInfo",
            |_| changed,
            &[
                b"filters",
                b"rowHierarchiesUsage",
                b"colHierarchiesUsage",
                b"extLst",
            ],
        )?;
    }
    package.replace(&table.part_name, definition.into_bytes());
    Ok(Prepared {
        cache,
        table,
        measure_keys,
    })
}
