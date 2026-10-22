use super::*;

/// Source topology only. Explicit breaks and section boundaries retain priority.
pub(super) fn group_end(
    elements: &[BodyElement],
    start: usize,
    section_end: usize,
) -> Option<usize> {
    let mut index = start;
    loop {
        let paragraph = elements.get(index)?.as_paragraph()?;
        if paragraph
            .runs
            .iter()
            .any(|run| run.text.contains(PAGE_BREAK_CHAR))
        {
            return if index > start { Some(index - 1) } else { None };
        }
        if !paragraph
            .properties
            .as_ref()
            .and_then(|props| props.keep_next)
            .unwrap_or(false)
        {
            return (index > start).then_some(index);
        }
        let next = index + 1;
        if next >= section_end {
            return (index > start).then_some(index);
        }
        let Some(next_paragraph) = elements.get(next).and_then(BodyElement::as_paragraph) else {
            return (index > start).then_some(index);
        };
        if next_paragraph
            .properties
            .as_ref()
            .and_then(|props| props.page_break_before)
            .unwrap_or(false)
        {
            return (index > start).then_some(index);
        }
        index = next;
    }
}

impl<P: Painter> CanvasRenderer<P>
where
    P::Error: PainterError,
{
    pub(super) fn keep_next_group_owner(&self, paragraph: usize) -> usize {
        self.keep_next_group_surfaces
            .range(..=paragraph)
            .next_back()
            .filter(|(_, last)| paragraph <= **last)
            .map_or(paragraph, |(first, _)| *first)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn measure_keep_next_group(
        &mut self,
        elements: &[BodyElement],
        options: &Options,
        numbering_part: Option<&NumberingPart>,
        counter: &NumberingCounter,
        deleted: &HashMap<String, Vec<u32>>,
        first_element: usize,
        section: usize,
    ) -> Result<f64, JsValue> {
        let mut measurer = self
            .keep_next_measurer
            .take()
            .unwrap_or_else(|| Box::new(Self::with_preconfigured_host(None, false)));
        // Direct body measurement bypasses the normal render-entry reset.
        // The previous group has returned and dropped all scratch borrows.
        measurer.render_pass_arena.borrow_mut().reset();
        // Object diagnostics belong to this group, not the reusable helper.
        measurer.object_debug_regions.borrow_mut().clear();
        measurer.page_width = self.page_width;
        measurer.page_height = self.page_height;
        measurer.margin_left = self.margin_left;
        measurer.margin_right = self.margin_right;
        measurer.margin_top = self.margin_top;
        measurer.margin_bottom = self.margin_bottom;
        measurer.current_page_number = self.current_page_number;
        measurer.page_offset_y = self.page_offset_y;
        measurer.column_layout = self.column_layout.clone();
        measurer.column_index = self.column_index;
        measurer
            .header_content_heights
            .clone_from(&self.header_content_heights);
        measurer
            .footer_content_heights
            .clone_from(&self.footer_content_heights);
        measurer.current_page_started_this_pass = self.current_page_started_this_pass;
        measurer.current_datetime = self.current_datetime;
        measurer
            .bookmark_paragraphs
            .clone_from(&self.bookmark_paragraphs);
        measurer
            .bookmark_note_targets
            .clone_from(&self.bookmark_note_targets);
        // PAGEREF reads the same established bookmark geometry as the body.
        // Copy only those entries; measurement never mutates body layout state.
        measurer.paragraph_layout_cache.borrow_mut().clear();
        for paragraph in self.bookmark_paragraphs.values() {
            if let Some(entry) = self
                .paragraph_layout_cache
                .borrow()
                .get_entry(*paragraph)
                .cloned()
            {
                measurer
                    .paragraph_layout_cache
                    .borrow_mut()
                    .insert(*paragraph, entry);
            }
        }
        measurer
            .page_section_indices
            .clone_from(&self.page_section_indices);
        measurer
            .section_page_numbering
            .clone_from(&self.section_page_numbering);
        measurer.page_number_start = self.page_number_start;
        measurer
            .page_number_format
            .clone_from(&self.page_number_format);
        measurer.document_has_notes = self.document_has_notes;
        measurer.document_has_comments = self.document_has_comments;
        measurer.dark_mode_text = self.dark_mode_text;
        measurer
            .footnote_display_map
            .clone_from(&self.footnote_display_map);
        measurer
            .endnote_display_map
            .clone_from(&self.endnote_display_map);
        measurer
            .footnote_section_map
            .clone_from(&self.footnote_section_map);
        measurer
            .endnote_section_map
            .clone_from(&self.endnote_section_map);
        measurer.footnote_order.clone_from(&self.footnote_order);
        measurer.endnote_order.clone_from(&self.endnote_order);
        measurer.footnote_counter = self.footnote_counter;
        measurer.endnote_counter = self.endnote_counter;
        measurer
            .section_footnote_numbering
            .clone_from(&self.section_footnote_numbering);
        measurer
            .section_endnote_numbering
            .clone_from(&self.section_endnote_numbering);
        measurer
            .footnote_surface_bases
            .clone_from(&self.footnote_surface_bases);
        measurer
            .footnote_surface_extras
            .clone_from(&self.footnote_surface_extras);
        measurer
            .footnote_number_format
            .clone_from(&self.footnote_number_format);
        measurer.footnote_number_start = self.footnote_number_start;
        measurer
            .endnote_number_format
            .clone_from(&self.endnote_number_format);
        measurer.endnote_number_start = self.endnote_number_start;
        measurer.footnote_number_restart = self.footnote_number_restart;
        measurer.endnote_number_restart = self.endnote_number_restart;
        measurer.footnote_numbering_scope = self.footnote_numbering_scope;
        measurer.endnote_numbering_scope = self.endnote_numbering_scope;
        measurer
            .word_oracle_active
            .set(self.word_oracle_active.get());
        if self.word_oracle_active.get() {
            measurer
                .word_oracle_advances
                .borrow_mut()
                .clone_from(&self.word_oracle_advances.borrow());
            measurer
                .word_oracle_text_advances
                .borrow_mut()
                .clone_from(&self.word_oracle_text_advances.borrow());
            measurer
                .word_oracle_paragraph_advances
                .borrow_mut()
                .clone_from(&self.word_oracle_paragraph_advances.borrow());
        }
        measurer.footnote_reserve_px = self.footnote_reserve_px;
        measurer.footnote_height_cache.clear();
        measurer.footnote_height_geometry = None;
        measurer.unplaced_footnote_refs.clear();
        measurer.note_resume = None;
        measurer.note_resume_offset = None;
        measurer.paragraph_restart = None;
        measurer
            .active_float_exclusions
            .clone_from(&self.active_float_exclusions);
        // The body pass has already refreshed this immutable snapshot for the
        // incoming model. Reuse it for every group instead of cloning the full
        // numbering definitions again. Counters below remain measurement-local.
        measurer.last_numbering_part = self.last_numbering_part.clone();
        measurer.current_y = self.current_y;
        measurer.last_line_baseline = self.last_line_baseline;
        measurer.trailing_spacing_after = self.trailing_spacing_after;
        measurer.paragraph_counter = self.paragraph_counter;
        measurer.active_section_index = section;
        measurer.last_document = self.last_document.clone();
        measurer.styles_part = self.styles_part.clone();
        measurer.theme_part = self.theme_part.clone();
        measurer.current_doc_grid = self.current_doc_grid.clone();
        measurer.current_doc_grid_char_spacing_px = self.current_doc_grid_char_spacing_px;
        measurer.current_text_direction = self.current_text_direction;
        measurer.document_snap_to_grid_default = self.document_snap_to_grid_default;
        measurer.document_default_tab_width = self.document_default_tab_width;
        measurer.document_no_tab_hanging_indent = self.document_no_tab_hanging_indent;
        measurer.document_hyphenation = self.document_hyphenation;
        measurer.document_expands_shift_return = self.document_expands_shift_return;
        measurer.document_underlines_trailing_spaces = self.document_underlines_trailing_spaces;
        measurer.document_adjust_line_height_in_table = self.document_adjust_line_height_in_table;
        measurer.document_east_asian_rules = self.document_east_asian_rules.clone();
        measurer
            .contextual_suppress_before
            .clone_from(&self.contextual_suppress_before);
        measurer
            .contextual_suppress_after
            .clone_from(&self.contextual_suppress_after);
        measurer
            .paragraph_border_box_continues
            .clone_from(&self.paragraph_border_box_continues);
        measurer.spacing_scan_from_story = self.spacing_scan_from_story;
        measurer.paragraph_wrap_cache = Rc::clone(&self.paragraph_wrap_cache);
        measurer.render_skip_draw = true;
        measurer.measuring_keep_next = true;
        measurer.incremental_layout_enabled = false;
        measurer.skip_layout_cache_update = true;
        measurer.selection.freeze();
        measurer.invalidate_context_state_tracking();
        let start_y = measurer.current_y;
        let mut counter = counter.clone();
        let mut deleted = deleted.clone();
        std::mem::swap(&mut measurer.host.value, &mut self.host.value);
        std::mem::swap(
            &mut measurer.text_measure_cache,
            &mut self.text_measure_cache,
        );
        std::mem::swap(
            &mut measurer.applied_run_style_cache,
            &mut self.applied_run_style_cache,
        );
        #[cfg(feature = "opentype-math")]
        std::mem::swap(&mut measurer.math_fonts, &mut self.math_fonts);
        // A group ending in keepNext (before a table, a page break or the
        // section end) has no tail paragraph.
        let tail = elements.len().checked_sub(1).filter(|&last| {
            elements[last].as_paragraph().is_some_and(|paragraph| {
                !paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.keep_next)
                    .unwrap_or(false)
            })
        });
        measurer.keep_next_tail_bottom = None;
        measurer.paragraph_join = None;
        measurer.paragraph_join_last = None;
        let result = (|| {
            for (offset, element) in elements.iter().enumerate() {
                measurer.prepare_paragraph_join(
                    elements,
                    offset,
                    elements.len(),
                    first_element,
                    options,
                );
                // The group needs only the lines of its first member that may
                // split that cannot be split from what precedes them (see
                // `keep_next_tail_pending`).
                measurer.keep_next_tail_pending = element
                    .as_paragraph()
                    .is_some()
                    .then_some(Some(offset) == tail);
                measurer.render_body_element(
                    element,
                    options,
                    numbering_part,
                    &mut counter,
                    &mut deleted,
                    first_element + offset,
                    section,
                )?;
                if measurer.keep_next_tail_bottom.is_some() {
                    break;
                }
            }
            measurer.keep_next_tail_pending = None;
            let bottom = measurer
                .keep_next_tail_bottom
                .take()
                .unwrap_or(measurer.current_y);
            Ok((bottom - start_y).max(0.0))
        })();
        std::mem::swap(
            &mut measurer.applied_run_style_cache,
            &mut self.applied_run_style_cache,
        );
        #[cfg(feature = "opentype-math")]
        std::mem::swap(&mut measurer.math_fonts, &mut self.math_fonts);
        std::mem::swap(
            &mut measurer.text_measure_cache,
            &mut self.text_measure_cache,
        );
        std::mem::swap(&mut measurer.host.value, &mut self.host.value);
        // The measurement path applies fonts/fills through host measurement APIs.
        // Re-activate body drawing state after returning the host; no painter is
        // required by a headless layout-only host.
        self.invalidate_context_state_tracking();
        self.keep_next_measurer = Some(measurer);
        result
    }

    pub(super) fn advance_keep_next_group(&mut self) -> Result<(), JsValue> {
        let remaining = (self.effective_body_bottom() - self.current_y).max(0.0);
        self.ensure_space(remaining + 1.0)
    }
}
