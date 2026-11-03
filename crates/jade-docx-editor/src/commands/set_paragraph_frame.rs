//! 枠 (Word's legacy text frame, `w:framePr`, ECMA-376 Part 1 §17.3.1.11):
//! paragraphs put in a frame from 開発 › 旧形式のツール › 枠, the frame's
//! 枠の書式設定 — 折り返し, サイズ, 水平/垂直の位置 and 基準, 文字列との間隔
//! — and 枠の削除. Consecutive paragraphs with the same frame share it.

use super::paragraph_property_sync::sync_paragraph_frame_xml;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{
    FrameAlign, FrameAnchor, FrameHeightRule, FrameWrap, ParagraphFrame, ParagraphProperties,
};
use jade_docx_headless_api::HeadlessParagraphFrame;
use jade_docx_layout::selection::SelectionRange;

fn twips(points: f32) -> i32 {
    (points * 20.0).round() as i32
}

/// The engine's frame for the dialog's, or why it is not one.
pub fn frame_from_headless(spec: &HeadlessParagraphFrame) -> Result<ParagraphFrame, String> {
    let anchor = |value: Option<&str>| -> Result<Option<FrameAnchor>, String> {
        Ok(match value {
            None => None,
            Some("text") => Some(FrameAnchor::Text),
            Some("margin") => Some(FrameAnchor::Margin),
            Some("page") => Some(FrameAnchor::Page),
            Some(other) => return Err(format!("unknown frame anchor `{other}`")),
        })
    };
    let align = |value: Option<&str>, horizontal: bool| -> Result<Option<FrameAlign>, String> {
        Ok(match value {
            None => None,
            Some("left") if horizontal => Some(FrameAlign::Start),
            Some("top") if !horizontal => Some(FrameAlign::Start),
            Some("center") => Some(FrameAlign::Center),
            Some("right") if horizontal => Some(FrameAlign::End),
            Some("bottom") if !horizontal => Some(FrameAlign::End),
            Some("inside") => Some(FrameAlign::Inside),
            Some("outside") => Some(FrameAlign::Outside),
            Some("inline") if !horizontal => Some(FrameAlign::Inline),
            Some(other) => return Err(format!("unknown frame alignment `{other}`")),
        })
    };
    let wrap = match spec.wrap.as_deref() {
        None => None,
        Some("around") => Some(FrameWrap::Around),
        Some("none") => Some(FrameWrap::None),
        Some("notBeside") => Some(FrameWrap::NotBeside),
        Some("tight") => Some(FrameWrap::Tight),
        Some("through") => Some(FrameWrap::Through),
        Some("auto") => Some(FrameWrap::Auto),
        Some(other) => return Err(format!("unknown frame wrap `{other}`")),
    };
    let height_rule = match spec.height_rule.as_deref() {
        None | Some("auto") => FrameHeightRule::Auto,
        Some("atLeast") => FrameHeightRule::AtLeast,
        Some("exact") => FrameHeightRule::Exact,
        Some(other) => return Err(format!("unknown frame height rule `{other}`")),
    };
    let positive = |value: Option<f32>, what: &str| -> Result<Option<i32>, String> {
        match value {
            Some(value) if !value.is_finite() || value < 0.0 => {
                Err(format!("the frame's {what} must not be negative"))
            }
            value => Ok(value.map(twips)),
        }
    };
    Ok(ParagraphFrame {
        width_twips: positive(spec.width, "width")?,
        height_twips: positive(spec.height, "height")?,
        height_rule,
        x_twips: spec.x.map(twips),
        y_twips: spec.y.map(twips),
        x_align: align(spec.x_align.as_deref(), true)?,
        y_align: align(spec.y_align.as_deref(), false)?,
        h_anchor: anchor(spec.h_anchor.as_deref())?,
        v_anchor: anchor(spec.v_anchor.as_deref())?,
        wrap,
        h_space_twips: positive(spec.h_space, "horizontal distance")?,
        v_space_twips: positive(spec.v_space, "vertical distance")?,
    })
}

/// The dialog's frame for the engine's: what 枠の書式設定 opens on.
pub fn frame_to_headless(frame: &ParagraphFrame) -> HeadlessParagraphFrame {
    let points = |twips: i32| twips as f32 / 20.0;
    let anchor = |anchor: Option<FrameAnchor>| {
        anchor.map(|anchor| {
            match anchor {
                FrameAnchor::Text => "text",
                FrameAnchor::Margin => "margin",
                FrameAnchor::Page => "page",
            }
            .to_string()
        })
    };
    let align = |align: Option<FrameAlign>, horizontal: bool| {
        align.map(|align| {
            match align {
                FrameAlign::Start if horizontal => "left",
                FrameAlign::Start => "top",
                FrameAlign::Center => "center",
                FrameAlign::End if horizontal => "right",
                FrameAlign::End => "bottom",
                FrameAlign::Inside => "inside",
                FrameAlign::Outside => "outside",
                FrameAlign::Inline => "inline",
            }
            .to_string()
        })
    };
    HeadlessParagraphFrame {
        width: frame.width_twips.map(points),
        height: frame.height_twips.map(points),
        height_rule: Some(
            match frame.height_rule {
                FrameHeightRule::Auto => "auto",
                FrameHeightRule::AtLeast => "atLeast",
                FrameHeightRule::Exact => "exact",
            }
            .to_string(),
        ),
        x: frame.x_twips.map(points),
        x_align: align(frame.x_align, true),
        h_anchor: anchor(frame.h_anchor),
        y: frame.y_twips.map(points),
        y_align: align(frame.y_align, false),
        v_anchor: anchor(frame.v_anchor),
        wrap: frame.wrap.map(|wrap| {
            match wrap {
                FrameWrap::Around => "around",
                FrameWrap::None => "none",
                FrameWrap::NotBeside => "notBeside",
                FrameWrap::Tight => "tight",
                FrameWrap::Through => "through",
                FrameWrap::Auto => "auto",
            }
            .to_string()
        }),
        h_space: frame.h_space_twips.map(points),
        v_space: frame.v_space_twips.map(points),
    }
}

/// `w:framePr`'s attributes for `frame`, in the schema's order.
pub fn frame_attributes(frame: &ParagraphFrame) -> Vec<(&'static str, String)> {
    let spec = frame_to_headless(frame);
    let mut out = Vec::new();
    let mut push = |name: &'static str, value: Option<String>| {
        if let Some(value) = value {
            out.push((name, value));
        }
    };
    push("w:w", frame.width_twips.map(|value| value.to_string()));
    push("w:h", frame.height_twips.map(|value| value.to_string()));
    push("w:vSpace", frame.v_space_twips.map(|value| value.to_string()));
    push("w:hSpace", frame.h_space_twips.map(|value| value.to_string()));
    push("w:wrap", spec.wrap);
    push("w:hAnchor", spec.h_anchor);
    push("w:vAnchor", spec.v_anchor);
    push("w:x", frame.x_twips.map(|value| value.to_string()));
    push("w:xAlign", spec.x_align);
    push("w:y", frame.y_twips.map(|value| value.to_string()));
    push("w:yAlign", spec.y_align);
    push(
        "w:hRule",
        (frame.height_rule != FrameHeightRule::Auto).then(|| spec.height_rule.unwrap_or_default()),
    );
    out
}

/// Puts paragraphs in a frame, or takes them out of theirs.
pub struct SetParagraphFrameCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    frame: Option<ParagraphFrame>,
    previous_properties: Vec<(usize, Option<ParagraphProperties>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetParagraphFrameCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize, frame: Option<ParagraphFrame>) -> Self {
        Self {
            start_paragraph: start_paragraph.min(end_paragraph),
            end_paragraph: start_paragraph.max(end_paragraph),
            frame,
            previous_properties: Vec::new(),
            previous_selection: None,
        }
    }
}

impl Command for SetParagraphFrameCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "set_paragraph_frame"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, end) = (self.start_paragraph, self.end_paragraph);
        if end >= ctx.document.paragraph_count() {
            return Err(CommandError::ParagraphUnavailable);
        }
        for index in start..=end {
            if !ctx.document.is_top_level_body_paragraph_surface(index) {
                return Err(CommandError::Custom(
                    "a frame holds body paragraphs outside tables".to_string(),
                ));
            }
            let has_drop_cap = ctx
                .document
                .paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .is_some_and(|props| props.drop_cap.is_some());
            if has_drop_cap {
                return Err(CommandError::Custom(
                    "a paragraph with a drop cap cannot be framed".to_string(),
                ));
            }
        }
        self.previous_properties.clear();
        self.previous_selection = *ctx.selection;
        for index in start..=end {
            let paragraph = ctx
                .document
                .paragraph_mut(index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous_properties
                .push((index, paragraph.properties.clone()));
            paragraph.properties.get_or_insert_with(Default::default).frame = self.frame;
            sync_paragraph_frame_xml(ctx.document, index, self.frame.as_ref());
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (index, previous) in std::mem::take(&mut self.previous_properties) {
            let frame = previous.as_ref().and_then(|props| props.frame);
            let paragraph = ctx
                .document
                .paragraph_mut(index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            paragraph.properties = previous;
            sync_paragraph_frame_xml(ctx.document, index, frame.as_ref());
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;

    fn fixture() -> WordDocument {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contract.docx");
        let data = std::fs::read(path).expect("read fixture");
        executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx")
    }

    fn xml(document: &WordDocument) -> String {
        document
            .raw_document
            .as_ref()
            .expect("raw xml")
            .to_string()
            .expect("serialize xml")
    }

    #[test]
    fn the_dialogs_frame_round_trips_through_the_engines() {
        let spec = HeadlessParagraphFrame {
            width: Some(144.0),
            height: Some(72.0),
            height_rule: Some("exact".to_string()),
            x_align: Some("center".to_string()),
            h_anchor: Some("margin".to_string()),
            y: Some(36.0),
            v_anchor: Some("text".to_string()),
            wrap: Some("around".to_string()),
            h_space: Some(9.0),
            v_space: Some(0.0),
            ..Default::default()
        };
        let frame = frame_from_headless(&spec).unwrap();
        assert_eq!(frame.width_twips, Some(2880));
        assert_eq!(frame.x_align, Some(FrameAlign::Center));
        assert_eq!(frame_to_headless(&frame), spec);
        assert!(frame_from_headless(&HeadlessParagraphFrame {
            width: Some(-1.0),
            ..Default::default()
        })
        .is_err());
    }

    #[test]
    fn a_frame_is_written_read_removed_and_undone() {
        let mut document = fixture();
        let paragraph = (0..document.paragraph_count())
            .find(|&index| {
                document.is_top_level_body_paragraph_surface(index)
                    && document
                        .paragraph(index)
                        .is_some_and(|p| !p.runs.is_empty())
            })
            .expect("a body paragraph");
        let before = xml(&document);
        let frame = frame_from_headless(&HeadlessParagraphFrame {
            width: Some(144.0),
            x_align: Some("right".to_string()),
            h_anchor: Some("margin".to_string()),
            y: Some(0.05),
            v_anchor: Some("text".to_string()),
            wrap: Some("around".to_string()),
            ..Default::default()
        })
        .unwrap();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphFrameCommand::new(paragraph, paragraph, Some(frame))),
                    &mut ctx,
                )
                .unwrap();
        }
        let after = xml(&document);
        assert!(
            after.contains(r#"<w:framePr w:w="2880" w:wrap="around" w:hAnchor="margin" w:vAnchor="text" w:xAlign="right" w:y="1"/>"#),
            "{after}"
        );
        let saved = document.export_docx().expect("export");
        let reloaded =
            executor::block_on(WordDocument::load(&saved, &Options::default())).expect("reload");
        let read = reloaded
            .paragraph(paragraph)
            .and_then(|p| p.properties.as_ref())
            .and_then(|props| props.frame);
        assert_eq!(read, Some(frame));

        // 枠の削除, then undo twice.
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphFrameCommand::new(paragraph, paragraph, None)),
                    &mut ctx,
                )
                .unwrap();
        }
        assert!(!xml(&document).contains("w:framePr w:w=\"2880\""));
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        executor.undo(&mut ctx).unwrap();
        assert_eq!(xml(&document), before);
    }
}
