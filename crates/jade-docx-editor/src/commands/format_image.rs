use crate::commands::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{ImageCrop, ImagePosition, Run, RunImageKind, WrapMode};
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageLayoutPreset {
    Inline,
    Square,
    Tight,
    Through,
    TopAndBottom,
    BehindText,
    InFrontOfText,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageFormatSpec {
    pub width_px: Option<f64>,
    pub height_px: Option<f64>,
    pub crop: Option<ImageCrop>,
    pub clear_crop: bool,
    pub layout: Option<ImageLayoutPreset>,
    pub horizontal_align: Option<String>,
    pub vertical_align: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub rotation_deg: Option<f64>,
    pub flip_horizontal: Option<bool>,
    pub flip_vertical: Option<bool>,
    pub dist_top: Option<f64>,
    pub dist_bottom: Option<f64>,
    pub dist_left: Option<f64>,
    pub dist_right: Option<f64>,
}

impl ImageFormatSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (label, value) in [("width", self.width_px), ("height", self.height_px)] {
            if let Some(value) = value {
                if !value.is_finite() || !(1.0..=100_000.0).contains(&value) {
                    return Err(format!("image {label} must be between 1 and 100000 pixels"));
                }
            }
        }
        if self.clear_crop && self.crop.is_some() {
            return Err("crop and clearCrop cannot be supplied together".into());
        }
        if let Some(crop) = self.crop {
            for (label, value) in [
                ("left", crop.left),
                ("top", crop.top),
                ("right", crop.right),
                ("bottom", crop.bottom),
            ] {
                if !value.is_finite() || !(0.0..1.0).contains(&value) {
                    return Err(format!("crop {label} must be at least 0 and less than 1"));
                }
            }
            if crop.left + crop.right >= 1.0 || crop.top + crop.bottom >= 1.0 {
                return Err("opposite crop edges must leave a visible image area".into());
            }
        }
        for (label, value) in [
            ("x", self.x),
            ("y", self.y),
            ("rotationDeg", self.rotation_deg),
        ] {
            if value.is_some_and(|value| !value.is_finite()) {
                return Err(format!("{label} must be finite"));
            }
        }
        for (label, value) in [
            ("distTop", self.dist_top),
            ("distBottom", self.dist_bottom),
            ("distLeft", self.dist_left),
            ("distRight", self.dist_right),
        ] {
            if value.is_some_and(|value| !value.is_finite() || !(0.0..=10_000.0).contains(&value)) {
                return Err(format!("{label} must be between 0 and 10000 pixels"));
            }
        }
        validate_alignment(
            "horizontalAlign",
            self.horizontal_align.as_deref(),
            &["left", "center", "right"],
        )?;
        validate_alignment(
            "verticalAlign",
            self.vertical_align.as_deref(),
            &["top", "center", "bottom"],
        )?;
        Ok(())
    }
}

fn validate_alignment(label: &str, value: Option<&str>, allowed: &[&str]) -> Result<(), String> {
    if let Some(value) = value {
        if !allowed.contains(&value) {
            return Err(format!("{label} must be one of {}", allowed.join(", ")));
        }
    }
    Ok(())
}

pub struct FormatImageCommand {
    target: FloatingObjectTarget,
    spec: ImageFormatSpec,
    previous_runs: Option<Vec<Run>>,
    previous_selection: Option<SelectionRange>,
}

impl FormatImageCommand {
    pub fn new(target: FloatingObjectTarget, spec: ImageFormatSpec) -> Self {
        Self {
            target,
            spec,
            previous_runs: None,
            previous_selection: None,
        }
    }

    fn collapse_selection(&self) -> SelectionRange {
        SelectionRange::collapsed(TextPosition::new(self.target.paragraph_index, 0))
    }
}

impl Command for FormatImageCommand {
    fn name(&self) -> &'static str {
        "format_image"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.spec.validate().map_err(CommandError::Custom)?;
        if self.target.kind != FloatingObjectKind::Image {
            return Err(CommandError::Custom(
                "selected object is not an image".into(),
            ));
        }
        self.previous_selection = *ctx.selection;
        let paragraph = ctx
            .document
            .paragraph_mut(self.target.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        self.previous_runs = Some(paragraph.runs.clone());
        let run = paragraph
            .runs
            .get_mut(self.target.run_index)
            .ok_or_else(|| {
                CommandError::Custom(format!(
                    "run {} is unavailable in paragraph {}",
                    self.target.run_index, self.target.paragraph_index
                ))
            })?;
        let image = run
            .image
            .as_mut()
            .ok_or_else(|| CommandError::Custom("target run does not contain image".into()))?;

        if let Some(width) = self.spec.width_px {
            image.width_px = Some(width);
        }
        if let Some(height) = self.spec.height_px {
            image.height_px = Some(height);
        }
        if self.spec.clear_crop {
            image.crop = None;
        } else if let Some(crop) = self.spec.crop {
            image.crop = Some(crop);
        }
        if let Some(rotation) = self.spec.rotation_deg {
            image.layout.rotation_deg = Some(rotation);
        }
        if let Some(flip_horizontal) = self.spec.flip_horizontal {
            image.layout.flip_horizontal = flip_horizontal;
        }
        if let Some(flip_vertical) = self.spec.flip_vertical {
            image.layout.flip_vertical = flip_vertical;
        }

        if let Some(preset) = self.spec.layout {
            match preset {
                ImageLayoutPreset::Inline => {
                    image.layout.kind = RunImageKind::Inline;
                    image.layout.wrap = WrapMode::None;
                    image.layout.behind_doc = false;
                    image.layout.position_h = None;
                    image.layout.position_v = None;
                }
                ImageLayoutPreset::Square
                | ImageLayoutPreset::Tight
                | ImageLayoutPreset::Through
                | ImageLayoutPreset::TopAndBottom
                | ImageLayoutPreset::BehindText
                | ImageLayoutPreset::InFrontOfText => {
                    image.layout.kind = RunImageKind::Floating;
                    image.layout.wrap = match preset {
                        ImageLayoutPreset::Square => WrapMode::Square,
                        ImageLayoutPreset::Tight => WrapMode::Tight,
                        ImageLayoutPreset::Through => WrapMode::Through,
                        ImageLayoutPreset::TopAndBottom => WrapMode::TopAndBottom,
                        ImageLayoutPreset::BehindText | ImageLayoutPreset::InFrontOfText => {
                            WrapMode::None
                        }
                        ImageLayoutPreset::Inline => unreachable!(),
                    };
                    image.layout.behind_doc = preset == ImageLayoutPreset::BehindText;
                    image
                        .layout
                        .position_h
                        .get_or_insert_with(|| ImagePosition {
                            relative_from: Some("margin".into()),
                            offset_px: Some(0.0),
                            align: None,
                        });
                    image
                        .layout
                        .position_v
                        .get_or_insert_with(|| ImagePosition {
                            relative_from: Some("paragraph".into()),
                            offset_px: Some(0.0),
                            align: None,
                        });
                }
            }
        }

        if let Some(align) = self.spec.horizontal_align.as_ref() {
            image.layout.kind = RunImageKind::Floating;
            image.layout.position_h = Some(ImagePosition {
                relative_from: Some("page".into()),
                offset_px: None,
                align: Some(align.clone()),
            });
        } else if let Some(x) = self.spec.x {
            image.layout.kind = RunImageKind::Floating;
            image.layout.position_h = Some(ImagePosition {
                relative_from: Some("page".into()),
                offset_px: Some(x),
                align: None,
            });
        }
        if let Some(align) = self.spec.vertical_align.as_ref() {
            image.layout.kind = RunImageKind::Floating;
            image.layout.position_v = Some(ImagePosition {
                relative_from: Some("page".into()),
                offset_px: None,
                align: Some(align.clone()),
            });
        } else if let Some(y) = self.spec.y {
            image.layout.kind = RunImageKind::Floating;
            image.layout.position_v = Some(ImagePosition {
                relative_from: Some("page".into()),
                offset_px: Some(y),
                align: None,
            });
        }
        for (destination, value) in [
            (&mut image.layout.dist_top, self.spec.dist_top),
            (&mut image.layout.dist_bottom, self.spec.dist_bottom),
            (&mut image.layout.dist_left, self.spec.dist_left),
            (&mut image.layout.dist_right, self.spec.dist_right),
        ] {
            if let Some(value) = value {
                *destination = Some(value);
            }
        }
        // A source-bound image drawing is preserved verbatim by ordinary
        // paragraph edits. Detach it for an explicit image mutation so sync
        // serializes the new modeled properties.
        run.node_id = None;
        run.text_node_id = None;
        run.revision_node_id = None;

        ctx.document
            .sync_paragraph_text(self.target.paragraph_index);
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = Some(self.collapse_selection());
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let paragraph = ctx
            .document
            .paragraph_mut(self.target.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        paragraph.runs = self
            .previous_runs
            .as_ref()
            .ok_or_else(|| CommandError::Custom("missing image-format backup for undo".into()))?
            .clone();
        ctx.document
            .sync_paragraph_text(self.target.paragraph_index);
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::document::{Body, BodyElement, Document, Paragraph, Run, RunImage};
    use crate::word_document::WordDocument;
    use proptest::prelude::*;

    fn image_document() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: String::new(),
                        properties: None,
                        image: Some(RunImage {
                            relationship_id: Some("rIdImage".into()),
                            width_px: Some(120.0),
                            height_px: Some(80.0),
                            ..RunImage::default()
                        }),
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

    fn target() -> FloatingObjectTarget {
        FloatingObjectTarget {
            paragraph_index: 0,
            run_index: 0,
            kind: FloatingObjectKind::Image,
        }
    }

    #[test]
    fn rejects_crop_that_removes_the_whole_image_without_mutating_history() {
        let mut document = image_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let result = executor.execute(
            Box::new(FormatImageCommand::new(
                target(),
                ImageFormatSpec {
                    crop: Some(ImageCrop {
                        left: 0.6,
                        right: 0.4,
                        ..ImageCrop::default()
                    }),
                    ..ImageFormatSpec::default()
                },
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        );
        assert!(result.is_err());
        assert!(!executor.can_undo());
        assert_eq!(
            document.paragraph(0).unwrap().runs[0]
                .image
                .as_ref()
                .unwrap()
                .crop,
            None
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 192, ..ProptestConfig::default() })]

        #[test]
        fn format_image_properties_apply_undo_and_redo_atomically(
            width in 1u32..5000,
            height in 1u32..5000,
            left in 0u32..45,
            right in 0u32..45,
            top in 0u32..45,
            bottom in 0u32..45,
            layout_index in 0u8..7,
            horizontal_index in 0u8..3,
            vertical_index in 0u8..3,
            flip_horizontal in any::<bool>(),
            flip_vertical in any::<bool>(),
        ) {
            let mut document = image_document();
            let before = document.paragraph(0).unwrap().runs[0].image.clone().unwrap();
            let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 0)));
            let mut executor = CommandExecutor::new();
            let layout = match layout_index {
                0 => ImageLayoutPreset::Inline,
                1 => ImageLayoutPreset::Square,
                2 => ImageLayoutPreset::Tight,
                3 => ImageLayoutPreset::Through,
                4 => ImageLayoutPreset::TopAndBottom,
                5 => ImageLayoutPreset::BehindText,
                _ => ImageLayoutPreset::InFrontOfText,
            };
            let horizontal = ["left", "center", "right"][horizontal_index as usize].to_string();
            let vertical = ["top", "center", "bottom"][vertical_index as usize].to_string();
            let spec = ImageFormatSpec {
                width_px: Some(width as f64),
                height_px: Some(height as f64),
                crop: Some(ImageCrop {
                    left: left as f64 / 100.0,
                    right: right as f64 / 100.0,
                    top: top as f64 / 100.0,
                    bottom: bottom as f64 / 100.0,
                }),
                layout: Some(layout),
                horizontal_align: (layout != ImageLayoutPreset::Inline).then_some(horizontal.clone()),
                vertical_align: (layout != ImageLayoutPreset::Inline).then_some(vertical.clone()),
                flip_horizontal: Some(flip_horizontal),
                flip_vertical: Some(flip_vertical),
                ..ImageFormatSpec::default()
            };
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                executor.execute(Box::new(FormatImageCommand::new(target(), spec.clone())), &mut ctx).unwrap();
            }
            let applied = document.paragraph(0).unwrap().runs[0].image.clone().unwrap();
            prop_assert_eq!(applied.width_px, Some(width as f64));
            prop_assert_eq!(applied.height_px, Some(height as f64));
            prop_assert_eq!(applied.crop, spec.crop);
            prop_assert_eq!(applied.layout.flip_horizontal, flip_horizontal);
            prop_assert_eq!(applied.layout.flip_vertical, flip_vertical);
            if layout == ImageLayoutPreset::Inline {
                prop_assert_eq!(applied.layout.kind, RunImageKind::Inline);
                prop_assert!(applied.layout.position_h.is_none());
                prop_assert!(applied.layout.position_v.is_none());
            } else {
                prop_assert_eq!(applied.layout.kind, RunImageKind::Floating);
                prop_assert_eq!(applied.layout.position_h.as_ref().unwrap().align.as_deref(), Some(horizontal.as_str()));
                prop_assert_eq!(applied.layout.position_v.as_ref().unwrap().align.as_deref(), Some(vertical.as_str()));
            }

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).unwrap());
            }
            prop_assert_eq!(document.paragraph(0).unwrap().runs[0].image.as_ref().unwrap(), &before);

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut ctx).unwrap());
            }
            prop_assert_eq!(document.paragraph(0).unwrap().runs[0].image.as_ref().unwrap(), &applied);
        }
    }
}
