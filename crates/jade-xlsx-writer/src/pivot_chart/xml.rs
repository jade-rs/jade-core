use super::projection::{Projection, Series};
use crate::{
    chart_cache::{tree, Element},
    patch::{apply, element_head_with_attribute, escape, Splice},
    Result, WriteError,
};
use std::collections::{BTreeMap, BTreeSet};
fn bad(s: &str) -> WriteError {
    WriteError::Pivot(format!("pivot chart: {s}"))
}
fn text(s: &str) -> String {
    escape(&jade_xlsx_reader::xstring::encode_xstring(s))
}
fn unique<'a>(n: &'a Element, name: &str) -> Result<Option<&'a Element>> {
    let children = n
        .children
        .iter()
        .filter(|n| n.main && n.name == name)
        .collect::<Vec<_>>();
    if children.len() > 1 {
        return Err(bad("duplicate owned chart child"));
    }
    Ok(children.into_iter().next())
}
fn literal(prefix: &str, numeric: bool, format: &str, points: &[Option<String>]) -> String {
    data_xml(
        prefix,
        if numeric { "numLit" } else { "strLit" },
        numeric,
        format,
        points,
    )
}
fn data_xml(
    prefix: &str,
    kind: &str,
    numeric: bool,
    format: &str,
    points: &[Option<String>],
) -> String {
    let mut out = format!("<{prefix}{kind}>");
    if numeric {
        out.push_str(&format!(
            "<{prefix}formatCode>{}</{prefix}formatCode>",
            text(format)
        ));
    }
    out.push_str(&format!("<{prefix}ptCount val=\"{}\"/>", points.len()));
    for (i, v) in points.iter().enumerate() {
        if let Some(v) = v {
            out.push_str(&format!(
                "<{prefix}pt idx=\"{i}\"><{prefix}v>{}</{prefix}v></{prefix}pt>",
                text(v)
            ));
        }
    }
    out.push_str(&format!("</{prefix}{kind}>"));
    out
}
fn data_safe(n: &Element) -> bool {
    if !n.main {
        return false;
    }
    let attrs = if n.name == "pt" {
        n.attrs.keys().all(|k| k == "idx" || k == "xmlns")
    } else if n.name == "ptCount" {
        n.attrs.keys().all(|k| k == "val" || k == "xmlns")
    } else {
        n.attrs.keys().all(|k| k == "xmlns")
    };
    attrs
        && matches!(
            n.name.as_str(),
            "strRef"
                | "numRef"
                | "strLit"
                | "numLit"
                | "numCache"
                | "strCache"
                | "ptCount"
                | "pt"
                | "v"
                | "f"
                | "formatCode"
        )
        && n.children.iter().all(data_safe)
}
fn patch_val(xml: &str, n: &Element, value: u32, splices: &mut Vec<Splice>) -> Result<()> {
    let mut r = quick_xml::Reader::from_str(&xml[n.start..n.head_end]);
    match r.read_event()? {
        quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
            splices.push(Splice {
                start: n.start,
                end: n.head_end,
                text: element_head_with_attribute(&e, b"val", &value.to_string(), n.empty)?,
            })
        }
        _ => return Err(bad("invalid chart index")),
    }
    Ok(())
}
fn point_identity(
    xml: &str,
    n: &Element,
    old: &Projection,
    new: &Projection,
    splices: &mut Vec<Splice>,
) -> Result<()> {
    let index = unique(n, "idx")?
        .and_then(|n| n.attrs.get("val"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .ok_or_else(|| bad("invalid per-point index"))?;
    let oldkey = &old
        .categories
        .get(index)
        .ok_or_else(|| bad("per-point index outside categories"))?
        .key;
    if let Some(next) = new.categories.iter().position(|c| &c.key == oldkey) {
        patch_val(xml, n.child("idx").unwrap(), next as u32, splices)?;
    } else {
        splices.push(Splice {
            start: n.start,
            end: n.end,
            text: String::new(),
        });
    }
    Ok(())
}
fn address(at: jade_xlsx_reader::CellRef) -> String {
    let mut n = at.column + 1;
    let mut letters = Vec::new();
    while n > 0 {
        letters.push((b'A' + ((n - 1) % 26) as u8) as char);
        n = (n - 1) / 26;
    }
    format!(
        "${}${}",
        letters.iter().rev().collect::<String>(),
        at.row + 1
    )
}
fn range_formula(sheet: &str, refs: &[jade_xlsx_reader::CellRef]) -> Option<String> {
    let first = *refs.first()?;
    if refs.windows(2).any(|w| {
        !((w[0].column == w[1].column && w[0].row.checked_add(1) == Some(w[1].row))
            || (w[0].row == w[1].row && w[0].column.checked_add(1) == Some(w[1].column)))
    }) {
        return None;
    }
    if refs.iter().any(|r| r.column != first.column) && refs.iter().any(|r| r.row != first.row) {
        return None;
    }
    let start = format!("'{}'!{}", sheet.replace('\'', "''"), address(first));
    Some(if refs.len() == 1 {
        start
    } else {
        format!("{start}:{}", address(*refs.last()?))
    })
}
struct Data<'a> {
    literal: String,
    points: &'a [Option<String>],
    numeric: bool,
    format: &'a str,
    formula: Option<String>,
}
fn plain_formula(xml: &str, n: &Element, value: &str, splices: &mut Vec<Splice>) -> Result<()> {
    if !n.children.is_empty() {
        return Err(bad("uninterpreted formula leaf markup"));
    }
    if n.empty {
        let head = xml[n.start..n.head_end]
            .trim_end()
            .trim_end_matches("/>")
            .trim_end();
        splices.push(Splice {
            start: n.start,
            end: n.end,
            text: format!("{head}>{}</{}{}>", escape(value), n.prefix, n.name),
        });
    } else {
        splices.push(Splice {
            start: n.head_end,
            end: n.tail_start,
            text: escape(value),
        });
    }
    Ok(())
}

pub(super) fn rename_source(xml: &str, name: &str) -> Result<String> {
    let root = tree(xml)?;
    let space = unique(&root, "chartSpace")?.ok_or_else(|| bad("missing chartSpace"))?;
    let source = unique(space, "pivotSource")?.ok_or_else(|| bad("missing pivot source"))?;
    let leaf = unique(source, "name")?.ok_or_else(|| bad("missing pivot source name"))?;
    let mut splices = Vec::new();
    plain_formula(xml, leaf, name, &mut splices)?;
    Ok(apply(xml, splices))
}
fn patch_data(
    xml: &str,
    parent: &Element,
    name: &str,
    data: Data<'_>,
    splices: &mut Vec<Splice>,
) -> Result<()> {
    if let Some(n) = unique(parent, name)? {
        let sources = n
            .children
            .iter()
            .filter(|c| {
                c.main
                    && matches!(
                        c.name.as_str(),
                        "v" | "strRef" | "numRef" | "strLit" | "numLit" | "multiLvlStrRef"
                    )
            })
            .collect::<Vec<_>>();
        if sources.len() > 1 {
            return Err(bad("duplicate owned data source"));
        }
        if let Some(source) = sources.first() {
            if let Some(formula) = data
                .formula
                .as_deref()
                .filter(|_| source.name == "strRef" || (name != "tx" && source.name == "numRef"))
            {
                let numeric = source.name == "numRef";
                let cache_name = if numeric { "numCache" } else { "strCache" };
                let f = unique(source, "f")?;
                let cache = unique(source, cache_name)?;
                let inner_f = format!(
                    "<{}f>{}</{}f>",
                    source.prefix,
                    escape(formula),
                    source.prefix
                );
                // A numeric category reference retains its numeric source. The
                // ordinary cache pass resolves its actual cell values immediately.
                let cache_points = if name == "cat" && numeric {
                    &[][..]
                } else {
                    data.points
                };
                let new_cache = data_xml(
                    &source.prefix,
                    cache_name,
                    numeric,
                    data.format,
                    cache_points,
                );
                if source.empty {
                    let head = xml[source.start..source.head_end]
                        .trim_end()
                        .trim_end_matches("/>")
                        .trim_end();
                    splices.push(Splice {
                        start: source.start,
                        end: source.end,
                        text: format!(
                            "{head}>{inner_f}{new_cache}</{}{}>",
                            source.prefix, source.name
                        ),
                    });
                } else {
                    if let Some(f) = f {
                        plain_formula(xml, f, formula, splices)?;
                    } else {
                        splices.push(Splice {
                            start: source.head_end,
                            end: source.head_end,
                            text: if cache.is_none() {
                                format!("{inner_f}{new_cache}")
                            } else {
                                inner_f
                            },
                        });
                    }
                    if let Some(cache) = cache {
                        if !(name == "cat" && numeric) {
                            crate::chart_cache::patch_cache_points(
                                xml,
                                cache,
                                data.points,
                                numeric.then_some(data.format),
                                splices,
                            )?;
                        }
                    } else if f.is_some() {
                        let at = source
                            .child("extLst")
                            .map_or(source.tail_start, |n| n.start);
                        splices.push(Splice {
                            start: at,
                            end: at,
                            text: new_cache,
                        });
                    }
                }
                return Ok(());
            }
            let kind = if data.numeric { "numLit" } else { "strLit" };
            if name != "tx" && source.name == kind {
                crate::chart_cache::patch_cache_points(
                    xml,
                    source,
                    data.points,
                    data.numeric.then_some(data.format),
                    splices,
                )?;
                return Ok(());
            }
            if name == "tx" && source.name == "v" {
                if !source.children.is_empty() {
                    return Err(bad("uninterpreted series name markup"));
                }
                crate::chart_cache::replace_text(
                    xml,
                    source,
                    data.points.first().and_then(|p| p.as_deref()).unwrap_or(""),
                    splices,
                );
                return Ok(());
            }
        }
        if n.children.iter().any(|c| !data_safe(c)) {
            return Err(bad("unsupported data-source metadata needs reconciliation"));
        }
        if n.empty {
            let head = xml[n.start..n.head_end]
                .trim_end()
                .trim_end_matches("/>")
                .trim_end();
            splices.push(Splice {
                start: n.start,
                end: n.end,
                text: format!("{head}>{}</{}{name}>", data.literal, n.prefix),
            });
        } else {
            splices.push(Splice {
                start: n.head_end,
                end: n.tail_start,
                text: data.literal,
            });
        }
    } else {
        let following: &[&str] = match name {
            "tx" => &[
                "spPr",
                "invertIfNegative",
                "pictureOptions",
                "marker",
                "dPt",
                "dLbls",
                "trendline",
                "errBars",
                "cat",
                "val",
                "smooth",
                "shape",
                "extLst",
            ],
            "cat" => &["val", "smooth", "shape", "extLst"],
            _ => &["smooth", "shape", "extLst"],
        };
        let at = parent
            .children
            .iter()
            .find(|n| n.main && following.contains(&n.name.as_str()))
            .map_or(parent.tail_start, |n| n.start);
        splices.push(Splice {
            start: at,
            end: at,
            text: format!(
                "<{}{name}>{}</{}{name}>",
                parent.prefix, data.literal, parent.prefix
            ),
        });
    }
    Ok(())
}
fn series(
    xml: &str,
    node: &Element,
    s: &Series,
    order: u32,
    old: &Projection,
    new: &Projection,
    sheet: &str,
) -> Result<String> {
    let mut patches = Vec::new();
    if let Some(n) = unique(node, "order")? {
        patch_val(xml, n, order, &mut patches)?;
    } else {
        let at = unique(node, "idx")?.map_or(node.head_end, |n| n.end);
        patches.push(Splice {
            start: at,
            end: at,
            text: format!("<{}order val=\"{order}\"/>", node.prefix),
        });
    }
    let names = [Some(s.name.clone())];
    patch_data(
        xml,
        node,
        "tx",
        Data {
            literal: format!("<{}v>{}</{}v>", node.prefix, text(&s.name), node.prefix),
            points: &names,
            numeric: false,
            format: "",
            formula: s.name_reference.and_then(|r| range_formula(sheet, &[r])),
        },
        &mut patches,
    )?;
    let labels = new
        .categories
        .iter()
        .map(|c| Some(c.label.clone()))
        .collect::<Vec<_>>();
    let category_refs = new
        .categories
        .iter()
        .map(|c| c.reference)
        .collect::<Option<Vec<_>>>();
    patch_data(
        xml,
        node,
        "cat",
        Data {
            literal: literal(&node.prefix, false, "", &labels),
            points: &labels,
            numeric: false,
            format: "General",
            formula: category_refs
                .as_deref()
                .and_then(|r| range_formula(sheet, r)),
        },
        &mut patches,
    )?;
    patch_data(
        xml,
        node,
        "val",
        Data {
            literal: literal(&node.prefix, true, &s.format, &s.values),
            points: &s.values,
            numeric: true,
            format: &s.format,
            formula: range_formula(sheet, &s.references),
        },
        &mut patches,
    )?;
    for n in node.children.iter().filter(|c| c.main) {
        match n.name.as_str() {
            "dPt" => point_identity(xml, n, old, new, &mut patches)?,
            "dLbls" => {
                for label in n.children.iter().filter(|c| c.main && c.name == "dLbl") {
                    point_identity(xml, label, old, new, &mut patches)?;
                }
            }
            "errBars"
                if n.child("errValType")
                    .and_then(|v| v.attrs.get("val"))
                    .is_some_and(|v| v == "cust") =>
            {
                return Err(bad("custom error bars need point reconciliation"))
            }
            _ => {}
        }
    }
    for p in &mut patches {
        p.start -= node.start;
        p.end -= node.start;
    }
    Ok(apply(&xml[node.start..node.end], patches))
}
fn new_series(prefix: &str, s: &Series, idx: u32, order: u32, new: &Projection) -> String {
    let labels = new
        .categories
        .iter()
        .map(|c| Some(c.label.clone()))
        .collect::<Vec<_>>();
    format!("<{prefix}ser><{prefix}idx val=\"{idx}\"/><{prefix}order val=\"{order}\"/><{prefix}tx><{prefix}v>{}</{prefix}v></{prefix}tx><{prefix}cat>{}</{prefix}cat><{prefix}val>{}</{prefix}val></{prefix}ser>",text(&s.name),literal(prefix,false,"",&labels),literal(prefix,true,&s.format,&s.values))
}
pub(super) fn update(
    xml: &str,
    old: &Projection,
    new: &Projection,
    bindings: &BTreeMap<(usize, u32), usize>,
    sheet: &str,
) -> Result<String> {
    let tree = tree(xml)?;
    let space = unique(&tree, "chartSpace")?.ok_or_else(|| bad("missing chartSpace"))?;
    if space.child("externalData").is_some() {
        return Err(bad("external chart data is not a local pivot binding"));
    }
    let area = space
        .child("chart")
        .and_then(|c| c.child("plotArea"))
        .ok_or_else(|| bad("missing chart plot"))?;
    let plots = area
        .children
        .iter()
        .filter(|n| n.main && n.name.ends_with("Chart"))
        .collect::<Vec<_>>();
    if plots.is_empty()
        || plots.iter().any(|p| {
            !matches!(
                p.name.as_str(),
                "barChart"
                    | "lineChart"
                    | "areaChart"
                    | "pieChart"
                    | "doughnutChart"
                    | "radarChart"
            )
        })
    {
        return Err(bad("this plot topology needs PivotChart reconciliation"));
    }
    let mut existing = BTreeMap::new();
    let mut indices = BTreeSet::new();
    let mut largest_idx: Option<u32> = None;
    let mut largest_order: Option<u32> = None;
    let custom_order = bindings
        .iter()
        .any(|((_, order), index)| *order as usize != *index);
    let mut old_nodes = Vec::new();
    let mut measure_plots = BTreeMap::<u32, BTreeSet<usize>>::new();
    for (pi, plot) in plots.iter().enumerate() {
        let nodes = plot
            .children
            .iter()
            .filter(|n| n.main && n.name == "ser")
            .collect::<Vec<_>>();
        for (position, node) in nodes.iter().enumerate() {
            let order = node
                .child("order")
                .and_then(|n| n.attrs.get("val"))
                .map(|n| n.trim().parse::<u32>())
                .transpose()
                .map_err(|_| bad("invalid series order"))?
                .unwrap_or(position as u32);
            let old_index = *bindings
                .get(&(pi, order))
                .ok_or_else(|| bad("missing saved series mapping"))?;
            let idx = node
                .child("idx")
                .and_then(|n| n.attrs.get("val"))
                .and_then(|v| v.trim().parse::<u32>().ok())
                .ok_or_else(|| bad("missing series identity"))?;
            if !indices.insert(idx) {
                return Err(bad("duplicate series identity"));
            }
            largest_idx = Some(largest_idx.map_or(idx, |old| old.max(idx)));
            largest_order = Some(largest_order.map_or(order, |old| old.max(order)));
            let key = &old
                .series
                .get(old_index)
                .ok_or_else(|| bad("series mapping outside pivot"))?
                .key;
            measure_plots.entry(key.measure).or_default().insert(pi);
            if existing.insert(key, (pi, *node, order)).is_some() {
                return Err(bad("duplicate series mapping"));
            }
        }
        old_nodes.push(nodes);
    }
    if existing.len() != old.series.len() {
        return Err(bad("saved series do not map to the visible pivot"));
    }
    let mut bodies = vec![String::new(); plots.len()];
    for (order, s) in new.series.iter().enumerate() {
        if let Some((pi, node, original_order)) = existing.get(&s.key) {
            let order = if custom_order {
                *original_order
            } else {
                order as u32
            };
            bodies[*pi].push_str(&series(xml, node, s, order, old, new, sheet)?);
        } else {
            let target = match measure_plots.get(&s.key.measure) {
                None => 0,
                Some(p) if p.len() == 1 => *p.first().unwrap(),
                _ => return Err(bad("new series has ambiguous plot routing")),
            };
            let idx = largest_idx
                .map(|n| {
                    n.checked_add(1)
                        .ok_or_else(|| bad("series identity overflow"))
                })
                .transpose()?
                .unwrap_or(0);
            largest_idx = Some(idx);
            let order = if custom_order {
                largest_order
                    .map(|n| n.checked_add(1).ok_or_else(|| bad("series order overflow")))
                    .transpose()?
                    .unwrap_or(0)
            } else {
                order as u32
            };
            largest_order = Some(largest_order.map_or(order, |n| n.max(order)));
            bodies[target].push_str(&new_series(&plots[target].prefix, s, idx, order, new));
        }
    }
    let mut patches = Vec::new();
    for ((plot, nodes), body) in plots.iter().zip(old_nodes).zip(bodies) {
        if nodes.is_empty() {
            if !body.is_empty() {
                let at = plot
                    .children
                    .iter()
                    .find(|n| {
                        n.main
                            && !matches!(
                                n.name.as_str(),
                                "barDir" | "grouping" | "varyColors" | "radarStyle"
                            )
                    })
                    .map_or(plot.tail_start, |n| n.start);
                patches.push(Splice {
                    start: at,
                    end: at,
                    text: body,
                });
            }
        } else {
            for (i, n) in nodes.iter().enumerate() {
                patches.push(Splice {
                    start: n.start,
                    end: n.end,
                    text: if i == 0 { body.clone() } else { String::new() },
                });
            }
        }
    }
    Ok(apply(xml, patches))
}

/// Freeze a detached PivotChart using its exact saved cache contents. This
/// deliberately preserves numeric category caches and format codes instead of
/// projecting them through displayed strings. Unknown source metadata refuses.
pub(super) fn detach(xml: &str) -> Result<String> {
    let root = tree(xml)?;
    let space = unique(&root, "chartSpace")?.ok_or_else(|| bad("missing chartSpace"))?;
    let pivot = unique(space, "pivotSource")?.ok_or_else(|| bad("missing pivot source"))?;
    let chart = unique(space, "chart")?.ok_or_else(|| bad("missing chart"))?;
    let area = unique(chart, "plotArea")?.ok_or_else(|| bad("missing plot area"))?;
    let mut parents = Vec::new();
    let mut title_tx = None;
    if let Some(title) = unique(chart, "title")? {
        if let Some(tx) = unique(title, "tx")? {
            parents.push(tx);
            title_tx = Some(tx.start);
        }
    }
    for plot in area
        .children
        .iter()
        .filter(|n| n.main && n.name.ends_with("Chart"))
    {
        for series in plot.children.iter().filter(|n| n.main && n.name == "ser") {
            for name in ["tx", "cat", "val", "xVal", "yVal", "bubbleSize"] {
                if let Some(n) = unique(series, name)? {
                    parents.push(n);
                }
            }
            for errors in series
                .children
                .iter()
                .filter(|n| n.main && n.name == "errBars")
            {
                for name in ["plus", "minus"] {
                    if let Some(n) = unique(errors, name)? {
                        parents.push(n);
                    }
                }
            }
        }
    }
    let mut sources = Vec::new();
    let mut needed = BTreeSet::new();
    for parent in parents {
        let refs = parent
            .children
            .iter()
            .filter(|n| n.main && matches!(n.name.as_str(), "numRef" | "strRef" | "multiLvlStrRef"))
            .collect::<Vec<_>>();
        if refs.len() > 1 {
            return Err(bad("duplicate source while detaching chart"));
        }
        for source in refs {
            if !data_safe(source) {
                return Err(bad("opaque or multilevel source cannot be detached"));
            }
            // The generic tree omits qualified attributes; conversion must also
            // account for those so they cannot disappear with the old wrapper.
            let mut reader = quick_xml::Reader::from_str(&xml[source.start..source.end]);
            loop {
                match reader.read_event()? {
                    quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                        for a in e.attributes() {
                            let a = a?;
                            let key = a.key.as_ref();
                            if key.contains(&b':') && !key.starts_with(b"xmlns:") {
                                return Err(bad("qualified source metadata cannot be detached"));
                            }
                        }
                    }
                    quick_xml::events::Event::Eof => break,
                    _ => {}
                }
            }
            let numeric = source.name == "numRef";
            let cache = unique(source, if numeric { "numCache" } else { "strCache" })?
                .ok_or_else(|| bad("detaching requires an actual saved chart cache"))?;
            needed.insert(source.start);
            needed.insert(cache.start);
            sources.push((parent, source, cache, numeric));
        }
    }
    let mut environments = BTreeMap::new();
    let mut reader = quick_xml::NsReader::from_str(xml);
    loop {
        let at = reader.buffer_position() as usize;
        match reader.read_event()? {
            quick_xml::events::Event::Start(_) | quick_xml::events::Event::Empty(_) => {
                if needed.contains(&at) {
                    environments.insert(at, crate::xml_edit_ns::bindings(&reader));
                }
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    fn formulas(n: &Element, out: &mut Vec<usize>) {
        if n.main && n.name == "f" {
            out.push(n.start);
        }
        for c in &n.children {
            formulas(c, out);
        }
    }
    let mut formula_positions = Vec::new();
    formulas(chart, &mut formula_positions);
    if formula_positions.iter().any(|at| {
        !sources
            .iter()
            .any(|(_, s, _, _)| s.start <= *at && *at < s.end)
    }) {
        return Err(bad("uninterpreted formula source cannot be detached"));
    }
    let mut patches = vec![Splice {
        start: pivot.start,
        end: pivot.end,
        text: String::new(),
    }];
    if let Some(formats) = unique(chart, "pivotFmts")? {
        if !formats.children.is_empty() {
            return Err(bad(
                "pivot format deltas need materialization before detaching",
            ));
        }
    }
    for (parent, source, cache, numeric) in sources {
        let replacement = if parent.name == "tx" {
            if numeric {
                return Err(bad("numeric chart title reference cannot be detached"));
            }
            let points = cache
                .children
                .iter()
                .filter(|n| n.main && n.name == "pt")
                .collect::<Vec<_>>();
            if points.len() > 1
                || points
                    .first()
                    .is_some_and(|p| p.attrs.get("idx").map(String::as_str) != Some("0"))
            {
                return Err(bad("ambiguous detached series/title name"));
            }
            let leaf = points
                .first()
                .map(|p| unique(p, "v"))
                .transpose()?
                .flatten();
            let raw = leaf.map(|v| &xml[v.head_end..v.tail_start]).unwrap_or("");
            if title_tx == Some(parent.start) {
                let mut reader = quick_xml::NsReader::from_str(xml);
                let strict = loop {
                    match reader.read_event()? {
                        quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                            break matches!(reader.resolver().resolve_element(e.name()).0,quick_xml::name::ResolveResult::Bound(n) if n.as_ref()==b"http://purl.oclc.org/ooxml/drawingml/chart")
                        }
                        quick_xml::events::Event::Eof => return Err(bad("missing chart root")),
                        _ => {}
                    }
                };
                let main = if strict {
                    "http://purl.oclc.org/ooxml/drawingml/main"
                } else {
                    "http://schemas.openxmlformats.org/drawingml/2006/main"
                };
                let value = jade_xlsx_reader::xstring::decode_xstring(
                    leaf.map(|v| v.text.as_str()).unwrap_or(""),
                );
                // Chart cache values use ST_Xstring; DrawingML a:t is plain
                // xsd:string. Decoding an escaped XML control into a:t would
                // return malformed XML even though both cached inputs parse.
                if !value.chars().all(crate::patch::is_xml_10_character) {
                    return Err(bad(
                        "cached title cannot be represented as static DrawingML text",
                    ));
                }
                let value = escape(&value).replace('\r', "&#13;");
                crate::xml_edit_ns::ContextXml::new(&format!("<{}rich xmlns:jadeStatic=\"{main}\"><jadeStatic:bodyPr/><jadeStatic:lstStyle/><jadeStatic:p><jadeStatic:r><jadeStatic:t>{value}</jadeStatic:t></jadeStatic:r></jadeStatic:p></{}rich>",source.prefix,source.prefix),&environments[&source.start])?.text
            } else {
                crate::xml_edit_ns::ContextXml::new(
                    &format!("<{}v>{raw}</{}v>", source.prefix, source.prefix),
                    &environments[&source.start],
                )?
                .text
            }
        } else {
            let literal = if numeric { "numLit" } else { "strLit" };
            let raw = crate::xml_edit_ns::ContextXml::new(
                &xml[cache.start..cache.end],
                &environments[&cache.start],
            )?
            .text;
            let old = format!("{}{}", cache.prefix, cache.name);
            let new = format!("{}{literal}", cache.prefix);
            let raw = raw.replacen(&format!("<{old}"), &format!("<{new}"), 1);
            if cache.empty {
                raw
            } else {
                let at = raw
                    .rfind(&format!("</{old}"))
                    .ok_or_else(|| bad("missing cache end"))?;
                format!("{}</{new}>", &raw[..at])
            }
        };
        patches.push(Splice {
            start: source.start,
            end: source.end,
            text: replacement,
        });
    }
    Ok(apply(xml, patches))
}
