//! マスター文書 (master documents) and their サブ文書 (subdocuments), as
//! Word keeps them: a master's body holds a `w:subDoc` (ECMA-376 Part 1
//! §17.17.2.1) where each subdocument goes, naming through an external
//! `subDocument` relationship the `.docx` it is in.
//!
//! While a master is open, each subdocument is a block-level content control
//! (`w:sdt`) the editor carries like any other, tagged with what it is:
//!
//! - collapsed (折りたたみ), it holds one paragraph, the link to the file —
//!   its full path, as Word shows a collapsed subdocument;
//! - expanded (展開), it holds the subdocument's own paragraphs and tables,
//!   brought in with the styles, lists and pictures they use;
//! - locked (ロック), its content control is locked against editing too.
//!
//! [`for_editing`] turns a master as stored into that form and
//! [`for_saving`] turns it back, giving each expanded subdocument's new
//! contents to write to its own file. In between, the shells expand and
//! collapse, insert (挿入) a file, create (作成) one from a heading and the
//! paragraphs under it, unlink (リンク解除) one into the master, and lock
//! one. A place in the document is pointed at with [`MARK`], typed there.

use std::collections::{HashMap, HashSet};
use std::ops::{Range, RangeInclusive};

use crate::word_document::WordDocument;

use jade_opc::{parse_relationships, resolve_target, OpcPackage, Relationship};
use quick_xml::events::Event;
use quick_xml::Reader;

/// The relationship a `w:subDoc` names its file by.
pub const SUBDOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/subDocument";

/// A character the shells type where an insert or a create is to happen:
/// from the Private Use Area, so no document has it of its own.
pub const MARK: char = '\u{E000}';

const TAG: &str = "jade.subdoc";
const DOCUMENT: &str = "word/document.xml";
const DOCUMENT_RELS: &str = "word/_rels/document.xml.rels";
const NUMBERING: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";

/// What a subdocument's content control says it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// The relationship naming its file.
    pub rel_id: String,
    pub expanded: bool,
    pub locked: bool,
}

impl Tag {
    /// The tag of a subdocument's content control, or `None` for any other.
    pub fn parse(tag: &str) -> Option<Self> {
        let mut words = tag.split_whitespace();
        if words.next()? != TAG {
            return None;
        }
        let rel_id = words.next()?.to_string();
        let rest: Vec<&str> = words.collect();
        Some(Tag {
            rel_id,
            expanded: rest.contains(&"expanded"),
            locked: rest.contains(&"locked"),
        })
    }

    fn value(&self) -> String {
        format!(
            "{TAG} {} {}{}",
            self.rel_id,
            if self.expanded {
                "expanded"
            } else {
                "collapsed"
            },
            if self.locked { " locked" } else { "" }
        )
    }
}

/// A subdocument of an open master.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubDocument {
    pub rel_id: String,
    /// Its file, as the master names it: relative to the master's folder,
    /// or absolute.
    pub target: String,
    pub expanded: bool,
    pub locked: bool,
}

/// A subdocument's file, as read.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub bytes: Vec<u8>,
    /// The file cannot be written: it opens locked, as in Word.
    pub read_only: bool,
}

/// What the master's operations need of the files around it.
pub struct Files<'a> {
    /// A subdocument's file by its target, when it can be read.
    pub load: &'a dyn Fn(&str) -> Option<Loaded>,
    /// What a collapsed subdocument's link reads: Word shows its full path.
    pub display: &'a dyn Fn(&str) -> String,
}

/// A subdocument file to write, as a save or a collapse leaves it.
#[derive(Clone, Debug)]
pub struct Saved {
    pub target: String,
    pub bytes: Vec<u8>,
    /// Only its lock changed: writing it may fail on a file that cannot be
    /// written, which is what locked it.
    pub locked: bool,
}

/// Whether `docx` is a master document: it has a subdocument.
pub fn is_master(docx: &[u8]) -> bool {
    !subdocuments(docx).is_empty()
}

/// The master's subdocuments, in order.
pub fn subdocuments(docx: &[u8]) -> Vec<SubDocument> {
    let Ok(package) = OpcPackage::open(docx) else {
        return Vec::new();
    };
    let Ok(Some(xml)) = package.part_str(DOCUMENT) else {
        return Vec::new();
    };
    if !xml.contains("w:subDoc") && !xml.contains(TAG) {
        return Vec::new();
    }
    let rels = relationships(&package);
    let Some((_, blocks)) = body(&xml) else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|block| match block_kind(&xml, block) {
            Block::Raw(rel_id) => Some(Tag {
                rel_id,
                expanded: false,
                locked: false,
            }),
            Block::Subdocument(tag, _) => Some(tag),
            Block::Other => None,
        })
        .map(|tag| SubDocument {
            target: rels
                .iter()
                .find(|rel| rel.id == tag.rel_id)
                .map(|rel| rel.target.clone())
                .unwrap_or_default(),
            rel_id: tag.rel_id,
            expanded: tag.expanded,
            locked: tag.locked,
        })
        .collect()
}

/// The master as the editor holds it: each `w:subDoc` a collapsed
/// subdocument showing its link.
pub fn for_editing(docx: &[u8], files: &Files) -> Result<Vec<u8>, String> {
    let mut package = open(docx)?;
    let mut xml = document(&package)?;
    if !xml.contains("w:subDoc") {
        return Ok(docx.to_vec());
    }
    let rels = relationships(&package);
    let (_, blocks) = body(&xml).ok_or("the document has no body")?;
    for block in blocks.iter().rev() {
        if let Block::Raw(rel_id) = block_kind(&xml, block) {
            let target = target_of(&rels, &rel_id);
            let tag = Tag {
                rel_id,
                expanded: false,
                locked: false,
            };
            xml.replace_range(block.start..block.end, &collapsed(&tag, &target, files));
        }
    }
    package.replace(DOCUMENT, xml.into_bytes());
    write(&package)
}

/// The master as it is stored: each subdocument a `w:subDoc`, and the new
/// contents of each expanded one, to write to its file.
pub fn for_saving(docx: &[u8], files: &Files) -> Result<(Vec<u8>, Vec<Saved>), String> {
    let mut package = open(docx)?;
    let mut xml = document(&package)?;
    let rels = relationships(&package);
    let (_, blocks) = body(&xml).ok_or("the document has no body")?;
    let mut saved = Vec::new();
    for block in blocks.iter().rev() {
        if let Block::Subdocument(tag, content) = block_kind(&xml, block) {
            let target = target_of(&rels, &tag.rel_id);
            if tag.expanded {
                if let Some(file) =
                    subdocument_file(&xml[content], &package, &target, tag.locked, files)?
                {
                    saved.push(file);
                }
            }
            xml.replace_range(
                block.start..block.end,
                &format!(r#"<w:subDoc r:id="{}"/>"#, escape(&tag.rel_id)),
            );
        }
    }
    saved.reverse();
    package.replace(DOCUMENT, xml.into_bytes());
    Ok((write(&package)?, saved))
}

/// Expands the subdocument `only` names, or every collapsed one: its
/// file's paragraphs in place of its link. The targets that could not be
/// read stay collapsed and are returned.
pub fn expand(
    docx: &[u8],
    only: Option<&str>,
    files: &Files,
) -> Result<(Vec<u8>, Vec<String>), String> {
    let docx = for_editing(docx, files)?;
    let mut package = open(&docx)?;
    let mut xml = document(&package)?;
    let rels = relationships(&package);
    let (_, blocks) = body(&xml).ok_or("the document has no body")?;
    let mut unreachable = Vec::new();
    for block in blocks.iter().rev() {
        let Block::Subdocument(tag, _) = block_kind(&xml, block) else {
            continue;
        };
        if tag.expanded || only.is_some_and(|only| only != tag.rel_id) {
            continue;
        }
        let target = target_of(&rels, &tag.rel_id);
        let Some(loaded) = (files.load)(&target) else {
            unreachable.push(target);
            continue;
        };
        let sub = open(&loaded.bytes).map_err(|error| format!("{target}: {error}"))?;
        let sub_xml = document(&sub)?;
        let (_, sub_blocks) = body(&sub_xml).ok_or_else(|| format!("{target}: no body"))?;
        let content: String = sub_blocks
            .iter()
            .filter(|block| block.name != "w:sectPr")
            .map(|block| &sub_xml[block.start..block.end])
            .collect();
        let content = import(&content, &sub, &mut package)?;
        let locked = tag.locked || loaded.read_only || write_protected(&sub);
        let expanded = Tag {
            expanded: true,
            locked,
            ..tag
        };
        xml.replace_range(
            block.start..block.end,
            &content_control(&expanded, &content),
        );
    }
    unreachable.reverse();
    package.replace(DOCUMENT, xml.into_bytes());
    Ok((write(&package)?, unreachable))
}

/// Collapses the subdocument `only` names, or every expanded one, to its
/// link; returns what each unlocked one now holds, to write to its file.
pub fn collapse(
    docx: &[u8],
    only: Option<&str>,
    files: &Files,
) -> Result<(Vec<u8>, Vec<Saved>), String> {
    let mut package = open(docx)?;
    let mut xml = document(&package)?;
    let rels = relationships(&package);
    let (_, blocks) = body(&xml).ok_or("the document has no body")?;
    let mut saved = Vec::new();
    for block in blocks.iter().rev() {
        let Block::Subdocument(tag, content) = block_kind(&xml, block) else {
            continue;
        };
        if !tag.expanded || only.is_some_and(|only| only != tag.rel_id) {
            continue;
        }
        let target = target_of(&rels, &tag.rel_id);
        if let Some(file) = subdocument_file(&xml[content], &package, &target, tag.locked, files)? {
            saved.push(file);
        }
        let collapsed_tag = Tag {
            expanded: false,
            locked: false,
            ..tag
        };
        xml.replace_range(
            block.start..block.end,
            &collapsed(&collapsed_tag, &target, files),
        );
    }
    saved.reverse();
    package.replace(DOCUMENT, xml.into_bytes());
    Ok((write(&package)?, saved))
}

/// サブ文書の挿入: the file `target` as an expanded subdocument where the
/// [`MARK`] is — in place of the marked paragraph when it holds nothing
/// else, after it when it does.
pub fn insert(docx: &[u8], target: &str, files: &Files) -> Result<Vec<u8>, String> {
    let mut package = open(docx)?;
    let mut xml = document(&package)?;
    let (_, blocks) = body(&xml).ok_or("the document has no body")?;
    let marked = marked_block(&xml, &blocks)?;
    let rel_id = add_relationship(&mut package, SUBDOCUMENT, target, true)?;
    let tag = Tag {
        rel_id: rel_id.clone(),
        expanded: false,
        locked: false,
    };
    let control = collapsed(&tag, target, files);
    let paragraph = &xml[marked.start..marked.end];
    let replacement = if plain_text(paragraph).trim_matches(MARK).trim().is_empty() {
        control
    } else {
        format!("{}{control}", paragraph.replace(MARK, ""))
    };
    xml.replace_range(marked.start..marked.end, &replacement);
    package.replace(DOCUMENT, xml.replace(MARK, "").into_bytes());
    let bytes = write(&package)?;
    let (bytes, unreachable) = expand(&bytes, Some(&rel_id), files)?;
    if !unreachable.is_empty() {
        return Err(format!("{target}: the file cannot be read"));
    }
    Ok(bytes)
}

/// サブ文書の作成: the heading where the [`MARK`] is and everything under
/// it — up to the next heading of its level or above — as a new expanded
/// subdocument, its file named from the heading's first characters and
/// not yet written. Returns the master and the new file's name; `exists`
/// says which names are taken.
pub fn create(docx: &[u8], exists: &dyn Fn(&str) -> bool) -> Result<(Vec<u8>, String), String> {
    let mut package = open(docx)?;
    let mut xml = document(&package)?;
    let (_, blocks) = body(&xml).ok_or("the document has no body")?;
    let marked = marked_block(&xml, &blocks)?;
    let styles = package
        .part_str("word/styles.xml")
        .ok()
        .flatten()
        .unwrap_or_default();
    let levels = style_levels(&styles);
    let start = blocks
        .iter()
        .position(|block| block.start == marked.start)
        .ok_or("the heading is not in the body")?;
    let level = paragraph_level(&xml[marked.start..marked.end], &levels)
        .ok_or("サブ文書は見出しから作成します (Subdocuments are created from headings)")?;
    let mut end = start + 1;
    while let Some(block) = blocks.get(end) {
        let text = &xml[block.start..block.end];
        let stop = match block.name.as_str() {
            "w:sectPr" | "w:sdt" | "w:subDoc" => true,
            "w:p" => paragraph_level(text, &levels).is_some_and(|other| other <= level),
            _ => false,
        };
        if stop {
            break;
        }
        end += 1;
    }
    let heading = plain_text(&xml[marked.start..marked.end]);
    let target = file_name_for(&heading, &relationships(&package), exists);
    let rel_id = add_relationship(&mut package, SUBDOCUMENT, &target, true)?;
    let range = blocks[start].start..blocks[end - 1].end;
    let content = xml[range.clone()].replace(MARK, "");
    let tag = Tag {
        rel_id,
        expanded: true,
        locked: false,
    };
    xml.replace_range(range, &content_control(&tag, &content));
    package.replace(DOCUMENT, xml.replace(MARK, "").into_bytes());
    Ok((write(&package)?, target))
}

/// サブ文書のリンク解除: the subdocument's paragraphs become the master's
/// own, and the link to its file is gone. A collapsed one is expanded first.
pub fn unlink(docx: &[u8], rel_id: &str, files: &Files) -> Result<Vec<u8>, String> {
    let (docx, unreachable) = expand(docx, Some(rel_id), files)?;
    if let Some(target) = unreachable.first() {
        return Err(format!("{target}: the file cannot be read"));
    }
    let mut package = open(&docx)?;
    let mut xml = document(&package)?;
    let (_, blocks) = body(&xml).ok_or("the document has no body")?;
    for block in blocks.iter().rev() {
        if let Block::Subdocument(tag, content) = block_kind(&xml, block) {
            if tag.rel_id == rel_id {
                let content = xml[content].to_string();
                xml.replace_range(block.start..block.end, &content);
            }
        }
    }
    remove_relationship(&mut package, rel_id);
    package.replace(DOCUMENT, xml.into_bytes());
    write(&package)
}

/// 文書のロック: locks or unlocks an expanded subdocument against editing.
pub fn set_locked(docx: &[u8], rel_id: &str, locked: bool) -> Result<Vec<u8>, String> {
    let mut package = open(docx)?;
    let mut xml = document(&package)?;
    let (_, blocks) = body(&xml).ok_or("the document has no body")?;
    let mut found = false;
    for block in blocks.iter().rev() {
        if let Block::Subdocument(tag, content) = block_kind(&xml, block) {
            if tag.rel_id != rel_id {
                continue;
            }
            if !tag.expanded {
                return Err(
                    "展開されたサブ文書だけをロックできます (Expand the subdocument first)".into(),
                );
            }
            found = true;
            let content = xml[content].to_string();
            let tag = Tag { locked, ..tag };
            xml.replace_range(block.start..block.end, &content_control(&tag, &content));
        }
    }
    if !found {
        return Err(format!("no subdocument {rel_id}"));
    }
    package.replace(DOCUMENT, xml.into_bytes());
    write(&package)
}

/// The subdocument the paragraph at surface `paragraph` is in, as the
/// editor holds it.
pub fn subdocument_at(document: &WordDocument, paragraph: usize) -> Option<Tag> {
    document
        .block_content_control_tags(paragraph)
        .iter()
        .rev()
        .find_map(|tag| Tag::parse(tag))
}

/// Whether an edit across `paragraphs` would change a locked subdocument,
/// or a collapsed one's link — which Word does not allow.
pub fn edit_is_locked(document: &WordDocument, paragraphs: RangeInclusive<usize>) -> bool {
    paragraphs.into_iter().any(|paragraph| {
        document
            .block_content_control_tags(paragraph)
            .iter()
            .filter_map(|tag| Tag::parse(tag))
            .any(|tag| tag.locked || !tag.expanded)
    })
}

// ---- the body's blocks -----------------------------------------------------

/// An element's place in the XML: all of it, and what is between its tags.
#[derive(Clone, Debug)]
struct Element {
    name: String,
    start: usize,
    end: usize,
    inner: Range<usize>,
}

enum Block {
    /// A `w:subDoc`, naming its relationship.
    Raw(String),
    /// A subdocument's content control, and where its content is.
    Subdocument(Tag, Range<usize>),
    Other,
}

/// The direct children of the element `within` spans (or of the document,
/// for the whole text).
fn children(xml: &str, within: Range<usize>) -> Vec<Element> {
    scan(xml, within, 1)
}

/// The elements `root_depth` levels into `within`: 1 for an element's
/// children, 0 for a document's root.
fn scan(xml: &str, within: Range<usize>, root_depth: usize) -> Vec<Element> {
    let offset = within.start;
    let fragment = &xml[within];
    let mut reader = Reader::from_str(fragment);
    let mut depth = 0usize;
    let mut open: Option<(String, usize, usize)> = None;
    let mut out = Vec::new();
    loop {
        let before = reader.buffer_position() as usize;
        let Ok(event) = reader.read_event() else {
            break;
        };
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(start) => {
                depth += 1;
                if depth == root_depth + 1 {
                    let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                    open = Some((name, before, after));
                }
            }
            Event::End(_) => {
                if depth == root_depth + 1 {
                    if let Some((name, start, inner_start)) = open.take() {
                        out.push(Element {
                            name,
                            start: offset + start,
                            end: offset + after,
                            inner: offset + inner_start..offset + before,
                        });
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Event::Empty(empty) => {
                if depth == root_depth {
                    out.push(Element {
                        name: String::from_utf8_lossy(empty.name().as_ref()).into_owned(),
                        start: offset + before,
                        end: offset + after,
                        inner: offset + after..offset + after,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// The document's body and its blocks.
fn body(xml: &str) -> Option<(Element, Vec<Element>)> {
    let root = scan(xml, 0..xml.len(), 0)
        .into_iter()
        .find(|element| element.name.ends_with(":document") || element.name == "document")?;
    let body = children(xml, root.start..root.end)
        .into_iter()
        .find(|element| element.name == "w:body")?;
    let blocks = children(xml, body.start..body.end);
    Some((body, blocks))
}

fn block_kind(xml: &str, block: &Element) -> Block {
    match block.name.as_str() {
        "w:subDoc" => attribute(&xml[block.start..block.end], "r:id")
            .map(Block::Raw)
            .unwrap_or(Block::Other),
        "w:sdt" => {
            let parts = children(xml, block.start..block.end);
            let tag = parts
                .iter()
                .find(|part| part.name == "w:sdtPr")
                .and_then(|properties| {
                    children(xml, properties.start..properties.end)
                        .into_iter()
                        .find(|element| element.name == "w:tag")
                })
                .and_then(|tag| attribute(&xml[tag.start..tag.end], "w:val"))
                .and_then(|value| Tag::parse(&value));
            let content = parts.iter().find(|part| part.name == "w:sdtContent");
            match (tag, content) {
                (Some(tag), Some(content)) => Block::Subdocument(tag, content.inner.clone()),
                _ => Block::Other,
            }
        }
        _ => Block::Other,
    }
}

/// The top-level block the [`MARK`] is in.
fn marked_block(xml: &str, blocks: &[Element]) -> Result<Element, String> {
    let at = xml.find(MARK).ok_or("no place is marked in the document")?;
    let block = blocks
        .iter()
        .find(|block| block.start <= at && at < block.end)
        .ok_or("the marked place is not in the body")?;
    if block.name != "w:p" {
        return Err("サブ文書の中には挿入できません (Not inside a subdocument or table)".into());
    }
    Ok(block.clone())
}

/// A collapsed subdocument: its link, as Word shows it.
fn collapsed(tag: &Tag, target: &str, files: &Files) -> String {
    let link = format!(
        r#"<w:p><w:r><w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
        escape(&(files.display)(target))
    );
    content_control(
        &Tag {
            expanded: false,
            ..tag.clone()
        },
        &link,
    )
}

fn content_control(tag: &Tag, content: &str) -> String {
    // A collapsed one is only its link: not text to edit.
    let lock = if tag.locked || !tag.expanded {
        r#"<w:lock w:val="sdtContentLocked"/>"#
    } else {
        ""
    };
    format!(
        r#"<w:sdt><w:sdtPr><w:alias w:val="Subdocument"/><w:tag w:val="{}"/>{lock}</w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>"#,
        escape(&tag.value())
    )
}

/// What an expanded subdocument's file now holds: its own package with
/// the content in place of its body — or, locked, only marked read-only.
/// `None` for a locked one with no file to mark.
fn subdocument_file(
    content: &str,
    master: &OpcPackage,
    target: &str,
    locked: bool,
    files: &Files,
) -> Result<Option<Saved>, String> {
    let loaded = (files.load)(target);
    if locked {
        let Some(loaded) = loaded.filter(|loaded| !loaded.read_only) else {
            return Ok(None);
        };
        let mut package = open(&loaded.bytes)?;
        set_write_protection(&mut package, true);
        return Ok(Some(Saved {
            target: target.to_string(),
            bytes: write(&package)?,
            locked: true,
        }));
    }
    let base = match loaded {
        Some(loaded) => loaded.bytes,
        None => blank_like(master)?,
    };
    let mut package = open(&base)?;
    let content = import(content, master, &mut package)?;
    let mut xml = document(&package)?;
    let (body, blocks) = body(&xml).ok_or_else(|| format!("{target}: no body"))?;
    let section = blocks
        .iter()
        .rev()
        .find(|block| block.name == "w:sectPr")
        .map(|block| xml[block.start..block.end].to_string())
        .unwrap_or_default();
    xml.replace_range(body.inner.clone(), &format!("{content}{section}"));
    package.replace(DOCUMENT, xml.into_bytes());
    set_write_protection(&mut package, false);
    Ok(Some(Saved {
        target: target.to_string(),
        bytes: write(&package)?,
        locked: false,
    }))
}

/// A new subdocument's package: the master's styles, lists and page with
/// an empty body.
fn blank_like(master: &OpcPackage) -> Result<Vec<u8>, String> {
    let mut package = open(&write(master)?)?;
    let mut xml = document(&package)?;
    let (body, blocks) = body(&xml).ok_or("the document has no body")?;
    let section = blocks
        .iter()
        .rev()
        .find(|block| block.name == "w:sectPr")
        .map(|block| xml[block.start..block.end].to_string())
        .unwrap_or_default();
    xml.replace_range(body.inner, &format!("<w:p/>{section}"));
    package.replace(DOCUMENT, xml.into_bytes());
    for rel in relationships(&package) {
        if rel.relationship_type == SUBDOCUMENT {
            remove_relationship(&mut package, &rel.id);
        }
    }
    write(&package)
}

// ---- bringing blocks from one package into another --------------------------

/// `content`, blocks of `from`'s body, made to work in `to`'s: the parts
/// its relationships name copied over, and the styles and lists it uses
/// that `to` does not have.
fn import(content: &str, from: &OpcPackage, to: &mut OpcPackage) -> Result<String, String> {
    let content = import_relationships(content, from, to)?;
    let content = import_numbering(&content, from, to)?;
    import_styles(&content, from, to);
    Ok(content)
}

const RELATIONSHIP_ATTRIBUTES: [&str; 6] = [
    "r:id=\"",
    "r:embed=\"",
    "r:link=\"",
    "r:dm=\"",
    "r:lo=\"",
    "r:qs=\"",
];

fn import_relationships(
    content: &str,
    from: &OpcPackage,
    to: &mut OpcPackage,
) -> Result<String, String> {
    let from_rels = relationships(from);
    let mut mapped: HashMap<String, String> = HashMap::new();
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    loop {
        let next = RELATIONSHIP_ATTRIBUTES
            .iter()
            .filter_map(|attribute| rest.find(attribute).map(|at| (at, attribute.len())))
            .min();
        let Some((at, length)) = next else {
            out.push_str(rest);
            break;
        };
        let value_start = at + length;
        let Some(value_end) = rest[value_start..].find('"').map(|end| value_start + end) else {
            out.push_str(rest);
            break;
        };
        let id = &rest[value_start..value_end];
        out.push_str(&rest[..value_start]);
        let new_id = match mapped.get(id) {
            Some(new_id) => new_id.clone(),
            None => {
                let new_id = match from_rels.iter().find(|rel| rel.id == id) {
                    Some(rel) => copy_relationship(rel, from, to)?,
                    None => id.to_string(),
                };
                mapped.insert(id.to_string(), new_id.clone());
                new_id
            }
        };
        out.push_str(&new_id);
        rest = &rest[value_end..];
    }
    Ok(out)
}

fn copy_relationship(
    rel: &Relationship,
    from: &OpcPackage,
    to: &mut OpcPackage,
) -> Result<String, String> {
    if rel.external {
        return add_relationship(to, &rel.relationship_type, &rel.target, true);
    }
    let source = resolve_target(DOCUMENT, &rel.target);
    let Some(bytes) = from.part(&source).map(<[u8]>::to_vec) else {
        return add_relationship(to, &rel.relationship_type, &rel.target, false);
    };
    // The same picture already copied, or a new name beside it.
    let (directory, file) = source.rsplit_once('/').unwrap_or(("word", source.as_str()));
    let (stem, extension) = file.rsplit_once('.').unwrap_or((file, ""));
    let mut name = source.clone();
    let mut counter = 1;
    while to
        .part(&name)
        .is_some_and(|existing| existing != bytes.as_slice())
    {
        counter += 1;
        name = format!("{directory}/{stem}_{counter}.{extension}");
    }
    if to.part(&name).is_none() {
        to.replace(&name, bytes);
        copy_content_type(from, to, &source, &name, extension);
    }
    let target = name.strip_prefix("word/").unwrap_or(&name).to_string();
    add_relationship(to, &rel.relationship_type, &target, false)
}

fn copy_content_type(
    from: &OpcPackage,
    to: &mut OpcPackage,
    source: &str,
    name: &str,
    extension: &str,
) {
    let from_types = from
        .part_str("[Content_Types].xml")
        .ok()
        .flatten()
        .unwrap_or_default();
    let Ok(Some(mut types)) = to.part_str("[Content_Types].xml") else {
        return;
    };
    let override_type = from_types
        .split("<Override ")
        .skip(1)
        .find(|entry| entry.contains(&format!("PartName=\"/{source}\"")))
        .and_then(|entry| attribute(entry, "ContentType"));
    let addition = match override_type {
        Some(kind) => format!(r#"<Override PartName="/{name}" ContentType="{kind}"/>"#),
        None => {
            if extension.is_empty()
                || types.contains(&format!("Extension=\"{extension}\""))
                || types.contains(&format!("Extension=\"{}\"", extension.to_ascii_uppercase()))
            {
                return;
            }
            let kind = from_types
                .split("<Default ")
                .skip(1)
                .find(|entry| {
                    entry
                        .to_ascii_lowercase()
                        .contains(&format!("extension=\"{}\"", extension.to_ascii_lowercase()))
                })
                .and_then(|entry| attribute(entry, "ContentType"))
                .unwrap_or_else(|| "application/octet-stream".to_string());
            format!(r#"<Default Extension="{extension}" ContentType="{kind}"/>"#)
        }
    };
    if let Some(end) = types.rfind("</Types>") {
        types.insert_str(end, &addition);
        to.replace("[Content_Types].xml", types.into_bytes());
    }
}

/// The lists `content` uses, as new lists of `to`'s; their numbers in
/// `content` changed to match.
fn import_numbering(
    content: &str,
    from: &OpcPackage,
    to: &mut OpcPackage,
) -> Result<String, String> {
    let used: Vec<String> = values_of(content, "<w:numId w:val=\"")
        .into_iter()
        .filter(|id| id != "0")
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if used.is_empty() {
        return Ok(content.to_string());
    }
    let from_numbering = numbering_part(from)
        .and_then(|part| from.part_str(&part).ok().flatten())
        .unwrap_or_default();
    if from_numbering.is_empty() {
        return Ok(content.to_string());
    }
    let part = match numbering_part(to) {
        Some(part) => part,
        None => {
            let part = "word/numbering.xml".to_string();
            to.replace(
                &part,
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></w:numbering>"#.to_vec(),
            );
            add_relationship(to, NUMBERING, "numbering.xml", false)?;
            if let Ok(Some(mut types)) = to.part_str("[Content_Types].xml") {
                if let Some(end) = types.rfind("</Types>") {
                    types.insert_str(end, r#"<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>"#);
                    to.replace("[Content_Types].xml", types.into_bytes());
                }
            }
            part
        }
    };
    let mut numbering = to.part_str(&part).ok().flatten().unwrap_or_default();
    let mut next_abstract = values_of(&numbering, "w:abstractNumId=\"")
        .iter()
        .filter_map(|value| value.parse::<u32>().ok())
        .max()
        .map_or(0, |max| max + 1);
    let mut next_num = values_of(&numbering, "<w:num w:numId=\"")
        .iter()
        .filter_map(|value| value.parse::<u32>().ok())
        .max()
        .map_or(1, |max| max + 1);
    let mut content = content.to_string();
    let mut renumbered = Vec::new();
    for id in used {
        let Some(num) = element_with(
            &from_numbering,
            "<w:num ",
            &format!("w:numId=\"{id}\""),
            "</w:num>",
        ) else {
            continue;
        };
        let Some(abstract_id) = values_of(&num, "<w:abstractNumId w:val=\"")
            .into_iter()
            .next()
        else {
            continue;
        };
        let Some(abstract_num) = element_with(
            &from_numbering,
            "<w:abstractNum ",
            &format!("w:abstractNumId=\"{abstract_id}\""),
            "</w:abstractNum>",
        ) else {
            continue;
        };
        let abstract_num = abstract_num.replacen(
            &format!("w:abstractNumId=\"{abstract_id}\""),
            &format!("w:abstractNumId=\"{next_abstract}\""),
            1,
        );
        let num = num
            .replacen(
                &format!("w:numId=\"{id}\""),
                &format!("w:numId=\"{next_num}\""),
                1,
            )
            .replacen(
                &format!("<w:abstractNumId w:val=\"{abstract_id}\""),
                &format!("<w:abstractNumId w:val=\"{next_abstract}\""),
                1,
            );
        // Every abstract definition comes before the first instance.
        let abstract_at = numbering
            .find("<w:num ")
            .or_else(|| numbering.rfind("</w:numbering>"))
            .ok_or("the numbering part is damaged")?;
        numbering.insert_str(abstract_at, &abstract_num);
        let num_at = numbering
            .rfind("</w:numbering>")
            .ok_or("the numbering part is damaged")?;
        numbering.insert_str(num_at, &num);
        renumbered.push((id, next_num));
        next_abstract += 1;
        next_num += 1;
    }
    // Through a placeholder, so a new number is not taken for an old one.
    for (old, new) in &renumbered {
        content = content.replace(
            &format!("<w:numId w:val=\"{old}\""),
            &format!("<w:numId w:val=\"\u{E001}{new}\""),
        );
    }
    to.replace(&part, numbering.into_bytes());
    Ok(content.replace('\u{E001}', ""))
}

/// The styles `content` names that `to` lacks, copied from `from` with the
/// styles they are based on.
fn import_styles(content: &str, from: &OpcPackage, to: &mut OpcPackage) {
    let (Ok(Some(from_styles)), Ok(Some(mut to_styles))) = (
        from.part_str("word/styles.xml"),
        to.part_str("word/styles.xml"),
    ) else {
        return;
    };
    let from_by_id: HashMap<String, String> = styles_by_id(&from_styles).into_iter().collect();
    let mut have: HashSet<String> = styles_by_id(&to_styles)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let mut wanted: Vec<String> = [
        "<w:pStyle w:val=\"",
        "<w:rStyle w:val=\"",
        "<w:tblStyle w:val=\"",
    ]
    .iter()
    .flat_map(|prefix| values_of(content, prefix))
    .collect();
    let mut added = String::new();
    while let Some(id) = wanted.pop() {
        if have.contains(&id) {
            continue;
        }
        let Some(style) = from_by_id.get(&id) else {
            continue;
        };
        have.insert(id);
        for prefix in [
            "<w:basedOn w:val=\"",
            "<w:link w:val=\"",
            "<w:next w:val=\"",
        ] {
            wanted.extend(values_of(style, prefix));
        }
        added.push_str(style);
    }
    if added.is_empty() {
        return;
    }
    if let Some(end) = to_styles.rfind("</w:styles>") {
        to_styles.insert_str(end, &added);
        to.replace("word/styles.xml", to_styles.into_bytes());
    }
}

/// The `<w:style …>…</w:style>` elements of a styles part, by style id.
fn styles_by_id(styles: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = styles[at..].find("<w:style ") {
        let start = at + found;
        let Some(open_end) = styles[start..].find('>').map(|end| start + end) else {
            break;
        };
        let end = if styles[..open_end].ends_with('/') {
            open_end + 1
        } else {
            match styles[open_end..].find("</w:style>") {
                Some(close) => open_end + close + "</w:style>".len(),
                None => break,
            }
        };
        let element = &styles[start..end];
        let id = attribute(element, "w:styleId").unwrap_or_default();
        out.push((id, element.to_string()));
        at = end;
    }
    out
}

// ---- headings ----------------------------------------------------------

/// Each paragraph style's outline level (0 for Heading 1) and the style it
/// is based on.
fn style_levels(styles: &str) -> HashMap<String, (Option<u8>, Option<String>)> {
    styles_by_id(styles)
        .into_iter()
        .map(|(id, style)| {
            let level = values_of(&style, "<w:outlineLvl w:val=\"")
                .first()
                .and_then(|value| value.parse::<u8>().ok())
                .or_else(|| {
                    values_of(&style, "<w:name w:val=\"")
                        .first()
                        .and_then(|name| {
                            name.to_ascii_lowercase()
                                .strip_prefix("heading ")
                                .map(str::to_string)
                        })
                        .and_then(|level| level.trim().parse::<u8>().ok())
                        .map(|level| level.saturating_sub(1))
                });
            let based_on = values_of(&style, "<w:basedOn w:val=\"").into_iter().next();
            (id, (level, based_on))
        })
        .collect()
}

/// A paragraph's outline level, when it is a heading.
fn paragraph_level(
    paragraph: &str,
    levels: &HashMap<String, (Option<u8>, Option<String>)>,
) -> Option<u8> {
    let properties = paragraph
        .find("<w:pPr")
        .and_then(|start| {
            paragraph[start..]
                .find("</w:pPr>")
                .map(|end| &paragraph[start..start + end])
        })
        .unwrap_or("");
    let level = match values_of(properties, "<w:outlineLvl w:val=\"").first() {
        Some(direct) => direct.parse::<u8>().ok(),
        None => {
            let mut style = values_of(properties, "<w:pStyle w:val=\"")
                .into_iter()
                .next();
            let mut found = None;
            let mut seen = 0;
            while let Some(id) = style.take() {
                seen += 1;
                if seen > 16 {
                    break;
                }
                match levels.get(&id) {
                    Some((Some(level), _)) => found = Some(*level),
                    Some((None, based_on)) => style = based_on.clone(),
                    None => {}
                }
            }
            found
        }
    };
    level.filter(|level| *level < 9)
}

/// A new subdocument's file name: the heading's first characters, as Word
/// names one, made unique.
fn file_name_for(heading: &str, rels: &[Relationship], exists: &dyn Fn(&str) -> bool) -> String {
    let stem: String = heading
        .chars()
        .filter(|ch| *ch != MARK && !ch.is_control() && !"/\\:*?\"<>|".contains(*ch))
        .collect::<String>()
        .trim()
        .chars()
        .take(24)
        .collect::<String>()
        .trim()
        .to_string();
    let stem = if stem.is_empty() {
        "Subdocument".to_string()
    } else {
        stem
    };
    let taken = |name: &str| exists(name) || rels.iter().any(|rel| rel.target == name);
    let mut name = format!("{stem}.docx");
    let mut counter = 1;
    while taken(&name) {
        counter += 1;
        name = format!("{stem}{counter}.docx");
    }
    name
}

// ---- small helpers ------------------------------------------------------

fn open(docx: &[u8]) -> Result<OpcPackage, String> {
    OpcPackage::open(docx).map_err(|error| error.to_string())
}

fn write(package: &OpcPackage) -> Result<Vec<u8>, String> {
    package.write().map_err(|error| error.to_string())
}

fn document(package: &OpcPackage) -> Result<String, String> {
    package
        .part_str(DOCUMENT)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the document has no body".to_string())
}

fn relationships(package: &OpcPackage) -> Vec<Relationship> {
    package
        .part_str(DOCUMENT_RELS)
        .ok()
        .flatten()
        .and_then(|xml| parse_relationships(&xml).ok())
        .map(|rels| rels.all().into_iter().cloned().collect())
        .unwrap_or_default()
}

fn target_of(rels: &[Relationship], rel_id: &str) -> String {
    rels.iter()
        .find(|rel| rel.id == rel_id)
        .map(|rel| rel.target.clone())
        .unwrap_or_default()
}

fn numbering_part(package: &OpcPackage) -> Option<String> {
    relationships(package)
        .into_iter()
        .find(|rel| rel.relationship_type.ends_with("/numbering"))
        .map(|rel| resolve_target(DOCUMENT, &rel.target))
}

fn add_relationship(
    package: &mut OpcPackage,
    kind: &str,
    target: &str,
    external: bool,
) -> Result<String, String> {
    let xml = package.part_str(DOCUMENT_RELS).ok().flatten().unwrap_or_else(|| {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#.to_string()
    });
    let taken: HashSet<String> = relationships(package)
        .into_iter()
        .map(|rel| rel.id)
        .collect();
    let mut number = taken.len() + 1;
    while taken.contains(&format!("rId{number}")) {
        number += 1;
    }
    let id = format!("rId{number}");
    let mode = if external {
        r#" TargetMode="External""#
    } else {
        ""
    };
    let entry = format!(
        r#"<Relationship Id="{id}" Type="{}" Target="{}"{mode}/>"#,
        escape(kind),
        escape(target)
    );
    let end = xml
        .rfind("</Relationships>")
        .ok_or("the relationships part is damaged")?;
    let mut xml = xml;
    xml.insert_str(end, &entry);
    package.replace(DOCUMENT_RELS, xml.into_bytes());
    Ok(id)
}

fn remove_relationship(package: &mut OpcPackage, id: &str) {
    let Ok(Some(mut xml)) = package.part_str(DOCUMENT_RELS) else {
        return;
    };
    let needle = format!("Id=\"{id}\"");
    let Some(at) = xml.find(&needle) else {
        return;
    };
    let Some(start) = xml[..at].rfind("<Relationship") else {
        return;
    };
    let Some(end) = xml[at..].find('>').map(|end| at + end + 1) else {
        return;
    };
    xml.replace_range(start..end, "");
    package.replace(DOCUMENT_RELS, xml.into_bytes());
}

/// Whether the file says to open it read-only.
fn write_protected(package: &OpcPackage) -> bool {
    package
        .part_str("word/settings.xml")
        .ok()
        .flatten()
        .is_some_and(|settings| settings.contains("<w:writeProtection"))
}

/// Marks the file read-only, as a locked subdocument, or clears the mark.
fn set_write_protection(package: &mut OpcPackage, on: bool) {
    let Ok(Some(mut settings)) = package.part_str("word/settings.xml") else {
        return;
    };
    if let Some(start) = settings.find("<w:writeProtection") {
        if let Some(end) = settings[start..].find("/>").map(|end| start + end + 2) {
            settings.replace_range(start..end, "");
        }
    }
    if on {
        // The first of the settings, as the schema orders them.
        let Some(open) = settings
            .find("<w:settings")
            .and_then(|start| settings[start..].find('>').map(|end| start + end + 1))
        else {
            return;
        };
        settings.insert_str(open, r#"<w:writeProtection w:recommended="1"/>"#);
    }
    package.replace("word/settings.xml", settings.into_bytes());
}

/// An attribute's value in an element's text.
fn attribute(element: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let mut at = 0;
    while let Some(found) = element[at..].find(&needle) {
        let start = at + found;
        // Not the end of a longer name.
        let before = element[..start].chars().last();
        if before.is_none_or(|ch| ch.is_whitespace()) {
            let value_start = start + needle.len();
            let value_end = element[value_start..].find('"')? + value_start;
            return Some(unescape(&element[value_start..value_end]));
        }
        at = start + needle.len();
    }
    None
}

/// Every value following `prefix` (which ends at the value's quote).
fn values_of(xml: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(prefix) {
        let value = &rest[at + prefix.len()..];
        let Some(end) = value.find('"') else {
            break;
        };
        out.push(unescape(&value[..end]));
        rest = &value[end..];
    }
    out
}

/// The element starting with `open` that has `marker` in its start tag.
fn element_with(xml: &str, open: &str, marker: &str, close: &str) -> Option<String> {
    let mut at = 0;
    while let Some(found) = xml[at..].find(open) {
        let start = at + found;
        let tag_end = xml[start..].find('>')? + start;
        if xml[start..tag_end].contains(marker) {
            if xml[..tag_end].ends_with('/') {
                return Some(xml[start..=tag_end].to_string());
            }
            let end = xml[tag_end..].find(close)? + tag_end + close.len();
            return Some(xml[start..end].to_string());
        }
        at = tag_end;
    }
    None
}

/// The text of a block's `w:t`s.
fn plain_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(at) = rest.find("<w:t") {
        let after = &rest[at + 4..];
        if !(after.starts_with('>') || after.starts_with(' ')) {
            rest = after;
            continue;
        }
        let Some(open_end) = after.find('>') else {
            break;
        };
        if after[..open_end].ends_with('/') {
            rest = &after[open_end..];
            continue;
        }
        let text = &after[open_end + 1..];
        let Some(close) = text.find("</w:t>") else {
            break;
        };
        out.push_str(&unescape(&text[..close]));
        rest = &text[close..];
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word::new::{blank_document, Blank};
    use std::cell::RefCell;

    /// A document of the blank one's styles with `body` as its blocks.
    fn document_with(body: &str) -> Vec<u8> {
        let mut package = OpcPackage::open(&blank_document(Blank::English)).unwrap();
        let xml = package.part_str(DOCUMENT).unwrap().unwrap();
        let xml = xml.replace("<w:body><w:p/>", &format!("<w:body>{body}"));
        package.replace(DOCUMENT, xml.into_bytes());
        package.write().unwrap()
    }

    fn paragraph(text: &str) -> String {
        format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
    }

    fn heading(level: u8, text: &str) -> String {
        format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading{level}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    }

    fn text_of(docx: &[u8]) -> String {
        let xml = document(&OpcPackage::open(docx).unwrap()).unwrap();
        let (_, blocks) = body(&xml).unwrap();
        blocks
            .iter()
            .filter(|block| block.name != "w:sectPr")
            .map(|block| plain_text(&xml[block.start..block.end]))
            .collect::<Vec<_>>()
            .join("|")
    }

    struct Disk(RefCell<HashMap<String, Vec<u8>>>);

    impl Disk {
        fn files<'a>(&'a self, load: &'a dyn Fn(&str) -> Option<Loaded>) -> Files<'a> {
            Files {
                load,
                display: &|target| format!("/docs/{target}"),
            }
        }
        fn loader(&self) -> impl Fn(&str) -> Option<Loaded> + '_ {
            move |target| {
                self.0.borrow().get(target).map(|bytes| Loaded {
                    bytes: bytes.clone(),
                    read_only: false,
                })
            }
        }
    }

    /// A master as Word stores it: chapter one in the master, chapter two
    /// a subdocument with a list and a style of its own.
    fn master_and_disk() -> (Vec<u8>, Disk) {
        let master = {
            let mut package = OpcPackage::open(&document_with(&format!(
                r#"{}{}<w:subDoc r:id="rId90"/>{}"#,
                heading(1, "Book"),
                paragraph("Preface"),
                paragraph("Afterword")
            )))
            .unwrap();
            let rels = package.part_str(DOCUMENT_RELS).unwrap().unwrap();
            package.replace(
                DOCUMENT_RELS,
                rels.replace(
                    "</Relationships>",
                    &format!(r#"<Relationship Id="rId90" Type="{SUBDOCUMENT}" Target="chapter2.docx" TargetMode="External"/></Relationships>"#),
                )
                .into_bytes(),
            );
            package.write().unwrap()
        };
        let chapter = {
            let mut package = OpcPackage::open(&document_with(&format!(
                r#"{}<w:p><w:pPr><w:pStyle w:val="Verse"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Line one</w:t></w:r></w:p>"#,
                heading(2, "Chapter two")
            )))
            .unwrap();
            let styles = package.part_str("word/styles.xml").unwrap().unwrap();
            package.replace(
                "word/styles.xml",
                styles
                    .replace(
                        "</w:styles>",
                        r#"<w:style w:type="paragraph" w:styleId="Verse"><w:name w:val="Verse"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/></w:pPr></w:style></w:styles>"#,
                    )
                    .into_bytes(),
            );
            package.replace(
                "word/numbering.xml",
                br#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#.to_vec(),
            );
            add_relationship(&mut package, NUMBERING, "numbering.xml", false).unwrap();
            package.write().unwrap()
        };
        let disk = Disk(RefCell::new(HashMap::from([(
            "chapter2.docx".to_string(),
            chapter,
        )])));
        (master, disk)
    }

    #[test]
    fn a_stored_master_opens_with_its_subdocuments_collapsed_to_links() {
        let (master, disk) = master_and_disk();
        let loader = disk.loader();
        let files = disk.files(&loader);
        assert!(is_master(&master));
        let editing = for_editing(&master, &files).unwrap();
        assert_eq!(
            text_of(&editing),
            "Book|Preface|/docs/chapter2.docx|Afterword"
        );
        assert_eq!(
            subdocuments(&editing),
            vec![SubDocument {
                rel_id: "rId90".into(),
                target: "chapter2.docx".into(),
                expanded: false,
                locked: false
            }]
        );
        // Stored again, it is as it was.
        let (stored, saved) = for_saving(&editing, &files).unwrap();
        assert!(saved.is_empty());
        let xml = document(&OpcPackage::open(&stored).unwrap()).unwrap();
        assert!(xml.contains(r#"<w:subDoc r:id="rId90"/>"#));
        assert!(!xml.contains("w:sdt"));
    }

    #[test]
    fn expanding_brings_in_the_subdocument_with_its_style_and_list() {
        let (master, disk) = master_and_disk();
        let loader = disk.loader();
        let files = disk.files(&loader);
        let (expanded, unreachable) = expand(&master, None, &files).unwrap();
        assert!(unreachable.is_empty());
        assert_eq!(
            text_of(&expanded),
            "Book|Preface|Chapter twoLine one|Afterword"
        );
        let package = OpcPackage::open(&expanded).unwrap();
        let styles = package.part_str("word/styles.xml").unwrap().unwrap();
        assert!(styles.contains(r#"w:styleId="Verse""#));
        let numbering = package.part_str("word/numbering.xml").unwrap().unwrap();
        assert!(numbering.contains("w:numId=\"1\""));
        assert!(subdocuments(&expanded)[0].expanded);
        // Editing it and saving writes the subdocument's file.
        let edited = {
            let mut package = OpcPackage::open(&expanded).unwrap();
            let xml = document(&package)
                .unwrap()
                .replace("Line one", "Line one, revised");
            package.replace(DOCUMENT, xml.into_bytes());
            package.write().unwrap()
        };
        let (stored, saved) = for_saving(&edited, &files).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].target, "chapter2.docx");
        assert_eq!(text_of(&saved[0].bytes), "Chapter two|Line one, revised");
        let sub = OpcPackage::open(&saved[0].bytes).unwrap();
        assert!(sub
            .part_str("word/styles.xml")
            .unwrap()
            .unwrap()
            .contains("Verse"));
        assert_eq!(text_of(&stored), "Book|Preface||Afterword");
        // And collapsing does too, leaving the link.
        let (collapsed, saved) = collapse(&edited, None, &files).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            text_of(&collapsed),
            "Book|Preface|/docs/chapter2.docx|Afterword"
        );
    }

    #[test]
    fn a_missing_file_stays_collapsed() {
        let (master, _) = master_and_disk();
        let nothing = |_: &str| None;
        let files = Files {
            load: &nothing,
            display: &|target| target.to_string(),
        };
        let (expanded, unreachable) = expand(&master, None, &files).unwrap();
        assert_eq!(unreachable, vec!["chapter2.docx".to_string()]);
        assert!(!subdocuments(&expanded)[0].expanded);
    }

    #[test]
    fn a_heading_and_what_is_under_it_become_a_subdocument() {
        let docx = document_with(&format!(
            "{}{}{}{}{}",
            heading(1, "Part one"),
            paragraph("Intro"),
            heading(1, &format!("{MARK}Part two")),
            heading(2, "Section"),
            paragraph("Body")
        ));
        let (created, target) = create(&docx, &|name| name == "Part two.docx").unwrap();
        assert_eq!(target, "Part two2.docx");
        let subs = subdocuments(&created);
        assert_eq!(subs.len(), 1);
        assert!(subs[0].expanded);
        assert_eq!(subs[0].target, "Part two2.docx");
        assert_eq!(text_of(&created), "Part one|Intro|Part twoSectionBody");
        // Saved, the new file has those paragraphs and the master its link.
        let disk = Disk(RefCell::new(HashMap::new()));
        let loader = disk.loader();
        let files = disk.files(&loader);
        let (stored, saved) = for_saving(&created, &files).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(text_of(&saved[0].bytes), "Part two|Section|Body");
        assert!(document(&OpcPackage::open(&stored).unwrap())
            .unwrap()
            .contains("<w:subDoc"));
        // Not from body text.
        let plain = document_with(&paragraph(&format!("{MARK}text")));
        assert!(create(&plain, &|_| false).is_err());
    }

    #[test]
    fn inserting_unlinking_and_locking() {
        let (_, disk) = master_and_disk();
        let loader = disk.loader();
        let files = disk.files(&loader);
        let docx = document_with(&format!(
            "{}{}",
            paragraph("Start"),
            paragraph(&MARK.to_string())
        ));
        let inserted = insert(&docx, "chapter2.docx", &files).unwrap();
        assert_eq!(text_of(&inserted), "Start|Chapter twoLine one");
        let rel_id = subdocuments(&inserted)[0].rel_id.clone();
        // Locked, its control is; a save marks the file read-only.
        let locked = set_locked(&inserted, &rel_id, true).unwrap();
        assert!(subdocuments(&locked)[0].locked);
        assert!(document(&OpcPackage::open(&locked).unwrap())
            .unwrap()
            .contains("sdtContentLocked"));
        let (_, saved) = for_saving(&locked, &files).unwrap();
        assert!(saved[0].locked);
        assert!(write_protected(&OpcPackage::open(&saved[0].bytes).unwrap()));
        // Unlinked, the text is the master's own and the link is gone.
        let unlinked = unlink(&inserted, &rel_id, &files).unwrap();
        assert!(subdocuments(&unlinked).is_empty());
        assert_eq!(text_of(&unlinked), "Start|Chapter two|Line one");
        assert!(!OpcPackage::open(&unlinked)
            .unwrap()
            .part_str(DOCUMENT_RELS)
            .unwrap()
            .unwrap()
            .contains(SUBDOCUMENT));
    }

    #[test]
    fn a_read_only_file_expands_locked() {
        let (master, disk) = master_and_disk();
        let load = |target: &str| {
            disk.0.borrow().get(target).map(|bytes| Loaded {
                bytes: bytes.clone(),
                read_only: true,
            })
        };
        let files = disk.files(&load);
        let (expanded, _) = expand(&master, None, &files).unwrap();
        assert!(subdocuments(&expanded)[0].locked);
        // Nothing is written for it.
        let (_, saved) = for_saving(&expanded, &files).unwrap();
        assert!(saved.is_empty());
    }
}
