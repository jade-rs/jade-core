use crate::commands::format_image::ImageLayoutPreset;
use crate::commands::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{
    ImagePosition, Run, RunImageKind, TextBoxBorderStyle, TextBoxVerticalAlignment, WrapMode,
    WrapTextSide,
};
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// One edit of a selected text box or shape: colors, and Word's 図形の書式
/// layout controls — 文字列の折り返し (`layout`), which side text may run
/// on, the distance from text, size, and page alignment. Every field is
/// optional and `None` leaves that aspect as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextBoxFormatSpec {
    pub fill_color: Option<String>,
    pub clear_fill: bool,
    pub border_color: Option<String>,
    pub clear_border: bool,
    pub layout: Option<ImageLayoutPreset>,
    pub wrap_side: Option<WrapTextSide>,
    pub width_px: Option<f64>,
    pub height_px: Option<f64>,
    pub horizontal_align: Option<String>,
    pub vertical_align: Option<String>,
    pub dist_top: Option<f64>,
    pub dist_bottom: Option<f64>,
    pub dist_left: Option<f64>,
    pub dist_right: Option<f64>,
    /// Inner margins of the text body (Word's 図形の書式設定 ▸ テキストボックス
    /// ▸ 内部の余白), in pixels.
    pub inset_left: Option<f64>,
    pub inset_top: Option<f64>,
    pub inset_right: Option<f64>,
    pub inset_bottom: Option<f64>,
    /// Vertical placement of the text body inside the shape.
    pub text_vertical_align: Option<TextBoxVerticalAlignment>,
    /// Clockwise rotation in degrees (Word's 回転); normalized to 0..360.
    pub rotation_deg: Option<f64>,
}

impl TextBoxFormatSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.clear_fill && self.fill_color.is_some() {
            return Err("fillColor and clearFill cannot be supplied together".into());
        }
        if self.clear_border && self.border_color.is_some() {
            return Err("borderColor and clearBorder cannot be supplied together".into());
        }
        for (label, color) in [
            ("fillColor", self.fill_color.as_deref()),
            ("borderColor", self.border_color.as_deref()),
        ] {
            if color.is_some_and(|color| !is_hex_color(color)) {
                return Err(format!("{label} must be a #RRGGBB color"));
            }
        }
        for (label, value) in [("width", self.width_px), ("height", self.height_px)] {
            if let Some(value) = value {
                if !value.is_finite() || !(1.0..=100_000.0).contains(&value) {
                    return Err(format!("shape {label} must be between 1 and 100000 pixels"));
                }
            }
        }
        for (label, value) in [
            ("distTop", self.dist_top),
            ("distBottom", self.dist_bottom),
            ("distLeft", self.dist_left),
            ("distRight", self.dist_right),
        ] {
            if let Some(value) = value {
                if !value.is_finite() || !(0.0..=10_000.0).contains(&value) {
                    return Err(format!("shape {label} must be between 0 and 10000 pixels"));
                }
            }
        }
        for (label, value) in [
            ("insetLeft", self.inset_left),
            ("insetTop", self.inset_top),
            ("insetRight", self.inset_right),
            ("insetBottom", self.inset_bottom),
        ] {
            if let Some(value) = value {
                if !value.is_finite() || !(0.0..=10_000.0).contains(&value) {
                    return Err(format!("shape {label} must be between 0 and 10000 pixels"));
                }
            }
        }
        if let Some(rotation) = self.rotation_deg {
            if !rotation.is_finite() {
                return Err("rotationDeg must be a finite number".into());
            }
        }
        if let Some(align) = self.horizontal_align.as_deref() {
            if !matches!(align, "left" | "center" | "right" | "inside" | "outside") {
                return Err(
                    "horizontalAlign must be left, center, right, inside or outside".into(),
                );
            }
        }
        if let Some(align) = self.vertical_align.as_deref() {
            if !matches!(align, "top" | "center" | "bottom" | "inside" | "outside") {
                return Err("verticalAlign must be top, center, bottom, inside or outside".into());
            }
        }
        Ok(())
    }
}

fn is_hex_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

pub struct FormatTextBoxCommand {
    target: FloatingObjectTarget,
    spec: TextBoxFormatSpec,
    previous_runs: Option<Vec<Run>>,
    previous_selection: Option<SelectionRange>,
}

impl FormatTextBoxCommand {
    pub fn new(target: FloatingObjectTarget, spec: TextBoxFormatSpec) -> Self {
        Self {
            target,
            spec,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for FormatTextBoxCommand {
    fn name(&self) -> &'static str {
        "format_text_box"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.spec.validate().map_err(CommandError::Custom)?;
        if self.target.kind != FloatingObjectKind::TextBox {
            return Err(CommandError::Custom(
                "selected object is not a text box or shape".into(),
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
        let text_box = run.text_box.as_mut().ok_or_else(|| {
            CommandError::Custom("target run does not contain a text box or shape".into())
        })?;

        if self.spec.clear_fill {
            text_box.fill_color = None;
            text_box.fill_theme_color = None;
            text_box.fill_theme_transforms.clear();
        } else if let Some(color) = self.spec.fill_color.as_ref() {
            text_box.fill_color = Some(color.to_ascii_uppercase());
            text_box.fill_theme_color = None;
            text_box.fill_theme_transforms.clear();
        }
        if self.spec.clear_border {
            text_box.border.color = None;
            text_box.border.theme_color = None;
            text_box.border.theme_transforms.clear();
            text_box.border.style = TextBoxBorderStyle::None;
        } else if let Some(color) = self.spec.border_color.as_ref() {
            text_box.border.color = Some(color.to_ascii_uppercase());
            text_box.border.theme_color = None;
            text_box.border.theme_transforms.clear();
            if text_box.border.style == TextBoxBorderStyle::None {
                text_box.border.style = TextBoxBorderStyle::Solid;
            }
        }

        if let Some(width) = self.spec.width_px {
            text_box.width_px = Some(width);
        }
        if let Some(height) = self.spec.height_px {
            text_box.height_px = Some(height);
        }
        // The layout presets mirror Word's 文字列の折り返し gallery, the way
        // pictures take them (`FormatImageCommand`).
        if let Some(preset) = self.spec.layout {
            match preset {
                ImageLayoutPreset::Inline => {
                    text_box.layout.kind = RunImageKind::Inline;
                    text_box.layout.wrap = WrapMode::None;
                    text_box.layout.behind_doc = false;
                    text_box.layout.position_h = None;
                    text_box.layout.position_v = None;
                }
                ImageLayoutPreset::Square
                | ImageLayoutPreset::Tight
                | ImageLayoutPreset::Through
                | ImageLayoutPreset::TopAndBottom
                | ImageLayoutPreset::BehindText
                | ImageLayoutPreset::InFrontOfText => {
                    text_box.layout.kind = RunImageKind::Floating;
                    text_box.layout.wrap = match preset {
                        ImageLayoutPreset::Square => WrapMode::Square,
                        ImageLayoutPreset::Tight => WrapMode::Tight,
                        ImageLayoutPreset::Through => WrapMode::Through,
                        ImageLayoutPreset::TopAndBottom => WrapMode::TopAndBottom,
                        ImageLayoutPreset::BehindText | ImageLayoutPreset::InFrontOfText => {
                            WrapMode::None
                        }
                        ImageLayoutPreset::Inline => unreachable!(),
                    };
                    text_box.layout.behind_doc = preset == ImageLayoutPreset::BehindText;
                    text_box
                        .layout
                        .position_h
                        .get_or_insert_with(|| ImagePosition {
                            relative_from: Some("margin".into()),
                            offset_px: Some(0.0),
                            align: None,
                        });
                    text_box
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
        if let Some(side) = self.spec.wrap_side {
            text_box.layout.wrap_text = side;
        }
        if let Some(align) = self.spec.horizontal_align.as_ref() {
            text_box.layout.kind = RunImageKind::Floating;
            text_box.layout.position_h = Some(ImagePosition {
                relative_from: Some("margin".into()),
                offset_px: None,
                align: Some(align.clone()),
            });
        }
        if let Some(align) = self.spec.vertical_align.as_ref() {
            text_box.layout.kind = RunImageKind::Floating;
            text_box.layout.position_v = Some(ImagePosition {
                relative_from: Some("margin".into()),
                offset_px: None,
                align: Some(align.clone()),
            });
        }
        for (destination, value) in [
            (&mut text_box.layout.dist_top, self.spec.dist_top),
            (&mut text_box.layout.dist_bottom, self.spec.dist_bottom),
            (&mut text_box.layout.dist_left, self.spec.dist_left),
            (&mut text_box.layout.dist_right, self.spec.dist_right),
        ] {
            if let Some(value) = value {
                *destination = Some(value);
            }
        }
        for (destination, value) in [
            (&mut text_box.insets.left_px, self.spec.inset_left),
            (&mut text_box.insets.top_px, self.spec.inset_top),
            (&mut text_box.insets.right_px, self.spec.inset_right),
            (&mut text_box.insets.bottom_px, self.spec.inset_bottom),
        ] {
            if let Some(value) = value {
                *destination = value;
            }
        }
        if let Some(alignment) = self.spec.text_vertical_align {
            text_box.vertical_alignment = alignment;
        }
        if let Some(rotation) = self.spec.rotation_deg {
            let normalized = rotation.rem_euclid(360.0);
            text_box.layout.rotation_deg = (normalized.abs() > f64::EPSILON).then_some(normalized);
        }

        ctx.document
            .sync_paragraph_text(self.target.paragraph_index);
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(
            self.target.paragraph_index,
            0,
        )));
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
            .ok_or_else(|| CommandError::Custom("missing shape-format backup for undo".into()))?
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
    use crate::document::{Body, BodyElement, Document, Paragraph, RunTextBox, TextBoxShape};
    use crate::word_document::WordDocument;
    use proptest::prelude::*;

    fn shape_document() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: String::new(),
                        properties: None,
                        image: None,
                        text_box: Some(RunTextBox {
                            fill_color: Some("#102030".into()),
                            fill_theme_color: Some("accent1".into()),
                            shape: TextBoxShape::RoundedRectangle,
                            ..RunTextBox::default()
                        }),
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
            kind: FloatingObjectKind::TextBox,
        }
    }

    fn color(value: u32) -> String {
        format!("#{:06X}", value & 0x00ff_ffff)
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name)
    }

    fn document_xml(word_doc: &WordDocument) -> String {
        use std::io::Read;
        let exported = word_doc.export_docx().expect("export docx");
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip archive");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut xml)
            .expect("read document.xml");
        xml
    }

    fn text_box_target(word_doc: &WordDocument) -> FloatingObjectTarget {
        for paragraph_index in 0..word_doc.paragraph_count() {
            let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
                continue;
            };
            if let Some(run_index) = paragraph.runs.iter().position(|run| run.text_box.is_some()) {
                return FloatingObjectTarget {
                    paragraph_index,
                    run_index,
                    kind: FloatingObjectKind::TextBox,
                };
            }
        }
        panic!("fixture has no text box");
    }

    #[test]
    fn formatting_a_vml_text_box_in_a_mixed_text_run_keeps_both_contents() {
        use std::io::{Read, Write};

        let fixture = std::fs::read(fixture_path("document-variable-fields.docx")).unwrap();
        for text_before_box in [true, false] {
            let mut source = zip::ZipArchive::new(std::io::Cursor::new(&fixture)).unwrap();
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for index in 0..source.len() {
                let mut entry = source.by_index(index).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                if entry.name().unwrap() == "word/document.xml" {
                    let xml = String::from_utf8(bytes).unwrap();
                    bytes = if text_before_box {
                        xml.replacen("<w:pict", "<w:t>host-text</w:t><w:pict", 1)
                    } else {
                        xml.replacen("</w:pict>", "</w:pict><w:t>host-text</w:t>", 1)
                    }
                    .into_bytes();
                }
                writer
                    .start_file(
                        entry.name().unwrap(),
                        zip::write::SimpleFileOptions::default(),
                    )
                    .unwrap();
                writer.write_all(&bytes).unwrap();
            }
            let data = writer.finish().unwrap().into_inner();
            let mut word_doc =
                futures::executor::block_on(WordDocument::load(&data, &crate::Options::default()))
                    .unwrap();
            let target = text_box_target(&word_doc);
            assert!(
                word_doc.paragraph(target.paragraph_index).unwrap().runs[target.run_index]
                    .text
                    .contains("host-text")
            );
            let mut selection = None;
            CommandExecutor::new()
                .execute(
                    Box::new(FormatTextBoxCommand::new(
                        target,
                        TextBoxFormatSpec {
                            fill_color: Some("#FF0000".to_string()),
                            ..TextBoxFormatSpec::default()
                        },
                    )),
                    &mut CommandContext::new(&mut word_doc, &mut selection),
                )
                .unwrap();
            let after = document_xml(&word_doc);
            assert_eq!(after.matches("host-text").count(), 1);
            assert_eq!(after.matches("<w:pict").count(), 0);
            assert_eq!(
                after.matches("<w:drawing").count(),
                1,
                "the mixed run must retain its text box"
            );
            assert_eq!(after.matches("<w:txbxContent").count(), 1);
            assert!(after.contains("FF0000"));
            assert_eq!(
                after.find("host-text") < after.find("<w:drawing"),
                text_before_box,
                "the box must stay on the same side of the host text"
            );
            // A subsequent edit must also keep the box after it has become
            // DrawingML; the in-memory run still contains its host text.
            CommandExecutor::new()
                .execute(
                    Box::new(FormatTextBoxCommand::new(
                        target,
                        TextBoxFormatSpec {
                            fill_color: Some("#00FF00".to_string()),
                            ..TextBoxFormatSpec::default()
                        },
                    )),
                    &mut CommandContext::new(&mut word_doc, &mut selection),
                )
                .unwrap();
            let second = document_xml(&word_doc);
            assert_eq!(
                second.matches("<w:drawing").count(),
                1,
                "repeated formatting must keep the box"
            );
            assert!(second.contains("00FF00"));
            assert_eq!(
                second.find("host-text") < second.find("<w:drawing"),
                text_before_box
            );
            let reloaded = futures::executor::block_on(WordDocument::load(
                &word_doc.export_docx().unwrap(),
                &crate::Options::default(),
            ))
            .unwrap();
            assert!(reloaded
                .paragraph(target.paragraph_index)
                .unwrap()
                .runs
                .iter()
                .any(|run| run.text.contains("host-text")));
            let text_box = (0..reloaded.paragraph_count())
                .find(|&index| reloaded.is_text_box_paragraph_surface(index))
                .unwrap();
            assert!(reloaded
                .paragraph(text_box)
                .unwrap()
                .runs
                .iter()
                .any(|run| run.field.is_some()));
        }
    }

    #[test]
    fn formatting_a_vml_text_box_saves_it_once_with_its_fields() {
        // document-variable-fields.docx saves its text box as VML only, and
        // the box's paragraph holds a DOCVARIABLE field. A fill rebuilt the
        // box as a `w:drawing` beside the `w:pict`, so the file held the box
        // twice, and the rebuilt text had the field's stale result as plain
        // text — the copy a reload reads.
        let data =
            std::fs::read(fixture_path("document-variable-fields.docx")).expect("read fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &crate::Options::default()))
                .expect("load fixture");
        let target = text_box_target(&word_doc);
        let before = document_xml(&word_doc);
        assert_eq!(before.matches("<w:pict").count(), 1);
        assert_eq!(before.matches("<w:drawing").count(), 0);
        let mut selection = None;
        CommandExecutor::new()
            .execute(
                Box::new(FormatTextBoxCommand::new(
                    target,
                    TextBoxFormatSpec {
                        fill_color: Some("#FF0000".to_string()),
                        ..TextBoxFormatSpec::default()
                    },
                )),
                &mut CommandContext::new(&mut word_doc, &mut selection),
            )
            .expect("fill");
        let after = document_xml(&word_doc);
        assert_eq!(
            after.matches("<w:pict").count() + after.matches("<w:drawing").count(),
            1,
            "{after}"
        );
        assert_eq!(after.matches("<w:txbxContent").count(), 1);
        assert!(after.contains("FF0000"));
        let reloaded = futures::executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &crate::Options::default(),
        ))
        .expect("reload");
        let text_box = (0..reloaded.paragraph_count())
            .find(|&index| reloaded.is_text_box_paragraph_surface(index))
            .expect("the text box paragraph");
        assert!(reloaded
            .paragraph(text_box)
            .is_some_and(|paragraph| paragraph.runs.iter().any(|run| run.field.is_some())));
    }

    #[test]
    fn editing_the_host_paragraph_keeps_one_word_text_box_with_its_wrap_and_frame() {
        // Word wraps its text boxes in `mc:AlternateContent`. Rebuilding the
        // host paragraph used to add a second `w:drawing` next to the
        // preserved block, so the saved file showed the box twice, and the
        // generated copy had lost the wrap, the distances and the anchor.
        let data = std::fs::read(fixture_path("textbox.docx")).expect("read textbox fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &crate::Options::default()))
                .expect("load textbox fixture");
        let target = text_box_target(&word_doc);
        let before = document_xml(&word_doc);
        assert_eq!(before.matches("<w:drawing").count(), 1);

        // Any edit of the host paragraph regenerates its runs.
        word_doc
            .paragraph_mut(target.paragraph_index)
            .expect("host paragraph")
            .runs
            .insert(
                0,
                Run {
                    text: "Z".to_string(),
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
                },
            );
        word_doc.sync_paragraph_text(target.paragraph_index);
        word_doc.rebuild_paragraph_surfaces();

        let after = document_xml(&word_doc);
        assert_eq!(after.matches("<w:drawing").count(), 1, "{after}");
        assert_eq!(after.matches("<mc:AlternateContent").count(), 1);
        assert_eq!(after.matches("<wps:txbx").count(), 1);
        assert_eq!(after.matches("<wp:wrapTopAndBottom").count(), 1);
        assert!(
            after.contains("distT=\"152400\""),
            "anchor distances kept: {after}"
        );
        assert!(
            after.contains("<wp:positionH relativeFrom=\"margin\">"),
            "{after}"
        );
        assert!(
            after.contains("<wp:positionV relativeFrom=\"line\">"),
            "{after}"
        );
        assert!(
            !after.contains("<mc:Fallback>"),
            "the stale VML fallback is dropped with the rebuilt choice: {after}"
        );

        let reopened = futures::executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &crate::Options::default(),
        ))
        .expect("reopen");
        let boxes = (0..reopened.paragraph_count())
            .filter_map(|index| reopened.paragraph(index))
            .flat_map(|paragraph| paragraph.runs.iter())
            .filter(|run| run.text_box.is_some())
            .count();
        assert_eq!(boxes, 1);
    }

    #[test]
    fn list_and_alignment_edits_inside_a_text_box_reach_the_package() {
        use crate::commands::apply_multilevel_list::ApplyMultilevelListCommand;
        use crate::commands::set_paragraph_alignment::SetParagraphAlignmentCommand;
        use crate::numbering::MultilevelListPreset;

        let data = std::fs::read(fixture_path("textbox.docx")).expect("read textbox fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &crate::Options::default()))
                .expect("load textbox fixture");
        let target = text_box_target(&word_doc);
        let paragraph_index = (0..word_doc.paragraph_count())
            .find(|index| {
                word_doc
                    .text_box_host_for_surface(*index)
                    .is_some_and(|host| {
                        host.paragraph_index == target.paragraph_index
                            && host.run_index == target.run_index
                    })
            })
            .expect("a paragraph inside the text box");
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(ApplyMultilevelListCommand::new(
                        paragraph_index,
                        paragraph_index,
                        MultilevelListPreset::BulletHierarchy,
                        0,
                    )),
                    &mut ctx,
                )
                .expect("bullets inside the text box");
            executor
                .execute(
                    Box::new(SetParagraphAlignmentCommand::new(
                        paragraph_index,
                        paragraph_index,
                        "center",
                    )),
                    &mut ctx,
                )
                .expect("center inside the text box");
        }

        let xml = document_xml(&word_doc);
        let content_start = xml.find("<w:txbxContent>").expect("text box content");
        let content = &xml[content_start..];
        let properties_end = content
            .find("</w:pPr>")
            .expect("paragraph properties in the box");
        let properties = &content[..properties_end];
        assert!(properties.contains("<w:numPr>"), "{properties}");
        assert!(
            properties.contains("<w:jc w:val=\"center\""),
            "{properties}"
        );

        let reopened = futures::executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &crate::Options::default(),
        ))
        .expect("reopen");
        let paragraph = reopened
            .paragraph(paragraph_index)
            .expect("text box paragraph after reopen");
        let properties = paragraph.properties.as_ref().expect("properties survive");
        assert!(properties
            .numbering
            .as_ref()
            .is_some_and(|n| n.id.is_some()));
        assert_eq!(properties.alignment.as_deref(), Some("center"));
    }

    #[test]
    fn wrap_layout_edits_round_trip_through_the_package() {
        let data = std::fs::read(fixture_path("textbox.docx")).expect("read textbox fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &crate::Options::default()))
                .expect("load textbox fixture");
        let target = text_box_target(&word_doc);
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(FormatTextBoxCommand::new(
                        target,
                        TextBoxFormatSpec {
                            layout: Some(ImageLayoutPreset::Square),
                            wrap_side: Some(WrapTextSide::Right),
                            dist_left: Some(12.0),
                            dist_right: Some(24.0),
                            width_px: Some(300.0),
                            inset_left: Some(20.0),
                            text_vertical_align: Some(TextBoxVerticalAlignment::Center),
                            rotation_deg: Some(90.0),
                            ..TextBoxFormatSpec::default()
                        },
                    )),
                    &mut ctx,
                )
                .expect("square wrap");
        }
        let xml = document_xml(&word_doc);
        assert_eq!(xml.matches("<w:drawing").count(), 1);
        assert!(xml.contains("<wp:wrapSquare wrapText=\"right\""), "{xml}");
        assert!(xml.contains("distL=\"114300\""), "12px = 114300 EMU: {xml}");
        assert!(xml.contains("distR=\"228600\""), "{xml}");
        assert!(
            xml.contains("<wp:extent cx=\"2857500\""),
            "300px width: {xml}"
        );
        assert!(
            xml.contains("<a:ext cx=\"2857500\""),
            "shape xfrm follows: {xml}"
        );
        assert!(
            xml.contains("lIns=\"190500\""),
            "20px inset = 190500 EMU: {xml}"
        );
        assert!(xml.contains("anchor=\"ctr\""), "{xml}");
        assert!(
            xml.contains("<a:xfrm rot=\"5400000\""),
            "90° = 5400000: {xml}"
        );

        let reopened = futures::executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &crate::Options::default(),
        ))
        .expect("reopen");
        let text_box = reopened
            .paragraph(target.paragraph_index)
            .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.text_box.clone()))
            .expect("text box survives");
        assert_eq!(text_box.layout.wrap, WrapMode::Square);
        assert_eq!(text_box.layout.wrap_text, WrapTextSide::Right);
        assert_eq!(text_box.layout.dist_left, Some(12.0));
        assert_eq!(text_box.width_px, Some(300.0));
        assert_eq!(text_box.insets.left_px, 20.0);
        assert_eq!(
            text_box.vertical_alignment,
            TextBoxVerticalAlignment::Center
        );
        assert_eq!(text_box.layout.rotation_deg, Some(90.0));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        let xml = document_xml(&word_doc);
        assert!(xml.contains("<wp:wrapTopAndBottom"), "{xml}");
        assert!(!xml.contains("<wp:wrapSquare"), "{xml}");
    }

    #[test]
    fn rejects_invalid_or_conflicting_colors_without_history() {
        for spec in [
            TextBoxFormatSpec {
                fill_color: Some("red".into()),
                ..TextBoxFormatSpec::default()
            },
            TextBoxFormatSpec {
                border_color: Some("#123456".into()),
                clear_border: true,
                ..TextBoxFormatSpec::default()
            },
        ] {
            let mut document = shape_document();
            let mut selection = None;
            let mut executor = CommandExecutor::new();
            let before = document.paragraph(0).unwrap().runs[0]
                .text_box
                .as_ref()
                .unwrap()
                .fill_color
                .clone();
            assert!(executor
                .execute(
                    Box::new(FormatTextBoxCommand::new(target(), spec)),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .is_err());
            assert!(!executor.can_undo());
            assert_eq!(
                document.paragraph(0).unwrap().runs[0]
                    .text_box
                    .as_ref()
                    .unwrap()
                    .fill_color,
                before
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 192, ..ProptestConfig::default() })]

        #[test]
        fn shape_fill_and_outline_apply_undo_redo_atomically(
            fill in 0u32..=0x00ff_ffff,
            border in 0u32..=0x00ff_ffff,
            clear_fill in any::<bool>(),
            clear_border in any::<bool>(),
            prior_no_border in any::<bool>(),
        ) {
            let mut document = shape_document();
            {
                let text_box = document.paragraph_mut(0).unwrap().runs[0].text_box.as_mut().unwrap();
                text_box.border.theme_color = Some("accent2".into());
                if prior_no_border { text_box.border.style = TextBoxBorderStyle::None; }
            }
            let mut selection = None;
            let mut executor = CommandExecutor::new();
            let before = document.paragraph(0).unwrap().runs[0].clone();
            let spec = TextBoxFormatSpec {
                fill_color: (!clear_fill).then(|| color(fill)),
                clear_fill,
                border_color: (!clear_border).then(|| color(border)),
                clear_border,
                ..TextBoxFormatSpec::default()
            };
            executor.execute(
                Box::new(FormatTextBoxCommand::new(target(), spec.clone())),
                &mut CommandContext::new(&mut document, &mut selection),
            ).unwrap();
            let applied = document.paragraph(0).unwrap().runs[0].clone();
            let text_box = applied.text_box.as_ref().unwrap();
            prop_assert_eq!(text_box.fill_color.as_deref(), (!clear_fill).then_some(spec.fill_color.as_deref()).flatten());
            prop_assert_eq!(text_box.border.color.as_deref(), (!clear_border).then_some(spec.border_color.as_deref()).flatten());
            prop_assert_eq!(text_box.border.style == TextBoxBorderStyle::None, clear_border);
            prop_assert!(text_box.fill_theme_color.is_none());
            prop_assert!(text_box.border.theme_color.is_none());

            prop_assert!(executor.undo(&mut CommandContext::new(&mut document, &mut selection)).unwrap());
            let restored = &document.paragraph(0).unwrap().runs[0];
            prop_assert_eq!(restored.text_box.as_ref().unwrap().fill_color.as_deref(), before.text_box.as_ref().unwrap().fill_color.as_deref());
            prop_assert_eq!(restored.text_box.as_ref().unwrap().border.color.as_deref(), before.text_box.as_ref().unwrap().border.color.as_deref());

            prop_assert!(executor.redo(&mut CommandContext::new(&mut document, &mut selection)).unwrap());
            let redone = &document.paragraph(0).unwrap().runs[0];
            prop_assert_eq!(redone.text_box.as_ref().unwrap().fill_color.as_deref(), applied.text_box.as_ref().unwrap().fill_color.as_deref());
            prop_assert_eq!(redone.text_box.as_ref().unwrap().border.color.as_deref(), applied.text_box.as_ref().unwrap().border.color.as_deref());
            prop_assert_eq!(redone.text_box.as_ref().unwrap().border.style.clone(), applied.text_box.as_ref().unwrap().border.style.clone());
        }
    }
}
