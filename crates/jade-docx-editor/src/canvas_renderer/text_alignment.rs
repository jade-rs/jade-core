use super::{Cow, InlineCluster, TextSegment};
use std::rc::Rc;

/// A run can switch font slots without changing its run properties. Give each
/// distinct vertical font box its own baseline, retaining adjacent glyphs with
/// equal boxes together so text shaping can still span them.
pub(super) fn segments_with_uniform_font_boxes<'a>(
    segments: &'a [TextSegment],
    clusters: &[InlineCluster],
) -> Cow<'a, [TextSegment]> {
    let mut result: Option<Vec<TextSegment>> = None;
    let mut cursor = 0;
    for (segment_index, segment) in segments.iter().enumerate() {
        while cursor < clusters.len() && clusters[cursor].end <= segment.start {
            cursor += 1;
        }
        let start = cursor;
        while cursor < clusters.len()
            && clusters[cursor].start < segment.end
            && clusters[cursor].end > segment.start
        {
            cursor += 1;
        }
        let parts = split_segment(segment, &clusters[start..cursor]);
        if let Some(parts) = parts {
            let output = result.get_or_insert_with(|| segments[..segment_index].to_vec());
            output.extend(parts);
        } else if let Some(output) = result.as_mut() {
            output.push(segment.clone());
        }
    }
    result.map_or(Cow::Borrowed(segments), Cow::Owned)
}

fn split_segment(segment: &TextSegment, clusters: &[InlineCluster]) -> Option<Vec<TextSegment>> {
    if segment.end.checked_sub(segment.start)? != segment.text.len() || clusters.len() < 2 {
        return None;
    }
    let font_box = |cluster: &InlineCluster| {
        if cluster.inline_object.is_some() {
            return None;
        }
        let layout = cluster.layout.as_ref()?;
        let (ascent, descent) = (layout.font_ascent?, layout.font_descent?);
        (ascent.is_finite() && descent.is_finite() && ascent > 0.0 && descent >= 0.0)
            .then_some((ascent, descent))
    };
    let first_box = font_box(clusters.first()?)?;
    if clusters
        .iter()
        .all(|cluster| font_box(cluster) == Some(first_box))
    {
        return None;
    }
    // The fast planner requires exact source coverage. Keep its normal fallback
    // for omitted source bytes, unknown metrics, or overlapping bidi ranges.
    let mut source_ranges = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        font_box(cluster)?;
        let local_start = cluster.start.checked_sub(segment.start)?;
        let local_end = cluster.end.checked_sub(segment.start)?;
        if local_start >= local_end
            || segment.text.get(local_start..local_end)? != cluster.text.as_str()
        {
            return None;
        }
        source_ranges.push((cluster.start, cluster.end));
    }
    source_ranges.sort_unstable();
    let mut source_end = segment.start;
    for (start, end) in source_ranges {
        if start != source_end {
            return None;
        }
        source_end = end;
    }
    if source_end != segment.end {
        return None;
    }

    let mut parts = Vec::new();
    let mut group_start = 0;
    while group_start < clusters.len() {
        let group_box = font_box(&clusters[group_start])?;
        let mut start = clusters[group_start].start;
        let mut end = clusters[group_start].end;
        let mut group_end = group_start + 1;
        while let Some(next) = clusters.get(group_end) {
            if font_box(next)? != group_box || (next.start != end && next.end != start) {
                break;
            }
            start = start.min(next.start);
            end = end.max(next.end);
            group_end += 1;
        }
        let mut properties = segment.properties.clone();
        if !parts.is_empty() {
            if let Some(properties) = properties.as_mut().filter(|p| p.ruby.is_some()) {
                // All pieces belong to the same ruby base. Only its original
                // head draws the annotation, across the full continued base.
                Rc::make_mut(properties).ruby.as_mut()?.continuation = true;
            }
        }
        parts.push(TextSegment {
            text: segment
                .text
                .get(start - segment.start..end - segment.start)?
                .to_owned(),
            start,
            end,
            properties,
            revision: segment.revision.clone(),
            hyperlink: segment.hyperlink.clone(),
            // A segment's extra advance precedes only its first visual part.
            leading_width: if parts.is_empty() {
                segment.leading_width
            } else {
                0.0
            },
        });
        group_start = group_end;
    }
    Some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::RunProperties;
    use jade_docx_layout::font_resolver::ResolvedFontSlot;
    use jade_docx_layout::line_wrap::{InlineClusterLayout, ScriptCategory};
    use std::rc::Rc;

    fn cluster(start: usize, text: &str, ascent: f64) -> InlineCluster {
        let mut cluster =
            InlineCluster::grapheme(start, start + text.len(), 10.0, text, false, false, false);
        cluster.layout = Some(InlineClusterLayout {
            character_border_edges: 0,
            script: ScriptCategory::Latin,
            font_slot: ResolvedFontSlot::LatinAscii,
            leading_advance: 0.0,
            glyph_box_width: 10.0,
            measured_glyph_width: 10.0,
            adjusted_glyph_width: 10.0,
            font_descriptor_hash: 0,
            font_ascent: Some(ascent),
            font_descent: Some(4.0),
        });
        cluster
    }

    fn segment() -> TextSegment {
        TextSegment {
            text: "A日bb".into(),
            start: 0,
            end: 6,
            properties: Some(Rc::new(RunProperties::default())),
            revision: None,
            hyperlink: None,
            leading_width: 3.0,
        }
    }

    #[test]
    fn text_alignment_splits_font_boxes_and_retains_adjacent_shaping_and_metadata() {
        let segments = [segment()];
        let clusters = [
            cluster(0, "A", 12.0),
            cluster(1, "日", 18.0),
            cluster(4, "b", 12.0),
            cluster(5, "b", 12.0),
        ];
        let split = segments_with_uniform_font_boxes(&segments, &clusters);
        assert!(matches!(split, Cow::Owned(_)));
        assert_eq!(
            split
                .iter()
                .map(|s| (s.text.as_str(), s.start, s.end, s.leading_width))
                .collect::<Vec<_>>(),
            vec![("A", 0, 1, 3.0), ("日", 1, 4, 0.0), ("bb", 4, 6, 0.0)]
        );
        for part in split.iter() {
            assert!(Rc::ptr_eq(
                part.properties.as_ref().unwrap(),
                segments[0].properties.as_ref().unwrap()
            ));
        }
    }

    #[test]
    fn text_alignment_split_keeps_one_ruby_head_without_mutating_source() {
        let mut original = segment();
        Rc::make_mut(original.properties.as_mut().unwrap()).ruby =
            Some(Box::new(crate::document::RubyAnnotation {
                text: "guide".into(),
                ..Default::default()
            }));
        let segments = [original];
        let clusters = [
            cluster(0, "A", 12.0),
            cluster(1, "日", 18.0),
            cluster(4, "b", 12.0),
            cluster(5, "b", 12.0),
        ];
        let split = segments_with_uniform_font_boxes(&segments, &clusters);
        assert_eq!(split.len(), 3);
        for (index, part) in split.iter().enumerate() {
            let ruby = part.properties.as_ref().unwrap().ruby.as_ref().unwrap();
            assert_eq!(ruby.continuation, index > 0);
            assert_eq!(ruby.text, "guide");
        }
        assert!(
            !segments[0]
                .properties
                .as_ref()
                .unwrap()
                .ruby
                .as_ref()
                .unwrap()
                .continuation
        );
    }

    #[test]
    fn text_alignment_preserves_visual_order_for_descending_source_ranges() {
        let segments = [segment()];
        let clusters = [
            cluster(5, "b", 12.0),
            cluster(4, "b", 12.0),
            cluster(1, "日", 18.0),
            cluster(0, "A", 12.0),
        ];
        let split = segments_with_uniform_font_boxes(&segments, &clusters);
        assert_eq!(
            split
                .iter()
                .map(|s| (s.text.as_str(), s.leading_width))
                .collect::<Vec<_>>(),
            vec![("bb", 3.0), ("日", 0.0), ("A", 0.0)]
        );
    }

    #[test]
    fn text_alignment_keeps_uniform_or_incomplete_cluster_plans_borrowed() {
        let segments = [segment()];
        let mut clusters = [
            cluster(0, "A", 12.0),
            cluster(1, "日", 12.0),
            cluster(4, "b", 12.0),
            cluster(5, "b", 12.0),
        ];
        assert!(matches!(
            segments_with_uniform_font_boxes(&segments, &clusters),
            Cow::Borrowed(_)
        ));
        clusters[1].layout.as_mut().unwrap().font_ascent = Some(18.0);
        assert!(matches!(
            segments_with_uniform_font_boxes(&segments, &clusters[..3]),
            Cow::Borrowed(_)
        ));
        clusters[3].layout = None;
        assert!(matches!(
            segments_with_uniform_font_boxes(&segments, &clusters),
            Cow::Borrowed(_)
        ));
    }
}
