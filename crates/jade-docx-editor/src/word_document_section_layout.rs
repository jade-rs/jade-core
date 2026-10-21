// A section's layout as Word's ページ設定 dialog reads and writes it, over
// the structured model (so the pages reflow at once) and the raw
// `w:sectPr` / `settings.xml` (so the saved document says the same).
//
// The dialog edits one section, a run of them or all of them, and only
// what the user changed: [`HeadlessSectionLayoutPatch`] leaves every
// `None` alone, so applying one margin to a whole document keeps each
// section's own paper, columns and grid.

use super::{PageMarginSpec, WordDocument};
use crate::document::{
    BodyElement, ColumnDefinition, DocGrid, LineNumbering, ParagraphProperties, SectionColumns,
    SectionProperties, TextDirection,
};
use crate::docx_xml::{XmlAttribute, XmlElement, XmlNode};
use jade_docx_headless_api::{
    HeadlessDocGrid, HeadlessLineNumbering, HeadlessPageNumbering, HeadlessSectionColumn,
    HeadlessSectionColumns, HeadlessSectionLayout, HeadlessSectionLayoutPatch,
    HeadlessSectionScope,
};
use jade_docx_layout::section_transition::{
    DEFAULT_MARGIN_PT, DEFAULT_PAGE_HEIGHT_PT, DEFAULT_PAGE_WIDTH_PT,
};

/// Word's default distance from the page edge to the header and the footer
/// (`w:header` / `w:footer` absent): half an inch.
const DEFAULT_HEADER_DISTANCE_PT: f32 = 36.0;
/// `w:cols/@w:space` absent: half an inch.
const DEFAULT_COLUMN_SPACE_PT: f32 = 36.0;
/// `w:docDefaults` without a size: 10 pt.
const DEFAULT_FONT_SIZE_PT: f32 = 10.0;

fn twips(points: f32) -> String {
    ((points * 20.0).round() as i64).max(0).to_string()
}

/// Line-number restarts as the model stores them (lowercased) and as Word
/// writes them.
fn line_restart_ooxml(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "continuous" => "continuous",
        "newsection" => "newSection",
        _ => "newPage",
    }
}

fn text_direction_name(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::LrTb => "lrTb",
        TextDirection::LrTbV => "lrTbV",
        TextDirection::TbRl => "tbRl",
        TextDirection::TbRlV => "tbRlV",
        TextDirection::BtLr => "btLr",
        TextDirection::BtLrV => "btLrV",
    }
}

impl WordDocument {
    /// How many sections the body has: one for a document without breaks.
    pub fn section_count(&self) -> usize {
        self.document
            .as_deref()
            .map_or(1, |document| document.sections.len().max(1))
    }

    /// The sections `scope` reaches, as indices for the `*_in_section`
    /// edits; `None` is the final section, for a caller without a caret.
    pub fn sections_in_scope(
        &self,
        scope: HeadlessSectionScope,
        caret_paragraph: Option<usize>,
    ) -> Vec<Option<usize>> {
        let count = self.section_count();
        let caret = caret_paragraph.and_then(|paragraph| self.section_index_for_paragraph(paragraph));
        let clamp = |index: usize| index.min(count - 1);
        match scope {
            HeadlessSectionScope::Caret => vec![caret],
            HeadlessSectionScope::Index { index } => vec![Some(clamp(index))],
            HeadlessSectionScope::Range { first, last } => {
                let (first, last) = (clamp(first.min(last)), clamp(first.max(last)));
                (first..=last).map(Some).collect()
            }
            HeadlessSectionScope::CaretToEnd => match caret {
                Some(first) => (first..count).map(Some).collect(),
                None => vec![None],
            },
            HeadlessSectionScope::All => (0..count).map(Some).collect(),
        }
    }

    /// One section's layout; `None` is the final section.
    pub fn section_layout(&self, section: Option<usize>) -> HeadlessSectionLayout {
        let count = self.section_count();
        let index = section.unwrap_or(count - 1).min(count - 1);
        let document = self.document.as_deref();
        let properties = self.section_properties(section).cloned().unwrap_or_default();
        let settings = self.section_page_settings(section).unwrap_or_default();
        let width = settings.width.unwrap_or(DEFAULT_PAGE_WIDTH_PT);
        let height = settings.height.unwrap_or(DEFAULT_PAGE_HEIGHT_PT);
        let default_font_size = document
            .and_then(|document| document.default_font_size)
            .unwrap_or(DEFAULT_FONT_SIZE_PT);
        let columns = properties.columns.clone().unwrap_or_default();
        let column_count = columns
            .count
            .unwrap_or(columns.columns.len().max(1) as u32)
            .max(1);
        let equal_width = columns.equal_width != Some(false) || columns.columns.is_empty();
        let line_numbering = properties.line_numbering.clone();
        let grid = properties.doc_grid.clone().unwrap_or_default();
        let page_numbering = properties.page_number_type.clone().unwrap_or_default();
        HeadlessSectionLayout {
            section: index,
            section_count: count,
            width,
            height,
            landscape: settings
                .orientation
                .as_deref()
                .map_or(width > height, |orientation| {
                    orientation.eq_ignore_ascii_case("landscape")
                }),
            margin_top: settings.margin_top.unwrap_or(DEFAULT_MARGIN_PT),
            margin_bottom: settings.margin_bottom.unwrap_or(DEFAULT_MARGIN_PT),
            margin_left: settings.margin_left.unwrap_or(DEFAULT_MARGIN_PT),
            margin_right: settings.margin_right.unwrap_or(DEFAULT_MARGIN_PT),
            header_distance: settings
                .header_distance
                .unwrap_or(DEFAULT_HEADER_DISTANCE_PT),
            footer_distance: settings
                .footer_distance
                .unwrap_or(DEFAULT_HEADER_DISTANCE_PT),
            gutter: settings.gutter.unwrap_or(0.0),
            gutter_at_top: document.is_some_and(|document| document.gutter_at_top),
            rtl_gutter: properties.rtl_gutter,
            mirror_margins: document.is_some_and(|document| document.mirror_margins),
            book_fold: document.is_some_and(|document| document.book_fold_printing),
            print_two_on_one: document.is_some_and(|document| document.print_two_on_one),
            start: properties
                .section_type
                .clone()
                .unwrap_or_else(|| "nextPage".to_string()),
            vertical_alignment: properties
                .vertical_alignment
                .clone()
                .unwrap_or_else(|| "top".to_string()),
            different_first_page: properties.title_page,
            different_odd_even: self.even_and_odd_headers,
            columns: HeadlessSectionColumns {
                count: column_count,
                space: columns.space.unwrap_or(DEFAULT_COLUMN_SPACE_PT),
                equal_width,
                separator: columns.separator.unwrap_or(false),
                columns: if equal_width {
                    Vec::new()
                } else {
                    columns
                        .columns
                        .iter()
                        .map(|column| HeadlessSectionColumn {
                            width: column.width.unwrap_or(0.0),
                            space: column.space.unwrap_or(0.0),
                        })
                        .collect()
                },
            },
            line_numbering: HeadlessLineNumbering {
                enabled: line_numbering.is_some(),
                start_at: line_numbering
                    .as_ref()
                    .and_then(|numbering| numbering.start)
                    .unwrap_or(0)
                    + 1,
                count_by: line_numbering
                    .as_ref()
                    .and_then(|numbering| numbering.count_by)
                    .unwrap_or(1)
                    .max(1),
                distance: line_numbering
                    .as_ref()
                    .and_then(|numbering| numbering.distance)
                    .map(|twips| twips as f32 / 20.0),
                restart: line_restart_ooxml(
                    line_numbering
                        .as_ref()
                        .and_then(|numbering| numbering.restart.as_deref())
                        .unwrap_or("newPage"),
                )
                .to_string(),
            },
            doc_grid: HeadlessDocGrid {
                grid_type: grid.grid_type.clone().unwrap_or_else(|| "default".to_string()),
                line_pitch: grid.line_pitch,
                char_pitch: grid.char_space.map(|space| default_font_size + space),
            },
            default_font_size,
            text_direction: properties
                .text_direction
                .map_or("lrTb", text_direction_name)
                .to_string(),
            bidi: properties.bidi,
            page_numbering: HeadlessPageNumbering {
                format: self
                    .page_number_format(section)
                    .format
                    .unwrap_or_else(|| "decimal".to_string()),
                start: page_numbering.start,
                chapter_style: page_numbering.chapter_style,
                chapter_separator: page_numbering.chapter_separator,
            },
            page_borders: properties
                .page_borders
                .as_ref()
                .map(|borders| {
                    let edge = |edge: Option<&crate::document::BorderEdge>| {
                        edge.filter(|edge| {
                            edge.val
                                .as_deref()
                                .is_some_and(|val| val != "none" && val != "nil")
                        })
                        .map(|edge| jade_docx_headless_api::HeadlessBorderEdge {
                            style: edge.val.clone(),
                            // An art border's `w:sz` is in whole points.
                            width: edge.size_eighths.map(|size| {
                                if edge.val.as_deref().is_some_and(|val| {
                                    crate::canvas_renderer::art_border_name(val).is_some()
                                }) {
                                    size as f32
                                } else {
                                    size as f32 / 8.0
                                }
                            }),
                            color: edge.color.clone(),
                        })
                    };
                    jade_docx_headless_api::HeadlessPageBorderSet {
                        top: edge(borders.top.as_ref()),
                        bottom: edge(borders.bottom.as_ref()),
                        left: edge(borders.left.as_ref()),
                        right: edge(borders.right.as_ref()),
                        offset_from: borders.offset_from.clone(),
                    }
                })
                .unwrap_or_default(),
        }
    }

    /// Applies `patch` to each of `sections` (see [`Self::sections_in_scope`]).
    /// The document-wide switches are set once.
    pub fn set_section_layout(
        &mut self,
        sections: &[Option<usize>],
        patch: &HeadlessSectionLayoutPatch,
    ) -> Result<(), String> {
        if self.document.is_none() {
            return Err("document is unavailable".to_string());
        }
        for (name, flag) in [
            ("w:gutterAtTop", patch.gutter_at_top),
            ("w:mirrorMargins", patch.mirror_margins),
            ("w:bookFoldPrinting", patch.book_fold),
            ("w:printTwoOnOne", patch.print_two_on_one),
        ] {
            if let Some(enabled) = flag {
                self.set_document_layout_flag(name, enabled)?;
            }
        }
        for section in sections {
            self.set_one_section_layout(*section, patch)?;
        }
        Ok(())
    }

    /// A `settings.xml` switch the renderer also reads off the model.
    fn set_document_layout_flag(&mut self, name: &str, enabled: bool) -> Result<(), String> {
        let current = self.document.as_deref().map(|document| match name {
            "w:gutterAtTop" => document.gutter_at_top,
            "w:mirrorMargins" => document.mirror_margins,
            "w:bookFoldPrinting" => document.book_fold_printing,
            _ => document.print_two_on_one,
        });
        if current == Some(enabled) {
            return Ok(());
        }
        // A package without settings.xml (generated documents) still lays
        // out with the switch; it is only not saved.
        let _ = self.set_settings_flag(name, enabled);
        if let Some(document) = self.document_mut() {
            match name {
                "w:gutterAtTop" => document.gutter_at_top = enabled,
                "w:mirrorMargins" => document.mirror_margins = enabled,
                "w:bookFoldPrinting" => document.book_fold_printing = enabled,
                _ => document.print_two_on_one = enabled,
            }
        }
        Ok(())
    }

    fn set_one_section_layout(
        &mut self,
        section: Option<usize>,
        patch: &HeadlessSectionLayoutPatch,
    ) -> Result<(), String> {
        let current = self.section_layout(section);

        // 用紙 and 印刷の向き.
        if patch.width.is_some() || patch.height.is_some() || patch.landscape.is_some() {
            let width = patch.width.unwrap_or(current.width);
            let height = patch.height.unwrap_or(current.height);
            let landscape = patch.landscape.unwrap_or(current.landscape);
            let (short, long) = (width.min(height), width.max(height));
            let (width, height) = if landscape {
                (long, short)
            } else {
                (short, long)
            };
            let orientation = if landscape { "landscape" } else { "portrait" };
            if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
                return Err("the paper needs a width and a height".to_string());
            }
            self.apply_page_setup_in_section(
                section,
                Some(width),
                Some(height),
                Some(orientation.to_string()),
            )?;
        }

        // 余白, とじしろ and the header and footer distances.
        let margins = PageMarginSpec {
            top: patch.margin_top,
            bottom: patch.margin_bottom,
            left: patch.margin_left,
            right: patch.margin_right,
            header: patch.header_distance,
            footer: patch.footer_distance,
            gutter: patch.gutter,
        };
        if margins != PageMarginSpec::default() {
            self.set_page_margin_values_in_section(section, margins)?;
        }

        if let Some(rtl_gutter) = patch.rtl_gutter {
            self.edit_section_properties(section, |properties| {
                properties.rtl_gutter = rtl_gutter
            });
            self.set_sect_pr_flag(section, "w:rtlGutter", rtl_gutter)?;
        }
        if let Some(bidi) = patch.bidi {
            self.edit_section_properties(section, |properties| properties.bidi = bidi);
            self.set_sect_pr_flag(section, "w:bidi", bidi)?;
        }

        // セクションの開始位置.
        if let Some(start) = patch.start.as_deref() {
            let start = match start {
                "continuous" | "nextColumn" | "evenPage" | "oddPage" | "nextPage" => start,
                other => return Err(format!("unknown section start: {other}")),
            }
            .to_string();
            self.edit_section_properties(section, |properties| {
                properties.section_type = Some(start.clone())
            });
            let value = start.clone();
            self.set_sect_pr_element(section, "w:type", Some(vec![("w:val", value)]))?;
        }

        // 垂直方向の配置.
        if let Some(alignment) = patch.vertical_alignment.as_deref() {
            let alignment = match alignment {
                "top" | "center" | "both" | "bottom" => alignment.to_string(),
                other => return Err(format!("unknown vertical alignment: {other}")),
            };
            let stored = (alignment != "top").then(|| alignment.clone());
            self.edit_section_properties(section, |properties| {
                properties.vertical_alignment = stored.clone()
            });
            self.set_sect_pr_element(
                section,
                "w:vAlign",
                (alignment != "top").then(|| vec![("w:val", alignment)]),
            )?;
        }

        // 先頭ページのみ別指定 and 奇数/偶数ページ別指定 materialize the
        // stories they switch to.
        if patch.different_first_page.is_some() || patch.different_odd_even.is_some() {
            let first = patch
                .different_first_page
                .unwrap_or(current.different_first_page);
            let odd_even = patch
                .different_odd_even
                .unwrap_or(current.different_odd_even);
            self.set_header_footer_variants(first, odd_even, section)?;
        }

        if let Some(columns) = patch.columns.as_ref() {
            self.set_section_columns(section, columns)?;
        }

        if let Some(numbering) = patch.line_numbering.as_ref() {
            self.set_section_line_numbering(section, numbering)?;
        }

        if let Some(grid) = patch.doc_grid.as_ref() {
            self.set_section_doc_grid(section, grid, current.default_font_size)?;
        }

        // 文字方向: 縦書き is `tbRl`; 横書き is the default and leaves no
        // element behind.
        if let Some(direction) = patch.text_direction.as_deref() {
            let parsed = TextDirection::try_from_str(direction)
                .ok_or_else(|| format!("unknown text direction: {direction}"))?;
            let stored = (parsed != TextDirection::LrTb).then_some(parsed);
            self.edit_section_properties(section, |properties| {
                properties.text_direction = stored
            });
            self.set_sect_pr_element(
                section,
                "w:textDirection",
                stored.map(|direction| vec![("w:val", text_direction_name(direction).to_string())]),
            )?;
        }

        if let Some(numbering) = patch.page_numbering.as_ref() {
            self.set_section_page_numbering(section, numbering)?;
        }

        // ページ罫線.
        if let Some(borders) = patch.page_borders.as_ref() {
            let edge = |edge: &Option<jade_docx_headless_api::HeadlessBorderEdge>| {
                edge.as_ref().map(|edge| super::BorderEdgeSpec {
                    style: edge.style.clone(),
                    width: edge.width,
                    color: edge.color.clone(),
                })
            };
            let spec = (!borders.is_empty()).then(|| super::PageBorderSpec {
                top: edge(&borders.top),
                bottom: edge(&borders.bottom),
                left: edge(&borders.left),
                right: edge(&borders.right),
                offset_from: borders.offset_from.clone(),
            });
            self.set_page_borders_in_section(section, spec.as_ref())?;
        }
        Ok(())
    }

    /// Edits `section`'s structured properties: the terminating paragraph's
    /// for every section but the last, the body's for the last.
    pub(super) fn edit_section_properties(
        &mut self,
        section: Option<usize>,
        edit: impl Fn(&mut SectionProperties),
    ) {
        let seed = self.section_properties(section).cloned().unwrap_or_default();
        let terminator = section.and_then(|index| self.section_terminator_body_index(index));
        let Some(document) = self.document_mut() else {
            return;
        };
        match terminator {
            Some(body_index) => {
                if let Some(BodyElement::Paragraph(paragraph)) = document
                    .body
                    .as_mut()
                    .and_then(|body| body.elements.get_mut(body_index))
                {
                    edit(
                        paragraph
                            .properties
                            .get_or_insert_with(ParagraphProperties::default)
                            .section
                            .get_or_insert(seed),
                    );
                }
            }
            None => edit(document.default_section.get_or_insert(seed)),
        }
        Self::rebuild_section_ranges(document);
    }

    /// Sets a `w:sectPr` child's attributes to exactly `attributes`, or
    /// removes the child for `None`.
    fn set_sect_pr_element(
        &mut self,
        section: Option<usize>,
        name: &str,
        attributes: Option<Vec<(&str, String)>>,
    ) -> Result<(), String> {
        let Some(sect_pr_id) = self.sect_pr_id_for_section(section)? else {
            return Ok(());
        };
        match attributes {
            Some(attributes) => self.upsert_sect_pr_child(sect_pr_id, name, |element| {
                element.attributes = attributes
                    .into_iter()
                    .map(|(name, value)| XmlAttribute {
                        name: name.to_string(),
                        value,
                    })
                    .collect();
            }),
            None => {
                let Some(raw) = self.raw_document.as_mut() else {
                    return Ok(());
                };
                let sect_pr = raw
                    .element_mut(sect_pr_id)
                    .ok_or_else(|| "failed to locate section properties".to_string())?;
                sect_pr
                    .children
                    .retain(|child| !matches!(child, XmlNode::Element(element) if element.name == name));
                raw.rebuild_index_for_subtree(sect_pr_id);
                Ok(())
            }
        }
    }

    /// A bare on/off `w:sectPr` child, present when `enabled`.
    fn set_sect_pr_flag(
        &mut self,
        section: Option<usize>,
        name: &str,
        enabled: bool,
    ) -> Result<(), String> {
        self.set_sect_pr_element(section, name, enabled.then(Vec::new))
    }

    /// 段組み: equal columns, or each column's width and the space after it.
    pub fn set_section_columns(
        &mut self,
        section: Option<usize>,
        columns: &HeadlessSectionColumns,
    ) -> Result<(), String> {
        let unequal = !columns.equal_width && !columns.columns.is_empty();
        let count = if unequal {
            columns.columns.len() as u32
        } else {
            columns.count
        };
        if !(1..=45).contains(&count) {
            return Err(format!("a section has 1 to 45 columns, not {count}"));
        }
        let content_width = self.section_page_setup(section).content_width;
        if unequal {
            let total: f32 = columns
                .columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    column.width
                        + if index + 1 < columns.columns.len() {
                            column.space
                        } else {
                            0.0
                        }
                })
                .sum();
            if columns
                .columns
                .iter()
                .any(|column| !column.width.is_finite() || column.width <= 0.0 || column.space < 0.0)
                || total > content_width + 1.0
            {
                return Err("the columns do not fit between the margins".to_string());
            }
        } else if !columns.space.is_finite()
            || columns.space < 0.0
            || columns.space * count.saturating_sub(1) as f32 >= content_width
        {
            return Err("page column spacing leaves no room for text in this section".to_string());
        }
        let model = SectionColumns {
            count: Some(count),
            space: Some(columns.space),
            equal_width: Some(!unequal),
            separator: Some(columns.separator && count > 1),
            columns: if unequal {
                columns
                    .columns
                    .iter()
                    .map(|column| ColumnDefinition {
                        width: Some(column.width),
                        space: Some(column.space),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        };
        let stored = model.clone();
        self.edit_section_properties(section, |properties| {
            properties.columns = Some(stored.clone())
        });
        let Some(sect_pr_id) = self.sect_pr_id_for_section(section)? else {
            return Ok(());
        };
        let children: Vec<XmlNode> = match self.raw_document.as_mut() {
            Some(raw) => model
                .columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    let mut attributes = vec![XmlAttribute {
                        name: "w:w".to_string(),
                        value: twips(column.width.unwrap_or(0.0)),
                    }];
                    if index + 1 < model.columns.len() {
                        attributes.push(XmlAttribute {
                            name: "w:space".to_string(),
                            value: twips(column.space.unwrap_or(0.0)),
                        });
                    }
                    XmlNode::Element(raw.create_element("w:col", attributes, Vec::new()))
                })
                .collect(),
            None => return Ok(()),
        };
        self.upsert_sect_pr_child(sect_pr_id, "w:cols", |element: &mut XmlElement| {
            let mut attributes = vec![
                ("w:num", count.to_string()),
                ("w:space", twips(columns.space)),
            ];
            if unequal {
                attributes.push(("w:equalWidth", "0".to_string()));
            }
            if columns.separator && count > 1 {
                attributes.push(("w:sep", "1".to_string()));
            }
            element.attributes = attributes
                .into_iter()
                .map(|(name, value)| XmlAttribute {
                    name: name.to_string(),
                    value,
                })
                .collect();
            element.children = children;
        })
    }

    /// 行番号: on with its start, interval, distance and restart, or off.
    pub fn set_section_line_numbering(
        &mut self,
        section: Option<usize>,
        numbering: &HeadlessLineNumbering,
    ) -> Result<(), String> {
        if !numbering.enabled {
            self.edit_section_properties(section, |properties| properties.line_numbering = None);
            return self.set_sect_pr_element(section, "w:lnNumType", None);
        }
        let count_by = numbering.count_by.max(1);
        let start = numbering.start_at.max(1) - 1;
        let restart = line_restart_ooxml(&numbering.restart);
        let distance = numbering
            .distance
            .filter(|distance| distance.is_finite() && *distance > 0.0)
            .map(|distance| (distance * 20.0).round() as u32);
        let model = LineNumbering {
            count_by: Some(count_by),
            start: (start > 0).then_some(start),
            restart: Some(restart.to_ascii_lowercase()),
            distance,
        };
        self.edit_section_properties(section, |properties| {
            properties.line_numbering = Some(model.clone())
        });
        let mut attributes = vec![("w:countBy", count_by.to_string())];
        if start > 0 {
            attributes.push(("w:start", start.to_string()));
        }
        if let Some(distance) = distance {
            attributes.push(("w:distance", distance.to_string()));
        }
        attributes.push(("w:restart", restart.to_string()));
        self.set_sect_pr_element(section, "w:lnNumType", Some(attributes))
    }

    /// 文字数と行数: the grid type, the line pitch and the character pitch.
    pub fn set_section_doc_grid(
        &mut self,
        section: Option<usize>,
        grid: &HeadlessDocGrid,
        default_font_size: f32,
    ) -> Result<(), String> {
        let grid_type = match grid.grid_type.as_str() {
            "default" | "lines" | "linesAndChars" | "snapToChars" => grid.grid_type.clone(),
            other => return Err(format!("unknown document grid: {other}")),
        };
        let line_pitch = grid
            .line_pitch
            .filter(|pitch| pitch.is_finite() && *pitch > 0.0)
            .map(|pitch| (pitch * 20.0).round() / 20.0);
        let char_space = grid
            .char_pitch
            .filter(|pitch| pitch.is_finite() && *pitch > 0.0)
            .filter(|_| matches!(grid_type.as_str(), "linesAndChars" | "snapToChars"))
            .map(|pitch| ((pitch - default_font_size) * 4096.0).round() as i64);
        let model = DocGrid {
            grid_type: (grid_type != "default").then(|| grid_type.clone()),
            line_pitch,
            char_space: char_space.map(|space| space as f32 / 4096.0),
        };
        self.edit_section_properties(section, |properties| {
            properties.doc_grid = Some(model.clone())
        });
        let mut attributes = Vec::new();
        if grid_type != "default" {
            attributes.push(("w:type", grid_type));
        }
        if let Some(pitch) = line_pitch {
            attributes.push(("w:linePitch", twips(pitch)));
        }
        if let Some(space) = char_space {
            attributes.push(("w:charSpace", space.to_string()));
        }
        self.set_sect_pr_element(section, "w:docGrid", Some(attributes))
    }

    /// ページ番号の書式: format, start and chapter number.
    pub fn set_section_page_numbering(
        &mut self,
        section: Option<usize>,
        numbering: &HeadlessPageNumbering,
    ) -> Result<(), String> {
        let format = crate::number_format::authorable_number_format(&numbering.format)
            .ok_or_else(|| format!("unsupported page-number format: {}", numbering.format))?
            .to_string();
        if numbering.start == Some(0) {
            return Err("page-number start must be at least 1".to_string());
        }
        let chapter_style = numbering.chapter_style.filter(|level| (1..=9).contains(level));
        let chapter_separator = chapter_style.and_then(|_| {
            numbering
                .chapter_separator
                .clone()
                .filter(|separator| {
                    matches!(
                        separator.as_str(),
                        "hyphen" | "period" | "colon" | "emDash" | "enDash"
                    )
                })
        });
        let model = crate::document::PageNumberType {
            format: Some(format.to_ascii_lowercase()),
            start: numbering.start,
            chapter_style,
            chapter_separator: chapter_separator.clone(),
        };
        self.edit_section_properties(section, |properties| {
            properties.page_number_type = Some(model.clone())
        });
        let mut attributes = vec![("w:fmt", format)];
        if let Some(start) = numbering.start {
            attributes.push(("w:start", start.to_string()));
        }
        if let Some(level) = chapter_style {
            attributes.push(("w:chapStyle", level.to_string()));
            if let Some(separator) = chapter_separator {
                attributes.push(("w:chapSep", separator));
            }
        }
        self.set_sect_pr_element(section, "w:pgNumType", Some(attributes))
    }

    /// 1 ページの行数を指定時に文字を行グリッド線に合わせる: whether the
    /// paragraphs `start..=end` snap to the section's document grid.
    pub fn set_paragraphs_snap_to_grid(
        &mut self,
        start: usize,
        end: usize,
        snap: bool,
    ) -> Result<(), String> {
        let (start, end) = (start.min(end), start.max(end));
        if end >= self.paragraph_count() {
            return Err("paragraph out of range".to_string());
        }
        for index in start..=end {
            let paragraph = self
                .paragraph_mut(index)
                .ok_or_else(|| "paragraph out of range".to_string())?;
            paragraph
                .properties
                .get_or_insert_with(ParagraphProperties::default)
                .snap_to_grid = (!snap).then_some(false);
            crate::commands::paragraph_property_sync::sync_paragraph_snap_to_grid_xml(
                self, index, snap,
            );
        }
        Ok(())
    }

    /// The text of the first body paragraph in the style named `style` (by
    /// name, as Word writes built-in ones, or by id): what a STYLEREF to it
    /// shows before a page lays it out.
    pub fn first_paragraph_text_in_style(&self, style: &str) -> Option<String> {
        let id = self
            .styles_part
            .as_ref()
            .and_then(|styles| {
                styles
                    .paragraph_style_definitions()
                    .find(|definition| {
                        definition
                            .name
                            .as_deref()
                            .is_some_and(|name| name.eq_ignore_ascii_case(style))
                    })
                    .map(|definition| definition.style_id.clone())
                    .or_else(|| styles.resolve_paragraph_style_id(style).map(str::to_string))
            })
            .unwrap_or_else(|| style.to_string());
        (0..self.paragraph_count()).find_map(|index| {
            let paragraph = self.paragraph(index)?;
            let paragraph_style = paragraph.properties.as_ref()?.style.as_deref()?;
            paragraph_style.eq_ignore_ascii_case(&id).then(|| {
                paragraph
                    .runs
                    .iter()
                    .flat_map(|run| run.text.chars())
                    .filter(|ch| !ch.is_control())
                    .collect()
            })
        })
    }
}
