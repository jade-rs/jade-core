//! Update chart caches from the same references used by shared layout. Only
//! cache values/counts are spliced; chart styles, formulas and extensions stay.
use crate::patch::{apply, element_head_with_attribute, escape, namespace_prefix, Splice};
use crate::{Result, WriteError};
use jade_opc::OpcPackage;
use jade_xlsx_layout::{conditional_format::ConditionalStyles, number_format};
use jade_xlsx_reader::{
    chart::{resolve_chart_reference, ChartPointState},
    CellRef, CellValue, Sheet, Workbook,
};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use std::collections::{BTreeMap, HashSet};

pub type VerifiedChartCells = BTreeMap<String, HashSet<CellRef>>;
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChartCacheRefresh {
    pub updated_caches: usize,
    pub deferred_points: usize,
    pub unresolved_references: usize,
}
#[derive(Default)]
pub(crate) struct Element {
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) main: bool,
    pub(crate) start: usize,
    pub(crate) head_end: usize,
    pub(crate) tail_start: usize,
    pub(crate) end: usize,
    pub(crate) empty: bool,
    pub(crate) text: String,
    pub(crate) attrs: BTreeMap<String, String>,
    pub(crate) children: Vec<Element>,
}
impl Element {
    pub(crate) fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|n| n.main && n.name == name)
    }
}
fn main_ns(ns: ResolveResult<'_>) -> bool {
    matches!(ns,ResolveResult::Bound(n) if matches!(n.as_ref(),b"http://schemas.openxmlformats.org/drawingml/2006/chart"|b"http://purl.oclc.org/ooxml/drawingml/chart"))
}
pub(crate) fn tree(xml: &str) -> Result<Element> {
    let mut reader = NsReader::from_str(xml);
    let mut stack = vec![Element::default()];
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let (ns, event) = reader.read_resolved_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let mut node = Element {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).into(),
                    prefix: namespace_prefix(e.name().as_ref()),
                    main: main_ns(ns),
                    start,
                    head_end: reader.buffer_position() as usize,
                    empty,
                    ..Default::default()
                };
                for attr in e.attributes() {
                    let attr = attr?;
                    if !attr.key.as_ref().contains(&b':') {
                        node.attrs.insert(
                            String::from_utf8_lossy(attr.key.as_ref()).into(),
                            attr.unescape_value()?.into_owned(),
                        );
                    }
                }
                if empty {
                    node.tail_start = node.head_end;
                    node.end = node.head_end;
                    stack.last_mut().unwrap().children.push(node)
                } else {
                    stack.push(node)
                }
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let mut node = stack.pop().unwrap();
                    node.tail_start = start;
                    node.end = reader.buffer_position() as usize;
                    stack.last_mut().unwrap().children.push(node)
                }
            }
            Event::Text(e) => stack.last_mut().unwrap().text.push_str(&e.decode()?),
            Event::CData(e) => stack.last_mut().unwrap().text.push_str(&e.decode()?),
            Event::GeneralRef(e) => {
                if let Some(ch) = e.resolve_char_ref()? {
                    stack.last_mut().unwrap().text.push(ch)
                } else if let Some(text) =
                    quick_xml::escape::resolve_predefined_entity(&e.decode()?)
                {
                    stack.last_mut().unwrap().text.push_str(text)
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(stack.remove(0))
}
fn xtext(value: &str) -> String {
    escape(&jade_xlsx_reader::xstring::encode_xstring(value))
}
fn value_xml(prefix: &str, value: &str) -> String {
    format!("<{prefix}v>{}</{prefix}v>", xtext(value))
}
fn point_xml(prefix: &str, index: usize, value: &str) -> String {
    format!(
        "<{prefix}pt idx=\"{index}\">{}</{prefix}pt>",
        value_xml(prefix, value)
    )
}
pub(crate) fn replace_text(xml: &str, node: &Element, value: &str, splices: &mut Vec<Splice>) {
    if jade_xlsx_reader::xstring::decode_xstring(&node.text) == value && !node.empty {
        return;
    }
    if node.empty {
        let head = &xml[node.start..node.head_end];
        splices.push(Splice {
            start: node.start,
            end: node.end,
            text: format!(
                "{}>{}</{}{}>",
                head.trim_end().trim_end_matches("/>").trim_end(),
                xtext(value),
                node.prefix,
                node.name
            ),
        });
    } else {
        splices.push(Splice {
            start: node.head_end,
            end: node.tail_start,
            text: xtext(value),
        });
    }
}
fn patch_count(xml: &str, node: &Element, count: usize, splices: &mut Vec<Splice>) -> Result<()> {
    if node.attrs.get("val").and_then(|v| v.parse::<usize>().ok()) == Some(count) {
        return Ok(());
    }
    let mut reader = quick_xml::Reader::from_str(&xml[node.start..node.head_end]);
    let mut buf = Vec::new();
    if let Event::Start(e) | Event::Empty(e) = reader.read_event_into(&mut buf)? {
        splices.push(Splice {
            start: node.start,
            end: node.head_end,
            text: element_head_with_attribute(&e, b"val", &count.to_string(), node.empty)?,
        });
    }
    Ok(())
}
fn cache_inner(prefix: &str, format: Option<&str>, points: &[Option<String>]) -> String {
    let mut out = String::new();
    if let Some(format) = format {
        out.push_str(&format!(
            "<{prefix}formatCode>{}</{prefix}formatCode>",
            xtext(format)
        ));
    }
    out.push_str(&format!("<{prefix}ptCount val=\"{}\"/>", points.len()));
    for (i, point) in points.iter().enumerate() {
        if let Some(value) = point {
            out.push_str(&point_xml(prefix, i, value));
        }
    }
    out
}
fn format_code<'a>(
    sheet: &Sheet,
    reference: CellRef,
    cell: Option<&jade_xlsx_reader::Cell>,
    book: &'a Workbook,
) -> Option<&'a str> {
    book.styles
        .cell_format(sheet.effective_style_index(reference, cell.and_then(|c| c.style_index)))
        .and_then(|f| book.styles.number_format_code(f))
}
pub(crate) fn patch_cache_points(
    xml: &str,
    cache: &Element,
    points: &[Option<String>],
    format: Option<&str>,
    splices: &mut Vec<Splice>,
) -> Result<()> {
    let old_points = cache
        .children
        .iter()
        .filter(|c| c.main && c.name == "pt")
        .filter_map(|pt| Some((pt.attrs.get("idx")?.parse::<usize>().ok()?, pt)))
        .collect::<BTreeMap<_, _>>();
    if cache.empty {
        let head = xml[cache.start..cache.head_end]
            .trim_end()
            .trim_end_matches("/>")
            .trim_end();
        splices.push(Splice {
            start: cache.start,
            end: cache.end,
            text: format!(
                "{head}>{}</{}{}>",
                cache_inner(&cache.prefix, format, points),
                cache.prefix,
                cache.name
            ),
        });
    } else {
        if let (Some(code), Some(node)) = (format, cache.child("formatCode")) {
            replace_text(xml, node, code, splices);
        }
        if format.is_some() && cache.child("formatCode").is_none() {
            splices.push(Splice {
                start: cache.head_end,
                end: cache.head_end,
                text: format!(
                    "<{}formatCode>{}</{}formatCode>",
                    cache.prefix,
                    xtext(format.unwrap_or("General")),
                    cache.prefix
                ),
            });
        }
        if let Some(count) = cache.child("ptCount") {
            patch_count(xml, count, points.len(), splices)?;
        } else {
            let at = cache
                .child("formatCode")
                .map(|n| n.end)
                .unwrap_or(cache.head_end);
            splices.push(Splice {
                start: at,
                end: at,
                text: format!("<{}ptCount val=\"{}\"/>", cache.prefix, points.len()),
            });
        }
        for (&i, pt) in &old_points {
            match points.get(i).and_then(Option::as_deref) {
                Some(value) => {
                    if pt.empty {
                        let head = xml[pt.start..pt.head_end]
                            .trim_end()
                            .trim_end_matches("/>")
                            .trim_end();
                        splices.push(Splice {
                            start: pt.start,
                            end: pt.end,
                            text: format!(
                                "{head}>{}</{}pt>",
                                value_xml(&pt.prefix, value),
                                pt.prefix
                            ),
                        });
                    } else if let Some(v) = pt.child("v") {
                        replace_text(xml, v, value, splices)
                    } else {
                        splices.push(Splice {
                            start: pt.head_end,
                            end: pt.head_end,
                            text: value_xml(&pt.prefix, value),
                        });
                    }
                }
                None => splices.push(Splice {
                    start: pt.start,
                    end: pt.end,
                    text: String::new(),
                }),
            }
        }
        let mut new_points = String::new();
        for (i, value) in points.iter().enumerate() {
            if !old_points.contains_key(&i) {
                if let Some(value) = value {
                    new_points.push_str(&point_xml(&cache.prefix, i, value));
                }
            }
        }
        if !new_points.is_empty() {
            let at = cache
                .child("extLst")
                .map(|n| n.start)
                .unwrap_or(cache.tail_start);
            splices.push(Splice {
                start: at,
                end: at,
                text: new_points,
            });
        }
    }
    Ok(())
}
struct ChartContext<'a> {
    sheet: &'a Sheet,
    book: &'a Workbook,
    verified: &'a VerifiedChartCells,
    snapshot: Option<bool>,
}
fn patch_reference(
    xml: &str,
    node: &Element,
    context: &ChartContext<'_>,
    splices: &mut Vec<Splice>,
    report: &mut ChartCacheRefresh,
    category: bool,
) -> Result<()> {
    let (sheet, book, verified, snapshot) = (
        context.sheet,
        context.book,
        context.verified,
        context.snapshot,
    );
    let Some(formula) = node.child("f") else {
        return Ok(());
    };
    let Some(reference) = resolve_chart_reference(&formula.text, sheet, Some(book)) else {
        report.unresolved_references += 1;
        return Ok(());
    };
    if snapshot == Some(true) && reference.points.iter().any(|p| p.hidden) {
        return Err(WriteError::Pivot(
            "detaching visible-only hidden chart points needs index reconciliation".into(),
        ));
    }
    let numeric = node.name == "numRef";
    let cache_name = if numeric { "numCache" } else { "strCache" };
    let cache = node.child(cache_name);
    let old_points: BTreeMap<usize, &Element> = cache
        .into_iter()
        .flat_map(|c| c.children.iter())
        .filter(|c| c.main && c.name == "pt")
        .filter_map(|pt| Some((pt.attrs.get("idx")?.parse::<usize>().ok()?, pt)))
        .collect();
    let fresh = |point: &jade_xlsx_reader::chart::ChartReferencePoint<'_>| {
        snapshot.is_some()
            || point.state == ChartPointState::Live
            || verified
                .get(&reference.sheet.part_name)
                .is_some_and(|refs| refs.contains(&point.reference))
    };
    let deferred = reference.points.iter().filter(|p| !fresh(p)).count();
    report.deferred_points += deferred;
    let old_count = cache
        .and_then(|c| c.child("ptCount"))
        .and_then(|n| n.attrs.get("val"))
        .and_then(|v| v.parse::<usize>().ok());
    // After a structural edit, an unknown formula's old point index may no
    // longer identify that formula. Leave this cache intact until calculated.
    if deferred > 0 && old_count.is_some_and(|count| count != reference.points.len()) {
        return Ok(());
    }
    let conditional = ConditionalStyles::new(reference.sheet, &book.styles);
    let mut points = Vec::with_capacity(reference.points.len());
    for (i, point) in reference.points.iter().enumerate() {
        if !fresh(point) {
            points.push(
                old_points
                    .get(&i)
                    .and_then(|pt| pt.child("v"))
                    .map(|v| jade_xlsx_reader::xstring::decode_xstring(&v.text)),
            );
            continue;
        }
        let value = point.cell.map(|c| &c.value).unwrap_or(&CellValue::Empty);
        let text = if numeric {
            match value {
                CellValue::Number(v) => v.is_finite().then(|| v.to_string()),
                CellValue::Bool(v) => Some(if *v { "1" } else { "0" }.into()),
                CellValue::Error(v) => Some(v.clone()),
                CellValue::Empty => None,
                CellValue::Text(v) => Some(v.clone()),
            }
        } else if value.is_empty() {
            None
        } else {
            let style = conditional.for_cell(point.reference, value);
            let code = style
                .number_format_code
                .as_deref()
                .or_else(|| format_code(reference.sheet, point.reference, point.cell, book));
            Some(number_format::format_value(value, code, book.date_system).text)
        };
        points.push(text);
    }
    let category_format = if snapshot.is_some() && category && numeric {
        let mut codes = std::collections::BTreeSet::new();
        for point in &reference.points {
            let value = point.cell.map(|c| &c.value).unwrap_or(&CellValue::Empty);
            if !value.is_empty() {
                let style = conditional.for_cell(point.reference, value);
                codes.insert(
                    style
                        .number_format_code
                        .as_deref()
                        .or_else(|| format_code(reference.sheet, point.reference, point.cell, book))
                        .unwrap_or("General")
                        .to_string(),
                );
            }
        }
        if codes.len() > 1 {
            return Err(WriteError::Pivot(
                "detaching mixed numeric category formats needs per-point format support".into(),
            ));
        }
        codes.into_iter().next()
    } else {
        None
    };
    let fmt = numeric.then(|| {
        if let Some(format) = category_format {
            return format;
        }

        cache
            .and_then(|c| c.child("formatCode"))
            .map(|n| jade_xlsx_reader::xstring::decode_xstring(&n.text))
            .unwrap_or_else(|| {
                reference
                    .points
                    .first()
                    .and_then(|p| format_code(reference.sheet, p.reference, p.cell, book))
                    .unwrap_or("General")
                    .to_string()
            })
    });
    let before = splices.len();
    if let Some(cache) = cache {
        patch_cache_points(xml, cache, &points, fmt.as_deref(), splices)?;
    } else {
        splices.push(Splice {
            start: formula.end,
            end: formula.end,
            text: format!(
                "<{}{cache_name}>{}</{}{cache_name}>",
                node.prefix,
                cache_inner(&node.prefix, fmt.as_deref(), &points),
                node.prefix
            ),
        });
    }
    if splices.len() > before {
        report.updated_caches += 1;
    }
    Ok(())
}
fn patch_references(
    xml: &str,
    node: &Element,
    context: &ChartContext<'_>,
    splices: &mut Vec<Splice>,
    report: &mut ChartCacheRefresh,
    parent: Option<&str>,
) -> Result<()> {
    if !node.main || matches!(node.name.as_str(), "ext" | "extLst") {
        return Ok(());
    }
    if matches!(node.name.as_str(), "numRef" | "strRef") {
        patch_reference(xml, node, context, splices, report, parent == Some("cat"))?;
        return Ok(());
    }
    for child in &node.children {
        patch_references(xml, child, context, splices, report, Some(&node.name))?;
    }
    Ok(())
}
fn has_chart_parts(package: &OpcPackage) -> Result<bool> {
    let Some(xml) = package.part_str("[Content_Types].xml")? else {
        return Ok(false);
    };
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) => {
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"ContentType"
                        && attr.unescape_value()?.as_ref()
                            == "application/vnd.openxmlformats-officedocument.drawingml.chart+xml"
                    {
                        return Ok(true);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(false)
}
pub(crate) fn refresh_package(
    package: &mut OpcPackage,
    verified: &VerifiedChartCells,
) -> Result<ChartCacheRefresh> {
    let mut report = ChartCacheRefresh::default();
    if !has_chart_parts(package)? {
        return Ok(report);
    }
    let book = jade_xlsx_reader::read_package(package)?;
    let mut seen = HashSet::new();
    for sheet in &book.sheets {
        for chart in sheet.shapes.iter().filter_map(|s| s.chart.as_ref()) {
            if chart.pivot_source.is_some()
                && chart
                    .unsupported_properties
                    .iter()
                    .any(|p| p == "external chart data")
            {
                continue;
            }
            let Some(part) = chart.part_name.as_ref() else {
                continue;
            };
            if !seen.insert(part.clone()) {
                continue;
            }
            let xml = package
                .part_str(part)?
                .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
            let tree = tree(&xml)?;
            let mut splices = Vec::new();
            for root in &tree.children {
                patch_references(
                    &xml,
                    root,
                    &ChartContext {
                        sheet,
                        book: &book,
                        verified,
                        snapshot: None,
                    },
                    &mut splices,
                    &mut report,
                    None,
                )?;
            }
            if !splices.is_empty() {
                package.replace(part, apply(&xml, splices).into_bytes());
            }
        }
    }
    Ok(report)
}
/// A static chart snapshot follows the same saved worksheet points displayed
/// by layout, including a formula's displayed cached value. It performs no
/// calculation and does not mark those values verified for normal refresh.
pub(crate) fn snapshot_chart_data(
    xml: &str,
    sheet: &Sheet,
    book: &Workbook,
    visible_only: bool,
) -> Result<String> {
    let root = tree(xml)?;
    let mut splices = Vec::new();
    let mut report = ChartCacheRefresh::default();
    for node in &root.children {
        patch_references(
            xml,
            node,
            &ChartContext {
                sheet,
                book,
                verified: &VerifiedChartCells::new(),
                snapshot: Some(visible_only),
            },
            &mut splices,
            &mut report,
            None,
        )?;
    }
    Ok(apply(xml, splices))
}
/// Explicit cache synchronization for an existing workbook. Unresolved and
/// uncalculated points retain their caches and are counted in the report.
pub fn refresh_chart_caches(original: &[u8]) -> Result<(Vec<u8>, ChartCacheRefresh)> {
    let mut package = OpcPackage::open(original)?;
    let report = refresh_package(&mut package, &VerifiedChartCells::new())?;
    let bytes = if report.updated_caches == 0 {
        original.to_vec()
    } else {
        package.write()?
    };
    Ok((bytes, report))
}
