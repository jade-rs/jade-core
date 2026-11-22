use crate::commands::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{ImagePosition, Run, RunImageLayout};
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget, FloatingObjectTransform};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Where the margins of the anchor's section start on the page, in CSS px:
/// the origin of an object positioned from the margin.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MarginOrigin {
    left: f64,
    top: f64,
}

impl MarginOrigin {
    fn of(ctx: &CommandContext<'_>, paragraph: usize) -> Option<Self> {
        let section = ctx.document.section_index_for_paragraph(paragraph);
        let layout = ctx.document.section_layout(section);
        let px = |points: f32| f64::from(points) * 96.0 / 72.0;
        let (gutter_left, gutter_top) = if layout.gutter_at_top {
            (0.0, layout.gutter)
        } else {
            (layout.gutter, 0.0)
        };
        Some(Self {
            left: px(layout.margin_left + gutter_left),
            top: px(layout.margin_top + gutter_top),
        })
    }
}

pub struct TransformObjectCommand {
    target: FloatingObjectTarget,
    transform: FloatingObjectTransform,
    anchor_paragraph_index: Option<usize>,
    anchor_paragraph_top_px: Option<f64>,
    frame_origin: Option<(f64, f64)>,
    previous_source_runs: Option<Vec<Run>>,
    previous_destination_runs: Option<Vec<Run>>,
    source_body_index: Option<usize>,
    destination_body_index: Option<usize>,
    previous_selection: Option<SelectionRange>,
}

impl TransformObjectCommand {
    pub fn new(target: FloatingObjectTarget, transform: FloatingObjectTransform) -> Self {
        Self {
            target,
            transform,
            anchor_paragraph_index: None,
            anchor_paragraph_top_px: None,
            frame_origin: None,
            previous_source_runs: None,
            previous_destination_runs: None,
            source_body_index: None,
            destination_body_index: None,
            previous_selection: None,
        }
    }

    pub fn with_anchor_paragraph(mut self, paragraph_index: usize) -> Self {
        self.anchor_paragraph_index = Some(paragraph_index);
        self
    }

    /// Keeps the object's current page-local Y while storing it relative to
    /// the paragraph that owns the Word drawing anchor. This lets the object
    /// follow that paragraph when Word lays preceding text out differently.
    pub fn with_anchor_paragraph_top(mut self, top_px: f64) -> Self {
        self.anchor_paragraph_top_px = Some(top_px);
        self
    }

    /// The page point the object's stored offsets count from as it is laid
    /// out now (`CanvasRenderer::cell_float_frame_origin` for an object in a
    /// table cell). The object keeps its frames, its offsets taken again from
    /// there, so it stays where it was dropped. Used only while it keeps its
    /// anchor paragraph.
    pub fn with_frame_origin(mut self, origin: (f64, f64)) -> Self {
        self.frame_origin = Some(origin);
        self
    }

    fn collapse_selection(paragraph_index: usize) -> SelectionRange {
        SelectionRange::collapsed(TextPosition::new(paragraph_index, 0))
    }

    pub(crate) fn apply_layout_transform(
        layout: &mut RunImageLayout,
        transform: FloatingObjectTransform,
        anchor_paragraph_top_px: Option<f64>,
        margins: Option<MarginOrigin>,
        frame_origin: Option<(f64, f64)>,
    ) {
        if let Some((origin_x, origin_y)) = frame_origin {
            // The renderer resolved the frames to `origin`: keep them, with
            // the offsets of the dropped box from there. Simple positioning
            // counts from the cell's corner, which `page` names in a cell.
            let simple = layout.simple_position.take().is_some();
            let frame = |position: &Option<ImagePosition>, default: &str| {
                position
                    .as_ref()
                    .and_then(|position| position.relative_from.clone())
                    .filter(|_| !simple)
                    .unwrap_or_else(|| default.to_string())
            };
            let relative_h = frame(&layout.position_h, if simple { "page" } else { "column" });
            let relative_v = frame(
                &layout.position_v,
                if simple { "page" } else { "paragraph" },
            );
            layout.position_h = Some(ImagePosition {
                relative_from: Some(relative_h),
                offset_px: Some(transform.x - origin_x),
                align: None,
            });
            layout.position_v = Some(ImagePosition {
                relative_from: Some(relative_v),
                offset_px: Some(transform.y - origin_y),
                align: None,
            });
            layout.rotation_deg = Some(transform.rotation_deg);
            return;
        }
        // Word keeps the frame an object is placed from when it is dragged:
        // one positioned from the margin stays positioned from the margin,
        // its offset taken again from there. Frames that cannot be worked
        // out here (a column, a character, an inside margin) become the
        // page's: only the renderer knows which column the anchor is in, and
        // keeping `column` with a margin-relative offset would make an
        // object in column 2 or later jump left by the columns before it.
        let from = |position: &Option<ImagePosition>| {
            position
                .as_ref()
                .and_then(|position| position.relative_from.as_deref())
                .map(str::to_ascii_lowercase)
        };
        let (relative_h, offset_h) = match (from(&layout.position_h).as_deref(), margins) {
            (Some("margin"), Some(margins)) => ("margin".to_string(), transform.x - margins.left),
            _ => ("page".to_string(), transform.x),
        };
        let (relative_v, offset_v) = match (
            anchor_paragraph_top_px,
            from(&layout.position_v).as_deref(),
            margins,
        ) {
            (Some(top), _, _) => ("paragraph".to_string(), transform.y - top),
            (None, Some("margin"), Some(margins)) => {
                ("margin".to_string(), transform.y - margins.top)
            }
            _ => ("page".to_string(), transform.y),
        };
        layout.position_h = Some(ImagePosition {
            relative_from: Some(relative_h),
            offset_px: Some(offset_h),
            align: None,
        });
        layout.position_v = Some(ImagePosition {
            relative_from: Some(relative_v),
            offset_px: Some(offset_v),
            align: None,
        });
        layout.rotation_deg = Some(transform.rotation_deg);
    }

    fn apply_run_transform(
        run: &mut Run,
        kind: FloatingObjectKind,
        transform: FloatingObjectTransform,
        anchor_paragraph_top_px: Option<f64>,
        margins: Option<MarginOrigin>,
        frame_origin: Option<(f64, f64)>,
    ) -> CommandResult<()> {
        match kind {
            FloatingObjectKind::Image => {
                let image = run.image.as_mut().ok_or_else(|| {
                    CommandError::Custom("target run does not contain image".into())
                })?;
                image.width_px = Some(transform.width.max(1.0));
                image.height_px = Some(transform.height.max(1.0));
                Self::apply_layout_transform(
                    &mut image.layout,
                    transform,
                    anchor_paragraph_top_px,
                    margins,
                    frame_origin,
                );
            }
            FloatingObjectKind::TextBox => {
                let text_box = run.text_box.as_mut().ok_or_else(|| {
                    CommandError::Custom("target run does not contain text box".into())
                })?;
                text_box.width_px = Some(transform.width.max(1.0));
                text_box.height_px = Some(transform.height.max(1.0));
                Self::apply_layout_transform(
                    &mut text_box.layout,
                    transform,
                    anchor_paragraph_top_px,
                    margins,
                    frame_origin,
                );
            }
        }
        Ok(())
    }
}

impl Command for TransformObjectCommand {
    fn name(&self) -> &'static str {
        "transform_object"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.previous_selection = *ctx.selection;
        let destination = self
            .anchor_paragraph_index
            .unwrap_or(self.target.paragraph_index);
        let margins = MarginOrigin::of(ctx, destination);

        if destination == self.target.paragraph_index {
            let paragraph = ctx
                .document
                .paragraph_mut(self.target.paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            if self.target.run_index >= paragraph.runs.len() {
                return Err(CommandError::Custom(format!(
                    "run {} is unavailable in paragraph {}",
                    self.target.run_index, self.target.paragraph_index
                )));
            }
            self.previous_source_runs = Some(paragraph.runs.clone());
            self.previous_destination_runs = None;
            self.source_body_index = None;
            self.destination_body_index = None;
            let run = paragraph
                .runs
                .get_mut(self.target.run_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            Self::apply_run_transform(
                run,
                self.target.kind,
                self.transform,
                self.anchor_paragraph_top_px,
                margins,
                self.frame_origin,
            )?;
            ctx.document
                .sync_paragraph_text(self.target.paragraph_index);
            ctx.document.rebuild_paragraph_surfaces();
            *ctx.selection = Some(Self::collapse_selection(self.target.paragraph_index));
            return Ok(());
        }

        let source_body_index = ctx
            .document
            .body_index_for_surface(self.target.paragraph_index)
            .ok_or_else(|| CommandError::Custom("object anchor is not a body paragraph".into()))?;
        let destination_body_index = ctx
            .document
            .body_index_for_surface(destination)
            .ok_or_else(|| {
                CommandError::Custom("destination anchor is not a body paragraph".into())
            })?;
        self.source_body_index = Some(source_body_index);
        self.destination_body_index = Some(destination_body_index);

        let mut moved_run = {
            let source = ctx
                .document
                .paragraph_mut(self.target.paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            if self.target.run_index >= source.runs.len() {
                return Err(CommandError::Custom(format!(
                    "run {} is unavailable in paragraph {}",
                    self.target.run_index, self.target.paragraph_index
                )));
            }
            self.previous_source_runs = Some(source.runs.clone());
            source.runs[self.target.run_index].clone()
        };
        // A new anchor paragraph lays the object out afresh.
        Self::apply_run_transform(
            &mut moved_run,
            self.target.kind,
            self.transform,
            self.anchor_paragraph_top_px,
            margins,
            None,
        )?;
        ctx.document
            .paragraph_mut(self.target.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?
            .runs
            .remove(self.target.run_index);

        let destination_run_index = {
            let destination_paragraph = ctx
                .document
                .paragraph_mut(destination)
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_destination_runs = Some(destination_paragraph.runs.clone());
            let run_index = destination_paragraph.runs.len();
            destination_paragraph.runs.push(moved_run);
            run_index
        };

        ctx.document
            .sync_paragraph_text(self.target.paragraph_index);
        ctx.document.sync_paragraph_text(destination);
        ctx.document.rebuild_paragraph_surfaces();
        let destination_surface = ctx
            .document
            .surface_index_for_body(destination_body_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let destination_paragraph = ctx
            .document
            .paragraph(destination_surface)
            .ok_or(CommandError::ParagraphUnavailable)?;
        if destination_run_index >= destination_paragraph.runs.len() {
            return Err(CommandError::Custom(
                "moved object run was not preserved in destination paragraph".into(),
            ));
        }
        *ctx.selection = Some(Self::collapse_selection(destination_surface));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let source_backup = self
            .previous_source_runs
            .as_ref()
            .ok_or_else(|| CommandError::Custom("missing source object backup for undo".into()))?;
        let (source_surface, destination_surface) =
            match (self.source_body_index, self.destination_body_index) {
                (Some(source_body), Some(destination_body)) => {
                    let source_surface = ctx
                        .document
                        .surface_index_for_body(source_body)
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    let destination_surface = ctx
                        .document
                        .surface_index_for_body(destination_body)
                        .ok_or(CommandError::ParagraphUnavailable)?;
                    (source_surface, Some(destination_surface))
                }
                _ => (self.target.paragraph_index, None),
            };

        ctx.document
            .paragraph_mut(source_surface)
            .ok_or(CommandError::ParagraphUnavailable)?
            .runs = source_backup.clone();
        if let Some(destination_surface) = destination_surface {
            let destination_backup = self.previous_destination_runs.as_ref().ok_or_else(|| {
                CommandError::Custom("missing destination object backup for undo".into())
            })?;
            ctx.document
                .paragraph_mut(destination_surface)
                .ok_or(CommandError::ParagraphUnavailable)?
                .runs = destination_backup.clone();
        }

        ctx.document.sync_paragraph_text(source_surface);
        if let Some(destination_surface) = destination_surface {
            ctx.document.sync_paragraph_text(destination_surface);
        }
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, Run, RunImage, RunImageKind, RunTextBox,
    };
    use crate::word_document::WordDocument;

    fn paragraph_with_runs(runs: Vec<Run>) -> Paragraph {
        Paragraph {
            runs,
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        }
    }

    fn image_run() -> Run {
        Run {
            text: String::new(),
            properties: None,
            image: Some(RunImage {
                signature_line: None,
                decorative: false,
                relationship_id: Some("rIdImage".to_string()),
                ole_relationship_id: None,
                ole_program_id: None,
                width_px: Some(120.0),
                height_px: Some(80.0),
                title: Some("floating image".to_string()),
                description: Some("floating image".to_string()),
                placeholder_label: None,
                data_uri: Some("data:image/png;base64,AAAA".to_string()),
                chart: None,
                shape_group: None,
                diagram_data_relationship_id: None,
                crop: None,
                layout: RunImageLayout {
                    kind: RunImageKind::Floating,
                    ..RunImageLayout::default()
                },
                ..Default::default()
            }),
            text_box: None,
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            text_controls: None,
            revision_node_id: None,
            comment_reference_ids: Vec::new(),
            footnote_reference_ids: Vec::new(),
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: None,
        }
    }

    fn text_box_run() -> Run {
        Run {
            text: String::new(),
            properties: None,
            image: None,
            text_box: Some(RunTextBox {
                width_px: Some(180.0),
                height_px: Some(90.0),
                text: "Inside textbox".to_string(),
                layout: RunImageLayout {
                    kind: RunImageKind::Floating,
                    ..RunImageLayout::default()
                },
                blocks: vec![BodyElement::from_paragraph(paragraph_with_runs(vec![
                    Run {
                        text: "Inside textbox".to_string(),
                        properties: None,
                        image: None,
                        text_box: None,
                        math: None,
                        field: None,
                        hyperlink: None,
                        node_id: None,
                        text_node_id: None,
                        text_controls: None,
                        revision_node_id: None,
                        comment_reference_ids: Vec::new(),
                        footnote_reference_ids: Vec::new(),
                        endnote_reference_ids: Vec::new(),
                        note_separator: None,
                        has_footnote_ref: false,
                        has_endnote_ref: false,
                        revision: None,
                    },
                ]))],
                border: Default::default(),
                shadow: None,
                inner_shadow: None,
                glow: None,
                blur_radius_px: None,
                soft_edge_radius_px: None,
                fill_color: None,
                fill_opacity: None,
                fill_theme_color: None,
                fill_theme_transforms: Vec::new(),
                insets: Default::default(),
                vertical_alignment: Default::default(),
                shape: Default::default(),
                ..Default::default()
            }),
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            text_controls: None,
            revision_node_id: None,
            comment_reference_ids: Vec::new(),
            footnote_reference_ids: Vec::new(),
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: None,
        }
    }

    fn sample_document(run: Run) -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph_with_runs(vec![run]))],
            }),
            ..Document::default()
        })
    }

    fn sample_document_with_destination(run: Run) -> WordDocument {
        let mut destination_run = text_box_run();
        destination_run.text = "Destination paragraph".to_string();
        destination_run.text_box = None;
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph_with_runs(vec![run])),
                    BodyElement::from_paragraph(paragraph_with_runs(vec![destination_run])),
                ],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn transforms_floating_image_and_restores_it_with_undo_redo() {
        let mut word_doc = sample_document(image_run());
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let transform = FloatingObjectTransform {
            x: 140.0,
            y: 56.0,
            width: 164.0,
            height: 118.0,
            rotation_deg: 24.0,
        };

        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(TransformObjectCommand::new(
                        FloatingObjectTarget {
                            paragraph_index: 0,
                            run_index: 0,
                            kind: FloatingObjectKind::Image,
                        },
                        transform,
                    )),
                    &mut ctx,
                )
                .expect("transform image");
        }

        let image = word_doc
            .paragraph(0)
            .and_then(|paragraph| paragraph.runs[0].image.as_ref())
            .expect("image");
        assert_eq!(image.width_px, Some(164.0));
        assert_eq!(image.height_px, Some(118.0));
        assert_eq!(
            image
                .layout
                .position_h
                .as_ref()
                .and_then(|position| position.offset_px),
            Some(140.0)
        );
        assert_eq!(
            image
                .layout
                .position_h
                .as_ref()
                .and_then(|position| position.relative_from.as_deref()),
            Some("page")
        );
        assert_eq!(
            image
                .layout
                .position_v
                .as_ref()
                .and_then(|position| position.offset_px),
            Some(56.0)
        );
        assert_eq!(image.layout.rotation_deg, Some(24.0));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo image transform"));
        }
        let image = word_doc
            .paragraph(0)
            .and_then(|paragraph| paragraph.runs[0].image.as_ref())
            .expect("image after undo");
        assert_eq!(image.width_px, Some(120.0));
        assert_eq!(image.height_px, Some(80.0));
        assert!(image.layout.position_h.is_none());
        assert!(image.layout.position_v.is_none());
        assert!(image.layout.rotation_deg.is_none());

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo image transform"));
        }
        let image = word_doc
            .paragraph(0)
            .and_then(|paragraph| paragraph.runs[0].image.as_ref())
            .expect("image after redo");
        assert_eq!(image.width_px, Some(164.0));
        assert_eq!(image.height_px, Some(118.0));
        assert_eq!(image.layout.rotation_deg, Some(24.0));
    }

    /// A dragged column-relative object keeps where it was dropped: its frame
    /// becomes the page and its offset the dropped page x, not an offset from
    /// the left margin left labelled `column` (which would put an object
    /// dropped in column 2 back in column 1).
    #[test]
    fn dragging_a_column_relative_image_positions_it_from_the_page() {
        let mut run = image_run();
        run.image.as_mut().expect("image").layout.position_h = Some(ImagePosition {
            relative_from: Some("column".to_string()),
            offset_px: Some(10.0),
            align: None,
        });
        let mut word_doc = sample_document(run);
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let transform = FloatingObjectTransform {
            x: 442.0,
            y: 56.0,
            width: 120.0,
            height: 80.0,
            rotation_deg: 0.0,
        };
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(TransformObjectCommand::new(
                        FloatingObjectTarget {
                            paragraph_index: 0,
                            run_index: 0,
                            kind: FloatingObjectKind::Image,
                        },
                        transform,
                    )),
                    &mut ctx,
                )
                .expect("transform image");
        }
        let position = word_doc
            .paragraph(0)
            .and_then(|paragraph| paragraph.runs[0].image.as_ref())
            .and_then(|image| image.layout.position_h.clone())
            .expect("horizontal position");
        assert_eq!(position.relative_from.as_deref(), Some("page"));
        assert_eq!(position.offset_px, Some(442.0));
    }

    #[test]
    fn transforms_floating_text_box_and_restores_it_with_undo_redo() {
        let mut word_doc = sample_document(text_box_run());
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let transform = FloatingObjectTransform {
            x: 96.0,
            y: 84.0,
            width: 220.0,
            height: 132.0,
            rotation_deg: -18.0,
        };

        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(TransformObjectCommand::new(
                        FloatingObjectTarget {
                            paragraph_index: 0,
                            run_index: 0,
                            kind: FloatingObjectKind::TextBox,
                        },
                        transform,
                    )),
                    &mut ctx,
                )
                .expect("transform text box");
        }

        let text_box = word_doc
            .paragraph(0)
            .and_then(|paragraph| paragraph.runs[0].text_box.as_ref())
            .expect("text box");
        assert_eq!(text_box.width_px, Some(220.0));
        assert_eq!(text_box.height_px, Some(132.0));
        assert_eq!(text_box.layout.rotation_deg, Some(-18.0));
        assert_eq!(
            text_box
                .layout
                .position_v
                .as_ref()
                .and_then(|position| position.offset_px),
            Some(84.0)
        );

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo text box transform"));
        }
        let text_box = word_doc
            .paragraph(0)
            .and_then(|paragraph| paragraph.runs[0].text_box.as_ref())
            .expect("text box after undo");
        assert_eq!(text_box.width_px, Some(180.0));
        assert_eq!(text_box.height_px, Some(90.0));
        assert!(text_box.layout.position_h.is_none());
        assert!(text_box.layout.position_v.is_none());
        assert!(text_box.layout.rotation_deg.is_none());

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo text box transform"));
        }
        let text_box = word_doc
            .paragraph(0)
            .and_then(|paragraph| paragraph.runs[0].text_box.as_ref())
            .expect("text box after redo");
        assert_eq!(text_box.width_px, Some(220.0));
        assert_eq!(text_box.height_px, Some(132.0));
        assert_eq!(text_box.layout.rotation_deg, Some(-18.0));
    }

    #[test]
    fn moves_text_box_anchor_between_paragraphs_and_restores_it_with_undo_redo() {
        let mut word_doc = sample_document_with_destination(text_box_run());
        // The textbox content is its own rendered surface between the two body
        // paragraphs, so the destination starts at rendered paragraph 2.
        assert!(word_doc.is_body_paragraph_surface(0));
        assert!(!word_doc.is_body_paragraph_surface(1));
        assert!(word_doc.is_body_paragraph_surface(2));
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
        let transform = FloatingObjectTransform {
            x: 144.0,
            y: 180.0,
            width: 220.0,
            height: 132.0,
            rotation_deg: 0.0,
        };

        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(
                        TransformObjectCommand::new(
                            FloatingObjectTarget {
                                paragraph_index: 0,
                                run_index: 0,
                                kind: FloatingObjectKind::TextBox,
                            },
                            transform,
                        )
                        .with_anchor_paragraph(2)
                        .with_anchor_paragraph_top(156.0),
                    ),
                    &mut ctx,
                )
                .expect("move textbox anchor");
        }

        assert!(word_doc
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .all(|run| run.text_box.is_none()));
        // After relocation, the destination body paragraph precedes the moved
        // textbox content surface and therefore becomes rendered paragraph 1.
        assert!(word_doc.is_body_paragraph_surface(1));
        let anchored = word_doc.paragraph(1).expect("destination paragraph");
        assert_eq!(anchored.runs[0].text, "Destination paragraph");
        let text_box = anchored.runs[1].text_box.as_ref().expect("moved textbox");
        assert_eq!(
            text_box.layout.position_h.as_ref().unwrap().offset_px,
            Some(144.0)
        );
        assert_eq!(
            text_box.layout.position_v.as_ref().unwrap().offset_px,
            Some(24.0)
        );
        assert_eq!(
            text_box
                .layout
                .position_v
                .as_ref()
                .unwrap()
                .relative_from
                .as_deref(),
            Some("paragraph")
        );
        assert_eq!(selection.unwrap().focus.paragraph, 1);

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo anchor move"));
        }
        assert!(word_doc.paragraph(0).unwrap().runs[0].text_box.is_some());
        assert!(word_doc.is_body_paragraph_surface(2));
        assert_eq!(word_doc.paragraph(2).unwrap().runs.len(), 1);

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo anchor move"));
        }
        assert!(word_doc
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .all(|run| run.text_box.is_none()));
        assert!(word_doc.paragraph(1).unwrap().runs[1].text_box.is_some());
    }
}
