//! Context-aware rewrites of structured references in saved formula XML.
use crate::{
    patch::{apply, Splice},
    Result,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{CellRef, Table, Workbook};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
fn attribute(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for a in e.attributes() {
        let a = a?;
        if a.key.as_ref() == key {
            return Ok(Some(a.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
fn formula_ns(ns: &ResolveResult<'_>, local: &[u8]) -> bool {
    let valid = match ns {
        ResolveResult::Unbound => true,
        ResolveResult::Bound(ns) => matches!(
            ns.as_ref(),
            b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                | b"http://schemas.openxmlformats.org/drawingml/2006/chart"
                | b"http://purl.oclc.org/ooxml/drawingml/chart"
                | b"http://schemas.microsoft.com/office/excel/2006/main"
                | b"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"
                | b"http://schemas.microsoft.com/office/drawing/2014/chartex"
        ),
        _ => false,
    };
    valid
        && matches!(
            local,
            b"f" | b"formula"
                | b"formula1"
                | b"formula2"
                | b"calculatedColumnFormula"
                | b"totalsRowFormula"
                | b"definedName"
        )
}
struct Capture {
    start: usize,
    depth: usize,
    text: String,
    cell: Option<CellRef>,
    sheet: String,
}
type FormulaRewrite<'a> = dyn Fn(&str, Option<&Table>, Option<CellRef>) -> String + 'a;
type ContextFormulaRewrite<'a> = dyn Fn(&str, Option<&Table>, Option<CellRef>, &str) -> String + 'a;
pub(crate) fn rewrite_package(
    package: &mut OpcPackage,
    book: &Workbook,
    rewrite: &FormulaRewrite<'_>,
) -> Result<()> {
    rewrite_package_context(package, book, &|formula, table, at, _| {
        rewrite(formula, table, at)
    })
}

pub(crate) fn rewrite_grid_package(
    package: &mut OpcPackage,
    book: &Workbook,
    rewrite: &dyn Fn(&str, &str) -> String,
) -> Result<()> {
    rewrite_package_context(package, book, &|formula, _, _, sheet| {
        rewrite(formula, sheet)
    })
}

fn rewrite_package_context(
    package: &mut OpcPackage,
    book: &Workbook,
    rewrite: &ContextFormulaRewrite<'_>,
) -> Result<()> {
    let mut parts = crate::sheets::formula_parts(package)?;
    parts.insert(crate::workbook_part_name(package)?);
    for part in parts {
        let xml = package.required_part_str(&part)?;
        let sheet = book.sheets.iter().find(|s| s.part_name == part);
        let definition = book
            .sheets
            .iter()
            .flat_map(|s| &s.tables)
            .find(|t| t.part_name == part);
        let context_sheet = sheet
            .or_else(|| {
                definition.and_then(|table| {
                    book.sheets
                        .iter()
                        .find(|s| s.tables.iter().any(|t| t.part_name == table.part_name))
                })
            })
            .map(|s| s.name.as_str())
            .unwrap_or("");
        if let Some(sheet) = sheet {
            let changed = crate::worksheet_formulas::rewrite(&xml, &|formula, at| {
                rewrite(
                    formula,
                    at.and_then(|at| sheet.table_at(at)),
                    at,
                    context_sheet,
                )
            })?;
            if changed != xml {
                package.replace(&part, changed.into_bytes());
            }
            continue;
        }
        if let Some(table) = definition {
            let changed = crate::cell_shift::children_map(&xml, b"tableColumns", |columns| {
                crate::cell_shift::children_map(columns, b"tableColumn", |column| {
                    let id = crate::cell_shift::head_attr(column, b"id")?
                        .and_then(|s| s.parse::<u32>().ok());
                    let position = id
                        .and_then(|id| table.column_properties.iter().position(|c| c.id == id))
                        .unwrap_or(0);
                    let mut changed = column.to_owned();
                    for name in [b"calculatedColumnFormula".as_slice(), b"totalsRowFormula"] {
                        changed = crate::cell_shift::children_map(&changed, name, |leaf| {
                            let formula = crate::cell_shift::leaf_text(leaf)?;
                            let at = CellRef::new(
                                table.reference.start.column + position as u32,
                                if name == b"totalsRowFormula" {
                                    table.reference.end.row
                                } else {
                                    table.reference.start.row + table.header_row_count
                                },
                            );
                            let updated = rewrite(&formula, Some(table), Some(at), context_sheet);
                            if updated
                                .chars()
                                .any(|c| !crate::patch::is_xml_10_character(c))
                            {
                                return Err(crate::WriteError::Table(
                                    "a rewritten table formula contains an invalid XML character"
                                        .into(),
                                ));
                            }
                            if updated == formula {
                                Ok(leaf.into())
                            } else {
                                crate::cell_shift::text_content(leaf, &updated)
                            }
                        })?;
                    }
                    Ok(changed)
                })
            })?;
            if changed != xml {
                package.replace(&part, changed.into_bytes());
            }
            continue;
        }
        let mut r = NsReader::from_str(&xml);
        let mut depth = 0usize;
        let mut at = None;
        let mut captures: Option<Capture> = None;
        let mut splices = Vec::new();
        loop {
            let pos = r.buffer_position() as usize;
            let event = r.read_event()?;
            match event {
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let empty = matches!(event, Event::Empty(_));
                    let (ns, local) = r.resolver().resolve_element(e.name());
                    let local = local.as_ref();
                    // A nested formula element is the leaf; a wrapper containing
                    // XML is never interpreted as formula text.
                    captures = None;
                    if sheet.is_some() {
                        if local == b"c" {
                            at = attribute(e, b"r")?.and_then(|s| CellRef::parse(&s).ok());
                        } else if matches!(local, b"conditionalFormatting" | b"dataValidation") {
                            at = attribute(e, b"sqref")?
                                .and_then(|s| {
                                    s.split_ascii_whitespace()
                                        .next()
                                        .and_then(|s| jade_xlsx_reader::CellRange::parse(s).ok())
                                })
                                .map(|r| r.start);
                        }
                    }
                    if !empty && formula_ns(&ns, local) {
                        let cell = at;
                        captures = Some(Capture {
                            start: r.buffer_position() as usize,
                            depth: depth + 1,
                            text: String::new(),
                            cell,
                            sheet: if local == b"definedName" {
                                attribute(e, b"localSheetId")?
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .and_then(|index| book.sheets.get(index))
                                    .or_else(|| book.sheets.first())
                                    .map(|s| s.name.clone())
                                    .unwrap_or_default()
                            } else {
                                context_sheet.into()
                            },
                        });
                    }
                    if !empty {
                        depth += 1;
                    }
                }
                Event::Text(e) => {
                    if let Some(c) = captures.as_mut() {
                        if c.depth == depth {
                            c.text.push_str(&e.decode()?);
                        }
                    }
                }
                Event::CData(e) => {
                    if let Some(c) = captures.as_mut() {
                        if c.depth == depth {
                            c.text.push_str(&e.decode()?);
                        }
                    }
                }
                Event::GeneralRef(e) => {
                    if let Some(c) = captures.as_mut() {
                        if c.depth == depth {
                            let name = e.decode()?;
                            if let Ok(text) = quick_xml::escape::unescape(&format!("&{name};")) {
                                c.text.push_str(&text);
                            }
                        }
                    }
                }
                Event::End(e) => {
                    if captures.as_ref().is_some_and(|c| c.depth == depth) {
                        let c = captures.take().unwrap();
                        let context = definition
                            .or_else(|| sheet.and_then(|s| c.cell.and_then(|at| s.table_at(at))));
                        let updated = rewrite(&c.text, context, c.cell, &c.sheet);
                        if updated
                            .chars()
                            .any(|c| !crate::patch::is_xml_10_character(c))
                        {
                            return Err(crate::WriteError::Table(
                                "a rewritten table formula contains an invalid XML character"
                                    .into(),
                            ));
                        }
                        if updated != c.text {
                            splices.push(Splice {
                                start: c.start,
                                end: pos,
                                text: updated
                                    .replace('&', "&amp;")
                                    .replace('<', "&lt;")
                                    .replace('>', "&gt;"),
                            });
                        }
                    }
                    if matches!(
                        e.local_name().as_ref(),
                        b"c" | b"conditionalFormatting" | b"dataValidation"
                    ) {
                        at = None;
                    }
                    depth = depth.saturating_sub(1);
                }
                Event::Eof => break,
                _ => {}
            }
        }
        if !splices.is_empty() {
            package.replace(&part, apply(&xml, splices).into_bytes());
        }
    }
    Ok(())
}
