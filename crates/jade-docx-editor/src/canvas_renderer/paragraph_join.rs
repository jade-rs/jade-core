//! Paragraphs joined by a hidden paragraph mark.
//!
//! A tracked deletion of a paragraph mark (`w:pPr/w:rPr/w:del`, ECMA-376 Part
//! 1 §17.13.5.15) removes the break between the paragraph and the one after
//! it once the deletion is applied. Word's final view (No Markup) therefore
//! draws both as one paragraph: `numbering-del2` (Word for Mac 16.114) draws
//! `Hello`, whose mark is deleted, two paragraphs deleted with their marks,
//! and `World` with its `W` deleted as the single numbered line
//! `１.Helloorld`. The original view hides an inserted mark the same way.
//!
//! The joined paragraphs are laid out as one paragraph under the first one's
//! surface index. The joined paragraph takes the paragraph properties of the
//! first member that shows any text (Word keeps the first paragraph's
//! formatting when two are joined; a paragraph deleted with its mark is gone
//! entirely) and the mark properties of the last member, whose mark remains.
//! Each member keeps its own selection registration (see
//! `SelectionManager::split_joined_paragraph`) and layout cache entry, and an
//! edit of any member lays the whole group out again from its first member.
//! The markup view shows every mark and never joins.

use super::*;

/// Body paragraphs drawn as one, planned before the first is rendered.
pub(super) struct ParagraphJoin {
    /// Body element index of the first member.
    first_element: usize,
    merged: Paragraph,
    /// Index in `merged.runs` of the first run of each member after the
    /// first.
    member_runs: Vec<usize>,
    /// Content hash of each member, as `find_first_changed_paragraph` reads
    /// the layout cache.
    member_hashes: Vec<ContentHash>,
}

/// The joined paragraph being rendered.
pub(super) struct JoinedRender {
    member_runs: Vec<usize>,
    /// Layout text offset where each member after the first starts.
    member_offsets: Vec<Option<usize>>,
    /// Content hash of the first member, keyed by the layout cache.
    pub(super) head_hash: ContentHash,
}

impl JoinedRender {
    /// Called for every run of the joined paragraph as its layout text is
    /// built.
    pub(super) fn record_run_start(&mut self, run_index: usize, offset: usize) {
        for (member, &first_run) in self.member_runs.iter().enumerate() {
            if first_run == run_index && self.member_offsets[member].is_none() {
                self.member_offsets[member] = Some(offset);
            }
        }
    }
}

/// Whether the view applies the tracked change on the paragraph mark, which
/// then no longer ends the paragraph. Mirrors `run_is_hidden_revision`.
fn paragraph_mark_hidden(paragraph: &Paragraph, options: &Options) -> bool {
    if options.render_changes {
        return false;
    }
    match paragraph
        .properties
        .as_ref()
        .and_then(|props| props.mark_revision.as_ref())
        .map(|mark| &mark.kind)
    {
        Some(RevisionKind::Deleted | RevisionKind::MovedFrom) => !options.render_original,
        Some(RevisionKind::Inserted | RevisionKind::MovedTo) => options.render_original,
        None => false,
    }
}

/// Content the joined layout does not carry over: objects and notes keep
/// per-paragraph state keyed by run index or by surface, frames and drop caps
/// lay a paragraph out on their own.
fn joinable(paragraph: &Paragraph) -> bool {
    !paragraph
        .properties
        .as_ref()
        .is_some_and(|props| props.frame.is_some() || props.drop_cap.is_some())
        && paragraph.runs.iter().all(|run| {
            run.image.is_none()
                && run.text_box.is_none()
                && run.math.is_none()
                && run.note_separator.is_none()
                && !run.has_footnote_ref
                && !run.has_endnote_ref
                && run.footnote_reference_ids.is_empty()
                && run.endnote_reference_ids.is_empty()
        })
}

fn shows_text(paragraph: &Paragraph, options: &Options) -> bool {
    paragraph.runs.iter().any(|run| {
        !run.text.is_empty()
            && !run_is_hidden_revision(run, options.render_changes, options.render_original)
    })
}

impl<P: Painter> CanvasRenderer<P>
where
    P::Error: PainterError,
{
    /// Plans the join that starts at body element `index` of `elements`
    /// (whose first element is body element `element_base`), or notes that
    /// the element belongs to a join planned earlier. Called for every body
    /// element before it is rendered.
    #[inline]
    pub(super) fn prepare_paragraph_join(
        &mut self,
        elements: &[BodyElement],
        index: usize,
        section_end: usize,
        element_base: usize,
        options: &Options,
    ) {
        // Almost every paragraph mark carries no revision.
        if self.paragraph_join_last.is_none()
            && elements
                .get(index)
                .and_then(BodyElement::as_paragraph)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .is_none_or(|props| props.mark_revision.is_none())
        {
            return;
        }
        self.plan_paragraph_join(elements, index, section_end, element_base, options);
    }

    fn plan_paragraph_join(
        &mut self,
        elements: &[BodyElement],
        index: usize,
        section_end: usize,
        element_base: usize,
        options: &Options,
    ) {
        let element = element_base + index;
        if self.paragraph_join_last.is_some_and(|last| element <= last) {
            return;
        }
        self.paragraph_join = None;
        self.paragraph_join_last = None;
        if self.rendering_header_footer_story
            || self.rendering_note_story
            || self.rendering_page_furniture
        {
            return;
        }
        let Some(first) = elements.get(index).and_then(BodyElement::as_paragraph) else {
            return;
        };
        if !paragraph_mark_hidden(first, options) {
            return;
        }
        let mut members = vec![first];
        let mut last = index;
        while paragraph_mark_hidden(members[members.len() - 1], options) {
            last += 1;
            // A hidden mark before a table or the section end has nothing to
            // join.
            let Some(next) = (last < section_end)
                .then(|| elements.get(last).and_then(BodyElement::as_paragraph))
                .flatten()
            else {
                return;
            };
            members.push(next);
        }
        if !members.iter().all(|member| joinable(member)) {
            return;
        }
        let formatting = members
            .iter()
            .find(|member| shows_text(member, options))
            .or(members.last())
            .copied()
            .unwrap_or(first);
        let mut properties = formatting.properties.clone();
        if let Some(props) = properties.as_mut() {
            props.mark_revision = None;
            props.mark_run_properties = members
                .last()
                .and_then(|member| member.properties.as_ref())
                .and_then(|props| props.mark_run_properties.clone());
        }
        let mut merged = Paragraph {
            runs: Vec::new(),
            properties,
            node_id: first.node_id,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let mut member_runs = Vec::with_capacity(members.len() - 1);
        let mut text_offset = 0;
        for (position, member) in members.iter().enumerate() {
            if position > 0 {
                member_runs.push(merged.runs.len());
            }
            merged
                .comment_ranges
                .extend(member.comment_ranges.iter().map(|range| CommentRange {
                    id: range.id.clone(),
                    start: range.start + text_offset,
                    end: range.end + text_offset,
                }));
            merged
                .comment_markers
                .extend(member.comment_markers.iter().map(|marker| CommentMarker {
                    id: marker.id.clone(),
                    position: marker.position + text_offset,
                }));
            text_offset += member.runs.iter().map(|run| run.text.len()).sum::<usize>();
            merged.runs.extend(member.runs.iter().cloned());
        }
        let member_hashes = members
            .iter()
            .map(|member| self.paragraph_content_hash_cached(member))
            .collect();
        self.paragraph_join_last = Some(element_base + last);
        self.paragraph_join = Some(ParagraphJoin {
            first_element: element,
            merged,
            member_runs,
            member_hashes,
        });
    }

    /// Renders the join that starts at `body_element_index`, or skips a
    /// later member that its first member has already drawn. `None` when the
    /// paragraph renders on its own.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_paragraph_join_member(
        &mut self,
        options: &Options,
        numbering_part: Option<&NumberingPart>,
        numbering_counter: &mut NumberingCounter,
        deleted_numbering_offsets: &mut HashMap<String, Vec<u32>>,
        paragraph_index: usize,
        body_element_index: usize,
        section_index: usize,
    ) -> Option<Result<(), JsValue>> {
        let head = self
            .paragraph_join
            .as_ref()
            .is_some_and(|join| join.first_element == body_element_index);
        if !head {
            if self
                .paragraph_join_last
                .is_some_and(|last| body_element_index <= last)
            {
                return Some(Ok(()));
            }
            self.forget_paragraph_join(paragraph_index);
            return None;
        }
        let join = self.paragraph_join.take()?;
        let members = join.member_hashes.len();
        self.joined_render = Some(JoinedRender {
            member_offsets: vec![None; join.member_runs.len()],
            member_runs: join.member_runs,
            head_hash: join.member_hashes[0],
        });
        let result = self.render_paragraph_with_numbering(
            &join.merged,
            options,
            numbering_part,
            numbering_counter,
            deleted_numbering_offsets,
            paragraph_index,
            body_element_index,
            section_index,
        );
        let joined = self.joined_render.take();
        if let Err(error) = result {
            return Some(Err(error));
        }
        let joined = joined?;
        let text_len = self
            .selection
            .paragraphs()
            .get(paragraph_index)
            .map_or(0, |layout| layout.text.len());
        let pieces: Vec<(usize, usize)> = joined
            .member_offsets
            .iter()
            .enumerate()
            .map(|(member, offset)| (paragraph_index + 1 + member, offset.unwrap_or(text_len)))
            .collect();
        self.selection
            .split_joined_paragraph(paragraph_index, &pieces);
        if self.incremental_layout_enabled && !self.skip_layout_cache_update {
            // Later members take no space of their own: they end where the
            // joined paragraph ends, so a cache walk lands on the next
            // paragraph after the group.
            let head_entry = self
                .paragraph_layout_cache
                .borrow()
                .get_entry(paragraph_index)
                .filter(|entry| entry.content_hash == joined.head_hash)
                .cloned();
            if let Some(head_entry) = head_entry {
                let mut cache = self.paragraph_layout_cache.borrow_mut();
                for (member, hash) in join.member_hashes.iter().enumerate().skip(1) {
                    let mut entry = head_entry.clone();
                    entry.content_hash = *hash;
                    entry.start_y = entry.end_y;
                    entry.start_page = entry.end_page;
                    entry.start_page_offset_y = entry.end_page_offset_y;
                    entry.start_column = entry.end_column;
                    entry.start_last_line_baseline = entry.end_last_line_baseline;
                    entry.height = 0.0;
                    cache.insert(paragraph_index + member, entry);
                }
            }
        }
        if !self.measuring_keep_next {
            self.paragraph_join_surfaces
                .insert(paragraph_index, paragraph_index + members - 1);
        }
        Some(Ok(()))
    }

    /// A paragraph that renders on its own no longer leads a join it led in
    /// an earlier pass: its former members are selectable again.
    fn forget_paragraph_join(&mut self, paragraph_index: usize) {
        if let Some(last) = self.paragraph_join_surfaces.remove(&paragraph_index) {
            for member in paragraph_index..=last {
                self.selection.set_paragraph_selectable(member, true);
            }
        }
    }

    /// The first surface of the join holding `paragraph`, from the last
    /// render: an edit of any member lays the group out again from there.
    pub(super) fn paragraph_join_owner(&self, paragraph: usize) -> usize {
        self.paragraph_join_surfaces
            .range(..=paragraph)
            .next_back()
            .filter(|(_, last)| paragraph <= **last)
            .map_or(paragraph, |(first, _)| *first)
    }

    /// The surfaces drawn together with `paragraph`.
    pub(super) fn paragraph_join_group(&self, paragraph: usize) -> std::ops::RangeInclusive<usize> {
        let first = self.paragraph_join_owner(paragraph);
        let last = self
            .paragraph_join_surfaces
            .get(&first)
            .copied()
            .unwrap_or(paragraph);
        first..=last
    }

    /// The first surface whose layout an edit of `paragraph` invalidates:
    /// joined paragraphs and `w:keepNext` groups are laid out together.
    pub(super) fn layout_group_owner(&self, paragraph: usize) -> usize {
        let owner = self.keep_next_group_owner(self.paragraph_join_owner(paragraph));
        self.paragraph_join_owner(owner)
    }
}

#[cfg(test)]
mod tests {
    use super::super::recording_host::recording_renderer;
    use super::super::section_tests::build_docx_with_body;
    use super::*;
    use crate::word_document::WordDocument;
    use jade_docx_layout::selection::TextPosition;
    use jade_painter::recording::RecordingPainter;

    const DELETED_MARK: &str = r#"<w:pPr><w:rPr><w:del w:id="1" w:author="A"/></w:rPr></w:pPr>"#;

    /// `numbering-del2` without the list: `Hello` with its mark deleted, two
    /// paragraphs deleted with their marks, `World` with its `W` deleted.
    fn joined_body() -> String {
        let deleted = |text: &str| {
            format!(
                r#"<w:p>{DELETED_MARK}<w:del w:id="2" w:author="A"><w:r><w:delText>{text}</w:delText></w:r></w:del></w:p>"#
            )
        };
        format!(
            r#"<w:p>{DELETED_MARK}<w:r><w:t>Hello</w:t></w:r></w:p>{}{}<w:p><w:del w:id="3" w:author="A"><w:r><w:delText>W</w:delText></w:r></w:del><w:r><w:t>orld</w:t></w:r></w:p><w:p><w:r><w:t>Next</w:t></w:r></w:p>"#,
            deleted("Foo"),
            deleted("Bar"),
        )
    }

    fn load(body: &str) -> WordDocument {
        let data = build_docx_with_body(body);
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load")
    }

    fn options(render_changes: bool) -> Options {
        Options {
            render_changes,
            ..Options::default()
        }
    }

    fn render(renderer: &mut CanvasRenderer<RecordingPainter>, doc: &WordDocument, changes: bool) {
        let document = doc.document.as_ref().expect("document");
        renderer
            .render(document, &options(changes))
            .expect("render");
    }

    /// `(start, end, left, right)` of a fragment.
    type Fragment = (usize, usize, i64, i64);
    /// A line's top and fragments.
    type Line = (i64, Vec<Fragment>);

    /// Text, line tops and fragment extents of every paragraph surface.
    fn registrations(renderer: &CanvasRenderer<RecordingPainter>) -> Vec<(String, Vec<Line>)> {
        renderer
            .selection
            .paragraphs()
            .iter()
            .map(|paragraph| {
                let lines = paragraph
                    .lines
                    .iter()
                    .map(|line| {
                        let fragments = line
                            .fragments
                            .iter()
                            .map(|fragment| {
                                (
                                    fragment.start,
                                    fragment.end,
                                    fragment.x.round() as i64,
                                    (fragment.x + fragment.width).round() as i64,
                                )
                            })
                            .collect();
                        (line.absolute_top.round() as i64, fragments)
                    })
                    .collect();
                (paragraph.text.clone(), lines)
            })
            .collect()
    }

    #[test]
    fn final_view_draws_a_deleted_paragraph_mark_joined_with_the_next_paragraph() {
        let doc = load(&joined_body());
        let mut renderer = recording_renderer();
        render(&mut renderer, &doc, false);
        let paragraphs = registrations(&renderer);
        let texts: Vec<&str> = paragraphs.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(texts, ["Hello", "", "", "orld", "Next"]);
        let (_, hello) = &paragraphs[0];
        let (_, world) = &paragraphs[3];
        let (_, next) = &paragraphs[4];
        assert_eq!(hello.len(), 1, "{paragraphs:?}");
        assert_eq!(world.len(), 1, "{paragraphs:?}");
        assert_eq!(hello[0].0, world[0].0, "one line: {paragraphs:?}");
        assert!(next[0].0 > hello[0].0, "{paragraphs:?}");
        // 10px a character: `orld` continues right after `Hello`, in its own
        // offsets.
        let hello_end = hello[0].1.last().expect("Hello fragment").3;
        let world_fragment = world[0].1.first().expect("orld fragment");
        assert_eq!((world_fragment.0, world_fragment.2), (0, hello_end));
        assert_eq!(world[0].1.last().expect("orld").1, 4);
        // The deleted paragraphs keep an empty line at the join, which
        // clicks pass over.
        for index in [1, 2] {
            let (_, lines) = &paragraphs[index];
            assert_eq!(lines.len(), 1);
            assert!(lines[0].1.is_empty());
            assert_eq!(lines[0].0, hello[0].0);
        }
        let selection = &renderer.selection;
        let line = &selection.paragraphs()[0].lines[0];
        let y = line.absolute_top + line.height / 2.0;
        assert_eq!(
            selection.hit_test(hello_end as f64 + 12.0, y),
            Some(TextPosition::new(3, 1))
        );
        assert_eq!(
            selection.hit_test(world_fragment.2 as f64 - 12.0, y),
            Some(TextPosition::new(0, 4))
        );
    }

    #[test]
    fn markup_view_keeps_every_paragraph_mark() {
        let doc = load(&joined_body());
        let mut renderer = recording_renderer();
        render(&mut renderer, &doc, true);
        let paragraphs = registrations(&renderer);
        let tops: Vec<i64> = paragraphs.iter().map(|(_, lines)| lines[0].0).collect();
        assert!(
            tops.windows(2).all(|pair| pair[0] < pair[1]),
            "{paragraphs:?}"
        );
        assert_eq!(paragraphs[0].0, "Hello");
        assert_eq!(paragraphs[3].0, "World");
    }

    /// An edit of a later member lays the whole join out again, as a fresh
    /// render of the edited document does, and so does switching views.
    #[test]
    fn editing_a_joined_paragraph_matches_a_fresh_render() {
        let mut doc = load(&joined_body());
        let mut renderer = recording_renderer();
        render(&mut renderer, &doc, false);
        render(&mut renderer, &doc, true);
        render(&mut renderer, &doc, false);
        let mut fresh = recording_renderer();
        render(&mut fresh, &doc, false);
        assert_eq!(registrations(&renderer), registrations(&fresh));

        for (paragraph, text) in [(3, "orld and more"), (0, "Hello there")] {
            renderer.drop_document_cache();
            let run = doc
                .paragraph_mut(paragraph)
                .and_then(|paragraph| paragraph.runs.last_mut())
                .expect("run");
            run.text = text.into();
            renderer.set_incremental_dirty_from(paragraph);
            render(&mut renderer, &doc, false);
            let mut fresh = recording_renderer();
            render(&mut fresh, &doc, false);
            assert_eq!(
                registrations(&renderer),
                registrations(&fresh),
                "after editing paragraph {paragraph}"
            );
        }
        let paragraphs = registrations(&renderer);
        assert_eq!(paragraphs[0].0, "Hello there");
        assert_eq!(paragraphs[3].0, "orld and more");
        assert_eq!(paragraphs[0].1[0].0, paragraphs[3].1[0].0);
    }

    /// Accepting the deleted marks joins the paragraphs in the model, and the
    /// joined paragraph no longer carries the mark revision.
    #[test]
    fn deciding_a_deleted_mark_updates_the_model() {
        let mut doc = load(&joined_body());
        let mark = |doc: &WordDocument, index: usize| {
            doc.paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.mark_revision.as_ref())
                .map(|mark| mark.kind.clone())
        };
        assert_eq!(mark(&doc, 0), Some(RevisionKind::Deleted));
        doc.reject_revisions(None, None, None, None)
            .expect("reject");
        assert!((0..doc.paragraph_count()).all(|index| mark(&doc, index).is_none()));

        let mut doc = load(&joined_body());
        doc.accept_revisions(None, None, None, None)
            .expect("accept");
        assert!((0..doc.paragraph_count()).all(|index| mark(&doc, index).is_none()));
        let mut renderer = recording_renderer();
        render(&mut renderer, &doc, false);
        let texts: Vec<String> = registrations(&renderer)
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        assert_eq!(texts, ["Helloorld", "Next"]);
    }
}
