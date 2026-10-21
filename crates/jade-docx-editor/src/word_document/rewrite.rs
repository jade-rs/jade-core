//! Edits that change the shape of `word/document.xml` itself — a content
//! control wrapped around runs, a merge field's result rewritten, a form
//! field put in — written to the round-trip XML and read back as Word would
//! open the file.
//!
//! The body model treats `w:sdt` and complex fields as wrappers around runs
//! it edits in place, so a new wrapper has nowhere to go but the XML. The
//! command below applies an XML edit to a copy of the tree, writes the
//! package with it, and loads the result: the model, the surfaces and the
//! XML ids are then exactly what a reopened file has. It is a click, not a
//! keystroke, so the cost of a load is affordable. Undo puts the whole
//! document back.

use super::*;
use crate::commands::{Command, CommandContext, CommandError, CommandResult};

/// Where the caret goes after a rewrite, worked out once the document is
/// read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretAfter {
    /// Where it was.
    Keep,
    /// A model position.
    At(TextPosition),
    /// The whole content of the content control at this index in document
    /// order — a placeholder, selected, so typing replaces it.
    SelectControl(usize),
    /// Just after the content control at this index.
    AfterControl(usize),
}

/// The XML edit a [`RewriteCommand`] makes: it gets a copy of the tree to
/// change and the document, whose other parts it may write (a picture, a
/// style, the settings), and returns where the caret goes.
pub type XmlRewrite =
    Box<dyn FnMut(&mut DocxXmlDocument, &mut WordDocument) -> Result<CaretAfter, String>>;

/// Runs a future that never waits — loading a package reads from memory —
/// to completion.
pub(crate) fn run_ready<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    loop {
        if let std::task::Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

impl WordDocument {
    /// The round-trip XML as it is now, building it first when the document
    /// was opened for display only.
    pub(crate) fn current_raw_document(&mut self) -> Result<&DocxXmlDocument, String> {
        self.ensure_roundtrip_xml()
            .map_err(|error| format!("the document XML is unavailable: {error}"))?;
        self.raw_document
            .as_ref()
            .ok_or_else(|| "the document XML is unavailable".to_string())
    }

    /// The package with `document_xml` as its main part and every other
    /// part as this document has it.
    pub(crate) fn package_bytes_with(&self, document_xml: String) -> Result<Vec<u8>, String> {
        let package = self
            .package
            .as_ref()
            .ok_or_else(|| "the document has no package".to_string())?;
        let mut overrides = HashMap::new();
        overrides.insert("word/document.xml".to_string(), document_xml.into_bytes());
        for (path, part) in &self.parts {
            if path == "word/document.xml" {
                continue;
            }
            let updated = part.content.as_bytes();
            let unchanged = package
                .files
                .get(path)
                .is_some_and(|original| original.as_slice() == updated);
            if !unchanged {
                overrides.insert(path.clone(), schema_valid_part(path, &part.content));
            }
        }
        for (path, bytes) in &self.binary_parts {
            overrides.insert(path.clone(), bytes.clone());
        }
        jade_docx_writer::write_docx_with_removed(&package.files, &overrides, &self.removed_parts)
            .map_err(|error| format!("the document could not be written: {error}"))
    }

    /// Applies `rewrite` to a copy of the XML and replaces this document
    /// with the package read back. Returns where the caret goes, as a
    /// rendered selection.
    pub(crate) fn rewrite_document_xml(
        &mut self,
        rewrite: &mut XmlRewrite,
    ) -> Result<Option<SelectionRange>, String> {
        let mut xml = self.current_raw_document()?.clone();
        let caret = rewrite(&mut xml, self)?;
        xml.rebuild_index();
        let document_xml = xml
            .to_string()
            .map_err(|error| format!("the document XML could not be written: {error:?}"))?;
        let bytes = self.package_bytes_with(document_xml)?;
        let mut reloaded = run_ready(WordDocument::load(&bytes, &Options::default()))
            .map_err(|error| format!("the edited document could not be read: {error}"))?;
        reloaded.revision_session = self.revision_session.take();
        reloaded.hf_edit_section = self.hf_edit_section;
        *self = reloaded;
        Ok(self.caret_after(caret))
    }

    fn caret_after(&self, caret: CaretAfter) -> Option<SelectionRange> {
        let rendered = |position: TextPosition| self.model_to_rendered_text_position(position);
        match caret {
            CaretAfter::Keep => None,
            CaretAfter::At(position) => Some(SelectionRange::collapsed(rendered(position))),
            CaretAfter::SelectControl(index) => {
                let control = self.content_controls().into_iter().nth(index)?;
                Some(SelectionRange::new(control.start, control.end))
            }
            CaretAfter::AfterControl(index) => {
                let control = self.content_controls().into_iter().nth(index)?;
                Some(SelectionRange::collapsed(control.end))
            }
        }
    }
}

/// An edit to the shape of `word/document.xml`, one undo step.
pub struct RewriteCommand {
    name: &'static str,
    rewrite: XmlRewrite,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl RewriteCommand {
    pub fn new(
        name: &'static str,
        rewrite: impl FnMut(&mut DocxXmlDocument, &mut WordDocument) -> Result<CaretAfter, String>
            + 'static,
    ) -> Self {
        Self {
            name,
            rewrite: Box::new(rewrite),
            snapshot: None,
            previous_selection: None,
        }
    }
}

impl Command for RewriteCommand {
    fn name(&self) -> &'static str {
        self.name
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        ctx.document
            .ensure_roundtrip_xml()
            .map_err(|error| CommandError::Custom(error.to_string()))?;
        let snapshot = ctx
            .document
            .snapshot_for_replacement()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        match ctx.document.rewrite_document_xml(&mut self.rewrite) {
            Ok(caret) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                if let Some(caret) = caret {
                    *ctx.selection = Some(caret);
                }
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self.snapshot.take().ok_or_else(|| {
            CommandError::Custom("the document before the edit is missing".into())
        })?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

/// Builds detached XML from text: `fragment` is parsed inside a root that
/// declares the WordprocessingML prefixes, and its elements are given ids
/// of `target`.
pub(crate) fn xml_fragment(target: &mut DocxXmlDocument, fragment: &str) -> Vec<XmlNode> {
    let wrapped = format!(
        "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" \
         xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
         xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
         xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
         xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">{fragment}</root>"
    );
    let Ok(parsed) = DocxXmlDocument::parse(&wrapped) else {
        return Vec::new();
    };
    parsed
        .root()
        .children
        .iter()
        .filter_map(|node| match node {
            XmlNode::Element(element) => Some(XmlNode::Element(target.create_element(
                element.name.clone(),
                element.attributes.clone(),
                element.children.clone(),
            ))),
            XmlNode::Text(text) if !text.trim().is_empty() => Some(XmlNode::Text(text.clone())),
            _ => None,
        })
        .collect()
}

/// Escapes text for an XML attribute or text node.
pub(crate) fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }
    escaped
}
