//! Word's automatic hyphenation (`w:autoHyphenation`), applied to lines the
//! wrapper has already broken at word boundaries.
//!
//! Word hyphenates the word that starts a line when the line before it would
//! otherwise leave at least the hyphenation zone empty at its end: it keeps as
//! much of the word as fits there, at a point the hyphenation patterns allow,
//! and ends the line with a hyphen. The rest of the paragraph then flows up
//! a word at a time, and the next line is considered in turn. No more than
//! `w:consecutiveHyphenLimit` lines in a row end in a hyphen, and with
//! `w:doNotHyphenateCaps` a word in capitals is left whole.

use super::{
    classify_script, detach_leading_cluster, InlineCluster, InlineClusterKind, LineBreakKind,
    LineHyphen, LineWrapContext, ParagraphStyle, TextSegment, WrappedLine, WRAP_FIT_TOLERANCE_PX,
};

/// Hyphenates `lines` as `paragraph_style.hyphenation` asks, and reflows the
/// lines after each hyphen. Does nothing when hyphenation is off, or for
/// layouts this pass does not handle (text around floats, right-to-left).
pub(super) fn hyphenate_lines<C: LineWrapContext>(
    ctx: &C,
    lines: &mut Vec<WrappedLine>,
    paragraph_style: &ParagraphStyle,
    base_available: f64,
) -> Result<(), C::Error> {
    let Some(settings) = paragraph_style.hyphenation else {
        return Ok(());
    };
    if lines.len() < 2 || !paragraph_style.float_line_segments.is_empty() || paragraph_style.bidi {
        return Ok(());
    }
    let available = |index: usize| {
        paragraph_style
            .available_width_for_line(base_available, index)
            .max(1.0)
    };

    let mut index = 0;
    while index + 1 < lines.len() {
        let consecutive = lines[..index]
            .iter()
            .rev()
            .take_while(|line| line.hyphen.is_some())
            .count();
        let limited = settings
            .consecutive_limit
            .is_some_and(|limit| consecutive >= limit);
        if !limited
            && hyphenate_line_end(
                ctx,
                lines,
                index,
                available(index),
                settings.zone_px,
                settings.hyphenate_caps,
            )?
        {
            reflow_after(lines, index + 1, &available);
        }
        index += 1;
    }
    Ok(())
}

/// Hyphenates the word starting line `index + 1` onto the end of line
/// `index`, when Word would. Returns whether it did.
fn hyphenate_line_end<C: LineWrapContext>(
    ctx: &C,
    lines: &mut [WrappedLine],
    index: usize,
    available: f64,
    zone_px: f64,
    hyphenate_caps: bool,
) -> Result<bool, C::Error> {
    let (head, tail) = lines.split_at_mut(index + 1);
    let line = &mut head[index];
    let next = &mut tail[0];
    if line.break_kind != LineBreakKind::Soft
        || line.hyphen.is_some()
        || !line
            .clusters
            .last()
            .is_some_and(|cluster| cluster.is_whitespace)
    {
        return Ok(false);
    }
    // The word: the letters starting the next line, ending there.
    let letters = next
        .clusters
        .iter()
        .take_while(|cluster| {
            cluster.kind == InlineClusterKind::Grapheme && is_word_letter(&cluster.text)
        })
        .count();
    if letters == 0
        || (letters == next.clusters.len() && next.break_kind == LineBreakKind::Soft)
        || next.clusters.get(letters).is_some_and(|cluster| {
            // `don't`, `e-mail`, `x86`: not a plain word.
            matches!(cluster.text.as_str(), "'" | "’" | "-" | "_")
                || cluster.text.chars().all(|ch| ch.is_ascii_digit())
        })
    {
        return Ok(false);
    }
    let width =
        |clusters: &[InlineCluster]| clusters.iter().map(|cluster| cluster.width).sum::<f64>();
    let trailing_space: f64 = line
        .clusters
        .iter()
        .rev()
        .take_while(|cluster| cluster.is_whitespace)
        .map(|cluster| cluster.width)
        .sum();
    let used = width(&line.clusters);
    if available - (used - trailing_space) < zone_px {
        return Ok(false);
    }
    let word: String = next.clusters[..letters]
        .iter()
        .map(|cluster| cluster.text.as_str())
        .collect();
    if !hyphenate_caps && word.chars().all(|ch| ch.is_uppercase()) {
        return Ok(false);
    }
    let segment = segment_of(next, next.clusters[0].start);
    let properties = segment.and_then(|segment| segment.properties.clone());
    let language = properties
        .as_deref()
        .and_then(|properties| properties.language.as_deref());
    let points = crate::hyphenation::hyphenation_points(&word, language);
    if points.is_empty() {
        return Ok(false);
    }
    let style = ctx.resolve_run_style(properties.as_deref())?;
    let hyphen = ctx.measure_text_with_font_key(
        "-",
        &style.font_descriptor_latin_ascii,
        style.font_descriptor_latin_ascii_hash,
        style.font_descriptor_latin_ascii_id,
    )? * style.text_scale_x.max(0.01);
    let room = available - used;
    let Some(kept) = points
        .iter()
        .rev()
        .copied()
        .find(|&kept| width(&next.clusters[..kept]) + hyphen <= room + WRAP_FIT_TOLERANCE_PX)
    else {
        return Ok(false);
    };
    for _ in 0..kept {
        let Some((cluster, segment)) = detach_leading_cluster(next) else {
            return Ok(false);
        };
        append_preserving_rules(line, cluster, segment);
    }
    if let Some(last) = line.clusters.last_mut() {
        last.width += hyphen;
        if let Some(layout) = last.layout.as_mut() {
            layout.glyph_box_width += hyphen;
        }
    }
    line.hyphen = Some(LineHyphen { width: hyphen });
    Ok(true)
}

/// A letter of a word the patterns can hyphenate: any Latin-script letter,
/// accented ones included (`ä`, `é`, `ñ`, `ĳ`).
fn is_word_letter(grapheme: &str) -> bool {
    classify_script(grapheme) == super::ScriptCategory::Latin
        && grapheme.chars().all(char::is_alphabetic)
}

/// The segment of `line` holding the byte at `offset`.
fn segment_of(line: &WrappedLine, offset: usize) -> Option<&TextSegment> {
    line.segments
        .iter()
        .find(|segment| segment.start <= offset && offset < segment.end)
}

/// Moves a cluster to the end of `line`, keeping its line-start and line-end
/// rules for the passes that follow.
fn append_preserving_rules(
    line: &mut WrappedLine,
    cluster: InlineCluster,
    mut segment: TextSegment,
) {
    line.clusters.push(cluster);
    segment.leading_width = 0.0;
    if let Some(last) = line.segments.last_mut() {
        if last.properties == segment.properties
            && last.revision == segment.revision
            && last.hyperlink == segment.hyperlink
            && last.end == segment.start
        {
            last.text.push_str(&segment.text);
            last.end = segment.end;
            return;
        }
    }
    line.segments.push(segment);
}

/// After line `from` lost the start of its first word to the line before,
/// lets every following line take up what now fits from the line after it,
/// a word (or an East Asian character with what may not be parted from it)
/// at a time. Empty lines left behind are dropped.
fn reflow_after(lines: &mut Vec<WrappedLine>, from: usize, available: &dyn Fn(usize) -> f64) {
    let mut index = from;
    while index + 1 < lines.len() {
        if lines[index].break_kind != LineBreakKind::Soft {
            index += 1;
            continue;
        }
        loop {
            if index + 1 >= lines.len() {
                break;
            }
            let (head, tail) = lines.split_at_mut(index + 1);
            let line = &mut head[index];
            let next = &mut tail[0];
            // A tab's width depends on what precedes it on its line.
            if next
                .clusters
                .iter()
                .any(|cluster| cluster.kind != InlineClusterKind::Grapheme)
            {
                break;
            }
            let unit = leading_unit(&next.clusters);
            if unit == 0 {
                break;
            }
            let trailing_space: f64 = next.clusters[..unit]
                .iter()
                .rev()
                .take_while(|cluster| cluster.is_whitespace)
                .map(|cluster| cluster.width)
                .sum();
            let used: f64 = line.clusters.iter().map(|cluster| cluster.width).sum();
            let adding: f64 = next.clusters[..unit]
                .iter()
                .map(|cluster| cluster.width)
                .sum();
            if used + adding - trailing_space > available(index) + WRAP_FIT_TOLERANCE_PX {
                break;
            }
            for _ in 0..unit {
                let Some((cluster, segment)) = detach_leading_cluster(next) else {
                    return;
                };
                append_preserving_rules(line, cluster, segment);
            }
            if next.clusters.is_empty() {
                line.break_kind = next.break_kind;
                line.break_range = next.break_range;
                line.break_properties = next.break_properties.clone();
                line.leaders.append(&mut next.leaders);
                line.tab_stops.append(&mut next.tab_stops);
                lines.remove(index + 1);
                break;
            }
        }
        index += 1;
    }
}

/// How many clusters at the start of a line move up together: a Latin word
/// and the spaces after it, or one other character with any that may not
/// start a line after it.
fn leading_unit(clusters: &[InlineCluster]) -> usize {
    let mut end = clusters
        .iter()
        .take_while(|cluster| !cluster.is_whitespace && is_word_letter(&cluster.text))
        .count();
    if end == 0 {
        if clusters.first().is_none_or(|cluster| cluster.is_whitespace) {
            return 0;
        }
        end = 1;
    }
    // A word's own punctuation (`end.`, `it,`) and what may not start a line.
    while let Some(cluster) = clusters.get(end) {
        let attached = !cluster.is_whitespace
            && (cluster.forbid_line_start
                || (cluster.text.chars().all(|ch| ch.is_ascii_punctuation())
                    && clusters[end - 1].text.chars().all(char::is_alphanumeric)));
        if !attached {
            break;
        }
        end += 1;
    }
    while clusters
        .get(end)
        .is_some_and(|cluster| cluster.is_whitespace)
    {
        end += 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::super::{wrap_text_into_lines, ParagraphHyphenation, RunStyle};
    use super::*;
    use crate::font_resolver::ResolvedRunStyle;
    use jade_docx_reader::RunProperties;
    use std::rc::Rc;
    use unicode_segmentation::UnicodeSegmentation;

    /// Every grapheme 10px wide.
    struct Fixed(Rc<ResolvedRunStyle>);

    impl Fixed {
        fn new() -> Self {
            Self(Rc::new(ResolvedRunStyle::new(
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
    }

    impl LineWrapContext for Fixed {
        type Error = ();
        fn resolve_run_style(
            &self,
            _properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, ()> {
            Ok(Rc::clone(&self.0))
        }
        fn measure_text(&self, text: &str, _font: &str) -> Result<f64, ()> {
            Ok(text.graphemes(true).count() as f64 * 10.0)
        }
    }

    fn lines(text: &str, width: f64, hyphenation: Option<ParagraphHyphenation>) -> Vec<String> {
        let style = ParagraphStyle {
            hyphenation,
            ..ParagraphStyle::default()
        };
        let run = RunStyle {
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        };
        wrap_text_into_lines(&Fixed::new(), text, &[run], 0.0, &style, 0, width)
            .unwrap()
            .iter()
            .map(|line| {
                let mut text: String = line
                    .segments
                    .iter()
                    .map(|segment| segment.text.as_str())
                    .collect();
                if line.hyphen.is_some() {
                    text.push('-');
                }
                text
            })
            .collect()
    }

    fn on(zone_px: f64) -> Option<ParagraphHyphenation> {
        Some(ParagraphHyphenation {
            zone_px,
            consecutive_limit: None,
            hyphenate_caps: true,
        })
    }

    #[test]
    fn the_word_after_a_short_line_is_hyphenated_and_the_rest_flows_up() {
        let text = "The quick hyphenation test text goes here";
        assert_eq!(
            lines(text, 150.0, None),
            ["The quick ", "hyphenation ", "test text goes ", "here"]
        );
        assert_eq!(
            lines(text, 150.0, on(20.0)),
            ["The quick hy-", "phenation test ", "text goes here"]
        );
    }

    #[test]
    fn a_line_within_the_zone_is_left_alone() {
        let text = "The quick hyphenation test text goes here";
        // 60px are left at the end of the first line: a wider zone keeps it.
        assert_eq!(lines(text, 150.0, on(70.0))[0], "The quick ");
    }

    #[test]
    fn the_hyphen_counts_toward_the_line_width() {
        let text = "The quick hyphenation test text goes here";
        let style = ParagraphStyle {
            hyphenation: on(20.0),
            ..ParagraphStyle::default()
        };
        let run = RunStyle {
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        };
        let wrapped =
            wrap_text_into_lines(&Fixed::new(), text, &[run], 0.0, &style, 0, 150.0).unwrap();
        let first: f64 = wrapped[0]
            .clusters
            .iter()
            .map(|cluster| cluster.width)
            .sum();
        assert_eq!(wrapped[0].hyphen, Some(LineHyphen { width: 10.0 }));
        assert_eq!(first, 130.0);
    }

    #[test]
    fn consecutive_hyphens_are_limited() {
        let text = "ab hyphenation hyphenation hyphenation hyphenation";
        let unlimited = lines(text, 130.0, on(10.0));
        let hyphens = |lines: &[String]| {
            lines
                .windows(2)
                .take_while(|pair| pair[0].ends_with('-'))
                .count()
        };
        let limited = lines(
            text,
            130.0,
            Some(ParagraphHyphenation {
                zone_px: 10.0,
                consecutive_limit: Some(1),
                hyphenate_caps: true,
            }),
        );
        assert_eq!(
            unlimited,
            [
                "ab hyphen-",
                "ation hyphen-",
                "ation hyphen-",
                "ation hyphen-",
                "ation"
            ]
        );
        assert!(hyphens(&unlimited) >= 2, "{unlimited:?}");
        assert_eq!(hyphens(&limited), 1, "{limited:?}");
        assert_eq!(limited[1], "ation ");
    }

    #[test]
    fn capitals_can_be_left_whole() {
        let text = "The quick HYPHENATION test";
        assert_eq!(lines(text, 150.0, on(20.0))[0], "The quick HY-");
        let caps_off = Some(ParagraphHyphenation {
            zone_px: 20.0,
            consecutive_limit: None,
            hyphenate_caps: false,
        });
        assert_eq!(lines(text, 150.0, caps_off)[0], "The quick ");
    }
}
