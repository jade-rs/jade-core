use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{
    ChartInsertSpec, FloatingObjectKind, FloatingObjectTarget, WordDocumentSnapshot,
};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Inserts a native ChartML drawing at the exact current-state caret.
pub struct InsertChartCommand {
    range: SelectionRange,
    spec: ChartInsertSpec,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    inserted_target: Option<FloatingObjectTarget>,
}

impl InsertChartCommand {
    pub fn new(range: SelectionRange, spec: ChartInsertSpec) -> Self {
        Self {
            range,
            spec,
            snapshot: None,
            previous_selection: None,
            inserted_target: None,
        }
    }

    pub fn inserted_target(&self) -> Option<FloatingObjectTarget> {
        self.inserted_target
    }
}

impl Command for InsertChartCommand {
    fn name(&self) -> &'static str {
        "insert_chart"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if !self.range.is_collapsed() {
            return Err(CommandError::Custom(
                "chart insertion requires a collapsed selection".into(),
            ));
        }
        let rendered_position = self.range.focus;
        if !ctx
            .document
            .is_body_paragraph_surface(rendered_position.paragraph)
        {
            return Err(CommandError::Custom(
                "charts can only be inserted in body paragraphs".into(),
            ));
        }
        let paragraph = ctx
            .document
            .paragraph(rendered_position.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let model_len = paragraph.runs.iter().map(|run| run.text.len()).sum();
        let rendered_len = ctx
            .document
            .model_to_rendered_text_position(TextPosition::new(
                rendered_position.paragraph,
                model_len,
            ))
            .offset;
        if rendered_position.offset > rendered_len {
            return Err(CommandError::Custom(format!(
                "offset {} is past the end of paragraph {}, which is {rendered_len} bytes long",
                rendered_position.offset, rendered_position.paragraph
            )));
        }
        let model_position = ctx
            .document
            .rendered_to_model_text_position(rendered_position);
        let paragraph_text = ctx
            .document
            .paragraph(model_position.paragraph)
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            })
            .ok_or(CommandError::ParagraphUnavailable)?;
        if !paragraph_text.is_char_boundary(model_position.offset) {
            return Err(CommandError::Custom(format!(
                "offset {} is inside a character in paragraph {}",
                rendered_position.offset, rendered_position.paragraph
            )));
        }

        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let run_index = match ctx.document.insert_chart_at(
            model_position.paragraph,
            model_position.offset,
            &self.spec,
        ) {
            Ok(run_index) => run_index,
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                return Err(CommandError::Custom(error));
            }
        };
        self.snapshot = Some(snapshot);
        self.previous_selection = previous_selection;
        self.inserted_target = Some(FloatingObjectTarget {
            paragraph_index: model_position.paragraph,
            run_index,
            kind: FloatingObjectKind::Image,
        });
        *ctx.selection = Some(SelectionRange::collapsed(rendered_position));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("missing chart backup for undo".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        self.inserted_target = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{ChartKind, ChartSeries};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    fn escape_xml(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn build_docx(text: &str) -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
        const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
        const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;
        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p></w:body></w:document>"#,
            escape_xml(text)
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", ROOT_RELS),
                ("word/document.xml", document_xml.as_str()),
                ("word/_rels/document.xml.rels", DOC_RELS),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn zip_text(bytes: &[u8], path: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open DOCX");
        let mut value = String::new();
        archive
            .by_name(path)
            .expect("zip member")
            .read_to_string(&mut value)
            .expect("read zip member");
        value
    }

    fn zip_bytes(bytes: &[u8], path: &str) -> Vec<u8> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open DOCX");
        let mut value = Vec::new();
        archive
            .by_name(path)
            .expect("zip member")
            .read_to_end(&mut value)
            .expect("read zip member");
        value
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn generated_native_charts_preserve_coordinates_history_and_round_trip(
            host in "[a-zA-Z0-9ぁ-ん一-龥 ]{0,24}",
            title in "[a-zA-Z0-9ぁ-ん一-龥<& ]{1,16}",
            category_seed in prop::collection::vec("[a-zA-Z0-9ぁ-ん一-龥<& ]{1,8}", 1..6),
            series_seed in prop::collection::vec("[a-zA-Z0-9ぁ-ん一-龥<& ]{1,8}", 1..4),
            value_seed in prop::collection::vec(prop::option::of(-1000.0f64..1000.0), 5..20),
            kind_index in 0usize..3,
            caret_character in 0usize..32,
        ) {
            let kind = match kind_index {
                0 => ChartKind::Bar,
                1 => ChartKind::Line,
                _ => ChartKind::Pie,
            };
            let categories = category_seed;
            let series = series_seed
                .into_iter()
                .enumerate()
                .map(|(series_index, name)| ChartSeries {
                    name,
                    values: (0..categories.len())
                        .map(|category_index| {
                            value_seed[(series_index * 5 + category_index) % value_seed.len()]
                        })
                        .collect(),
                })
                .collect::<Vec<_>>();
            let character_count = host.chars().count();
            let character_index = caret_character.min(character_count);
            let caret = host
                .char_indices()
                .nth(character_index)
                .map_or(host.len(), |(offset, _)| offset);
            let expected_chart = crate::document::RunChart {
                kind,
                title,
                categories,
                series,
            };
            let source = build_docx(&host);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load DOCX");
            let before = document.export_docx().expect("baseline export");
            let selection = SelectionRange::collapsed(TextPosition::new(0, caret));
            let mut current_selection = Some(selection);
            let mut executor = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut current_selection);
                executor.execute(
                    Box::new(InsertChartCommand::new(
                        selection,
                        ChartInsertSpec {
                            kind: expected_chart.kind,
                            title: expected_chart.title.clone(),
                            categories: expected_chart.categories.clone(),
                            series: expected_chart.series.clone(),
                            width_px: 480.0,
                            height_px: 288.0,
                        },
                    )),
                    &mut context,
                ).expect("insert chart");
            }
            let paragraph = document.paragraph(0).expect("paragraph");
            prop_assert_eq!(paragraph.runs.iter().map(|run| run.text.as_str()).collect::<String>(), host.as_str());
            let live_chart = paragraph.runs.iter()
                .find_map(|run| run.image.as_ref()?.chart.as_ref())
                .expect("live chart");
            prop_assert_eq!(live_chart, &expected_chart);

            let exported = document.export_docx().expect("export chart");
            let document_xml = zip_text(&exported, "word/document.xml");
            let rels_xml = zip_text(&exported, "word/_rels/document.xml.rels");
            let chart_xml = zip_text(&exported, "word/charts/chart1.xml");
            let chart_rels_xml = zip_text(&exported, "word/charts/_rels/chart1.xml.rels");
            let workbook = zip_bytes(&exported, "word/charts/embeddings/jade-chart1.xlsx");
            let workbook_sheet = zip_text(&workbook, "xl/worksheets/sheet1.xml");
            prop_assert!(document_xml.contains("drawingml/2006/chart"));
            prop_assert!(rels_xml.contains("relationships/chart"));
            prop_assert!(chart_rels_xml.contains("relationships/package"));
            prop_assert!(chart_xml.contains("<c:externalData r:id=\"rId1\""));
            prop_assert!(workbook_sheet.contains("<sheetData>"));
            let expected_tag = match kind {
                ChartKind::Bar => "<c:barChart>",
                ChartKind::Line => "<c:lineChart>",
                ChartKind::Pie => "<c:pieChart>",
            };
            prop_assert!(chart_xml.contains(expected_tag));

            let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reopen chart DOCX");
            let reopened_chart = reopened.paragraph(0).expect("reopened paragraph").runs.iter()
                .find_map(|run| run.image.as_ref()?.chart.as_ref())
                .expect("reopened chart");
            prop_assert_eq!(reopened_chart, &expected_chart);

            let chart_run_index = document.paragraph(0).expect("chart paragraph").runs.iter()
                .position(|run| run.image.as_ref().is_some_and(|image| image.chart.is_some()))
                .expect("chart run index");
            let mut updated_chart = expected_chart.clone();
            updated_chart.title.push_str(" 更新");
            updated_chart.kind = match updated_chart.kind {
                ChartKind::Bar => ChartKind::Line,
                ChartKind::Line => ChartKind::Pie,
                ChartKind::Pie => ChartKind::Bar,
            };
            updated_chart.categories.reverse();
            for series in &mut updated_chart.series {
                series.values.reverse();
            }
            {
                let mut context = CommandContext::new(&mut document, &mut current_selection);
                executor.execute(
                    Box::new(crate::commands::update_chart::UpdateChartCommand::new(
                        crate::word_document::FloatingObjectTarget {
                            paragraph_index: 0,
                            run_index: chart_run_index,
                            kind: crate::word_document::FloatingObjectKind::Image,
                        },
                        updated_chart.clone(),
                    )),
                    &mut context,
                ).expect("update chart");
            }
            let updated_export = document.export_docx().expect("export updated chart");
            let updated_reopened = executor::block_on(WordDocument::load(&updated_export, &Options::default()))
                .expect("reopen updated chart DOCX");
            let updated_reopened_chart = updated_reopened.paragraph(0).expect("updated paragraph").runs.iter()
                .find_map(|run| run.image.as_ref()?.chart.as_ref())
                .expect("updated reopened chart");
            prop_assert_eq!(updated_reopened_chart, &updated_chart);

            {
                let mut context = CommandContext::new(&mut document, &mut current_selection);
                prop_assert!(executor.undo(&mut context).expect("undo chart update"));
            }
            let undo_update = document.export_docx().expect("export undo update");
            prop_assert_eq!(undo_update.as_slice(), exported.as_slice());
            {
                let mut context = CommandContext::new(&mut document, &mut current_selection);
                prop_assert!(executor.undo(&mut context).expect("undo chart insertion"));
            }
            let undo_insertion = document.export_docx().expect("export undo insertion");
            prop_assert_eq!(undo_insertion.as_slice(), before.as_slice());
            {
                let mut context = CommandContext::new(&mut document, &mut current_selection);
                prop_assert!(executor.redo(&mut context).expect("redo chart insertion"));
            }
            let redo_insertion = document.export_docx().expect("export redo insertion");
            prop_assert_eq!(redo_insertion.as_slice(), exported.as_slice());
            {
                let mut context = CommandContext::new(&mut document, &mut current_selection);
                prop_assert!(executor.redo(&mut context).expect("redo chart update"));
            }
            let redo_update = document.export_docx().expect("export redo update");
            prop_assert_eq!(redo_update.as_slice(), updated_export.as_slice());
        }
    }
}
