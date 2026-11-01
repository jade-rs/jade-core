use super::paragraph_property_sync::sync_paragraph_style_xml;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Numbering, ParagraphProperties, RunProperties};
use crate::styles::{merge_paragraph_properties_override, merge_run_properties_override};
use crate::word_document::{
    keep_declared_paragraph_properties, rebase_paragraph_style_properties, RemovedRunProperties,
    WordDocument, WordDocumentSnapshot,
};
use jade_docx_layout::selection::SelectionRange;

/// Applies a paragraph style (e.g. `Heading1`, `Normal`) to a contiguous
/// paragraph range by setting `ParagraphProperties::style`, re-cascading the
/// style's `pPr` / `rPr` defaults onto the affected paragraphs and runs, and
/// upserting the underlying `w:pStyle` child in the raw XML so re-export
/// round-trips the change. Passing `None` reverts to the document default
/// paragraph style (typically `Normal`).
///
/// The cascade is applied with override semantics (not fill-missing) because
/// `WordDocument::load` already materialised inherited font properties on
/// every run at load time; a fill-missing pass at edit time would therefore
/// be a no-op and the style change would not be visible. Direct formatting
/// that the new style does not redefine is preserved; what it does redefine is
/// also dropped from the runs' `w:rPr`, so a reload shows the style too.
pub struct SetParagraphStyleCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    style_id: Option<String>,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_runs: Vec<(usize, Vec<Option<RunProperties>>)>,
    /// Run formatting the style replaced in the saved XML, per paragraph.
    previous_run_xml: Vec<(usize, RemovedRunProperties)>,
    previous_selection: Option<SelectionRange>,
    /// Package state before a missing built-in style definition was added,
    /// or before a note paragraph was restyled. Undo restores it whole so the
    /// definition leaves with the style, and a note's part comes back as it
    /// was.
    package_before_style_creation: Option<WordDocumentSnapshot>,
}

impl SetParagraphStyleCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize, style_id: Option<String>) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            style_id,
            previous_properties: Vec::new(),
            previous_runs: Vec::new(),
            previous_run_xml: Vec::new(),
            previous_selection: None,
            package_before_style_creation: None,
        }
    }

    fn normalized_style_id(&self) -> Option<String> {
        self.style_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }
}

impl Command for SetParagraphStyleCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "set_paragraph_style"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let requested_style_id = self.normalized_style_id();
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }
        self.package_before_style_creation = None;
        // A note paragraph's part is re-read on every edit, so the run
        // properties taken out of it cannot be put back by id: undo restores
        // the document as it was instead.
        let note_snapshot = (start..=end)
            .any(|paragraph_index| ctx.document.is_note_paragraph_surface(paragraph_index))
            .then(|| ctx.document.snapshot_including_package())
            .flatten();
        if let Some(requested) = requested_style_id.as_deref() {
            let defined = ctx
                .document
                .styles_part
                .as_ref()
                .is_some_and(|styles| styles.resolve_paragraph_style_id(requested).is_some());
            if !defined && WordDocument::is_creatable_builtin_paragraph_style(requested) {
                let snapshot = ctx.document.snapshot_including_package();
                if ctx
                    .document
                    .ensure_builtin_paragraph_style(requested)
                    .is_some()
                {
                    self.package_before_style_creation = snapshot;
                }
            }
        }
        if self.package_before_style_creation.is_none() {
            self.package_before_style_creation = note_snapshot;
        }
        let style_id = requested_style_id.as_ref().map(|requested| {
            ctx.document
                .styles_part
                .as_ref()
                .and_then(|styles| styles.resolve_paragraph_style_id(requested))
                .unwrap_or(requested)
                .to_string()
        });

        self.previous_properties.clear();
        self.previous_runs.clear();
        self.previous_run_xml.clear();
        self.previous_selection = *ctx.selection;

        // Resolve the style cascade up front so we don't fight the borrow
        // checker when we later take `&mut` on individual paragraphs. When the
        // caller clears the style (styleId = None), fall back to the document
        // default paragraph style so switching "Heading1 → 標準" actually
        // overrides the Heading1 font size/bold back to Normal's defaults
        // rather than leaving the Heading1-inherited fields in place.
        let (style_paragraph_props, style_run_props) = {
            let styles = ctx.document.styles_part.as_ref();
            // A style the document does not define resolves as the default
            // paragraph style, as Word and a reload of the saved file read
            // its `w:pStyle`. Resolved as nothing, the paragraph kept the old
            // style's formatting on screen only.
            let effective_id: Option<String> = style_id
                .clone()
                .filter(|id| styles.is_some_and(|s| s.get_style(id).is_some()))
                .or_else(|| {
                    styles.and_then(|s| s.default_paragraph_style_id().map(|id| id.to_string()))
                });
            match (styles, effective_id.as_deref()) {
                (Some(styles), Some(id)) => (
                    styles.resolve_paragraph_properties(id),
                    styles.resolve_run_properties(id),
                ),
                _ => (None, None),
            }
        };

        for paragraph_index in start..=end {
            let (previous_props, previous_run_props) = {
                let paragraph = ctx
                    .document
                    .paragraph(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let run_props = paragraph
                    .runs
                    .iter()
                    .map(|run| run.properties.clone())
                    .collect::<Vec<_>>();
                (paragraph.properties.clone(), run_props)
            };
            // The paragraph's properties hold its old style's formatting
            // resolved into them; what the new style leaves undefined has to
            // go back to what it resolves to, or a Heading 1 turned Normal
            // keeps the heading's size and keep-with-next on screen only.
            let previous_style = previous_props
                .as_ref()
                .and_then(|props| props.style.as_deref())
                .map(str::to_string);
            let previous_style_props = ctx
                .document
                .inherited_paragraph_properties(paragraph_index, previous_style.as_deref());
            let inherited_props = ctx
                .document
                .inherited_paragraph_properties(paragraph_index, style_id.as_deref());
            let declared: Vec<String> = ctx
                .document
                .paragraph_declared_property_tags(paragraph_index);
            let declared_attributes = ctx
                .document
                .paragraph_declared_property_attributes(paragraph_index);
            // A style's `w:numPr` numbers its paragraphs, which a load
            // resolves into them; switching the style has to take that
            // numbering away again, or a heading turned Normal keeps its
            // number on screen only.
            let style_numbering = |id: Option<&str>| {
                let styles = ctx.document.styles_part.as_ref()?;
                let id = id.or_else(|| styles.default_paragraph_style_id())?;
                styles
                    .resolve_numbering(id)
                    .filter(|numbering| numbering.id.is_some())
            };
            let numbering_change = (!declared.iter().any(|tag| tag == "w:numPr")).then(|| {
                (
                    style_numbering(previous_style.as_deref()),
                    style_numbering(style_id.as_deref()),
                )
            });
            let run_count = previous_run_props.len();
            // Each run's character style, which outranks the paragraph style
            // for what both define.
            let character_style_props: Vec<Option<RunProperties>> = previous_run_props
                .iter()
                .map(|props| {
                    let styles = ctx.document.styles_part.as_ref()?;
                    let id = props.as_ref()?.style.as_deref()?;
                    styles.resolve_character_run_properties(id)
                })
                .collect();
            self.previous_properties
                .push((paragraph_index, previous_props));
            self.previous_runs
                .push((paragraph_index, previous_run_props));

            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let props = paragraph.properties.get_or_insert_with(Default::default);
                props.style = style_id.clone().map(Into::into);
                if let (Some(old), Some(new)) =
                    (previous_style_props.as_ref(), inherited_props.as_ref())
                {
                    rebase_paragraph_style_properties(props, old, new, |tag| {
                        declared.iter().any(|declared| declared == tag)
                    });
                }
                if let Some((old, new)) = numbering_change {
                    let key = |numbering: Option<&Numbering>| {
                        numbering.map(|numbering| (numbering.id.clone(), numbering.level))
                    };
                    if key(props.numbering.as_ref()) == key(old.as_ref()) {
                        props.numbering = new;
                    }
                }
                let before = props.clone();
                if let Some(style_props) = style_paragraph_props.as_ref() {
                    merge_paragraph_properties_override(props, style_props);
                }
                // What a partly declared `w:spacing` or `w:ind` does not
                // declare comes from the new style alone; the merge above
                // only overrides, and would keep the old style's value
                // wherever the new one is silent. Also when the new style
                // gives no paragraph formatting at all (a Normal with no
                // `w:pPr`): skipped then, a heading's space before stayed
                // with a paragraph set back to Normal, on screen only.
                if let Some(new) = inherited_props.as_ref() {
                    if declared.iter().any(|tag| tag == "w:spacing") {
                        props.spacing = new.spacing.clone();
                    }
                    if declared.iter().any(|tag| tag == "w:ind") {
                        props.indent = new.indent.clone();
                    }
                }
                keep_declared_paragraph_properties(
                    props,
                    &before,
                    |tag| declared.iter().any(|declared| declared == tag),
                    |tag, attribute| {
                        declared_attributes
                            .iter()
                            .any(|(declared, name)| declared == tag && name == attribute)
                    },
                );
                if let Some(run_style_props) = style_run_props.as_ref() {
                    for (run, character) in paragraph.runs.iter_mut().zip(&character_style_props) {
                        match run.properties.as_mut() {
                            Some(rp) => {
                                merge_run_properties_override(rp, run_style_props);
                                if let Some(character) = character {
                                    keep_character_style_fonts(rp, run_style_props, character);
                                }
                                forget_direct_fonts_replaced_by(rp, run_style_props);
                                forget_style_owned_run_properties(rp, run_style_props);
                            }
                            None => {
                                let mut effective = run_style_props.clone();
                                effective.direct_font_properties.known = true;
                                forget_style_owned_run_properties(&mut effective, run_style_props);
                                run.properties = Some(effective);
                            }
                        }
                    }
                }
            }
            ctx.document.restyle_runs_from(
                paragraph_index,
                0..run_count,
                previous_style.as_deref(),
            );
            // The style's fonts name the theme's by key; a load resolves them
            // into the runs, so the runs given them here need the same. Left
            // to the restyle above, a style applied again (which it skips)
            // left them unresolved, and the runs fell back to no font at all.
            if let Some(theme) = ctx.document.loaded_theme_part() {
                ctx.document.apply_theme_fonts_at(paragraph_index, &theme);
            }
            sync_paragraph_style_xml(ctx.document, paragraph_index, style_id.as_deref());
            if let Some(run_style_props) = style_run_props.as_ref() {
                let removed = ctx.document.remove_paragraph_run_properties(
                    paragraph_index,
                    &run_property_tags(run_style_props),
                );
                self.previous_run_xml.push((paragraph_index, removed));
            }
        }

        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(snapshot) = self.package_before_style_creation.take() {
            ctx.document.restore_snapshot(snapshot);
            self.previous_run_xml.clear();
            *ctx.selection = self.previous_selection;
            return Ok(());
        }
        // The removed `w:rPr` children go back by their parent's id. An edit
        // undone in between can rebuild a run's element, and then they have
        // nowhere to go: those paragraphs are written again from their runs,
        // restored below, or the formatting came back on screen only.
        let mut rewrite = Vec::new();
        for (paragraph_index, removed) in self.previous_run_xml.drain(..).rev() {
            if !ctx.document.restore_paragraph_run_properties(removed) {
                rewrite.push(paragraph_index);
            }
        }
        for (paragraph_index, previous) in &self.previous_properties {
            let restored_style = {
                let paragraph = ctx
                    .document
                    .paragraph_mut(*paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                paragraph.properties = previous.clone();
                paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.style.clone())
            };
            sync_paragraph_style_xml(ctx.document, *paragraph_index, restored_style.as_deref());
        }
        for (paragraph_index, previous_runs) in &self.previous_runs {
            let paragraph = ctx
                .document
                .paragraph_mut(*paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            for (run, prev) in paragraph.runs.iter_mut().zip(previous_runs.iter()) {
                run.properties = prev.clone();
            }
        }
        for paragraph_index in rewrite {
            ctx.document
                .sync_paragraph_text_preserving_drawings(paragraph_index);
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

/// The run no longer carries, as direct formatting, the font properties the
/// style replaced. The writer re-emits `w:sz` / `w:kern` / `w:w` / `w:rFonts`
/// from these whenever it rebuilds the run (typing into it, say), which would
/// otherwise bring the replaced size back into the saved file.
/// Puts back the font properties the run's character style gives it where
/// the paragraph style's were just merged over them: a load resolves the
/// character style above the paragraph style. Without this a run in a
/// character style took the heading's size when the heading was applied
/// again, and went back to its character style's on reload.
fn keep_character_style_fonts(
    run: &mut RunProperties,
    paragraph_style: &RunProperties,
    character_style: &RunProperties,
) {
    macro_rules! keep {
        ($($field:ident),* $(,)?) => {
            $(
                if paragraph_style.$field.is_some() && character_style.$field.is_some() {
                    run.$field = character_style.$field.clone();
                }
            )*
        };
    }
    keep!(
        font_size,
        font_size_complex,
        kerning,
        text_scale,
        font_family,
        fonts,
        language,
        east_asia_language,
    );
}

fn forget_direct_fonts_replaced_by(run: &mut RunProperties, style: &RunProperties) {
    let direct = &mut run.direct_font_properties;
    if style.font_size.is_some() {
        direct.font_size = None;
    }
    if style.font_size_complex.is_some() {
        direct.font_size_complex = None;
    }
    if style.kerning.is_some() {
        direct.kerning = None;
    }
    if style.text_scale.is_some() {
        direct.text_scale = None;
    }
    if style.font_family.is_some() || style.fonts.is_some() {
        direct.fonts = None;
    }
}

/// Leaves to the style the run properties it defines, other than the font
/// ones [`forget_direct_fonts_replaced_by`] tracks: their `w:rPr` children
/// are removed below, and a run without them takes them from its paragraph
/// style when drawn, as a loaded document's runs do. Keeping the style's
/// value in the run wrote it into the run's `w:rPr` at its next edit: a
/// heading's theme colour became a fixed direct colour, and the paragraph an
/// Enter started after it carried that colour on.
fn forget_style_owned_run_properties(run: &mut RunProperties, style: &RunProperties) {
    macro_rules! forget {
        ($($field:ident),* $(,)?) => {
            $(
                if style.$field.is_some() {
                    run.$field = None;
                }
            )*
        };
    }
    forget!(
        bold,
        bold_complex,
        italic,
        italic_complex,
        strike,
        double_strike,
        caps,
        small_caps,
        vanish,
        spec_vanish,
        shadow,
        outline,
        emboss,
        imprint,
        emphasis,
        effect,
        baseline,
        text_position,
        char_spacing,
        fit_text,
        color,
        highlight,
        shading,
        border,
    );
    if style.underline.is_some() {
        run.underline = None;
        run.underline_kind = None;
        run.underline_color = None;
    }
}

/// The `w:rPr` children that hold the properties `style` defines: the ones
/// [`merge_run_properties_override`] lets the style replace.
fn run_property_tags(style: &RunProperties) -> Vec<&'static str> {
    let mut tags = Vec::new();
    let mut add = |defined: bool, names: &[&'static str]| {
        if defined {
            tags.extend_from_slice(names);
        }
    };
    add(style.bold.is_some(), &["w:b"]);
    add(style.bold_complex.is_some(), &["w:bCs"]);
    add(style.italic.is_some(), &["w:i"]);
    add(style.italic_complex.is_some(), &["w:iCs"]);
    add(style.underline.is_some(), &["w:u"]);
    add(style.strike.is_some(), &["w:strike"]);
    add(style.double_strike.is_some(), &["w:dstrike"]);
    add(style.caps.is_some(), &["w:caps"]);
    add(style.small_caps.is_some(), &["w:smallCaps"]);
    add(style.vanish.is_some(), &["w:vanish"]);
    add(style.spec_vanish.is_some(), &["w:specVanish"]);
    add(style.shadow.is_some(), &["w:shadow"]);
    add(style.outline.is_some(), &["w:outline"]);
    add(style.emboss.is_some(), &["w:emboss"]);
    add(style.imprint.is_some(), &["w:imprint"]);
    add(style.emphasis.is_some(), &["w:em"]);
    add(style.effect.is_some(), &["w:effect"]);
    add(style.baseline.is_some(), &["w:vertAlign"]);
    add(style.text_position.is_some(), &["w:position"]);
    add(style.char_spacing.is_some(), &["w:spacing"]);
    add(style.kerning.is_some(), &["w:kern"]);
    add(style.text_scale.is_some(), &["w:w"]);
    add(style.fit_text.is_some(), &["w:fitText"]);
    add(style.font_size.is_some(), &["w:sz"]);
    add(style.font_size_complex.is_some(), &["w:szCs"]);
    add(
        style.font_family.is_some() || style.fonts.is_some(),
        &["w:rFonts"],
    );
    add(
        style.language.is_some() || style.east_asia_language.is_some(),
        &["w:lang"],
    );
    add(style.color.is_some(), &["w:color"]);
    add(style.highlight.is_some(), &["w:highlight"]);
    add(style.shading.is_some(), &["w:shd"]);
    add(style.border.is_some(), &["w:bdr"]);
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;

    fn sample_document() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: "Hello".to_string(),
                        properties: None,
                        image: None,
                        text_box: None,
                        math: None,
                        field: None,
                        hyperlink: None,
                        node_id: None,
                        text_node_id: None,
                        revision_node_id: None,
                        comment_reference_ids: Vec::new(),
                        footnote_reference_ids: Vec::new(),
                        endnote_reference_ids: Vec::new(),
                        note_separator: None,
                        has_footnote_ref: false,
                        has_endnote_ref: false,
                        revision: None,
                    }],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn applies_and_undoes_paragraph_style() {
        let mut document = sample_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphStyleCommand::new(
                        0,
                        0,
                        Some("Heading1".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("set paragraph style");
        }

        let style = document
            .paragraph(0)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.style.clone());
        assert_eq!(style.as_deref(), Some("Heading1"));

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let after_undo = document
            .paragraph(0)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.style.clone());
        assert_eq!(after_undo, None);
    }

    #[test]
    fn clears_style_when_id_is_none() {
        let mut document = sample_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphStyleCommand::new(
                        0,
                        0,
                        Some("Heading2".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("seed style");
        }

        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphStyleCommand::new(0, 0, None)),
                    &mut ctx,
                )
                .expect("clear style");
        }

        let style = document
            .paragraph(0)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.style.clone());
        assert_eq!(style, None);
    }

    #[test]
    fn cascades_new_style_run_properties_onto_runs() {
        // Regression for the IDE "Style ボタンを押してもスタイルが当たらない"
        // bug: load-time cascade materialised Normal's font_size onto every
        // run, so a subsequent `fill_missing` pass when switching to Heading1
        // was a no-op. We now override fields that the new style redefines.
        let styles_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:sz w:val="20"/></w:rPr></w:rPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:name w:val="Normal"/>
    <w:rPr><w:sz w:val="20"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Heading1">
    <w:name w:val="heading 1"/>
    <w:rPr><w:b/><w:sz w:val="44"/></w:rPr>
  </w:style>
</w:styles>"#;
        let styles_part = crate::styles::StylesPart::from_xml(styles_xml).expect("styles parse");

        let mut document = sample_document();
        // Simulate the load-time cascade having populated font_size from Normal.
        {
            let run = document
                .paragraph_mut(0)
                .and_then(|p| p.runs.first_mut())
                .expect("run");
            run.properties = Some(RunProperties {
                font_size: Some(10.0),
                ..Default::default()
            });
        }
        document.styles_part = Some(styles_part);

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphStyleCommand::new(
                        0,
                        0,
                        Some("Heading1".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("set Heading1");
        }

        let run_props = document
            .paragraph(0)
            .and_then(|p| p.runs.first())
            .and_then(|r| r.properties.clone())
            .expect("run properties");
        // NOTE: the styles parser in `jade-docx-reader` currently only extracts
        // font-level rPr (sz / szCs / kern / w / rFonts). Bold / italic /
        // color cascade is a follow-up; pinning `font_size` here is enough to
        // catch the "nothing happens" regression because heading font sizes
        // differ markedly from body defaults and drive visible layout change.
        assert_eq!(run_props.font_size, Some(22.0), "Heading1 sz=44 → 22pt");

        // Undo must restore the pre-cascade font_size (10pt).
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let restored = document
            .paragraph(0)
            .and_then(|p| p.runs.first())
            .and_then(|r| r.properties.clone())
            .expect("run properties restored");
        assert_eq!(restored.font_size, Some(10.0));
    }

    #[test]
    fn resolves_heading_preset_to_package_local_style_id() {
        // Japanese Office documents commonly store built-in headings with
        // compact package-local IDs (`1`, `2`, ...) while keeping the built-in
        // style name as `heading 1`. The IDE sends canonical English preset
        // IDs such as `Heading1`, so the command must resolve by style name
        // before applying the cascade.
        let styles_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="a">
    <w:name w:val="Normal"/>
    <w:rPr><w:sz w:val="20"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="1">
    <w:name w:val="heading 1"/>
    <w:basedOn w:val="a"/>
    <w:rPr><w:sz w:val="44"/></w:rPr>
  </w:style>
</w:styles>"#;
        let styles_part = crate::styles::StylesPart::from_xml(styles_xml).expect("styles parse");

        let mut document = sample_document();
        document.styles_part = Some(styles_part);

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphStyleCommand::new(
                        0,
                        0,
                        Some("Heading1".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("set Heading1");
        }

        let paragraph = document.paragraph(0).expect("paragraph");
        let style = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.style.as_deref());
        assert_eq!(style, Some("1"));

        let font_size = paragraph
            .runs
            .first()
            .and_then(|run| run.properties.as_ref())
            .and_then(|props| props.font_size);
        assert_eq!(font_size, Some(22.0));
    }
}
