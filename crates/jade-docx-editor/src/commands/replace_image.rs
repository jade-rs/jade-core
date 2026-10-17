use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

/// Replaces the bytes behind one selected image without disturbing its
/// DrawingML formatting. A package snapshot makes undo byte-exact across the
/// new media part, relationship, content type, and regenerated drawing.
pub struct ReplaceImageCommand {
    target: FloatingObjectTarget,
    bytes: Vec<u8>,
    mime_type: String,
    width_px: Option<f64>,
    height_px: Option<f64>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl ReplaceImageCommand {
    pub fn new(
        target: FloatingObjectTarget,
        bytes: Vec<u8>,
        mime_type: String,
        width_px: Option<f64>,
        height_px: Option<f64>,
    ) -> Self {
        Self {
            target,
            bytes,
            mime_type,
            width_px,
            height_px,
            snapshot: None,
            previous_selection: None,
            changed: false,
        }
    }
}

impl Command for ReplaceImageCommand {
    fn name(&self) -> &'static str {
        "replace_image"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.target.kind != FloatingObjectKind::Image {
            return Err(CommandError::Custom(
                "selected object is not an image".into(),
            ));
        }
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        match ctx.document.replace_image_at_target(
            self.target,
            &self.bytes,
            &self.mime_type,
            self.width_px,
            self.height_px,
        ) {
            Ok(changed) => {
                self.changed = changed;
                self.snapshot = changed.then_some(snapshot);
                self.previous_selection = previous_selection;
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                self.changed = false;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        self.changed = false;
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::WordDocument;
    use crate::Options;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use futures::executor;
    use proptest::prelude::*;

    const IMAGE_DOCX: &[u8] = include_bytes!("../../../../public/fixtures/image-floating.docx");
    const PNG_1X1_BASE64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    fn image_bytes(mime: &str) -> Vec<u8> {
        match mime {
            "image/png" => BASE64.decode(PNG_1X1_BASE64).expect("decode png"),
            "image/jpeg" => vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x01, 0xff, 0xd9],
            "image/gif" => b"GIF89a\x01\x00\x01\x00".to_vec(),
            _ => unreachable!("test MIME"),
        }
    }

    fn load_document() -> WordDocument {
        executor::block_on(WordDocument::load(IMAGE_DOCX, &Options::default()))
            .expect("load image fixture")
    }

    fn first_image_target(document: &WordDocument) -> FloatingObjectTarget {
        for paragraph_index in 0..document.paragraph_count() {
            if let Some((run_index, _)) =
                document.paragraph(paragraph_index).and_then(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .enumerate()
                        .find(|(_, run)| run.image.is_some())
                })
            {
                return FloatingObjectTarget {
                    paragraph_index,
                    run_index,
                    kind: FloatingObjectKind::Image,
                };
            }
        }
        panic!("fixture has no image")
    }

    fn selected_image(
        document: &WordDocument,
        target: FloatingObjectTarget,
    ) -> crate::document::RunImage {
        document.paragraph(target.paragraph_index).unwrap().runs[target.run_index]
            .image
            .clone()
            .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

        #[test]
        fn replaced_image_media_round_trips_and_restores_package_atomically(
            mime in prop::sample::select(vec!["image/png", "image/jpeg", "image/gif"]),
            width in 1u32..5000,
            height in 1u32..5000,
            replace_width in any::<bool>(),
            replace_height in any::<bool>(),
        ) {
            let mut document = load_document();
            let target = first_image_target(&document);
            let before_image = selected_image(&document, target);
            let baseline = document.export_docx().expect("export baseline");
            let mut selection = Some(SelectionRange::collapsed(
                jade_docx_layout::selection::TextPosition::new(target.paragraph_index, 0),
            ));
            let mut command_executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                command_executor.execute(
                    Box::new(ReplaceImageCommand::new(
                        target,
                        image_bytes(mime),
                        mime.to_string(),
                        replace_width.then_some(width as f64),
                        replace_height.then_some(height as f64),
                    )),
                    &mut ctx,
                ).expect("replace image");
            }

            let applied_image = selected_image(&document, target);
            let expected_data_uri_prefix = format!("data:{};base64,", mime);
            prop_assert_ne!(&applied_image.relationship_id, &before_image.relationship_id);
            prop_assert!(applied_image.data_uri.as_deref().is_some_and(|value| value.starts_with(&expected_data_uri_prefix)));
            prop_assert_eq!(applied_image.width_px, if replace_width { Some(width as f64) } else { before_image.width_px });
            prop_assert_eq!(applied_image.height_px, if replace_height { Some(height as f64) } else { before_image.height_px });
            prop_assert_eq!(&applied_image.crop, &before_image.crop);
            prop_assert_eq!(&applied_image.layout, &before_image.layout);
            prop_assert_eq!(&applied_image.title, &before_image.title);
            prop_assert_eq!(&applied_image.description, &before_image.description);

            let applied = document.export_docx().expect("export replacement");
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.undo(&mut ctx).expect("undo replacement"));
            }
            prop_assert_eq!(document.export_docx().expect("export undo"), baseline);
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.redo(&mut ctx).expect("redo replacement"));
            }
            prop_assert_eq!(document.export_docx().expect("export redo"), applied.clone());

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen replacement");
            let reopened_image = selected_image(&reopened, target);
            prop_assert!(reopened_image.data_uri.as_deref().is_some_and(|value| value.starts_with(&expected_data_uri_prefix)));
            prop_assert_eq!(reopened_image.width_px, applied_image.width_px);
            prop_assert_eq!(reopened_image.height_px, applied_image.height_px);
            prop_assert_eq!(reopened_image.crop, before_image.crop);
            prop_assert_eq!(reopened_image.layout, before_image.layout);
        }
    }

    #[test]
    fn identical_replacement_is_a_noop_and_invalid_payloads_leave_no_history() {
        let mut document = load_document();
        let target = first_image_target(&document);
        let image = selected_image(&document, target);
        let data_uri = image.data_uri.as_deref().expect("resolved data URI");
        let (meta, encoded) = data_uri.split_once(',').expect("data URI parts");
        let mime = meta
            .strip_prefix("data:")
            .and_then(|value| value.strip_suffix(";base64"))
            .expect("data URI MIME");
        let bytes = BASE64.decode(encoded).expect("decode fixture image");
        let baseline = document.export_docx().expect("export baseline");
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        executor
            .execute(
                Box::new(ReplaceImageCommand::new(
                    target,
                    bytes,
                    mime.to_string(),
                    None,
                    None,
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .expect("same image is accepted");
        assert!(!executor.can_undo());
        assert_eq!(document.export_docx().unwrap(), baseline);

        for (bytes, mime, width) in [
            (Vec::new(), "image/png", None),
            (image_bytes("image/png"), "image/jpeg", None),
            (image_bytes("image/png"), "image/png", Some(f64::NAN)),
        ] {
            let mut rejected = load_document();
            let rejected_target = first_image_target(&rejected);
            let rejected_baseline = rejected.export_docx().unwrap();
            let mut selection = None;
            let mut rejected_executor = CommandExecutor::new();
            assert!(rejected_executor
                .execute(
                    Box::new(ReplaceImageCommand::new(
                        rejected_target,
                        bytes,
                        mime.to_string(),
                        width,
                        None,
                    )),
                    &mut CommandContext::new(&mut rejected, &mut selection),
                )
                .is_err());
            assert!(!rejected_executor.can_undo());
            assert_eq!(rejected.export_docx().unwrap(), rejected_baseline);
        }
    }
}
