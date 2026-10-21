use crate::docx_xml::{XmlNode, XmlNodeId};
use crate::numbering::{
    is_english_language, to_english_cardinal, to_english_ordinal, to_english_ordinal_text, to_hex,
    to_letter, to_roman,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::rc::Rc;

mod math;
mod table;
mod table_conditional;
pub use math::{math_elements_to_omml, MathElement, MathElementKind, MathParagraph, MathRun};
pub use table::{
    resolve_cell_margin_edge_points, StyleMap, Table, TableBorders, TableCell, TableCellMargins,
    TableColumn, TableLayoutMode, TableRow, TableRowHeightRule, TableStyleDisplay,
    TableStyleFirstRow, TextDirection, DEFAULT_VERTICAL_CELL_MARGIN_PT,
};
pub use table_conditional::{
    table_style_condition_matches, TableLook, TableStyleBandSizes, TableStyleCellBorders,
    TableStyleCellFormat, TableStyleCellPosition, TableStyleCondition, TableStyleConditionMatch,
    TableStyleConditionalFormat, TableStyleRegionEdges,
};

/// Top-level representation of a DOCX document after parsing.
///
/// The structure mirrors the parts that the renderer consumes: the document
/// body with block-level elements and the optional page settings derived from
/// section properties.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Document {
    pub body: Option<Body>,
    /// Ordered references to external-format blocks (`w:altChunk`). The block
    /// index records where resolved content belongs among `Body::elements`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alt_chunks: Vec<AltChunkReference>,
    pub sections: Vec<SectionRange>,
    pub default_section: Option<SectionProperties>,
    pub page_settings: Option<PageSettings>,
    pub header_reference: Option<String>,
    pub footer_reference: Option<String>,
    #[serde(default)]
    pub footnotes: BTreeMap<String, Note>,
    #[serde(default)]
    pub endnotes: BTreeMap<String, Note>,
    /// Document-wide note numbering defaults from `settings.xml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footnote_properties: Option<NoteNumbering>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endnote_properties: Option<NoteNumbering>,
    /// Document-wide `settings.xml/endnotePr/pos`, overridden by section settings.
    /// Absence uses the OOXML document-end default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endnote_position: Option<EndnotePosition>,
    #[serde(default)]
    pub snap_to_grid_default: bool,
    /// Authored `settings.xml/revisionView/@insDel`; hosts may override it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_content_revisions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_tab_stop: Option<f32>,
    /// `w:noTabHangInd`: disables the implicit tab stop normally created by a
    /// hanging indent.
    #[serde(default)]
    pub no_tab_hanging_indent: bool,
    /// `w:adjustLineHeightInTable`: applies the section's line grid pitch to
    /// lines in table cells. Absent, table cells ignore the line grid.
    #[serde(default)]
    pub adjust_line_height_in_table: bool,
    /// Effective `w:docDefaults/w:rPrDefault/w:rPr/w:sz` font size in points.
    /// Empty paragraphs need this value even though they contain no run to
    /// carry the materialized style cascade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_font_size: Option<f32>,
    #[serde(default)]
    pub east_asian_line_breaking: EastAsianLineBreakingSettings,
    /// Document-level `w:gutterAtTop` setting. When enabled, page gutters
    /// enlarge the top margin rather than the left binding margin.
    #[serde(default)]
    pub gutter_at_top: bool,
    /// Document-level `w:mirrorMargins` setting. Even pages exchange the
    /// physical left/right margins to preserve inside/outside semantics.
    #[serde(default)]
    pub mirror_margins: bool,
    /// Document-level `w:bookFoldPrinting` (本 (縦方向に谷折り)): each
    /// sheet folds into two pages, so a page is half the sheet's width and
    /// its margins face each other like mirror margins.
    #[serde(default)]
    pub book_fold_printing: bool,
    /// Document-level `w:printTwoOnOne` (袋とじ / 2 pages per sheet): each
    /// sheet carries two pages, so a page is half the sheet along its
    /// longer side.
    #[serde(default)]
    pub print_two_on_one: bool,
    /// All bookmarks (`<w:bookmarkStart>` / `<w:bookmarkEnd>` pairs) found
    /// while parsing the body, indexed in document order. Used by internal
    /// hyperlinks (`w:anchor`) and by consumers that need to resolve or
    /// display named anchors.
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    /// Document page background color (`<w:background w:color="…"/>`, a child of
    /// `<w:document>`; ECMA-376 Part 1 §17.2.1). Normalized to `#RRGGBB`; `auto`
    /// or absent leaves it `None` (the default white/transparent page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    /// Word-compatible text watermark discovered in a referenced header VML
    /// shape (`v:textpath/@string`). The raw header XML remains the export
    /// source of truth; this normalized value lets renderers and hosts expose
    /// the watermark without understanding VML.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark_text: Option<String>,
    /// Table-of-contents fields found in the body, in document order. Each entry
    /// is the parsed switch set of one `TOC` field instruction
    /// (`{ TOC \o "1-3" \h \z \u }`; ECMA-376 Part 1 §17.16.5.68). The TOC
    /// result (the generated entry paragraphs Word last computed) still renders
    /// as ordinary body content; this captures the field's options so a host can
    /// detect the TOC, drive a navigation pane, or regenerate the entries with
    /// [`Document::generate_outline`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toc_fields: Vec<TocOptions>,
    /// Hidden `TC` (Table of Contents Entry) fields found in the body. Word
    /// uses these to add entries that do not have a heading style.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toc_entry_fields: Vec<TocEntryField>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EndnotePosition {
    #[serde(rename = "sectEnd")]
    SectionEnd,
    #[serde(rename = "docEnd")]
    DocumentEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AltChunkReference {
    pub relationship_id: String,
    pub block_index: usize,
}

/// Parsed source metadata from a hidden `TC "Text"` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TocEntryField {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    pub level: u8,
    pub hide_page_number: bool,
    pub paragraph: usize,
}

/// Parsed switch set of a `TOC` field instruction (ECMA-376 Part 1 §17.16.5.68).
///
/// Only the switches that affect which headings are collected and how they are
/// presented are modeled. The defaults match Word's behavior for a bare `TOC`
/// field (all outline levels, no hyperlinks, page numbers shown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TocOptions {
    /// Lowest heading level included, 1-based (`\o "MIN-MAX"`). Defaults to 1.
    pub outline_min: u8,
    /// Highest heading level included, 1-based. Defaults to 9.
    pub outline_max: u8,
    /// `\h` — render entries as hyperlinks to the heading bookmarks.
    pub hyperlinks: bool,
    /// `\n` — omit page numbers from entries.
    pub hide_page_numbers: bool,
    /// Optional level range carried by `\n "MIN-MAX"`. Word suppresses page
    /// numbers only for entries in this range; bare `\n` uses
    /// [`Self::hide_page_numbers`] to suppress every level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_page_number_levels: Option<(u8, u8)>,
    /// Style-to-level mappings from `\t "Style,1,Other Style,2"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_style_levels: Vec<(String, u8)>,
    /// Custom entry/page-number separator from `\p "TEXT"`. Word uses the
    /// first character; `None` retains the default right tab and dot leader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_number_separator: Option<String>,
    /// `\w` — preserve tab characters found in source heading text.
    #[serde(default)]
    pub preserve_tabs: bool,
    /// `\x` — preserve manual line breaks found in source heading text.
    #[serde(default)]
    pub preserve_line_breaks: bool,
    /// Bookmark name from `\b "NAME"`, limiting entries to that range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark_name: Option<String>,
    /// Sequence identifier from `\c "LABEL"` for a table of figures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_label: Option<String>,
    /// `\a` — omit the caption label and SEQ number from generated entry text.
    #[serde(default)]
    pub omit_caption_label_and_number: bool,
    /// `\f [IDENTIFIER]` — build from hidden `TC` fields, optionally filtering
    /// to the matching entry type.
    #[serde(default)]
    pub use_toc_entry_fields: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toc_entry_identifier: Option<String>,
    /// `\l "MIN-MAX"` — include only TC fields assigned to these levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toc_entry_levels: Option<(u8, u8)>,
    /// `\s IDENTIFIER` and optional `\d "SEPARATOR"`: prefix each page
    /// number with the most recent matching SEQ value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence_page_number: Option<TocSequencePageNumber>,
    /// `\u` — collect paragraphs by their applied outline level (in addition to
    /// the TOC heading styles).
    pub use_applied_outline_levels: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TocSequencePageNumber {
    pub identifier: String,
    /// `None` retains Word's default hyphen separator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separator: Option<String>,
}

impl Default for TocOptions {
    fn default() -> Self {
        Self {
            outline_min: 1,
            outline_max: 9,
            hyperlinks: false,
            hide_page_numbers: false,
            hide_page_number_levels: None,
            custom_style_levels: Vec::new(),
            page_number_separator: None,
            preserve_tabs: false,
            preserve_line_breaks: false,
            bookmark_name: None,
            caption_label: None,
            omit_caption_label_and_number: false,
            use_toc_entry_fields: false,
            toc_entry_identifier: None,
            toc_entry_levels: None,
            sequence_page_number: None,
            use_applied_outline_levels: false,
        }
    }
}

impl TocOptions {
    pub fn hides_page_number_at_level(&self, level: u8) -> bool {
        self.hide_page_numbers
            || self
                .hide_page_number_levels
                .is_some_and(|(min, max)| (min..=max).contains(&level))
    }
}

/// A single generated table-of-contents / outline entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TocEntry {
    /// The heading's text (concatenated run text, trimmed).
    pub text: String,
    /// 1-based heading level (Heading 1 → 1, … Heading 9 → 9).
    pub level: u8,
    /// 0-based index into `Body::elements` of the heading paragraph. Lets a host
    /// jump to / scroll the heading even when no bookmark is present.
    pub paragraph: usize,
    /// Name of the bookmark anchoring the heading (`_Toc…` when Word generated
    /// the TOC), if one starts on the heading paragraph. Used as the hyperlink
    /// target for `\h` TOCs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<String>,
    /// A per-entry `TC \n` override. This is independent of the TOC-level
    /// `\n` switch and suppresses the page number only for this entry.
    #[serde(default)]
    pub hide_page_number: bool,
}

/// A resolved `<w:bookmarkStart>` / `<w:bookmarkEnd>` pair.
///
/// Positions are expressed in the same coordinate space the renderer uses for
/// selection and hit testing: a paragraph index into `Body::elements`
/// (top-level) plus a byte offset within that paragraph's concatenated run
/// text. An unclosed bookmark (start without matching end — malformed but
/// tolerated) records `end_paragraph` / `end_offset` equal to its start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    /// Raw `w:id` from the source XML. Word uses integer strings but we keep
    /// it as a string to round-trip unknown shapes.
    pub id: String,
    /// Human-readable `w:name`. Internal hyperlinks reference bookmarks by
    /// this name, not by id.
    pub name: String,
    pub start_paragraph: usize,
    pub start_offset: usize,
    /// Run boundary at which the bookmark starts. Computed fields are
    /// zero-width in plain-text offsets, so this preserves their relative
    /// position when several fields share a paragraph.
    #[serde(default)]
    pub start_run: usize,
    pub end_paragraph: usize,
    pub end_offset: usize,
    /// Run boundary at which the bookmark ends; see [`Bookmark::start_run`].
    #[serde(default)]
    pub end_run: usize,
}

impl Document {
    /// Section children override only their corresponding document-wide values.
    /// The final body `sectPr` supplies defaults only for its own section.
    pub fn resolved_note_numbering(
        &self,
        section_index: Option<usize>,
        is_footnote: bool,
    ) -> NoteNumbering {
        let defaults = if is_footnote {
            self.footnote_properties.as_ref()
        } else {
            self.endnote_properties.as_ref()
        };
        let section = section_index
            .and_then(|index| self.sections.get(index))
            .and_then(|section| section.properties.as_ref())
            .or(self.default_section.as_ref());
        let overrides = section.and_then(|section| {
            if is_footnote {
                section.footnote_properties.as_ref()
            } else {
                section.endnote_properties.as_ref()
            }
        });
        NoteNumbering {
            number_format: overrides
                .and_then(|note| note.number_format.clone())
                .or_else(|| defaults.and_then(|note| note.number_format.clone())),
            number_start: overrides
                .and_then(|note| note.number_start)
                .or_else(|| defaults.and_then(|note| note.number_start)),
            number_restart: overrides
                .and_then(|note| note.number_restart.clone())
                .or_else(|| defaults.and_then(|note| note.number_restart.clone())),
        }
    }

    /// Derives [`Self::page_settings`] — what the renderer lays the first
    /// section out with — from that section's properties, falling back to
    /// the body-level `w:sectPr`. Load calls this once `sections` is built;
    /// an edit that rebuilds the sections calls it again, so a join that
    /// gives the first section different properties lays out as a reload
    /// of the saved file would. A document with no section properties at
    /// all keeps the value it has, since there is nothing to derive it from.
    pub fn sync_page_settings_with_first_section(&mut self) {
        if let Some(properties) = self
            .sections
            .first()
            .and_then(|section| section.properties.as_ref())
            .or(self.default_section.as_ref())
        {
            self.page_settings = properties.page_settings.clone();
        }
    }

    /// Resolves section placement over the document-wide endnote default.
    pub fn resolved_endnote_position(&self, section_index: Option<usize>) -> EndnotePosition {
        let section = match section_index {
            Some(index) if !self.sections.is_empty() => self
                .sections
                .get(index)
                .and_then(|section| section.properties.as_ref()),
            _ => self.default_section.as_ref(),
        };
        section
            .and_then(|section| section.endnote_position)
            .or(self.endnote_position)
            .unwrap_or(EndnotePosition::DocumentEnd)
    }

    /// Resolves structural and comparison field values, storing
    /// the result in each field's [`RunField::resolved_value`]. Call once after
    /// parsing (and after `bookmarks` is populated).
    ///
    /// - `REF bookmarkName` (§17.16.5.52) → the text spanned by the named
    ///   bookmark.
    /// - `SET Bookmark "Text"` (§17.16.5.54) → an invisible bookmark value
    ///   assignment observed by later REF/IF/COMPARE fields.
    /// - `QUOTE "LiteralText"` (§17.16.5.51) → its instruction literal.
    /// - `MACROBUTTON MacroName DisplayText` → its inert display label; the
    ///   macro itself is never executed.
    /// - `GOTOBUTTON Destination DisplayText` → its display label; bookmark
    ///   destinations also retain internal navigation metadata.
    /// - `SEQ identifier` (§17.16.5.64) → a 1-based counter, incremented for
    ///   each occurrence sharing the same (case-insensitive) identifier, in
    ///   document order (body paragraphs first, recursing into table cells),
    ///   including restart/repeat/hidden/heading-level switch semantics.
    /// - `COMPARE` / `IF` operands that name a bookmark → the text spanned by
    ///   that bookmark before Word-compatible numeric/string comparison.
    pub fn resolve_structural_fields(&mut self) {
        let bookmark_text = self.bookmark_text_map();
        self.resolve_structural_fields_with_bookmark_text(bookmark_text);
    }

    /// Resolves fields in this story with bookmark values inherited from a
    /// different story in the same Word package. Header/footer `REF` fields,
    /// for example, can target bookmarks and final `SET` assignments defined
    /// in the main document.
    pub fn resolve_structural_fields_with_bookmarks_from(&mut self, source: &Document) {
        let mut bookmark_text = source.final_bookmark_value_map();
        // A bookmark physically defined in this story is the closest scope and
        // therefore wins if a malformed package repeats a name across stories.
        bookmark_text.extend(self.bookmark_text_map());
        self.resolve_structural_fields_with_bookmark_text(bookmark_text);
    }

    fn resolve_structural_fields_with_bookmark_text(
        &mut self,
        mut bookmark_text: std::collections::HashMap<String, String>,
    ) {
        let bookmarks = self.bookmarks.clone();
        let Some(body) = self.body.as_mut() else {
            return;
        };
        // A bookmark can enclose a computed caption number, including a
        // forward reference whose saved SEQ result is stale. Resolve those
        // sequences before constructing the REF environment. Use a separate
        // environment so SET assignments still take effect in document order
        // during the final pass.
        let sequence_bookmarks: Vec<_> = bookmarks
            .iter()
            .filter(|bookmark| {
                (bookmark.start_paragraph..=bookmark.end_paragraph).any(|index| {
                    body.elements
                        .get(index)
                        .and_then(BodyElement::as_paragraph)
                        .is_some_and(|paragraph| {
                            paragraph.runs.iter().any(|run| {
                                matches!(
                                    run.field.as_ref().map(|field| &field.kind),
                                    Some(RunFieldKind::Seq { .. })
                                )
                            })
                        })
                })
            })
            .collect();
        if !sequence_bookmarks.is_empty() {
            let mut preliminary_values = bookmark_text.clone();
            resolve_body_fields(body, &bookmarks, &mut preliminary_values);
            for bookmark in sequence_bookmarks {
                if let Some(text) = extract_bookmark_text(body, bookmark) {
                    bookmark_text.insert(bookmark.name.clone(), text);
                }
            }
        }
        resolve_body_fields(body, &bookmarks, &mut bookmark_text);
    }

    /// Returns the displayed text inside a bookmark, including computed fields.
    pub fn bookmark_display_text(&self, bookmark: &Bookmark) -> Option<String> {
        extract_bookmark_text(self.body.as_ref()?, bookmark)
    }

    /// Returns the displayed value of the last matching `SEQ` field at or
    /// before a top-level body element. Word's `TOC \s IDENTIFIER` uses this
    /// value as the chapter-like prefix of each generated page number.
    pub fn sequence_value_at_or_before(
        &self,
        identifier: &str,
        body_element_index: usize,
    ) -> Option<String> {
        let body = self.body.as_ref()?;
        let mut value = None;
        for element in body.elements.iter().take(body_element_index + 1) {
            update_last_sequence_value(element, identifier, &mut value);
        }
        value
    }

    /// Generates table-of-contents entries by scanning top-level heading
    /// paragraphs, honoring the level range in `options` (`\o "MIN-MAX"`).
    ///
    /// A paragraph is a heading when it carries an applied outline level
    /// (`w:outlineLvl`, mapped 0→level 1 … 8→level 9) or its paragraph style id
    /// looks like a built-in heading style (`Heading1`…`Heading9`). Entries keep
    /// the heading text, level, paragraph index, and the bookmark anchoring the
    /// heading (`_Toc…` when Word generated the TOC) for hyperlink navigation.
    ///
    /// This is the structured foundation for a navigation pane and for
    /// regenerating a `TOC` field's body. It does not itself rewrite the
    /// document. Headings nested inside tables are not collected (Word omits
    /// them from a default TOC as well in the common case).
    pub fn generate_outline(&self, options: &TocOptions) -> Vec<TocEntry> {
        let Some(body) = self.body.as_ref() else {
            return Vec::new();
        };
        let bookmark_range = if let Some(name) = options.bookmark_name.as_deref() {
            // Word retains the last definition when duplicate bookmark names
            // occur. Restricting here keeps navigation outlines and regenerated
            // TOC fields on the same source range.
            let Some(bookmark) = self
                .bookmarks
                .iter()
                .rev()
                .find(|bookmark| bookmark.name.eq_ignore_ascii_case(name))
            else {
                return Vec::new();
            };
            Some((
                bookmark.start_paragraph.min(bookmark.end_paragraph),
                bookmark.start_paragraph.max(bookmark.end_paragraph),
            ))
        } else {
            None
        };
        let mut entries = Vec::new();
        if options.use_toc_entry_fields {
            for field in &self.toc_entry_fields {
                if bookmark_range
                    .is_some_and(|(start, end)| field.paragraph < start || field.paragraph > end)
                {
                    continue;
                }
                if let Some(identifier) = options.toc_entry_identifier.as_deref() {
                    if !field
                        .identifier
                        .as_deref()
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(identifier))
                    {
                        continue;
                    }
                } else if field.identifier.is_some() {
                    // A bare `\f` selects the default/untyped TC list. Typed
                    // entries belong to their explicitly named contents list.
                    continue;
                }
                if options
                    .toc_entry_levels
                    .is_some_and(|(min, max)| field.level < min || field.level > max)
                {
                    continue;
                }
                entries.push(TocEntry {
                    text: field.text.clone(),
                    level: field.level,
                    paragraph: field.paragraph,
                    bookmark: self.bookmark_anchoring_paragraph(field.paragraph),
                    hide_page_number: field.hide_page_number,
                });
            }
            return entries;
        }
        for (index, element) in body.elements.iter().enumerate() {
            if bookmark_range.is_some_and(|(start, end)| index < start || index > end) {
                continue;
            }
            let Some(paragraph) = element.as_paragraph() else {
                continue;
            };
            let level = if let Some(label) = options.caption_label.as_deref() {
                if !paragraph_has_seq_identifier(paragraph, label) {
                    continue;
                }
                // Word's table-of-figures result is a flat list using one
                // paragraph style, independent of heading outline levels.
                1
            } else {
                let Some(level) = custom_toc_style_level(paragraph, options)
                    .or_else(|| heading_level(paragraph, options))
                else {
                    continue;
                };
                if level < options.outline_min || level > options.outline_max {
                    continue;
                }
                level
            };
            let text = toc_heading_text(paragraph, options);
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            entries.push(TocEntry {
                text: text.to_string(),
                level,
                paragraph: index,
                bookmark: self.bookmark_anchoring_paragraph(index),
                hide_page_number: false,
            });
        }
        // Microsoft Word includes untyped TC fields in an ordinary TOC even
        // when the TOC instruction has no explicit `\f` switch. Typed fields
        // remain isolated to their matching `TOC \f identifier` list. Merge
        // these manual entries with heading/style entries in document order.
        if options.caption_label.is_none() {
            entries.extend(
                self.toc_entry_fields
                    .iter()
                    .filter(|field| field.identifier.is_none())
                    .filter(|field| {
                        !bookmark_range.is_some_and(|(start, end)| {
                            field.paragraph < start || field.paragraph > end
                        })
                    })
                    .map(|field| TocEntry {
                        text: field.text.clone(),
                        level: field.level,
                        paragraph: field.paragraph,
                        bookmark: self.bookmark_anchoring_paragraph(field.paragraph),
                        hide_page_number: field.hide_page_number,
                    }),
            );
            entries.sort_by_key(|entry| entry.paragraph);
        }
        entries
    }

    /// Convenience over [`Document::generate_outline`] using the switches of the
    /// document's first `TOC` field, or default options (all levels) when the
    /// document has no TOC field.
    pub fn outline(&self) -> Vec<TocEntry> {
        let options = self.toc_fields.first().cloned().unwrap_or_default();
        self.generate_outline(&options)
    }

    /// Finds the bookmark that starts on the given top-level paragraph, if any.
    /// A `_Toc…` bookmark (the kind Word emits around TOC-referenced headings) is
    /// preferred so generated entries point at the same anchor the existing TOC
    /// hyperlinks use.
    fn bookmark_anchoring_paragraph(&self, paragraph: usize) -> Option<String> {
        let mut fallback: Option<&str> = None;
        for bookmark in &self.bookmarks {
            if bookmark.start_paragraph != paragraph {
                continue;
            }
            if bookmark.name.starts_with("_Toc") {
                return Some(bookmark.name.clone());
            }
            fallback.get_or_insert(bookmark.name.as_str());
        }
        fallback.map(|name| name.to_string())
    }

    /// Builds a `bookmark name → spanned text` map from the parsed bookmarks.
    pub(crate) fn bookmark_text_map(&self) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        let Some(body) = self.body.as_ref() else {
            return map;
        };
        for bookmark in &self.bookmarks {
            if let Some(text) = extract_bookmark_text(body, bookmark) {
                map.entry(bookmark.name.clone()).or_insert(text);
            }
        }
        map
    }

    fn final_bookmark_value_map(&self) -> std::collections::HashMap<String, String> {
        let mut map = self.bookmark_text_map();
        if let Some(body) = self.body.as_ref() {
            collect_set_assignments(&body.elements, &mut map);
        }
        map
    }
}

fn collect_set_assignments(
    elements: &[BodyElement],
    bookmark_text: &mut std::collections::HashMap<String, String>,
) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                for run in &paragraph.runs {
                    if let Some(RunFieldKind::Set { bookmark, value }) =
                        run.field.as_ref().map(|field| &field.kind)
                    {
                        bookmark_text.insert(bookmark.clone(), value.clone());
                    }
                }
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_set_assignments(&cell.blocks, bookmark_text);
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
}

fn update_last_sequence_value(element: &BodyElement, identifier: &str, value: &mut Option<String>) {
    match element {
        BodyElement::Paragraph(paragraph) => {
            for run in &paragraph.runs {
                if matches!(
                    run.field.as_ref().map(|field| &field.kind),
                    Some(RunFieldKind::Seq {
                        identifier: candidate,
                        bookmark: None,
                        ..
                    })
                        if candidate.eq_ignore_ascii_case(identifier)
                ) {
                    if let Some(display) = run.field.as_ref().and_then(|field| {
                        field
                            .resolved_value
                            .as_deref()
                            .or(field.cached_text.as_deref())
                    }) {
                        if !display.is_empty() {
                            *value = Some(display.to_string());
                        }
                    }
                }
                if let Some(text_box) = run.text_box.as_ref() {
                    for block in &text_box.blocks {
                        update_last_sequence_value(block, identifier, value);
                    }
                }
            }
        }
        BodyElement::Table(table) => {
            for row in &table.rows {
                for cell in &row.cells {
                    for block in &cell.blocks {
                        update_last_sequence_value(block, identifier, value);
                    }
                }
            }
        }
        BodyElement::Math(_) => {}
    }
}

#[derive(Debug, Clone)]
struct ResolvedSequenceValue {
    identifier: String,
    number: u32,
    display: String,
    run_index: Option<usize>,
}

fn sequence_value_near_bookmark(
    values_by_element: &[Vec<ResolvedSequenceValue>],
    bookmark: &Bookmark,
    identifier: &str,
) -> Option<ResolvedSequenceValue> {
    if values_by_element.is_empty() {
        return None;
    }
    let start = bookmark.start_paragraph.min(values_by_element.len() - 1);
    let end = bookmark
        .end_paragraph
        .min(values_by_element.len() - 1)
        .max(start);
    if start == end {
        // Plain-text offsets cannot order computed fields because their saved
        // result is suppressed from `Run::text`. Run boundaries retain that
        // ordering for a bookmark and candidate fields in one paragraph.
        if let Some(value) = values_by_element[start]
            .iter()
            .filter(|value| value.identifier.eq_ignore_ascii_case(identifier))
            .filter_map(|value| value.run_index.map(|run_index| (value, run_index)))
            .min_by_key(|(_, run_index)| {
                if *run_index < bookmark.start_run {
                    (bookmark.start_run - *run_index, false)
                } else if *run_index >= bookmark.end_run {
                    (*run_index - bookmark.end_run, true)
                } else {
                    (0, false)
                }
            })
            .map(|(value, _)| value)
        {
            return Some(value.clone());
        }
    }
    if let Some(value) = values_by_element[start..=end]
        .iter()
        .flat_map(|values| values.iter())
        .rfind(|value| value.identifier.eq_ignore_ascii_case(identifier))
    {
        return Some(value.clone());
    }

    let max_distance = start.max(values_by_element.len() - 1 - end);
    for distance in 1..=max_distance {
        if let Some(index) = start.checked_sub(distance) {
            if let Some(previous) = values_by_element[index]
                .iter()
                .rev()
                .find(|value| value.identifier.eq_ignore_ascii_case(identifier))
            {
                return Some(previous.clone());
            }
        }
        let next = end + distance;
        if next < values_by_element.len() {
            // This element follows the bookmark, so its first matching field
            // is closer in document order. Previous elements intentionally
            // search in reverse for the symmetric reason above.
            if let Some(following) = values_by_element[next]
                .iter()
                .find(|value| value.identifier.eq_ignore_ascii_case(identifier))
            {
                return Some(following.clone());
            }
        }
    }
    None
}

fn resolve_sequence_bookmark_fields(
    body: &mut Body,
    bookmarks: &[Bookmark],
    values_by_element: &[Vec<ResolvedSequenceValue>],
) {
    let mut queries = Vec::new();
    collect_sequence_bookmark_queries(&body.elements, &mut queries);
    let mut resolved = std::collections::HashMap::new();
    for (identifier, bookmark_name) in queries {
        let Some(bookmark) = bookmarks
            .iter()
            .find(|bookmark| bookmark.name == bookmark_name)
        else {
            continue;
        };
        if let Some(value) = sequence_value_near_bookmark(values_by_element, bookmark, &identifier)
        {
            resolved.insert((identifier.to_ascii_lowercase(), bookmark_name), value);
        }
    }
    apply_sequence_bookmark_results(&mut body.elements, &resolved);
}

fn collect_sequence_bookmark_queries(
    elements: &[BodyElement],
    queries: &mut Vec<(String, String)>,
) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                for run in &paragraph.runs {
                    if let Some(RunField {
                        kind:
                            RunFieldKind::Seq {
                                identifier,
                                bookmark: Some(bookmark),
                                ..
                            },
                        ..
                    }) = run.field.as_ref()
                    {
                        queries.push((identifier.clone(), bookmark.clone()));
                    }
                }
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_sequence_bookmark_queries(&cell.blocks, queries);
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
}

fn apply_sequence_bookmark_results(
    elements: &mut [BodyElement],
    resolved: &std::collections::HashMap<(String, String), ResolvedSequenceValue>,
) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                for run in &mut paragraph.runs {
                    let language = run
                        .properties
                        .as_ref()
                        .and_then(|properties| properties.language.as_deref());
                    let Some(field) = run.field.as_mut() else {
                        continue;
                    };
                    let RunFieldKind::Seq {
                        identifier,
                        bookmark: Some(bookmark),
                        hide_result,
                        number_format,
                        case_format,
                        ..
                    } = &field.kind
                    else {
                        continue;
                    };
                    if let Some(value) =
                        resolved.get(&(identifier.to_ascii_lowercase(), bookmark.clone()))
                    {
                        let display = if *hide_result {
                            String::new()
                        } else if let Some(format) = *number_format {
                            format_seq_number(value.number, Some(format), language)
                        } else {
                            value.display.clone()
                        };
                        field.resolved_value = Some(apply_field_text_case(&display, *case_format));
                    }
                }
            }
            BodyElement::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        apply_sequence_bookmark_results(&mut cell.blocks, resolved);
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
}

/// Parses the switches of a `TOC` field instruction (ECMA-376 Part 1
/// §17.16.5.68) into [`TocOptions`]. Recognizes `\o "MIN-MAX"` (level range,
/// bare `\o` means all levels), `\h` (hyperlinks), `\n` (hide page numbers),
/// `\t` (custom style-to-level mappings), `\p` (entry/page separator), `\b`
/// (bookmark scope), `\c` (caption sequence), `\a` (caption text without label
/// and number), `\s` / `\d` (SEQ-prefixed page numbers), `\w` (preserve tabs),
/// `\x` (preserve manual line breaks), and
/// `\u` (use applied outline levels), `\f` (TC entry type), and `\l` (TC entry
/// levels). Unknown switches are ignored. The leading
/// `TOC` token is optional, so both the raw instruction and a switch-only string
/// are accepted.
pub fn parse_toc_switches(instruction: &str) -> TocOptions {
    let mut options = TocOptions::default();
    let mut sequence_identifier: Option<String> = None;
    let mut sequence_separator: Option<String> = None;
    // Tokenize, keeping quoted arguments (`"1-3"`) intact.
    let tokens = tokenize_field_instruction(instruction);
    let mut idx = 0;
    while idx < tokens.len() {
        // Word field codes and their switches are ASCII case-insensitive.
        // Preserve argument spelling, but normalize the control token before
        // dispatch so hand-edited `\\O`/`\\H` and producer-emitted variants
        // behave exactly like Word's usual lowercase instructions.
        let token = tokens[idx].to_ascii_lowercase();
        match token.as_str() {
            "\\o" => {
                // The optional next token is the quoted/plain level range.
                if let Some(range) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    if let Some((min, max)) = parse_outline_range(range) {
                        options.outline_min = min;
                        options.outline_max = max;
                    }
                    idx += 1;
                }
                // Bare `\o` keeps the default full 1-9 range.
            }
            "\\h" => options.hyperlinks = true,
            "\\n" => {
                // Bare `\n` suppresses every page number. With a range, Word
                // suppresses only the specified TOC levels.
                if let Some(range) = tokens
                    .get(idx + 1)
                    .filter(|t| !t.starts_with('\\'))
                    .and_then(|t| parse_outline_range(t))
                {
                    options.hide_page_number_levels = Some(range);
                    idx += 1;
                } else {
                    options.hide_page_numbers = true;
                }
            }
            "\\u" => options.use_applied_outline_levels = true,
            "\\w" => options.preserve_tabs = true,
            "\\x" => options.preserve_line_breaks = true,
            "\\b" => {
                if let Some(name) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    options.bookmark_name = Some(name.clone());
                    idx += 1;
                }
            }
            "\\c" => {
                if let Some(label) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    options.caption_label = Some(label.clone());
                    options.omit_caption_label_and_number = false;
                    idx += 1;
                }
            }
            "\\a" => {
                if let Some(label) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    options.caption_label = Some(label.clone());
                    options.omit_caption_label_and_number = true;
                    idx += 1;
                }
            }
            "\\f" => {
                options.use_toc_entry_fields = true;
                // Word resolves the otherwise ambiguous `\\f` + `\\l`
                // combination by field-code order: a later `\\f` supersedes
                // the earlier level selection and restores all TC levels. A
                // subsequent `\\l` can establish a new restriction below.
                options.toc_entry_levels = None;
                if let Some(identifier) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    options.toc_entry_identifier = Some(identifier.clone());
                    idx += 1;
                }
            }
            "\\l" => {
                options.use_toc_entry_fields = true;
                if let Some(range) = tokens
                    .get(idx + 1)
                    .filter(|t| !t.starts_with('\\'))
                    .and_then(|value| parse_outline_range(value))
                {
                    options.toc_entry_levels = Some(range);
                    idx += 1;
                }
            }
            "\\t" => {
                if let Some(mapping) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    options.custom_style_levels = parse_toc_style_levels(mapping);
                    idx += 1;
                }
            }
            "\\p" => {
                if let Some(separator) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    // ISO/IEC 29500 permits a character sequence, but Word uses
                    // only its first character. Select a Unicode scalar value
                    // rather than the first UTF-8 byte.
                    options.page_number_separator =
                        Some(separator.chars().take(1).collect::<String>());
                    idx += 1;
                }
            }
            "\\s" => {
                if let Some(identifier) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    if !identifier.is_empty() {
                        sequence_identifier = Some(identifier.clone());
                    }
                    idx += 1;
                }
            }
            "\\d" => {
                if let Some(separator) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    sequence_separator = Some(separator.clone());
                    idx += 1;
                }
            }
            _ => {}
        }
        idx += 1;
    }
    if let Some(identifier) = sequence_identifier {
        options.sequence_page_number = Some(TocSequencePageNumber {
            identifier,
            separator: sequence_separator,
        });
    }
    options
}

/// Parses a hidden `TC "Text" [\f TYPE] [\l LEVEL] [\n]` field. The field
/// contributes metadata to a generated TOC but has no visible field result.
pub(crate) fn parse_toc_entry_field(instruction: &str, paragraph: usize) -> Option<TocEntryField> {
    let tokens = tokenize_field_instruction(instruction);
    let tc_index = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case("TC"))?;
    let text = tokens.get(tc_index + 1)?.clone();
    if text.is_empty() || text.starts_with('\\') {
        return None;
    }
    let mut entry = TocEntryField {
        text,
        identifier: None,
        level: 1,
        hide_page_number: false,
        paragraph,
    };
    let mut idx = tc_index + 2;
    while idx < tokens.len() {
        match tokens[idx].to_ascii_lowercase().as_str() {
            "\\f" => {
                if let Some(identifier) = tokens.get(idx + 1).filter(|t| !t.starts_with('\\')) {
                    entry.identifier = Some(identifier.clone());
                    idx += 1;
                }
            }
            "\\l" => {
                if let Some(level) = tokens
                    .get(idx + 1)
                    .filter(|t| !t.starts_with('\\'))
                    .and_then(|value| value.parse::<u8>().ok())
                {
                    entry.level = level.clamp(1, 9);
                    idx += 1;
                }
            }
            "\\n" => entry.hide_page_number = true,
            _ => {}
        }
        idx += 1;
    }
    Some(entry)
}

fn parse_toc_style_levels(value: &str) -> Vec<(String, u8)> {
    // Word follows the operating system list separator here. English systems
    // normally emit commas, while several European locales emit semicolons.
    let separator = if value.contains(';') { ';' } else { ',' };
    let parts: Vec<&str> = value.split(separator).map(str::trim).collect();
    let mut mappings = Vec::new();
    for pair in parts.chunks_exact(2) {
        let style = pair[0];
        let Ok(level) = pair[1].parse::<u8>() else {
            continue;
        };
        if !style.is_empty() {
            mappings.push((style.to_string(), level.clamp(1, 9)));
        }
    }
    mappings
}

/// Splits a field instruction into tokens, treating a `"…"` run as a single
/// token (quotes stripped) so switch arguments like `\o "1-3"` survive.
/// Quote boundaries also delimit adjacent compact syntax such as `"a"="b"`.
/// Within a quoted token, `\"` represents a literal quotation mark.
/// An explicitly quoted empty string remains an empty token because Word uses
/// it as a valid IF/COMPARE operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldInstructionToken {
    pub text: String,
    pub quoted: bool,
}

/// Prefixes a nested field result that holds a straight double quote when it
/// becomes a comparison operand, so that it equals no literal, as in Word.
pub(crate) const QUOTED_NESTED_RESULT_MARK: char = '\u{E7A0}';

/// The result Word for Mac with a Japanese user interface shows, in bold,
/// for an IF field it cannot parse.
pub const WORD_IF_SYNTAX_ERROR: &str = "エラー! 条件式が正しくありません。";

impl RunField {
    /// Whether the field shows Word's IF syntax error, which Word sets bold.
    pub fn shows_word_syntax_error(&self) -> bool {
        matches!(&self.kind, RunFieldKind::Quote { text, .. } if text == WORD_IF_SYNTAX_ERROR)
    }
}

pub(crate) fn tokenize_field_instruction(instruction: &str) -> Vec<String> {
    tokenize_field_instruction_with_quotes(instruction)
        .into_iter()
        .map(|token| token.text)
        .collect()
}

pub(crate) fn tokenize_field_instruction_with_quotes(
    instruction: &str,
) -> Vec<FieldInstructionToken> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut token_started = false;
    let mut token_quoted = false;
    let mut chars = instruction.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if in_quotes && chars.peek() == Some(&'"') => {
                // A backslash-escaped quote is content, not the end of the
                // quoted operand. Consume the quote but omit the escape.
                chars.next();
                current.push('"');
                token_started = true;
            }
            '"' => {
                if in_quotes {
                    // A closing quote completes a token even when it is empty.
                    // Commit it immediately so an adjacent operator starts a
                    // distinct token rather than merging into the literal.
                    tokens.push(FieldInstructionToken {
                        text: std::mem::take(&mut current),
                        quoted: token_quoted,
                    });
                    token_started = false;
                    token_quoted = false;
                    in_quotes = false;
                } else {
                    // Likewise, a quote opening immediately after compact
                    // syntax closes the preceding operator token first.
                    if token_started {
                        tokens.push(FieldInstructionToken {
                            text: std::mem::take(&mut current),
                            quoted: token_quoted,
                        });
                    }
                    token_started = true;
                    token_quoted = true;
                    in_quotes = true;
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if token_started {
                    tokens.push(FieldInstructionToken {
                        text: std::mem::take(&mut current),
                        quoted: token_quoted,
                    });
                    token_started = false;
                    token_quoted = false;
                }
            }
            c => {
                current.push(c);
                token_started = true;
            }
        }
    }
    if token_started {
        tokens.push(FieldInstructionToken {
            text: current,
            quoted: token_quoted,
        });
    }
    tokens
}

/// Parses a `"MIN-MAX"` (or single-level `"N"`) outline range, clamped to 1-9.
fn parse_outline_range(value: &str) -> Option<(u8, u8)> {
    let value = value.trim().trim_matches('"');
    let clamp = |n: u8| n.clamp(1, 9);
    if let Some((min, max)) = value.split_once('-') {
        let min = min.trim().parse::<u8>().ok()?;
        let max = max.trim().parse::<u8>().ok()?;
        Some((clamp(min), clamp(max.max(min))))
    } else {
        let level = value.parse::<u8>().ok()?;
        Some((clamp(level), clamp(level)))
    }
}

/// Returns the 1-based TOC level of a paragraph (Heading 1 → 1 … Heading 9 →
/// 9), or `None` when it is not a source for this TOC. Style-defined outline
/// levels participate through `\o`; a directly applied `w:outlineLvl` requires
/// the `\u` switch. Built-in Heading style ids remain recognized when no
/// resolved style outline level is available.
fn heading_level(paragraph: &Paragraph, options: &TocOptions) -> Option<u8> {
    if let Some(props) = paragraph.properties.as_ref() {
        if let Some(level) = props.outline_level {
            // 0..=8 are Heading 1..9; 9 (and anything higher) is body text.
            // A direct body-text override continues to suppress an inherited
            // heading level, matching the precedence Word applies to direct
            // paragraph formatting.
            if level > 8 {
                return None;
            }
            if !props.outline_level_is_direct || options.use_applied_outline_levels {
                return Some(level + 1);
            }
        }
    }
    let style = paragraph
        .properties
        .as_ref()
        .and_then(|p| p.style.as_deref())?;
    heading_style_level(style)
}

fn custom_toc_style_level(paragraph: &Paragraph, options: &TocOptions) -> Option<u8> {
    let style = paragraph
        .properties
        .as_ref()
        .and_then(|properties| properties.style.as_deref())?;
    options
        .custom_style_levels
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(style))
        .map(|(_, level)| *level)
}

/// Maps a built-in heading style id (`Heading1`…`Heading9`, tolerating case and
/// an internal space like `Heading 3`) to its 1-based level. Returns `None` for
/// non-heading style ids.
fn heading_style_level(style_id: &str) -> Option<u8> {
    let normalized: String = style_id
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect();
    let suffix = normalized.strip_prefix("heading")?;
    let level = suffix.parse::<u8>().ok()?;
    (1..=9).contains(&level).then_some(level)
}

/// Concatenates a paragraph's run text in the same coordinate space the parser
/// uses for bookmark offsets (field result text is diverted to the run's
/// `field.cached_text`, so `run.text` is empty for fields and does not shift
/// offsets).
pub(crate) fn paragraph_concatenated_text(paragraph: &Paragraph) -> String {
    paragraph.runs.iter().map(|run| run.text.as_str()).collect()
}

/// Produces the source heading text Word copies into a generated TOC entry.
/// Tabs and manual line breaks are flattened independently unless the field
/// carries `\w` and `\x`, respectively. Keeping this normalization in the
/// reader model also makes navigation outlines agree with regenerated fields.
fn toc_heading_text(paragraph: &Paragraph, options: &TocOptions) -> String {
    let mut text = String::new();
    let first_run = if options.omit_caption_label_and_number {
        options
            .caption_label
            .as_deref()
            .and_then(|label| {
                paragraph.runs.iter().position(|run| {
                    matches!(
                        run.field.as_ref().map(|field| &field.kind),
                        Some(RunFieldKind::Seq { identifier, .. })
                            if identifier.eq_ignore_ascii_case(label)
                    )
                })
            })
            .map_or(0, |seq_run| seq_run + 1)
    } else {
        0
    };
    for run in &paragraph.runs[first_run..] {
        text.push_str(&run.text);
        if let Some(field) = &run.field {
            // Field result text is intentionally excluded from `run.text` so
            // bookmark/selection offsets stay aligned with parsed text nodes.
            // Word copies the displayed result into a regenerated TOC entry,
            // preferring our structural value (REF/SEQ) over a stale cache.
            if let Some(value) = field
                .resolved_value
                .as_deref()
                .or(field.cached_text.as_deref())
            {
                text.push_str(value);
            }
        }
    }
    let text = if options.omit_caption_label_and_number {
        trim_caption_entry_prefix(&text).to_string()
    } else {
        text
    };
    text.chars()
        .map(|ch| match ch {
            '\t' if !options.preserve_tabs => ' ',
            '\n' if !options.preserve_line_breaks => ' ',
            other => other,
        })
        .collect()
}

fn trim_caption_entry_prefix(text: &str) -> &str {
    text.trim_start()
        .trim_start_matches([':', '：', '.', '．', ',', '，', '-', '–', '—'])
        .trim_start()
}

fn paragraph_has_seq_identifier(paragraph: &Paragraph, expected: &str) -> bool {
    paragraph.runs.iter().any(|run| {
        matches!(
            run.field.as_ref().map(|field| &field.kind),
            Some(RunFieldKind::Seq { identifier, .. })
                if identifier.eq_ignore_ascii_case(expected)
        )
    })
}

/// Slices `text` by byte range, clamping to bounds and refusing to split a
/// UTF-8 char boundary (returns `None` for an invalid boundary).
pub(crate) fn slice_byte_range(text: &str, start: usize, end: usize) -> Option<String> {
    let start = start.min(text.len());
    let end = end.min(text.len());
    if start >= end {
        return Some(String::new());
    }
    if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return None;
    }
    Some(text[start..end].to_string())
}

/// Slices plain-text offsets while retaining zero-width computed fields
/// enclosed by the bookmark's run boundaries.
fn bookmark_paragraph_text(
    paragraph: &Paragraph,
    start: usize,
    end: usize,
    start_run: usize,
    end_run: usize,
) -> Option<String> {
    let mut result = String::new();
    let mut offset = 0;
    for (index, run) in paragraph.runs.iter().enumerate() {
        let run_end = offset + run.text.len();
        let enclosed = index >= start_run && index < end_run;
        if let Some(value) = run
            .field
            .as_ref()
            .and_then(|field| field.resolved_value.as_deref())
        {
            if enclosed || (offset >= start && run_end <= end && offset < run_end) {
                result.push_str(value);
                offset = run_end;
                continue;
            }
        }
        let slice_start = start.saturating_sub(offset).min(run.text.len());
        let slice_end = end.saturating_sub(offset).min(run.text.len());
        if slice_start < slice_end {
            result.push_str(&slice_byte_range(&run.text, slice_start, slice_end)?);
        }
        offset = run_end;
    }
    Some(result)
}

/// Extracts bookmark display text; multi-paragraph ranges retain paragraph breaks.
fn extract_bookmark_text(body: &Body, bookmark: &Bookmark) -> Option<String> {
    if bookmark.start_paragraph > bookmark.end_paragraph
        || bookmark.end_paragraph >= body.elements.len()
    {
        return None;
    }
    let mut parts = Vec::new();
    for index in bookmark.start_paragraph..=bookmark.end_paragraph {
        let Some(paragraph) = body.elements.get(index).and_then(BodyElement::as_paragraph) else {
            continue;
        };
        parts.push(bookmark_paragraph_text(
            paragraph,
            if index == bookmark.start_paragraph {
                bookmark.start_offset
            } else {
                0
            },
            if index == bookmark.end_paragraph {
                bookmark.end_offset
            } else {
                usize::MAX
            },
            if index == bookmark.start_paragraph {
                bookmark.start_run
            } else {
                0
            },
            if index == bookmark.end_paragraph {
                bookmark.end_run
            } else {
                paragraph.runs.len()
            },
        )?);
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

#[derive(Default)]
struct FieldNumberingState {
    heading_generations: [u32; 9],
    auto_levels: [u32; 10],
    auto_heading_depth: usize,
    current_heading: Option<u8>,
}

fn resolve_body_fields(
    body: &mut Body,
    bookmarks: &[Bookmark],
    bookmark_text: &mut std::collections::HashMap<String, String>,
) {
    let mut seq_counters = std::collections::HashMap::new();
    let mut numbering = FieldNumberingState::default();
    let mut seq_heading_generations = std::collections::HashMap::new();
    let mut sequence_values_by_element = Vec::with_capacity(body.elements.len());
    for element in &mut body.elements {
        let mut sequence_values = Vec::new();
        resolve_fields_in_element(
            element,
            bookmark_text,
            &mut seq_counters,
            &mut numbering,
            &mut seq_heading_generations,
            &mut sequence_values,
            true,
        );
        sequence_values_by_element.push(sequence_values);
    }
    resolve_sequence_bookmark_fields(body, bookmarks, &sequence_values_by_element);
}

/// Resolves structural fields within a block element, recursing into table
/// cells so SET assignments and sequence counters advance in true document
/// order.
fn resolve_fields_in_element(
    element: &mut BodyElement,
    bookmark_text: &mut std::collections::HashMap<String, String>,
    seq_counters: &mut std::collections::HashMap<String, u32>,
    numbering: &mut FieldNumberingState,
    seq_heading_generations: &mut std::collections::HashMap<(String, u8), u32>,
    sequence_values: &mut Vec<ResolvedSequenceValue>,
    top_level: bool,
) {
    match element {
        BodyElement::Paragraph(paragraph) => {
            numbering.current_heading = paragraph
                .properties
                .as_ref()
                .and_then(|properties| properties.style.as_deref())
                .and_then(heading_style_level);
            if let Some(level) = numbering.current_heading {
                let generation = &mut numbering.heading_generations[usize::from(level - 1)];
                *generation = generation.saturating_add(1);
            }
            resolve_fields_in_paragraph(
                paragraph,
                bookmark_text,
                seq_counters,
                numbering,
                seq_heading_generations,
                sequence_values,
                top_level,
            );
        }
        BodyElement::Table(table) => {
            for row in &mut table.rows {
                for cell in &mut row.cells {
                    for block in &mut cell.blocks {
                        resolve_fields_in_element(
                            block,
                            bookmark_text,
                            seq_counters,
                            numbering,
                            seq_heading_generations,
                            sequence_values,
                            false,
                        );
                    }
                }
            }
        }
        BodyElement::Math(_) => {}
    }
}

/// Resolves structural and comparison fields on every run of a paragraph.
fn resolve_fields_in_paragraph(
    paragraph: &mut Paragraph,
    bookmark_text: &mut std::collections::HashMap<String, String>,
    seq_counters: &mut std::collections::HashMap<String, u32>,
    numbering: &mut FieldNumberingState,
    seq_heading_generations: &mut std::collections::HashMap<(String, u8), u32>,
    sequence_values: &mut Vec<ResolvedSequenceValue>,
    record_run_positions: bool,
) {
    let mut auto_number = None;
    for (run_index, run) in paragraph.runs.iter_mut().enumerate() {
        let language = run
            .properties
            .as_ref()
            .and_then(|properties| properties.language.as_deref());
        let Some(field) = run.field.as_mut() else {
            continue;
        };
        if matches!(
            &field.kind,
            RunFieldKind::Seq {
                bookmark: Some(_),
                ..
            }
        ) {
            // Bookmark operands are resolved in a second pass after ordinary
            // SEQ values are known. Clear a prior result so repeated resolver
            // runs cannot retain a target that was removed or renamed.
            field.resolved_value = None;
            continue;
        }
        let resolved = match &field.kind {
            RunFieldKind::Locked { kind } => {
                if matches!(
                    kind.base_kind(),
                    RunFieldKind::NonDisplay { .. }
                        | RunFieldKind::Ask { .. }
                        | RunFieldKind::Set { .. }
                ) {
                    Some(String::new())
                } else {
                    None
                }
            }
            RunFieldKind::Set { bookmark, value } => {
                // SET is evaluated in document order. Reassignment must affect
                // only fields that follow it, so update the shared bookmark
                // environment here instead of precomputing a final-value map.
                bookmark_text.insert(bookmark.clone(), value.clone());
                None
            }
            RunFieldKind::Ref {
                name,
                case_format,
                numbering: false,
                ..
            } => bookmark_text
                .get(name)
                .map(|value| apply_field_text_case(value, *case_format)),
            RunFieldKind::Ref {
                numbering: true, ..
            } => None,
            // QUOTE is self-contained: the instruction literal is the field
            // result, so it should replace a stale saved value on every load.
            RunFieldKind::Quote { text, case_format } => {
                Some(apply_field_text_case(text, *case_format))
            }
            RunFieldKind::MacroButton { display_text, .. } => Some(display_text.clone()),
            RunFieldKind::GoToButton { display_text, .. } => Some(display_text.clone()),
            RunFieldKind::Symbol {
                code_point,
                character_set,
                ..
            } => resolve_symbol_character(*code_point, *character_set),
            RunFieldKind::AutoNum {
                kind,
                separator,
                omit_trailing_period,
                number_format,
                case_format,
            } => {
                let number = *auto_number.get_or_insert_with(|| {
                    let index = numbering
                        .current_heading
                        .map_or(9, |level| usize::from(level - 1));
                    numbering.auto_levels[index] = numbering.auto_levels[index].saturating_add(1);
                    if index < 9 {
                        numbering.auto_levels[index + 1..].fill(0);
                        numbering.auto_heading_depth = index + 1;
                    }
                    numbering.auto_levels[index]
                });
                // Default outline heading formats are not yet corroborated.
                // Preserve their saved display while still advancing paragraph
                // counters; body and explicitly formatted fields are supported.
                if *kind == AutoNumberKind::Outline
                    && numbering.current_heading.is_some()
                    && number_format.is_none()
                {
                    field.resolved_value = None;
                    continue;
                }
                let mut display = String::new();
                if *kind == AutoNumberKind::Legal {
                    let depth = numbering
                        .current_heading
                        .map_or(numbering.auto_heading_depth, |level| usize::from(level - 1));
                    for &value in &numbering.auto_levels[..depth] {
                        if value > 0 {
                            display.push_str(&format_seq_number(value, *number_format, language));
                            display.push('.');
                        }
                    }
                }
                display.push_str(&format_seq_number(number, *number_format, language));
                if let Some(separator) = separator {
                    display.push_str(separator);
                } else if !omit_trailing_period && number_format.is_none() {
                    display.push('.');
                }
                Some(apply_field_text_case(&display, *case_format))
            }
            RunFieldKind::Seq {
                identifier,
                restart_at,
                repeat_previous,
                hide_result,
                restart_after_heading,
                number_format,
                case_format,
                ..
            } => {
                let key = identifier.to_ascii_lowercase();
                if let (false, Some(level)) = (*repeat_previous, *restart_after_heading) {
                    let current_generation = numbering.heading_generations[usize::from(level - 1)];
                    let generation_key = (key.clone(), level);
                    match seq_heading_generations.entry(generation_key) {
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(current_generation);
                            if current_generation > 0 {
                                seq_counters.insert(key.clone(), 0);
                            }
                        }
                        std::collections::hash_map::Entry::Occupied(mut entry)
                            if *entry.get() != current_generation =>
                        {
                            entry.insert(current_generation);
                            seq_counters.insert(key.clone(), 0);
                        }
                        std::collections::hash_map::Entry::Occupied(_) => {}
                    }
                }
                let counter_value = if *repeat_previous {
                    seq_counters.get(&key).copied()
                } else {
                    let counter = seq_counters.entry(key.clone()).or_insert(0);
                    *counter = restart_at.unwrap_or_else(|| counter.saturating_add(1));
                    Some(*counter)
                };
                let resolved = if *hide_result {
                    Some(String::new())
                } else {
                    counter_value.map(|counter| {
                        let display = format_seq_number(counter, *number_format, language);
                        apply_field_text_case(&display, *case_format)
                    })
                };
                if let (Some(number), Some(display)) = (counter_value, resolved.as_ref()) {
                    if !display.is_empty() {
                        sequence_values.push(ResolvedSequenceValue {
                            identifier: key,
                            number,
                            display: display.clone(),
                            run_index: record_run_positions.then_some(run_index),
                        });
                    }
                }
                resolved
            }
            RunFieldKind::Compare {
                left,
                left_is_quoted,
                operator,
                right,
                right_is_quoted,
            } => Some(
                if evaluate_field_comparison(
                    left,
                    *left_is_quoted,
                    *operator,
                    right,
                    *right_is_quoted,
                    bookmark_text,
                ) {
                    "1"
                } else {
                    "0"
                }
                .to_string(),
            ),
            RunFieldKind::If {
                left,
                left_is_quoted,
                operator,
                right,
                right_is_quoted,
                true_text,
                false_text,
            } => {
                let result = |mac| {
                    if evaluate_field_comparison_as(
                        left,
                        *left_is_quoted,
                        *operator,
                        right,
                        *right_is_quoted,
                        bookmark_text,
                        mac,
                    ) {
                        true_text.clone()
                    } else {
                        false_text.clone().unwrap_or_default()
                    }
                };
                let resolved = result(false);
                if field.word_mac_body_result.is_none() {
                    let mac = result(true);
                    field.word_mac_body_result = (mac != resolved).then(|| mac.into_boxed_str());
                }
                Some(resolved)
            }
            // PRIVATE/ADDIN store opaque data but do not display a field
            // result. Record an explicit empty resolution so structural
            // consumers cannot fall back to stale cached text.
            RunFieldKind::NonDisplay { .. } => Some(String::new()),
            _ => None,
        };
        if resolved.is_some() {
            field.resolved_value = resolved;
        }
    }
}

pub(crate) fn evaluate_field_comparison(
    left: &str,
    left_is_quoted: bool,
    operator: FieldComparisonOperator,
    right: &str,
    right_is_quoted: bool,
    bookmark_text: &std::collections::HashMap<String, String>,
) -> bool {
    evaluate_field_comparison_as(
        left,
        left_is_quoted,
        operator,
        right,
        right_is_quoted,
        bookmark_text,
        false,
    )
}

/// [`evaluate_field_comparison`] as Word for Mac evaluates it (`mac`): a
/// quoted Expression2 that is a number still compares numerically with a
/// numeric Expression1, and a wildcard has no 128-character limit
/// (`if-compare-field-edge-cases`: `IF 2 = "02"` prints "Numeric equal" and
/// a 129-character value matches `"*"`).
pub(crate) fn evaluate_field_comparison_as(
    left: &str,
    left_is_quoted: bool,
    operator: FieldComparisonOperator,
    right: &str,
    right_is_quoted: bool,
    bookmark_text: &std::collections::HashMap<String, String>,
    mac: bool,
) -> bool {
    // Quoted Expression1 is a literal string even when its text collides with
    // a bookmark name. Preserve that distinction after token normalization.
    let left = if left_is_quoted {
        left
    } else {
        bookmark_text.get(left).map(String::as_str).unwrap_or(left)
    }
    .trim();
    // A quoted Expression2 is a character string, not a bookmark reference.
    // Keep the literal even when its text happens to equal a bookmark name;
    // otherwise `IF Target = "Target"` would dereference both sides and turn
    // a false comparison into a true one.
    let right = if right_is_quoted {
        right
    } else {
        bookmark_text
            .get(right)
            .map(String::as_str)
            .unwrap_or(right)
    }
    .trim();

    // Word compares numeric expressions numerically. Only choose that path
    // when *both* sides parse, so a text value such as an account identifier
    // with a nonnumeric suffix stays a string comparison.
    if !right_is_quoted || mac {
        if let Some(result) = evaluate_numeric_field_comparison(left, operator, right) {
            return result;
        }
    }

    let left = left.to_lowercase();
    let right = right.to_lowercase();
    let ordering = left.cmp(&right);
    match operator {
        // Microsoft documents `?` and `*` specially for equality and
        // inequality. Apply the pattern only on Expression2, matching Word's
        // asymmetric field syntax.
        FieldComparisonOperator::Equal => word_field_wildcard_match(&left, &right, !mac),
        FieldComparisonOperator::NotEqual => !word_field_wildcard_match(&left, &right, !mac),
        FieldComparisonOperator::Greater => ordering.is_gt(),
        FieldComparisonOperator::Less => ordering.is_lt(),
        FieldComparisonOperator::GreaterOrEqual => ordering.is_ge(),
        FieldComparisonOperator::LessOrEqual => ordering.is_le(),
    }
}

/// Evaluates a comparison only when both operands are self-contained numeric
/// values. The parser uses this narrower form for a field nested inside another
/// field instruction: numeric literals can be refreshed immediately, whereas
/// bookmark names must wait for document-wide structural resolution.
pub(crate) fn evaluate_numeric_field_comparison(
    left: &str,
    operator: FieldComparisonOperator,
    right: &str,
) -> Option<bool> {
    let left = left.trim().parse::<f64>().ok()?;
    let right = right.trim().parse::<f64>().ok()?;
    Some(match operator {
        FieldComparisonOperator::Equal => left == right,
        FieldComparisonOperator::NotEqual => left != right,
        FieldComparisonOperator::Greater => left > right,
        FieldComparisonOperator::Less => left < right,
        FieldComparisonOperator::GreaterOrEqual => left >= right,
        FieldComparisonOperator::LessOrEqual => left <= right,
    })
}

fn word_field_wildcard_match(value: &str, pattern: &str, capped: bool) -> bool {
    let value: Vec<char> = value.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    // Word caps the portion of Expression1 corresponding to the first `*`
    // plus the remaining Expression2 pattern at 128 characters. Before the
    // first star, every literal and `?` consumes exactly one value character,
    // so the unmatched value suffix is precisely the bounded portion. Count
    // Unicode scalar values rather than UTF-8 bytes to avoid penalizing CJK.
    if let Some(first_asterisk) = pattern
        .iter()
        .position(|token| *token == '*')
        .filter(|_| capped)
    {
        let remaining_value_chars = value.len().saturating_sub(first_asterisk);
        if remaining_value_chars > 128 {
            return false;
        }
    }
    // A one-row dynamic program avoids regex escaping surprises and keeps the
    // exact Word field wildcard vocabulary intentionally limited to `?` and
    // `*`. `*` may match an empty sequence; `?` consumes exactly one scalar.
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;
    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if token == '*' {
            current[0] = previous[0];
        }
        for index in 1..=value.len() {
            current[index] = match token {
                '*' => previous[index] || current[index - 1],
                '?' => previous[index - 1],
                literal => previous[index - 1] && value[index - 1] == literal,
            };
        }
        previous = current;
    }
    previous[value.len()]
}

/// Container for the block-level elements that appear in the DOCX body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Body {
    pub elements: Vec<BodyElement>,
}

/// Represents the block-level content that can appear inside the document
/// body. WordprocessingML allows paragraphs and tables to exist at the same
/// hierarchy level, and tables can be nested within table cells. The enum is
/// therefore recursive by design.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BodyElement {
    Paragraph(Box<Paragraph>),
    Table(Box<Table>),
    Math(Box<MathParagraph>),
}

impl BodyElement {
    pub fn from_paragraph(paragraph: Paragraph) -> Self {
        Self::Paragraph(Box::new(paragraph))
    }

    pub fn from_table(table: Table) -> Self {
        Self::Table(Box::new(table))
    }

    pub fn from_math_paragraph(math: MathParagraph) -> Self {
        Self::Math(Box::new(math))
    }

    pub fn as_paragraph(&self) -> Option<&Paragraph> {
        match self {
            Self::Paragraph(paragraph) => Some(paragraph.as_ref()),
            _ => None,
        }
    }

    pub fn as_paragraph_mut(&mut self) -> Option<&mut Paragraph> {
        match self {
            Self::Paragraph(paragraph) => Some(paragraph.as_mut()),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Self::Table(table) => Some(table.as_ref()),
            _ => None,
        }
    }

    pub fn as_math_paragraph(&self) -> Option<&MathParagraph> {
        match self {
            Self::Math(math) => Some(math.as_ref()),
            _ => None,
        }
    }

    pub fn as_table_mut(&mut self) -> Option<&mut Table> {
        match self {
            Self::Table(table) => Some(table.as_mut()),
            _ => None,
        }
    }

    pub fn as_math_paragraph_mut(&mut self) -> Option<&mut MathParagraph> {
        match self {
            Self::Math(math) => Some(math.as_mut()),
            _ => None,
        }
    }
}

/// Represents a Word paragraph, including its inline runs and paragraph level
/// properties such as spacing, indentation, and numbering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub properties: Option<ParagraphProperties>,
    #[serde(skip_serializing, skip_deserializing)]
    pub node_id: Option<XmlNodeId>,
    #[serde(default)]
    pub comment_ranges: Vec<CommentRange>,
    #[serde(default)]
    pub comment_markers: Vec<CommentMarker>,
}

/// Inline rule occupying the footnote/endnote separator story's line box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteSeparatorKind {
    Separator,
    ContinuationSeparator,
}

/// A single run of content with optional styling information.
///
/// Runs typically contain plain text but can also embed images. When the
/// `image` field is present the textual payload is usually empty; renderers
/// must therefore inspect the `image` value before assuming that `text`
/// contains user-visible characters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub properties: Option<RunProperties>,
    pub image: Option<RunImage>,
    pub text_box: Option<RunTextBox>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub math: Option<MathRun>,
    pub field: Option<RunField>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hyperlink: Option<RunHyperlink>,
    #[serde(skip_serializing, skip_deserializing)]
    pub node_id: Option<XmlNodeId>,
    #[serde(skip_serializing, skip_deserializing)]
    pub text_node_id: Option<XmlNodeId>,
    #[serde(skip_serializing, skip_deserializing)]
    pub revision_node_id: Option<XmlNodeId>,
    #[serde(default)]
    pub comment_reference_ids: Vec<String>,
    #[serde(default)]
    pub footnote_reference_ids: Vec<String>,
    #[serde(default)]
    pub endnote_reference_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub note_separator: Option<NoteSeparatorKind>,
    #[serde(default)]
    pub has_footnote_ref: bool,
    #[serde(default)]
    pub has_endnote_ref: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub revision: Option<RevisionMark>,
}

impl Run {
    /// Whether this run's glyph metrics must be excluded from paragraph line
    /// height. Word's `SYMBOL \h` switch is intentionally visual-only: a large
    /// symbol may overlap neighbouring leading instead of expanding the line.
    pub fn suppresses_line_height_contribution(&self) -> bool {
        matches!(
            self.field.as_ref().map(|field| field.kind.base_kind()),
            Some(RunFieldKind::Symbol {
                preserve_line_height: true,
                ..
            })
        )
    }
}

/// Placeholder that represents a page number field when converting headers or
/// footers into plain-text representations.
pub const PAGE_FIELD_PLACEHOLDER: &str = "{PAGE}";

/// Placeholder that represents a `NUMPAGES` (total page count) field when
/// converting headers or footers into plain-text representations. See
/// `RunFieldKind::NumPages`.
pub const PAGE_COUNT_FIELD_PLACEHOLDER: &str = "{NUMPAGES}";

/// Placeholder that represents a `SECTIONPAGES` field while header/footer
/// content is flattened into positioned plain-text segments. The renderer
/// substitutes the page count of the physical section containing that header
/// or footer.
pub const SECTION_PAGE_COUNT_FIELD_PLACEHOLDER: &str = "{SECTIONPAGES}";

/// Placeholder for the 1-based current section number while header/footer
/// stories are flattened before their physical section is known.
pub const SECTION_NUMBER_FIELD_PLACEHOLDER: &str = "{SECTION}";

/// 制御文字として扱う手動改ページ。内部では `\u{000C}` を利用し、
/// `<w:br w:type="page"/>` と相互変換される。
pub const PAGE_BREAK_CHAR: char = '\u{000C}';

/// 文字列版の手動改ページ。テストや文字列操作向けに用意する。
pub const PAGE_BREAK_STR: &str = "\u{000C}";

/// 段区切り (`<w:br w:type="column"/>`, ECMA-376 Part 1 §17.3.3.1): the
/// text continues at the top of the next column, or of the next page after
/// the last column. Word's own text model uses the same control character.
pub const COLUMN_BREAK_CHAR: char = '\u{000E}';

/// [`COLUMN_BREAK_CHAR`] as a string.
pub const COLUMN_BREAK_STR: &str = "\u{000E}";

/// Metadata describing a hyperlink applied to a run.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunHyperlink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// Browser frame/window name requested by Word (`w:tgtFrame` or the
    /// HYPERLINK field's `\t` switch). Kept separately from relationship
    /// target mode: one identifies the URI kind, the other the browsing
    /// context used to activate it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_frame: Option<String>,
}

/// Field metadata embedded in a run. WordprocessingML often represents page
/// numbers and similar dynamic values as field instructions rather than plain
/// text. The renderer resolves these fields at draw time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunField {
    pub kind: RunFieldKind,
    /// Original simple-field wrapper identity retained by split fragments. This prevents
    /// a newly inserted field from joining a neighbouring simple-field wrapper.
    #[serde(skip)]
    pub source_field_id: Option<XmlNodeId>,
    /// Word's last-computed result text for a computed field (the run children
    /// placed between `fldChar` `separate` and `end`, or the `fldSimple`
    /// children). Editable HYPERLINK/MERGEFIELD/FILLIN and bookmark-backed
    /// REF/PAGEREF results remain in `Run::text` instead so normal text
    /// selection and wrapper preservation keep working.
    /// Preserved so the renderer can show the cached value when it cannot
    /// recompute the field (e.g. a `PAGEREF` whose bookmark page is not yet
    /// laid out). `PAGE`/`NUMPAGES`/`SECTIONPAGES` ignore it once live page
    /// geometry is available because they recompute at draw time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_text: Option<String>,
    /// Host-resolved field value computed from document structure at load time
    /// rather than at draw time. Used for `REF` (bookmark text) and `SEQ`
    /// (sequence number), which depend only on document order/content and not on
    /// the laid-out page geometry. Takes precedence over `cached_text` when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_value: Option<String>,
    /// The result Word for Mac prints for this IF or COMPARE in the main
    /// story, when it differs from the evaluated one: there Word takes a
    /// nested COMPARE or IF at its saved result instead of evaluating it
    /// (`if-compare-field-edge-cases`), while a nested REF stays live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_mac_body_result: Option<Box<str>>,
}

/// Supported field kinds. Additional variants can be added as new field types
/// are required.
///
/// Spec references (ECMA-376 Part 1, Part 11 — WordprocessingML fields at
/// <https://ooxml.info/docs/11/>):
/// - `PAGE`: §17.16.5.42 — the 1-based page number of the current page.
/// - `NUMPAGES`: §17.16.5.37 — the total number of pages in the document.
/// - `EDITTIME`: the package's total editing time, expressed in minutes.
/// - `SECTIONPAGES`: the total number of pages in the current section.
/// - `PAGEREF`: §17.16.5.45 — the page number on which the referenced
///   bookmark begins.
/// - `HYPERLINK`: §17.16.5.31 — an external target and/or local bookmark.
/// - `EQ`: §17.16.5.22 — equation field instructions, including `\o\ad`
///   overstrike arrays used by Word's Japanese distributed text command.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DocumentDateSource {
    Created,
    Saved,
    Printed,
}

/// Capitalization requested by Word's general `\*` field-result switch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FieldTextCase {
    Upper,
    Lower,
    FirstCap,
    Caps,
}

/// Legacy automatic paragraph field family.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AutoNumberKind {
    Plain,
    Legal,
    Outline,
}

/// Numeric presentation requested by a SEQ field's general `\*` switch.
/// Word uses the switch value's spelling case to choose uppercase or lowercase
/// Roman numerals and alphabetic counters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SeqNumberFormat {
    Arabic,
    UpperRoman,
    LowerRoman,
    UpperAlphabetic,
    LowerAlphabetic,
    Hex,
    Ordinal,
    CardText,
    OrdText,
    DollarText,
}

fn format_seq_number(
    value: u32,
    format: Option<SeqNumberFormat>,
    language: Option<&str>,
) -> String {
    // Roman numerals and alphabetic counters have no representation for zero.
    // Keep Word's numeric restart value visible instead of underflowing the
    // shared alphabetic formatter on a tolerated `\r 0` instruction.
    if value == 0 {
        return value.to_string();
    }
    match format {
        None | Some(SeqNumberFormat::Arabic) => value.to_string(),
        Some(SeqNumberFormat::UpperRoman) => to_roman(value, true),
        Some(SeqNumberFormat::LowerRoman) => to_roman(value, false),
        Some(SeqNumberFormat::UpperAlphabetic) => to_letter(value, true),
        Some(SeqNumberFormat::LowerAlphabetic) => to_letter(value, false),
        Some(SeqNumberFormat::Hex) => to_hex(value),
        Some(SeqNumberFormat::Ordinal) if language.is_some_and(is_english_language) => {
            to_english_ordinal(value)
        }
        Some(SeqNumberFormat::CardText) if language.is_some_and(is_english_language) => {
            to_english_cardinal(value)
        }
        Some(SeqNumberFormat::OrdText) if language.is_some_and(is_english_language) => {
            to_english_ordinal_text(value)
        }
        Some(SeqNumberFormat::DollarText) if language.is_some_and(is_english_language) => {
            // SEQ counters are integers, so DollarText's rounded two-digit
            // fractional component is always zero hundredths.
            format!("{} and 00/100", to_english_cardinal(value))
        }
        // These formats are language-dependent. Preserve the numeric value
        // when no supported result language is declared instead of silently
        // manufacturing English output for an unrelated locale.
        Some(
            SeqNumberFormat::Ordinal
            | SeqNumberFormat::CardText
            | SeqNumberFormat::OrdText
            | SeqNumberFormat::DollarText,
        ) => value.to_string(),
    }
}

pub fn apply_field_text_case(value: &str, format: Option<FieldTextCase>) -> String {
    match format {
        None => value.to_string(),
        Some(FieldTextCase::Upper) => value.to_uppercase(),
        Some(FieldTextCase::Lower) => value.to_lowercase(),
        Some(FieldTextCase::FirstCap) => {
            let mut capitalized = false;
            value
                .chars()
                .flat_map(|character| {
                    if !capitalized && character.is_alphabetic() {
                        capitalized = true;
                        character.to_uppercase().collect::<Vec<_>>()
                    } else {
                        vec![character]
                    }
                })
                .collect()
        }
        Some(FieldTextCase::Caps) => {
            let mut starts_word = true;
            value
                .chars()
                .flat_map(|character| {
                    if character.is_alphanumeric() {
                        let mapped = if starts_word {
                            character.to_uppercase().collect::<Vec<_>>()
                        } else {
                            vec![character]
                        };
                        starts_word = false;
                        mapped
                    } else {
                        starts_word = true;
                        vec![character]
                    }
                })
                .collect()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunFieldKind {
    /// A field whose saved result is frozen by `w:fldLock`. The shared wrapper
    /// preserves its semantic kind without enlarging the inline RunField/Run
    /// storage; ordinary fields do not allocate a wrapper.
    Locked {
        kind: Rc<RunFieldKind>,
    },
    PageNumber,
    /// Total number of pages in the document (`NUMPAGES` field). Commonly
    /// paired with `PAGE` in headers/footers to produce "X / Y" style page
    /// indicators.
    NumPages,
    /// Number of words saved in the package extended properties (`NUMWORDS`).
    /// Word updates this statistic when document properties/fields are refreshed.
    NumWords,
    /// Number of characters excluding spaces saved in the package extended
    /// properties (`NUMCHARS`).
    NumChars,
    /// Total editing time in minutes saved in the package extended properties
    /// (`EDITTIME`). Word persists the authoritative value as `TotalTime` in
    /// `docProps/app.xml`; a saved field result can lag behind that metadata.
    EditTime,
    /// Size of the compressed document package (`FILESIZE`). The unit switch
    /// is retained so the loader can resolve the original DOCX byte length
    /// without reparsing instruction text in each story.
    FileSize {
        unit: FileSizeUnit,
    },
    /// Total number of pages in the section containing the field
    /// (`SECTIONPAGES`). Common in section-specific headers and footers.
    SectionPages,
    /// 1-based number of the section containing the field (`SECTION`).
    SectionNumber,
    /// The text of the nearest paragraph in a style (`STYLEREF "name"`,
    /// §17.16.5.66): in a header or footer, the first such paragraph on the
    /// page (the last with `\l`), or else the nearest one before it — the
    /// running chapter title of a dictionary-style header.
    StyleRef {
        /// The style as the instruction names it (`heading 1`, `見出し 1`,
        /// or a style id).
        style: String,
        /// `\l`: the last paragraph on the page rather than the first.
        from_bottom: bool,
    },
    /// Value of a package document property (`DOCPROPERTY name`). The property
    /// name is retained exactly as written in the instruction; package lookup
    /// is ASCII case-insensitive and occurs after all OPC metadata parts load.
    DocumentProperty {
        name: String,
        case_format: Option<FieldTextCase>,
    },
    /// Value of a document variable declared in `settings.xml`
    /// (`DOCVARIABLE name`). Word stores these values under
    /// `w:settings/w:docVars`; the field keeps the instruction spelling so the
    /// package lookup can happen after the settings part has loaded.
    DocumentVariable {
        name: String,
        case_format: Option<FieldTextCase>,
    },
    /// Mail-merge column (`MERGEFIELD name`). The field name is modeled for
    /// round-trip/inspection, while Word's displayed result remains editable
    /// `Run::text` until a mail-merge data source is available to recompute it.
    MergeField {
        name: String,
    },
    /// Interactive response field (`FILLIN ["Prompt"] [\d "Default"] [\o]`).
    /// Jade preserves the saved response as editable `Run::text`; prompting
    /// for a replacement remains a host/UI responsibility.
    FillIn {
        prompt: Option<String>,
        default: Option<String>,
        ask_once: bool,
    },
    /// Bookmark-assignment prompt
    /// (`ASK Bookmark "Prompt" [\d "Default"] [\o]`). ASK itself is hidden;
    /// Word displays the assigned response through a separate REF field.
    Ask {
        bookmark: String,
        prompt: Option<String>,
        default: Option<String>,
        ask_once: bool,
    },
    /// Page number of a named bookmark (`PAGEREF bookmarkName [\h]`). The
    /// `name` is the bookmark target; the renderer resolves it to a page number
    /// using the document's bookmark table and the laid-out page positions.
    /// `hyperlink` retains Word's `\h` switch so the result remains an
    /// interactive jump to the bookmark.
    PageRef {
        name: String,
        #[serde(default)]
        hyperlink: bool,
    },
    /// Number of a bookmarked footnote/endnote reference (`NOTEREF`).
    /// Relative-position results retain Word's saved, localized text.
    NoteRef {
        name: String,
        #[serde(default)]
        hyperlink: bool,
        #[serde(default)]
        formatted: bool,
        #[serde(default)]
        relative_position: bool,
    },
    /// Current date (`DATE` field, §17.16.5.16). The optional `format` is the
    /// `\@` date-picture switch (e.g. `yyyy/MM/dd`); when absent a locale
    /// default is used. The renderer formats the host clock at display time.
    Date {
        format: Option<String>,
    },
    /// Package-backed document timestamp (`CREATEDATE`, `SAVEDATE`, or
    /// `PRINTDATE`). It must not be evaluated against the host clock.
    DocumentDate {
        source: DocumentDateSource,
        format: Option<String>,
    },
    /// Current time (`TIME` field, §17.16.5.65). Like `DATE` but defaults to a
    /// time picture (e.g. `H:mm`) when no `\@` switch is present.
    Time {
        format: Option<String>,
    },
    /// Cross-reference to a bookmark's text (`REF bookmarkName`, §17.16.5.52).
    /// The renderer resolves `name` to the run text spanned by the bookmark and
    /// substitutes it for the field result, falling back to `cached_text`.
    Ref {
        name: String,
        #[serde(default)]
        case_format: Option<FieldTextCase>,
        /// Whether a paragraph-number switch (`\\n`, `\\r`, or `\\w`) is set.
        /// These results require numbering context, so the cached Word result is
        /// retained until Jade can evaluate that context faithfully.
        #[serde(default)]
        numbering: bool,
        /// Whether Word's `\h` switch makes the result an internal hyperlink.
        #[serde(default)]
        hyperlink: bool,
    },
    /// Literal text insertion (`QUOTE "LiteralText"`, §17.16.5.51). The
    /// quoted operand is authoritative and can safely be recomputed without
    /// package metadata, pagination, or host interaction.
    Quote {
        text: String,
        case_format: Option<FieldTextCase>,
    },
    /// Character insertion (`SYMBOL value [\a|\j|\u] [\f font] [\s points]
    /// [\h]`, §17.16.5.68). Formatting operands are retained separately from
    /// the saved result so readers do not need to parse instruction text again.
    Symbol {
        code_point: u32,
        character_set: SymbolCharacterSet,
        font: Option<String>,
        font_size_points: Option<u32>,
        preserve_line_height: bool,
    },
    /// Invisible bookmark assignment (`SET Bookmark "Text"`, §17.16.5.54).
    /// Later REF/IF fields consume the value in document order; SET has no
    /// display result at its own position.
    Set {
        bookmark: String,
        value: String,
    },
    /// Legacy automatic paragraph numbering. AUTONUM and AUTONUMLGL share
    /// document-order counters; legal numbering includes preceding heading levels.
    AutoNum {
        kind: AutoNumberKind,
        separator: Option<String>,
        omit_trailing_period: bool,
        number_format: Option<SeqNumberFormat>,
        case_format: Option<FieldTextCase>,
    },
    /// Sequence counter (`SEQ identifier`, §17.16.5.64). Each occurrence of a
    /// `SEQ` with the same `identifier` increments a counter and displays its
    /// value (used for figure/table numbering). `restart_at` is Word's `\r n`
    /// switch: that field displays `n` and subsequent fields continue at
    /// `n + 1`. `repeat_previous` is Word's `\c` switch and reuses the nearest
    /// preceding value without advancing the counter. `hide_result` models
    /// `\h`: the counter still changes while the displayed result is empty.
    /// `restart_after_heading` stores `\s level`, which restarts the sequence
    /// whenever a new matching built-in Heading style is encountered.
    /// `bookmark` is Word's optional second operand and references the nearest
    /// matching SEQ field without advancing this counter. The renderer numbers
    /// fields by document order. `number_format` retains the supported numeric
    /// `\*` result format independently from the counter state; `case_format`
    /// is applied to that formatted display afterwards.
    Seq {
        identifier: String,
        #[serde(default)]
        restart_at: Option<u32>,
        #[serde(default)]
        repeat_previous: bool,
        #[serde(default)]
        hide_result: bool,
        #[serde(default)]
        restart_after_heading: Option<u8>,
        #[serde(default)]
        bookmark: Option<String>,
        #[serde(default)]
        number_format: Option<SeqNumberFormat>,
        #[serde(default)]
        case_format: Option<FieldTextCase>,
    },
    /// Logical comparison (`COMPARE Expression1 Operator Expression2`). Word
    /// displays `1` for true and `0` for false.
    Compare {
        left: String,
        #[serde(default)]
        left_is_quoted: bool,
        operator: FieldComparisonOperator,
        right: String,
        #[serde(default)]
        right_is_quoted: bool,
    },
    /// Conditional text (`IF Expression1 Operator Expression2 TrueText
    /// [FalseText]`). An omitted false branch resolves to an empty string.
    If {
        left: String,
        #[serde(default)]
        left_is_quoted: bool,
        operator: FieldComparisonOperator,
        right: String,
        #[serde(default)]
        right_is_quoted: bool,
        true_text: String,
        false_text: Option<String>,
    },
    /// Clickable link expressed as a `HYPERLINK` field rather than a
    /// `w:hyperlink` wrapper (§17.16.5.31). Word commonly emits this form for
    /// links with tooltips, local bookmark targets, or legacy documents.
    Hyperlink {
        target: Option<String>,
        anchor: Option<String>,
        tooltip: Option<String>,
        target_frame: Option<String>,
    },
    /// Legacy macro button (`MACROBUTTON MacroName DisplayText`). The macro
    /// name is retained for inspection/round-trip only: loading or rendering a
    /// document never executes it. The self-contained display text is safe to
    /// recompute instead of trusting a stale saved field result.
    MacroButton {
        macro_name: String,
        display_text: String,
    },
    /// Legacy navigation button (`GOTOBUTTON Destination DisplayText`). The
    /// destination is retained exactly for inspection/round-trip. Plain
    /// bookmark destinations can use the normal internal hyperlink path;
    /// Word's special page/section/line/note destinations remain modeled but
    /// inert until matching navigation support exists.
    GoToButton {
        destination: String,
        display_text: String,
    },
    /// Equation-field distributed text (`EQ \o\ad(text, guide)`, §17.16.5.22).
    /// Word emits this legacy field shape for Japanese equal distribution in
    /// compatibility documents. `text` is the visible string; `guide` is
    /// measured with the same run style to derive the fitted advance.
    EqDistributed {
        text: String,
        guide: String,
    },
    /// Word's 囲い文字: an `EQ` overstrike of an enclosure and a character
    /// (`EQ \o\ac(○,秘)`, §17.16.5.22), drawn one over the other.
    EqEnclosed {
        /// The enclosed character.
        text: String,
        /// The enclosure: ○, □, △ or ◇.
        enclosure: String,
    },
    /// Ruby written as an `EQ` overstrike (`EQ \* jc2 \* hps11
    /// \o\ad(\s\up11(guide),text)`, §17.16.5.22): `text` is the base and
    /// `guide` the raised phonetic guide. Word lays it out like `w:ruby`.
    EqRuby {
        text: String,
        guide: String,
        /// `w:rubyAlign` token, lowercased, from `\* jcN` or the overstrike
        /// alignment switch.
        align: Option<String>,
        /// `\* hpsN`: the guide size in half-points.
        size_half_points: Option<u32>,
        /// `\s\upN`: the guide raise in whole points.
        raise_points: u32,
    },
    /// A Word field whose instruction Jade cannot safely evaluate, either
    /// because the field kind is not implemented or because required operands
    /// or external state are unavailable. Keeping the normalized instruction
    /// distinguishes Word's cached result from ordinary editable text and lets
    /// callers surface an explicit compatibility/readiness warning without
    /// discarding content. Display continues to use [`RunField::cached_text`].
    Unsupported {
        instruction: String,
    },
    /// A Word field that stores data but has no visible result. PRIVATE is a
    /// cold field and ADDIN is hidden add-in storage; both retain their
    /// normalized instruction and cached bytes for inspection/round-trip while
    /// renderers deliberately suppress any stale result text.
    NonDisplay {
        instruction: String,
    },
}

impl RunFieldKind {
    /// The ruby an `EQ` overstrike field draws, as a `w:ruby` annotation.
    pub fn eq_ruby_annotation(&self) -> Option<RubyAnnotation> {
        let RunFieldKind::EqRuby {
            guide,
            align,
            size_half_points,
            raise_points,
            ..
        } = self
        else {
            return None;
        };
        Some(RubyAnnotation {
            text: guide.clone(),
            size_pt: size_half_points.map(|half_points| half_points as f32 / 2.0),
            align: align.clone(),
            raise_half_points: Some(raise_points * 2),
            ..RubyAnnotation::default()
        })
    }

    pub fn with_lock(self, locked: bool) -> Self {
        match (locked, self) {
            (true, kind @ Self::Locked { .. }) => kind,
            (true, kind) => Self::Locked {
                kind: Rc::new(kind),
            },
            (false, Self::Locked { kind }) => Rc::unwrap_or_clone(kind).with_lock(false),
            (false, kind) => kind,
        }
    }

    pub fn is_locked(&self) -> bool {
        matches!(self, Self::Locked { .. })
    }

    /// Original semantic kind, for formatting/inspection rather than refresh.
    pub fn base_kind(&self) -> &Self {
        let mut kind = self;
        while let Self::Locked { kind: inner } = kind {
            kind = inner;
        }
        kind
    }

    /// Whether a top-level field of this kind keeps its result as editable
    /// `Run::text` (HYPERLINK, MERGEFIELD, FILLIN, PAGEREF, NOTEREF, and REF
    /// without a paragraph-number switch).
    /// Every other kind is computed or hidden: its result lives in
    /// `RunField::cached_text`/`resolved_value`, is zero-width in paragraph
    /// offsets, and is replaced whenever Word updates fields.
    pub fn keeps_result_in_run_text(&self) -> bool {
        matches!(
            self.base_kind(),
            RunFieldKind::Hyperlink { .. }
                | RunFieldKind::MergeField { .. }
                | RunFieldKind::FillIn { .. }
                | RunFieldKind::Ref {
                    numbering: false,
                    ..
                }
                | RunFieldKind::PageRef { .. }
                | RunFieldKind::NoteRef { .. }
        )
    }
}

/// Display unit requested by a Word `FILESIZE` field.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FileSizeUnit {
    Bytes,
    Kilobytes,
    Megabytes,
}

/// Code-page interpretation requested by a Word `SYMBOL` field switch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SymbolCharacterSet {
    Default,
    Ansi,
    ShiftJis,
    Unicode,
}

fn resolve_symbol_character(code_point: u32, character_set: SymbolCharacterSet) -> Option<String> {
    match character_set {
        // Without a code-page switch Word treats ordinary scalar values as the
        // requested character.
        SymbolCharacterSet::Default => {
            char::from_u32(code_point).map(|character| character.to_string())
        }
        // Word truncates SYMBOL \u operands outside Unicode plane 0 to their
        // low 16 bits. Applying that documented Office deviation keeps Jade
        // from drawing a supplementary-plane glyph Word itself will not show.
        SymbolCharacterSet::Unicode => {
            char::from_u32(code_point & 0xffff).map(|character| character.to_string())
        }
        // ASCII is invariant within the Windows ANSI family. Values above it
        // depend on the document/system code page, which is not represented by
        // the field instruction, so keep Word's cached result for those cases.
        SymbolCharacterSet::Ansi if code_point <= 0x7f => {
            char::from_u32(code_point).map(|character| character.to_string())
        }
        SymbolCharacterSet::ShiftJis => resolve_shift_jis_symbol(code_point),
        // Non-ASCII ANSI values depend on the document/system code page, which
        // the field instruction does not identify. Keep Word's saved result.
        SymbolCharacterSet::Ansi => None,
    }
}

fn resolve_shift_jis_symbol(value: u32) -> Option<String> {
    let bytes = if value <= u8::MAX as u32 {
        vec![value as u8]
    } else if value <= u16::MAX as u32 {
        // Word writes the numeric argument in the same order as the Shift-JIS
        // lead and trail bytes (for example 0x82A0 = HIRAGANA LETTER A).
        vec![(value >> 8) as u8, value as u8]
    } else {
        return None;
    };
    let decoded =
        encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(&bytes)?;
    // A SYMBOL field represents one character. Reject malformed or unexpected
    // multi-character decodes so the renderer can retain the saved Word result.
    (decoded.chars().count() == 1).then(|| decoded.into_owned())
}

/// Comparison operators shared by Word's `COMPARE` and `IF` fields.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FieldComparisonOperator {
    Equal,
    NotEqual,
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
}

/// Metadata for an image that appears inside a run.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RunImage {
    /// Relationship identifier (`rId`) that links the image to a media part.
    pub relationship_id: Option<String>,
    /// Relationship to an embedded OLE package (`o:OLEObject/@r:id`). This is
    /// distinct from the VML preview image relationship above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ole_relationship_id: Option<String>,
    /// OLE program identifier such as `Equation.3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ole_program_id: Option<String>,
    /// Image width in CSS pixels derived from the WordprocessingML extent.
    pub width_px: Option<f64>,
    /// Image height in CSS pixels derived from the WordprocessingML extent.
    pub height_px: Option<f64>,
    /// Optional title (often used as tooltip text in Word).
    pub title: Option<String>,
    /// Optional alternative text/description for accessibility.
    pub description: Option<String>,
    /// Optional label shown when the image bytes cannot be decoded (e.g. WMF/EMF placeholder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder_label: Option<String>,
    /// Eagerly resolved data URI that can be fed directly to DOM APIs.
    pub data_uri: Option<String>,
    /// Native DrawingML chart data when this drawing references a
    /// `word/charts/chart*.xml` part instead of bitmap media. Charts reuse the
    /// image layout box for pagination, positioning, selection, and resizing,
    /// but remain editable Word charts on export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<RunChart>,
    /// Editable subset of a WordprocessingGroup (`wpg:wgp`). Child geometry
    /// remains in the group's authored coordinate space and is mapped into
    /// this image's anchor extent by the renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_group: Option<RunShapeGroup>,
    /// SmartArt data-model relationship (`dgm:relIds/@r:dm`). The document
    /// loader follows it to Word's cached `dsp:drawing` and fills
    /// [`Self::shape_group`] with that pre-rendered shape tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagram_data_relationship_id: Option<String>,
    /// Source-image crop edges from DrawingML `a:srcRect`, expressed as
    /// fractions of the intrinsic bitmap dimensions (`0.15` means 15%).
    /// Negative values expand the image outward beyond its destination edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<ImageCrop>,
    /// Layout information describing whether the image participates in text
    /// flow or floats at an absolute position.
    pub layout: RunImageLayout,
}

/// Coordinate system and supported child shapes from a DrawingML group.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunShapeGroup {
    /// Position and extent inside the parent group's authored coordinate
    /// space. Root `wpg:wgp` values are retained but its drawing anchor remains
    /// authoritative when rendered.
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    /// Paint order within the parent group.
    #[serde(default)]
    pub z_order: usize,
    pub child_offset_x: f64,
    pub child_offset_y: f64,
    pub child_extent_width: f64,
    pub child_extent_height: f64,
    /// Clockwise rotation authored on the group's `a:xfrm`.
    #[serde(default)]
    pub rotation_deg: f64,
    #[serde(default)]
    pub flip_horizontal: bool,
    #[serde(default)]
    pub flip_vertical: bool,
    pub children: Vec<RunGroupShape>,
    /// Nested DrawingML groups (`wpg:grpSp`) in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<RunShapeGroup>,
}

/// A preset child shape within a WordprocessingGroup.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunGroupShape {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Paint order within the containing group.
    #[serde(default)]
    pub z_order: usize,
    /// Clockwise rotation authored on this child's `a:xfrm`.
    #[serde(default)]
    pub rotation_deg: f64,
    #[serde(default)]
    pub flip_horizontal: bool,
    #[serde(default)]
    pub flip_vertical: bool,
    pub shape: TextBoxShape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    /// Opacity of the solid fill, normalized from DrawingML's `a:alpha`.
    /// `None` means the OOXML default (fully opaque).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_opacity: Option<f64>,
    /// DrawingML theme slot used by this child's fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_theme_color: Option<String>,
    /// Ordered transforms nested below the fill's `a:schemeClr`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fill_theme_transforms: Vec<ThemeColorTransform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<TextBoxBorder>,
    /// Shape-level outer shadow authored in DrawingML `a:effectLst`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShapeShadow>,
    /// Shape-level inner shadow authored in DrawingML `a:effectLst`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_shadow: Option<ShapeShadow>,
    /// Shape-level glow authored in DrawingML `a:effectLst`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<ShapeGlow>,
    /// Radius of DrawingML's shape-level `a:blur`, in CSS pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur_radius_px: Option<f64>,
    /// Radius of DrawingML's `a:softEdge`, in CSS pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_edge_radius_px: Option<f64>,
    /// Click action authored on the child's DrawingML non-visual properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hyperlink: Option<RunHyperlink>,
    /// Plain text contained by this grouped WordprocessingShape.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Inner text margins authored on the child's `wps:bodyPr`.
    #[serde(default)]
    pub insets: TextBoxInsets,
    /// Vertical placement of the child shape's text body.
    #[serde(default)]
    pub vertical_alignment: TextBoxVerticalAlignment,
    /// Character formatting of a DrawingML text body (`a:txBody`), such as
    /// the text of a SmartArt drawing. WordprocessingShape children keep
    /// `None` and use the text-box defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_style: Option<GroupShapeTextStyle>,
}

/// Uniform formatting for a grouped shape's DrawingML text body. Taken from
/// the first run (`a:rPr`) and paragraph (`a:pPr@algn`) of the body, with the
/// shape style's `a:fontRef` color as fallback.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupShapeTextStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size_pt: Option<f32>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    /// Resolved `#RRGGBB` text color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub theme_transforms: Vec<ThemeColorTransform>,
    /// Paragraph alignment as a WordprocessingML `w:jc` value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alignment: Option<String>,
}

/// The three non-AI chart kinds exposed by GenOffice's Insert Chart dialog.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ChartKind {
    #[default]
    Bar,
    Line,
    Pie,
}

/// One named chart series. A missing value is a genuine chart gap and must not
/// be coerced to zero during render or ChartML round-trip.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<Option<f64>>,
}

/// Editable cached data for a native Word chart part.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunChart {
    pub kind: ChartKind,
    pub title: String,
    pub categories: Vec<String>,
    pub series: Vec<ChartSeries>,
}

/// Fractions removed from each edge of an image before it is fitted into the
/// destination rectangle. DrawingML stores these values in 1/1000 percent;
/// negative fractions represent outward expansion rather than cropping.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct ImageCrop {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// Preset geometry used by a WordprocessingShape. Word stores these values in
/// DrawingML `a:prstGeom@prst`; keep the common editable shapes explicit so the
/// renderer and exporter do not silently flatten every object to a rectangle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum TextBoxShape {
    #[default]
    Rectangle,
    Line,
    LineArrow,
    LineArrowDouble,
    LineBent,
    LineCurved,
    RoundedRectangle,
    Ellipse,
    Diamond,
    Triangle,
    RightTriangle,
    Parallelogram,
    Trapezoid,
    Pentagon,
    Hexagon,
    Octagon,
    Plus,
    MathPlus,
    LeftBrace,
    RightBrace,
    BlockArc,
    Pie,
    Chord,
    Teardrop,
    Frame,
    HalfFrame,
    Corner,
    #[serde(rename = "diagStripe")]
    DiagonalStripe,
    Donut,
    NoSmoking,
    FoldedCorner,
    Bevel,
    Cube,
    Can,
    LightningBolt,
    Heart,
    Sun,
    Moon,
    Cloud,
    Arc,
    Plaque,
    SmileyFace,
    LeftBracket,
    RightBracket,
    FourPointStar,
    FivePointStar,
    SixPointStar,
    SevenPointStar,
    EightPointStar,
    TenPointStar,
    TwelvePointStar,
    SixteenPointStar,
    TwentyFourPointStar,
    ThirtyTwoPointStar,
    IrregularSealOne,
    IrregularSealTwo,
    Ribbon,
    #[serde(rename = "ribbon2", alias = "ribbonTwo")]
    RibbonTwo,
    Wave,
    DoubleWave,
    FlowChartProcess,
    FlowChartAlternateProcess,
    FlowChartDecision,
    FlowChartPredefinedProcess,
    FlowChartInternalStorage,
    FlowChartDocument,
    FlowChartMultidocument,
    FlowChartTerminator,
    FlowChartPreparation,
    FlowChartManualInput,
    FlowChartManualOperation,
    FlowChartConnector,
    FlowChartOffpageConnector,
    FlowChartMagneticDisk,
    FlowChartMagneticDrum,
    FlowChartDisplay,
    FlowChartDelay,
    FlowChartOr,
    FlowChartCollate,
    FlowChartSort,
    FlowChartExtract,
    FlowChartMerge,
    FlowChartSummingJunction,
    FlowChartPunchedTape,
    #[serde(rename = "wedgeRectCallout")]
    WedgeRectangleCallout,
    #[serde(rename = "wedgeRoundRectCallout")]
    WedgeRoundedRectangleCallout,
    WedgeEllipseCallout,
    CloudCallout,
    RightArrow,
    LeftArrow,
    UpArrow,
    DownArrow,
    LeftRightArrow,
    UpDownArrow,
    NotchedRightArrow,
    Chevron,
    HomePlate,
    QuadArrow,
    BentArrow,
    UturnArrow,
    CurvedRightArrow,
    StripedRightArrow,
    SnipOneRectangle,
    SnipTwoSameRectangle,
    SnipTwoDiagonalRectangle,
    SnipRoundRectangle,
    RoundOneRectangle,
    RoundTwoSameRectangle,
    RoundTwoDiagonalRectangle,
}

impl TextBoxShape {
    /// Every preset whose default geometry Jade traces and re-exports without
    /// silently substituting a rectangle.
    pub const EXACT_PRESETS: [Self; 109] = [
        Self::Rectangle,
        Self::Line,
        Self::LineArrow,
        Self::LineArrowDouble,
        Self::LineBent,
        Self::LineCurved,
        Self::RoundedRectangle,
        Self::Ellipse,
        Self::Diamond,
        Self::Triangle,
        Self::RightTriangle,
        Self::Parallelogram,
        Self::Trapezoid,
        Self::Pentagon,
        Self::Hexagon,
        Self::Octagon,
        Self::Plus,
        Self::MathPlus,
        Self::LeftBrace,
        Self::RightBrace,
        Self::BlockArc,
        Self::Pie,
        Self::Chord,
        Self::Teardrop,
        Self::Frame,
        Self::HalfFrame,
        Self::Corner,
        Self::DiagonalStripe,
        Self::Donut,
        Self::NoSmoking,
        Self::FoldedCorner,
        Self::Bevel,
        Self::Cube,
        Self::Can,
        Self::LightningBolt,
        Self::Heart,
        Self::Sun,
        Self::Moon,
        Self::Cloud,
        Self::Arc,
        Self::Plaque,
        Self::SmileyFace,
        Self::LeftBracket,
        Self::RightBracket,
        Self::FourPointStar,
        Self::FivePointStar,
        Self::SixPointStar,
        Self::SevenPointStar,
        Self::EightPointStar,
        Self::TenPointStar,
        Self::TwelvePointStar,
        Self::SixteenPointStar,
        Self::TwentyFourPointStar,
        Self::ThirtyTwoPointStar,
        Self::IrregularSealOne,
        Self::IrregularSealTwo,
        Self::Ribbon,
        Self::RibbonTwo,
        Self::Wave,
        Self::DoubleWave,
        Self::FlowChartProcess,
        Self::FlowChartAlternateProcess,
        Self::FlowChartDecision,
        Self::FlowChartPredefinedProcess,
        Self::FlowChartInternalStorage,
        Self::FlowChartDocument,
        Self::FlowChartMultidocument,
        Self::FlowChartTerminator,
        Self::FlowChartPreparation,
        Self::FlowChartManualInput,
        Self::FlowChartManualOperation,
        Self::FlowChartConnector,
        Self::FlowChartOffpageConnector,
        Self::FlowChartMagneticDisk,
        Self::FlowChartMagneticDrum,
        Self::FlowChartDisplay,
        Self::FlowChartDelay,
        Self::FlowChartOr,
        Self::FlowChartCollate,
        Self::FlowChartSort,
        Self::FlowChartExtract,
        Self::FlowChartMerge,
        Self::FlowChartSummingJunction,
        Self::FlowChartPunchedTape,
        Self::WedgeRectangleCallout,
        Self::WedgeRoundedRectangleCallout,
        Self::WedgeEllipseCallout,
        Self::CloudCallout,
        Self::RightArrow,
        Self::LeftArrow,
        Self::UpArrow,
        Self::DownArrow,
        Self::LeftRightArrow,
        Self::UpDownArrow,
        Self::NotchedRightArrow,
        Self::Chevron,
        Self::HomePlate,
        Self::QuadArrow,
        Self::BentArrow,
        Self::UturnArrow,
        Self::CurvedRightArrow,
        Self::StripedRightArrow,
        Self::SnipOneRectangle,
        Self::SnipTwoSameRectangle,
        Self::SnipTwoDiagonalRectangle,
        Self::SnipRoundRectangle,
        Self::RoundOneRectangle,
        Self::RoundTwoSameRectangle,
        Self::RoundTwoDiagonalRectangle,
    ];

    /// DrawingML `a:prstGeom@prst` token.
    pub const fn preset_name(self) -> &'static str {
        match self {
            Self::Rectangle => "rect",
            Self::Line => "line",
            Self::LineArrow | Self::LineArrowDouble => "straightConnector1",
            Self::LineBent => "bentConnector3",
            Self::LineCurved => "curvedConnector3",
            Self::RoundedRectangle => "roundRect",
            Self::Ellipse => "ellipse",
            Self::Diamond => "diamond",
            Self::Triangle => "triangle",
            Self::RightTriangle => "rtTriangle",
            Self::Parallelogram => "parallelogram",
            Self::Trapezoid => "trapezoid",
            Self::Pentagon => "pentagon",
            Self::Hexagon => "hexagon",
            Self::Octagon => "octagon",
            Self::Plus => "plus",
            Self::MathPlus => "mathPlus",
            Self::LeftBrace => "leftBrace",
            Self::RightBrace => "rightBrace",
            Self::BlockArc => "blockArc",
            Self::Pie => "pie",
            Self::Chord => "chord",
            Self::Teardrop => "teardrop",
            Self::Frame => "frame",
            Self::HalfFrame => "halfFrame",
            Self::Corner => "corner",
            Self::DiagonalStripe => "diagStripe",
            Self::Donut => "donut",
            Self::NoSmoking => "noSmoking",
            Self::FoldedCorner => "foldedCorner",
            Self::Bevel => "bevel",
            Self::Cube => "cube",
            Self::Can => "can",
            Self::LightningBolt => "lightningBolt",
            Self::Heart => "heart",
            Self::Sun => "sun",
            Self::Moon => "moon",
            Self::Cloud => "cloud",
            Self::Arc => "arc",
            Self::Plaque => "plaque",
            Self::SmileyFace => "smileyFace",
            Self::LeftBracket => "leftBracket",
            Self::RightBracket => "rightBracket",
            Self::FourPointStar => "star4",
            Self::FivePointStar => "star5",
            Self::SixPointStar => "star6",
            Self::SevenPointStar => "star7",
            Self::EightPointStar => "star8",
            Self::TenPointStar => "star10",
            Self::TwelvePointStar => "star12",
            Self::SixteenPointStar => "star16",
            Self::TwentyFourPointStar => "star24",
            Self::ThirtyTwoPointStar => "star32",
            Self::IrregularSealOne => "irregularSeal1",
            Self::IrregularSealTwo => "irregularSeal2",
            Self::Ribbon => "ribbon",
            Self::RibbonTwo => "ribbon2",
            Self::Wave => "wave",
            Self::DoubleWave => "doubleWave",
            Self::FlowChartProcess => "flowChartProcess",
            Self::FlowChartAlternateProcess => "flowChartAlternateProcess",
            Self::FlowChartDecision => "flowChartDecision",
            Self::FlowChartPredefinedProcess => "flowChartPredefinedProcess",
            Self::FlowChartInternalStorage => "flowChartInternalStorage",
            Self::FlowChartDocument => "flowChartDocument",
            Self::FlowChartMultidocument => "flowChartMultidocument",
            Self::FlowChartTerminator => "flowChartTerminator",
            Self::FlowChartPreparation => "flowChartPreparation",
            Self::FlowChartManualInput => "flowChartManualInput",
            Self::FlowChartManualOperation => "flowChartManualOperation",
            Self::FlowChartConnector => "flowChartConnector",
            Self::FlowChartOffpageConnector => "flowChartOffpageConnector",
            Self::FlowChartMagneticDisk => "flowChartMagneticDisk",
            Self::FlowChartMagneticDrum => "flowChartMagneticDrum",
            Self::FlowChartDisplay => "flowChartDisplay",
            Self::FlowChartDelay => "flowChartDelay",
            Self::FlowChartOr => "flowChartOr",
            Self::FlowChartCollate => "flowChartCollate",
            Self::FlowChartSort => "flowChartSort",
            Self::FlowChartExtract => "flowChartExtract",
            Self::FlowChartMerge => "flowChartMerge",
            Self::FlowChartSummingJunction => "flowChartSummingJunction",
            Self::FlowChartPunchedTape => "flowChartPunchedTape",
            Self::WedgeRectangleCallout => "wedgeRectCallout",
            Self::WedgeRoundedRectangleCallout => "wedgeRoundRectCallout",
            Self::WedgeEllipseCallout => "wedgeEllipseCallout",
            Self::CloudCallout => "cloudCallout",
            Self::RightArrow => "rightArrow",
            Self::LeftArrow => "leftArrow",
            Self::UpArrow => "upArrow",
            Self::DownArrow => "downArrow",
            Self::LeftRightArrow => "leftRightArrow",
            Self::UpDownArrow => "upDownArrow",
            Self::NotchedRightArrow => "notchedRightArrow",
            Self::Chevron => "chevron",
            Self::HomePlate => "homePlate",
            Self::QuadArrow => "quadArrow",
            Self::BentArrow => "bentArrow",
            Self::UturnArrow => "uturnArrow",
            Self::CurvedRightArrow => "curvedRightArrow",
            Self::StripedRightArrow => "stripedRightArrow",
            Self::SnipOneRectangle => "snip1Rect",
            Self::SnipTwoSameRectangle => "snip2SameRect",
            Self::SnipTwoDiagonalRectangle => "snip2DiagRect",
            Self::SnipRoundRectangle => "snipRoundRect",
            Self::RoundOneRectangle => "round1Rect",
            Self::RoundTwoSameRectangle => "round2SameRect",
            Self::RoundTwoDiagonalRectangle => "round2DiagRect",
        }
    }

    pub fn from_preset_name(value: &str) -> Option<Self> {
        // Arrow-end elements disambiguate the three straight connector
        // variants after parsing. Use the plain line here as the safe base.
        match value {
            "straightConnector1" => return Some(Self::Line),
            "bentConnector2" | "bentConnector3" | "bentConnector4" => return Some(Self::LineBent),
            "curvedConnector2" | "curvedConnector3" | "curvedConnector4" => {
                return Some(Self::LineCurved)
            }
            _ => {}
        }
        Self::EXACT_PRESETS
            .into_iter()
            .find(|shape| shape.preset_name() == value)
    }

    /// Public camelCase name accepted by the browser API.
    pub fn from_public_name(value: &str) -> Option<Self> {
        match value {
            "rectangle" => Some(Self::Rectangle),
            "line" => Some(Self::Line),
            "lineArrow" => Some(Self::LineArrow),
            "lineArrowDouble" => Some(Self::LineArrowDouble),
            "lineBent" => Some(Self::LineBent),
            "lineCurved" => Some(Self::LineCurved),
            "roundedRectangle" => Some(Self::RoundedRectangle),
            "ellipse" => Some(Self::Ellipse),
            "diamond" => Some(Self::Diamond),
            "triangle" => Some(Self::Triangle),
            "rightTriangle" => Some(Self::RightTriangle),
            "parallelogram" => Some(Self::Parallelogram),
            "trapezoid" => Some(Self::Trapezoid),
            "pentagon" => Some(Self::Pentagon),
            "hexagon" => Some(Self::Hexagon),
            "octagon" => Some(Self::Octagon),
            "plus" => Some(Self::Plus),
            "mathPlus" => Some(Self::MathPlus),
            "leftBrace" => Some(Self::LeftBrace),
            "rightBrace" => Some(Self::RightBrace),
            "blockArc" => Some(Self::BlockArc),
            "pie" => Some(Self::Pie),
            "chord" => Some(Self::Chord),
            "teardrop" => Some(Self::Teardrop),
            "frame" => Some(Self::Frame),
            "halfFrame" => Some(Self::HalfFrame),
            "corner" => Some(Self::Corner),
            "diagStripe" => Some(Self::DiagonalStripe),
            "donut" => Some(Self::Donut),
            "noSmoking" => Some(Self::NoSmoking),
            "foldedCorner" => Some(Self::FoldedCorner),
            "bevel" => Some(Self::Bevel),
            "cube" => Some(Self::Cube),
            "can" => Some(Self::Can),
            "lightningBolt" => Some(Self::LightningBolt),
            "heart" => Some(Self::Heart),
            "sun" => Some(Self::Sun),
            "moon" => Some(Self::Moon),
            "cloud" => Some(Self::Cloud),
            "arc" => Some(Self::Arc),
            "plaque" => Some(Self::Plaque),
            "smileyFace" => Some(Self::SmileyFace),
            "leftBracket" => Some(Self::LeftBracket),
            "rightBracket" => Some(Self::RightBracket),
            "fourPointStar" => Some(Self::FourPointStar),
            "fivePointStar" => Some(Self::FivePointStar),
            "sixPointStar" => Some(Self::SixPointStar),
            "sevenPointStar" => Some(Self::SevenPointStar),
            "eightPointStar" => Some(Self::EightPointStar),
            "tenPointStar" => Some(Self::TenPointStar),
            "twelvePointStar" => Some(Self::TwelvePointStar),
            "sixteenPointStar" => Some(Self::SixteenPointStar),
            "twentyFourPointStar" => Some(Self::TwentyFourPointStar),
            "thirtyTwoPointStar" => Some(Self::ThirtyTwoPointStar),
            "irregularSealOne" => Some(Self::IrregularSealOne),
            "irregularSealTwo" => Some(Self::IrregularSealTwo),
            "ribbon" => Some(Self::Ribbon),
            "ribbon2" => Some(Self::RibbonTwo),
            "wave" => Some(Self::Wave),
            "doubleWave" => Some(Self::DoubleWave),
            "flowChartProcess" => Some(Self::FlowChartProcess),
            "flowChartAlternateProcess" => Some(Self::FlowChartAlternateProcess),
            "flowChartDecision" => Some(Self::FlowChartDecision),
            "flowChartPredefinedProcess" => Some(Self::FlowChartPredefinedProcess),
            "flowChartInternalStorage" => Some(Self::FlowChartInternalStorage),
            "flowChartDocument" => Some(Self::FlowChartDocument),
            "flowChartMultidocument" => Some(Self::FlowChartMultidocument),
            "flowChartTerminator" => Some(Self::FlowChartTerminator),
            "flowChartPreparation" => Some(Self::FlowChartPreparation),
            "flowChartManualInput" => Some(Self::FlowChartManualInput),
            "flowChartManualOperation" => Some(Self::FlowChartManualOperation),
            "flowChartConnector" => Some(Self::FlowChartConnector),
            "flowChartOffpageConnector" => Some(Self::FlowChartOffpageConnector),
            "flowChartMagneticDisk" => Some(Self::FlowChartMagneticDisk),
            "flowChartMagneticDrum" => Some(Self::FlowChartMagneticDrum),
            "flowChartDisplay" => Some(Self::FlowChartDisplay),
            "flowChartDelay" => Some(Self::FlowChartDelay),
            "flowChartOr" => Some(Self::FlowChartOr),
            "flowChartCollate" => Some(Self::FlowChartCollate),
            "flowChartSort" => Some(Self::FlowChartSort),
            "flowChartExtract" => Some(Self::FlowChartExtract),
            "flowChartMerge" => Some(Self::FlowChartMerge),
            "flowChartSummingJunction" => Some(Self::FlowChartSummingJunction),
            "flowChartPunchedTape" => Some(Self::FlowChartPunchedTape),
            "wedgeRectCallout" => Some(Self::WedgeRectangleCallout),
            "wedgeRoundRectCallout" => Some(Self::WedgeRoundedRectangleCallout),
            "wedgeEllipseCallout" => Some(Self::WedgeEllipseCallout),
            "cloudCallout" => Some(Self::CloudCallout),
            "rightArrow" => Some(Self::RightArrow),
            "leftArrow" => Some(Self::LeftArrow),
            "upArrow" => Some(Self::UpArrow),
            "downArrow" => Some(Self::DownArrow),
            "leftRightArrow" => Some(Self::LeftRightArrow),
            "upDownArrow" => Some(Self::UpDownArrow),
            "notchedRightArrow" => Some(Self::NotchedRightArrow),
            "chevron" => Some(Self::Chevron),
            "homePlate" => Some(Self::HomePlate),
            "quadArrow" => Some(Self::QuadArrow),
            "bentArrow" => Some(Self::BentArrow),
            "uturnArrow" => Some(Self::UturnArrow),
            "curvedRightArrow" => Some(Self::CurvedRightArrow),
            "stripedRightArrow" => Some(Self::StripedRightArrow),
            "snipOneRectangle" => Some(Self::SnipOneRectangle),
            "snipTwoSameRectangle" => Some(Self::SnipTwoSameRectangle),
            "snipTwoDiagonalRectangle" => Some(Self::SnipTwoDiagonalRectangle),
            "snipRoundRectangle" => Some(Self::SnipRoundRectangle),
            "roundOneRectangle" => Some(Self::RoundOneRectangle),
            "roundTwoSameRectangle" => Some(Self::RoundTwoSameRectangle),
            "roundTwoDiagonalRectangle" => Some(Self::RoundTwoDiagonalRectangle),
            _ => None,
        }
    }

    pub const fn is_line(self) -> bool {
        matches!(
            self,
            Self::Line
                | Self::LineArrow
                | Self::LineArrowDouble
                | Self::LineBent
                | Self::LineCurved
        )
    }

    /// Presets whose default GenOffice/Word silhouette is an open stroke.
    /// They remain ordinary editable shapes (unlike connectors), but the
    /// canvas must not close their final subpath with an artificial chord.
    pub const fn is_open_outline(self) -> bool {
        matches!(
            self,
            Self::Arc | Self::LeftBracket | Self::RightBracket | Self::LeftBrace | Self::RightBrace
        )
    }

    pub const fn has_head_arrow(self) -> bool {
        matches!(self, Self::LineArrowDouble)
    }

    pub const fn has_tail_arrow(self) -> bool {
        matches!(self, Self::LineArrow | Self::LineArrowDouble)
    }
}

/// Ordered DrawingML transforms applied to a colour. Values are stored
/// as normalized fractions (`60000` in OOXML becomes `0.6`) so package-level
/// theme resolution can reproduce Word's colour without retaining XML units.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ThemeColorTransform {
    LuminanceMod(f64),
    LuminanceOffset(f64),
    /// Mix the input color with white, retaining this fraction of the input.
    Tint(f64),
    /// Mix the input color with black, retaining this fraction of the input.
    Shade(f64),
}

/// A DrawingML shape-level outer or inner shadow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShapeShadow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub theme_transforms: Vec<ThemeColorTransform>,
    /// Opacity normalized to `0.0..=1.0` from DrawingML's `a:alpha` value.
    #[serde(default = "default_shape_shadow_opacity")]
    pub opacity: f64,
    pub blur_radius_px: f64,
    pub distance_px: f64,
    /// Clockwise direction where zero degrees points right.
    pub direction_deg: f64,
}

const fn default_shape_shadow_opacity() -> f64 {
    1.0
}

impl Default for ShapeShadow {
    fn default() -> Self {
        Self {
            color: None,
            theme_color: None,
            theme_transforms: Vec::new(),
            opacity: default_shape_shadow_opacity(),
            blur_radius_px: 0.0,
            distance_px: 0.0,
            direction_deg: 0.0,
        }
    }
}

/// A DrawingML shape-level glow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShapeGlow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub theme_transforms: Vec<ThemeColorTransform>,
    /// Opacity normalized to `0.0..=1.0` from DrawingML's `a:alpha` value.
    #[serde(default = "default_shape_glow_opacity")]
    pub opacity: f64,
    pub radius_px: f64,
}

const fn default_shape_glow_opacity() -> f64 {
    1.0
}

impl Default for ShapeGlow {
    fn default() -> Self {
        Self {
            color: None,
            theme_color: None,
            theme_transforms: Vec::new(),
            opacity: default_shape_glow_opacity(),
            radius_px: 0.0,
        }
    }
}

/// Describes a shape or text box embedded inside a drawing run.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunTextBox {
    /// Width of the text box in CSS pixels, if specified.
    pub width_px: Option<f64>,
    /// Height of the text box in CSS pixels, if specified.
    pub height_px: Option<f64>,
    /// Aggregated plain-text representation of the text box content.
    pub text: String,
    /// Layout metadata describing anchoring behaviour and offsets.
    pub layout: RunImageLayout,
    /// Parsed block elements (paragraphs/tables) contained within the text box.
    pub blocks: Vec<BodyElement>,
    #[serde(default)]
    pub border: TextBoxBorder,
    /// Shape-level outer shadow authored in DrawingML `a:effectLst`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShapeShadow>,
    /// Shape-level inner shadow authored in DrawingML `a:effectLst`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_shadow: Option<ShapeShadow>,
    /// Shape-level glow authored in DrawingML `a:effectLst`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<ShapeGlow>,
    /// Radius of DrawingML's shape-level `a:blur`, in CSS pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur_radius_px: Option<f64>,
    /// Radius of DrawingML's `a:softEdge`, in CSS pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_edge_radius_px: Option<f64>,
    /// Shape fill colour for the text box. `None` represents an explicit or
    /// unresolved no-fill/transparent shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    /// Opacity of the solid fill, normalized from DrawingML's `a:alpha`.
    /// `None` means the OOXML default (fully opaque).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_opacity: Option<f64>,
    /// DrawingML theme slot (for example `accent1`) when the fill is expressed
    /// as `a:schemeClr`. Package loading resolves it after theme1.xml is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_theme_color: Option<String>,
    /// Ordered transforms nested below the fill's `a:schemeClr`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fill_theme_transforms: Vec<ThemeColorTransform>,
    /// Inner text margins from DrawingML `bodyPr` or VML `textbox@inset`.
    #[serde(default)]
    pub insets: TextBoxInsets,
    /// Vertical placement of the text body inside the shape.
    #[serde(default)]
    pub vertical_alignment: TextBoxVerticalAlignment,
    /// DrawingML preset geometry. The historical model called all
    /// WordprocessingShapes text boxes; retaining geometry here also lets it
    /// represent shapes that do not contain `wps:txbx`.
    #[serde(default)]
    pub shape: TextBoxShape,
}

/// Text box inner margins in CSS pixels. The defaults mirror Word's standard
/// 0.1 inch horizontal and 0.05 inch vertical text-box insets.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextBoxInsets {
    pub left_px: f64,
    pub top_px: f64,
    pub right_px: f64,
    pub bottom_px: f64,
}

impl Default for TextBoxInsets {
    fn default() -> Self {
        Self {
            left_px: 9.6,
            top_px: 4.8,
            right_px: 9.6,
            bottom_px: 4.8,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TextBoxVerticalAlignment {
    #[default]
    Top,
    Center,
    Bottom,
    /// Keep the first and last lines at the text body's edges and distribute
    /// the remaining height between lines.
    Justified,
    /// Distribute remaining height around every line, including the outer
    /// edges of the text body.
    Distributed,
}

/// Tracks revision metadata (e.g. change tracking) attached to a run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RevisionMark {
    pub kind: RevisionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_utc: Option<String>,
    /// For deleted text: the tracked insertion it belongs to
    /// (`<w:ins><w:del>…</w:del></w:ins>`), as Word writes when someone
    /// deletes text that is still an insertion. Rejecting the deletion
    /// brings the insertion back; rejecting the insertion removes the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insertion: Option<Box<RevisionMark>>,
}

impl RevisionMark {
    /// Whether this mark removes its text (a deletion or a move away).
    pub fn is_deletion(&self) -> bool {
        matches!(self.kind, RevisionKind::Deleted | RevisionKind::MovedFrom)
    }

    /// The insertion layer of a run carrying this mark: the mark itself for
    /// an insertion (or a move here), the insertion a deletion sits in.
    pub fn insertion_layer(&self) -> Option<&RevisionMark> {
        if self.is_deletion() {
            self.insertion.as_deref()
        } else {
            Some(self)
        }
    }
}

/// Enumerates the supported revision kinds emitted by Word.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RevisionKind {
    Inserted,
    Deleted,
    /// Content moved away from this location (`<w:moveFrom>`). Rendered like a
    /// deletion but tracked separately so move pairs can be distinguished.
    MovedFrom,
    /// Content moved into this location (`<w:moveTo>`). Rendered like an
    /// insertion but tracked separately. ECMA-376 Part 1 §17.13.5.20/25.
    MovedTo,
}

/// Metadata about the border drawn around a floating text box. Values are
/// populated from DrawingML (`a:ln`) or VML (`v:stroke`) definitions when
/// present in the source DOCX.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TextBoxBorder {
    /// Border width in CSS pixels after unit conversion. When absent we fall
    /// back to a renderer-defined default.
    pub width_px: Option<f64>,
    /// Hex colour code (e.g. `#000000`) preserving author intent.
    pub color: Option<String>,
    /// Opacity normalized to `0.0..=1.0` from DrawingML's `a:alpha` value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// DrawingML theme slot used by the outline's `a:schemeClr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<String>,
    /// Ordered transforms nested below the outline's `a:schemeClr`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub theme_transforms: Vec<ThemeColorTransform>,
    /// Simplified representation of the line preset/dash style.
    #[serde(default)]
    pub style: TextBoxBorderStyle,
    /// Marker attached to the connector's start point (`a:headEnd`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_end: Option<TextBoxLineEnd>,
    /// Marker attached to the connector's end point (`a:tailEnd`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_end: Option<TextBoxLineEnd>,
}

/// DrawingML connector end marker. Keep this separate from `TextBoxShape` so
/// head-only connectors and non-triangle markers survive load/edit/export.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TextBoxLineEnd {
    Arrow,
    Triangle,
    Stealth,
    Diamond,
    Oval,
}

impl TextBoxLineEnd {
    pub fn from_drawingml(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "arrow" => Some(Self::Arrow),
            "triangle" => Some(Self::Triangle),
            "stealth" => Some(Self::Stealth),
            "diamond" => Some(Self::Diamond),
            "oval" => Some(Self::Oval),
            _ => None,
        }
    }

    pub fn drawingml_name(self) -> &'static str {
        match self {
            Self::Arrow => "arrow",
            Self::Triangle => "triangle",
            Self::Stealth => "stealth",
            Self::Diamond => "diamond",
            Self::Oval => "oval",
        }
    }
}

/// Supported border styles for floating text boxes. The variants mirror the
/// presets emitted by Microsoft Word (`a:prstDash`) as well as the legacy VML
/// `dashstyle` values.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum TextBoxBorderStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
    DashDotDot,
    Double,
    None,
}

impl TextBoxBorderStyle {
    pub fn from_preset_dash(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "solid" => TextBoxBorderStyle::Solid,
            "dash" | "sysdash" | "lgdash" | "smdash" => TextBoxBorderStyle::Dashed,
            "dot" | "sysdot" | "lgdot" | "smdot" => TextBoxBorderStyle::Dotted,
            "dashdot" | "lgdashdot" | "smdashdot" | "sysdashdot" => TextBoxBorderStyle::DashDot,
            "dashdotdot" | "lgdashdotdot" | "smdashdotdot" | "sysdashdotdot" => {
                TextBoxBorderStyle::DashDotDot
            }
            _ => TextBoxBorderStyle::Solid,
        }
    }

    /// Maps the VML `dashstyle` attribute (used in legacy `w:pict` fallbacks)
    /// onto our simplified style enumeration.
    pub fn from_vml_dashstyle(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "dash" | "dashsmall" | "dashlarge" => TextBoxBorderStyle::Dashed,
            "dot" | "shortdot" | "shortdotdot" => TextBoxBorderStyle::Dotted,
            "dashdot" | "shortdashdot" | "dashdotdot" | "shortdashdotdot" => {
                if value.to_ascii_lowercase().contains("dotdot") {
                    TextBoxBorderStyle::DashDotDot
                } else {
                    TextBoxBorderStyle::DashDot
                }
            }
            "solid" | "single" => TextBoxBorderStyle::Solid,
            "dbl" | "double" => TextBoxBorderStyle::Double,
            "none" => TextBoxBorderStyle::None,
            _ => TextBoxBorderStyle::Solid,
        }
    }

    /// Returns a human-readable identifier for the style, used by the debug
    /// APIs and JavaScript bindings.
    pub fn as_str(&self) -> &'static str {
        match self {
            TextBoxBorderStyle::Solid => "solid",
            TextBoxBorderStyle::Dashed => "dashed",
            TextBoxBorderStyle::Dotted => "dotted",
            TextBoxBorderStyle::DashDot => "dash-dot",
            TextBoxBorderStyle::DashDotDot => "dash-dot-dot",
            TextBoxBorderStyle::Double => "double",
            TextBoxBorderStyle::None => "none",
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn locked_field_metadata_keeps_existing_inline_run_sizes_and_shares_clones() {
        use super::{Run, RunField, RunFieldKind};
        assert_eq!(std::mem::size_of::<RunFieldKind>(), 104);
        // 16 bytes of it: the result Word for Mac prints for a body IF
        // (`RunField::word_mac_body_result`).
        assert_eq!(std::mem::size_of::<RunField>(), 176);
        // 8 bytes of it: the insertion a tracked deletion sits in
        // (`RevisionMark::insertion`); 24 the boxed theme colour, East Asian
        // layout and text effects the フォント dialog sets.
        assert_eq!(std::mem::size_of::<Run>(), 3056);
        let kind = RunFieldKind::NumPages.with_lock(true);
        let clone = kind.clone();
        match (&kind, &clone) {
            (RunFieldKind::Locked { kind: a }, RunFieldKind::Locked { kind: b }) => {
                assert!(std::rc::Rc::ptr_eq(a, b))
            }
            _ => panic!("lock wrapper missing"),
        }
        assert_eq!(kind.base_kind(), &RunFieldKind::NumPages);
        assert_eq!(clone.with_lock(false), RunFieldKind::NumPages);
        let serialized = serde_json::to_string(&kind).unwrap();
        assert_eq!(
            serde_json::from_str::<RunFieldKind>(&serialized).unwrap(),
            kind
        );
    }

    use super::BorderEdge;
    use super::Indent;
    use super::ParagraphBorders;
    use super::Shading;
    use super::TextBoxBorderStyle;
    use super::{
        apply_field_text_case, heading_style_level, parse_outline_range, parse_toc_entry_field,
        parse_toc_switches, FieldTextCase,
    };
    use proptest::prelude::*;

    #[test]
    fn field_text_case_matches_word_capitalization_switches() {
        assert_eq!(
            apply_field_text_case("Straße report", Some(FieldTextCase::Upper)),
            "STRASSE REPORT"
        );
        assert_eq!(
            apply_field_text_case("MIXED Case", Some(FieldTextCase::Lower)),
            "mixed case"
        );
        assert_eq!(
            apply_field_text_case("  élan VITAL", Some(FieldTextCase::FirstCap)),
            "  Élan VITAL"
        );
        assert_eq!(
            apply_field_text_case("alpha-beta \"gamma\"", Some(FieldTextCase::Caps)),
            "Alpha-Beta \"Gamma\""
        );
    }

    #[test]
    fn parse_toc_switches_reads_outline_range_and_flags() {
        let opts = parse_toc_switches(r#" TOC \o "1-3" \h \z \u \d ":" \s "Chapter" "#);
        assert_eq!(opts.outline_min, 1);
        assert_eq!(opts.outline_max, 3);
        assert!(opts.hyperlinks);
        assert!(opts.use_applied_outline_levels);
        assert!(!opts.hide_page_numbers);
        let sequence = opts
            .sequence_page_number
            .as_ref()
            .expect("sequence-prefixed page numbers");
        assert_eq!(sequence.identifier, "Chapter");
        assert_eq!(sequence.separator.as_deref(), Some(":"));
    }

    #[test]
    fn parse_toc_switches_defaults_and_hide_page_numbers() {
        // Bare `\o` keeps the full 1-9 range; `\n` hides page numbers.
        let opts = parse_toc_switches(r#"TOC \o \n"#);
        assert_eq!((opts.outline_min, opts.outline_max), (1, 9));
        assert!(opts.hide_page_numbers);
        assert_eq!(opts.hide_page_number_levels, None);
        assert!(!opts.hyperlinks);
        // A switch-only string (no leading TOC token) parses the same.
        let switch_only = parse_toc_switches(r#"\o "2-2""#);
        assert_eq!((switch_only.outline_min, switch_only.outline_max), (2, 2));

        let ranged = parse_toc_switches(r#"TOC \n "2-3""#);
        assert!(!ranged.hide_page_numbers);
        assert_eq!(ranged.hide_page_number_levels, Some((2, 3)));
        assert!(!ranged.hides_page_number_at_level(1));
        assert!(ranged.hides_page_number_at_level(2));
        assert!(ranged.hides_page_number_at_level(3));

        let custom = parse_toc_switches(r#"TOC \t "AppendixTitle,1,Custom Section,3""#);
        assert_eq!(
            custom.custom_style_levels,
            vec![
                ("AppendixTitle".to_string(), 1),
                ("Custom Section".to_string(), 3)
            ]
        );

        let separator = parse_toc_switches(r#"TOC \p "---""#);
        assert_eq!(separator.page_number_separator.as_deref(), Some("-"));

        let whitespace = parse_toc_switches(r#"TOC \w \x"#);
        assert!(whitespace.preserve_tabs);
        assert!(whitespace.preserve_line_breaks);
    }

    #[test]
    fn parse_toc_switches_accepts_locale_semicolon_style_mapping() {
        let options = parse_toc_switches(r#"TOC \t "ChapterTitle;1;Appendix Heading;2""#);
        assert_eq!(
            options.custom_style_levels,
            vec![
                ("ChapterTitle".to_string(), 1),
                ("Appendix Heading".to_string(), 2),
            ]
        );
    }

    #[test]
    fn toc_f_after_l_resets_the_tc_level_filter() {
        // Word gives `\\f` order-sensitive precedence over an earlier `\\l`:
        // selecting a TC identifier after the level filter includes all levels
        // for that identifier. Reversing the switches keeps the later `\\l`
        // restriction. Parsing both into the same final pair would regenerate
        // a different set of entries than Word for one of these field codes.
        let f_after_l = parse_toc_switches(r#"TOC \l "2-3" \f i"#);
        assert!(f_after_l.use_toc_entry_fields);
        assert_eq!(f_after_l.toc_entry_identifier.as_deref(), Some("i"));
        assert_eq!(f_after_l.toc_entry_levels, None);

        let l_after_f = parse_toc_switches(r#"TOC \f i \l "2-3""#);
        assert!(l_after_f.use_toc_entry_fields);
        assert_eq!(l_after_f.toc_entry_identifier.as_deref(), Some("i"));
        assert_eq!(l_after_f.toc_entry_levels, Some((2, 3)));
    }

    #[test]
    fn parse_toc_switches_are_ascii_case_insensitive() {
        let options =
            parse_toc_switches(r#"toc \O "2-4" \H \N "3-4" \T "AppendixTitle;2" \P "→" \W \X \U"#);
        assert_eq!((options.outline_min, options.outline_max), (2, 4));
        assert!(options.hyperlinks);
        assert_eq!(options.hide_page_number_levels, Some((3, 4)));
        assert_eq!(
            options.custom_style_levels,
            vec![("AppendixTitle".to_string(), 2)]
        );
        assert_eq!(options.page_number_separator.as_deref(), Some("→"));
        assert!(options.preserve_tabs);
        assert!(options.preserve_line_breaks);
        assert!(options.use_applied_outline_levels);
    }

    #[test]
    fn parse_tc_switches_are_ascii_case_insensitive() {
        let entry = parse_toc_entry_field(r#"tc "Mixed case entry" \F I \L 3 \N"#, 7)
            .expect("mixed-case TC field");
        assert_eq!(entry.text, "Mixed case entry");
        assert_eq!(entry.identifier.as_deref(), Some("I"));
        assert_eq!(entry.level, 3);
        assert!(entry.hide_page_number);
        assert_eq!(entry.paragraph, 7);
    }

    #[test]
    fn parse_outline_range_clamps_and_handles_single_level() {
        assert_eq!(parse_outline_range("1-3"), Some((1, 3)));
        assert_eq!(parse_outline_range("\"2-5\""), Some((2, 5)));
        assert_eq!(parse_outline_range("4"), Some((4, 4)));
        // Out-of-range values clamp to 1-9; reversed ranges are normalized.
        assert_eq!(parse_outline_range("0-42"), Some((1, 9)));
        assert_eq!(parse_outline_range("5-2"), Some((5, 5)));
        assert_eq!(parse_outline_range("abc"), None);
    }

    #[test]
    fn heading_style_level_recognizes_builtin_heading_styles() {
        assert_eq!(heading_style_level("Heading1"), Some(1));
        assert_eq!(heading_style_level("heading 3"), Some(3));
        assert_eq!(heading_style_level("HEADING9"), Some(9));
        assert_eq!(heading_style_level("Heading10"), None);
        assert_eq!(heading_style_level("Heading0"), None);
        assert_eq!(heading_style_level("Normal"), None);
        assert_eq!(heading_style_level("a9"), None);
    }

    fn shading(val: Option<&str>, color: Option<&str>, fill: Option<&str>) -> Shading {
        Shading {
            val: val.map(str::to_string),
            color: color.map(str::to_string),
            fill: fill.map(str::to_string),
        }
    }

    #[test]
    fn clear_pattern_shows_fill() {
        let s = shading(Some("clear"), None, Some("#FFFF00"));
        assert_eq!(s.resolved_background_color().as_deref(), Some("#FFFF00"));
    }

    #[test]
    fn solid_pattern_shows_color_then_fill() {
        let s = shading(Some("solid"), Some("#FF0000"), Some("#00FF00"));
        assert_eq!(s.resolved_background_color().as_deref(), Some("#FF0000"));
        let no_color = shading(Some("solid"), None, Some("#00FF00"));
        assert_eq!(
            no_color.resolved_background_color().as_deref(),
            Some("#00FF00")
        );
    }

    #[test]
    fn solid_match_is_case_insensitive() {
        let s = shading(Some("SOLID"), Some("#112233"), Some("#445566"));
        assert_eq!(s.resolved_background_color().as_deref(), Some("#112233"));
    }

    #[test]
    fn percentage_pattern_blends_foreground_over_fill() {
        let s = shading(Some("pct25"), Some("#FF0000"), Some("#0000FF"));
        assert_eq!(s.resolved_background_color().as_deref(), Some("#4000BF"));
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn logical_indent_resolution_is_direction_symmetric(
            left in any::<Option<i16>>(),
            right in any::<Option<i16>>(),
            start in any::<Option<i16>>(),
            end in any::<Option<i16>>(),
        ) {
            let indent = Indent {
                left: left.map(f32::from),
                right: right.map(f32::from),
                start: start.map(f32::from),
                end: end.map(f32::from),
                left_chars: left.map(i32::from),
                right_chars: right.map(i32::from),
                start_chars: start.map(i32::from),
                end_chars: end.map(i32::from),
                ..Indent::default()
            };

            prop_assert_eq!(indent.physical_left(false), start.or(left).map(f32::from));
            prop_assert_eq!(indent.physical_right(false), end.or(right).map(f32::from));
            prop_assert_eq!(indent.physical_left(true), end.or(left).map(f32::from));
            prop_assert_eq!(indent.physical_right(true), start.or(right).map(f32::from));
            prop_assert_eq!(indent.physical_left_chars(false), start.or(left).map(i32::from));
            prop_assert_eq!(indent.physical_right_chars(false), end.or(right).map(i32::from));
            prop_assert_eq!(indent.physical_left_chars(true), end.or(left).map(i32::from));
            prop_assert_eq!(indent.physical_right_chars(true), start.or(right).map(i32::from));
        }

        #[test]
        fn percentage_shading_blends_arbitrary_rgb_channels(
            foreground in any::<[u8; 3]>(),
            background in any::<[u8; 3]>(),
            percentage in 0_u8..=100,
        ) {
            let foreground_hex = format!(
                "#{:02X}{:02X}{:02X}",
                foreground[0], foreground[1], foreground[2],
            );
            let background_hex = format!(
                "#{:02X}{:02X}{:02X}",
                background[0], background[1], background[2],
            );
            let actual = shading(
                Some(&format!("pct{percentage}")),
                Some(&foreground_hex),
                Some(&background_hex),
            )
            .resolved_background_color()
            .expect("blended color");
            let expected_channel = |foreground: u8, background: u8| {
                ((u16::from(foreground) * u16::from(percentage)
                    + u16::from(background) * u16::from(100 - percentage)
                    + 50)
                    / 100) as u8
            };
            let expected = format!(
                "#{:02X}{:02X}{:02X}",
                expected_channel(foreground[0], background[0]),
                expected_channel(foreground[1], background[1]),
                expected_channel(foreground[2], background[2]),
            );
            prop_assert_eq!(actual, expected);
        }
    }

    #[test]
    fn empty_shading_has_no_background() {
        assert_eq!(shading(None, None, None).resolved_background_color(), None);
        assert!(shading(None, None, None).is_empty());
    }

    #[test]
    fn non_solid_without_fill_uses_color() {
        let s = shading(Some("clear"), Some("#ABCDEF"), None);
        assert_eq!(s.resolved_background_color().as_deref(), Some("#ABCDEF"));
    }

    #[test]
    fn border_edge_with_style_is_visible() {
        let edge = BorderEdge {
            val: Some("single".to_string()),
            size_eighths: Some(8),
            space: Some(1),
            color: Some("#000000".to_string()),
        };
        assert!(edge.is_visible());
    }

    #[test]
    fn border_edge_nil_or_none_is_not_visible() {
        for token in ["nil", "none"] {
            let edge = BorderEdge {
                val: Some(token.to_string()),
                ..Default::default()
            };
            assert!(!edge.is_visible(), "{token} should not be visible");
        }
    }

    #[test]
    fn border_edge_without_style_is_not_visible() {
        assert!(!BorderEdge::default().is_visible());
    }

    #[test]
    fn paragraph_borders_default_is_empty() {
        assert!(ParagraphBorders::default().is_empty());
    }

    #[test]
    fn paragraph_borders_with_one_edge_is_not_empty() {
        let borders = ParagraphBorders {
            bottom: Some(BorderEdge {
                val: Some("single".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!borders.is_empty());
    }

    #[test]
    fn preset_dash_mapping_covers_common_values() {
        assert_eq!(
            TextBoxBorderStyle::from_preset_dash("sysDot"),
            TextBoxBorderStyle::Dotted
        );
        assert_eq!(
            TextBoxBorderStyle::from_preset_dash("lgDash"),
            TextBoxBorderStyle::Dashed
        );
        assert_eq!(
            TextBoxBorderStyle::from_preset_dash("lgDashDotDot"),
            TextBoxBorderStyle::DashDotDot
        );
        assert_eq!(
            TextBoxBorderStyle::from_preset_dash("solid"),
            TextBoxBorderStyle::Solid
        );
    }

    #[test]
    fn vml_dash_mapping_covers_legacy_aliases() {
        assert_eq!(
            TextBoxBorderStyle::from_vml_dashstyle("dashDotDot"),
            TextBoxBorderStyle::DashDotDot
        );
        assert_eq!(
            TextBoxBorderStyle::from_vml_dashstyle("shortDashDot"),
            TextBoxBorderStyle::DashDot
        );
        assert_eq!(
            TextBoxBorderStyle::from_vml_dashstyle("solid"),
            TextBoxBorderStyle::Solid
        );
        assert_eq!(
            TextBoxBorderStyle::from_vml_dashstyle("dbl"),
            TextBoxBorderStyle::Double
        );
    }
}

/// Describes how an image is positioned relative to the surrounding content.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RunImageLayout {
    /// Anchoring mode used by the image. Floating images use absolute
    /// positioning while inline images flow with text.
    pub kind: RunImageKind,
    /// Optional horizontal position data for floating images.
    pub position_h: Option<ImagePosition>,
    /// Optional vertical position data for floating images.
    pub position_v: Option<ImagePosition>,
    /// Clockwise rotation in degrees.
    pub rotation_deg: Option<f64>,
    /// Mirrors the picture around its vertical axis (`a:xfrm@flipH`).
    #[serde(default)]
    pub flip_horizontal: bool,
    /// Mirrors the picture around its horizontal axis (`a:xfrm@flipV`).
    #[serde(default)]
    pub flip_vertical: bool,
    /// How surrounding body text wraps around the (floating) object.  See
    /// ECMA-376 Part 1 §20.4.2.17–§20.4.2.20 (<https://ooxml.info/docs/11/>).
    #[serde(default)]
    pub wrap: WrapMode,
    /// Which side of the floating object may receive body text. Sourced from
    /// `wp:wrapSquare` / `wp:wrapTight` / `wp:wrapThrough@wrapText`.
    #[serde(default)]
    pub wrap_text: WrapTextSide,
    /// Optional edited contour used by tight/through wrapping. Coordinates are
    /// stored in DrawingML's 21600-by-21600 wrap-polygon space so they can be
    /// transformed together with the floating object at layout time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_polygon: Option<WrapPolygon>,
    /// Whether a floating object is painted behind document text. Word uses
    /// `wp:anchor@behindDoc` to distinguish the otherwise identical
    /// "Behind Text" and "In Front of Text" wrap presets.
    #[serde(default)]
    pub behind_doc: bool,
    /// Stacking order among floating objects in the same document layer.
    /// Higher `wp:anchor@relativeHeight` values paint above lower values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_height: Option<u32>,
    /// Page-relative coordinates selected by `wp:anchor@simplePos="1"` and
    /// stored in the child `wp:simplePos@x/@y` element. When present these
    /// coordinates take precedence over `positionH` / `positionV`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simple_position: Option<ImageSimplePosition>,
    /// Distance (CSS pixels) to keep clear of the object on each side when
    /// wrapping text.  Sourced from the `distT`/`distB`/`distL`/`distR`
    /// attributes on `wp:anchor` (overridable by the wrap element itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dist_top: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dist_bottom: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dist_left: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dist_right: Option<f64>,
}

/// Absolute page coordinates used by DrawingML's simple-positioning mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct ImageSimplePosition {
    pub x_px: f64,
    pub y_px: f64,
}

/// A point in DrawingML wrap-polygon coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapPolygonPoint {
    pub x: i64,
    pub y: i64,
}

/// The contour nested under `wp:wrapTight` or `wp:wrapThrough`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WrapPolygon {
    #[serde(default)]
    pub edited: bool,
    #[serde(default)]
    pub points: Vec<WrapPolygonPoint>,
}

/// How body text flows around a floating drawing object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WrapMode {
    /// Object floats in front of / behind text; text does not avoid it
    /// (`wp:wrapNone`, also the inline default).
    #[default]
    None,
    /// Text wraps around the object's bounding rectangle (`wp:wrapSquare`).
    Square,
    /// Text wraps to the object's tight outline (`wp:wrapTight`).
    Tight,
    /// Like Tight but text may flow through transparent regions
    /// (`wp:wrapThrough`).
    Through,
    /// Text is pushed above and below the object, never beside it
    /// (`wp:wrapTopAndBottom`).
    TopAndBottom,
}

/// Side-selection policy for horizontal floating-object wrapping
/// (`ST_WrapText`, ECMA-376 Part 1 §20.4.3.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WrapTextSide {
    /// Text may occupy either side. Layout fills both sides of one line band
    /// when each is wide enough, else the wider side.
    #[default]
    BothSides,
    /// Text may occupy only the object's left side.
    Left,
    /// Text may occupy only the object's right side.
    Right,
    /// Text occupies whichever side has more room.
    Largest,
}

impl WrapTextSide {
    pub fn from_ooxml(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "left" => Self::Left,
            "right" => Self::Right,
            "largest" => Self::Largest,
            _ => Self::BothSides,
        }
    }

    pub fn as_ooxml(self) -> &'static str {
        match self {
            Self::BothSides => "bothSides",
            Self::Left => "left",
            Self::Right => "right",
            Self::Largest => "largest",
        }
    }
}

/// Enumeration of supported image layout strategies.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum RunImageKind {
    /// Image is part of the text flow (e.g. `wp:inline`).
    #[default]
    Inline,
    /// Image is floating and positioned absolutely (e.g. `wp:anchor`).
    Floating,
}

/// Horizontal or vertical positioning descriptor for floating images.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ImagePosition {
    /// Base reference the coordinates are relative to (margin, page, line...).
    pub relative_from: Option<String>,
    /// Offset in CSS pixels from the base reference.
    pub offset_px: Option<f64>,
    /// Relative alignment keyword (`wp:align`) used instead of `wp:posOffset`
    /// when Word anchors the object by alignment (e.g. `center`, `right`,
    /// `inside`).  Stored lowercased.  See ECMA-376 Part 1 §20.4.2.1 (align,
    /// horizontal) / §20.4.2.2 (align, vertical) (<https://ooxml.info/docs/11/>).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
}

/// Word drop-cap placement stored by `w:pPr/w:framePr@w:dropCap`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum DropCapKind {
    /// The enlarged first letter occupies the paragraph's text area and the
    /// following lines wrap beside it (`w:dropCap="drop"`).
    Drop,
    /// The enlarged first letter hangs into the start margin while the body
    /// text keeps the full content width (`w:dropCap="margin"`).
    Margin,
}

impl ParagraphFrame {
    /// Parses every attribute of a `w:framePr` element. `attribute` returns
    /// the raw value for a local attribute name (`"w"`, `"hAnchor"`, ...).
    /// Returns `None` for drop caps, which keep their own model, and for an
    /// element without any frame geometry.
    pub fn from_attributes<'a>(attribute: impl Fn(&str) -> Option<&'a str>) -> Option<Self> {
        if attribute("dropCap")
            .map(|value| value.trim().to_ascii_lowercase())
            .is_some_and(|value| value == "drop" || value == "margin")
        {
            return None;
        }
        let twips = |name: &str| {
            attribute(name).and_then(|value| {
                let value = value.trim();
                value
                    .parse::<i32>()
                    .ok()
                    .or_else(|| value.parse::<f64>().ok().map(|value| value.round() as i32))
            })
        };
        let lower = |name: &str| attribute(name).map(|value| value.trim().to_ascii_lowercase());
        let anchor = |name: &str| match lower(name).as_deref() {
            Some("text") => Some(FrameAnchor::Text),
            Some("margin") => Some(FrameAnchor::Margin),
            Some("page") => Some(FrameAnchor::Page),
            _ => None,
        };
        let align = |name: &str| match lower(name).as_deref() {
            Some("left" | "top") => Some(FrameAlign::Start),
            Some("center") => Some(FrameAlign::Center),
            Some("right" | "bottom") => Some(FrameAlign::End),
            Some("inside") => Some(FrameAlign::Inside),
            Some("outside") => Some(FrameAlign::Outside),
            Some("inline") => Some(FrameAlign::Inline),
            _ => None,
        };
        let frame = Self {
            width_twips: twips("w").filter(|value| *value > 0),
            height_twips: twips("h").map(i32::abs).filter(|value| *value > 0),
            height_rule: match lower("hRule").as_deref() {
                Some("exact") => FrameHeightRule::Exact,
                Some("atleast") => FrameHeightRule::AtLeast,
                // A bare `w:h` without `w:hRule` is a minimum height, the
                // same as the `w:spacing@w:lineRule` default.
                Some("auto") => FrameHeightRule::Auto,
                _ if twips("h").is_some_and(|value| value != 0) => FrameHeightRule::AtLeast,
                _ => FrameHeightRule::Auto,
            },
            x_twips: twips("x"),
            y_twips: twips("y"),
            x_align: align("xAlign").filter(|value| *value != FrameAlign::Inline),
            y_align: align("yAlign"),
            h_anchor: anchor("hAnchor"),
            v_anchor: anchor("vAnchor"),
            wrap: match lower("wrap").as_deref() {
                Some("auto") => Some(FrameWrap::Auto),
                Some("notbeside") => Some(FrameWrap::NotBeside),
                Some("around") => Some(FrameWrap::Around),
                Some("tight") => Some(FrameWrap::Tight),
                Some("through") => Some(FrameWrap::Through),
                Some("none") => Some(FrameWrap::None),
                _ => None,
            },
            h_space_twips: twips("hSpace").map(i32::abs),
            v_space_twips: twips("vSpace").map(i32::abs),
        };
        // `<w:framePr/>` or a cleared drop cap (`w:dropCap="none"`) with no
        // geometry leaves the paragraph in the flow.
        (frame != Self::default()).then_some(frame)
    }
}

/// Paragraph-level drop-cap geometry parsed from `w:framePr`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct DropCap {
    pub kind: DropCapKind,
    /// Number of body lines occupied by the enlarged first letter. Word's UI
    /// authors 1–10; Jade clamps imported values to that safe render range.
    pub lines: u8,
}

/// Base a text frame's horizontal or vertical position is measured from
/// (`w:framePr@w:hAnchor` / `@w:vAnchor`, ECMA-376 Part 1 §17.18.35/§17.18.100).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum FrameAnchor {
    /// The text column (horizontal) or the anchor paragraph (vertical).
    Text,
    /// The page margins.
    Margin,
    /// The page edge.
    Page,
}

/// How tall a text frame is (`w:framePr@w:hRule`, ECMA-376 Part 1 §17.18.37).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "camelCase")]
pub enum FrameHeightRule {
    /// Height follows the content; `w:h` is ignored.
    #[default]
    Auto,
    /// `w:h` is the minimum height.
    AtLeast,
    /// `w:h` is the exact height; overflowing content is clipped.
    Exact,
}

/// Text wrapping around a text frame (`w:framePr@w:wrap`, ECMA-376 Part 1
/// §17.18.104).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum FrameWrap {
    Auto,
    NotBeside,
    Around,
    Tight,
    Through,
    None,
}

/// Positioned paragraph text frame parsed from `w:framePr` (ECMA-376 Part 1
/// §17.3.1.11) when it is not a drop cap. Lengths are twentieths of a point
/// exactly as authored; [`ParagraphFrame::default`] is a frame with no
/// attributes. Consecutive paragraphs whose frames compare equal share one
/// frame (§17.3.1.11: "identical frame properties").
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_twips: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_twips: Option<i32>,
    #[serde(default)]
    pub height_rule: FrameHeightRule,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_twips: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_twips: Option<i32>,
    /// `w:xAlign`, lowercased: `left`/`center`/`right`/`inside`/`outside`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_align: Option<FrameAlign>,
    /// `w:yAlign`, lowercased: `top`/`center`/`bottom`/`inside`/`outside`/`inline`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_align: Option<FrameAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_anchor: Option<FrameAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v_anchor: Option<FrameAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap: Option<FrameWrap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_space_twips: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v_space_twips: Option<i32>,
}

/// Relative alignment of a text frame (`w:xAlign` ST_XAlign §17.18.108 /
/// `w:yAlign` ST_YAlign §17.18.109). One enum covers both axes: `Start` is
/// left/top and `End` is right/bottom.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum FrameAlign {
    Start,
    Center,
    End,
    Inside,
    Outside,
    Inline,
}

/// Paragraph-level properties that influence layout and alignment.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParagraphProperties {
    /// The paragraph's `w14:paraId` attribute (a stable, document-unique
    /// identifier on `<w:p>`), surfaced here as a read-side convenience so a
    /// paragraph can be looked up by id (like `getElementById`) independently of
    /// its shifting positional index. Round-trip preservation is via raw XML.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub para_id: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<Rc<str>>,
    /// Direct formatting of the paragraph mark (`w:pPr/w:rPr`, ECMA-376 Part
    /// 1 §17.3.1.29). These properties affect the paragraph mark and the line
    /// box of an otherwise empty paragraph; they are not inherited by existing
    /// `w:r` content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark_run_properties: Option<RunProperties>,
    /// Tracked change on the paragraph mark itself (`w:pPr/w:rPr/w:del`,
    /// `w:ins`, `w:moveFrom` or `w:moveTo`, ECMA-376 Part 1 §17.13.5). A
    /// deleted mark joins the paragraph with the next one once the deletion
    /// is applied, as Word's final view shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark_revision: Option<Box<RevisionMark>>,
    pub alignment: Option<Rc<str>>,
    /// Base paragraph direction (`w:bidi`, ECMA-376 Part 1 §17.3.1.6).
    /// `Some(true)` selects an RTL paragraph base direction, while
    /// `Some(false)` explicitly overrides an inherited RTL style with LTR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bidi: Option<bool>,
    pub spacing: Option<Spacing>,
    pub indent: Option<Indent>,
    pub numbering: Option<Numbering>,
    pub section: Option<SectionProperties>,
    /// First-letter drop cap (`w:framePr w:dropCap="drop|margin"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_cap: Option<DropCap>,
    /// Positioned text frame (`w:framePr` without a drop cap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<ParagraphFrame>,
    #[serde(default)]
    pub tabs: Vec<TabStop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_to_grid: Option<bool>,
    /// `w:adjustRightInd`: automatic right-indent adjustment with a section
    /// document grid. Absence inherits; the OOXML default is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjust_right_indent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widow_control: Option<bool>,
    /// `w:suppressAutoHyphens` (ECMA-376 Part 1 §17.3.1.34): when true the
    /// paragraph is excluded from automatic hyphenation even if the document
    /// enables it. Absent means off (the paragraph follows the document
    /// hyphenation setting). Structural metadata — we do not run a hyphenation
    /// engine, so this is preserved (not rendered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_auto_hyphens: Option<bool>,
    /// `w:pageBreakBefore` (ECMA-376 Part 1 §17.3.1.23): when true the paragraph
    /// must start at the top of a new page. Absent means off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_break_before: Option<bool>,
    /// `w:suppressLineNumbers` (ECMA-376 Part 1 §17.3.1.35): when true the lines
    /// of this paragraph are excluded from margin line numbering — they are
    /// neither counted nor labelled. Absent means off (the paragraph
    /// participates in line numbering).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_line_numbers: Option<bool>,
    /// `w:keepLines` (ECMA-376 Part 1 §17.3.1.15): when true all lines of the
    /// paragraph must stay together on a single page rather than splitting
    /// across a page boundary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_lines: Option<bool>,
    /// `w:keepNext` (ECMA-376 Part 1 §17.3.1.16): when true the paragraph must
    /// stay on the same page as the paragraph that follows it (used for
    /// headings that should not be stranded at the bottom of a page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_next: Option<bool>,
    /// `w:textAlignment` (ECMA-376 Part 1 §17.3.1.39): vertical alignment of the
    /// glyphs on every line within the paragraph's line box. Holds the
    /// normalized lowercase token (`top`/`center`/`bottom`/`baseline`/`auto`).
    /// Absent (or `auto`/`baseline`) means the application default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_alignment: Option<Rc<str>>,
    /// Mirror left/right (start/end) indents on facing pages (`w:mirrorIndents`).
    /// When enabled, paragraphs that begin on a verso (even) page have their
    /// start and end indents swapped so binding-side indentation stays on the
    /// inside edge.  See ECMA-376 Part 1 §17.3.1.18
    /// (<https://ooxml.info/docs/11/Part1/17.3.1.18>).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror_indents: Option<bool>,
    /// `w:outlineLvl` (ECMA-376 Part 1 §17.3.1.20): the paragraph's outline
    /// (heading) level, `0`–`8` where `0` is the top level (Heading 1). A value
    /// of `9` (or absent) means body text — no outline level. Drives Word's
    /// navigation pane / table-of-contents structure. Stored clamped to `0..=9`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_level: Option<u8>,
    /// Whether [`Self::outline_level`] came from `w:p/w:pPr/w:outlineLvl`
    /// rather than from the applied paragraph style. Word uses this provenance
    /// for TOC generation: directly applied levels participate only when the
    /// field carries `\u`, while style-defined levels participate through
    /// `\o`. This is parse-time metadata and does not change layout.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outline_level_is_direct: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub east_asian_settings: Option<EastAsianLineBreakingSettings>,
    /// Paragraph shading (`w:shd` inside `w:pPr`).  See ECMA-376 Part 1
    /// §17.3.5 (<https://ooxml.info/docs/11/Part1/17.3.5>).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<Shading>,
    /// Paragraph borders (`w:pBdr` inside `w:pPr`).  See ECMA-376 Part 1
    /// §17.3.1.24 (<https://ooxml.info/docs/11/Part1/17.3.1.24>).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<ParagraphBorders>,
    /// Contextual spacing (`w:contextualSpacing`, ECMA-376 Part 1 §17.3.1.9).
    /// When `Some(true)`, the paragraph's `w:spacing` before/after is ignored
    /// against an adjacent paragraph that uses the identical paragraph style —
    /// e.g. consecutive bullet-list items collapse the inter-item gap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contextual_spacing: Option<bool>,
}

/// Paragraph borders (`w:pBdr`).  Holds the six possible edges Word can draw
/// around (or between) a paragraph.  See ECMA-376 Part 1 §17.3.1.24.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphBorders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<BorderEdge>,
    /// Border drawn between consecutive paragraphs that share identical border
    /// settings (`w:between`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub between: Option<BorderEdge>,
    /// Vertical bar drawn to the left of the paragraph (`w:bar`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar: Option<BorderEdge>,
}

impl ParagraphBorders {
    /// Returns true when no edge carries any information, so callers may treat
    /// the whole `w:pBdr` as absent.
    pub fn is_empty(&self) -> bool {
        self.top.is_none()
            && self.bottom.is_none()
            && self.left.is_none()
            && self.right.is_none()
            && self.between.is_none()
            && self.bar.is_none()
    }
}

/// Page borders (`w:pgBorders`, ECMA-376 Part 1 §17.6.10). Drawn around the
/// whole page (or text area). Reuses [`BorderEdge`] for each side; the
/// `offset_from` attribute records whether the inset is measured from the page
/// edge (`page`) or the text margin (`text`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PageBorders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<BorderEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<BorderEdge>,
    /// `w:offsetFrom` — `page` (inset from the paper edge) or `text` (inset from
    /// the text margin). Defaults to `text` semantics when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_from: Option<String>,
}

impl PageBorders {
    /// Returns true when no edge carries any information, so callers may treat
    /// the whole `w:pgBorders` as absent.
    pub fn is_empty(&self) -> bool {
        self.top.is_none() && self.bottom.is_none() && self.left.is_none() && self.right.is_none()
    }
}

/// A single border edge (`w:top` / `w:bottom` / `w:left` / `w:right` /
/// `w:between` / `w:bar` inside `w:pBdr`, and the analogous table-cell edges).
///
/// - `val` is the line style (`w:val`, e.g. `single`, `double`, `dashed`),
///   stored lowercased so renderers match case-insensitively.
/// - `size_eighths` is the line width in eighths of a point (`w:sz`).
/// - `space` is the spacing between border and text in points (`w:space`).
/// - `color` is normalized to `#RRGGBB`; the OOXML sentinel `auto` is `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "camelCase")]
pub struct BorderEdge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub val: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_eighths: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl BorderEdge {
    /// Returns true when the edge should be drawn: it has a line style that is
    /// not the `nil`/`none` sentinel. Word treats a missing `w:val` or an
    /// explicit `nil`/`none` as "no line".
    pub fn is_visible(&self) -> bool {
        self.val
            .as_deref()
            .is_some_and(|v| !matches!(v, "nil" | "none"))
    }
}

/// WordprocessingML shading (`w:shd`).  Used by paragraphs (`w:pPr`), runs
/// (`w:rPr`), and table cells (`w:tcPr`).  See ECMA-376 Part 1 §17.3.5.
///
/// - `val` is the shading pattern (`w:val`, e.g. `clear`, `solid`, `pct25`),
///   stored lowercased.
/// - `color` is the pattern foreground color (`w:color`).
/// - `fill` is the background fill color (`w:fill`).
///
/// Colors are normalized to `#RRGGBB`; the OOXML sentinel `auto` is stored as
/// `None` so callers fall back to their inherited/default color.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "camelCase")]
pub struct Shading {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub val: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
}

impl Shading {
    /// Returns true when the shading carries no visible information (no pattern
    /// and no colors), so callers can treat it as absent.
    pub fn is_empty(&self) -> bool {
        self.val.is_none() && self.color.is_none() && self.fill.is_none()
    }

    /// Resolves the effective solid background color a renderer should paint
    /// behind the content, as `#RRGGBB`, or `None` when the shading paints no
    /// visible background.
    ///
    /// Word's shading is `pattern(color) over fill`. We approximate:
    /// - `solid`: the pattern fully covers the area, so the visible color is
    ///   the pattern `color` (falling back to `fill`).
    /// - `clear` / `nil` / absent: no pattern overlay, so the visible color is
    ///   the `fill`.
    /// - percentage patterns (`pct25`, …): blend `color` over `fill` using the
    ///   encoded foreground coverage.
    /// - stripe patterns (`horzStripe`, …): use `fill` (falling back to
    ///   `color`) until patterned canvas brushes are available.
    pub fn resolved_background_color(&self) -> Option<String> {
        fn parse_hex_rgb(value: &str) -> Option<[u8; 3]> {
            let value = value.strip_prefix('#').unwrap_or(value);
            if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            Some([
                u8::from_str_radix(&value[0..2], 16).ok()?,
                u8::from_str_radix(&value[2..4], 16).ok()?,
                u8::from_str_radix(&value[4..6], 16).ok()?,
            ])
        }

        fn percentage_pattern(value: &str) -> Option<u8> {
            value
                .strip_prefix("pct")?
                .parse::<u8>()
                .ok()
                .filter(|percentage| *percentage <= 100)
        }

        if let Some(percentage) = self
            .val
            .as_deref()
            .and_then(|value| percentage_pattern(&value.to_ascii_lowercase()))
        {
            if let (Some(foreground), Some(background)) = (
                self.color.as_deref().and_then(parse_hex_rgb),
                self.fill.as_deref().and_then(parse_hex_rgb),
            ) {
                let blend = |foreground: u8, background: u8| {
                    ((u16::from(foreground) * u16::from(percentage)
                        + u16::from(background) * u16::from(100 - percentage)
                        + 50)
                        / 100) as u8
                };
                return Some(format!(
                    "#{:02X}{:02X}{:02X}",
                    blend(foreground[0], background[0]),
                    blend(foreground[1], background[1]),
                    blend(foreground[2], background[2]),
                ));
            }
        }
        let is_solid = self
            .val
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("solid"));
        if is_solid {
            self.color.clone().or_else(|| self.fill.clone())
        } else {
            self.fill.clone().or_else(|| self.color.clone())
        }
    }
}

/// Collection of indentation controls. Point-based fields are expressed in
/// points. Character-based fields are in hundredths of a character width.
/// Office 2010 logical `start`/`end` values take precedence over their physical
/// `left`/`right` counterparts on the edge they resolve to.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Indent {
    pub left: Option<f32>,
    pub right: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f32>,
    pub first_line: Option<f32>,
    pub hanging: Option<f32>,
    /// Character-based left indent in hundredths of a character width
    /// (`w:leftChars`).  100 = 1 character.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_chars: Option<i32>,
    /// Character-based right indent in hundredths of a character width
    /// (`w:rightChars`).  100 = 1 character.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_chars: Option<i32>,
    /// Logical leading indent (`w:startChars`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_chars: Option<i32>,
    /// Logical trailing indent (`w:endChars`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_chars: Option<i32>,
    /// Character-based first-line indent in hundredths of a character width
    /// (`w:firstLineChars`).  100 = 1 character.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_line_chars: Option<i32>,
    /// Character-based hanging indent in hundredths of a character width
    /// (`w:hangingChars`).  100 = 1 character.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hanging_chars: Option<i32>,
}

impl Indent {
    pub fn physical_left(&self, bidi: bool) -> Option<f32> {
        if bidi {
            self.end.or(self.left)
        } else {
            self.start.or(self.left)
        }
    }

    pub fn physical_right(&self, bidi: bool) -> Option<f32> {
        if bidi {
            self.start.or(self.right)
        } else {
            self.end.or(self.right)
        }
    }

    pub fn physical_left_chars(&self, bidi: bool) -> Option<i32> {
        if bidi {
            self.end_chars.or(self.left_chars)
        } else {
            self.start_chars.or(self.left_chars)
        }
    }

    pub fn physical_right_chars(&self, bidi: bool) -> Option<i32> {
        if bidi {
            self.start_chars.or(self.right_chars)
        } else {
            self.end_chars.or(self.right_chars)
        }
    }
}

/// Paragraph-level tab stop definition converted into points.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TabStop {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EastAsianLineBreakingSettings {
    pub use_east_asian_layout: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinsoku: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overflow_punctuation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_line_punctuation: Option<bool>,
    #[serde(default)]
    pub balance_single_byte_double_byte_width: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_space_de: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_space_dn: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_spacing_control: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_line_break_before: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_line_break_after: Vec<String>,
    /// `no_line_break_before` is a Japanese custom set, which replaces the
    /// default table (ECMA-376 §17.15.1.59) rather than extending it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_line_break_before_replaces_default: bool,
    /// `no_line_break_after` is a Japanese custom set (ECMA-376 §17.15.1.58).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_line_break_after_replaces_default: bool,
    /// `w:strictFirstAndLastChars`: the strict Japanese set, which adds the
    /// small kana and `ー` to the characters that may not start a line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strict_first_and_last_chars: bool,
    /// `w:compat/w:compatSetting[@w:name="compatibilityMode"]`: the Word
    /// version whose layout rules the document asks for (12 = Word 2007,
    /// 14 = Word 2010, 15 = Word 2013 and later). `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility_mode: Option<u32>,
}

/// Numbering metadata that associates a paragraph with a numbering definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Numbering {
    pub id: Option<String>,
    pub level: Option<u32>,
}

impl Numbering {
    /// The numbering instance this paragraph is a list item of. `w:numId`
    /// `0` is not an instance: it switches numbering off (typically the
    /// numbering a paragraph style supplies), so it is no list at all.
    pub fn list_id(&self) -> Option<&str> {
        self.id.as_deref().filter(|id| *id != "0")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum FontHint {
    #[default]
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "eastAsia")]
    EastAsia,
    #[serde(rename = "cs")]
    Complex,
}

impl FontHint {
    pub fn from_xml_hint(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "default" | "" => Some(FontHint::Default),
            "eastasia" => Some(FontHint::EastAsia),
            "cs" | "complex" => Some(FontHint::Complex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunFonts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ascii: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_ansi: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub east_asia: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complex: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ascii_theme: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_ansi_theme: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub east_asia_theme: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complex_theme: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<FontHint>,
}

impl RunFonts {
    pub fn is_empty(&self) -> bool {
        self.ascii.is_none()
            && self.h_ansi.is_none()
            && self.east_asia.is_none()
            && self.complex.is_none()
            && self.ascii_theme.is_none()
            && self.h_ansi_theme.is_none()
            && self.east_asia_theme.is_none()
            && self.complex_theme.is_none()
            && self.hint.is_none()
    }

    /// Layers `other` (a later style layer or direct formatting) over these
    /// fonts. Per script slot the later layer's choice replaces the earlier
    /// one whole: a theme reference drops an inherited font name, and a font
    /// name drops an inherited theme reference. A Title style giving
    /// `w:asciiTheme="majorHAnsi"` over a Normal style's `w:ascii="Arial"`
    /// prints in the theme's Calibri in Word for Mac. When `other` carries
    /// both (a theme already resolved to its face), both are kept.
    pub fn merge_from(&mut self, other: &RunFonts) {
        fn layer(
            name: &mut Option<Rc<str>>,
            theme: &mut Option<Rc<str>>,
            other_name: &Option<Rc<str>>,
            other_theme: &Option<Rc<str>>,
        ) {
            match (other_name, other_theme) {
                (Some(other_name), Some(other_theme)) => {
                    *name = Some(other_name.clone());
                    *theme = Some(other_theme.clone());
                }
                (Some(other_name), None) => {
                    *name = Some(other_name.clone());
                    *theme = None;
                }
                (None, Some(other_theme)) => {
                    *name = None;
                    *theme = Some(other_theme.clone());
                }
                (None, None) => {}
            }
        }
        layer(
            &mut self.ascii,
            &mut self.ascii_theme,
            &other.ascii,
            &other.ascii_theme,
        );
        layer(
            &mut self.h_ansi,
            &mut self.h_ansi_theme,
            &other.h_ansi,
            &other.h_ansi_theme,
        );
        layer(
            &mut self.east_asia,
            &mut self.east_asia_theme,
            &other.east_asia,
            &other.east_asia_theme,
        );
        layer(
            &mut self.complex,
            &mut self.complex_theme,
            &other.complex,
            &other.complex_theme,
        );
        if let Some(value) = other.hint.as_ref() {
            self.hint = Some(value.clone());
        }
    }

    /// Drops each font name its theme attribute overrides. Call it on the
    /// fonts of one `w:rFonts` element: ECMA-376 Part 1 §17.3.2.26 ignores
    /// `w:ascii` when the same element gives `w:asciiTheme` (and likewise for
    /// `hAnsi`, `eastAsia` and `cs`). Word for Mac prints a Title whose style
    /// says `w:asciiTheme="majorHAnsi" w:ascii="Arial"` in the theme's Calibri.
    pub fn drop_names_shadowed_by_themes(&mut self) {
        if self.ascii_theme.is_some() {
            self.ascii = None;
        }
        if self.h_ansi_theme.is_some() {
            self.h_ansi = None;
        }
        if self.east_asia_theme.is_some() {
            self.east_asia = None;
        }
        if self.complex_theme.is_some() {
            self.complex = None;
        }
    }

    pub fn preferred_latin(&self) -> Option<&str> {
        self.ascii.as_deref().or(self.h_ansi.as_deref())
    }

    pub fn preferred_east_asia(&self) -> Option<&str> {
        self.east_asia.as_deref()
    }

    pub fn preferred_complex(&self) -> Option<&str> {
        self.complex.as_deref()
    }
}

/// Styling attributes of a single run. Measurements are stored in points.
/// Raw `w:rPr` child elements that the writer does not model explicitly
/// (e.g. `w:lang`, `w:rtl`, `w:eastAsianLayout`, `w:rStyle`). Captured at load
/// from the raw XML and carried on the run so that — when a run is split and a
/// fragment loses its `node_id` — the fragment can re-emit these children
/// instead of dropping them (preservation bridge; see docs/headless-todo.md P2).
///
/// Equality intentionally ignores the contents: two runs differing only by
/// preserved raw children are still considered equal so that layout-time run
/// coalescing (`RunProperties == RunProperties`) is unaffected.
#[derive(Debug, Clone, Default)]
pub struct PreservedRunProps(pub Vec<XmlNode>);

impl PartialEq for PreservedRunProps {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// Font-related run properties authored directly in `w:rPr` before style and
/// document defaults are materialized for layout. The writer uses this snapshot
/// so inherited effective values are not emitted as direct formatting.
///
/// Equality intentionally ignores this provenance, just like
/// [`PreservedRunProps`], because it must not affect layout-time run coalescing.
#[derive(Debug, Clone, Default)]
pub struct DirectRunFontProperties {
    pub known: bool,
    pub font_size: Option<f32>,
    pub font_size_complex: Option<f32>,
    pub kerning: Option<f32>,
    pub text_scale: Option<f32>,
    pub fonts: Option<RunFonts>,
}

impl PartialEq for DirectRunFontProperties {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RunProperties {
    pub style: Option<Rc<str>>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// Complex-script bold (`w:bCs`, ECMA-376 Part 1 §17.3.2.6). Word applies
    /// this to complex-script (e.g. Arabic/Hebrew) text, the counterpart of
    /// `w:b` for Latin/East-Asian text. The renderer is script-agnostic, so it
    /// ORs this with `w:b` when deciding whether a run is bold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold_complex: Option<bool>,
    /// Complex-script italic (`w:iCs`, ECMA-376 Part 1 §17.3.2.17), the
    /// counterpart of `w:i` for complex-script text. ORed with `w:i`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic_complex: Option<bool>,
    /// Complex-script font size in points (`w:szCs`, ECMA-376 Part 1
    /// §17.3.2.39). Sizes complex-script (Arabic/Hebrew/Thai) glyphs; `w:sz`
    /// keeps sizing Latin/East-Asian glyphs. When only `w:szCs` is present the
    /// parser also mirrors it into `font_size` (legacy fallback).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size_complex: Option<f32>,
    pub underline: Option<bool>,
    /// Underline kind beyond on/off (`w:u@w:val`, ECMA-376 Part 1 §17.3.2.40):
    /// "double", "dotted", "dash", "wave", "thick", "words", … `None` means a
    /// plain single underline when `underline` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_kind: Option<Rc<str>>,
    /// Underline color (`w:u@w:color`): an RRGGBB hex token or "auto" (follow
    /// the text color).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_color: Option<Rc<str>>,
    pub strike: Option<bool>,
    /// Double strikethrough (`w:dstrike`, ECMA-376 Part 1 §17.3.2.9). Drawn as
    /// two parallel horizontal lines through the text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub double_strike: Option<bool>,
    /// All-capitals display (`w:caps`, ECMA-376 Part 1 §17.3.2.1). When true
    /// the run's text is rendered uppercased without altering the stored text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<bool>,
    /// Small-capitals display (`w:smallCaps`, ECMA-376 Part 1 §17.3.2.33). When
    /// true the run's lowercase letters are rendered as smaller capitals without
    /// altering the stored text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_caps: Option<bool>,
    /// Hidden text (`w:vanish`, ECMA-376 Part 1 §17.3.2.45). When `Some(true)`
    /// the run is not displayed and occupies no layout space in normal/print
    /// view (matching Word, which hides such text unless "show hidden text" is
    /// enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vanish: Option<bool>,
    /// Always-hidden text (`w:specVanish`, ECMA-376 Part 1 §17.3.2.36). Set on
    /// runs that are part of an auto-generated structure (e.g. the hidden
    /// numbering/label text of a heading) which Word forces hidden regardless of
    /// the "show hidden text" toggle. Treated like `w:vanish` for rendering: the
    /// run is not displayed and occupies no layout space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_vanish: Option<bool>,
    /// Drop shadow (`w:shadow`, ECMA-376 Part 1 §17.3.2.31). When `Some(true)`
    /// the run's glyphs are drawn with an offset shadow behind them, matching
    /// Word's text-shadow character effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    /// Outline (hollow) text (`w:outline`, ECMA-376 Part 1 §17.3.2.23). When
    /// `Some(true)` the run's glyphs are drawn as stroked outlines instead of
    /// solid fills, matching Word's outline character effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<bool>,
    /// Embossed (raised) text (`w:emboss`, ECMA-376 Part 1 §17.3.2.13). When
    /// `Some(true)` the run's glyphs are drawn as if raised off the page,
    /// matching Word's emboss character effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emboss: Option<bool>,
    /// Imprinted (engraved) text (`w:imprint`, ECMA-376 Part 1 §17.3.2.18).
    /// When `Some(true)` the run's glyphs are drawn as if pressed into the page,
    /// matching Word's engrave character effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imprint: Option<bool>,
    /// Emphasis mark (`w:em`, ECMA-376 Part 1 §17.3.2.12). A small mark drawn
    /// above (or, for `underDot`, below) each character — common in Japanese
    /// typography (圏点). Stored as the normalized lowercased token
    /// ("dot" / "comma" / "circle" / "underdot"); `None`/"none" means no mark.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emphasis: Option<Rc<str>>,
    /// Animated text effect (`w:effect`, ECMA-376 Part 1 §17.3.2.10,
    /// ST_TextEffect). Values: "blinkBackground" / "lights" / "antsBlack" /
    /// "antsRed" / "shimmer" / "sparkle". Stored as the normalized lowercased
    /// token; `None`/"none" means no effect. These are screen-only animations
    /// that Word does not render in print/layout view, so this is parsed and
    /// preserved (round-trip) but produces no static visual.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<Rc<str>>,
    pub baseline: Option<Rc<str>>,
    /// Baseline raise/lower in points (`w:position`, ECMA-376 Part 1
    /// §17.3.2.24). The attribute `w:val` is in half-points; positive raises the
    /// text above the baseline, negative lowers it. Unlike `w:vertAlign`
    /// (super/subscript) this only shifts the baseline and does not change the
    /// font size, and the two combine additively.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_position: Option<f32>,
    pub char_spacing: Option<f32>,
    /// Kerning threshold in points (`w:kern` in half-points).
    pub kerning: Option<f32>,
    /// Horizontal text scaling percent (`w:w`, 100 = normal width).
    pub text_scale: Option<f32>,
    /// Fit-text width metadata (`w:fitText`, ECMA-376 Part 1 §17.3.2.14).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_text: Option<RunFitText>,
    pub font_size: Option<f32>,
    pub font_family: Option<Rc<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fonts: Option<RunFonts>,
    /// Whether this exact property level authored `w:rFonts` with an
    /// East-Asian hint.
    ///
    /// This is provenance rather than the computed hint value. `None` means
    /// no `w:rFonts` was authored here, `Some(true)` means
    /// `w:hint="eastAsia"`, and `Some(false)` means `w:rFonts` was authored
    /// without that exact hint. The concrete `default`/`cs` value, when one was
    /// authored, remains in [`RunFonts::hint`]. When the attribute is omitted,
    /// retaining this provenance prevents a style's inherited East-Asian hint
    /// from being mistaken for direct formatting.
    ///
    /// The flag must not be used as a shortcut for paragraph properties such
    /// as `w:autoSpaceDN`: those rules remain independent after font-slot
    /// resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authored_east_asia_font_hint: Option<bool>,
    /// Whether this exact `w:rFonts` element authored `w:eastAsia` or
    /// `w:eastAsiaTheme`. This remains separate from the merged font names so
    /// run-level intent survives paragraph/style fallback filling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authored_east_asia_font_name: Option<bool>,
    /// Result language (`w:lang/@w:val`). Word's language-dependent field
    /// number formats use this value rather than assuming English. This covers
    /// `CardText`, `Ordinal`, `OrdText`, and `DollarText` field results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Rc<str>>,
    /// East-Asian editing language (`w:lang/@w:eastAsia`).
    ///
    /// Word uses this independently from `w:lang/@w:val`: the former selects
    /// the language-specific `w:overflowPunct` character set and other
    /// East-Asian line-breaking behavior, while the latter remains the result
    /// language for fields and general language services. Keeping both values
    /// prevents Japanese punctuation from falling back to a locale-neutral
    /// union after style resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub east_asia_language: Option<Rc<str>>,
    pub color: Option<Rc<str>>,
    /// The theme colour `color` was resolved from (`w:color/@w:themeColor`,
    /// `@w:themeTint`, `@w:themeShade`, ECMA-376 Part 1 §17.3.2.6). Kept so
    /// a written run still follows the theme, as Word's テーマの色 do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<Box<RunThemeColor>>,
    /// Highlight (background) color name (`w:highlight`).
    /// Values are Word highlight color names: "yellow", "green", "cyan",
    /// "magenta", "blue", "red", "darkBlue", "darkCyan", "darkGreen",
    /// "darkMagenta", "darkRed", "darkYellow", "darkGray", "lightGray",
    /// "black", "white", or "none".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Rc<str>>,
    /// Character shading (`w:rPr/w:shd`, ECMA-376 Part 1 §17.3.5). Unlike
    /// `w:highlight` (limited to 16 named colors), run shading lets Word paint
    /// an arbitrary `w:fill` color behind the run's glyphs. The renderer draws
    /// the resolved background color as a fill band behind the text (highlight
    /// takes visual precedence when both are present).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<Shading>,
    /// Character border (`w:bdr`, ECMA-376 Part 1 §17.3.2.4). A single box drawn
    /// around the run's text. Reuses [`BorderEdge`] (line style / width / space /
    /// color). `None` means no character border.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<BorderEdge>,
    /// Ruby (phonetic guide / ふりがな) annotation (`w:ruby`, ECMA-376 Part 1
    /// §17.3.3.25). Attached by the parser to every run produced from the
    /// `w:rubyBase` contents (see [`RubyAnnotation::continuation`]); the guide
    /// text from `w:rt` is stored here instead of flowing into the body text
    /// (so selection offsets stay consistent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ruby: Option<Box<RubyAnnotation>>,
    /// East Asian layout (`w:eastAsianLayout`, ECMA-376 Part 1 §17.3.2.10):
    /// two lines in one (組み文字 / 割注) and horizontal-in-vertical text
    /// (縦中横).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub east_asian_layout: Option<Box<EastAsianLayout>>,
    /// Word 2010's text effects and OpenType features (`w14:ligatures`,
    /// `w14:glow`, `w14:textOutline`, …). Their XML is kept with the
    /// unmodeled children; this is what the renderer and the フォント dialog
    /// read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_effects: Option<Box<RunTextEffects>>,
    /// Unmodeled `w:rPr` children preserved verbatim (see [`PreservedRunProps`]).
    /// Not serialized: this is an in-memory preservation bridge, reconstructed at
    /// load and consulted only by the export writer.
    #[serde(skip)]
    pub preserved_rpr: PreservedRunProps,
    /// Direct-formatting provenance for font properties. Effective inherited
    /// values remain in the regular fields for rendering and measurement.
    #[serde(skip)]
    pub direct_font_properties: DirectRunFontProperties,
}

impl RunProperties {
    pub fn capture_direct_font_properties(&mut self) {
        self.direct_font_properties = DirectRunFontProperties {
            known: true,
            font_size: self.font_size,
            font_size_complex: self.font_size_complex,
            kerning: self.kerning,
            text_scale: self.text_scale,
            fonts: self.fonts.clone(),
        };
    }

    pub fn mark_font_properties_direct_from(&mut self, source: &RunProperties) {
        if !self.direct_font_properties.known {
            return;
        }
        if let Some(value) = source.font_size {
            self.direct_font_properties.font_size = Some(value);
            self.direct_font_properties.font_size_complex = Some(value);
        }
        if let Some(value) = source.font_size_complex {
            self.direct_font_properties.font_size_complex = Some(value);
        }
        if let Some(value) = source.kerning {
            self.direct_font_properties.kerning = Some(value);
        }
        if let Some(value) = source.text_scale {
            self.direct_font_properties.text_scale = Some(value);
        }
        if let Some(fonts) = source.fonts.as_ref() {
            self.direct_font_properties
                .fonts
                .get_or_insert_with(RunFonts::default)
                .merge_from(fonts);
        }
    }

    pub fn clear_direct_font_themes(&mut self, include_east_asia: bool) {
        if let Some(fonts) = self.direct_font_properties.fonts.as_mut() {
            fonts.ascii_theme = None;
            fonts.h_ansi_theme = None;
            if include_east_asia {
                fonts.east_asia_theme = None;
                fonts.complex_theme = None;
            }
        }
    }

    pub fn clear_direct_font_properties(&mut self) {
        if self.direct_font_properties.known {
            self.direct_font_properties.font_size = None;
            self.direct_font_properties.font_size_complex = None;
            self.direct_font_properties.kerning = None;
            self.direct_font_properties.text_scale = None;
            self.direct_font_properties.fonts = None;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.preserved_rpr.0.is_empty()
            && self.style.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.bold_complex.is_none()
            && self.italic_complex.is_none()
            && self.font_size_complex.is_none()
            && self.underline.is_none()
            && self.underline_kind.is_none()
            && self.underline_color.is_none()
            && self.strike.is_none()
            && self.double_strike.is_none()
            && self.caps.is_none()
            && self.small_caps.is_none()
            && self.vanish.is_none()
            && self.spec_vanish.is_none()
            && self.shadow.is_none()
            && self.outline.is_none()
            && self.emboss.is_none()
            && self.imprint.is_none()
            && self.emphasis.is_none()
            && self.effect.is_none()
            && self.baseline.is_none()
            && self.text_position.is_none()
            && self.char_spacing.is_none()
            && self.kerning.is_none()
            && self.text_scale.is_none()
            && self.fit_text.is_none()
            && self.font_size.is_none()
            && self.font_family.is_none()
            && self.fonts.as_ref().is_none_or(RunFonts::is_empty)
            && self.authored_east_asia_font_hint.is_none()
            && self.authored_east_asia_font_name.is_none()
            && self.language.is_none()
            && self.east_asia_language.is_none()
            && self.color.is_none()
            && self.highlight.is_none()
            && self.shading.is_none()
            && self.border.as_ref().is_none_or(|b| !b.is_visible())
            && self.ruby.is_none()
            && self.theme_color.is_none()
            && self.east_asian_layout.is_none()
            && self.text_effects.is_none()
    }
}

/// Word 2010's run extensions (the `w14` namespace, [MS-DOCX] 2.6).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunTextEffects {
    /// `w14:ligatures/@w14:val`: `none`, `standard`, `contextual`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ligatures: Option<Rc<str>>,
    /// `w14:numForm/@w14:val`: `default`, `lining`, `oldStyle`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_form: Option<Rc<str>>,
    /// `w14:numSpacing/@w14:val`: `default`, `proportional`, `tabular`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_spacing: Option<Rc<str>>,
    /// `w14:stylisticSets`: the one set turned on, 1–20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stylistic_set: Option<u8>,
    /// `w14:cntxtAlts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contextual_alternates: Option<bool>,
    /// `w14:glow`'s colour, `#RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow_color: Option<Rc<str>>,
    /// `w14:glow/@w14:rad` in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow_radius: Option<f32>,
    /// `w14:glow`'s opacity, 0–1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow_alpha: Option<f32>,
    /// `w14:textOutline`'s solid line colour, `#RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_color: Option<Rc<str>>,
    /// `w14:textOutline/@w14:w` in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_width: Option<f32>,
    /// Render-time only: the enclosure a 囲い文字 field (`EQ \o\ac`) draws
    /// round the run. Never read from or written to `w:rPr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enclosure: Option<Rc<str>>,
}

impl RunTextEffects {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A run colour's theme reference (`w:themeColor`, `w:themeTint`,
/// `w:themeShade`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunThemeColor {
    /// ST_ThemeColor, as written (`accent1`, `text1`, …).
    pub name: Rc<str>,
    /// Tint byte, two hex digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<Rc<str>>,
    /// Shade byte, two hex digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade: Option<Rc<str>>,
}

/// `w:eastAsianLayout` (ECMA-376 Part 1 §17.3.2.10).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EastAsianLayout {
    /// `w:id`: runs of one combined or vertical group share it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// `w:combine`: the run's text is set as two lines in one line's height.
    #[serde(default)]
    pub combine: bool,
    /// `w:combineBrackets`: `none`, `round`, `square`, `angle`, `curly`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combine_brackets: Option<Rc<str>>,
    /// `w:vert`: horizontal in vertical text (縦中横).
    #[serde(default)]
    pub vert: bool,
    /// `w:vertCompress`: the horizontal text is compressed to the line.
    #[serde(default)]
    pub vert_compress: bool,
}

impl EastAsianLayout {
    /// Reads the element's attributes, given as (name, value) pairs.
    pub fn from_attributes<'a>(attributes: impl IntoIterator<Item = (&'a [u8], String)>) -> Self {
        let mut layout = Self::default();
        for (name, value) in attributes {
            let on = !matches!(value.as_str(), "0" | "false" | "off");
            match name {
                b"w:id" => layout.id = value.parse().ok(),
                b"w:combine" => layout.combine = on,
                b"w:combineBrackets" => layout.combine_brackets = Some(Rc::from(value.as_str())),
                b"w:vert" => layout.vert = on,
                b"w:vertCompress" => layout.vert_compress = on,
                _ => {}
            }
        }
        layout
    }
}

/// Ruby (phonetic guide) annotation parsed from `w:ruby` (ECMA-376 Part 1
/// §17.3.3.25). Word draws the guide text (`w:rt`) in a small font above the
/// base text (`w:rubyBase`); common in Japanese documents as ふりがな.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RubyAnnotation {
    /// Concatenated guide text from the `w:rt` runs.
    pub text: String,
    /// Guide text size in points, from the `w:rt` run's `w:sz` (half-points)
    /// or the `w:rubyPr/w:hps` fallback. `None` means "half the base size".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_pt: Option<f32>,
    /// Distribution of the guide over the base (`w:rubyPr/w:rubyAlign`,
    /// §17.3.3.27), normalized to lowercase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    /// Distance from the base baseline up to the guide baseline in
    /// half-points (`w:rubyPr/w:hpsRaise`, §17.3.3.12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raise_half_points: Option<u32>,
    /// Fonts the guide run names directly (`w:rt/w:r/w:rPr/w:rFonts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fonts: Option<RunFonts>,
    /// Whether this run continues the base started by an earlier run. A base
    /// may span several runs (one per formatting change); every one of them
    /// carries the annotation so edits know which text belongs to the base,
    /// but only the run that starts it (`false`) draws the guide.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub continuation: bool,
}

/// Run-level fixed-width fit text property (`w:fitText`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunFitText {
    /// Exact width in twentieths of a point (`w:val`).
    pub val: u32,
    /// Optional signed contiguous fit-text group ID (`w:id`, ST_DecimalNumber).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
}

/// Represents the range of text associated with a document comment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommentRange {
    pub id: String,
    pub start: usize,
    pub end: usize,
}

/// Marker indicating where a comment reference is anchored within a paragraph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommentMarker {
    pub id: String,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Note {
    pub id: String,
    pub note_type: Option<String>,
    pub body: Vec<BodyElement>,
}

/// Metadata describing a resolved comment entry from `comments.xml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DocumentComment {
    pub id: String,
    pub author: Option<String>,
    pub initials: Option<String>,
    pub date: Option<String>,
    pub resolved: Option<bool>,
    pub thread_parent_id: Option<String>,
    pub text: String,
}

/// Paragraph spacing details, expressed either in points or line-based
/// multipliers depending on the originating Word attribute.
/// Word's spacing for `w:beforeAutospacing`/`w:afterAutospacing`, which HTML
/// imports set ("auto" in the paragraph dialog): 14pt, whatever the
/// `w:before`/`w:after` value beside it says.
pub const WORD_AUTO_PARAGRAPH_SPACING_PT: f32 = 14.0;

/// Whether an OOXML on/off attribute value is on.
pub(crate) fn ooxml_on(value: &[u8]) -> bool {
    matches!(value, b"1" | b"true" | b"on")
}

impl Spacing {
    /// Applies `w:beforeAutospacing`/`w:afterAutospacing` once every other
    /// attribute of the element is read: auto spacing replaces the authored
    /// points and lines on its side.
    pub(crate) fn apply_autospacing(&mut self, before: bool, after: bool) {
        if before {
            self.before = Some(WORD_AUTO_PARAGRAPH_SPACING_PT);
            self.before_lines = None;
        }
        if after {
            self.after = Some(WORD_AUTO_PARAGRAPH_SPACING_PT);
            self.after_lines = None;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Spacing {
    pub before: Option<f32>,
    pub after: Option<f32>,
    pub before_lines: Option<f32>,
    pub after_lines: Option<f32>,
    pub line: Option<f32>,
    pub line_rule: Option<String>,
}

/// Page size and margin information derived from the section properties.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PageSettings {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub margin_left: Option<f32>,
    pub margin_right: Option<f32>,
    pub margin_top: Option<f32>,
    pub margin_bottom: Option<f32>,
    /// Distance from the top page edge to the header story (`w:header`), in
    /// points. Word's default is 0.5 inch when the attribute is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_distance: Option<f32>,
    /// Distance from the bottom page edge to the footer story (`w:footer`), in
    /// points. Word's default is 0.5 inch when the attribute is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer_distance: Option<f32>,
    pub orientation: Option<String>,
    /// Binding gutter (`w:pgMar/@w:gutter`, ECMA-376 Part 1 §17.6.11), in
    /// points. Extra space reserved on the binding edge (added to the left
    /// margin in the MVP). `None`/`0` (the default for most documents) leaves
    /// the text region unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gutter: Option<f32>,
}

/// Section-level properties that may affect page layout or header/footer links.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SectionProperties {
    pub page_settings: Option<PageSettings>,
    pub header_reference: Option<String>,
    pub footer_reference: Option<String>,
    /// Relationship id of the header used on the first page when `title_page` is
    /// set. Word uses this for cover-page style documents.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub header_reference_first: Option<String>,
    /// Relationship id of the footer used on the first page when `title_page`
    /// is set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub footer_reference_first: Option<String>,
    /// Relationship id of the header used on even-numbered pages when the
    /// document-level `evenAndOddHeaders` setting is enabled.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub header_reference_even: Option<String>,
    /// Relationship id of the footer used on even-numbered pages when the
    /// document-level `evenAndOddHeaders` setting is enabled.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub footer_reference_even: Option<String>,
    /// Mirrors `<w:titlePg/>` — when true, the first page of the section uses
    /// the `*_first` header/footer references rather than the default pair.
    #[serde(default)]
    pub title_page: bool,
    pub section_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_direction: Option<TextDirection>,
    pub columns: Option<SectionColumns>,
    pub doc_grid: Option<DocGrid>,
    /// Page borders (`w:pgBorders`) drawn around the page or text area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_borders: Option<PageBorders>,
    /// Page number format/start (`w:pgNumType`) applied to `PAGE` fields in this
    /// section (e.g. lower-roman front matter, or a restart at page 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_number_type: Option<PageNumberType>,
    /// Line numbering settings (`w:lnNumType`) that draw line numbers in the
    /// left margin of the section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_numbering: Option<LineNumbering>,
    /// Footnote numbering settings (`w:footnotePr`) — numeral format and start
    /// number applied to footnote references in this section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footnote_properties: Option<NoteNumbering>,
    /// Endnote numbering settings (`w:endnotePr`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endnote_properties: Option<NoteNumbering>,
    /// Section-wide endnote placement (`w:endnotePr/w:pos`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endnote_position: Option<EndnotePosition>,
    /// Vertical justification of the section's text on the page
    /// (`w:vAlign`, ECMA-376 Part 1 §17.6.22 — ST_VerticalJc). One of
    /// `top` (default), `center`, `both` (justified), or `bottom`,
    /// stored lowercased. `None`/absent means top alignment. Word uses
    /// `center` for cover/title pages so the heading sits in the middle
    /// of the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical_alignment: Option<String>,
    /// The section reads right to left (`w:bidi`): columns run from the
    /// right.
    #[serde(default)]
    pub bidi: bool,
    /// The binding gutter is on the right edge (`w:rtlGutter`).
    #[serde(default)]
    pub rtl_gutter: bool,
}

/// Footnote/endnote numbering settings (`w:footnotePr` / `w:endnotePr`,
/// ECMA-376 Part 1 §17.11.12 / §17.11.4). Controls the numeral format, the
/// starting number, and when numbering restarts. Stored in a section's
/// `sectPr` (and, in Word, also at the document-settings level — this model
/// captures both forms).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NoteNumbering {
    /// `w:numFmt` — numeral format (`decimal`, `upperRoman`, `lowerRoman`,
    /// `upperLetter`, `lowerLetter`, `chicago`, …), stored lowercased.
    /// `None`/absent means decimal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
    /// `w:numStart` — the first note number used (defaults to 1 when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_start: Option<u32>,
    /// `w:numRestart` — when numbering resets: `continuous`, `eachSect`, or
    /// `eachPage`. Stored lowercased.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_restart: Option<String>,
}

impl NoteNumbering {
    /// Returns true when no meaningful note-numbering attribute was supplied.
    pub fn is_empty(&self) -> bool {
        self.number_format.is_none() && self.number_start.is_none() && self.number_restart.is_none()
    }
}

/// Margin line numbering settings (`w:lnNumType`, ECMA-376 Part 1 §17.6.8).
/// Word draws a running line number in the left margin; the attributes here
/// control how often a number is shown, where numbering restarts, and how far
/// the number sits from the text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LineNumbering {
    /// `w:countBy` — show a line number every Nth line. `None`/absent means
    /// every line (effectively 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count_by: Option<u32>,
    /// Raw `w:start`. Word treats it as the count of skipped numbers:
    /// `Some(1)` displays 2 first; absent displays 1 (MS-OE376 §2.1.214).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    /// `w:restart` — when numbering resets: `continuous`, `newPage`, or
    /// `newSection`. Stored lowercased. `None` means `newPage` (the Word
    /// default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart: Option<String>,
    /// `w:distance` — gap from the number's right edge to the text, in twips.
    /// `None` lets the renderer pick a default gap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<u32>,
}

impl LineNumbering {
    /// Returns true when no meaningful line-numbering attribute was supplied.
    pub fn is_empty(&self) -> bool {
        self.count_by.is_none()
            && self.start.is_none()
            && self.restart.is_none()
            && self.distance.is_none()
    }
}

/// Page numbering settings (`w:pgNumType`, ECMA-376 Part 1 §17.6.12). Controls
/// the numeral format and (optionally) the starting page number for a section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PageNumberType {
    /// `w:fmt` — numeral format (`decimal`, `upperRoman`, `lowerRoman`,
    /// `upperLetter`, `lowerLetter`, …), stored lowercased. `None`/absent means
    /// decimal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// `w:start` — the displayed page number of the section's first page. `None`
    /// means continue from the previous section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    /// `w:chapStyle` — the heading level whose number leads the page number
    /// (Word's 章番号を含める), as `1-3` for page 3 of chapter 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_style: Option<u8>,
    /// `w:chapSep` — what separates the chapter number from the page number:
    /// `hyphen` (the default), `period`, `colon`, `emDash` or `enDash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_separator: Option<String>,
}

impl PageNumberType {
    /// Returns true when neither a format nor a start value was supplied.
    pub fn is_empty(&self) -> bool {
        self.format.is_none() && self.start.is_none() && self.chapter_style.is_none()
    }
}

/// Describes a contiguous range of body elements that share the same section properties.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SectionRange {
    pub start: usize,
    pub end: usize,
    pub properties: Option<SectionProperties>,
}

/// Column layout settings derived from `<w:cols>`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SectionColumns {
    /// Number of columns defined by the section (Word defaults to 1).
    pub count: Option<u32>,
    /// Space between columns, expressed in points.
    pub space: Option<f32>,
    /// Whether columns should use equal width.
    pub equal_width: Option<bool>,
    /// Whether a vertical separator should be drawn between columns.
    pub separator: Option<bool>,
    /// Optional per-column overrides for width/space.
    pub columns: Vec<ColumnDefinition>,
}

/// Individual column overrides when a section specifies custom widths.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ColumnDefinition {
    /// Column width in points when specified.
    pub width: Option<f32>,
    /// Custom spacing that follows the column, in points.
    pub space: Option<f32>,
}

/// Simplified representation of the WordprocessingML `w:docGrid` element.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocGrid {
    /// Grid behaviour (`default`, `lines`, `linesAndChars`, `snapToChars`, ...).
    pub grid_type: Option<String>,
    /// Target line pitch in points (converted from 1/20 pt units).
    pub line_pitch: Option<f32>,
    /// Additional character spacing in points (converted from 1/4096 pt units).
    pub char_space: Option<f32>,
}
