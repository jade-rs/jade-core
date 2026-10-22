//! Edits that rewrite a part of the package other than the body — a style in
//! `styles.xml`, a list definition in `numbering.xml`, the theme, the
//! settings — as one undo step.
//!
//! A style or a theme reaches every paragraph that uses it, and the engine
//! resolves styles into the paragraphs when it reads a document. So rather
//! than restyle each paragraph after a style changed, the edit is made to the
//! saved package and the package is read again: the document on screen is
//! then exactly what a reopened file shows, which is also what saving it
//! writes. The engine's whole-package replacement (`package_edit_command`)
//! makes that one undo step, followed — in the same step — by any commands
//! that act on the paragraphs (putting them in the list just defined).

use crate::commands::snapshot_edit::package_edit_command;
use crate::commands::{Command, CommandContext, CommandError, CommandExecutor, CommandResult};
use crate::headless_api::apply_headless_commands_with_executor_to_document;
use crate::word_document::WordDocument;
use crate::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_opc::OpcPackage;

use super::xml::{Document, Element};
use super::WordSession;

const RELATIONSHIPS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const RELATIONSHIP_BASE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
pub(crate) const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// The parts a Word package relates to its main document, by the last
/// segment of the relationship type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Styles,
    Numbering,
    Theme,
    Settings,
    Glossary,
}

impl PartKind {
    fn relationship(self) -> &'static str {
        match self {
            PartKind::Styles => "styles",
            PartKind::Numbering => "numbering",
            PartKind::Theme => "theme",
            PartKind::Settings => "settings",
            PartKind::Glossary => "glossaryDocument",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            PartKind::Styles => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"
            }
            PartKind::Numbering => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"
            }
            PartKind::Theme => "application/vnd.openxmlformats-officedocument.theme+xml",
            PartKind::Settings => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"
            }
            PartKind::Glossary => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml"
            }
        }
    }

    /// Where a new part of this kind goes, relative to `word/`.
    fn default_target(self) -> &'static str {
        match self {
            PartKind::Styles => "styles.xml",
            PartKind::Numbering => "numbering.xml",
            PartKind::Theme => "theme/theme1.xml",
            PartKind::Settings => "settings.xml",
            PartKind::Glossary => "glossary/document.xml",
        }
    }

    /// What a new part of this kind starts as.
    fn empty_xml(self) -> String {
        let root = match self {
            PartKind::Styles => "w:styles",
            PartKind::Numbering => "w:numbering",
            PartKind::Settings => "w:settings",
            PartKind::Glossary => {
                return format!(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#
                )
            }
            PartKind::Theme => return super::design::OFFICE_THEME.to_string(),
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><{root} xmlns:w="{W_NS}"/>"#
        )
    }
}

fn part_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.trim_start_matches('\u{feff}').to_string()
}

/// The `.rels` part of `part`: `word/document.xml` → `word/_rels/document.xml.rels`.
fn rels_name(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((directory, file)) => format!("{directory}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// A relationship target resolved against the part it is from.
fn resolve(base: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_string();
    }
    let mut segments: Vec<&str> = base.split('/').collect();
    segments.pop();
    for segment in target.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    segments.join("/")
}

/// A Word package being edited.
pub struct Package {
    opc: OpcPackage,
    main: String,
}

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Self, String> {
        let opc = OpcPackage::open(bytes).map_err(|error| error.to_string())?;
        let rels = opc
            .part("_rels/.rels")
            .map(part_text)
            .ok_or("the package has no relationships")?;
        let rels = Document::parse(&rels)?;
        let main = rels
            .root
            .elements()
            .find(|rel| {
                rel.get("Type")
                    .is_some_and(|kind| kind.ends_with("/officeDocument"))
            })
            .and_then(|rel| rel.get("Target"))
            .map(|target| resolve("", &target))
            .unwrap_or_else(|| "word/document.xml".to_string());
        Ok(Package { opc, main })
    }

    pub fn write(&self) -> Result<Vec<u8>, String> {
        self.opc.write().map_err(|error| error.to_string())
    }

    /// The main document part's name.
    pub fn main(&self) -> &str {
        &self.main
    }

    pub fn has(&self, name: &str) -> bool {
        self.opc.part(name).is_some()
    }

    pub fn bytes(&self, name: &str) -> Option<&[u8]> {
        self.opc.part(name)
    }

    pub fn put_bytes(&mut self, name: &str, bytes: Vec<u8>) {
        self.opc.replace(name, bytes);
    }

    pub fn xml(&self, name: &str) -> Result<Document, String> {
        let bytes = self
            .opc
            .part(name)
            .ok_or_else(|| format!("the package has no {name}"))?;
        Document::parse(&part_text(bytes))
    }

    pub fn put_xml(&mut self, name: &str, document: &Document) {
        self.opc.replace(name, document.to_xml().into_bytes());
    }

    fn relationships(&self, part: &str) -> Result<Document, String> {
        match self.opc.part(&rels_name(part)) {
            Some(bytes) => Document::parse(&part_text(bytes)),
            None => Ok(Document {
                prolog: String::new(),
                root: Element::new("Relationships").attr("xmlns", RELATIONSHIPS_NS),
            }),
        }
    }

    /// The part `part` relates with a relationship of type `kind` (the type
    /// URI's last segment).
    pub fn related(&self, part: &str, kind: &str) -> Option<String> {
        let rels = self.relationships(part).ok()?;
        let suffix = format!("/{kind}");
        let target = rels
            .root
            .elements()
            .filter(|rel| rel.get("TargetMode").as_deref() != Some("External"))
            .find(|rel| {
                rel.get("Type")
                    .is_some_and(|value| value.ends_with(&suffix))
            })
            .and_then(|rel| rel.get("Target"))?;
        Some(resolve(part, &target))
    }

    /// Relates `target` (a part name) from `part` with type `kind`, unless it
    /// already is; returns the relationship id.
    pub fn relate(
        &mut self,
        part: &str,
        kind: &str,
        target: &str,
        external: bool,
    ) -> Result<String, String> {
        let mut rels = self.relationships(part)?;
        let relative = if external {
            target.to_string()
        } else {
            relative_target(part, target)
        };
        let full_type = format!("{RELATIONSHIP_BASE}{kind}");
        if let Some(existing) = rels.root.elements().find(|rel| {
            rel.get("Type").as_deref() == Some(full_type.as_str())
                && rel.get("Target").as_deref() == Some(relative.as_str())
        }) {
            return Ok(existing.get("Id").unwrap_or_default());
        }
        let used: Vec<String> = rels
            .root
            .elements()
            .filter_map(|rel| rel.get("Id"))
            .collect();
        let id = (1..)
            .map(|n| format!("rId{n}"))
            .find(|id| !used.contains(id))
            .expect("an unused id");
        let mut rel = Element::new("Relationship")
            .attr("Id", &id)
            .attr("Type", &full_type)
            .attr("Target", &relative);
        if external {
            rel.set_attr("TargetMode", "External");
        }
        rels.root.children.push(super::xml::Node::Element(rel));
        let name = rels_name(part);
        self.put_xml(&name, &rels);
        Ok(id)
    }

    /// Declares `part`'s content type, unless the package already does.
    pub fn declare(&mut self, part: &str, content_type: &str) -> Result<(), String> {
        let mut types = self.xml("[Content_Types].xml")?;
        let part_name = format!("/{part}");
        if types
            .root
            .elements()
            .any(|entry| entry.get("PartName").as_deref() == Some(part_name.as_str()))
        {
            return Ok(());
        }
        types.root.children.push(super::xml::Node::Element(
            Element::new("Override")
                .attr("PartName", &part_name)
                .attr("ContentType", content_type),
        ));
        self.put_xml("[Content_Types].xml", &types);
        Ok(())
    }

    /// Declares the content type of every part with `extension`, unless
    /// the package already does.
    pub fn declare_default(&mut self, extension: &str, content_type: &str) -> Result<(), String> {
        let mut types = self.xml("[Content_Types].xml")?;
        if types.root.elements().any(|entry| {
            entry.name == "Default"
                && entry
                    .get("Extension")
                    .is_some_and(|known| known.eq_ignore_ascii_case(extension))
        }) {
            return Ok(());
        }
        types.root.children.insert(
            0,
            super::xml::Node::Element(
                Element::new("Default")
                    .attr("Extension", extension)
                    .attr("ContentType", content_type),
            ),
        );
        self.put_xml("[Content_Types].xml", &types);
        Ok(())
    }

    /// The main document's part of `kind`, created (related, declared and
    /// empty) when the package has none.
    pub fn part(&mut self, kind: PartKind) -> Result<String, String> {
        let main = self.main.clone();
        if let Some(name) = self.related(&main, kind.relationship()) {
            if self.has(&name) {
                return Ok(name);
            }
        }
        let directory = main.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        let name = if directory.is_empty() {
            kind.default_target().to_string()
        } else {
            format!("{directory}/{}", kind.default_target())
        };
        if !self.has(&name) {
            self.opc.replace(&name, kind.empty_xml().into_bytes());
        }
        self.relate(&main, kind.relationship(), &name, false)?;
        self.declare(&name, kind.content_type())?;
        Ok(name)
    }

    /// Reads, edits and writes back the main document's part of `kind`.
    pub fn edit(
        &mut self,
        kind: PartKind,
        edit: impl FnOnce(&mut Element) -> Result<(), String>,
    ) -> Result<(), String> {
        let name = self.part(kind)?;
        let mut document = self.xml(&name)?;
        edit(&mut document.root)?;
        self.put_xml(&name, &document);
        Ok(())
    }
}

/// `target` relative to the directory of `part`.
fn relative_target(part: &str, target: &str) -> String {
    let base: Vec<&str> = part.split('/').collect();
    let base = &base[..base.len().saturating_sub(1)];
    let target_segments: Vec<&str> = target.split('/').collect();
    let common = base
        .iter()
        .zip(&target_segments)
        .take_while(|(a, b)| a == b)
        .count();
    let mut out: Vec<&str> = std::iter::repeat_n("..", base.len() - common).collect();
    out.extend(&target_segments[common..]);
    out.join("/")
}

/// Commands applied in order and undone as one: the package replacement and
/// what acts on the paragraphs after it.
struct Sequence {
    steps: Vec<Box<dyn Command>>,
}

impl Command for Sequence {
    fn name(&self) -> &'static str {
        self.steps
            .first()
            .map_or("edit_package", |step| step.name())
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for index in 0..self.steps.len() {
            if let Err(error) = self.steps[index].apply(ctx) {
                for step in self.steps[..index].iter_mut().rev() {
                    let _ = step.undo(ctx);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for step in self.steps.iter_mut().rev() {
            step.undo(ctx)?;
        }
        Ok(())
    }

    fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for step in self.steps.iter_mut() {
            step.redo(ctx)?;
        }
        Ok(())
    }
}

/// Headless commands run as one step of a [`Sequence`], with an executor of
/// their own to undo them by.
pub struct HeadlessStep {
    commands: Vec<HeadlessCommand>,
    executor: CommandExecutor,
}

impl HeadlessStep {
    pub fn boxed(commands: Vec<HeadlessCommand>) -> Box<dyn Command> {
        Box::new(HeadlessStep {
            commands,
            executor: CommandExecutor::new(),
        })
    }
}

impl Command for HeadlessStep {
    fn name(&self) -> &'static str {
        "headless_step"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.executor = CommandExecutor::new();
        let selection = *ctx.selection;
        let after = apply_headless_commands_with_executor_to_document(
            ctx.document,
            &mut self.executor,
            selection,
            &HeadlessEditRequest {
                commands: self.commands.clone(),
            },
        )
        .map_err(|error| CommandError::Custom(format!("{error:?}")))?;
        *ctx.selection = after.or(selection);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        while self.executor.undo(ctx)? {}
        Ok(())
    }

    fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        while self.executor.redo(ctx)? {}
        Ok(())
    }
}

/// Reads a package the way the shells open one.
pub(crate) fn load(bytes: &[u8]) -> Result<WordDocument, String> {
    futures::executor::block_on(WordDocument::load(bytes, &Options::default()))
        .map_err(|error| format!("the edited document could not be read: {error}"))
}

/// Edits the session's saved package with `edit`, reads it back in place of
/// the document, then runs `then` — all one undo step. The selection stays
/// where it was: no paragraph comes or goes.
pub fn edit_package<S: WordSession + ?Sized>(
    session: &mut S,
    edit: impl FnOnce(&mut Package) -> Result<(), String>,
    then: Vec<Box<dyn Command>>,
) -> Result<(), String> {
    let bytes = session
        .word_document()
        .export_docx()
        .map_err(|error| error.to_string())?;
    let mut package = Package::open(&bytes)?;
    edit(&mut package)?;
    let edited = package.write()?;
    let replacement = load(&edited)?
        .snapshot_for_replacement()
        .ok_or("the edited document is empty")?;
    let caret = session.selection_range().map(|range| range.focus);
    let mut steps = vec![package_edit_command(bytes, replacement, caret)];
    steps.extend(then);
    session.execute_command(Box::new(Sequence { steps }), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_resolve_and_relativise() {
        assert_eq!(
            resolve("word/document.xml", "styles.xml"),
            "word/styles.xml"
        );
        assert_eq!(
            resolve("word/document.xml", "../customXml/item1.xml"),
            "customXml/item1.xml"
        );
        assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
        assert_eq!(
            relative_target("word/document.xml", "word/theme/theme1.xml"),
            "theme/theme1.xml"
        );
        assert_eq!(
            relative_target("word/document.xml", "customXml/a.xml"),
            "../customXml/a.xml"
        );
        assert_eq!(
            rels_name("word/document.xml"),
            "word/_rels/document.xml.rels"
        );
    }
}
