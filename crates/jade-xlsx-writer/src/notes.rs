//! Cell notes — the legacy comments current Excel calls notes.
//!
//! A note lives in two parts: the comments part holds the text, and the
//! VML drawing holds the yellow box it is shown in, with the cell it belongs
//! to repeated in `x:Row` / `x:Column`. Excel needs both, plus the
//! relationships from the sheet to each, the sheet's `<legacyDrawing>`
//! pointing at the VML, and the content types — so a workbook with no notes
//! gains all of that with its first one, and a workbook whose last note goes
//! keeps its VML part (empty of notes) rather than losing the relationship a
//! form control might share.

use quick_xml::events::Event;
use quick_xml::Reader;

use jade_opc::OpcPackage;
use jade_xlsx_reader::CellRef;

use crate::header_footer::set_child;
use crate::patch::{apply, attribute, escape, local, Splice};
use crate::shift::{drop_part, reference_text, rewrite};
use crate::{Result, WriteError};

const COMMENTS_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const VML_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing";
const COMMENTS_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml";
const VML_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.vmlDrawing";

fn owned_comment_target(rels: &jade_opc::Relationships) -> Result<Option<&str>> {
    let items: Vec<_> = rels
        .by_type_suffix("/comments")
        .into_iter()
        .filter(|r| jade_xlsx_reader::object_protection::office_relationship(r, "comments"))
        .collect();
    if items.len() > 1 {
        return Err(jade_xlsx_reader::XlsxError::BadDrawingProtection(
            "multiple owned comments relationships".into(),
        )
        .into());
    }
    Ok(items.first().map(|r| r.target.as_str()))
}
fn selected_vml_target<'a>(
    package: &OpcPackage,
    sheet: &str,
    rels: &'a jade_opc::Relationships,
) -> Result<Option<&'a str>> {
    let xml = package.required_part_str(sheet)?;
    let Some(id) = jade_xlsx_reader::object_protection::worksheet_drawing_relationship(&xml, true)?
    else {
        return Ok(None);
    };
    let r = rels
        .relationship(&id)
        .filter(|r| jade_xlsx_reader::object_protection::office_relationship(r, "vmlDrawing"))
        .ok_or_else(|| {
            jade_xlsx_reader::XlsxError::BadDrawingProtection(
                "invalid selected VML relationship".into(),
            )
        })?;
    Ok(Some(r.target.as_str()))
}

/// Puts a note on `at` — replacing the one it has — in the sheet at
/// `sheet_part`, creating the comments and VML parts when the sheet has
/// none.
pub fn set_note(
    package: &mut OpcPackage,
    sheet_part: &str,
    at: CellRef,
    author: &str,
    text: &str,
) -> Result<()> {
    let rels_name = jade_opc::rels_part_name(sheet_part);
    let mut rels_xml = package.part_str(&rels_name)?;
    let relationships = match &rels_xml {
        Some(xml) => jade_xlsx_reader::object_protection::owned_relationships(xml)?,
        None => Default::default(),
    };

    // The comments part.
    match owned_comment_target(&relationships)? {
        Some(target) => {
            let part = jade_opc::resolve_target(sheet_part, target);
            let xml = package
                .part_str(&part)?
                .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
            let patched = upsert_comment(&xml, at, author, text)?;
            package.replace(&part, patched.into_bytes());
        }
        None => {
            let part = free_part_name(package, "xl/comments", ".xml");
            let xml = new_comments_part(at, author, text);
            package.replace(&part, xml.into_bytes());
            let (xml, _) = add_relationship(
                rels_xml.as_deref(),
                COMMENTS_TYPE,
                &relative_target(sheet_part, &part),
            )?;
            rels_xml = Some(xml);
            ensure_override(package, &part, COMMENTS_CONTENT_TYPE)?;
        }
    }

    // The VML part that places it.
    match selected_vml_target(package, sheet_part, &relationships)? {
        Some(target) => {
            let part = jade_opc::resolve_target(sheet_part, target);
            let xml = package
                .part_str(&part)?
                .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
            let patched = upsert_vml_note(&xml, at)?;
            package.replace(&part, patched.into_bytes());
        }
        None => {
            let part = free_part_name(package, "xl/drawings/vmlDrawing", ".vml");
            package.replace(&part, new_vml_part(at).into_bytes());
            let (xml, id) = add_relationship(
                rels_xml.as_deref(),
                VML_TYPE,
                &relative_target(sheet_part, &part),
            )?;
            rels_xml = Some(xml);
            ensure_default(package, "vml", VML_CONTENT_TYPE)?;
            let sheet_xml = package
                .part_str(sheet_part)?
                .ok_or_else(|| WriteError::MissingPart(sheet_part.to_string()))?;
            let sheet_xml = set_child(
                &sheet_xml,
                b"legacyDrawing",
                |prefix| {
                    format!("<{prefix}legacyDrawing xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"{id}\"/>")
                },
                &[
                    b"legacyDrawingHF",
                    b"drawingHF",
                    b"picture",
                    b"oleObjects",
                    b"controls",
                    b"webPublishItems",
                    b"tableParts",
                    b"extLst",
                ],
            )?;
            package.replace(sheet_part, sheet_xml.into_bytes());
        }
    }

    if let Some(xml) = rels_xml {
        package.replace(&rels_name, xml.into_bytes());
    }
    Ok(())
}

/// Removes the note on `at`. `false` when the cell had none.
pub fn delete_note(package: &mut OpcPackage, sheet_part: &str, at: CellRef) -> Result<bool> {
    let rels_name = jade_opc::rels_part_name(sheet_part);
    let Some(rels_xml) = package.part_str(&rels_name)? else {
        return Ok(false);
    };
    let relationships = jade_xlsx_reader::object_protection::owned_relationships(&rels_xml)?;
    let vml = selected_vml_target(package, sheet_part, &relationships)?.map(str::to_string);
    let registry_plain =
        jade_xlsx_reader::object_protection::plain_package_registry(&rels_xml, false)?
            && package.part_str("[Content_Types].xml")?.is_some_and(|xml| {
                jade_xlsx_reader::object_protection::plain_package_registry(&xml, true)
                    .unwrap_or(false)
            });
    let mut removed = false;
    for relationship in relationships
        .by_type_suffix("/comments")
        .into_iter()
        .filter(|r| jade_xlsx_reader::object_protection::office_relationship(r, "comments"))
    {
        let part = jade_opc::resolve_target(sheet_part, &relationship.target);
        let Some(xml) = package.part_str(&part)? else {
            continue;
        };
        let (patched, found, discard_empty) = remove_comment(&xml, at)?;
        if !found {
            continue;
        }
        removed = true;
        if discard_empty && registry_plain {
            drop_part(package, &part, &rels_name, &relationship.id)?;
        } else {
            package.replace(&part, patched.into_bytes());
        }
    }
    if removed {
        if let Some(target) = vml {
            let part = jade_opc::resolve_target(sheet_part, &target);
            if let Some(xml) = package.part_str(&part)? {
                let patched = remove_vml_note(&xml, at)?;
                if patched != xml {
                    package.replace(&part, patched.into_bytes());
                }
            }
        }
    }
    Ok(removed)
}

fn comment_xml(at: CellRef, author_id: usize, text: &str, prefix: &str) -> String {
    format!(
        "<{prefix}comment ref=\"{}\" authorId=\"{author_id}\"><{prefix}text><{prefix}r><{prefix}rPr><{prefix}sz val=\"9\"/><{prefix}color indexed=\"81\"/><{prefix}rFont val=\"Tahoma\"/><{prefix}family val=\"2\"/></{prefix}rPr><{prefix}t xml:space=\"preserve\">{}</{prefix}t></{prefix}r></{prefix}text></{prefix}comment>",
        reference_text(at),
        escape(&jade_xlsx_reader::xstring::encode_xstring(text))
    )
}

fn new_comments_part(at: CellRef, author: &str, text: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><authors><author>{}</author></authors><commentList>{}</commentList></comments>",
        escape(&jade_xlsx_reader::xstring::encode_xstring(author)),
        comment_xml(at, 0, text, "")
    )
}

/// The comments part with the note on `at` set to `text` by `author`.
fn element_insert(
    xml: &str,
    e: &jade_xlsx_reader::object_protection::ElementSource,
    entry: &str,
) -> Splice {
    if e.empty {
        Splice {
            start: e.span.start,
            end: e.span.end,
            text: format!(
                "{}>{entry}</{}>",
                &xml[e.span.start..e.span.end - 2],
                e.qualified
            ),
        }
    } else {
        Splice {
            start: e.end_start,
            end: e.end_start,
            text: entry.into(),
        }
    }
}
fn element_contents(
    xml: &str,
    e: &jade_xlsx_reader::object_protection::ElementSource,
    entry: &str,
) -> String {
    if e.empty {
        format!(
            "{}>{entry}</{}>",
            &xml[e.span.start..e.span.end - 2],
            e.qualified
        )
    } else {
        format!(
            "{}{entry}{}",
            &xml[e.span.start..e.head_end],
            &xml[e.end_start..e.span.end]
        )
    }
}
pub fn upsert_comment(xml: &str, at: CellRef, author: &str, text: &str) -> Result<String> {
    let source = jade_xlsx_reader::object_protection::comments_source(xml)?;
    let mut name = "jadeComment".to_string();
    while xml.contains(&name) {
        name.push('_');
    }
    let prefix = if source.namespace.is_empty() {
        String::new()
    } else {
        format!("{name}:")
    };
    let declaration = if source.namespace.is_empty() {
        String::new()
    } else {
        format!(" xmlns:{name}=\"{}\"", escape(&source.namespace))
    };
    let mut splices = Vec::new();
    let author_id = if let Some(index) = source.authors.iter().position(|a| a == author) {
        index
    } else {
        let entry = format!(
            "<{prefix}author{declaration}>{}</{prefix}author>",
            escape(&jade_xlsx_reader::xstring::encode_xstring(author))
        );
        if let Some(authors) = &source.authors_element {
            splices.push(element_insert(xml, authors, &entry));
        } else {
            splices.push(Splice {
                start: source.list.span.start,
                end: source.list.span.start,
                text: format!("<{prefix}authors{declaration}>{entry}</{prefix}authors>"),
            });
        }
        source.authors.len()
    };
    let value = format!(
        "<{prefix}t{declaration} xml:space=\"preserve\">{}</{prefix}t>",
        escape(&jade_xlsx_reader::xstring::encode_xstring(text))
    );
    let replacement = if let Some(existing) = source.comments.iter().find(|c| c.at == Some(at)) {
        let body = |properties: &str, declarations: &str| {
            let properties = if properties.is_empty() {
                format!("<{prefix}rPr><{prefix}sz val=\"9\"/><{prefix}color indexed=\"81\"/><{prefix}rFont val=\"Tahoma\"/><{prefix}family val=\"2\"/></{prefix}rPr>")
            } else {
                properties.to_string()
            };
            format!("<{prefix}r{declaration}{declarations}>{properties}{value}</{prefix}r>")
        };
        let mut raw = xml[existing.element.span.clone()].to_string();
        if !existing.alternatives.is_empty() {
            let mut variants: Vec<_> = existing.alternatives.iter().collect();
            variants.sort_by_key(|v| v.element.span.start);
            for variant in variants.into_iter().rev() {
                let text = element_contents(
                    xml,
                    &variant.element,
                    &body(&variant.run_properties, &variant.declarations),
                );
                raw.replace_range(
                    variant.element.span.start - existing.element.span.start
                        ..variant.element.span.end - existing.element.span.start,
                    &text,
                );
            }
        } else {
            let entry = format!("<{prefix}text{declaration}>{}</{prefix}text>", body("", ""));
            let insertion = element_insert(xml, &existing.element, &entry);
            raw = apply(
                &raw,
                vec![Splice {
                    start: insertion.start - existing.element.span.start,
                    end: insertion.end - existing.element.span.start,
                    text: insertion.text,
                }],
            );
        }
        let raw =
            crate::xml_edit::root_attributes(&raw, &[("authorId", Some(author_id.to_string()))])?;
        Splice {
            start: existing.element.span.start,
            end: existing.element.span.end,
            text: raw,
        }
    } else {
        let mut entry = comment_xml(at, author_id, text, &prefix);
        if !declaration.is_empty() {
            entry = entry.replacen(
                &format!("<{prefix}comment "),
                &format!("<{prefix}comment{declaration} "),
                1,
            );
        }
        if source.list.empty {
            element_insert(xml, &source.list, &entry)
        } else {
            let pos = source
                .comments
                .iter()
                .find(|c| {
                    c.at.is_some_and(|v| (v.row, v.column) > (at.row, at.column))
                })
                .map_or(source.list.end_start, |c| c.element.span.start);
            Splice {
                start: pos,
                end: pos,
                text: entry,
            }
        }
    };
    splices.push(replacement);
    Ok(apply(xml, splices))
}
/// Remove only the owned note. Opaque root/list metadata prevents dropping the part.
fn remove_comment(xml: &str, at: CellRef) -> Result<(String, bool, bool)> {
    let source = jade_xlsx_reader::object_protection::comments_source(xml)?;
    let mut found = false;
    let mut remaining = 0;
    let mut splices = Vec::new();
    for c in source.comments {
        if c.at == Some(at) {
            found = true;
            splices.push(Splice {
                start: c.element.span.start,
                end: c.element.span.end,
                text: String::new(),
            });
        } else {
            remaining += 1;
        }
    }
    Ok((
        apply(xml, splices),
        found,
        remaining == 0 && source.can_drop_empty,
    ))
}

const VML_SHAPETYPE: &str = r#"<v:shapetype xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype>"#;

/// The box a note is shown in: Excel's default size, placed to the right
/// of the cell and hidden until the cell is pointed at.
fn vml_note_shape(at: CellRef, shape_number: u32) -> String {
    let top_row = at.row.saturating_sub(1);
    let left = (at.column + 1) as f64 * 48.0;
    let top = top_row as f64 * 15.0;
    format!(
        "<v:shape xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\" id=\"_x0000_s{shape_number}\" type=\"#_x0000_t202\" style=\"position:absolute;margin-left:{left}pt;margin-top:{top}pt;width:108pt;height:59.25pt;z-index:{};visibility:hidden\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\"><v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/><v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"></div></v:textbox><x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{}, 15, {top_row}, 2, {}, 15, {}, 2</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{}</x:Row><x:Column>{}</x:Column></x:ClientData></v:shape>",
        shape_number - 1024,
        at.column + 1,
        at.column + 3,
        top_row + 4,
        at.row,
        at.column
    )
}

fn new_vml_part(at: CellRef) -> String {
    format!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"1\"/></o:shapelayout>{VML_SHAPETYPE}{}</xml>",
        vml_note_shape(at, 1025)
    )
}

/// Existing note geometry, locking and extensions are unchanged by a text update.
pub fn upsert_vml_note(xml: &str, at: CellRef) -> Result<String> {
    let source = jade_xlsx_reader::object_protection::vml_sources(xml)?;
    let existing: Vec<_> = source.notes.iter().filter(|n| n.at == at).collect();
    if existing.iter().any(|n| n.locks.ambiguous) {
        return Err(jade_xlsx_reader::XlsxError::BadDrawingProtection(
            "ambiguous note identity or locks".into(),
        )
        .into());
    }
    if !existing.is_empty() {
        return Ok(xml.to_string());
    }
    let shape = vml_note_shape(
        at,
        source.max_shape_number.checked_add(1).ok_or_else(|| {
            jade_xlsx_reader::XlsxError::BadDrawingProtection("VML shape id overflow".into())
        })?,
    );
    let shape_type = if source.has_note_type {
        ""
    } else {
        VML_SHAPETYPE
    };
    if source.root_empty {
        let head = &xml[source.root_span.start..source.root_span.end - 2];
        return Ok(apply(
            xml,
            vec![Splice {
                start: source.root_span.start,
                end: source.root_span.end,
                text: format!("{head}>{shape_type}{shape}</{}>", source.root_qualified),
            }],
        ));
    }
    let mut splices = vec![Splice {
        start: source.root_end,
        end: source.root_end,
        text: shape,
    }];
    if !shape_type.is_empty() {
        let at = source.first_shape.unwrap_or(source.root_end);
        splices.insert(
            0,
            Splice {
                start: at,
                end: at,
                text: shape_type.into(),
            },
        );
    }
    Ok(apply(xml, splices))
}
fn remove_vml_note(xml: &str, at: CellRef) -> Result<String> {
    let notes = jade_xlsx_reader::object_protection::note_sources(xml)?;
    if notes.iter().any(|n| n.at == at && n.locks.ambiguous) {
        return Err(jade_xlsx_reader::XlsxError::BadDrawingProtection(
            "ambiguous note identity or locks".into(),
        )
        .into());
    }
    Ok(apply(
        xml,
        notes
            .into_iter()
            .filter(|n| n.at == at)
            .map(|n| Splice {
                start: n.span.start,
                end: n.span.end,
                text: String::new(),
            })
            .collect(),
    ))
}

/// `prefix{n}{suffix}` for the smallest `n` the package does not have.
pub(crate) fn free_part_name(package: &OpcPackage, prefix: &str, suffix: &str) -> String {
    (1u32..)
        .map(|n| format!("{prefix}{n}{suffix}"))
        .find(|name| package.part(name).is_none())
        .expect("an unused part name")
}

/// `part` as a target relative to `from`'s directory, the way Excel writes
/// relationship targets (`../comments1.xml` from a worksheet).
pub(crate) fn relative_target(from: &str, part: &str) -> String {
    let from_dir: Vec<&str> = from
        .rsplit_once('/')
        .map(|(dir, _)| dir.split('/').collect())
        .unwrap_or_default();
    let part_segments: Vec<&str> = part.split('/').collect();
    let shared = from_dir
        .iter()
        .zip(part_segments.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut target: Vec<&str> = vec![".."; from_dir.len() - shared];
    target.extend_from_slice(&part_segments[shared..]);
    target.join("/")
}

/// `rels` with a relationship of `relationship_type` to `target` added:
/// the new text, and the id given.
pub fn add_relationship(
    rels: Option<&str>,
    relationship_type: &str,
    target: &str,
) -> Result<(String, String)> {
    let xml = match rels {
        Some(xml) => xml.to_string(),
        None => "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".to_string(),
    };
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut max_id = 0u32;
    let mut root_end: Option<usize> = None;
    let mut empty_root: Option<(usize, usize)> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                match local(element.name().as_ref()) {
                    b"Relationship" => {
                        if let Some(number) = attribute(element, b"Id")?
                            .and_then(|id| id.strip_prefix("rId").map(str::to_string))
                            .and_then(|number| number.parse::<u32>().ok())
                        {
                            max_id = max_id.max(number);
                        }
                    }
                    b"Relationships" if empty => {
                        empty_root = Some((start, reader.buffer_position() as usize));
                    }
                    _ => {}
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"Relationships" => {
                root_end = Some(start);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let id = format!("rId{}", max_id + 1);
    let entry = format!(
        "<Relationship Id=\"{id}\" Type=\"{}\" Target=\"{}\"/>",
        escape(relationship_type),
        escape(target)
    );
    let splice = match (root_end, empty_root) {
        (Some(at), _) => Splice {
            start: at,
            end: at,
            text: entry,
        },
        (None, Some((start, end))) => {
            let head = &xml[start..end];
            let mut reader = Reader::from_str(head);
            let mut buf = Vec::new();
            let opened = match reader.read_event_into(&mut buf)? {
                Event::Empty(ref element) => rewrite(element, &[], false)?,
                _ => "<Relationships>".to_string(),
            };
            Splice {
                start,
                end,
                text: format!("{opened}{entry}</Relationships>"),
            }
        }
        (None, None) => return Err(WriteError::MissingPart("Relationships".to_string())),
    };
    Ok(apply(&xml, vec![splice])).map(|xml| (xml, id))
}

pub(crate) fn ensure_override(
    package: &mut OpcPackage,
    part: &str,
    content_type: &str,
) -> Result<()> {
    let Some(types) = package.part_str("[Content_Types].xml")? else {
        return Ok(());
    };
    let wanted = format!("/{part}");
    if types.contains(&format!("PartName=\"{wanted}\"")) {
        return Ok(());
    }
    let entry = format!(
        "<Override PartName=\"{}\" ContentType=\"{}\"/>",
        escape(&wanted),
        escape(content_type)
    );
    package.replace(
        "[Content_Types].xml",
        append_before_root_end(&types, b"Types", &entry)?.into_bytes(),
    );
    Ok(())
}

pub(crate) fn ensure_default(
    package: &mut OpcPackage,
    extension: &str,
    content_type: &str,
) -> Result<()> {
    let Some(types) = package.part_str("[Content_Types].xml")? else {
        return Ok(());
    };
    if types.contains(&format!("Extension=\"{extension}\"")) {
        return Ok(());
    }
    let entry = format!(
        "<Default Extension=\"{}\" ContentType=\"{}\"/>",
        escape(extension),
        escape(content_type)
    );
    package.replace(
        "[Content_Types].xml",
        append_before_root_end(&types, b"Types", &entry)?.into_bytes(),
    );
    Ok(())
}

fn append_before_root_end(xml: &str, root: &[u8], entry: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf)? {
            Event::End(ref element) if local(element.name().as_ref()) == root => {
                return Ok(apply(
                    xml,
                    vec![Splice {
                        start,
                        end: start,
                        text: entry.to_string(),
                    }],
                ));
            }
            Event::Eof => return Ok(xml.to_string()),
            _ => {}
        }
        buf.clear();
    }
}
