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
use quick_xml::name::QName;
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
        Some(xml) => jade_opc::parse_relationships(xml)?,
        None => Default::default(),
    };

    // The comments part.
    match relationships.target_by_type_suffix("/comments") {
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
    match relationships.target_by_type_suffix("/vmlDrawing") {
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
                |prefix| format!("<{prefix}legacyDrawing r:id=\"{id}\"/>"),
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
    let relationships = jade_opc::parse_relationships(&rels_xml)?;
    let mut removed = false;
    for relationship in relationships.by_type_suffix("/comments") {
        let part = jade_opc::resolve_target(sheet_part, &relationship.target);
        let Some(xml) = package.part_str(&part)? else {
            continue;
        };
        let (patched, found, remaining) = remove_comment(&xml, at)?;
        if !found {
            continue;
        }
        removed = true;
        if remaining == 0 {
            drop_part(package, &part, &rels_name, &relationship.id)?;
        } else {
            package.replace(&part, patched.into_bytes());
        }
    }
    if removed {
        for relationship in relationships.by_type_suffix("/vmlDrawing") {
            let part = jade_opc::resolve_target(sheet_part, &relationship.target);
            let Some(xml) = package.part_str(&part)? else {
                continue;
            };
            let patched = remove_vml_note(&xml, at).unwrap_or_else(|_| xml.clone());
            if patched != xml {
                package.replace(&part, patched.into_bytes());
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
        escape(author),
        comment_xml(at, 0, text, "")
    )
}

/// The comments part with the note on `at` set to `text` by `author`.
pub fn upsert_comment(xml: &str, at: CellRef, author: &str, text: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut prefix = String::new();
    let mut authors: Vec<String> = Vec::new();
    let mut author_text: Option<String> = None;
    let mut authors_end: Option<usize> = None;
    let mut existing: Option<(usize, usize)> = None;
    // The first comment that sorts after `at`, to keep the list in order.
    let mut insert_before: Option<usize> = None;
    let mut list_end: Option<usize> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                match local(&name) {
                    b"comments" => prefix = crate::patch::namespace_prefix(&name),
                    b"author" if !empty => author_text = Some(String::new()),
                    b"author" => authors.push(String::new()),
                    b"comment" => {
                        let this = attribute(element, b"ref")?
                            .and_then(|value| CellRef::parse(&value).ok());
                        if !empty {
                            reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                        }
                        let end = reader.buffer_position() as usize;
                        if this == Some(at) {
                            existing = Some((start, end));
                        } else if insert_before.is_none()
                            && this
                                .is_some_and(|this| (this.row, this.column) > (at.row, at.column))
                        {
                            insert_before = Some(start);
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref text) => {
                if let Some(author) = author_text.as_mut() {
                    author.push_str(&text.decode()?);
                }
            }
            Event::GeneralRef(ref reference) => {
                if let Some(author) = author_text.as_mut() {
                    if let Ok(name) = reference.decode() {
                        if let Ok(decoded) = quick_xml::escape::unescape(&format!("&{name};")) {
                            author.push_str(&decoded);
                        }
                    }
                }
            }
            Event::End(ref element) => match local(element.name().as_ref()) {
                b"author" => {
                    if let Some(author) = author_text.take() {
                        authors.push(author);
                    }
                }
                b"authors" => authors_end = Some(start),
                b"commentList" => list_end = Some(start),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let Some(list_end) = list_end else {
        return Err(WriteError::NoCommentList);
    };
    let mut splices = Vec::new();
    let author_id = match authors.iter().position(|name| name == author) {
        Some(index) => index,
        None => {
            if let Some(authors_end) = authors_end {
                splices.push(Splice {
                    start: authors_end,
                    end: authors_end,
                    text: format!("<{prefix}author>{}</{prefix}author>", escape(author)),
                });
            }
            authors.len()
        }
    };
    let comment = comment_xml(at, author_id, text, &prefix);
    match existing {
        Some((start, end)) => splices.push(Splice {
            start,
            end,
            text: comment,
        }),
        None => {
            let at = insert_before.unwrap_or(list_end);
            splices.push(Splice {
                start: at,
                end: at,
                text: comment,
            });
        }
    }
    Ok(apply(xml, splices))
}

/// The comments part without the note on `at`: the text, whether there was
/// one, and how many notes remain.
fn remove_comment(xml: &str, at: CellRef) -> Result<(String, bool, usize)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices = Vec::new();
    let mut found = false;
    let mut remaining = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"comment" =>
            {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                let this =
                    attribute(element, b"ref")?.and_then(|value| CellRef::parse(&value).ok());
                if !empty {
                    reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                }
                if this == Some(at) {
                    found = true;
                    splices.push(Splice {
                        start,
                        end: reader.buffer_position() as usize,
                        text: String::new(),
                    });
                } else {
                    remaining += 1;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok((apply(xml, splices), found, remaining))
}

const VML_SHAPETYPE: &str = r#"<v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype>"#;

/// The box a note is shown in: Excel's default size, placed to the right
/// of the cell and hidden until the cell is pointed at.
fn vml_note_shape(at: CellRef, shape_number: u32) -> String {
    let top_row = at.row.saturating_sub(1);
    let left = (at.column + 1) as f64 * 48.0;
    let top = top_row as f64 * 15.0;
    format!(
        "<v:shape id=\"_x0000_s{shape_number}\" type=\"#_x0000_t202\" style=\"position:absolute;margin-left:{left}pt;margin-top:{top}pt;width:108pt;height:59.25pt;z-index:{};visibility:hidden\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\"><v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/><v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"></div></v:textbox><x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{}, 15, {top_row}, 2, {}, 15, {}, 2</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{}</x:Row><x:Column>{}</x:Column></x:ClientData></v:shape>",
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

/// What the VML holds: each note shape's span and cell, the highest shape
/// number, whether the note shape type is declared, and where the document
/// ends.
struct VmlShapes {
    notes: Vec<((usize, usize), CellRef)>,
    max_shape_number: u32,
    has_note_type: bool,
    first_shape: Option<usize>,
    root_end: Option<usize>,
}

fn read_vml(xml: &str) -> Result<VmlShapes> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut buf = Vec::new();
    let mut shapes = VmlShapes {
        notes: Vec::new(),
        max_shape_number: 1024,
        has_note_type: false,
        first_shape: None,
        root_end: None,
    };
    let mut shape: Option<(usize, bool, Option<u32>, Option<u32>)> = None;
    let mut content: Option<(usize, &[u8])> = None;
    let mut depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let head_end = reader.buffer_position() as usize;
                match local(element.name().as_ref()) {
                    b"shapetype" => {
                        if attribute(element, b"id")?.as_deref() == Some("_x0000_t202") {
                            shapes.has_note_type = true;
                        }
                    }
                    b"shape" if !empty => {
                        shapes.first_shape.get_or_insert(start);
                        if let Some(number) = attribute(element, b"id")?
                            .and_then(|id| id.strip_prefix("_x0000_s").map(str::to_string))
                            .and_then(|number| number.parse::<u32>().ok())
                        {
                            shapes.max_shape_number = shapes.max_shape_number.max(number);
                        }
                        shape = Some((start, false, None, None));
                    }
                    b"ClientData" if shape.is_some() => {
                        if attribute(element, b"ObjectType")?.as_deref() == Some("Note") {
                            if let Some(shape) = shape.as_mut() {
                                shape.1 = true;
                            }
                        }
                    }
                    b"Row" if shape.is_some() && !empty => content = Some((head_end, b"Row")),
                    b"Column" if shape.is_some() && !empty => content = Some((head_end, b"Column")),
                    _ => {}
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(ref element) => {
                depth = depth.saturating_sub(1);
                match local(element.name().as_ref()) {
                    b"Row" | b"Column" => {
                        if let (Some((content_start, kind)), Some(shape)) =
                            (content.take(), shape.as_mut())
                        {
                            let value = xml[content_start..start].trim().parse::<u32>().ok();
                            if kind == b"Row" {
                                shape.2 = value;
                            } else {
                                shape.3 = value;
                            }
                        }
                    }
                    b"shape" => {
                        if let Some((shape_start, note, row, column)) = shape.take() {
                            let end = reader.buffer_position() as usize;
                            if let (true, Some(row), Some(column)) = (note, row, column) {
                                shapes
                                    .notes
                                    .push(((shape_start, end), CellRef::new(column, row)));
                            }
                        }
                    }
                    _ => {}
                }
                if depth == 0 {
                    shapes.root_end = Some(start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(shapes)
}

/// The VML with a note box for `at`, replacing the one it has.
pub fn upsert_vml_note(xml: &str, at: CellRef) -> Result<String> {
    let shapes = read_vml(xml)?;
    let Some(root_end) = shapes.root_end else {
        return Ok(xml.to_string());
    };
    let mut splices = Vec::new();
    if !shapes.has_note_type {
        let at = shapes.first_shape.unwrap_or(root_end);
        splices.push(Splice {
            start: at,
            end: at,
            text: VML_SHAPETYPE.to_string(),
        });
    }
    let shape = vml_note_shape(at, shapes.max_shape_number + 1);
    match shapes.notes.iter().find(|(_, cell)| *cell == at) {
        Some(((start, end), _)) => splices.push(Splice {
            start: *start,
            end: *end,
            text: shape,
        }),
        None => splices.push(Splice {
            start: root_end,
            end: root_end,
            text: shape,
        }),
    }
    Ok(apply(xml, splices))
}

fn remove_vml_note(xml: &str, at: CellRef) -> Result<String> {
    let shapes = read_vml(xml)?;
    let splices = shapes
        .notes
        .iter()
        .filter(|(_, cell)| *cell == at)
        .map(|((start, end), _)| Splice {
            start: *start,
            end: *end,
            text: String::new(),
        })
        .collect();
    Ok(apply(xml, splices))
}

/// `prefix{n}{suffix}` for the smallest `n` the package does not have.
fn free_part_name(package: &OpcPackage, prefix: &str, suffix: &str) -> String {
    (1u32..)
        .map(|n| format!("{prefix}{n}{suffix}"))
        .find(|name| package.part(name).is_none())
        .expect("an unused part name")
}

/// `part` as a target relative to `from`'s directory, the way Excel writes
/// relationship targets (`../comments1.xml` from a worksheet).
fn relative_target(from: &str, part: &str) -> String {
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

fn ensure_default(package: &mut OpcPackage, extension: &str, content_type: &str) -> Result<()> {
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
