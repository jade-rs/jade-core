//! Carry source control identity into derived Windows layout slices, keeping
//! authored Unicode text, UTF-8 offsets and run formatting unchanged.
use crate::line_metrics::WordLayoutPlatform;
use crate::line_wrap::RunStyle;
use jade_docx_reader::Run;
use std::rc::Rc;

#[inline]
pub fn mark_literal_soft_hyphens(
    run: &Run,
    rendered_text: &str,
    start: usize,
    first_style: usize,
    styles: &mut Vec<RunStyle>,
    platform: WordLayoutPlatform,
) {
    if run.text_controls.is_none() || platform != WordLayoutPlatform::Windows {
        return;
    }
    let source: Vec<_> = run
        .text
        .match_indices('\u{ad}')
        .map(|(offset, _)| run.is_literal_soft_hyphen(offset))
        .collect();
    let rendered: Vec<_> = rendered_text
        .match_indices('\u{ad}')
        .map(|(offset, _)| offset)
        .collect();
    // Case conversion may change UTF-8 lengths before a control. Its ordinal
    // identity survives; a computed field with a different set of controls is
    // not the same source text and cannot borrow the source annotations.
    if source.len() != rendered.len() {
        return;
    }
    let literals: Vec<_> = source
        .into_iter()
        .zip(rendered)
        .filter_map(|(literal, offset)| literal.then_some(start + offset))
        .collect();
    if literals.is_empty() {
        return;
    }
    for original in styles.split_off(first_style) {
        if original.start == original.end {
            styles.push(original);
            continue;
        }
        let mut cursor = original.start;
        for &literal in literals.iter().filter(|&&offset| {
            offset >= original.start && offset + '\u{ad}'.len_utf8() <= original.end
        }) {
            if cursor < literal {
                let mut prefix = original.clone();
                prefix.start = cursor;
                prefix.end = literal;
                styles.push(prefix);
            }
            let mut marker = original.clone();
            marker.start = literal;
            marker.end = literal + '\u{ad}'.len_utf8();
            Rc::make_mut(marker.properties.get_or_insert_with(Rc::default)).literal_soft_hyphen =
                true;
            cursor = marker.end;
            styles.push(marker);
        }
        if cursor < original.end {
            let mut suffix = original;
            suffix.start = cursor;
            styles.push(suffix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_resolver::ResolvedRunStyle;
    use crate::line_wrap::{wrap_text_into_lines, LineWrapContext, ParagraphStyle};
    use jade_docx_reader::{BodyElement, DocumentParser, RunProperties};
    use unicode_segmentation::UnicodeSegmentation;

    struct Fixed;
    impl LineWrapContext for Fixed {
        type Error = ();
        fn resolve_run_style(&self, _: Option<&RunProperties>) -> Result<Rc<ResolvedRunStyle>, ()> {
            Ok(Rc::new(ResolvedRunStyle::new(
                "12pt Test",
                "12pt Test",
                "12pt Test",
                "12pt Test",
                0.0,
                1.0,
                false,
                None,
            )))
        }
        fn measure_text(&self, text: &str, _: &str) -> Result<f64, ()> {
            Ok(text.graphemes(true).filter(|&g| g != "\u{ad}").count() as f64 * 10.0)
        }
    }

    fn frames(text: &str, styles: &[RunStyle], width: f64) -> Vec<(String, Vec<f64>, bool)> {
        frames_for_platform(text, styles, width, WordLayoutPlatform::Windows)
    }

    fn frames_for_platform(
        text: &str,
        styles: &[RunStyle],
        width: f64,
        platform: WordLayoutPlatform,
    ) -> Vec<(String, Vec<f64>, bool)> {
        let paragraph = ParagraphStyle {
            word_platform: platform,
            ..Default::default()
        };
        wrap_text_into_lines(&Fixed, text, styles, 0.0, &paragraph, 0, width)
            .unwrap()
            .into_iter()
            .map(|line| {
                (
                    line.segments
                        .iter()
                        .map(|s| s.text.as_str())
                        .collect::<String>()
                        .replace('\u{ad}', "\u{2011}"),
                    line.clusters.iter().map(|c| c.width).collect(),
                    line.hyphen.is_some(),
                )
            })
            .collect()
    }

    fn run() -> Run {
        let xml="<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>ı\u{ad}a</w:t><w:softHyphen/><w:t>b\u{ad}c</w:t></w:r></w:p></w:body></w:document>";
        let doc = DocumentParser::new().parse_document_xml(xml).unwrap();
        let body = doc.body.unwrap();
        let BodyElement::Paragraph(paragraph) = &body.elements[0] else {
            panic!("paragraph")
        };
        paragraph.runs[0].clone()
    }

    fn style(start: usize, end: usize) -> RunStyle {
        RunStyle {
            start,
            end,
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        }
    }

    #[test]
    fn windows_literal_hyphen_word_moves_at_preceding_space_without_changing_mac() {
        let xml = "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>netic re\u{ad}enter</w:t></w:r></w:p></w:body></w:document>";
        let doc = DocumentParser::new().parse_document_xml(xml).unwrap();
        let body = doc.body.unwrap();
        let paragraph = body.elements[0].as_paragraph().unwrap();
        let run = &paragraph.runs[0];
        let text = run.text.as_str();
        let original = vec![style(0, text.len())];
        let mut windows = original.clone();
        mark_literal_soft_hyphens(run, text, 0, 0, &mut windows, WordLayoutPlatform::Windows);
        let actual = frames(text, &windows, 90.0);
        assert_eq!(
            actual
                .iter()
                .map(|line| line.0.as_str())
                .collect::<Vec<_>>(),
            ["netic ", "re\u{2011}enter"]
        );
        assert!(actual.iter().all(|line| !line.2));
        let optional = frames(text, &original, 90.0);
        assert_ne!(actual, optional);
        assert!(optional.iter().any(|line| line.2));
        let mut mac = original.clone();
        mark_literal_soft_hyphens(run, text, 0, 0, &mut mac, WordLayoutPlatform::Mac);
        assert_eq!(mac.len(), original.len());
        assert_eq!(mac[0].start, original[0].start);
        assert_eq!(mac[0].end, original[0].end);
        assert!(mac[0].properties.is_none());
        assert_eq!(
            frames_for_platform(text, &mac, 90.0, WordLayoutPlatform::Mac),
            frames_for_platform(text, &original, 90.0, WordLayoutPlatform::Mac)
        );
        assert_eq!(text, "netic re\u{ad}enter");
    }

    #[test]
    fn windows_marks_only_literal_control_slices_after_utf8_case_expansion() {
        let source = run();
        let source_properties = source.properties.clone();
        let rendered = "I\u{ad}A\u{ad}B\u{ad}C";
        let mut styles = vec![style(0, rendered.len())];
        mark_literal_soft_hyphens(
            &source,
            rendered,
            0,
            0,
            &mut styles,
            WordLayoutPlatform::Windows,
        );
        let flagged: Vec<_> = styles
            .iter()
            .filter(|s| s.properties.as_ref().is_some_and(|p| p.literal_soft_hyphen))
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(flagged, [(1, 3), (7, 9)]);
        assert_eq!(
            styles
                .iter()
                .map(|s| &rendered[s.start..s.end])
                .collect::<String>(),
            rendered
        );
        assert_eq!(source.properties, source_properties);
        assert!(!source
            .properties
            .as_ref()
            .is_some_and(|p| p.literal_soft_hyphen));
        assert_eq!(source.text, "ı\u{ad}a\u{ad}b\u{ad}c");
    }

    #[test]
    fn mac_styles_keep_their_original_ranges_and_properties() {
        let source = run();
        let mut styles = vec![style(20, 20 + source.text.len())];
        mark_literal_soft_hyphens(
            &source,
            &source.text,
            20,
            0,
            &mut styles,
            WordLayoutPlatform::Mac,
        );
        assert_eq!(styles.len(), 1);
        assert_eq!(styles[0].start, 20);
        assert_eq!(styles[0].end, 20 + source.text.len());
        assert!(styles[0].properties.is_none());
    }

    #[test]
    fn changed_field_controls_do_not_acquire_unrelated_source_identity() {
        let source = run();
        let rendered = "result\u{ad}";
        let mut styles = vec![style(0, rendered.len())];
        mark_literal_soft_hyphens(
            &source,
            rendered,
            0,
            0,
            &mut styles,
            WordLayoutPlatform::Windows,
        );
        assert_eq!(styles.len(), 1);
        assert!(styles[0].properties.is_none());
    }

    #[test]
    fn literal_metrics_and_wrap_match_visible_nonbreaking_hyphens_without_optional_ink() {
        let xml="<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>ab\u{ad}cdef</w:t></w:r></w:p></w:body></w:document>";
        let doc = DocumentParser::new().parse_document_xml(xml).unwrap();
        let body = doc.body.unwrap();
        let BodyElement::Paragraph(paragraph) = &body.elements[0] else {
            panic!("paragraph")
        };
        let run = &paragraph.runs[0];
        let mut styles = vec![style(0, run.text.len())];
        mark_literal_soft_hyphens(
            run,
            &run.text,
            0,
            0,
            &mut styles,
            WordLayoutPlatform::Windows,
        );
        let visible = "ab\u{2011}cdef";
        for width in [40.0, 80.0] {
            let actual = frames(&run.text, &styles, width);
            assert_eq!(actual, frames(visible, &[style(0, visible.len())], width));
            assert!(actual.iter().all(|line| !line.2));
        }
        assert!(frames(&run.text, &[style(0, run.text.len())], 40.0)
            .iter()
            .any(|line| line.2));
        assert_eq!(run.text, "ab\u{ad}cdef");
    }
}
