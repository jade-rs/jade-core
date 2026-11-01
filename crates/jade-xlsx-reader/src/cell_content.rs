//! The interpreted standard grid/string stream. The original package is never
//! serialized from this view: preserving writers use the source tag locations.
use crate::{data_validations::MC_NAMESPACE, CellRef, Result};
use quick_xml::{
    events::{BytesEnd, BytesStart, Event},
    name::ResolveResult,
    NsReader, Writer,
};
use std::borrow::Cow;
use std::collections::HashSet;

#[derive(Default)]
pub struct WorksheetSource {
    starts: Vec<usize>,
    ends: Vec<usize>,
    cells: Vec<CellSource>,
    drawing_links: [Vec<Option<String>>; 2],
    /// Every row after the one before it and every cell after the one
    /// before it in its row, with no compatibility branches to reorder.
    ordered: bool,
}
impl WorksheetSource {
    /// Whether the sheet's rows, and each row's cells, are already in
    /// order, so a writer need not sort them.
    pub fn is_ordered(&self) -> bool {
        self.ordered
    }
    fn drawing_relationship(&self, legacy: bool) -> Result<Option<String>> {
        if self.starts.is_empty() {
            return Err(crate::XlsxError::BadDrawingProtection(
                "not a worksheet".into(),
            ));
        }
        let links = &self.drawing_links[usize::from(legacy)];
        if links.is_empty() {
            return Ok(None);
        }
        if links.len() != 1 || links[0].is_none() {
            return Err(crate::XlsxError::BadDrawingProtection(
                "ambiguous worksheet drawing relationship".into(),
            ));
        }
        Ok(links[0].clone())
    }
    pub fn owns_start(&self, offset: usize) -> bool {
        self.starts.binary_search(&offset).is_ok()
    }
    pub fn owns_end(&self, offset: usize) -> bool {
        self.ends.binary_search(&offset).is_ok()
    }
}

#[derive(Clone)]
pub enum CacheInsertion {
    InBranch(usize),
    AfterFormula(std::ops::Range<usize>),
    OpenEmpty(std::ops::Range<usize>),
}

/// A standard cell in one understood representation of a worksheet.
pub struct CellSource {
    pub reference: CellRef,
    pub namespace: String,
    pub range: std::ops::Range<usize>,
    pub head_end: usize,
    pub selected: bool,
    pub has_formula: bool,
    pub selected_formula: bool,
    pub preferred_has_value: bool,
    preferred_formula: Option<std::ops::Range<usize>>,
    preferred_formula_nested: bool,
    preferred_branch: Option<CacheInsertion>,
    branch_priority: (bool, usize),
    pub values: Vec<std::ops::Range<usize>>,
}

impl CellSource {
    pub fn missing_cache_insertion(&self) -> Option<CacheInsertion> {
        if self.preferred_has_value {
            return None;
        }
        let formula = self.preferred_formula.clone()?;
        if self.preferred_formula_nested {
            return Some(CacheInsertion::AfterFormula(formula));
        }
        self.preferred_branch
            .clone()
            .or(Some(CacheInsertion::AfterFormula(formula)))
    }
}

type Coordinates = (u32, u32, u32);
enum Frame {
    Element {
        record: bool,
        cell: bool,
        leaf: Option<(usize, bool, bool, bool)>,
    },
    Alternate {
        chosen: bool,
        before: Coordinates,
        selected_after: Option<Coordinates>,
        contains_value: bool,
        preferred_branch: Option<CacheInsertion>,
        depth: usize,
    },
    Branch {
        selected: bool,
        head_end: usize,
    },
}

pub(crate) fn worksheet(xml: &str) -> Result<String> {
    Ok(project(xml, "worksheet", false, false, None, false, None)?.0)
}
pub(crate) fn comments(xml: &str) -> Result<String> {
    Ok(project(xml, "comments", false, false, None, false, None)?.0)
}
pub(crate) fn shared_strings(xml: &str) -> Result<String> {
    Ok(project(xml, "sst", false, false, None, false, None)?.0)
}
/// Original byte offsets of the selected standard worksheet structure and
/// formulas. Namespace aliases and supported MCE branches use the same policy
/// as cell values, rich runs and phonetic readings.
pub fn worksheet_source(xml: &str) -> Result<WorksheetSource> {
    Ok(project(xml, "worksheet", true, false, None, false, None)?.1)
}
/// [`worksheet_source`] read only through the first row after `last_row`:
/// enough for a writer placing cells no further down, in a sheet already
/// known to be in order. What it says about order covers what it read.
pub fn worksheet_source_through(xml: &str, last_row: u32) -> Result<WorksheetSource> {
    Ok(project(xml, "worksheet", true, false, None, false, Some(last_row))?.1)
}
/// Relationship-only projection skips grid subtrees and builds no cell/span view.
pub(crate) fn worksheet_drawing_relationship(xml: &str, legacy: bool) -> Result<Option<String>> {
    // Chart/dialog sheets also own drawing references. They remain distinct
    // from the grid, but must not make an otherwise readable workbook fail.
    let mut reader = NsReader::from_str(xml);
    let root = loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                break String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
            }
            Event::Eof => {
                return Err(crate::XlsxError::BadDrawingProtection(
                    "missing sheet root".into(),
                ))
            }
            _ => {}
        }
    };
    if !matches!(root.as_str(), "worksheet" | "chartsheet" | "dialogsheet") {
        return Err(crate::XlsxError::BadDrawingProtection(
            "not a supported sheet".into(),
        ));
    }
    // Neither `drawing` nor `legacyDrawing` anywhere: nothing to find.
    if !xml.contains("rawing") {
        return Ok(None);
    }
    project(xml, &root, true, false, None, true, None)?
        .1
        .drawing_relationship(legacy)
}

/// Cell spans in every understood Choice and Fallback. Implicit coordinates
/// resume from the selected representation, rather than counting alternatives.
pub fn compatible_cells(xml: &str, references: &HashSet<CellRef>) -> Result<Vec<CellSource>> {
    Ok(
        project(xml, "worksheet", true, true, Some(references), false, None)?
            .1
            .cells,
    )
}

fn record_leaf(
    cell: &mut CellSource,
    span: std::ops::Range<usize>,
    formula: bool,
    direct: bool,
    selected: bool,
    preferred: bool,
) {
    if formula {
        cell.has_formula = true;
        cell.selected_formula |= selected;
        if preferred {
            cell.preferred_formula = Some(span.clone());
            cell.preferred_formula_nested = !direct;
        }
    } else {
        cell.preferred_has_value |= preferred;
        cell.values.push(span);
    }
}

fn main_namespace(ns: &[u8]) -> bool {
    matches!(
        ns,
        b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
            | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
    )
}
/// An element's local name, borrowed for those every sheet is made of so
/// that reading one does not allocate per cell.
fn element_name(local: &[u8]) -> Cow<'static, str> {
    match local {
        b"c" => Cow::Borrowed("c"),
        b"v" => Cow::Borrowed("v"),
        b"f" => Cow::Borrowed("f"),
        b"row" => Cow::Borrowed("row"),
        b"is" => Cow::Borrowed("is"),
        b"t" => Cow::Borrowed("t"),
        b"r" => Cow::Borrowed("r"),
        b"rPr" => Cow::Borrowed("rPr"),
        b"si" => Cow::Borrowed("si"),
        b"sheetData" => Cow::Borrowed("sheetData"),
        _ => Cow::Owned(String::from_utf8_lossy(local).into_owned()),
    }
}
fn allowed(path: &[Cow<'static, str>], name: &str) -> bool {
    let parent = path.last().map(|n| n.as_ref()).unwrap_or("");
    match parent {
        "worksheet" => matches!(
            name,
            "dimension"
                | "sheetPr"
                | "sheetViews"
                | "sheetFormatPr"
                | "cols"
                | "sheetData"
                | "mergeCells"
                | "headerFooter"
                | "rowBreaks"
                | "colBreaks"
                | "pageSetup"
                | "pageMargins"
                | "printOptions"
                | "sheetProtection"
                | "drawing"
                | "legacyDrawing"
        ),
        "chartsheet" | "dialogsheet" => matches!(name, "drawing" | "legacyDrawing"),
        "comments" => matches!(name, "authors" | "commentList"),
        "authors" => name == "author",
        "commentList" => name == "comment",
        "comment" => name == "text",
        "text" => matches!(name, "t" | "r" | "rPh" | "phoneticPr"),
        "sheetPr" => matches!(name, "tabColor" | "outlinePr" | "pageSetUpPr"),
        "sheetViews" => name == "sheetView",
        "sheetView" => name == "pane",
        "cols" => name == "col",
        "sheetData" => name == "row",
        "row" => name == "c",
        "c" => matches!(name, "v" | "f" | "is"),
        "sst" => name == "si",
        "is" | "si" => matches!(name, "t" | "r" | "rPh" | "phoneticPr"),
        "r" => matches!(name, "t" | "rPr"),
        "rPr" => matches!(
            name,
            "b" | "i"
                | "u"
                | "strike"
                | "sz"
                | "rFont"
                | "name"
                | "color"
                | "vertAlign"
                | "charset"
                | "family"
                | "scheme"
                | "outline"
                | "shadow"
                | "condense"
                | "extend"
        ),
        "rPh" => name == "t",
        "mergeCells" => name == "mergeCell",
        "rowBreaks" | "colBreaks" => name == "brk",
        "headerFooter" => matches!(
            name,
            "oddHeader" | "oddFooter" | "evenHeader" | "evenFooter" | "firstHeader" | "firstFooter"
        ),
        _ => false,
    }
}
fn text_leaf(path: &[Cow<'static, str>]) -> bool {
    path.last().is_some_and(|n| {
        matches!(
            n.as_ref(),
            "v" | "f"
                | "t"
                | "author"
                | "oddHeader"
                | "oddFooter"
                | "evenHeader"
                | "evenFooter"
                | "firstHeader"
                | "firstFooter"
        )
    })
}
fn emit(writer: &mut Writer<Vec<u8>>, event: Event<'_>, enabled: bool) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    writer.write_event(event).map_err(quick_xml::Error::from)?;
    Ok(())
}
/// An attribute's value, borrowed from the tag when it needs no unescaping:
/// read for every row and cell of a sheet, so it allocates as little as it
/// can.
fn attribute<'b>(e: &'b BytesStart<'_>, name: &[u8]) -> Result<Option<Cow<'b, str>>> {
    for a in e.attributes().with_checks(false) {
        let a = a?;
        if a.key.as_ref() == name {
            if let Cow::Borrowed(raw) = a.value {
                if !raw.contains(&b'&') {
                    if let Ok(text) = std::str::from_utf8(raw) {
                        return Ok(Some(Cow::Borrowed(text)));
                    }
                }
            }
            return Ok(Some(Cow::Owned(a.unescape_value()?.into_owned())));
        }
    }
    Ok(None)
}
fn address(cell: CellRef) -> String {
    let mut col = u64::from(cell.column) + 1;
    let mut letters = Vec::new();
    while col > 0 {
        letters.push(b'A' + ((col - 1) % 26) as u8);
        col = (col - 1) / 26;
    }
    letters.reverse();
    format!(
        "{}{}",
        String::from_utf8(letters).expect("ASCII column"),
        u64::from(cell.row) + 1
    )
}

fn project(
    xml: &str,
    root: &str,
    record_structure: bool,
    all_known: bool,
    references: Option<&HashSet<CellRef>>,
    metadata_only: bool,
    stop_after_row: Option<u32>,
) -> Result<(String, WorksheetSource)> {
    let mut reader = NsReader::from_str(xml);
    let mut writer = Writer::new(Vec::new());
    let mut source = WorksheetSource::default();
    let mut frames: Vec<Frame> = Vec::new();
    let mut path: Vec<Cow<'static, str>> = Vec::new();
    let mut namespace = None::<Vec<u8>>;
    let mut coordinates: Coordinates = (0, 0, 0);
    let mut current_cell = None::<CellSource>;
    let mut cell_frame = 0usize;
    source.ordered = true;
    let mut last_row = None::<u32>;
    let mut last_column = None::<u32>;
    // Looking for given cells in a sheet without compatibility branches,
    // where each cell is written once: done once all are found.
    let stop_when_found = references
        .filter(|wanted| all_known && !wanted.is_empty() && !xml.contains("AlternateContent"))
        .map(HashSet::len);
    loop {
        if stop_when_found.is_some_and(|wanted| source.cells.len() >= wanted)
            && current_cell.is_none()
        {
            break;
        }
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element_name(e.local_name().as_ref());
                // The element's namespace, compared where it is resolved; it
                // is copied only where it is kept: the root's, and a cell's.
                let (is_mc, is_main, in_namespace, kept_ns) = {
                    let ns: &[u8] = match reader.resolver().resolve_element(e.name()).0 {
                        ResolveResult::Unbound => b"",
                        ResolveResult::Bound(n) => n.0,
                        _ => b"?unknown",
                    };
                    (
                        ns == MC_NAMESPACE,
                        main_namespace(ns),
                        namespace.as_deref() == Some(ns),
                        // A cell's namespace is kept only where the cell
                        // is recorded.
                        (path.is_empty() || (all_known && name == "c")).then(|| ns.to_vec()),
                    )
                };
                let ns = kept_ns.as_deref().unwrap_or_default();
                if is_mc && name == "AlternateContent" && !path.is_empty() {
                    source.ordered = false;
                    if !empty {
                        frames.push(Frame::Alternate {
                            chosen: false,
                            before: coordinates,
                            selected_after: None,
                            contains_value: false,
                            preferred_branch: None,
                            depth: frames.len(),
                        });
                    }
                    continue;
                }
                if is_mc && matches!(name.as_ref(), "Choice" | "Fallback") {
                    let supported = name == "Fallback"
                        || crate::data_validations::choice_supported(&reader, e)?;
                    let (include, selected) = if let Some(Frame::Alternate {
                        chosen,
                        before,
                        selected_after,
                        ..
                    }) = frames.last_mut()
                    {
                        let selected = !*chosen && supported;
                        if selected {
                            *chosen = true;
                            if empty {
                                *selected_after = Some(*before);
                            }
                        }
                        if all_known {
                            coordinates = *before;
                        }
                        (supported && (all_known || selected), selected)
                    } else {
                        (false, false)
                    };
                    if include {
                        let local = current_cell.is_some()
                            && path.last().is_some_and(|n| n == "c")
                            && frames.iter().skip(cell_frame + 1).all(|f| {
                                !matches!(
                                    f,
                                    Frame::Branch {
                                        selected: false,
                                        ..
                                    }
                                )
                            });
                        if empty && selected && local {
                            if let Some(Frame::Alternate {
                                preferred_branch, ..
                            }) = frames.last_mut()
                            {
                                *preferred_branch = Some(CacheInsertion::OpenEmpty(start..end));
                            }
                        }
                        if !empty {
                            frames.push(Frame::Branch {
                                selected,
                                head_end: end,
                            });
                        }
                    } else if !empty {
                        reader.read_to_end(e.name())?;
                    }
                    continue;
                }
                let owned = if path.is_empty() {
                    name == root && namespace.is_none() && is_main
                } else {
                    in_namespace
                        && allowed(&path, &name)
                        && (!metadata_only
                            || (path.len() == 1
                                && matches!(name.as_ref(), "drawing" | "legacyDrawing")))
                        && !matches!(frames.last(), Some(Frame::Alternate { .. }))
                };
                if !owned {
                    if !empty {
                        reader.read_to_end(e.name())?;
                    }
                    continue;
                }
                if path.is_empty() {
                    namespace = Some(ns.to_vec());
                }
                let mut clean = BytesStart::new(name.as_ref());
                let drawing =
                    if path.len() == 1 && matches!(name.as_ref(), "drawing" | "legacyDrawing") {
                        Some(usize::from(name == "legacyDrawing"))
                    } else {
                        None
                    };
                let mut drawing_ids = Vec::new();
                // Only the projected text needs the cleaned attributes; the
                // offset views read just the drawing relationship.
                for a in e
                    .attributes()
                    .filter(|_| !record_structure || drawing.is_some())
                {
                    let a = a?;
                    if drawing.is_some() {
                        let (uri, local) = reader.resolver().resolve_attribute(a.key);
                        if local.as_ref() == b"id"
                            && matches!(uri,ResolveResult::Bound(n) if matches!(n.as_ref(),b"http://schemas.openxmlformats.org/officeDocument/2006/relationships"|b"http://purl.oclc.org/ooxml/officeDocument/relationships"))
                        {
                            drawing_ids.push(a.unescape_value()?.into_owned());
                        }
                    }
                    let key = a.key.as_ref();
                    if !record_structure
                        && key != b"xmlns"
                        && (!key.contains(&b':') || key == b"xml:space")
                    {
                        let value = a.unescape_value()?;
                        clean.push_attribute((
                            String::from_utf8_lossy(key).as_ref(),
                            value.as_ref(),
                        ));
                    }
                }
                if let Some(kind) = drawing {
                    source.drawing_links[kind].push(if drawing_ids.len() == 1 {
                        drawing_ids.pop()
                    } else {
                        None
                    });
                }
                if path.is_empty() && !ns.is_empty() {
                    clean.push_attribute(("xmlns", String::from_utf8_lossy(ns).as_ref()));
                }
                let is_cell = name == "c" && path.last().is_some_and(|n| n == "row");
                if name == "row" && path.last().is_some_and(|n| n == "sheetData") {
                    coordinates.1 = attribute(e, b"r")?
                        .and_then(|v| v.parse::<u32>().ok())
                        .filter(|v| *v > 0)
                        .map(|v| v - 1)
                        .unwrap_or(coordinates.0);
                    coordinates.0 = coordinates.1.saturating_add(1);
                    coordinates.2 = 0;
                    if last_row.is_some_and(|last| coordinates.1 <= last) {
                        source.ordered = false;
                    }
                    last_row = Some(coordinates.1);
                    last_column = None;
                } else if is_cell {
                    let explicit = attribute(e, b"r")?;
                    let cell = explicit
                        .as_ref()
                        .and_then(|v| CellRef::parse(v).ok())
                        .unwrap_or(CellRef::new(coordinates.2, coordinates.1));
                    coordinates.2 = cell.column.saturating_add(1);
                    if last_column.is_some_and(|last| cell.column <= last) {
                        source.ordered = false;
                    }
                    last_column = Some(cell.column);
                    if explicit.is_none() && !record_structure {
                        clean.push_attribute(("r", address(cell).as_str()));
                    }
                    if all_known && references.is_none_or(|wanted| wanted.contains(&cell)) {
                        let parsed = CellSource {
                            reference: cell,
                            namespace: String::from_utf8_lossy(ns).into_owned(),
                            range: start..end,
                            head_end: end,
                            selected: frames.iter().all(|f| {
                                !matches!(
                                    f,
                                    Frame::Branch {
                                        selected: false,
                                        ..
                                    }
                                )
                            }),
                            has_formula: false,
                            selected_formula: false,
                            preferred_has_value: false,
                            preferred_formula: None,
                            preferred_formula_nested: false,
                            preferred_branch: None,
                            branch_priority: (false, 0),
                            values: Vec::new(),
                        };
                        if empty {
                            source.cells.push(parsed);
                        } else {
                            current_cell = Some(parsed);
                            cell_frame = frames.len();
                        }
                    }
                }
                let leaf = if let Some(cell) = current_cell
                    .as_mut()
                    .filter(|_| all_known && matches!(name.as_ref(), "f" | "v"))
                {
                    let direct = frames.len() == cell_frame + 1;
                    if name == "v" {
                        for frame in frames.iter_mut().skip(cell_frame + 1) {
                            if let Frame::Alternate { contains_value, .. } = frame {
                                *contains_value = true;
                            }
                        }
                    }
                    let formula = name == "f";
                    let selected = frames.iter().all(|f| {
                        !matches!(
                            f,
                            Frame::Branch {
                                selected: false,
                                ..
                            }
                        )
                    });
                    let preferred = frames.iter().skip(cell_frame + 1).all(|f| {
                        !matches!(
                            f,
                            Frame::Branch {
                                selected: false,
                                ..
                            }
                        )
                    });
                    if empty {
                        record_leaf(cell, start..end, formula, direct, selected, preferred);
                        None
                    } else {
                        Some((start, formula, direct, selected))
                    }
                } else {
                    None
                };
                let record = record_structure
                    && !all_known
                    && matches!(
                        name.as_ref(),
                        "worksheet"
                            | "chartsheet"
                            | "dialogsheet"
                            | "dimension"
                            | "sheetData"
                            | "row"
                            | "c"
                            | "f"
                            | "v"
                            | "drawing"
                            | "legacyDrawing"
                    );
                if record {
                    source.starts.push(start);
                }
                if empty && name == "si" {
                    emit(&mut writer, Event::Start(clean), !record_structure)?;
                    emit(
                        &mut writer,
                        Event::End(BytesEnd::new("si")),
                        !record_structure,
                    )?;
                } else if empty {
                    emit(&mut writer, Event::Empty(clean), !record_structure)?;
                } else {
                    emit(&mut writer, Event::Start(clean), !record_structure)?;
                    path.push(name);
                    frames.push(Frame::Element {
                        record,
                        cell: all_known && is_cell,
                        leaf,
                    });
                }
            }
            Event::End(_) => match frames.pop() {
                Some(Frame::Element { record, cell, leaf }) => {
                    if record {
                        source.ends.push(start);
                    }
                    if let Some((leaf_start, formula, direct, selected)) = leaf {
                        if let Some(cell) = current_cell.as_mut() {
                            let preferred = frames.iter().skip(cell_frame + 1).all(|f| {
                                !matches!(
                                    f,
                                    Frame::Branch {
                                        selected: false,
                                        ..
                                    }
                                )
                            });
                            record_leaf(
                                cell,
                                leaf_start..end,
                                formula,
                                direct,
                                selected,
                                preferred,
                            );
                        }
                    }
                    if cell {
                        if let Some(mut cell) = current_cell.take() {
                            cell.range.end = end;
                            source.cells.push(cell);
                        }
                    }
                    // Element frames and path entries are pushed together.
                    let name = path.pop().unwrap_or_default();
                    // A row past the last one asked about closes the read.
                    if name == "row"
                        && path.last().is_some_and(|parent| parent == "sheetData")
                        && stop_after_row.is_some_and(|last| coordinates.1 > last)
                        && !frames
                            .iter()
                            .any(|frame| matches!(frame, Frame::Alternate { .. }))
                    {
                        break;
                    }
                    emit(
                        &mut writer,
                        Event::End(BytesEnd::new(name)),
                        !record_structure,
                    )?;
                }
                Some(Frame::Branch { selected, head_end }) if all_known => {
                    let local = current_cell.is_some()
                        && path.last().is_some_and(|n| n == "c")
                        && frames.iter().skip(cell_frame + 1).all(|f| {
                            !matches!(
                                f,
                                Frame::Branch {
                                    selected: false,
                                    ..
                                }
                            )
                        });
                    if let Some(Frame::Alternate {
                        before,
                        selected_after,
                        preferred_branch,
                        ..
                    }) = frames.last_mut()
                    {
                        if selected {
                            *selected_after = Some(coordinates);
                            if local {
                                *preferred_branch = Some(CacheInsertion::InBranch(head_end));
                            }
                        }
                        coordinates = *before;
                    }
                }
                Some(Frame::Alternate {
                    before,
                    selected_after,
                    contains_value,
                    preferred_branch,
                    depth,
                    ..
                }) => {
                    if all_known {
                        coordinates = selected_after.unwrap_or(before);
                    }
                    if let (Some(cell), Some(slot)) = (current_cell.as_mut(), preferred_branch) {
                        let priority = (contains_value, depth);
                        if cell.preferred_branch.is_none() || priority > cell.branch_priority {
                            cell.preferred_branch = Some(slot);
                            cell.branch_priority = priority;
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            event
                if text_leaf(&path) && !matches!(frames.last(), Some(Frame::Alternate { .. })) =>
            {
                emit(&mut writer, event, !record_structure)?;
            }
            _ => {}
        }
    }
    Ok((
        String::from_utf8(writer.into_inner()).expect("UTF-8 input and ASCII element names"),
        source,
    ))
}
