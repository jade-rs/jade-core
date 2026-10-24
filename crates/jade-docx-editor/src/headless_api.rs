use crate::commands::apply_bold::ApplyBoldCommand;
use crate::commands::apply_italic::ApplyItalicCommand;
use crate::commands::apply_multilevel_list::ApplyMultilevelListCommand;
use crate::commands::apply_strike::ApplyStrikeCommand;
use crate::commands::apply_subscript::ApplySubscriptCommand;
use crate::commands::apply_superscript::ApplySuperscriptCommand;
use crate::commands::apply_underline::ApplyUnderlineCommand;
use crate::commands::change_case::ChangeCaseCommand;
use crate::commands::clear_formatting::ClearFormattingCommand;
use crate::commands::delete_object::DeleteObjectCommand;
use crate::commands::delete_table::DeleteTableCommand;
use crate::commands::delete_text::{DeleteDirection, DeleteTextRunCommand};
use crate::commands::insert_blank_page::InsertBlankPageCommand;
use crate::commands::insert_caption::InsertCaptionCommand;
use crate::commands::insert_citation::{
    AddCitationSourceCommand, InsertBibliographyCommand, InsertCitationCommand,
};
use crate::commands::insert_cover_page::InsertCoverPageCommand;
use crate::commands::insert_endnote::{
    delete_endnote_command, update_endnote_command, InsertEndnoteCommand,
};
use crate::commands::insert_equation::InsertEquationCommand;
use crate::commands::insert_field::InsertFieldCommand;
use crate::commands::insert_footnote::{
    convert_note_command, delete_footnote_command, update_footnote_command, InsertFootnoteCommand,
};
use crate::commands::insert_index::{InsertIndexCommand, MarkIndexEntryCommand};
use crate::commands::insert_paragraph_break::InsertParagraphBreaksCommand;
use crate::commands::insert_section_break::InsertSectionBreakCommand;
use crate::commands::insert_shape::InsertShapeCommand;
use crate::commands::insert_table::InsertTableCommand;
use crate::commands::insert_text::InsertTextCommand;
use crate::commands::insert_toc::{InsertTableOfContentsCommand, UpdateTableOfContentsCommand};
use crate::commands::merge_table_cells::{MergeTableCellsCommand, TableCellSelection};
use crate::commands::replace_equation::ReplaceEquationCommand;
use crate::commands::resize_table_column::ResizeTableColumnCommand;
use crate::commands::resize_table_row::ResizeTableRowCommand;
use crate::commands::set_bold::SetBoldCommand;
use crate::commands::set_character_style::SetCharacterStyleCommand;
use crate::commands::set_document_protection::SetDocumentProtectionCommand;
use crate::commands::set_document_setting::SetDocumentSettingCommand;
use crate::commands::set_document_theme::SetDocumentThemeCommand;
use crate::commands::set_drop_cap::SetDropCapCommand;
use crate::commands::set_font_color::SetFontColorCommand;
use crate::commands::set_font_family::SetFontFamilyCommand;
use crate::commands::set_font_size::SetFontSizeCommand;
use crate::commands::set_header_footer_variants::SetHeaderFooterVariantsCommand;
use crate::commands::set_highlight_color::SetHighlightColorCommand;
use crate::commands::set_numbering::SetNumberingCommand;
use crate::commands::set_page_borders::SetPageBordersCommand;
use crate::commands::set_page_setup::{
    SetPageBackgroundColorCommand, SetPageMarginsCommand, SetPageOrientationCommand,
    SetPageSizeCommand,
};
use crate::commands::set_paragraph_alignment::SetParagraphAlignmentCommand;
use crate::commands::set_paragraph_direction::SetParagraphDirectionCommand;
use crate::commands::set_paragraph_indent::SetParagraphIndentCommand;
use crate::commands::set_paragraph_line_spacing::SetParagraphLineSpacingCommand;
use crate::commands::set_paragraph_metrics::{SetParagraphMetricsCommand, SpecialIndent};
use crate::commands::set_paragraph_options::{ParagraphOptions, SetParagraphOptionsCommand};
use crate::commands::set_paragraph_style::SetParagraphStyleCommand;
use crate::commands::set_paragraph_visuals::SetParagraphVisualsCommand;
use crate::commands::set_ruby::SetRubyCommand;
use crate::commands::set_run_format::SetRunFormatCommand;
use crate::commands::set_typography::{
    SetJapaneseTypographyCommand, SetParagraphTypographyCommand,
};
use crate::commands::set_watermark::SetWatermarkCommand;
use crate::commands::snapshot_edit::{EditScope, PackageSnapshotCommand, SnapshotEditCommand};
use crate::commands::split_table::SplitTableCommand;
use crate::commands::table_structure::{TableStructureCommand, TableStructureOp};
use crate::commands::unmerge_table_cells::UnmergeTableCellsCommand;
use crate::commands::{Command, CommandContext, CommandError, CommandExecutor};
use crate::document::{
    BodyElement, DropCapKind, Paragraph, RevisionKind as DocumentRevisionKind, Run, RunProperties,
    RunTextBox, Table, TableCell, TableCellMargins, TableRow, TextBoxShape,
};
use crate::legal_structure::{
    article_title, build_chunks, find_references, full_width_digit_to_ascii, is_article_heading,
    is_ascii_or_full_width_digit, ChunkOptions, ParagraphInput,
};
use crate::numbering::{MultilevelListPreset, NumberingCounter, NumberingIndent, NumberingLevel};
use crate::word_document::{
    AddCommentRequest, BorderEdgeSpec, CitationSourceInfo, CoverPageParagraphSpec,
    DocumentProtectionInfo, DocumentThemeSpec, EquationPlacement, FloatingObjectKind,
    FloatingObjectTarget, PageBorderSpec, PageMarginSpec, ReplyToCommentRequest, RevisionIdFilter,
    RevisionSession, RevisionTargets, SectionBreakType, ShapeInsertSpec, TableBorderSpec,
    WordDocument,
};
use jade_docx_headless_api::{
    HeadlessCharacterStyleSnapshot, HeadlessCitationSource, HeadlessCommand, HeadlessCommentAnchor,
    HeadlessCommentSnapshot, HeadlessCommentThreadSnapshot, HeadlessCoverParagraph,
    HeadlessCustomNumberingLevel, HeadlessDeleteDirection, HeadlessDocumentProtection,
    HeadlessDocumentTheme, HeadlessDropCapKind, HeadlessEditRequest, HeadlessEquationPlacement,
    HeadlessHeaderFooterLine, HeadlessHeaderFooterSnapshot, HeadlessMultilevelListPreset,
    HeadlessOutlineItem, HeadlessPageSetupSnapshot, HeadlessParagraphIndent,
    HeadlessParagraphSnapshot, HeadlessParagraphSpacing, HeadlessParagraphText, HeadlessQuery,
    HeadlessQueryRequest, HeadlessQueryResponse, HeadlessQueryResult, HeadlessRevisionKindFilter,
    HeadlessRevisionSnapshot, HeadlessRevisionTarget, HeadlessRunSnapshot, HeadlessRunStyle,
    HeadlessSearchMatch, HeadlessSectionBreakType, HeadlessShapeKind, HeadlessStyleKindFilter,
    HeadlessStyleSnapshot, HeadlessTableCellDeleteDirection, HeadlessTableCellInsertDirection,
    HeadlessTableCellSelection, HeadlessTableCellSnapshot, HeadlessTableColumnDetailSnapshot,
    HeadlessTableColumnSnapshot, HeadlessTableFitMode, HeadlessTableRowSnapshot,
    HeadlessTableSnapshot, HeadlessTableTextWrapping,
};
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use jade_docx_reader::docx_xml::{XmlElement, XmlNode};
use jade_docx_reader::styles::StyleDefinition;
use std::collections::BTreeMap;
use unicode_segmentation::UnicodeSegmentation;

#[derive(thiserror::Error, Debug)]
pub enum HeadlessApiApplyError {
    #[error("the command list is empty")]
    EmptyCommands,
    #[error("headless command failed at index {index}: {source}")]
    Command {
        index: usize,
        #[source]
        source: CommandError,
    },
}

#[derive(thiserror::Error, Debug)]
pub enum HeadlessApiQueryError {
    #[error("the query list is empty")]
    EmptyQueries,
    #[error("headless query failed at index {index}: {message}")]
    InvalidQuery { index: usize, message: String },
}

impl From<HeadlessDeleteDirection> for DeleteDirection {
    fn from(value: HeadlessDeleteDirection) -> Self {
        match value {
            HeadlessDeleteDirection::Backward => DeleteDirection::Backward,
            HeadlessDeleteDirection::Forward => DeleteDirection::Forward,
        }
    }
}

impl From<HeadlessDocumentTheme> for DocumentThemeSpec {
    fn from(value: HeadlessDocumentTheme) -> Self {
        Self {
            name: value.name,
            major_font: value.major_font,
            minor_font: value.minor_font,
            east_asia_font: value.east_asia_font,
            dark2: value.dark2,
            light2: value.light2,
            accent1: value.accent1,
            accent2: value.accent2,
            accent3: value.accent3,
            accent4: value.accent4,
            accent5: value.accent5,
            accent6: value.accent6,
        }
    }
}

fn text_box_shape(value: HeadlessShapeKind) -> TextBoxShape {
    match value {
        HeadlessShapeKind::Rectangle => TextBoxShape::Rectangle,
        HeadlessShapeKind::Line => TextBoxShape::Line,
        HeadlessShapeKind::LineArrow => TextBoxShape::LineArrow,
        HeadlessShapeKind::LineArrowDouble => TextBoxShape::LineArrowDouble,
        HeadlessShapeKind::LineBent => TextBoxShape::LineBent,
        HeadlessShapeKind::LineCurved => TextBoxShape::LineCurved,
        HeadlessShapeKind::RoundedRectangle => TextBoxShape::RoundedRectangle,
        HeadlessShapeKind::Ellipse => TextBoxShape::Ellipse,
        HeadlessShapeKind::Diamond => TextBoxShape::Diamond,
        HeadlessShapeKind::Triangle => TextBoxShape::Triangle,
        HeadlessShapeKind::RightTriangle => TextBoxShape::RightTriangle,
        HeadlessShapeKind::Parallelogram => TextBoxShape::Parallelogram,
        HeadlessShapeKind::Trapezoid => TextBoxShape::Trapezoid,
        HeadlessShapeKind::Pentagon => TextBoxShape::Pentagon,
        HeadlessShapeKind::Hexagon => TextBoxShape::Hexagon,
        HeadlessShapeKind::Octagon => TextBoxShape::Octagon,
        HeadlessShapeKind::Plus => TextBoxShape::Plus,
        HeadlessShapeKind::MathPlus => TextBoxShape::MathPlus,
        HeadlessShapeKind::LeftBrace => TextBoxShape::LeftBrace,
        HeadlessShapeKind::RightBrace => TextBoxShape::RightBrace,
        HeadlessShapeKind::BlockArc => TextBoxShape::BlockArc,
        HeadlessShapeKind::Pie => TextBoxShape::Pie,
        HeadlessShapeKind::Chord => TextBoxShape::Chord,
        HeadlessShapeKind::Teardrop => TextBoxShape::Teardrop,
        HeadlessShapeKind::Frame => TextBoxShape::Frame,
        HeadlessShapeKind::HalfFrame => TextBoxShape::HalfFrame,
        HeadlessShapeKind::Corner => TextBoxShape::Corner,
        HeadlessShapeKind::DiagonalStripe => TextBoxShape::DiagonalStripe,
        HeadlessShapeKind::Donut => TextBoxShape::Donut,
        HeadlessShapeKind::NoSmoking => TextBoxShape::NoSmoking,
        HeadlessShapeKind::FoldedCorner => TextBoxShape::FoldedCorner,
        HeadlessShapeKind::Bevel => TextBoxShape::Bevel,
        HeadlessShapeKind::Cube => TextBoxShape::Cube,
        HeadlessShapeKind::Can => TextBoxShape::Can,
        HeadlessShapeKind::LightningBolt => TextBoxShape::LightningBolt,
        HeadlessShapeKind::Heart => TextBoxShape::Heart,
        HeadlessShapeKind::Sun => TextBoxShape::Sun,
        HeadlessShapeKind::Moon => TextBoxShape::Moon,
        HeadlessShapeKind::Cloud => TextBoxShape::Cloud,
        HeadlessShapeKind::Arc => TextBoxShape::Arc,
        HeadlessShapeKind::Plaque => TextBoxShape::Plaque,
        HeadlessShapeKind::SmileyFace => TextBoxShape::SmileyFace,
        HeadlessShapeKind::LeftBracket => TextBoxShape::LeftBracket,
        HeadlessShapeKind::RightBracket => TextBoxShape::RightBracket,
        HeadlessShapeKind::FourPointStar => TextBoxShape::FourPointStar,
        HeadlessShapeKind::FivePointStar => TextBoxShape::FivePointStar,
        HeadlessShapeKind::SixPointStar => TextBoxShape::SixPointStar,
        HeadlessShapeKind::SevenPointStar => TextBoxShape::SevenPointStar,
        HeadlessShapeKind::EightPointStar => TextBoxShape::EightPointStar,
        HeadlessShapeKind::TenPointStar => TextBoxShape::TenPointStar,
        HeadlessShapeKind::TwelvePointStar => TextBoxShape::TwelvePointStar,
        HeadlessShapeKind::SixteenPointStar => TextBoxShape::SixteenPointStar,
        HeadlessShapeKind::TwentyFourPointStar => TextBoxShape::TwentyFourPointStar,
        HeadlessShapeKind::ThirtyTwoPointStar => TextBoxShape::ThirtyTwoPointStar,
        HeadlessShapeKind::IrregularSealOne => TextBoxShape::IrregularSealOne,
        HeadlessShapeKind::IrregularSealTwo => TextBoxShape::IrregularSealTwo,
        HeadlessShapeKind::Ribbon => TextBoxShape::Ribbon,
        HeadlessShapeKind::RibbonTwo => TextBoxShape::RibbonTwo,
        HeadlessShapeKind::Wave => TextBoxShape::Wave,
        HeadlessShapeKind::DoubleWave => TextBoxShape::DoubleWave,
        HeadlessShapeKind::FlowChartProcess => TextBoxShape::FlowChartProcess,
        HeadlessShapeKind::FlowChartAlternateProcess => TextBoxShape::FlowChartAlternateProcess,
        HeadlessShapeKind::FlowChartDecision => TextBoxShape::FlowChartDecision,
        HeadlessShapeKind::FlowChartPredefinedProcess => TextBoxShape::FlowChartPredefinedProcess,
        HeadlessShapeKind::FlowChartInternalStorage => TextBoxShape::FlowChartInternalStorage,
        HeadlessShapeKind::FlowChartDocument => TextBoxShape::FlowChartDocument,
        HeadlessShapeKind::FlowChartMultidocument => TextBoxShape::FlowChartMultidocument,
        HeadlessShapeKind::FlowChartTerminator => TextBoxShape::FlowChartTerminator,
        HeadlessShapeKind::FlowChartPreparation => TextBoxShape::FlowChartPreparation,
        HeadlessShapeKind::FlowChartManualInput => TextBoxShape::FlowChartManualInput,
        HeadlessShapeKind::FlowChartManualOperation => TextBoxShape::FlowChartManualOperation,
        HeadlessShapeKind::FlowChartConnector => TextBoxShape::FlowChartConnector,
        HeadlessShapeKind::FlowChartOffpageConnector => TextBoxShape::FlowChartOffpageConnector,
        HeadlessShapeKind::FlowChartMagneticDisk => TextBoxShape::FlowChartMagneticDisk,
        HeadlessShapeKind::FlowChartMagneticDrum => TextBoxShape::FlowChartMagneticDrum,
        HeadlessShapeKind::FlowChartDisplay => TextBoxShape::FlowChartDisplay,
        HeadlessShapeKind::FlowChartDelay => TextBoxShape::FlowChartDelay,
        HeadlessShapeKind::FlowChartOr => TextBoxShape::FlowChartOr,
        HeadlessShapeKind::FlowChartCollate => TextBoxShape::FlowChartCollate,
        HeadlessShapeKind::FlowChartSort => TextBoxShape::FlowChartSort,
        HeadlessShapeKind::FlowChartExtract => TextBoxShape::FlowChartExtract,
        HeadlessShapeKind::FlowChartMerge => TextBoxShape::FlowChartMerge,
        HeadlessShapeKind::FlowChartSummingJunction => TextBoxShape::FlowChartSummingJunction,
        HeadlessShapeKind::FlowChartPunchedTape => TextBoxShape::FlowChartPunchedTape,
        HeadlessShapeKind::WedgeRectangleCallout => TextBoxShape::WedgeRectangleCallout,
        HeadlessShapeKind::WedgeRoundedRectangleCallout => {
            TextBoxShape::WedgeRoundedRectangleCallout
        }
        HeadlessShapeKind::WedgeEllipseCallout => TextBoxShape::WedgeEllipseCallout,
        HeadlessShapeKind::CloudCallout => TextBoxShape::CloudCallout,
        HeadlessShapeKind::RightArrow => TextBoxShape::RightArrow,
        HeadlessShapeKind::LeftArrow => TextBoxShape::LeftArrow,
        HeadlessShapeKind::UpArrow => TextBoxShape::UpArrow,
        HeadlessShapeKind::DownArrow => TextBoxShape::DownArrow,
        HeadlessShapeKind::LeftRightArrow => TextBoxShape::LeftRightArrow,
        HeadlessShapeKind::UpDownArrow => TextBoxShape::UpDownArrow,
        HeadlessShapeKind::NotchedRightArrow => TextBoxShape::NotchedRightArrow,
        HeadlessShapeKind::Chevron => TextBoxShape::Chevron,
        HeadlessShapeKind::HomePlate => TextBoxShape::HomePlate,
        HeadlessShapeKind::QuadArrow => TextBoxShape::QuadArrow,
        HeadlessShapeKind::BentArrow => TextBoxShape::BentArrow,
        HeadlessShapeKind::UturnArrow => TextBoxShape::UturnArrow,
        HeadlessShapeKind::CurvedRightArrow => TextBoxShape::CurvedRightArrow,
        HeadlessShapeKind::StripedRightArrow => TextBoxShape::StripedRightArrow,
        HeadlessShapeKind::SnipOneRectangle => TextBoxShape::SnipOneRectangle,
        HeadlessShapeKind::SnipTwoSameRectangle => TextBoxShape::SnipTwoSameRectangle,
        HeadlessShapeKind::SnipTwoDiagonalRectangle => TextBoxShape::SnipTwoDiagonalRectangle,
        HeadlessShapeKind::SnipRoundRectangle => TextBoxShape::SnipRoundRectangle,
        HeadlessShapeKind::RoundOneRectangle => TextBoxShape::RoundOneRectangle,
        HeadlessShapeKind::RoundTwoSameRectangle => TextBoxShape::RoundTwoSameRectangle,
        HeadlessShapeKind::RoundTwoDiagonalRectangle => TextBoxShape::RoundTwoDiagonalRectangle,
    }
}

impl From<HeadlessSectionBreakType> for SectionBreakType {
    fn from(value: HeadlessSectionBreakType) -> Self {
        match value {
            HeadlessSectionBreakType::NextPage => Self::NextPage,
            HeadlessSectionBreakType::Continuous => Self::Continuous,
            HeadlessSectionBreakType::EvenPage => Self::EvenPage,
            HeadlessSectionBreakType::OddPage => Self::OddPage,
        }
    }
}

impl From<HeadlessCoverParagraph> for CoverPageParagraphSpec {
    fn from(value: HeadlessCoverParagraph) -> Self {
        Self {
            text: value.text,
            bold: value.bold,
            italic: value.italic,
            color: value.color,
            font_size: value.font_size,
            alignment: value.alignment,
            space_before: value.space_before,
            space_after: value.space_after,
            shading_color: value.shading_color,
            border_top: value.border_top,
            border_bottom: value.border_bottom,
            border_left: value.border_left,
            border_right: value.border_right,
            indent_left: value.indent_left,
            indent_right: value.indent_right,
        }
    }
}

/// Runs a review-side mutation — tracked changes, comments — as one undoable
/// history step.
///
/// These write to `comments.xml` and its siblings as well as to the body, so
/// they cannot be reversed by replaying an inverse text edit; they keep the
/// state `scope` says they can change instead. See
/// [`crate::commands::snapshot_edit`]. Routing them through the executor is
/// also what makes `can_undo` honest: before this, an `accept_revisions`
/// reported the *previous* edit's undoability and undoing it silently reverted
/// that unrelated edit.
fn execute_snapshot_edit(
    word_doc: &mut WordDocument,
    command_executor: &mut CommandExecutor,
    selection: &mut Option<SelectionRange>,
    index: usize,
    name: &'static str,
    scope: EditScope,
    mutate: impl FnMut(&mut WordDocument) -> Result<(), String> + 'static,
) -> Result<(), HeadlessApiApplyError> {
    let mut ctx = CommandContext::new(word_doc, selection);
    command_executor
        .execute(
            Box::new(SnapshotEditCommand::with_scope(name, scope, mutate)),
            &mut ctx,
        )
        .map_err(|source| HeadlessApiApplyError::Command { index, source })
}

/// The scope of an edit confined to one paragraph, or to the paragraphs
/// between two (in either order).
fn paragraphs_scope(first: usize, last: usize) -> EditScope {
    EditScope::Paragraphs(Some((first.min(last), first.max(last))))
}

/// Checks an offset a caller supplied against the paragraph it names.
///
/// Byte offsets come out of an earlier read, and a document that has moved on
/// since then can make one point past the end of a paragraph or into the middle
/// of a character — the latter is easy to hit in Japanese, where one character
/// is three bytes. Both used to be accepted silently: the edit simply landed
/// somewhere else. For a contract that is a wrong document rather than a failed
/// call, so the command is rejected and the message says what the paragraph
/// actually looks like.
///
/// Offsets arrive in the *rendered* space. The first paragraph of a footnote
/// body renders the note number in front of its runs, and every offset the
/// editor hands out for it — caret, hit test, search result — counts that
/// number even though no run holds it. `InsertTextCommand` converts back to
/// model offsets before touching the document, so the check has to run in the
/// same space the commands accept. Comparing a rendered offset against the run
/// text alone rejects the position right after an empty footnote's tab, which
/// is exactly where the caret lands when the footnote is inserted.
fn validate_text_offset(
    word_doc: &WordDocument,
    index: usize,
    paragraph: usize,
    offset: usize,
) -> Result<(), HeadlessApiApplyError> {
    let text =
        paragraph_text(word_doc, paragraph).ok_or_else(|| HeadlessApiApplyError::Command {
            index,
            source: CommandError::Custom(format!("paragraph {paragraph} does not exist")),
        })?;
    let rendered_len = word_doc
        .model_to_rendered_text_position(TextPosition::new(paragraph, text.len()))
        .offset;
    if offset > rendered_len {
        return Err(HeadlessApiApplyError::Command {
            index,
            source: CommandError::Custom(format!(
                "offset {offset} is past the end of paragraph {paragraph}, which is {rendered_len} bytes long",
            )),
        });
    }
    let model_offset = word_doc
        .rendered_to_model_text_position(TextPosition::new(paragraph, offset))
        .offset;
    if !text.is_char_boundary(model_offset) {
        return Err(HeadlessApiApplyError::Command {
            index,
            source: CommandError::Custom(format!(
                "offset {offset} is inside a character in paragraph {paragraph}; \
                 offsets are UTF-8 byte offsets, and a search_text result gives valid ones"
            )),
        });
    }
    Ok(())
}

/// The wire form of one border edge, as the model layer wants it.
fn border_edge_spec(edge: &jade_docx_headless_api::HeadlessBorderEdge) -> BorderEdgeSpec {
    BorderEdgeSpec {
        style: edge.style.clone(),
        width: edge.width,
        color: edge.color.clone(),
    }
}

fn paragraph_border_edge(
    edge: &jade_docx_headless_api::HeadlessBorderEdge,
) -> Result<crate::document::BorderEdge, CommandError> {
    let style = edge
        .style
        .as_deref()
        .unwrap_or("single")
        .trim()
        .to_ascii_lowercase();
    if style.is_empty() || style == "none" || style == "nil" {
        return Err(CommandError::Custom(
            "paragraph border sides must use a visible style".into(),
        ));
    }
    let width = edge.width.unwrap_or(0.75);
    if !width.is_finite() || !(0.125..=12.0).contains(&width) {
        return Err(CommandError::Custom(
            "paragraph border width must be between 0.125 and 12 points".into(),
        ));
    }
    let color = edge.color.as_deref().unwrap_or("#000000").trim();
    let color = if color.eq_ignore_ascii_case("auto") {
        None
    } else {
        let hex = color.strip_prefix('#').unwrap_or(color);
        if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CommandError::Custom(
                "paragraph border color must be #RRGGBB or auto".into(),
            ));
        }
        Some(format!("#{}", hex.to_ascii_uppercase()))
    };
    Ok(crate::document::BorderEdge {
        val: Some(style),
        size_eighths: Some((width * 8.0).round().max(1.0) as u32),
        space: Some(1),
        color,
    })
}

/// The headless form of [`ParagraphOptions`], checked.
fn paragraph_options(
    tabs: Option<&[jade_docx_headless_api::HeadlessTabStop]>,
    contextual_spacing: Option<bool>,
    suppress_line_numbers: Option<bool>,
    suppress_auto_hyphens: Option<bool>,
    borders: Option<&jade_docx_headless_api::HeadlessParagraphBorderBox>,
    shading: Option<&Option<jade_docx_headless_api::HeadlessParagraphShading>>,
) -> Result<ParagraphOptions, CommandError> {
    use crate::commands::set_paragraph_options::{border_line, shading as shading_of, tab_stop};
    let line = |line: Option<&jade_docx_headless_api::HeadlessParagraphBorderLine>| {
        line.map(|line| border_line(&line.style, line.width, line.color.as_deref(), line.space))
            .transpose()
    };
    Ok(ParagraphOptions {
        tabs: tabs
            .map(|tabs| {
                tabs.iter()
                    .map(|tab| {
                        tab_stop(
                            tab.position,
                            tab.alignment.as_deref(),
                            tab.leader.as_deref(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?,
        contextual_spacing,
        suppress_line_numbers,
        suppress_auto_hyphens,
        borders: borders
            .map(|borders| -> Result<_, CommandError> {
                Ok(crate::document::ParagraphBorders {
                    top: line(borders.top.as_ref())?,
                    bottom: line(borders.bottom.as_ref())?,
                    left: line(borders.left.as_ref())?,
                    right: line(borders.right.as_ref())?,
                    between: line(borders.between.as_ref())?,
                    bar: None,
                })
            })
            .transpose()?,
        shading: shading
            .map(|shading| {
                shading
                    .as_ref()
                    .map(|shading| {
                        shading_of(
                            shading.pattern.as_deref(),
                            shading.color.as_deref(),
                            shading.fill.as_deref(),
                        )
                    })
                    .transpose()
            })
            .transpose()?,
    })
}

fn range_from_bounds(
    start_paragraph: usize,
    start_offset: usize,
    end_paragraph: usize,
    end_offset: usize,
) -> SelectionRange {
    SelectionRange::new(
        TextPosition::new(start_paragraph, start_offset),
        TextPosition::new(end_paragraph, end_offset),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SnapshotPosition {
    paragraph: usize,
    offset: usize,
}

impl SnapshotPosition {
    fn new(paragraph: usize, offset: usize) -> Self {
        Self { paragraph, offset }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapshotEffect {
    None(SnapshotPosition),
    Insert(SnapshotPosition),
    Observe {
        start: SnapshotPosition,
        end: SnapshotPosition,
    },
    Replace {
        start: SnapshotPosition,
        end: SnapshotPosition,
    },
}

struct SnapshotCommand {
    original_index: usize,
    command: HeadlessCommand,
    effect: SnapshotEffect,
    inserted_len: Option<usize>,
}

fn citation_source_info(source: &HeadlessCitationSource) -> CitationSourceInfo {
    CitationSourceInfo {
        tag: source.tag.clone(),
        source_type: source.source_type.clone(),
        author: source.author.clone(),
        title: source.title.clone(),
        year: source.year.clone(),
        publisher: source.publisher.clone(),
        url: source.url.clone(),
    }
}

fn headless_citation_source(source: CitationSourceInfo) -> HeadlessCitationSource {
    HeadlessCitationSource {
        tag: source.tag,
        source_type: source.source_type,
        author: source.author,
        title: source.title,
        year: source.year,
        publisher: source.publisher,
        url: source.url,
    }
}

fn document_protection_info(protection: &HeadlessDocumentProtection) -> DocumentProtectionInfo {
    DocumentProtectionInfo {
        edit: protection.edit.clone(),
        enforced: protection.enforced,
        hash: protection.hash.clone(),
        salt: protection.salt.clone(),
        spin_count: protection.spin_count,
        algorithm_sid: protection.algorithm_sid,
    }
}

fn headless_document_protection(protection: DocumentProtectionInfo) -> HeadlessDocumentProtection {
    HeadlessDocumentProtection {
        edit: protection.edit,
        enforced: protection.enforced,
        hash: protection.hash,
        salt: protection.salt,
        spin_count: protection.spin_count,
        algorithm_sid: protection.algorithm_sid,
    }
}

fn snapshot_error(index: usize, message: impl Into<String>) -> HeadlessApiApplyError {
    HeadlessApiApplyError::Command {
        index,
        source: CommandError::Custom(message.into()),
    }
}

fn snapshot_position(
    word_doc: &WordDocument,
    index: usize,
    paragraph: usize,
    offset: usize,
) -> Result<SnapshotPosition, HeadlessApiApplyError> {
    validate_text_offset(word_doc, index, paragraph, offset)?;
    Ok(SnapshotPosition::new(paragraph, offset))
}

fn previous_snapshot_grapheme(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .and_then(|prefix| prefix.grapheme_indices(true).next_back().map(|(at, _)| at))
        .unwrap_or(offset)
}

fn next_snapshot_grapheme(text: &str, offset: usize) -> usize {
    text.get(offset..)
        .and_then(|suffix| {
            suffix
                .grapheme_indices(true)
                .next()
                .map(|(_, grapheme)| offset + grapheme.len())
        })
        .unwrap_or(offset)
}

/// Resolves the original range removed by `delete_text`.
///
/// The command first deletes as many Unicode scalar values as fit in the
/// caret's paragraph, then consumes paragraph boundaries and graphemes one at
/// a time. Mirroring that here lets snapshot batches reject overlap before
/// mutating the live document, including a later command aimed at a paragraph
/// an earlier deletion would remove.
fn snapshot_delete_effect(
    word_doc: &WordDocument,
    index: usize,
    paragraph: usize,
    offset: usize,
    direction: HeadlessDeleteDirection,
    count: usize,
) -> Result<SnapshotEffect, HeadlessApiApplyError> {
    let caret = snapshot_position(word_doc, index, paragraph, offset)?;
    let count = count.max(1);
    let text = paragraph_text(word_doc, paragraph)
        .ok_or_else(|| snapshot_error(index, format!("paragraph {paragraph} does not exist")))?;

    match direction {
        HeadlessDeleteDirection::Backward => {
            let boundaries = text
                .get(..offset)
                .unwrap_or_default()
                .char_indices()
                .map(|(at, _)| at)
                .collect::<Vec<_>>();
            let taken = count.min(boundaries.len());
            let mut start = if taken == 0 {
                caret
            } else {
                SnapshotPosition::new(paragraph, boundaries[boundaries.len() - taken])
            };

            for _ in taken..count {
                if start.offset > 0 {
                    let text = paragraph_text(word_doc, start.paragraph).unwrap_or_default();
                    let previous = previous_snapshot_grapheme(&text, start.offset);
                    if previous == start.offset {
                        break;
                    }
                    start.offset = previous;
                } else if start.paragraph > 0
                    && word_doc.paragraphs_share_a_container(start.paragraph - 1, start.paragraph)
                {
                    start.paragraph -= 1;
                    start.offset =
                        paragraph_text(word_doc, start.paragraph).map_or(0, |text| text.len());
                } else {
                    break;
                }
            }

            Ok(if start == caret {
                SnapshotEffect::None(caret)
            } else {
                SnapshotEffect::Replace { start, end: caret }
            })
        }
        HeadlessDeleteDirection::Forward => {
            let mut end = caret;
            let mut taken = 0usize;
            if let Some(suffix) = text.get(offset..) {
                for (at, character) in suffix.char_indices().take(count) {
                    end.offset = offset + at + character.len_utf8();
                    taken += 1;
                }
            }

            for _ in taken..count {
                let current = paragraph_text(word_doc, end.paragraph).unwrap_or_default();
                if end.offset < current.len() {
                    let next = next_snapshot_grapheme(&current, end.offset);
                    if next == end.offset {
                        break;
                    }
                    end.offset = next;
                } else if end.paragraph + 1 < word_doc.paragraph_count()
                    && word_doc.paragraphs_share_a_container(end.paragraph, end.paragraph + 1)
                {
                    end.paragraph += 1;
                    end.offset = 0;
                } else {
                    break;
                }
            }

            Ok(if end == caret {
                SnapshotEffect::None(caret)
            } else {
                SnapshotEffect::Replace { start: caret, end }
            })
        }
    }
}

fn snapshot_effect(
    word_doc: &WordDocument,
    index: usize,
    command: &HeadlessCommand,
) -> Result<SnapshotEffect, HeadlessApiApplyError> {
    match command {
        HeadlessCommand::InsertText {
            paragraph,
            offset,
            text,
        } => {
            let position = snapshot_position(word_doc, index, *paragraph, *offset)?;
            Ok(if text.is_empty() {
                SnapshotEffect::None(position)
            } else {
                SnapshotEffect::Insert(position)
            })
        }
        HeadlessCommand::ReplaceRange {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            text,
        } => {
            let first = snapshot_position(word_doc, index, *start_paragraph, *start_offset)?;
            let second = snapshot_position(word_doc, index, *end_paragraph, *end_offset)?;
            let (start, end) = if first <= second {
                (first, second)
            } else {
                (second, first)
            };
            Ok(if start == end {
                if text.is_empty() {
                    SnapshotEffect::None(start)
                } else {
                    SnapshotEffect::Insert(start)
                }
            } else {
                SnapshotEffect::Replace { start, end }
            })
        }
        HeadlessCommand::DeleteText {
            paragraph,
            offset,
            direction,
            count,
        } => snapshot_delete_effect(word_doc, index, *paragraph, *offset, *direction, *count),
        HeadlessCommand::InsertParagraphBreak {
            paragraph, offset, ..
        } => snapshot_position(word_doc, index, *paragraph, *offset).map(SnapshotEffect::Insert),
        HeadlessCommand::InsertSectionBreak {
            paragraph, offset, ..
        } => snapshot_position(word_doc, index, *paragraph, *offset).map(SnapshotEffect::Insert),
        HeadlessCommand::InsertEndnote {
            paragraph, offset, ..
        } => snapshot_position(word_doc, index, *paragraph, *offset).map(SnapshotEffect::None),
        HeadlessCommand::UpdateEndnote { .. } | HeadlessCommand::DeleteEndnote { .. } => {
            Ok(SnapshotEffect::None(SnapshotPosition::new(0, 0)))
        }
        HeadlessCommand::InsertFootnote {
            paragraph, offset, ..
        } => snapshot_position(word_doc, index, *paragraph, *offset).map(SnapshotEffect::None),
        HeadlessCommand::UpdateFootnote { .. }
        | HeadlessCommand::DeleteFootnote { .. }
        | HeadlessCommand::ConvertFootnoteToEndnote { .. }
        | HeadlessCommand::ConvertEndnoteToFootnote { .. } => {
            Ok(SnapshotEffect::None(SnapshotPosition::new(0, 0)))
        }
        HeadlessCommand::AddCitationSource { .. }
        | HeadlessCommand::SetDocumentProtection { .. } => {
            Ok(SnapshotEffect::None(SnapshotPosition::new(0, 0)))
        }
        HeadlessCommand::InsertCitation {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        } => {
            let first = snapshot_position(word_doc, index, *start_paragraph, *start_offset)?;
            let second = snapshot_position(word_doc, index, *end_paragraph, *end_offset)?;
            let (start, end) = if first <= second {
                (first, second)
            } else {
                (second, first)
            };
            Ok(if start == end {
                SnapshotEffect::Insert(start)
            } else {
                SnapshotEffect::Replace { start, end }
            })
        }
        HeadlessCommand::ApplyBold {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::ApplyItalic {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::ApplyUnderline {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::ApplyStrike {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::ApplySuperscript {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::ApplySubscript {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::SetFontSize {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::SetFontFamily {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::ClearFormatting {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
        }
        | HeadlessCommand::SetBold {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::SetCharacterStyle {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::SetFontColor {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::SetHighlightColor {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        }
        | HeadlessCommand::SetRunFormat {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            ..
        } => {
            let first = snapshot_position(word_doc, index, *start_paragraph, *start_offset)?;
            let second = snapshot_position(word_doc, index, *end_paragraph, *end_offset)?;
            let (start, end) = if first <= second {
                (first, second)
            } else {
                (second, first)
            };
            Ok(if start == end {
                SnapshotEffect::None(start)
            } else {
                SnapshotEffect::Observe { start, end }
            })
        }
        unsupported => Err(snapshot_error(
            index,
            format!(
                "snapshot coordinates support text edits and text-range formatting; command at \
                 index {index} is {}",
                unsupported.name(),
            ),
        )),
    }
}

fn snapshot_effects_overlap(left: SnapshotEffect, right: SnapshotEffect) -> bool {
    match (left, right) {
        (SnapshotEffect::None(_), _) | (_, SnapshotEffect::None(_)) => false,
        (SnapshotEffect::Observe { .. }, SnapshotEffect::Observe { .. }) => false,
        (SnapshotEffect::Insert(left), SnapshotEffect::Insert(right)) => left == right,
        (SnapshotEffect::Insert(position), SnapshotEffect::Observe { start, end })
        | (SnapshotEffect::Observe { start, end }, SnapshotEffect::Insert(position)) => {
            start <= position && position <= end
        }
        (SnapshotEffect::Insert(position), SnapshotEffect::Replace { start, end })
        | (SnapshotEffect::Replace { start, end }, SnapshotEffect::Insert(position)) => {
            start <= position && position <= end
        }
        (
            SnapshotEffect::Observe {
                start: observed_start,
                end: observed_end,
            },
            SnapshotEffect::Replace {
                start: replaced_start,
                end: replaced_end,
            },
        )
        | (
            SnapshotEffect::Replace {
                start: replaced_start,
                end: replaced_end,
            },
            SnapshotEffect::Observe {
                start: observed_start,
                end: observed_end,
            },
        ) => observed_start < replaced_end && replaced_start < observed_end,
        (
            SnapshotEffect::Replace {
                start: left_start,
                end: left_end,
            },
            SnapshotEffect::Replace {
                start: right_start,
                end: right_end,
            },
        ) => left_start < right_end && right_start < left_end,
    }
}

#[derive(Clone, Copy, Debug)]
enum AppliedSnapshotTransform {
    TextInsert {
        original_at: SnapshotPosition,
        current_at: SnapshotPosition,
        inserted_len: usize,
    },
    ParagraphBreak {
        original_at: SnapshotPosition,
        current_at: SnapshotPosition,
        count: usize,
    },
    Replace {
        original_start: SnapshotPosition,
        original_end: SnapshotPosition,
        current_start: SnapshotPosition,
        current_end: SnapshotPosition,
        inserted_len: usize,
    },
}

fn rebase_snapshot_position(
    mut current: SnapshotPosition,
    original: SnapshotPosition,
    index: usize,
    transforms: &[AppliedSnapshotTransform],
) -> Result<SnapshotPosition, HeadlessApiApplyError> {
    for transform in transforms {
        match *transform {
            AppliedSnapshotTransform::TextInsert {
                original_at,
                current_at,
                inserted_len,
            } => {
                if original > original_at && current.paragraph == current_at.paragraph {
                    current.offset = current.offset.saturating_add(inserted_len);
                }
            }
            AppliedSnapshotTransform::ParagraphBreak {
                original_at,
                current_at,
                count,
            } => {
                if original > original_at {
                    if current.paragraph == current_at.paragraph {
                        current.paragraph = current.paragraph.saturating_add(count);
                        current.offset = current.offset.saturating_sub(current_at.offset);
                    } else if current.paragraph > current_at.paragraph {
                        current.paragraph = current.paragraph.saturating_add(count);
                    }
                }
            }
            AppliedSnapshotTransform::Replace {
                original_start,
                original_end,
                current_start,
                current_end,
                inserted_len,
            } => {
                if original <= original_start {
                    continue;
                }
                if original < original_end {
                    return Err(snapshot_error(
                        index,
                        format!(
                            "snapshot coordinate at paragraph {}, offset {} was removed or \
                             replaced by an earlier command",
                            original.paragraph, original.offset,
                        ),
                    ));
                }

                if current.paragraph == current_end.paragraph {
                    current.paragraph = current_start.paragraph;
                    current.offset = current_start
                        .offset
                        .saturating_add(inserted_len)
                        .saturating_add(current.offset.saturating_sub(current_end.offset));
                } else if current.paragraph > current_end.paragraph {
                    current.paragraph = current.paragraph.saturating_sub(
                        current_end
                            .paragraph
                            .saturating_sub(current_start.paragraph),
                    );
                }
            }
        }
    }
    Ok(current)
}

fn rebase_snapshot_effect(
    effect: SnapshotEffect,
    index: usize,
    transforms: &[AppliedSnapshotTransform],
) -> Result<SnapshotEffect, HeadlessApiApplyError> {
    match effect {
        SnapshotEffect::None(position) => {
            rebase_snapshot_position(position, position, index, transforms)
                .map(SnapshotEffect::None)
        }
        SnapshotEffect::Insert(position) => {
            rebase_snapshot_position(position, position, index, transforms)
                .map(SnapshotEffect::Insert)
        }
        SnapshotEffect::Observe { start, end } => Ok(SnapshotEffect::Observe {
            start: rebase_snapshot_position(start, start, index, transforms)?,
            end: rebase_snapshot_position(end, end, index, transforms)?,
        }),
        SnapshotEffect::Replace { start, end } => Ok(SnapshotEffect::Replace {
            start: rebase_snapshot_position(start, start, index, transforms)?,
            end: rebase_snapshot_position(end, end, index, transforms)?,
        }),
    }
}

fn rebase_snapshot_range(
    start_paragraph: usize,
    start_offset: usize,
    end_paragraph: usize,
    end_offset: usize,
    index: usize,
    transforms: &[AppliedSnapshotTransform],
) -> Result<(SnapshotPosition, SnapshotPosition), HeadlessApiApplyError> {
    let start = SnapshotPosition::new(start_paragraph, start_offset);
    let end = SnapshotPosition::new(end_paragraph, end_offset);
    Ok((
        rebase_snapshot_position(start, start, index, transforms)?,
        rebase_snapshot_position(end, end, index, transforms)?,
    ))
}

fn rebase_snapshot_command(
    command: HeadlessCommand,
    index: usize,
    transforms: &[AppliedSnapshotTransform],
) -> Result<HeadlessCommand, HeadlessApiApplyError> {
    Ok(match command {
        HeadlessCommand::InsertText {
            paragraph,
            offset,
            text,
        } => {
            let position = rebase_snapshot_position(
                SnapshotPosition::new(paragraph, offset),
                SnapshotPosition::new(paragraph, offset),
                index,
                transforms,
            )?;
            HeadlessCommand::InsertText {
                paragraph: position.paragraph,
                offset: position.offset,
                text,
            }
        }
        HeadlessCommand::ReplaceRange {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            text,
        } => {
            let start = SnapshotPosition::new(start_paragraph, start_offset);
            let end = SnapshotPosition::new(end_paragraph, end_offset);
            let start = rebase_snapshot_position(start, start, index, transforms)?;
            let end = rebase_snapshot_position(end, end, index, transforms)?;
            HeadlessCommand::ReplaceRange {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                text,
            }
        }
        HeadlessCommand::DeleteText {
            paragraph,
            offset,
            direction,
            count,
        } => {
            let position = SnapshotPosition::new(paragraph, offset);
            let position = rebase_snapshot_position(position, position, index, transforms)?;
            HeadlessCommand::DeleteText {
                paragraph: position.paragraph,
                offset: position.offset,
                direction,
                count,
            }
        }
        HeadlessCommand::InsertParagraphBreak {
            paragraph,
            offset,
            count,
        } => {
            let position = SnapshotPosition::new(paragraph, offset);
            let position = rebase_snapshot_position(position, position, index, transforms)?;
            HeadlessCommand::InsertParagraphBreak {
                paragraph: position.paragraph,
                offset: position.offset,
                count,
            }
        }
        HeadlessCommand::InsertSectionBreak {
            paragraph,
            offset,
            break_type,
        } => {
            let position = SnapshotPosition::new(paragraph, offset);
            let position = rebase_snapshot_position(position, position, index, transforms)?;
            HeadlessCommand::InsertSectionBreak {
                paragraph: position.paragraph,
                offset: position.offset,
                break_type,
            }
        }
        HeadlessCommand::InsertEndnote {
            paragraph,
            offset,
            text,
        } => {
            let position = rebase_snapshot_position(
                SnapshotPosition::new(paragraph, offset),
                SnapshotPosition::new(paragraph, offset),
                index,
                transforms,
            )?;
            HeadlessCommand::InsertEndnote {
                paragraph: position.paragraph,
                offset: position.offset,
                text,
            }
        }
        HeadlessCommand::InsertFootnote {
            paragraph,
            offset,
            text,
        } => {
            let position = rebase_snapshot_position(
                SnapshotPosition::new(paragraph, offset),
                SnapshotPosition::new(paragraph, offset),
                index,
                transforms,
            )?;
            HeadlessCommand::InsertFootnote {
                paragraph: position.paragraph,
                offset: position.offset,
                text,
            }
        }
        command @ (HeadlessCommand::UpdateEndnote { .. }
        | HeadlessCommand::DeleteEndnote { .. }
        | HeadlessCommand::UpdateFootnote { .. }
        | HeadlessCommand::DeleteFootnote { .. }
        | HeadlessCommand::ConvertFootnoteToEndnote { .. }
        | HeadlessCommand::ConvertEndnoteToFootnote { .. }
        | HeadlessCommand::AddCitationSource { .. }
        | HeadlessCommand::SetDocumentProtection { .. }) => command,
        HeadlessCommand::InsertCitation {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            tag,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::InsertCitation {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                tag,
            }
        }
        HeadlessCommand::ApplyBold {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            enabled,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::ApplyBold {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                enabled,
            }
        }
        HeadlessCommand::ApplyItalic {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            enabled,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::ApplyItalic {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                enabled,
            }
        }
        HeadlessCommand::ApplyUnderline {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            kind,
            enabled,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::ApplyUnderline {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                kind: kind.clone(),
                enabled,
            }
        }
        HeadlessCommand::ApplyStrike {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            double,
            enabled,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::ApplyStrike {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                double,
                enabled,
            }
        }
        HeadlessCommand::ApplySuperscript {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            enabled,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::ApplySuperscript {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                enabled,
            }
        }
        HeadlessCommand::ApplySubscript {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            enabled,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::ApplySubscript {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                enabled,
            }
        }
        HeadlessCommand::SetFontSize {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            font_size,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::SetFontSize {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                font_size,
            }
        }
        HeadlessCommand::SetFontFamily {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            font_family,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::SetFontFamily {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                font_family,
            }
        }
        HeadlessCommand::ClearFormatting {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::ClearFormatting {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
            }
        }
        HeadlessCommand::SetBold {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            value,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::SetBold {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                value,
            }
        }
        HeadlessCommand::SetCharacterStyle {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            style_id,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::SetCharacterStyle {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                style_id,
            }
        }
        HeadlessCommand::SetFontColor {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            color,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::SetFontColor {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                color,
            }
        }
        HeadlessCommand::SetHighlightColor {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            color,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::SetHighlightColor {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                color,
            }
        }
        HeadlessCommand::SetRunFormat {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            format,
        } => {
            let (start, end) = rebase_snapshot_range(
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                index,
                transforms,
            )?;
            HeadlessCommand::SetRunFormat {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                format,
            }
        }
        unsupported => {
            return Err(snapshot_error(
                index,
                format!("unsupported snapshot command: {}", unsupported.name()),
            ));
        }
    })
}

fn applied_snapshot_transform(
    original: SnapshotEffect,
    current: SnapshotEffect,
    command: &HeadlessCommand,
    inserted_len: Option<usize>,
) -> Option<AppliedSnapshotTransform> {
    match (original, current) {
        (SnapshotEffect::None(_), SnapshotEffect::None(_)) => None,
        (SnapshotEffect::Observe { .. }, SnapshotEffect::Observe { .. }) => None,
        (SnapshotEffect::Insert(original_at), SnapshotEffect::Insert(current_at)) => {
            match command {
                HeadlessCommand::InsertParagraphBreak { count, .. } => {
                    Some(AppliedSnapshotTransform::ParagraphBreak {
                        original_at,
                        current_at,
                        count: (*count).max(1),
                    })
                }
                HeadlessCommand::InsertSectionBreak { .. } => {
                    Some(AppliedSnapshotTransform::ParagraphBreak {
                        original_at,
                        current_at,
                        count: 1,
                    })
                }
                HeadlessCommand::InsertText { text, .. }
                | HeadlessCommand::ReplaceRange { text, .. } => {
                    Some(AppliedSnapshotTransform::TextInsert {
                        original_at,
                        current_at,
                        inserted_len: text.len(),
                    })
                }
                HeadlessCommand::InsertCitation { .. } => {
                    Some(AppliedSnapshotTransform::TextInsert {
                        original_at,
                        current_at,
                        inserted_len: inserted_len.unwrap_or_default(),
                    })
                }
                _ => None,
            }
        }
        (
            SnapshotEffect::Replace {
                start: original_start,
                end: original_end,
            },
            SnapshotEffect::Replace {
                start: current_start,
                end: current_end,
            },
        ) => Some(AppliedSnapshotTransform::Replace {
            original_start,
            original_end,
            current_start,
            current_end,
            inserted_len: match command {
                HeadlessCommand::ReplaceRange { text, .. } => text.len(),
                HeadlessCommand::DeleteText { .. } => 0,
                HeadlessCommand::InsertCitation { .. } => inserted_len.unwrap_or_default(),
                _ => 0,
            },
        }),
        _ => None,
    }
}

/// Refuses a batch that would type into a content control whose contents
/// cannot be edited, delete one that cannot be deleted, change a group's
/// fixed text, or edit outside the forms of a document protected for them.
fn refuse_locked_edits(
    word_doc: &WordDocument,
    commands: &[HeadlessCommand],
) -> Result<(), HeadlessApiApplyError> {
    for (index, command) in commands.iter().enumerate() {
        if let Some(reason) = word_doc.headless_edit_refusal(command) {
            return Err(HeadlessApiApplyError::Command {
                index,
                source: CommandError::Custom(reason.to_string()),
            });
        }
    }
    Ok(())
}

/// Text typed over a content control's placeholder, or over a form text
/// box's whole result, replaces it as Word does — out of the placeholder's
/// grey, inside the control or field — as one step. `None` when the batch
/// is not that.
fn type_over_placeholder(
    word_doc: &mut WordDocument,
    command_executor: &mut CommandExecutor,
    selection: Option<SelectionRange>,
    commands: &[HeadlessCommand],
) -> Result<Option<Option<SelectionRange>>, HeadlessApiApplyError> {
    let [command] = commands else {
        return Ok(None);
    };
    let Some(action) = word_doc.placeholder_replacement(command) else {
        return Ok(None);
    };
    let range = selection.unwrap_or_else(|| SelectionRange::collapsed(TextPosition::new(0, 0)));
    let mut selection = Some(range);
    let mut ctx = CommandContext::new(word_doc, &mut selection);
    command_executor
        .execute(Box::new(action.command(range)), &mut ctx)
        .map_err(|source| HeadlessApiApplyError::Command { index: 0, source })?;
    Ok(Some(selection))
}

/// Applies text commands whose coordinates all refer to the document state at
/// the start of the batch.
///
/// Non-overlapping edits keep their caller-provided execution and undo order;
/// only their coordinates are transformed through the earlier edits.
/// Ambiguous edits are rejected during the preflight pass, before the document
/// or undo history changes.
pub fn apply_snapshot_headless_commands_with_executor_to_document(
    word_doc: &mut WordDocument,
    command_executor: &mut CommandExecutor,
    selection: Option<SelectionRange>,
    request: &HeadlessEditRequest,
) -> Result<Option<SelectionRange>, HeadlessApiApplyError> {
    if request.commands.is_empty() {
        return Err(HeadlessApiApplyError::EmptyCommands);
    }
    refuse_locked_edits(word_doc, &request.commands)?;
    if let Some(selection) =
        type_over_placeholder(word_doc, command_executor, selection, &request.commands)?
    {
        return Ok(selection);
    }

    let mut available_sources = word_doc
        .citation_sources()
        .into_iter()
        .map(|source| (source.tag.clone(), source))
        .collect::<BTreeMap<_, _>>();
    let mut commands = Vec::with_capacity(request.commands.len());
    for (original_index, command) in request.commands.iter().cloned().enumerate() {
        let effect = snapshot_effect(word_doc, original_index, &command)?;
        let inserted_len = match &command {
            HeadlessCommand::AddCitationSource { source } => {
                let source = citation_source_info(source);
                available_sources.insert(source.tag.clone(), source);
                None
            }
            HeadlessCommand::InsertCitation { tag, .. } => Some(
                available_sources
                    .get(tag)
                    .ok_or_else(|| snapshot_error(original_index, "citation source was not found"))?
                    .citation_text()
                    .len(),
            ),
            _ => None,
        };
        commands.push(SnapshotCommand {
            original_index,
            command,
            effect,
            inserted_len,
        });
    }

    for right_index in 0..commands.len() {
        for left_index in 0..right_index {
            let left = &commands[left_index];
            let right = &commands[right_index];
            if snapshot_effects_overlap(left.effect, right.effect) {
                return Err(snapshot_error(
                    right.original_index,
                    format!(
                        "snapshot coordinate overlap between command at index {} ({}) and \
                         command at index {} ({}); split dependent edits into sequential batches",
                        left.original_index,
                        left.command.name(),
                        right.original_index,
                        right.command.name(),
                    ),
                ));
            }
        }
    }

    let mut transforms = Vec::with_capacity(commands.len());
    let mut rebased_commands = Vec::with_capacity(commands.len());
    for snapshot_command in commands {
        let current_effect = rebase_snapshot_effect(
            snapshot_command.effect,
            snapshot_command.original_index,
            &transforms,
        )?;
        let command = rebase_snapshot_command(
            snapshot_command.command,
            snapshot_command.original_index,
            &transforms,
        )?;
        if let Some(transform) = applied_snapshot_transform(
            snapshot_command.effect,
            current_effect,
            &command,
            snapshot_command.inserted_len,
        ) {
            transforms.push(transform);
        }
        rebased_commands.push(command);
    }

    apply_headless_commands_with_executor_to_document(
        word_doc,
        command_executor,
        selection,
        &HeadlessEditRequest {
            commands: rebased_commands,
        },
    )
}

fn custom_numbering_levels(values: &[HeadlessCustomNumberingLevel]) -> Vec<NumberingLevel> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| NumberingLevel {
            ilvl: index as u32,
            start: Some(value.start.unwrap_or(1)),
            restart: None,
            num_fmt: Some(value.num_fmt.clone()),
            lvl_text: Some(value.lvl_text.clone()),
            lvl_jc: Some("left".to_string()),
            suffix: Some("tab".to_string()),
            indent: Some(NumberingIndent {
                left: Some(value.indent_left as f32 / 20.0),
                hanging: Some(value.hanging.unwrap_or(360) as f32 / 20.0),
            }),
            paragraph_style: None,
            paragraph_props: None,
            run_props: None,
            language: None,
            is_legal: false,
            picture_bullet: None,
        })
        .collect()
}

pub fn apply_headless_commands_to_document(
    word_doc: &mut WordDocument,
    request: &HeadlessEditRequest,
) -> Result<(), HeadlessApiApplyError> {
    apply_headless_commands_with_selection_to_document(word_doc, None, request).map(|_| ())
}

pub fn apply_headless_commands_with_selection_to_document(
    word_doc: &mut WordDocument,
    initial_selection: Option<SelectionRange>,
    request: &HeadlessEditRequest,
) -> Result<Option<SelectionRange>, HeadlessApiApplyError> {
    let mut command_executor = CommandExecutor::new();
    apply_headless_commands_with_executor_to_document(
        word_doc,
        &mut command_executor,
        initial_selection,
        request,
    )
}

/// The commands Word's plain-text paste performs at a collapsed caret: each
/// line is typed and each line break splits the paragraph. Blank lines become
/// empty paragraphs (empty list items inside a list) — the interactive
/// "Enter on an empty list item ends the list" rule is a keyboard behaviour,
/// not a paste one. The caller applies the batch inside one history group so
/// the paste undoes as a single step, and change tracking records the text
/// and the paragraph marks as insertions.
pub fn plain_text_paste_commands(caret: TextPosition, text: &str) -> Vec<HeadlessCommand> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut commands = Vec::new();
    let mut at = caret;
    for (index, line) in normalized.split('\n').enumerate() {
        if index > 0 {
            commands.push(HeadlessCommand::InsertParagraphBreak {
                paragraph: at.paragraph,
                offset: at.offset,
                count: 1,
            });
            at = TextPosition::new(at.paragraph + 1, 0);
        }
        if !line.is_empty() {
            commands.push(HeadlessCommand::InsertText {
                paragraph: at.paragraph,
                offset: at.offset,
                text: line.to_string(),
            });
            at.offset += line.len();
        }
    }
    commands
}

pub fn apply_headless_commands_with_executor_to_document(
    word_doc: &mut WordDocument,
    command_executor: &mut CommandExecutor,
    mut selection: Option<SelectionRange>,
    request: &HeadlessEditRequest,
) -> Result<Option<SelectionRange>, HeadlessApiApplyError> {
    if request.commands.is_empty() {
        return Err(HeadlessApiApplyError::EmptyCommands);
    }
    refuse_locked_edits(word_doc, &request.commands)?;
    if let Some(selection) =
        type_over_placeholder(word_doc, command_executor, selection, &request.commands)?
    {
        return Ok(selection);
    }

    for (index, command) in request.commands.iter().enumerate() {
        match command {
            HeadlessCommand::InsertText {
                paragraph,
                offset,
                text,
            } => {
                validate_text_offset(word_doc, index, *paragraph, *offset)?;
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                selection = Some(range);

                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertTextCommand::new(range, text.clone(), None)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ClearFormatting {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(Box::new(ClearFormattingCommand::new(range)), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ChangeCase {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                mode,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(Box::new(ChangeCaseCommand::new(range, *mode)), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ReplaceRange {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                text,
            } => {
                validate_text_offset(word_doc, index, *start_paragraph, *start_offset)?;
                validate_text_offset(word_doc, index, *end_paragraph, *end_offset)?;
                // 同じままの前後の文字を変更履歴や書式変更に巻き込まない。
                // 削除済み文字がある選択は、既存履歴を保護する通常の経路に渡す。
                let (start_offset, end_offset, text) = trimmed_replacement(
                    word_doc,
                    *start_paragraph,
                    *start_offset,
                    *end_paragraph,
                    *end_offset,
                    text,
                );
                let range =
                    range_from_bounds(*start_paragraph, start_offset, *end_paragraph, end_offset);
                if start_paragraph == end_paragraph && start_offset == end_offset && text.is_empty()
                {
                    continue;
                }
                selection = Some(range);

                // A replacement that spans paragraphs removes the ones in
                // between, and any comment anchored there goes with them.
                let spans_paragraphs = start_paragraph != end_paragraph;
                let command: Box<dyn Command> =
                    Box::new(InsertTextCommand::new(range, text.to_owned(), None));
                let command: Box<dyn Command> = if spans_paragraphs {
                    Box::new(PackageSnapshotCommand::wrapping_exact_undo(command))
                } else {
                    command
                };
                // The notes whose reference marks the range holds go with
                // them, as in Word.
                let notes = crate::commands::range_notes::notes_in_rendered_range(word_doc, range);
                let command: Box<dyn Command> = if notes.is_empty() {
                    command
                } else {
                    Box::new(crate::commands::range_notes::DeleteRangeNotesCommand::new(
                        command, notes,
                    ))
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(command, &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ReplacePlaceholder {
                placeholder,
                text,
                max_replacements,
            } => {
                if placeholder.is_empty() {
                    return Err(HeadlessApiApplyError::Command {
                        index,
                        source: CommandError::Custom(
                            "replace_placeholder.placeholder must not be empty".to_string(),
                        ),
                    });
                }
                let replacements = placeholder_matches(
                    word_doc,
                    placeholder,
                    max_replacements.unwrap_or(usize::MAX),
                );
                for replacement in replacements.into_iter().rev() {
                    let range = range_from_bounds(
                        replacement.paragraph,
                        replacement.start_offset,
                        replacement.paragraph,
                        replacement.end_offset,
                    );
                    selection = Some(range);

                    let mut ctx = CommandContext::new(word_doc, &mut selection);
                    command_executor
                        .execute(
                            Box::new(InsertTextCommand::new(
                                range,
                                text.clone(),
                                replacement.replacement_properties,
                            )),
                            &mut ctx,
                        )
                        .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
                }
            }
            HeadlessCommand::DeleteText {
                paragraph,
                offset,
                direction,
                count,
            } => {
                validate_text_offset(word_doc, index, *paragraph, *offset)?;
                let direction = DeleteDirection::from(*direction);
                let repeat = (*count).max(1);
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                selection = Some(range);

                // One command, one history step, however many characters and
                // paragraph boundaries the run covers.
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(DeleteTextRunCommand::new(range, direction, repeat)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertParagraphBreak {
                paragraph,
                offset,
                count,
            } => {
                let repeat = (*count).max(1);
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertParagraphBreaksCommand::new(range, repeat)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertSectionBreak {
                paragraph,
                offset,
                break_type,
            } => {
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertSectionBreakCommand::new(range, (*break_type).into())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertEndnote {
                paragraph,
                offset,
                text,
            } => {
                validate_text_offset(word_doc, index, *paragraph, *offset)?;
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertEndnoteCommand::new(range, text.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::UpdateEndnote { endnote_id, text } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(update_endnote_command(endnote_id.clone(), text.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::DeleteEndnote { endnote_id } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(delete_endnote_command(endnote_id.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertFootnote {
                paragraph,
                offset,
                text,
            } => {
                validate_text_offset(word_doc, index, *paragraph, *offset)?;
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertFootnoteCommand::new(range, text.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::UpdateFootnote { footnote_id, text } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(update_footnote_command(footnote_id.clone(), text.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::DeleteFootnote { footnote_id } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(delete_footnote_command(footnote_id.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ConvertFootnoteToEndnote { footnote_id } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(convert_note_command(footnote_id.clone(), true)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ConvertEndnoteToFootnote { endnote_id } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(convert_note_command(endnote_id.clone(), false)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::AddCitationSource { source } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(AddCitationSourceCommand::new(citation_source_info(source))),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertCitation {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                tag,
            } => {
                validate_text_offset(word_doc, index, *start_paragraph, *start_offset)?;
                validate_text_offset(word_doc, index, *end_paragraph, *end_offset)?;
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertCitationCommand::new(range, tag.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertBibliography {
                after_paragraph,
                heading,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertBibliographyCommand::new(
                            *after_paragraph,
                            heading.clone(),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetDocumentProtection { protection } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetDocumentProtectionCommand::new(
                            protection.as_ref().map(document_protection_info),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ApplyBold {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                enabled,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ApplyBoldCommand::new_with_enabled(range, *enabled)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ApplyItalic {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                enabled,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ApplyItalicCommand::new_with_enabled(range, *enabled)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ApplyUnderline {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                kind,
                enabled,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                let command = match (*enabled, kind) {
                    (false, _) => ApplyUnderlineCommand::new_with_enabled(range, false),
                    (true, Some(kind)) => ApplyUnderlineCommand::new_with_kind(range, kind.clone()),
                    (true, None) => ApplyUnderlineCommand::new(range),
                };
                command_executor
                    .execute(Box::new(command), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ApplyStrike {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                double,
                enabled,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                let command = if !*enabled {
                    ApplyStrikeCommand::new_with_enabled(range, false)
                } else if *double {
                    ApplyStrikeCommand::new_double(range)
                } else {
                    ApplyStrikeCommand::new(range)
                };
                command_executor
                    .execute(Box::new(command), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ApplySuperscript {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                enabled,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ApplySuperscriptCommand::new_with_enabled(range, *enabled)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ApplySubscript {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                enabled,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ApplySubscriptCommand::new_with_enabled(range, *enabled)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetFontSize {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                font_size,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetFontSizeCommand::new(range, *font_size)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetFontFamily {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                font_family,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetFontFamilyCommand::new(range, font_family.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetBold {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                value,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(Box::new(SetBoldCommand::new(range, *value)), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphAlignment {
                start_paragraph,
                end_paragraph,
                alignment,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphAlignmentCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            alignment.clone(),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphDirection {
                start_paragraph,
                end_paragraph,
                bidi,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphDirectionCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            *bidi,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetCharacterStyle {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                style_id,
            } => {
                validate_text_offset(word_doc, index, *start_paragraph, *start_offset)?;
                validate_text_offset(word_doc, index, *end_paragraph, *end_offset)?;
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);

                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetCharacterStyleCommand::new(range, style_id.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphStyle {
                start_paragraph,
                end_paragraph,
                style_id,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphStyleCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            style_id.clone(),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetDropCap {
                start_paragraph,
                end_paragraph,
                kind,
                lines,
            } => {
                let kind = kind.map(|kind| match kind {
                    HeadlessDropCapKind::Drop => DropCapKind::Drop,
                    HeadlessDropCapKind::Margin => DropCapKind::Margin,
                });
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetDropCapCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            kind,
                            *lines,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphOptions {
                start_paragraph,
                end_paragraph,
                tabs,
                contextual_spacing,
                suppress_line_numbers,
                suppress_auto_hyphens,
                borders,
                shading,
            } => {
                let options = paragraph_options(
                    tabs.as_deref(),
                    *contextual_spacing,
                    *suppress_line_numbers,
                    *suppress_auto_hyphens,
                    borders.as_ref(),
                    shading.as_ref(),
                )
                .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphOptionsCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            options,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphTabs {
                start_paragraph,
                end_paragraph,
                tabs,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(
                            crate::commands::set_paragraph_tabs::SetParagraphTabsCommand::new(
                                *start_paragraph,
                                *end_paragraph,
                                tabs.clone(),
                            ),
                        ),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphIndent {
                start_paragraph,
                end_paragraph,
                left,
                right,
                first_line,
                hanging,
                left_chars,
                right_chars,
                first_line_chars,
                hanging_chars,
                clear_special,
            } => {
                // Every field the call names is applied: points, character
                // units beside them, and 特殊インデント「なし」 last.
                let cmd = SetParagraphIndentCommand::new(
                    *start_paragraph,
                    *end_paragraph,
                    *left,
                    *right,
                    *first_line,
                    *hanging,
                )
                .with_edge_chars(*left_chars, *right_chars)
                .with_special_chars(*first_line_chars, *hanging_chars)
                .with_clear_special(*clear_special);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(Box::new(cmd), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::AdjustParagraphIndent {
                start_paragraph,
                end_paragraph,
                direction,
                target,
            } => {
                let direction = match direction {
                    jade_docx_headless_api::HeadlessIndentDirection::Increase => {
                        crate::commands::adjust_paragraph_indent::IndentDirection::Increase
                    }
                    jade_docx_headless_api::HeadlessIndentDirection::Decrease => {
                        crate::commands::adjust_paragraph_indent::IndentDirection::Decrease
                    }
                };
                let target = match target {
                    jade_docx_headless_api::HeadlessIndentTarget::Start => {
                        crate::commands::adjust_paragraph_indent::IndentTarget::Start
                    }
                    jade_docx_headless_api::HeadlessIndentTarget::Hanging => {
                        crate::commands::adjust_paragraph_indent::IndentTarget::Hanging
                    }
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(
                            crate::commands::adjust_paragraph_indent::AdjustParagraphIndentCommand::with_target(
                                *start_paragraph,
                                *end_paragraph,
                                direction,
                                target,
                            ),
                        ),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphPagination {
                start_paragraph,
                end_paragraph,
                keep_next,
                keep_lines,
                page_break_before,
                widow_control,
            } => {
                use crate::commands::set_paragraph_pagination::{
                    Pagination, SetParagraphPaginationCommand,
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphPaginationCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            Pagination {
                                keep_next: *keep_next,
                                keep_lines: *keep_lines,
                                page_break_before: *page_break_before,
                                widow_control: *widow_control,
                            },
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ToggleParagraphSpaceBefore {
                start_paragraph,
                end_paragraph,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(
                            crate::commands::toggle_paragraph_space_before::ToggleParagraphSpaceBeforeCommand::new(
                                *start_paragraph,
                                *end_paragraph,
                            ),
                        ),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ResetParagraphFormatting {
                start_paragraph,
                end_paragraph,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(
                            crate::commands::reset_paragraph_formatting::ResetParagraphFormattingCommand::new(
                                *start_paragraph,
                                *end_paragraph,
                            ),
                        ),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::AdjustFontSize {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                direction,
                step,
            } => {
                let direction = match direction {
                    jade_docx_headless_api::HeadlessFontSizeDirection::Grow => {
                        crate::commands::adjust_font_size::FontSizeDirection::Grow
                    }
                    jade_docx_headless_api::HeadlessFontSizeDirection::Shrink => {
                        crate::commands::adjust_font_size::FontSizeDirection::Shrink
                    }
                };
                let step = match step {
                    jade_docx_headless_api::HeadlessFontSizeStep::Ladder => {
                        crate::commands::adjust_font_size::FontSizeStep::Ladder
                    }
                    jade_docx_headless_api::HeadlessFontSizeStep::Point => {
                        crate::commands::adjust_font_size::FontSizeStep::Point
                    }
                };
                let range = SelectionRange::new(
                    TextPosition::new(*start_paragraph, *start_offset),
                    TextPosition::new(*end_paragraph, *end_offset),
                );
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(
                            crate::commands::adjust_font_size::AdjustFontSizeCommand::with_step(
                                range, direction, step,
                            ),
                        ),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphLineSpacing {
                start_paragraph,
                end_paragraph,
                line,
                line_rule,
                before,
                after,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphLineSpacingCommand::with_spacing(
                            *start_paragraph,
                            *end_paragraph,
                            *line,
                            line_rule.clone(),
                            *before,
                            *after,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphMetrics {
                start_paragraph,
                end_paragraph,
                left,
                right,
                before,
                after,
                first_line,
                hanging,
                clear_special,
            } => {
                let special = if *clear_special {
                    Some(SpecialIndent::None)
                } else if let Some(amount) = first_line {
                    Some(SpecialIndent::FirstLine(*amount))
                } else {
                    hanging.map(SpecialIndent::Hanging)
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(
                            SetParagraphMetricsCommand::new(
                                *start_paragraph,
                                *end_paragraph,
                                *left,
                                *right,
                                *before,
                                *after,
                            )
                            .with_special_indent(special),
                        ),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphShading {
                start_paragraph,
                end_paragraph,
                color,
            } => {
                let color = color
                    .as_deref()
                    .map(|value| {
                        let value = value.trim();
                        let hex = value.strip_prefix('#').unwrap_or(value);
                        if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                            return Err(CommandError::Custom(
                                "paragraph shading color must be #RRGGBB".into(),
                            ));
                        }
                        Ok(format!("#{}", hex.to_ascii_uppercase()))
                    })
                    .transpose()
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphVisualsCommand::shading(
                            *start_paragraph,
                            *end_paragraph,
                            color,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphBorders {
                start_paragraph,
                end_paragraph,
                top,
                bottom,
                left,
                right,
            } => {
                let borders = crate::document::ParagraphBorders {
                    top: top
                        .as_ref()
                        .map(paragraph_border_edge)
                        .transpose()
                        .map_err(|source| HeadlessApiApplyError::Command { index, source })?,
                    bottom: bottom
                        .as_ref()
                        .map(paragraph_border_edge)
                        .transpose()
                        .map_err(|source| HeadlessApiApplyError::Command { index, source })?,
                    left: left
                        .as_ref()
                        .map(paragraph_border_edge)
                        .transpose()
                        .map_err(|source| HeadlessApiApplyError::Command { index, source })?,
                    right: right
                        .as_ref()
                        .map(paragraph_border_edge)
                        .transpose()
                        .map_err(|source| HeadlessApiApplyError::Command { index, source })?,
                    ..Default::default()
                };
                let borders = (!borders.is_empty()).then_some(borders);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphVisualsCommand::borders(
                            *start_paragraph,
                            *end_paragraph,
                            borders,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetNumbering {
                start_paragraph,
                end_paragraph,
                num_id,
                level,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetNumberingCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            num_id.clone(),
                            *level,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ApplyMultilevelList {
                start_paragraph,
                end_paragraph,
                preset,
                level,
            } => {
                let preset = match preset {
                    HeadlessMultilevelListPreset::DecimalHierarchy => {
                        MultilevelListPreset::DecimalHierarchy
                    }
                    HeadlessMultilevelListPreset::ChineseOfficial => {
                        MultilevelListPreset::ChineseOfficial
                    }
                    HeadlessMultilevelListPreset::BulletHierarchy => {
                        MultilevelListPreset::BulletHierarchy
                    }
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ApplyMultilevelListCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            preset,
                            *level,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ApplyCustomMultilevelList {
                start_paragraph,
                end_paragraph,
                levels,
                level,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ApplyMultilevelListCommand::new_custom(
                            *start_paragraph,
                            *end_paragraph,
                            custom_numbering_levels(levels),
                            *level,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetFontColor {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                color,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetFontColorCommand::new(range, color.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetHighlightColor {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                color,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetHighlightColorCommand::new(range, color.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetRunFormat {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                format,
            } => {
                let range =
                    range_from_bounds(*start_paragraph, *start_offset, *end_paragraph, *end_offset);
                selection = Some(range);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetRunFormatCommand::new(range, (**format).clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphTypography {
                start_paragraph,
                end_paragraph,
                typography,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetParagraphTypographyCommand::new(
                            *start_paragraph,
                            *end_paragraph,
                            typography.clone(),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertEnclosedCharacter {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                text,
                enclosure,
            } => {
                let range = SelectionRange::new(
                    TextPosition::new(*start_paragraph, *start_offset),
                    TextPosition::new(*end_paragraph, *end_offset),
                );
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertFieldCommand::of_kind(
                            range,
                            crate::document::RunFieldKind::EqEnclosed {
                                text: text.clone(),
                                enclosure: enclosure.clone(),
                            },
                            text.clone(),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetRuby {
                paragraph,
                start_offset,
                end_offset,
                ruby,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetRubyCommand::new(
                            *paragraph,
                            *start_offset,
                            *end_offset,
                            ruby.clone(),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetJapaneseTypography { typography } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetJapaneseTypographyCommand::new(typography.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetDocumentSetting { name, enabled } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetDocumentSettingCommand::new(name.clone(), *enabled)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetPageOrientation { landscape } => {
                // Page setup follows Word's ribbon: it applies to the section
                // the caret is in. Callers without a selection (CLI, MCP) keep
                // addressing the final section.
                let section = page_setup_section(word_doc, selection);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetPageOrientationCommand::for_section(section, *landscape)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetPageSize {
                portrait_width,
                portrait_height,
            } => {
                let section = page_setup_section(word_doc, selection);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetPageSizeCommand::for_section(
                            section,
                            *portrait_width,
                            *portrait_height,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetPageMargins {
                top,
                bottom,
                left,
                right,
                header,
                footer,
                gutter,
            } => {
                let spec = PageMarginSpec {
                    top: *top,
                    bottom: *bottom,
                    left: *left,
                    right: *right,
                    header: *header,
                    footer: *footer,
                    gutter: *gutter,
                };
                if spec.top.is_none()
                    && spec.bottom.is_none()
                    && spec.left.is_none()
                    && spec.right.is_none()
                    && spec.header.is_none()
                    && spec.footer.is_none()
                    && spec.gutter.is_none()
                {
                    return Err(HeadlessApiApplyError::Command {
                        index,
                        source: CommandError::Custom(
                            "set_page_margins needs at least one edge".to_string(),
                        ),
                    });
                }
                let section = page_setup_section(word_doc, selection);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetPageMarginsCommand::for_section(section, spec)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetPageColumns {
                count,
                space,
                separator,
            } => {
                let count = *count;
                let space = *space;
                let separator = *separator;
                let section = page_setup_section(word_doc, selection);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SnapshotEditCommand::new(
                            "set_page_columns",
                            move |document| {
                                document
                                    .set_page_columns_in_section(section, count, space, separator)
                            },
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetPageBorders {
                clear,
                top,
                bottom,
                left,
                right,
                offset_from,
            } => {
                let spec = if *clear {
                    if top.is_some()
                        || bottom.is_some()
                        || left.is_some()
                        || right.is_some()
                        || offset_from.is_some()
                    {
                        return Err(HeadlessApiApplyError::Command {
                            index,
                            source: CommandError::Custom(
                                "set_page_borders clear=true cannot include edges or offset_from"
                                    .into(),
                            ),
                        });
                    }
                    None
                } else {
                    Some(PageBorderSpec {
                        top: top.as_ref().map(border_edge_spec),
                        bottom: bottom.as_ref().map(border_edge_spec),
                        left: left.as_ref().map(border_edge_spec),
                        right: right.as_ref().map(border_edge_spec),
                        offset_from: offset_from.clone(),
                    })
                };
                let section = page_setup_section(word_doc, selection);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetPageBordersCommand::for_section(section, spec)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetWatermark { clear, text } => {
                let text = if *clear {
                    if text.is_some() {
                        return Err(HeadlessApiApplyError::Command {
                            index,
                            source: CommandError::Custom(
                                "set_watermark clear=true cannot include text".into(),
                            ),
                        });
                    }
                    None
                } else {
                    Some(text.clone().ok_or_else(|| HeadlessApiApplyError::Command {
                        index,
                        source: CommandError::Custom(
                            "set_watermark clear=false requires text".into(),
                        ),
                    })?)
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(Box::new(SetWatermarkCommand::new(text)), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetDocumentTheme { theme } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetDocumentThemeCommand::new(theme.clone().into())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetPageBackgroundColor { color } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetPageBackgroundColorCommand::new(color.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertAfterHeading {
                heading,
                text,
                level,
                occurrence,
            } => {
                insert_relative_to_heading(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    HeadingInsertSpec {
                        heading,
                        text,
                        level: *level,
                        occurrence: *occurrence,
                        position: HeadingInsertPosition::After,
                    },
                )?;
            }
            HeadlessCommand::InsertBeforeHeading {
                heading,
                text,
                level,
                occurrence,
            } => {
                insert_relative_to_heading(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    HeadingInsertSpec {
                        heading,
                        text,
                        level: *level,
                        occurrence: *occurrence,
                        position: HeadingInsertPosition::Before,
                    },
                )?;
            }
            HeadlessCommand::StartChangeTracking {
                author,
                initials: _,
                date,
                date_utc,
            } => {
                let session = RevisionSession {
                    author: author.clone(),
                    date: date.clone(),
                    date_utc: date_utc.clone(),
                };
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "start_change_tracking",
                    EditScope::Document,
                    move |doc| {
                        doc.set_revision_session(Some(session.clone()));
                        Ok(())
                    },
                )?;
            }
            HeadlessCommand::StopChangeTracking => {
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "stop_change_tracking",
                    EditScope::Document,
                    |doc| {
                        doc.set_revision_session(None);
                        Ok(())
                    },
                )?;
            }
            HeadlessCommand::AcceptRevisions {
                kind,
                start_paragraph,
                end_paragraph,
                revision_id,
                revision_ids,
                revision_targets,
            }
            | HeadlessCommand::RejectRevisions {
                kind,
                start_paragraph,
                end_paragraph,
                revision_id,
                revision_ids,
                revision_targets,
            } => {
                let accept = matches!(command, HeadlessCommand::AcceptRevisions { .. });
                let ids = RevisionIdSelection::new(
                    word_doc,
                    revision_id.as_deref(),
                    revision_ids.as_deref(),
                );
                let targets = revision_targets.as_ref().map(|targets| {
                    let groups = revision_groups(word_doc);
                    let mut set = RevisionTargets::default();
                    for target in targets {
                        if target.paragraph_mark {
                            set.add_paragraph_mark(target.paragraph, target.id.clone());
                        } else {
                            for id in revision_target_run_ids(&groups, target) {
                                set.add_run(
                                    target.paragraph,
                                    target.start_offset,
                                    target.end_offset,
                                    id,
                                );
                            }
                        }
                    }
                    set
                });
                // An empty id or target list selects nothing: skip it rather
                // than record an undo step that changes nothing.
                if ids.is_empty() || targets.as_ref().is_some_and(RevisionTargets::is_empty) {
                    continue;
                }
                let kind_filter = kind.as_ref().map(revision_kind_from_filter);
                let (start_paragraph, end_paragraph) = (*start_paragraph, *end_paragraph);
                let scope = word_doc.revision_decision_scope(
                    accept,
                    kind_filter.as_ref(),
                    start_paragraph,
                    end_paragraph,
                    ids.filter(),
                    targets.as_ref(),
                );
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    if accept {
                        "accept_revisions"
                    } else {
                        "reject_revisions"
                    },
                    scope,
                    move |doc| {
                        doc.decide_revisions_unguarded(
                            accept,
                            kind_filter.clone(),
                            start_paragraph,
                            end_paragraph,
                            ids.filter(),
                            targets.as_ref(),
                        )
                        .map(|_| ())
                    },
                )?;
            }
            HeadlessCommand::AddComment {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                text,
                author,
                initials,
                date,
            } => {
                // オフセット省略時は段落全体を対象にする。
                // 「この段落にコメント」を表現するためだけに、呼び出し側が
                // バイトオフセットを数える必要をなくす。
                let start_offset = *start_offset;
                let end_offset = *end_offset;
                let (start_paragraph, end_paragraph) = (*start_paragraph, *end_paragraph);
                let (text, author, initials, date) =
                    (text.clone(), author.clone(), initials.clone(), date.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "add_comment",
                    paragraphs_scope(start_paragraph, end_paragraph),
                    move |doc| {
                        let start_offset = start_offset.unwrap_or(0);
                        let end_offset = match end_offset {
                            Some(value) => value,
                            None => paragraph_text(doc, end_paragraph)
                                .map(|t| t.len())
                                .unwrap_or(0),
                        };
                        doc.add_comment(AddCommentRequest {
                            start_paragraph,
                            start_offset,
                            end_paragraph,
                            end_offset,
                            text: text.clone(),
                            author: author.clone(),
                            initials: initials.clone(),
                            date: date.clone(),
                        })
                        .map(|_| ())
                    },
                )?;
            }
            HeadlessCommand::AddCommentByText {
                query,
                occurrence,
                text,
                author,
                initials,
                date,
            } => {
                // 検索してその場にアンカーする。search_text -> オフセット -> add_comment を
                // 呼び出し側でつなぐ必要をなくす（バイトオフセットの計算は間違えやすい）。
                let wanted = occurrence.unwrap_or(0);
                let query = query.clone();
                let (text, author, initials, date) =
                    (text.clone(), author.clone(), initials.clone(), date.clone());
                let scope = find_nth_text_match(word_doc, &query, wanted)
                    .map_or(EditScope::Document, |hit| {
                        paragraphs_scope(hit.paragraph, hit.paragraph)
                    });
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "add_comment_by_text",
                    scope,
                    move |doc| {
                        let hit = find_nth_text_match(doc, &query, wanted).ok_or_else(|| {
                            format!("text not found: {query:?} (occurrence {wanted})")
                        })?;
                        doc.add_comment(AddCommentRequest {
                            start_paragraph: hit.paragraph,
                            start_offset: hit.start,
                            end_paragraph: hit.paragraph,
                            end_offset: hit.end,
                            text: text.clone(),
                            author: author.clone(),
                            initials: initials.clone(),
                            date: date.clone(),
                        })
                        .map(|_| ())
                    },
                )?;
            }
            HeadlessCommand::AddCommentByParaId {
                para_id,
                start_offset,
                end_offset,
                text,
                author,
                initials,
                date,
            } => {
                let para_id = para_id.clone();
                let (start_offset, end_offset) = (*start_offset, *end_offset);
                let (text, author, initials, date) =
                    (text.clone(), author.clone(), initials.clone(), date.clone());
                let scope = paragraph_index_by_para_id(word_doc, &para_id)
                    .map_or(EditScope::Document, |paragraph| {
                        paragraphs_scope(paragraph, paragraph)
                    });
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "add_comment_by_para_id",
                    scope,
                    move |doc| {
                        let paragraph = paragraph_index_by_para_id(doc, &para_id)
                            .ok_or_else(|| format!("no paragraph found with paraId {para_id}"))?;
                        doc.add_comment(AddCommentRequest {
                            start_paragraph: paragraph,
                            start_offset,
                            end_paragraph: paragraph,
                            end_offset,
                            text: text.clone(),
                            author: author.clone(),
                            initials: initials.clone(),
                            date: date.clone(),
                        })
                        .map(|_| ())
                    },
                )?;
            }
            HeadlessCommand::ReplyToComment {
                comment_id,
                text,
                author,
                initials,
                date,
            } => {
                let comment_id = comment_id.clone();
                let (text, author, initials, date) =
                    (text.clone(), author.clone(), initials.clone(), date.clone());
                let scope = word_doc
                    .comment_reply_paragraph(&comment_id)
                    .map_or(EditScope::Document, |paragraph| {
                        paragraphs_scope(paragraph, paragraph)
                    });
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "reply_to_comment",
                    scope,
                    move |doc| {
                        doc.reply_to_comment(ReplyToCommentRequest {
                            parent_comment_id: comment_id.clone(),
                            text: text.clone(),
                            author: author.clone(),
                            initials: initials.clone(),
                            date: date.clone(),
                        })
                        .map(|_| ())
                    },
                )?;
            }
            HeadlessCommand::UpdateComment { comment_id, text } => {
                let (comment_id, text) = (comment_id.clone(), text.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "update_comment",
                    EditScope::Paragraphs(None),
                    move |doc| doc.update_comment(&comment_id, &text),
                )?;
            }
            HeadlessCommand::DeleteComment { comment_id } => {
                let comment_id = comment_id.clone();
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "delete_comment",
                    EditScope::Document,
                    move |doc| doc.delete_comment(&comment_id),
                )?;
            }
            HeadlessCommand::ResolveComment {
                comment_id,
                resolved,
            } => {
                let comment_id = comment_id.clone();
                let resolved = *resolved;
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "resolve_comment",
                    EditScope::Paragraphs(None),
                    move |doc| doc.resolve_comment(&comment_id, resolved),
                )?;
            }
            HeadlessCommand::SetHeaderParagraphText { paragraph, text } => {
                word_doc
                    .replace_header_paragraph_text(*paragraph, text)
                    .map_err(|message| HeadlessApiApplyError::Command {
                        index,
                        source: CommandError::Custom(message),
                    })?;
            }
            HeadlessCommand::SetFooterParagraphText { paragraph, text } => {
                word_doc
                    .replace_footer_paragraph_text(*paragraph, text)
                    .map_err(|message| HeadlessApiApplyError::Command {
                        index,
                        source: CommandError::Custom(message),
                    })?;
            }
            HeadlessCommand::InsertTable {
                after_paragraph,
                rows,
                columns,
                column_widths,
                caret_offset,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                let command = match caret_offset {
                    Some(offset) => InsertTableCommand::at_caret(
                        *after_paragraph,
                        *offset,
                        *rows,
                        *columns,
                        column_widths.clone(),
                    ),
                    None => InsertTableCommand::new(
                        *after_paragraph,
                        *rows,
                        *columns,
                        column_widths.clone(),
                    ),
                };
                command_executor
                    .execute(Box::new(command), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetSectionLayout { scope, patch } => {
                let sections = word_doc
                    .sections_in_scope(*scope, selection.map(|range| range.focus.paragraph));
                let patch = patch.clone();
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SnapshotEditCommand::new(
                            "set_section_layout",
                            move |document| document.set_section_layout(&sections, &patch),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetHyphenation { patch } => {
                let patch = patch.clone();
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SnapshotEditCommand::new(
                            "set_hyphenation",
                            move |document| document.set_hyphenation(&patch),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetParagraphSnapToGrid {
                start_paragraph,
                end_paragraph,
                snap,
            } => {
                let (start, end, snap) = (*start_paragraph, *end_paragraph, *snap);
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SnapshotEditCommand::with_scope(
                            "set_paragraph_snap_to_grid",
                            EditScope::Paragraphs(Some((start.min(end), start.max(end)))),
                            move |document| document.set_paragraphs_snap_to_grid(start, end, snap),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetHeaderFooterLink {
                section,
                role,
                variant,
                linked,
            } => {
                let section = *section;
                let linked = *linked;
                let role = role.clone();
                let variant = variant.clone();
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SnapshotEditCommand::new(
                            "set_header_footer_link",
                            move |document| {
                                document
                                    .set_header_footer_link(section, &role, &variant, linked)
                                    .map(|_| ())
                            },
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetHeaderFooterVariants {
                different_first_page,
                different_odd_even_pages,
                section,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SetHeaderFooterVariantsCommand::new(
                            *different_first_page,
                            *different_odd_even_pages,
                            *section,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertTableOfContents {
                after_paragraph,
                outline_min,
                outline_max,
            } => {
                let options = crate::document::TocOptions {
                    outline_min: *outline_min,
                    outline_max: *outline_max,
                    hyperlinks: true,
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
                    use_applied_outline_levels: true,
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertTableOfContentsCommand::new(*after_paragraph, options)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertCaption {
                after_paragraph,
                label,
                description,
            } => {
                let range = SelectionRange::collapsed(TextPosition::new(*after_paragraph, 0));
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertCaptionCommand::new(
                            range,
                            label.clone(),
                            description.clone(),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertField {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                kind,
                cached_text,
            } => {
                let range = SelectionRange::new(
                    TextPosition::new(*start_paragraph, *start_offset),
                    TextPosition::new(*end_paragraph, *end_offset),
                );
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertFieldCommand::new(range, *kind, cached_text.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertDateTimeField {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                time,
                format,
                cached_text,
            } => {
                let range = SelectionRange::new(
                    TextPosition::new(*start_paragraph, *start_offset),
                    TextPosition::new(*end_paragraph, *end_offset),
                );
                let kind = if *time {
                    jade_docx_headless_api::HeadlessFieldKind::Time
                } else {
                    jade_docx_headless_api::HeadlessFieldKind::Date
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(
                            InsertFieldCommand::new(range, kind, cached_text.clone())
                                .with_format(Some(format.clone())),
                        ),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertEquation {
                paragraph,
                offset,
                omml,
                placement,
            } => {
                validate_text_offset(word_doc, index, *paragraph, *offset)?;
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                selection = Some(range);
                let placement = match placement {
                    HeadlessEquationPlacement::Inline => EquationPlacement::Inline,
                    HeadlessEquationPlacement::Block => EquationPlacement::Block,
                };
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertEquationCommand::new(range, omml.clone(), placement)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ReplaceEquation {
                paragraph,
                run_index,
                omml,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ReplaceEquationCommand::new(
                            *paragraph,
                            *run_index,
                            omml.clone(),
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertBlankPage {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
            } => {
                let range = SelectionRange::new(
                    TextPosition::new(*start_paragraph, *start_offset),
                    TextPosition::new(*end_paragraph, *end_offset),
                );
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(Box::new(InsertBlankPageCommand::new(range)), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertCoverPage { paragraphs } => {
                let paragraphs = paragraphs
                    .iter()
                    .cloned()
                    .map(CoverPageParagraphSpec::from)
                    .collect();
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(Box::new(InsertCoverPageCommand::new(paragraphs)), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertShape {
                paragraph,
                offset,
                shape,
                width_px,
                height_px,
                fill_color,
                border_color,
                border_width_px,
                text,
            } => {
                validate_text_offset(word_doc, index, *paragraph, *offset)?;
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                selection = Some(range);
                let spec = ShapeInsertSpec {
                    shape: text_box_shape(*shape),
                    width_px: *width_px,
                    height_px: *height_px,
                    fill_color: fill_color.clone(),
                    border_color: border_color.clone(),
                    border_width_px: *border_width_px,
                    text: text.clone(),
                    text_color: None,
                    text_font_size_pt: None,
                    text_bold: None,
                    text_italic: None,
                    text_alignment: None,
                };
                // The offset is a rendered one, like every text command's:
                // the shape command counts in the runs' text.
                let model_offset = word_doc
                    .rendered_to_model_text_position(TextPosition::new(*paragraph, *offset))
                    .offset;
                let command: Box<dyn Command> =
                    Box::new(InsertShapeCommand::new(*paragraph, model_offset, spec));
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(PackageSnapshotCommand::wrapping(command)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertTextAmongAnchors {
                paragraph,
                offset,
                after_anchors,
                text,
            } => {
                validate_text_offset(word_doc, index, *paragraph, *offset)?;
                let model_offset = word_doc
                    .rendered_to_model_text_position(TextPosition::new(*paragraph, *offset))
                    .offset;
                let (paragraph, after, text) = (*paragraph, *after_anchors, text.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "insert_text_among_anchors",
                    EditScope::Document,
                    move |doc| doc.insert_text_among_anchors(paragraph, model_offset, after, &text),
                )?;
            }
            HeadlessCommand::DeleteTextBox {
                paragraph,
                text_box,
            } => {
                let missing = |message: String| HeadlessApiApplyError::Command {
                    index,
                    source: CommandError::Custom(message),
                };
                let runs = &word_doc
                    .paragraph(*paragraph)
                    .ok_or_else(|| missing(format!("paragraph {paragraph} does not exist")))?
                    .runs;
                let run_index = runs
                    .iter()
                    .enumerate()
                    .filter(|(_, run)| run.text_box.is_some())
                    .nth(*text_box)
                    .map(|(run_index, _)| run_index)
                    .ok_or_else(|| {
                        missing(format!("paragraph {paragraph} has no text box {text_box}"))
                    })?;
                let target = FloatingObjectTarget {
                    paragraph_index: *paragraph,
                    run_index,
                    kind: FloatingObjectKind::TextBox,
                };
                let command: Box<dyn Command> = Box::new(DeleteObjectCommand::new(target));
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(PackageSnapshotCommand::wrapping(command)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::MarkIndexEntry {
                paragraph,
                offset,
                term,
            } => {
                let range = SelectionRange::collapsed(TextPosition::new(*paragraph, *offset));
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(MarkIndexEntryCommand::new(range, term.clone())),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertIndex { after_paragraph } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertIndexCommand::new(*after_paragraph)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::UpdateTableOfContents => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(Box::new(UpdateTableOfContentsCommand::new()), &mut ctx)
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            // Table geometry and appearance rewrite `w:tblPr` / `w:tcPr` as well
            // as the model, so they are undone by restoring a snapshot rather
            // than by an inverse edit — and going through the executor is what
            // puts them on the undo history at all.
            HeadlessCommand::SetTableWidth {
                table_order_index,
                width,
            } => {
                let (table_order_index, width) = (*table_order_index, width.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_width",
                    EditScope::Document,
                    move |doc| doc.set_table_width(table_order_index, &width),
                )?;
            }
            HeadlessCommand::SetTableFit {
                table_order_index,
                mode,
            } => {
                let (table_order_index, mode) = (*table_order_index, *mode);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_fit",
                    EditScope::Document,
                    move |doc| {
                        let mode = match mode {
                            HeadlessTableFitMode::Contents => "contents",
                            HeadlessTableFitMode::Window => "window",
                            HeadlessTableFitMode::Fixed => "fixed",
                        };
                        doc.set_table_fit(table_order_index, mode)
                    },
                )?;
            }
            HeadlessCommand::SetTableTextWrapping {
                table_order_index,
                wrapping,
            } => {
                let (table_order_index, wrapping) = (*table_order_index, *wrapping);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_text_wrapping",
                    EditScope::Document,
                    move |doc| {
                        doc.set_table_text_wrapping(
                            table_order_index,
                            matches!(wrapping, HeadlessTableTextWrapping::Around),
                        )
                    },
                )?;
            }
            HeadlessCommand::SetTableColumnWidths {
                table_order_index,
                widths,
            } => {
                let (table_order_index, widths) = (*table_order_index, widths.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_column_widths",
                    EditScope::Document,
                    move |doc| doc.set_table_column_widths(table_order_index, &widths),
                )?;
            }
            HeadlessCommand::SetTableAlignment {
                table_order_index,
                alignment,
            } => {
                let (table_order_index, alignment) = (*table_order_index, alignment.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_alignment",
                    EditScope::Document,
                    move |doc| doc.set_table_alignment(table_order_index, &alignment),
                )?;
            }
            HeadlessCommand::SetTableCellShading {
                table_order_index,
                row,
                column,
                color,
            } => {
                let (table_order_index, row, column) = (*table_order_index, *row, *column);
                let color = color.clone();
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_cell_shading",
                    EditScope::Document,
                    move |doc| {
                        doc.set_table_cell_shading(table_order_index, row, column, color.as_deref())
                    },
                )?;
            }
            HeadlessCommand::ApplyTableStylePreset {
                table_order_index,
                preset,
            } => {
                let (table_order_index, preset) = (*table_order_index, preset.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "apply_table_style_preset",
                    EditScope::Document,
                    move |doc| doc.apply_table_style_preset(table_order_index, &preset),
                )?;
            }
            HeadlessCommand::ApplyTableNamedStyle {
                table_order_index,
                style_id,
            } => {
                let (table_order_index, style_id) = (*table_order_index, style_id.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "apply_table_named_style",
                    EditScope::Document,
                    move |doc| doc.apply_table_named_style(table_order_index, style_id.as_deref()),
                )?;
            }
            HeadlessCommand::SetTableLook {
                table_order_index,
                first_row,
                banded_rows,
                last_row,
                first_column,
                last_column,
                banded_columns,
            } => {
                let table_order_index = *table_order_index;
                let update = crate::word_document::TableLookUpdate {
                    first_row: *first_row,
                    last_row: *last_row,
                    first_column: *first_column,
                    last_column: *last_column,
                    banded_rows: *banded_rows,
                    banded_columns: *banded_columns,
                };
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_look",
                    EditScope::Document,
                    move |doc| doc.set_table_look_options(table_order_index, update),
                )?;
            }
            HeadlessCommand::SetTableBorders {
                table_order_index,
                top,
                bottom,
                left,
                right,
                inside_h,
                inside_v,
            } => {
                let table_order_index = *table_order_index;
                let spec = TableBorderSpec {
                    top: top.as_ref().map(border_edge_spec),
                    bottom: bottom.as_ref().map(border_edge_spec),
                    left: left.as_ref().map(border_edge_spec),
                    right: right.as_ref().map(border_edge_spec),
                    inside_h: inside_h.as_ref().map(border_edge_spec),
                    inside_v: inside_v.as_ref().map(border_edge_spec),
                };
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_borders",
                    EditScope::Document,
                    move |doc| doc.set_table_borders(table_order_index, &spec),
                )?;
            }
            HeadlessCommand::SetTableCellBorders {
                table_order_index,
                row,
                column,
                top,
                bottom,
                left,
                right,
            } => {
                let (table_order_index, row, column) = (*table_order_index, *row, *column);
                let spec = TableBorderSpec {
                    top: top.as_ref().map(border_edge_spec),
                    bottom: bottom.as_ref().map(border_edge_spec),
                    left: left.as_ref().map(border_edge_spec),
                    right: right.as_ref().map(border_edge_spec),
                    ..TableBorderSpec::default()
                };
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_cell_borders",
                    EditScope::Document,
                    move |doc| doc.set_table_cell_borders(table_order_index, row, column, &spec),
                )?;
            }
            HeadlessCommand::SetTableRowProperties {
                table_order_index,
                row,
                repeat_header,
                cant_split,
            } => {
                let (table_order_index, row) = (*table_order_index, *row);
                let (repeat_header, cant_split) = (*repeat_header, *cant_split);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_row_properties",
                    EditScope::Document,
                    move |doc| {
                        doc.set_table_row_properties(
                            table_order_index,
                            row,
                            repeat_header,
                            cant_split,
                        )
                    },
                )?;
            }
            HeadlessCommand::ResetTableCellMargins {
                table_order_index,
                row,
                column,
            } => {
                let (table_order_index, row, column) = (*table_order_index, *row, *column);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "reset_table_cell_margins",
                    EditScope::Document,
                    move |doc| doc.reset_table_cell_margins(table_order_index, row, column),
                )?;
            }
            HeadlessCommand::SetTableRowHeightProperties {
                table_order_index,
                row,
                mode,
                height,
            } => {
                let (table_order_index, row, mode, height) =
                    (*table_order_index, *row, mode.clone(), *height);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_row_height_properties",
                    EditScope::Document,
                    move |doc| {
                        doc.set_table_row_height_properties(table_order_index, row, &mode, height)
                    },
                )?;
            }
            HeadlessCommand::SetTableCellMargins {
                table_order_index,
                row,
                column,
                top,
                bottom,
                left,
                right,
            } => {
                let table_order_index = *table_order_index;
                let cell = match (row, column) {
                    (Some(row), Some(column)) => Some((*row, *column)),
                    (None, None) => None,
                    _ => {
                        return Err(HeadlessApiApplyError::Command {
                            index,
                            source: CommandError::Custom(
                                "set_table_cell_margins takes both row and column, or neither \
                                 (which sets the table's default for every cell)"
                                    .to_string(),
                            ),
                        })
                    }
                };
                let margins = TableCellMargins {
                    top: *top,
                    bottom: *bottom,
                    left: *left,
                    right: *right,
                    ..TableCellMargins::default()
                };
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_cell_margins",
                    EditScope::Document,
                    move |doc| doc.set_table_cell_margins(table_order_index, cell, margins.clone()),
                )?;
            }
            HeadlessCommand::SetTableCellVerticalAlignment {
                table_order_index,
                row,
                column,
                alignment,
            } => {
                let (table_order_index, row, column) = (*table_order_index, *row, *column);
                let alignment = alignment.clone();
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_cell_vertical_alignment",
                    EditScope::Document,
                    move |doc| {
                        doc.set_table_cell_vertical_alignment(
                            table_order_index,
                            row,
                            column,
                            &alignment,
                        )
                    },
                )?;
            }
            HeadlessCommand::SetTableCellText {
                table_order_index,
                row,
                column,
                text,
            } => {
                let paragraph_index =
                    table_cell_first_paragraph_index(word_doc, *table_order_index, *row, *column)
                        .map_err(|message| HeadlessApiApplyError::Command {
                        index,
                        source: CommandError::Custom(message),
                    })?;
                let current_text = paragraph_text(word_doc, paragraph_index).unwrap_or_default();
                let range =
                    range_from_bounds(paragraph_index, 0, paragraph_index, current_text.len());
                selection = Some(range);

                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(InsertTextCommand::new(range, text.clone(), None)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::FillTableRow {
                table_order_index,
                row,
                values,
            } => {
                fill_table_row(word_doc, *table_order_index, *row, values).map_err(|message| {
                    HeadlessApiApplyError::Command {
                        index,
                        source: CommandError::Custom(message),
                    }
                })?;
            }
            HeadlessCommand::InsertTableRow {
                table_order_index,
                row_index,
                template_row,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(TableStructureCommand::new(
                            *table_order_index,
                            TableStructureOp::InsertRow {
                                row_index: *row_index,
                                template_row: *template_row,
                            },
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::CloneTableRow {
                table_order_index,
                source_row,
                target_row,
                clear_text,
            } => {
                clone_table_row(
                    word_doc,
                    *table_order_index,
                    *source_row,
                    *target_row,
                    *clear_text,
                )
                .map_err(|message| HeadlessApiApplyError::Command {
                    index,
                    source: CommandError::Custom(message),
                })?;
            }
            HeadlessCommand::DeleteTableRow {
                table_order_index,
                row_index,
            } => {
                // Deleting a row takes the comments anchored inside it; the
                // wrapper keeps comments.xml in step and restores the whole
                // package on undo.
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(PackageSnapshotCommand::wrapping(Box::new(
                            TableStructureCommand::new(
                                *table_order_index,
                                TableStructureOp::DeleteRow {
                                    row_index: *row_index,
                                },
                            ),
                        ))),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SplitTable {
                table_order_index,
                row_index,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(SplitTableCommand::new(*table_order_index, *row_index)),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SplitTableCell {
                table_order_index,
                row,
                column,
                columns,
                rows,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(TableStructureCommand::new(
                            *table_order_index,
                            TableStructureOp::SplitCell {
                                row: *row,
                                column: *column,
                                columns: *columns,
                                rows: *rows,
                            },
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::DistributeTableRows {
                table_order_index,
                height,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(TableStructureCommand::new(
                            *table_order_index,
                            TableStructureOp::DistributeRows { height: *height },
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::DistributeTableColumns {
                table_order_index,
                total_width,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(TableStructureCommand::new(
                            *table_order_index,
                            TableStructureOp::DistributeColumns {
                                total_width: *total_width,
                            },
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::InsertTableColumn {
                table_order_index,
                column_index,
                width,
                template_column,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(TableStructureCommand::new(
                            *table_order_index,
                            TableStructureOp::InsertColumn {
                                column_index: *column_index,
                                width: *width,
                                template_column: *template_column,
                            },
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::DeleteTableColumn {
                table_order_index,
                column_index,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(PackageSnapshotCommand::wrapping(Box::new(
                            TableStructureCommand::new(
                                *table_order_index,
                                TableStructureOp::DeleteColumn {
                                    column_index: *column_index,
                                },
                            ),
                        ))),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::DeleteTableCell {
                table_order_index,
                row,
                column,
                direction,
            } => {
                let (table_order_index, row, column, direction) =
                    (*table_order_index, *row, *column, *direction);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "delete_table_cell",
                    EditScope::Document,
                    move |doc| {
                        doc.delete_table_cell(
                            table_order_index,
                            row,
                            column,
                            matches!(direction, HeadlessTableCellDeleteDirection::ShiftUp),
                        )
                    },
                )?;
            }
            HeadlessCommand::InsertTableCell {
                table_order_index,
                row,
                column,
                direction,
            } => {
                let (table_order_index, row, column, direction) =
                    (*table_order_index, *row, *column, *direction);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "insert_table_cell",
                    EditScope::Document,
                    move |doc| {
                        doc.insert_table_cell(
                            table_order_index,
                            row,
                            column,
                            matches!(direction, HeadlessTableCellInsertDirection::ShiftDown),
                        )
                    },
                )?;
            }
            HeadlessCommand::DeleteTable { table_order_index } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(PackageSnapshotCommand::wrapping(Box::new(
                            DeleteTableCommand::new(*table_order_index),
                        ))),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ResizeTableColumnBetween {
                table_order_index,
                left_column_index,
                new_left_width,
                new_right_width,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ResizeTableColumnCommand::between_columns(
                            *table_order_index,
                            *left_column_index,
                            *new_left_width,
                            *new_right_width,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ResizeTableColumnRightEdge {
                table_order_index,
                column_index,
                new_width,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ResizeTableColumnCommand::table_right_edge(
                            *table_order_index,
                            *column_index,
                            *new_width,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ResizeTableRowBetween {
                table_order_index,
                row_index,
                new_top_height,
                new_bottom_height,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ResizeTableRowCommand::between_rows(
                            *table_order_index,
                            *row_index,
                            *new_top_height,
                            *new_bottom_height,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::ResizeTableRowBottomEdge {
                table_order_index,
                row_index,
                new_height,
            } => {
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(ResizeTableRowCommand::table_bottom_edge(
                            *table_order_index,
                            *row_index,
                            *new_height,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::MergeTableCells {
                table_order_index,
                selected_cells,
            } => {
                let table_selection: Vec<TableCellSelection> = selected_cells
                    .iter()
                    .map(|cell: &HeadlessTableCellSelection| TableCellSelection {
                        row: cell.row,
                        column: cell.column,
                        column_span: cell.column_span,
                    })
                    .collect();
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(MergeTableCellsCommand::new(
                            *table_order_index,
                            table_selection,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::UnmergeTableCells {
                table_order_index,
                target_cells,
            } => {
                let table_selection: Vec<TableCellSelection> = target_cells
                    .iter()
                    .map(|cell: &HeadlessTableCellSelection| TableCellSelection {
                        row: cell.row,
                        column: cell.column,
                        column_span: cell.column_span,
                    })
                    .collect();
                let mut ctx = CommandContext::new(word_doc, &mut selection);
                command_executor
                    .execute(
                        Box::new(UnmergeTableCellsCommand::new(
                            *table_order_index,
                            table_selection,
                        )),
                        &mut ctx,
                    )
                    .map_err(|source| HeadlessApiApplyError::Command { index, source })?;
            }
            HeadlessCommand::SetTableCellSpacing {
                table_order_index,
                spacing,
            } => {
                let (table_order_index, spacing) = (*table_order_index, *spacing);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_cell_spacing",
                    EditScope::Document,
                    move |doc| doc.set_table_cell_spacing(table_order_index, spacing),
                )?;
            }
            HeadlessCommand::SetTableCellTextDirection {
                table_order_index,
                row,
                column,
                direction,
            } => {
                let (table_order_index, row, column) = (*table_order_index, *row, *column);
                let direction = direction.clone();
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_cell_text_direction",
                    EditScope::Document,
                    move |doc| {
                        doc.set_table_cell_text_direction(
                            table_order_index,
                            row,
                            column,
                            direction.as_deref(),
                        )
                    },
                )?;
            }
            HeadlessCommand::SetTableIndent {
                table_order_index,
                indent,
            } => {
                let (table_order_index, indent) = (*table_order_index, *indent);
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_indent",
                    EditScope::Document,
                    move |doc| doc.set_table_indent(table_order_index, indent),
                )?;
            }
            HeadlessCommand::SetTablePosition {
                table_order_index,
                horizontal_align,
                horizontal_offset,
                horizontal_anchor,
                vertical_align,
                vertical_offset,
                vertical_anchor,
                distance_top,
                distance_bottom,
                distance_left,
                distance_right,
                allow_overlap,
            } => {
                let table_order_index = *table_order_index;
                let spec = crate::word_document::TablePositionSpec {
                    horizontal_align: horizontal_align.clone(),
                    horizontal_offset: *horizontal_offset,
                    horizontal_anchor: horizontal_anchor.clone(),
                    vertical_align: vertical_align.clone(),
                    vertical_offset: *vertical_offset,
                    vertical_anchor: vertical_anchor.clone(),
                    distance_top: *distance_top,
                    distance_bottom: *distance_bottom,
                    distance_left: *distance_left,
                    distance_right: *distance_right,
                    allow_overlap: *allow_overlap,
                };
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "set_table_position",
                    EditScope::Document,
                    move |doc| doc.set_table_position(table_order_index, &spec),
                )?;
            }
            HeadlessCommand::ConvertTextToTable {
                start_paragraph,
                end_paragraph,
                separator,
                columns,
            } => {
                let (start, end, columns) = (*start_paragraph, *end_paragraph, *columns);
                let separator = separator.clone();
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "convert_text_to_table",
                    EditScope::Document,
                    move |doc| {
                        doc.convert_text_to_table(start, end, &separator, columns)
                            .map(|_| ())
                    },
                )?;
            }
            HeadlessCommand::ConvertTableToText {
                table_order_index,
                separator,
            } => {
                let table_order_index = *table_order_index;
                let separator = separator.clone();
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "convert_table_to_text",
                    EditScope::Document,
                    move |doc| doc.convert_table_to_text(table_order_index, &separator),
                )?;
            }
            HeadlessCommand::SortTable {
                table_order_index,
                keys,
                header_rows,
            } => {
                let (table_order_index, header_rows) = (*table_order_index, *header_rows);
                let keys: Vec<crate::word_document::TableSortKey> = keys
                    .iter()
                    .map(|key| crate::word_document::TableSortKey {
                        column: key.column,
                        kind: match key.kind {
                            jade_docx_headless_api::HeadlessTableSortKind::Text => {
                                crate::word_document::TableSortKind::Text
                            }
                            jade_docx_headless_api::HeadlessTableSortKind::Number => {
                                crate::word_document::TableSortKind::Number
                            }
                            jade_docx_headless_api::HeadlessTableSortKind::Date => {
                                crate::word_document::TableSortKind::Date
                            }
                        },
                        descending: key.descending,
                    })
                    .collect();
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "sort_table",
                    EditScope::Document,
                    move |doc| doc.sort_table(table_order_index, &keys, header_rows),
                )?;
            }
            HeadlessCommand::InsertTableFormula {
                paragraph,
                offset,
                formula,
                number_format,
            } => {
                let (paragraph, offset) = (*paragraph, *offset);
                validate_text_offset(word_doc, index, paragraph, offset)?;
                let (formula, number_format) = (formula.clone(), number_format.clone());
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "insert_table_formula",
                    EditScope::Document,
                    move |doc| {
                        doc.insert_table_formula(
                            paragraph,
                            offset,
                            &formula,
                            number_format.as_deref(),
                        )
                    },
                )?;
            }
            HeadlessCommand::UpdateTableFormulas => {
                execute_snapshot_edit(
                    word_doc,
                    command_executor,
                    &mut selection,
                    index,
                    "update_table_formulas",
                    EditScope::Document,
                    move |doc| doc.update_table_formulas().map(|_| ()),
                )?;
            }
        }
    }

    Ok(selection)
}

pub fn execute_headless_queries_on_document(
    word_doc: &WordDocument,
    request: &HeadlessQueryRequest,
) -> Result<HeadlessQueryResponse, HeadlessApiQueryError> {
    if request.queries.is_empty() {
        return Err(HeadlessApiQueryError::EmptyQueries);
    }

    let mut results = Vec::with_capacity(request.queries.len());
    for (index, query) in request.queries.iter().enumerate() {
        let result = execute_headless_query(word_doc, index, query)?;
        results.push(result);
    }

    Ok(HeadlessQueryResponse { results })
}

fn execute_headless_query(
    word_doc: &WordDocument,
    index: usize,
    query: &HeadlessQuery,
) -> Result<HeadlessQueryResult, HeadlessApiQueryError> {
    match query {
        HeadlessQuery::DocumentStats => Ok(HeadlessQueryResult::DocumentStats {
            stats: jade_docx_headless_api::HeadlessDocumentStats {
                paragraph_count: word_doc.paragraph_count(),
                table_count: table_count(word_doc),
                comment_count: word_doc.comments.len(),
                revision_count: revision_snapshots(word_doc).len(),
            },
        }),
        HeadlessQuery::DocumentOutline {
            start_heading,
            max_headings,
            max_level,
        } => {
            let headings = outline_snapshots(word_doc)
                .into_iter()
                .filter(|item| max_level.is_none_or(|max| item.level <= max))
                .collect::<Vec<_>>();
            let total = headings.len();
            let start = (*start_heading).min(total);
            let limit = (*max_headings).max(1);
            let end = start.saturating_add(limit).min(total);
            Ok(HeadlessQueryResult::DocumentOutline {
                start_heading: start,
                headings: headings[start..end].to_vec(),
                has_more: end < total,
                total_headings: total,
            })
        }
        HeadlessQuery::ParagraphCount => Ok(HeadlessQueryResult::ParagraphCount {
            paragraph_count: word_doc.paragraph_count(),
        }),
        HeadlessQuery::ParagraphText { paragraph } => Ok(HeadlessQueryResult::ParagraphText {
            paragraph: *paragraph,
            text: paragraph_text(word_doc, *paragraph),
            numbering_text: paragraph_numbering_text(word_doc, *paragraph),
        }),
        HeadlessQuery::Paragraph { paragraph } => Ok(HeadlessQueryResult::Paragraph {
            paragraph: *paragraph,
            value: paragraph_snapshot(word_doc, *paragraph),
        }),
        HeadlessQuery::ParagraphByParaId { para_id } => {
            let paragraph = paragraph_index_by_para_id(word_doc, para_id);
            Ok(HeadlessQueryResult::ParagraphByParaId {
                para_id: para_id.clone(),
                paragraph,
                value: paragraph.and_then(|index| paragraph_snapshot(word_doc, index)),
            })
        }
        HeadlessQuery::ParagraphSlice {
            start_paragraph,
            max_paragraphs,
        } => {
            let total = word_doc.paragraph_count();
            let start = (*start_paragraph).min(total);
            let limit = (*max_paragraphs).max(1);
            let end = start.saturating_add(limit).min(total);
            let numbering_texts = numbering_texts(word_doc);
            let mut paragraphs = Vec::with_capacity(end.saturating_sub(start));

            for paragraph_index in start..end {
                paragraphs.push(HeadlessParagraphText {
                    paragraph: paragraph_index,
                    text: paragraph_text(word_doc, paragraph_index).unwrap_or_default(),
                    numbering_text: numbering_texts.get(paragraph_index).cloned().flatten(),
                });
            }

            Ok(HeadlessQueryResult::ParagraphSlice {
                start_paragraph: start,
                paragraphs,
                has_more: end < total,
            })
        }
        HeadlessQuery::SearchText {
            query,
            start_paragraph,
            max_results,
            context_before_chars,
            context_after_chars,
            case_insensitive,
        } => {
            if query.is_empty() {
                return Err(HeadlessApiQueryError::InvalidQuery {
                    index,
                    message: "search_text.query must not be empty".to_string(),
                });
            }

            let total = word_doc.paragraph_count();
            let start = (*start_paragraph).min(total);
            let result_limit = (*max_results).max(1);
            let mut matches = Vec::with_capacity(result_limit.min(64));
            let mut truncated = false;
            // ASCII case folding leaves every byte sequence the same length —
            // including multi-byte CJK, which it does not touch — so offsets
            // found in the folded copy index the original text unchanged.
            let needle = if *case_insensitive {
                query.to_ascii_lowercase()
            } else {
                query.clone()
            };

            'paragraphs: for paragraph_index in start..total {
                let Some(text) = paragraph_text(word_doc, paragraph_index) else {
                    continue;
                };
                let haystack = if *case_insensitive {
                    text.to_ascii_lowercase()
                } else {
                    text.clone()
                };
                for (start_offset, folded_match) in haystack.match_indices(needle.as_str()) {
                    let end_offset = start_offset + folded_match.len();
                    // Report the document's own casing, not the folded form.
                    let matched_text = &text[start_offset..end_offset];
                    let (context_before, context_after, start_char_offset, end_char_offset) =
                        extract_match_context(
                            &text,
                            start_offset,
                            end_offset,
                            *context_before_chars,
                            *context_after_chars,
                        );
                    let snippet = format!("{context_before}{matched_text}{context_after}");

                    matches.push(HeadlessSearchMatch {
                        paragraph: paragraph_index,
                        start_offset,
                        end_offset,
                        start_char_offset,
                        end_char_offset,
                        matched_text: matched_text.to_string(),
                        context_before,
                        context_after,
                        snippet,
                    });

                    // Flag truncation only once a further match has actually
                    // been seen: a caller uses this to decide whether to page,
                    // and a search that happens to return exactly `max_results`
                    // hits has nothing left to page through.
                    if matches.len() > result_limit {
                        matches.truncate(result_limit);
                        truncated = true;
                        break 'paragraphs;
                    }
                }
            }

            Ok(HeadlessQueryResult::SearchText {
                query: query.clone(),
                matches,
                truncated,
            })
        }
        HeadlessQuery::CharacterStyleAt { paragraph, offset } => {
            Ok(HeadlessQueryResult::CharacterStyleAt {
                paragraph: *paragraph,
                offset: *offset,
                value: character_style_snapshot(word_doc, *paragraph, *offset),
            })
        }
        HeadlessQuery::TableCount => Ok(HeadlessQueryResult::TableCount {
            table_count: table_count(word_doc),
        }),
        HeadlessQuery::Table { table_order_index } => Ok(HeadlessQueryResult::Table {
            table_order_index: *table_order_index,
            value: table_snapshot(word_doc, *table_order_index),
        }),
        HeadlessQuery::TableRow {
            table_order_index,
            row,
        } => Ok(HeadlessQueryResult::TableRow {
            table_order_index: *table_order_index,
            row: *row,
            value: table_row_snapshot(word_doc, *table_order_index, *row),
        }),
        HeadlessQuery::TableColumn {
            table_order_index,
            column,
        } => Ok(HeadlessQueryResult::TableColumn {
            table_order_index: *table_order_index,
            column: *column,
            value: table_column_snapshot(word_doc, *table_order_index, *column),
        }),
        HeadlessQuery::TableCell {
            table_order_index,
            row,
            column,
        } => Ok(HeadlessQueryResult::TableCell {
            table_order_index: *table_order_index,
            row: *row,
            column: *column,
            value: table_cell_snapshot(word_doc, *table_order_index, *row, *column),
        }),
        HeadlessQuery::Revisions {
            start_revision,
            max_revisions,
            kind,
            author,
            date_from,
            date_to,
        } => {
            let revisions = revision_snapshots(word_doc)
                .into_iter()
                .filter(|revision| {
                    revision_matches_filters(
                        revision,
                        kind.as_ref(),
                        author.as_ref(),
                        date_from.as_ref(),
                        date_to.as_ref(),
                    )
                })
                .collect::<Vec<_>>();
            let total = revisions.len();
            let start = (*start_revision).min(total);
            let limit = (*max_revisions).max(1);
            let end = start.saturating_add(limit).min(total);
            Ok(HeadlessQueryResult::Revisions {
                start_revision: start,
                revisions: revisions[start..end].to_vec(),
                has_more: end < total,
                total_revisions: total,
            })
        }
        HeadlessQuery::Comments {
            start_comment,
            max_comments,
            author,
            resolved,
        } => {
            // Filter before paging so `total_comments` counts the matches
            // rather than the whole document: a reviewer asking for unresolved
            // comments wants to know how many of *those* are left.
            let comments: Vec<_> = comment_snapshots(word_doc)
                .into_iter()
                .filter(|comment| {
                    author
                        .as_deref()
                        .is_none_or(|wanted| comment.author.as_deref() == Some(wanted))
                })
                .filter(|comment| {
                    resolved.is_none_or(|wanted| comment.resolved.unwrap_or(false) == wanted)
                })
                .collect();
            let total = comments.len();
            let start = (*start_comment).min(total);
            let limit = (*max_comments).max(1);
            let end = start.saturating_add(limit).min(total);
            Ok(HeadlessQueryResult::Comments {
                start_comment: start,
                comments: comments[start..end].to_vec(),
                has_more: end < total,
                total_comments: total,
            })
        }
        HeadlessQuery::CommentThreads {
            start_thread,
            max_threads,
        } => {
            let threads = comment_thread_snapshots(word_doc);
            let total = threads.len();
            let start = (*start_thread).min(total);
            let limit = (*max_threads).max(1);
            let end = start.saturating_add(limit).min(total);
            Ok(HeadlessQueryResult::CommentThreads {
                start_thread: start,
                threads: threads[start..end].to_vec(),
                has_more: end < total,
                total_threads: total,
            })
        }
        HeadlessQuery::Styles { kind } => Ok(HeadlessQueryResult::Styles {
            styles: style_snapshots(word_doc, *kind),
        }),
        HeadlessQuery::PageSetup => Ok(HeadlessQueryResult::PageSetup {
            page_setup: page_setup_snapshot(word_doc),
        }),
        HeadlessQuery::SectionLayout { section } => Ok(HeadlessQueryResult::SectionLayout {
            layout: Box::new(word_doc.section_layout(*section)),
        }),
        HeadlessQuery::HeaderFooterText => Ok(HeadlessQueryResult::HeaderFooterText {
            stories: header_footer_snapshots(word_doc),
        }),
        HeadlessQuery::DocumentProperties => Ok(HeadlessQueryResult::DocumentProperties {
            properties: word_doc.document_properties(),
        }),
        HeadlessQuery::CitationSources => Ok(HeadlessQueryResult::CitationSources {
            sources: word_doc
                .citation_sources()
                .into_iter()
                .map(headless_citation_source)
                .collect(),
        }),
        HeadlessQuery::DocumentProtection => Ok(HeadlessQueryResult::DocumentProtection {
            protection: word_doc
                .document_protection()
                .map(headless_document_protection),
        }),
        HeadlessQuery::Chunks {
            start_chunk,
            max_chunks,
            disable_table_splitting,
            enable_branch_splitting,
            include_text,
        } => {
            let chunks = build_chunks(
                &paragraph_inputs(word_doc),
                &ChunkOptions {
                    disable_table_splitting: *disable_table_splitting,
                    enable_branch_splitting: *enable_branch_splitting,
                    include_text: *include_text,
                },
            );
            let total = chunks.len();
            let start = (*start_chunk).min(total);
            let limit = (*max_chunks).max(1);
            let end = start.saturating_add(limit).min(total);
            Ok(HeadlessQueryResult::Chunks {
                start_chunk: start,
                chunks: chunks[start..end].to_vec(),
                has_more: end < total,
                total_chunks: total,
            })
        }
        HeadlessQuery::References {
            start_reference,
            max_references,
            unresolved_only,
            start_paragraph,
            end_paragraph,
        } => {
            // Filter before paging so the reported total counts the matches:
            // "how many citations are dangling" is the question being asked.
            let references: Vec<_> = find_references(&paragraph_inputs(word_doc))
                .into_iter()
                .filter(|reference| !*unresolved_only || !reference.resolved)
                .filter(|reference| {
                    start_paragraph.is_none_or(|first| reference.paragraph >= first)
                        && end_paragraph.is_none_or(|last| reference.paragraph <= last)
                })
                .collect();
            let total = references.len();
            let start = (*start_reference).min(total);
            let limit = (*max_references).max(1);
            let end = start.saturating_add(limit).min(total);
            Ok(HeadlessQueryResult::References {
                start_reference: start,
                references: references[start..end].to_vec(),
                has_more: end < total,
                total_references: total,
            })
        }
    }
}

/// The whole body as the legal-structure rules read it.
fn paragraph_inputs(word_doc: &WordDocument) -> Vec<ParagraphInput> {
    let numbering_texts = numbering_texts(word_doc);
    (0..word_doc.paragraph_count())
        .map(|index| ParagraphInput {
            text: paragraph_text(word_doc, index).unwrap_or_default(),
            numbering_text: numbering_texts.get(index).cloned().flatten(),
            para_id: word_doc
                .paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.para_id.as_deref().map(str::to_string)),
            in_table: word_doc.paragraph_is_in_table(index),
        })
        .collect()
}

/// The document's style catalogue, sorted by id so repeated calls agree.
fn style_snapshots(
    word_doc: &WordDocument,
    kind: Option<HeadlessStyleKindFilter>,
) -> Vec<HeadlessStyleSnapshot> {
    let Some(styles_part) = word_doc.styles_part.as_ref() else {
        return Vec::new();
    };
    let snapshot = |definition: &StyleDefinition, kind: &'static str| HeadlessStyleSnapshot {
        style_id: definition.style_id.clone(),
        name: definition.name.clone(),
        based_on: definition.based_on.clone(),
        is_default: definition.is_default,
        kind: kind.to_string(),
    };

    let mut styles = Vec::new();
    if kind != Some(HeadlessStyleKindFilter::Character) {
        styles.extend(
            styles_part
                .paragraph_style_definitions()
                .map(|definition| snapshot(definition, "paragraph")),
        );
    }
    if kind != Some(HeadlessStyleKindFilter::Paragraph) {
        styles.extend(
            styles_part
                .character_style_definitions()
                .map(|definition| snapshot(definition, "character")),
        );
    }
    styles.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.style_id.cmp(&right.style_id))
    });
    styles
}

/// Page geometry of the default section, mirroring what the `set_page_*`
/// commands write.
fn page_setup_snapshot(word_doc: &WordDocument) -> HeadlessPageSetupSnapshot {
    let Some(document) = word_doc.document.as_deref() else {
        return HeadlessPageSetupSnapshot::default();
    };
    let background_color = document.background_color.clone();
    let page = document.page_settings.as_ref();
    let columns = document
        .default_section
        .as_ref()
        .and_then(|section| section.columns.as_ref());
    HeadlessPageSetupSnapshot {
        width: page.and_then(|page| page.width),
        height: page.and_then(|page| page.height),
        orientation: page.and_then(|page| page.orientation.clone()),
        margin_top: page.and_then(|page| page.margin_top),
        margin_bottom: page.and_then(|page| page.margin_bottom),
        margin_left: page.and_then(|page| page.margin_left),
        margin_right: page.and_then(|page| page.margin_right),
        header_distance: page.and_then(|page| page.header_distance),
        footer_distance: page.and_then(|page| page.footer_distance),
        column_count: columns.and_then(|columns| columns.count),
        column_space: columns.and_then(|columns| columns.space),
        column_equal_width: columns.and_then(|columns| columns.equal_width),
        column_separator: columns.and_then(|columns| columns.separator),
        background_color,
        watermark_text: document.watermark_text.clone(),
    }
}

/// Every header/footer story the document defines, in the order Word applies
/// them. Lines are reported in the same shape
/// `set_header_paragraph_text` / `set_footer_paragraph_text` write back.
fn header_footer_snapshots(word_doc: &WordDocument) -> Vec<HeadlessHeaderFooterSnapshot> {
    [
        ("header", "default", word_doc.header_lines.as_ref()),
        ("header", "first", word_doc.header_lines_first.as_ref()),
        ("header", "even", word_doc.header_lines_even.as_ref()),
        ("footer", "default", word_doc.footer_lines.as_ref()),
        ("footer", "first", word_doc.footer_lines_first.as_ref()),
        ("footer", "even", word_doc.footer_lines_even.as_ref()),
    ]
    .into_iter()
    .filter_map(|(role, variant, lines)| {
        let lines = lines?;
        if lines.is_empty() {
            return None;
        }
        Some(HeadlessHeaderFooterSnapshot {
            role: role.to_string(),
            variant: variant.to_string(),
            lines: lines
                .iter()
                .enumerate()
                .map(|(paragraph, line)| HeadlessHeaderFooterLine {
                    paragraph,
                    left: line.left.clone(),
                    center: line.center.clone(),
                    right: line.right.clone(),
                    // Word lays a header line out as left/center/right zones
                    // separated by tabs; the editing commands take that same
                    // tab-joined string.
                    text: [&line.left, &line.center, &line.right]
                        .iter()
                        .map(|zone| zone.as_deref().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\t")
                        .trim_end_matches('\t')
                        .to_string(),
                })
                .collect(),
        })
    })
    .collect()
}

/// Resolve a `w14:paraId` to the current positional index of the paragraph that
/// carries it (the first match in document order), or `None`. Scanning live
/// paragraphs keeps the result correct as edits shift indices.
fn paragraph_index_by_para_id(word_doc: &WordDocument, para_id: &str) -> Option<usize> {
    (0..word_doc.paragraph_count()).find(|&index| {
        word_doc
            .paragraph(index)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|props| props.para_id.as_deref())
            == Some(para_id)
    })
}

/// 文書順に `query` を数えて `nth`（0 起点）番目の一致位置を返す。
///
/// オフセットは段落内のバイト位置で、`AddCommentRequest` がそのまま受け取れる形。
/// 一致が段落をまたぐ場合は扱わない（コメントのアンカーとしては 1 段落に閉じるのが自然）。
struct TextMatch {
    paragraph: usize,
    start: usize,
    end: usize,
}

fn find_nth_text_match(word_doc: &WordDocument, query: &str, nth: usize) -> Option<TextMatch> {
    if query.is_empty() {
        return None;
    }
    let mut seen = 0usize;
    let deleted_rows: std::collections::HashSet<usize> = word_doc
        .table_row_revisions()
        .into_iter()
        .filter(|revision| revision.mark.is_deletion())
        .flat_map(|revision| revision.paragraphs)
        .collect();
    for paragraph in 0..word_doc.paragraph_count() {
        if deleted_rows.contains(&paragraph) {
            continue;
        }
        let source = word_doc.paragraph(paragraph)?;
        let mut text = String::new();
        let mut offsets = Vec::new();
        let mut original = 0;
        for run in &source.runs {
            if !run.revision.as_ref().is_some_and(|mark| mark.is_deletion()) {
                for (offset, character) in run.text.char_indices() {
                    offsets.push((text.len(), original + offset, character.len_utf8()));
                    text.push(character);
                }
            }
            original += run.text.len();
        }
        let mut from = 0usize;
        while let Some(found) = text[from..].find(query) {
            let start = from + found;
            if seen == nth {
                // 表示上の文字位置を、アンカーが使う元の UTF-8 座標へ戻す。
                let end = start + query.len();
                let original_start = offsets.iter().find(|(at, _, _)| *at == start)?.1;
                let (_, last, width) = offsets.iter().rev().find(|(at, _, _)| *at < end)?;
                return Some(TextMatch {
                    paragraph,
                    start: original_start,
                    end: last + width,
                });
            }
            seen += 1;
            from = start + query.len();
        }
    }
    None
}

fn trimmed_replacement<'a>(
    word_doc: &WordDocument,
    start_paragraph: usize,
    start: usize,
    end_paragraph: usize,
    end: usize,
    replacement: &'a str,
) -> (usize, usize, &'a str) {
    if start_paragraph != end_paragraph || start > end {
        return (start, end, replacement);
    }
    let Some(paragraph) = word_doc.paragraph(start_paragraph) else {
        return (start, end, replacement);
    };
    let mut at = 0;
    for run in &paragraph.runs {
        let next = at + run.text.len();
        if at < end && next > start && run.revision.as_ref().is_some_and(|mark| mark.is_deletion())
        {
            return (start, end, replacement);
        }
        at = next;
    }
    let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
    let Some(selected) = text.get(start..end) else {
        return (start, end, replacement);
    };
    let prefix = selected
        .chars()
        .zip(replacement.chars())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    let suffix = selected[prefix..]
        .chars()
        .rev()
        .zip(replacement[prefix..].chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    (
        start + prefix,
        end - suffix,
        &replacement[prefix..replacement.len() - suffix],
    )
}

fn paragraph_text(word_doc: &WordDocument, index: usize) -> Option<String> {
    let paragraph = word_doc.paragraph(index)?;
    let mut text = String::new();
    for run in &paragraph.runs {
        text.push_str(&run.text);
    }
    Some(text)
}

#[derive(Debug, Clone)]
struct PlaceholderMatch {
    paragraph: usize,
    start_offset: usize,
    end_offset: usize,
    replacement_properties: Option<RunProperties>,
}

fn placeholder_matches(
    word_doc: &WordDocument,
    placeholder: &str,
    max_replacements: usize,
) -> Vec<PlaceholderMatch> {
    if max_replacements == 0 {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for paragraph in 0..word_doc.paragraph_count() {
        let Some(text) = paragraph_text(word_doc, paragraph) else {
            continue;
        };
        for (start_offset, matched) in text.match_indices(placeholder) {
            matches.push(PlaceholderMatch {
                paragraph,
                start_offset,
                end_offset: start_offset + matched.len(),
                replacement_properties: run_properties_at_offset(word_doc, paragraph, start_offset),
            });
            if matches.len() >= max_replacements {
                return matches;
            }
        }
    }
    matches
}

fn run_properties_at_offset(
    word_doc: &WordDocument,
    paragraph_index: usize,
    offset: usize,
) -> Option<RunProperties> {
    let paragraph = word_doc.paragraph(paragraph_index)?;
    let mut cursor = 0usize;
    for run in &paragraph.runs {
        let run_start = cursor;
        let run_end = run_start.saturating_add(run.text.len());
        if offset >= run_start && offset < run_end {
            return run.properties.clone();
        }
        cursor = run_end;
    }
    None
}

#[derive(Debug, Clone, Copy)]
enum HeadingInsertPosition {
    Before,
    After,
}

struct HeadingInsertSpec<'a> {
    heading: &'a str,
    text: &'a str,
    level: Option<u8>,
    occurrence: usize,
    position: HeadingInsertPosition,
}

fn insert_relative_to_heading(
    word_doc: &mut WordDocument,
    command_executor: &mut CommandExecutor,
    selection: &mut Option<SelectionRange>,
    command_index: usize,
    spec: HeadingInsertSpec<'_>,
) -> Result<(), HeadlessApiApplyError> {
    if spec.heading.trim().is_empty() {
        return Err(HeadlessApiApplyError::Command {
            index: command_index,
            source: CommandError::Custom("heading must not be empty".to_string()),
        });
    }
    let paragraph = resolve_heading_paragraph(word_doc, spec.heading, spec.level, spec.occurrence)
        .ok_or_else(|| HeadlessApiApplyError::Command {
            index: command_index,
            source: CommandError::Custom(format!(
                "heading {:?} occurrence {} was not found",
                spec.heading, spec.occurrence
            )),
        })?;
    let offset = match spec.position {
        HeadingInsertPosition::Before => 0,
        HeadingInsertPosition::After => paragraph_text(word_doc, paragraph)
            .map(|text| text.len())
            .unwrap_or(0),
    };
    let break_range = SelectionRange::collapsed(TextPosition::new(paragraph, offset));
    *selection = Some(break_range);
    {
        let mut ctx = CommandContext::new(word_doc, selection);
        command_executor
            .execute(
                Box::new(InsertParagraphBreaksCommand::new(break_range, 1)),
                &mut ctx,
            )
            .map_err(|source| HeadlessApiApplyError::Command {
                index: command_index,
                source,
            })?;
    }

    let insert_paragraph = match spec.position {
        HeadingInsertPosition::Before => paragraph,
        HeadingInsertPosition::After => paragraph.saturating_add(1),
    };
    let insert_range = SelectionRange::collapsed(TextPosition::new(insert_paragraph, 0));
    *selection = Some(insert_range);
    let mut ctx = CommandContext::new(word_doc, selection);
    command_executor
        .execute(
            Box::new(InsertTextCommand::new(
                insert_range,
                spec.text.to_string(),
                None,
            )),
            &mut ctx,
        )
        .map_err(|source| HeadlessApiApplyError::Command {
            index: command_index,
            source,
        })?;
    Ok(())
}

fn resolve_heading_paragraph(
    word_doc: &WordDocument,
    heading: &str,
    level: Option<u8>,
    occurrence: usize,
) -> Option<usize> {
    let target = heading.trim();
    outline_snapshots(word_doc)
        .into_iter()
        // Either the paragraph's own text or the text as the reader sees it,
        // numbering label included (`第１条　（秘密情報）`).
        .filter(|item| {
            item.text == target
                || heading_lookup_text(item.numbering_text.as_deref(), &item.text).trim() == target
        })
        .filter(|item| level.is_none_or(|target_level| item.level == target_level))
        .nth(occurrence)
        .map(|item| item.paragraph)
}

fn clone_table_row(
    word_doc: &mut WordDocument,
    table_order_index: usize,
    source_row: usize,
    target_row: usize,
    clear_text: bool,
) -> Result<(), String> {
    {
        let table = word_doc
            .table_mut_by_order(table_order_index)
            .ok_or_else(|| format!("table {table_order_index} was not found"))?;
        if source_row >= table.rows.len() {
            return Err(format!("source row {source_row} is out of range"));
        }
        let insert_at = target_row.min(table.rows.len());
        let mut row = table.rows[source_row].clone();
        clear_table_row_runtime_ids(&mut row);
        if clear_text {
            clear_table_row_text(&mut row);
        }
        table.rows.insert(insert_at, row);
    }

    word_doc.rewrite_table_xml_from_model(table_order_index)?;
    word_doc.rebuild_paragraph_surfaces();
    Ok(())
}

fn fill_table_row(
    word_doc: &mut WordDocument,
    table_order_index: usize,
    row: usize,
    values: &[String],
) -> Result<(), String> {
    // Resolve every target before mutating so an out-of-range logical column
    // cannot leave a partially filled row. These paragraph surface indices
    // also let us synchronize only the edited cell bodies below.
    let mut paragraph_indices = values
        .iter()
        .enumerate()
        .map(|(column, _)| {
            table_cell_first_paragraph_index(word_doc, table_order_index, row, column)
        })
        .collect::<Result<Vec<_>, _>>()?;

    {
        let table = word_doc
            .table_mut_by_order(table_order_index)
            .ok_or_else(|| format!("table {table_order_index} was not found"))?;
        let row_ref = table
            .rows
            .get_mut(row)
            .ok_or_else(|| format!("row {row} is out of range"))?;
        for (column, text) in values.iter().enumerate() {
            let cell_index = cell_index_by_logical_column(row_ref, column)
                .ok_or_else(|| format!("column {column} is out of range"))?;
            let cell = row_ref
                .cells
                .get_mut(cell_index)
                .ok_or_else(|| format!("column {column} is out of range"))?;
            let paragraph = first_paragraph_mut_in_blocks(&mut cell.blocks)
                .ok_or_else(|| "target cell has no paragraph".to_string())?;
            set_paragraph_plain_text(paragraph, text);
        }
    }

    // Filling a row changes paragraph content, not table structure. Rewriting
    // `<w:tbl>` here would make the raw-cell preservation path restore the
    // pre-fill body over the newly generated text. Synchronize the affected
    // paragraphs in place so bookmarks, OMML, nested tables and other sibling
    // cell content remain intact.
    paragraph_indices.sort_unstable();
    paragraph_indices.dedup();
    for paragraph_index in paragraph_indices {
        word_doc.sync_paragraph_text(paragraph_index);
    }
    Ok(())
}

fn set_paragraph_plain_text(paragraph: &mut Paragraph, text: &str) {
    if let Some(first) = paragraph.runs.first_mut() {
        first.text = text.to_string();
        for run in paragraph.runs.iter_mut().skip(1) {
            run.text.clear();
        }
    } else {
        paragraph.runs.push(Run {
            text: text.to_string(),
            properties: None,
            image: None,
            text_box: None,
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            revision_node_id: None,
            comment_reference_ids: Vec::new(),
            footnote_reference_ids: Vec::new(),
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: None,
        });
    }
}

fn clear_table_row_runtime_ids(row: &mut TableRow) {
    row.node_id = None;
    for cell in &mut row.cells {
        clear_table_cell_runtime_ids(cell);
    }
}

fn clear_table_cell_runtime_ids(cell: &mut TableCell) {
    cell.node_id = None;
    for block in &mut cell.blocks {
        clear_body_element_runtime_ids(block);
    }
}

fn clear_body_element_runtime_ids(block: &mut BodyElement) {
    match block {
        BodyElement::Paragraph(paragraph) => {
            paragraph.node_id = None;
            for run in &mut paragraph.runs {
                clear_run_runtime_ids(run);
            }
        }
        BodyElement::Table(table) => clear_table_runtime_ids(table),
        BodyElement::Math(_) => {}
    }
}

fn clear_table_runtime_ids(table: &mut Table) {
    table.node_id = None;
    for row in &mut table.rows {
        clear_table_row_runtime_ids(row);
    }
}

fn clear_run_runtime_ids(run: &mut Run) {
    run.node_id = None;
    run.text_node_id = None;
    run.revision_node_id = None;
    if let Some(text_box) = run.text_box.as_mut() {
        clear_text_box_runtime_ids(text_box);
    }
}

fn clear_text_box_runtime_ids(text_box: &mut RunTextBox) {
    for block in &mut text_box.blocks {
        clear_body_element_runtime_ids(block);
    }
}

fn clear_table_row_text(row: &mut TableRow) {
    for cell in &mut row.cells {
        for block in &mut cell.blocks {
            clear_body_element_text(block);
        }
    }
}

fn clear_body_element_text(block: &mut BodyElement) {
    match block {
        BodyElement::Paragraph(paragraph) => {
            for run in &mut paragraph.runs {
                run.text.clear();
                if let Some(text_box) = run.text_box.as_mut() {
                    for block in &mut text_box.blocks {
                        clear_body_element_text(block);
                    }
                }
            }
        }
        BodyElement::Table(table) => {
            for row in &mut table.rows {
                clear_table_row_text(row);
            }
        }
        BodyElement::Math(_) => {}
    }
}

fn paragraph_numbering_text(word_doc: &WordDocument, index: usize) -> Option<String> {
    numbering_texts(word_doc).get(index).cloned().flatten()
}

fn numbering_texts(word_doc: &WordDocument) -> Vec<Option<String>> {
    let total = word_doc.paragraph_count();
    let mut results = vec![None; total];
    let Some(numbering_part) = word_doc.numbering_part.as_ref() else {
        return results;
    };

    let mut counters = NumberingCounter::new();
    for (paragraph_index, entry) in results.iter_mut().enumerate().take(total) {
        let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
            continue;
        };
        let Some(numbering) = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.numbering.as_ref())
        else {
            continue;
        };
        let (Some(num_id), Some(level)) = (numbering.id.as_deref(), numbering.level) else {
            continue;
        };
        if !numbering_part.has_instance(num_id) {
            continue;
        }
        let abstract_num_id = numbering_part.abstract_num_id(num_id).unwrap_or_default();
        let start = numbering_part.start_value(num_id, level);
        let has_start_override = numbering_part.has_start_override(num_id, level);
        counters.initialize_ancestor_levels(Some(numbering_part), num_id, level);
        counters.increment_with(
            Some(numbering_part),
            num_id,
            abstract_num_id,
            level,
            start,
            has_start_override,
        );
        let marker = numbering_part.format_number(num_id, level, &counters);
        if !marker.is_empty() {
            *entry = Some(marker);
        }
    }

    results
}

fn table_cell_first_paragraph_index(
    word_doc: &WordDocument,
    table_order_index: usize,
    row: usize,
    column: usize,
) -> Result<usize, String> {
    let table = word_doc
        .table_by_order(table_order_index)
        .ok_or_else(|| format!("table {table_order_index} was not found"))?;
    let row_ref = table
        .rows
        .get(row)
        .ok_or_else(|| format!("row {row} is out of range"))?;
    let cell_index = cell_index_by_logical_column(row_ref, column)
        .ok_or_else(|| format!("column {column} is out of range"))?;
    let cell_ref = row_ref
        .cells
        .get(cell_index)
        .ok_or_else(|| format!("column {column} is out of range"))?;

    let target_ptr = first_paragraph_ptr_in_blocks(&cell_ref.blocks)
        .ok_or_else(|| "target cell has no paragraph".to_string())?;
    for paragraph_index in 0..word_doc.paragraph_count() {
        let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
            continue;
        };
        let paragraph_ptr: *const Paragraph = paragraph as *const Paragraph;
        if paragraph_ptr == target_ptr {
            return Ok(paragraph_index);
        }
    }

    Err("failed to locate target cell paragraph".to_string())
}

fn cell_index_by_logical_column(row: &crate::document::TableRow, column: usize) -> Option<usize> {
    let mut start = row.grid_before.unwrap_or(0) as usize;
    for (cell_index, cell) in row.cells.iter().enumerate() {
        let span = cell.span.unwrap_or(1).max(1) as usize;
        let end = start + span;
        if column >= start && column < end {
            return Some(cell_index);
        }
        start = end;
    }
    None
}

fn first_paragraph_ptr_in_blocks(blocks: &[BodyElement]) -> Option<*const Paragraph> {
    for block in blocks {
        match block {
            BodyElement::Paragraph(paragraph) => {
                return Some(paragraph.as_ref() as *const Paragraph);
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        if let Some(found) = first_paragraph_ptr_in_blocks(&cell.blocks) {
                            return Some(found);
                        }
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
    None
}

fn first_paragraph_mut_in_blocks(blocks: &mut [BodyElement]) -> Option<&mut Paragraph> {
    for block in blocks {
        match block {
            BodyElement::Paragraph(paragraph) => return Some(paragraph.as_mut()),
            BodyElement::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        if let Some(found) = first_paragraph_mut_in_blocks(&mut cell.blocks) {
                            return Some(found);
                        }
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
    None
}

fn paragraph_snapshot(
    word_doc: &WordDocument,
    paragraph_index: usize,
) -> Option<HeadlessParagraphSnapshot> {
    let paragraph = word_doc.paragraph(paragraph_index)?;
    let mut text = String::new();
    for run in &paragraph.runs {
        text.push_str(&run.text);
    }

    Some(HeadlessParagraphSnapshot {
        paragraph: paragraph_index,
        text,
        numbering_text: paragraph_numbering_text(word_doc, paragraph_index),
        alignment: paragraph
            .properties
            .as_ref()
            .and_then(|props| props.alignment.as_deref().map(str::to_string)),
        indent: paragraph
            .properties
            .as_ref()
            .and_then(|props| props.indent.as_ref())
            .map(|indent| HeadlessParagraphIndent {
                left: indent.left,
                right: indent.right,
                first_line: indent.first_line,
                hanging: indent.hanging,
            }),
        style_id: paragraph
            .properties
            .as_ref()
            .and_then(|props| props.style.as_deref().map(str::to_string)),
        spacing: paragraph
            .properties
            .as_ref()
            .and_then(|props| props.spacing.as_ref())
            .map(|spacing| HeadlessParagraphSpacing {
                before: spacing.before,
                after: spacing.after,
                line: spacing.line,
                line_rule: spacing.line_rule.clone(),
            }),
        runs: paragraph_runs_snapshot(paragraph),
    })
}

fn paragraph_runs_snapshot(paragraph: &crate::document::Paragraph) -> Vec<HeadlessRunSnapshot> {
    let mut runs = Vec::with_capacity(paragraph.runs.len());
    let mut byte_offset = 0usize;
    let mut char_offset = 0usize;

    for run in &paragraph.runs {
        let run_start = byte_offset;
        let run_end = run_start + run.text.len();
        let run_start_char = char_offset;
        let run_char_len = run.text.chars().count();
        let run_end_char = run_start_char + run_char_len;

        runs.push(HeadlessRunSnapshot {
            text: run.text.clone(),
            start_offset: run_start,
            end_offset: run_end,
            start_char_offset: run_start_char,
            end_char_offset: run_end_char,
            style: run_style_snapshot(run.properties.as_ref()),
        });

        byte_offset = run_end;
        char_offset = run_end_char;
    }

    runs
}

fn character_style_snapshot(
    word_doc: &WordDocument,
    paragraph_index: usize,
    offset: usize,
) -> Option<HeadlessCharacterStyleSnapshot> {
    let paragraph = word_doc.paragraph(paragraph_index)?;
    let paragraph_text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
    if paragraph_text.is_empty() {
        return None;
    }

    let clamped = offset.min(paragraph_text.len().saturating_sub(1));
    let mut run_start = 0usize;

    for run in &paragraph.runs {
        let run_end = run_start + run.text.len();
        if run.text.is_empty() {
            run_start = run_end;
            continue;
        }
        if clamped >= run_end {
            run_start = run_end;
            continue;
        }

        let local_target = clamped.saturating_sub(run_start);
        let local_start = clamp_to_char_boundary(&run.text, local_target);

        let run_start_char = paragraph_text[..run_start].chars().count();
        let run_end_char = paragraph_text[..run_end].chars().count();
        let char_offset = paragraph_text[..run_start + local_start].chars().count();

        return Some(HeadlessCharacterStyleSnapshot {
            paragraph: paragraph_index,
            offset,
            char_offset,
            run_start_offset: run_start,
            run_end_offset: run_end,
            run_start_char_offset: run_start_char,
            run_end_char_offset: run_end_char,
            run_text: run.text.clone(),
            // What the run looks like: its paragraph's and character
            // style's formatting under its own, as the renderer resolves it
            // — a heading's bold is bold at the caret too.
            style: run_style_snapshot(
                word_doc
                    .styles_part
                    .as_ref()
                    .and_then(|styles| {
                        styles.resolve_effective_run_properties(
                            paragraph
                                .properties
                                .as_ref()
                                .and_then(|props| props.style.as_deref()),
                            run.properties.as_ref(),
                        )
                    })
                    .as_ref()
                    .or(run.properties.as_ref()),
            ),
        });
    }

    None
}

fn run_style_snapshot(props: Option<&crate::document::RunProperties>) -> HeadlessRunStyle {
    let Some(props) = props else {
        return HeadlessRunStyle::default();
    };

    HeadlessRunStyle {
        bold: props.bold,
        italic: props.italic,
        underline: props.underline,
        strike: props.strike,
        baseline: props.baseline.as_deref().map(str::to_string),
        font_size: props.font_size,
        font_family: props.font_family.as_deref().map(str::to_string),
        color: props.color.as_deref().map(str::to_string),
    }
}

fn clamp_to_char_boundary(text: &str, index: usize) -> usize {
    let mut i = index.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i = i.saturating_sub(1);
    }
    i
}

fn extract_match_context(
    text: &str,
    start_offset: usize,
    end_offset: usize,
    context_before_chars: usize,
    context_after_chars: usize,
) -> (String, String, usize, usize) {
    let safe_start = start_offset.min(text.len());
    let safe_end = end_offset.min(text.len());
    let start_char_offset = text[..safe_start].chars().count();
    let end_char_offset = text[..safe_end].chars().count();
    let total_chars = text.chars().count();

    let context_start_char = start_char_offset.saturating_sub(context_before_chars);
    let context_end_char = end_char_offset
        .saturating_add(context_after_chars)
        .min(total_chars);

    let context_start_offset = char_index_to_byte_offset(text, context_start_char);
    let context_end_offset = char_index_to_byte_offset(text, context_end_char);

    let context_before = text
        .get(context_start_offset..safe_start)
        .unwrap_or("")
        .to_string();
    let context_after = text
        .get(safe_end..context_end_offset)
        .unwrap_or("")
        .to_string();

    (
        context_before,
        context_after,
        start_char_offset,
        end_char_offset,
    )
}

fn char_index_to_byte_offset(text: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    if char_index >= text.chars().count() {
        return text.len();
    }
    text.char_indices()
        .nth(char_index)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len())
}

/// The `w:id` constraint of an accept/reject command. `revision_id` and
/// `revision_ids` combine like the other filters: both given means a revision
/// must match both.
enum RevisionIdSelection {
    Any,
    One(String),
    Set(std::collections::HashSet<String>),
}

impl RevisionIdSelection {
    /// A listed revision's id stands for every piece of that change (see
    /// [`revision_groups`]), so deciding it decides all of them.
    fn new(
        word_doc: &WordDocument,
        revision_id: Option<&str>,
        revision_ids: Option<&[String]>,
    ) -> Self {
        let expanded = revision_ids.map(|ids| {
            ids.iter()
                .flat_map(|id| revision_id_group(word_doc, id))
                .collect::<Vec<_>>()
        });
        let revision_ids = expanded.as_deref();
        match (revision_id, revision_ids) {
            (None, None) => Self::Any,
            (Some(id), None) => match revision_id_group(word_doc, id).as_slice() {
                [single] => Self::One(single.clone()),
                group => Self::Set(group.iter().cloned().collect()),
            },
            (one, Some(ids)) => {
                let one = one.map(|id| revision_id_group(word_doc, id));
                Self::Set(
                    ids.iter()
                        .filter(|id| one.as_ref().is_none_or(|group| group.contains(id)))
                        .cloned()
                        .collect(),
                )
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::Set(ids) if ids.is_empty())
    }

    fn filter(&self) -> Option<RevisionIdFilter<'_>> {
        match self {
            Self::Any => None,
            Self::One(id) => Some(RevisionIdFilter::One(id)),
            Self::Set(ids) => Some(RevisionIdFilter::Set(ids)),
        }
    }
}

fn revision_kind_from_filter(kind: &HeadlessRevisionKindFilter) -> DocumentRevisionKind {
    match kind {
        HeadlessRevisionKindFilter::Inserted => DocumentRevisionKind::Inserted,
        HeadlessRevisionKindFilter::Deleted => DocumentRevisionKind::Deleted,
    }
}

fn table_count(word_doc: &WordDocument) -> usize {
    let mut count = 0usize;
    while word_doc.table_by_order(count).is_some() {
        count = count.saturating_add(1);
    }
    count
}

fn table_snapshot(
    word_doc: &WordDocument,
    table_order_index: usize,
) -> Option<HeadlessTableSnapshot> {
    let table = word_doc.table_by_order(table_order_index)?;
    let columns = table
        .columns
        .iter()
        .enumerate()
        .map(|(column, col)| HeadlessTableColumnSnapshot {
            column,
            width: col.width,
        })
        .collect();
    let rows = table
        .rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| table_row_snapshot_from_row(row_index, row))
        .collect();

    Some(HeadlessTableSnapshot {
        table_order_index,
        table_width: table.style.get("width").cloned(),
        alignment: table.style.get("text-align").cloned(),
        style: BTreeMap::from_iter(
            table
                .style
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        ),
        cell_style: BTreeMap::from_iter(
            table
                .cell_style
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        ),
        columns,
        rows,
    })
}

fn table_row_snapshot(
    word_doc: &WordDocument,
    table_order_index: usize,
    row: usize,
) -> Option<HeadlessTableRowSnapshot> {
    let table = word_doc.table_by_order(table_order_index)?;
    let row_ref = table.rows.get(row)?;
    Some(table_row_snapshot_from_row(row, row_ref))
}

fn table_column_snapshot(
    word_doc: &WordDocument,
    table_order_index: usize,
    column: usize,
) -> Option<HeadlessTableColumnDetailSnapshot> {
    let table = word_doc.table_by_order(table_order_index)?;
    let width = table.columns.get(column)?.width;
    let mut cells = Vec::new();
    for (row_index, row_ref) in table.rows.iter().enumerate() {
        let Some(boundary) = table_row_cell_boundaries(row_ref)
            .into_iter()
            .find(|boundary| column >= boundary.start && column < boundary.end)
        else {
            continue;
        };
        let Some(cell) = row_ref.cells.get(boundary.cell_index) else {
            continue;
        };
        cells.push(table_cell_snapshot_from_cell(
            row_index,
            boundary.start,
            cell,
        ));
    }
    Some(HeadlessTableColumnDetailSnapshot {
        column,
        width,
        cells,
    })
}

fn table_row_snapshot_from_row(
    row_index: usize,
    row: &crate::document::TableRow,
) -> HeadlessTableRowSnapshot {
    let cells = table_row_cell_boundaries(row)
        .into_iter()
        .filter_map(|boundary| {
            row.cells
                .get(boundary.cell_index)
                .map(|cell| table_cell_snapshot_from_cell(row_index, boundary.start, cell))
        })
        .collect();

    HeadlessTableRowSnapshot {
        row: row_index,
        is_header: row.is_header,
        height: row.height,
        height_rule: row.height_rule.map(|rule| format!("{rule:?}")),
        grid_before: row.grid_before,
        grid_after: row.grid_after,
        cells,
    }
}

fn table_cell_snapshot(
    word_doc: &WordDocument,
    table_order_index: usize,
    row: usize,
    column: usize,
) -> Option<HeadlessTableCellSnapshot> {
    let table = word_doc.table_by_order(table_order_index)?;
    let row_ref = table.rows.get(row)?;
    let boundary = table_row_cell_boundaries(row_ref)
        .into_iter()
        .find(|boundary| column >= boundary.start && column < boundary.end)?;
    let cell = row_ref.cells.get(boundary.cell_index)?;
    Some(table_cell_snapshot_from_cell(row, boundary.start, cell))
}

fn table_cell_snapshot_from_cell(
    row: usize,
    logical_column: usize,
    cell: &crate::document::TableCell,
) -> HeadlessTableCellSnapshot {
    HeadlessTableCellSnapshot {
        row,
        column: logical_column,
        span: cell.span,
        vertical_merge: cell.vertical_merge.clone(),
        text_direction: cell
            .text_direction
            .map(|direction| format!("{direction:?}")),
        text: collect_blocks_text(&cell.blocks),
        style: BTreeMap::from_iter(
            cell.style
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        ),
    }
}

fn table_row_cell_boundaries(row: &crate::document::TableRow) -> Vec<HeadlessTableRowBoundary> {
    let mut start = row.grid_before.unwrap_or(0) as usize;
    let mut boundaries = Vec::with_capacity(row.cells.len());
    for (cell_index, cell) in row.cells.iter().enumerate() {
        let span = cell.span.unwrap_or(1).max(1) as usize;
        let end = start + span;
        boundaries.push(HeadlessTableRowBoundary {
            cell_index,
            start,
            end,
        });
        start = end;
    }
    boundaries
}

struct HeadlessTableRowBoundary {
    cell_index: usize,
    start: usize,
    end: usize,
}

fn collect_blocks_text(blocks: &[crate::document::BodyElement]) -> String {
    let mut lines = Vec::new();
    collect_blocks_text_into(blocks, &mut lines);
    lines.join("\n")
}

fn collect_blocks_text_into(blocks: &[crate::document::BodyElement], out: &mut Vec<String>) {
    for block in blocks {
        match block {
            crate::document::BodyElement::Paragraph(paragraph) => {
                let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
                if !text.is_empty() {
                    out.push(text);
                }
            }
            crate::document::BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_blocks_text_into(&cell.blocks, out);
                    }
                }
            }
            crate::document::BodyElement::Math(_) => {}
        }
    }
}

fn comment_snapshots(word_doc: &WordDocument) -> Vec<HeadlessCommentSnapshot> {
    let mut anchors_by_id: std::collections::HashMap<String, Vec<HeadlessCommentAnchor>> =
        std::collections::HashMap::new();
    for paragraph_index in 0..word_doc.paragraph_count() {
        let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
            continue;
        };
        for range in &paragraph.comment_ranges {
            let marker_position = paragraph
                .comment_markers
                .iter()
                .find(|marker| marker.id == range.id)
                .map(|marker| marker.position);
            anchors_by_id
                .entry(range.id.clone())
                .or_default()
                .push(HeadlessCommentAnchor {
                    paragraph: paragraph_index,
                    start: range.start,
                    end: range.end,
                    marker_position,
                });
        }
    }

    let mut comments: Vec<_> = word_doc
        .comments
        .values()
        .map(|comment| {
            let mut anchors = anchors_by_id.remove(&comment.id).unwrap_or_default();
            anchors.sort_by_key(|anchor| (anchor.paragraph, anchor.start));
            HeadlessCommentSnapshot {
                id: comment.id.clone(),
                author: comment.author.clone(),
                initials: comment.initials.clone(),
                date: comment.date.clone(),
                resolved: comment.resolved,
                reply_to_comment_id: comment.thread_parent_id.clone(),
                text: comment.text.clone(),
                anchors,
            }
        })
        .collect();
    comments.sort_by(|a, b| a.id.cmp(&b.id));
    comments
}

fn comment_thread_snapshots(word_doc: &WordDocument) -> Vec<HeadlessCommentThreadSnapshot> {
    fn build_thread_node(
        by_id: &std::collections::HashMap<String, HeadlessCommentSnapshot>,
        children_by_parent: &std::collections::HashMap<String, Vec<String>>,
        id: &str,
        visiting: &mut std::collections::HashSet<String>,
    ) -> Option<HeadlessCommentThreadSnapshot> {
        if !visiting.insert(id.to_string()) {
            return None;
        }
        let mut replies = Vec::new();
        if let Some(children) = children_by_parent.get(id) {
            for child_id in children {
                if let Some(child) =
                    build_thread_node(by_id, children_by_parent, child_id, visiting)
                {
                    replies.push(child);
                }
            }
        }
        visiting.remove(id);
        let comment = by_id.get(id)?.clone();
        Some(HeadlessCommentThreadSnapshot { comment, replies })
    }

    let comments = comment_snapshots(word_doc);
    let by_id: std::collections::HashMap<String, HeadlessCommentSnapshot> = comments
        .iter()
        .map(|comment| (comment.id.clone(), comment.clone()))
        .collect();

    let mut children_by_parent: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut roots = Vec::new();
    for comment in &comments {
        if let Some(parent_id) = comment.reply_to_comment_id.as_ref() {
            if by_id.contains_key(parent_id) {
                children_by_parent
                    .entry(parent_id.clone())
                    .or_default()
                    .push(comment.id.clone());
                continue;
            }
        }
        roots.push(comment.id.clone());
    }
    for children in children_by_parent.values_mut() {
        children.sort();
    }

    roots.sort();
    let mut threads = Vec::new();
    let mut visiting = std::collections::HashSet::new();
    for root_id in roots {
        if let Some(thread) =
            build_thread_node(&by_id, &children_by_parent, &root_id, &mut visiting)
        {
            threads.push(thread);
        }
    }
    threads
}

pub(crate) fn revision_snapshots(word_doc: &WordDocument) -> Vec<HeadlessRevisionSnapshot> {
    revision_groups(word_doc)
        .into_iter()
        .map(|(snapshot, _)| snapshot)
        .collect()
}

/// Every `w:id` listed under the same revision as `id`: the pieces of one
/// change that [`revision_groups`] shows as one. An id no group lists stands
/// for itself.
fn revision_id_group(word_doc: &WordDocument, id: &str) -> Vec<String> {
    revision_groups(word_doc)
        .into_iter()
        .map(|(_, ids)| ids)
        .find(|ids| ids.iter().any(|candidate| candidate.as_deref() == Some(id)))
        .map(|ids| ids.into_iter().flatten().collect())
        .unwrap_or_else(|| vec![id.to_string()])
}

/// The `w:id`s a run target decides inside its span: its own, and every
/// piece of a change listed there under that id. A word typed with changes
/// tracked is one listed change stored as one `w:ins` and `w:id` a keystroke,
/// so the target the list hands out names only the first keystroke's id.
fn revision_target_run_ids(
    groups: &[(HeadlessRevisionSnapshot, Vec<Option<String>>)],
    target: &HeadlessRevisionTarget,
) -> Vec<Option<String>> {
    let mut ids = vec![target.id.clone()];
    for (listed, pieces) in groups {
        if !listed.paragraph_mark
            && listed.paragraph == target.paragraph
            && target.start_offset <= listed.start_offset
            && listed.end_offset <= target.end_offset
            && listed.id == target.id
        {
            for piece in pieces {
                if !ids.contains(piece) {
                    ids.push(piece.clone());
                }
            }
        }
    }
    ids
}

/// The tracked changes as Word's Reviewing pane lists them, each with the
/// `w:id`s of the elements it spans (`None` for an element without one).
/// Adjacent text with the same kind, author and date is one change even when
/// its pieces carry different ids: Word gives every `w:ins`/`w:del` element
/// its own id, so a change split by a comment boundary or by formatting part
/// of it arrives in several elements, jade's own export renumbers such pieces
/// the same way, and typing with changes tracked stores one a keystroke.
fn revision_groups(
    word_doc: &WordDocument,
) -> Vec<(HeadlessRevisionSnapshot, Vec<Option<String>>)> {
    let mut revisions: Vec<(HeadlessRevisionSnapshot, Vec<Option<String>>)> = Vec::new();
    let rows = word_doc.table_row_revisions();
    for paragraph_index in 0..word_doc.paragraph_count() {
        let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
            continue;
        };
        // A table row inserted or deleted, listed at its first paragraph.
        for row in rows
            .iter()
            .filter(|row| row.paragraphs.first() == Some(&paragraph_index))
        {
            let text = row
                .paragraphs
                .iter()
                .filter_map(|&index| word_doc.paragraph(index))
                .map(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\t");
            revisions.push((
                HeadlessRevisionSnapshot {
                    id: row.mark.id.clone(),
                    kind: if row.mark.is_deletion() {
                        "deletedRow"
                    } else {
                        "insertedRow"
                    }
                    .to_string(),
                    author: row.mark.author.clone(),
                    date: row.mark.date.clone(),
                    date_utc: row.mark.date_utc.clone(),
                    paragraph: paragraph_index,
                    start_offset: 0,
                    end_offset: paragraph.runs.iter().map(|run| run.text.len()).sum(),
                    text,
                    paragraph_mark: false,
                },
                vec![row.mark.id.clone()],
            ));
        }
        let mut offset = 0usize;
        for run in &paragraph.runs {
            let run_start = offset;
            let run_end = run_start.saturating_add(run.text.len());
            offset = run_end;
            // A formatting change (`w:rPrChange`), listed apart from any
            // insertion or deletion the same text is.
            if let Some(change) = run
                .properties
                .as_ref()
                .and_then(|properties| properties.change.as_deref())
            {
                let continues = revisions.last().is_some_and(|(last, _)| {
                    last.paragraph == paragraph_index
                        && !last.paragraph_mark
                        && last.kind == "formatted"
                        && last.end_offset == run_start
                        && last.author == change.author
                        && last.date == change.date
                });
                if continues {
                    if let Some((last, ids)) = revisions.last_mut() {
                        last.end_offset = run_end;
                        last.text.push_str(&run.text);
                        if !ids.contains(&change.id) {
                            ids.push(change.id.clone());
                        }
                    }
                } else {
                    revisions.push((
                        HeadlessRevisionSnapshot {
                            id: change.id.clone(),
                            kind: "formatted".to_string(),
                            author: change.author.clone(),
                            date: change.date.clone(),
                            date_utc: None,
                            paragraph: paragraph_index,
                            start_offset: run_start,
                            end_offset: run_end,
                            text: run.text.clone(),
                            paragraph_mark: false,
                        },
                        vec![change.id.clone()],
                    ));
                }
            }
            let Some(mark) = run.revision.as_ref() else {
                continue;
            };
            let kind = match mark.kind {
                DocumentRevisionKind::Inserted => "inserted",
                DocumentRevisionKind::Deleted => "deleted",
                DocumentRevisionKind::MovedFrom => "movedFrom",
                DocumentRevisionKind::MovedTo => "movedTo",
            }
            .to_string();

            if let Some((last, ids)) = revisions.last_mut() {
                if last.paragraph == paragraph_index
                    && !last.paragraph_mark
                    && last.end_offset == run_start
                    && last.kind == kind
                    && last.author == mark.author
                    && last.date == mark.date
                    && last.date_utc == mark.date_utc
                {
                    last.end_offset = run_end;
                    last.text.push_str(&run.text);
                    if !ids.contains(&mark.id) {
                        ids.push(mark.id.clone());
                    }
                    continue;
                }
            }

            revisions.push((
                HeadlessRevisionSnapshot {
                    id: mark.id.clone(),
                    kind,
                    author: mark.author.clone(),
                    date: mark.date.clone(),
                    date_utc: mark.date_utc.clone(),
                    paragraph: paragraph_index,
                    start_offset: run_start,
                    end_offset: run_end,
                    text: run.text.clone(),
                    paragraph_mark: false,
                },
                vec![mark.id.clone()],
            ));
        }
        // A change to the paragraph's own formatting (`w:pPrChange`).
        if let Some(change) = word_doc.paragraph_format_change(paragraph_index) {
            revisions.push((
                HeadlessRevisionSnapshot {
                    id: change.id.clone(),
                    kind: "formatted".to_string(),
                    author: change.author,
                    date: change.date,
                    date_utc: None,
                    paragraph: paragraph_index,
                    start_offset: 0,
                    end_offset: offset,
                    text: paragraph.runs.iter().map(|run| run.text.as_str()).collect(),
                    paragraph_mark: false,
                },
                vec![change.id],
            ));
        }
        if let Some(element) = word_doc
            .raw_document
            .as_ref()
            .and_then(|raw| paragraph.node_id.and_then(|id| raw.element(id)))
            .filter(|element| element.name.rsplit(':').next() == Some("p"))
        {
            revisions.extend(
                paragraph_mark_revision_snapshots(element, paragraph_index, offset)
                    .into_iter()
                    .map(|snapshot| {
                        let ids = vec![snapshot.id.clone()];
                        (snapshot, ids)
                    }),
            );
        }
    }
    revisions
}

/// Which revisions an editor-side review decision targets (Word の「この変更を
/// 反映」 over a selection, or 「表示されているすべての変更を反映」 under the
/// kind/reviewer filters). Every field is optional and they combine.
#[derive(Debug, Clone, Default)]
pub struct RevisionSelector {
    /// Model-space range, start <= end. A collapsed range is a caret.
    pub range: Option<(TextPosition, TextPosition)>,
    /// Snapshot kinds (`"inserted"`, `"deleted"`, `"movedFrom"`, `"movedTo"`).
    pub kinds: Option<Vec<String>>,
    /// Reviewers; `None` in the list stands for an unattributed revision.
    pub authors: Option<Vec<Option<String>>>,
}

/// The span a revision occupies in model coordinates. A paragraph mark sits
/// after the paragraph's last character, so it gets a one-unit span there:
/// a selection reaches it only when it runs past the end of the paragraph.
fn revision_span(revision: &HeadlessRevisionSnapshot) -> ((usize, usize), (usize, usize)) {
    let start = (revision.paragraph, revision.start_offset);
    let end = if revision.paragraph_mark {
        (revision.paragraph, revision.start_offset.saturating_add(1))
    } else {
        (revision.paragraph, revision.end_offset)
    };
    (start, end)
}

/// Picks the revisions `selector` targets, in document order.
///
/// A non-empty range takes every revision it overlaps, the way Word's
/// 「この変更を反映」 handles a selection. A caret takes the revision it sits in
/// (or, failing that, the one that ends right at it), matching Word's
/// behaviour when nothing is selected.
pub fn select_revisions<'a>(
    revisions: &'a [HeadlessRevisionSnapshot],
    selector: &RevisionSelector,
) -> Vec<&'a HeadlessRevisionSnapshot> {
    let shown = |revision: &&HeadlessRevisionSnapshot| revision_is_shown(revision, selector);
    let Some((start, end)) = selector.range else {
        return revisions.iter().filter(shown).collect();
    };
    let (start, end) = ((start.paragraph, start.offset), (end.paragraph, end.offset));
    if start < end {
        return revisions
            .iter()
            .filter(shown)
            .filter(|revision| {
                let (rev_start, rev_end) = revision_span(revision);
                rev_start < end && rev_end > start
            })
            .collect();
    }
    let containing: Vec<_> = revisions
        .iter()
        .filter(shown)
        .filter(|revision| {
            let (rev_start, rev_end) = revision_span(revision);
            rev_start <= start && start < rev_end
        })
        .collect();
    if !containing.is_empty() {
        return containing;
    }
    revisions
        .iter()
        .filter(shown)
        .filter(|revision| revision_span(revision).1 == start)
        .collect()
}

/// Whether `revision` passes `selector`'s kind and reviewer filters (its
/// range is not considered).
fn revision_is_shown(revision: &HeadlessRevisionSnapshot, selector: &RevisionSelector) -> bool {
    selector
        .kinds
        .as_ref()
        .is_none_or(|kinds| kinds.iter().any(|kind| kind == &revision.kind))
        && selector
            .authors
            .as_ref()
            .is_none_or(|authors| authors.contains(&revision.author))
}

/// The first revision at or after `position` that `selector`'s kind and
/// reviewer filters show, wrapping to the first such one in the document —
/// Word's 「次へ」 after 「反映して次へ進む」, which skips hidden markup.
pub fn next_revision_from<'a>(
    revisions: &'a [HeadlessRevisionSnapshot],
    position: TextPosition,
    selector: &RevisionSelector,
) -> Option<&'a HeadlessRevisionSnapshot> {
    let position = (position.paragraph, position.offset);
    let mut shown = revisions
        .iter()
        .filter(|revision| revision_is_shown(revision, selector));
    shown
        .clone()
        .find(|revision| revision_span(revision).0 >= position)
        .or_else(|| shown.next())
}

/// The engine targets for exactly these revisions: by position and id, so a
/// decision never reaches another revision that happens to share the id.
pub fn revision_targets_for(
    revisions: &[&HeadlessRevisionSnapshot],
) -> Vec<HeadlessRevisionTarget> {
    revisions
        .iter()
        .map(|revision| HeadlessRevisionTarget {
            paragraph: revision.paragraph,
            start_offset: revision.start_offset,
            end_offset: revision.end_offset,
            paragraph_mark: revision.paragraph_mark,
            id: revision.id.clone(),
        })
        .collect()
}

/// Where `position` — model coordinates in `word_doc` before the decision —
/// lands once `decided` revisions are accepted (`accept`) or rejected.
///
/// Removed text shifts later offsets in its paragraph left, and a position
/// inside it moves to where it started. Removing a paragraph mark joins its
/// paragraph with the next one, so later paragraphs move up by one and the
/// joined paragraph's offsets move behind the text left in the one before.
pub fn rebase_position_after_decision(
    word_doc: &WordDocument,
    decided: &[&HeadlessRevisionSnapshot],
    accept: bool,
    position: TextPosition,
) -> TextPosition {
    let removes = |revision: &HeadlessRevisionSnapshot| {
        matches!(
            (accept, revision.kind.as_str()),
            (true, "deleted" | "movedFrom") | (false, "inserted" | "movedTo")
        )
    };
    let removed_before = |paragraph: usize, offset: usize| -> usize {
        decided
            .iter()
            .filter(|revision| {
                !revision.paragraph_mark
                    && revision.paragraph == paragraph
                    && revision.start_offset < offset
                    && removes(revision)
            })
            .map(|revision| revision.end_offset.min(offset) - revision.start_offset)
            .sum()
    };
    let mut paragraph = position.paragraph;
    let mut offset = position
        .offset
        .saturating_sub(removed_before(position.paragraph, position.offset));
    let mut joins: Vec<usize> = decided
        .iter()
        .filter(|revision| revision.paragraph_mark && removes(revision))
        .map(|revision| revision.paragraph)
        .collect();
    joins.sort_unstable();
    joins.dedup();
    // The engine joins from the last paragraph up; replay it the same way.
    for join in joins.into_iter().rev() {
        if paragraph > join + 1 {
            paragraph -= 1;
        } else if paragraph == join + 1 {
            let len = word_doc.paragraph_text_len_at(join).unwrap_or(0);
            paragraph = join;
            offset += len.saturating_sub(removed_before(join, len));
        }
    }
    TextPosition::new(paragraph, offset)
}

fn paragraph_mark_revision_snapshots(
    paragraph: &XmlElement,
    paragraph_index: usize,
    offset: usize,
) -> Vec<HeadlessRevisionSnapshot> {
    fn children_named<'a>(
        element: &'a XmlElement,
        name: &'a str,
    ) -> impl Iterator<Item = &'a XmlElement> {
        element.children.iter().filter_map(move |node| match node {
            XmlNode::Element(child) if child.name.rsplit(':').next() == Some(name) => Some(child),
            _ => None,
        })
    }
    let mut revisions = Vec::new();
    for p_pr in children_named(paragraph, "pPr") {
        for r_pr in children_named(p_pr, "rPr") {
            for node in &r_pr.children {
                let XmlNode::Element(mark) = node else {
                    continue;
                };
                let kind = match mark.name.rsplit(':').next() {
                    Some("ins") => "inserted",
                    Some("del") => "deleted",
                    _ => continue,
                };
                let attr = |name: &str| {
                    mark.attributes
                        .iter()
                        .find(|attribute| attribute.name.rsplit(':').next() == Some(name))
                        .map(|attribute| attribute.value.clone())
                };
                revisions.push(HeadlessRevisionSnapshot {
                    id: attr("id"),
                    kind: kind.to_string(),
                    author: attr("author"),
                    date_utc: attr("dateUtc"),
                    date: attr("date"),
                    paragraph: paragraph_index,
                    start_offset: offset,
                    end_offset: offset,
                    text: "¶".to_string(),
                    paragraph_mark: true,
                });
            }
        }
    }
    revisions
}

fn revision_matches_filters(
    revision: &HeadlessRevisionSnapshot,
    kind: Option<&HeadlessRevisionKindFilter>,
    author: Option<&String>,
    date_from: Option<&String>,
    date_to: Option<&String>,
) -> bool {
    if let Some(kind) = kind {
        let expected_kind = revision_kind_to_str(*kind);
        if revision.kind != expected_kind {
            return false;
        }
    }

    if let Some(author) = author {
        if revision
            .author
            .as_ref()
            .is_none_or(|revision_author| revision_author != author)
        {
            return false;
        }
    }

    let revision_date = revision
        .date_utc
        .as_deref()
        .or(revision.date.as_deref())
        .unwrap_or_default();
    if (date_from.is_some() || date_to.is_some()) && revision_date.is_empty() {
        return false;
    }
    if let Some(date_from) = date_from {
        if revision_date < date_from.as_str() {
            return false;
        }
    }
    if let Some(date_to) = date_to {
        if revision_date > date_to.as_str() {
            return false;
        }
    }

    true
}

fn revision_kind_to_str(kind: HeadlessRevisionKindFilter) -> &'static str {
    match kind {
        HeadlessRevisionKindFilter::Inserted => "inserted",
        HeadlessRevisionKindFilter::Deleted => "deleted",
    }
}

fn outline_snapshots(word_doc: &WordDocument) -> Vec<HeadlessOutlineItem> {
    let numbering_texts = numbering_texts(word_doc);
    // Article titles are resolved over the whole body because the title of
    // `第１条` is often the `（秘密保持）` line above it, not part of the
    // heading paragraph at all.
    let inputs = paragraph_inputs(word_doc);
    let mut outline = Vec::new();
    for paragraph_index in 0..word_doc.paragraph_count() {
        let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
            continue;
        };
        let text = paragraph
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let style = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.style.as_deref().map(str::to_string));
        // The navigation pane's rule comes first, so every heading Word lists
        // there — localized style ids (`1`, `11`), custom styles based on a
        // heading, direct `w:outlineLvl` — is found here too. Only when no
        // outline level was resolved at all (a document built without a
        // styles part) does a built-in heading style id stand in for one; it
        // never overrides a resolved body text level (9).
        let has_outline_level = paragraph
            .properties
            .as_ref()
            .is_some_and(|props| props.outline_level.is_some());
        let outline_level = if has_outline_level {
            WordDocument::paragraph_heading_level(paragraph)
        } else {
            style
                .as_ref()
                .and_then(|value| parse_heading_level_from_style(value))
        };
        let numbering_text = numbering_texts.get(paragraph_index).cloned().flatten();
        // Japanese contracts almost never spell `第N条` out in a run: Word
        // renders it from `w:numPr` (`lvlText` = `第%1条　`), leaving only the
        // parenthesised article title in the body text. Heading detection has
        // to see the rendered label, otherwise such documents have no outline
        // at all.
        let text_level =
            parse_heading_level_from_text(&heading_lookup_text(numbering_text.as_deref(), trimmed));
        let Some(level) = outline_level.or(text_level) else {
            continue;
        };
        let title = article_title(&inputs, paragraph_index);
        outline.push(HeadlessOutlineItem {
            paragraph: paragraph_index,
            level,
            style,
            text: trimmed.to_string(),
            numbering_text,
            title: title.as_ref().map(|(title, _, _)| title.clone()),
            title_source: title
                .as_ref()
                .map(|(_, source, _)| source.as_str().to_string()),
        });
    }
    outline
}

/// Text a heading is recognised and matched by: the rendered numbering label
/// followed by the paragraph's own text.
fn heading_lookup_text(numbering_text: Option<&str>, text: &str) -> String {
    match numbering_text {
        Some(marker) => format!("{marker}{text}"),
        None => text.to_string(),
    }
}

fn parse_heading_level_from_style(style: &str) -> Option<u8> {
    let trimmed = style.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("heading") {
        let digits = lower
            .chars()
            .skip_while(|ch| !ch.is_ascii_digit())
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if digits.is_empty() {
            return Some(1);
        }
        return digits.parse::<u8>().ok();
    }

    if trimmed.starts_with("見出し") {
        let digits = trimmed
            .chars()
            .skip_while(|ch| !is_ascii_or_full_width_digit(*ch))
            .take_while(|ch| is_ascii_or_full_width_digit(*ch))
            .map(full_width_digit_to_ascii)
            .collect::<String>();
        if digits.is_empty() {
            return Some(1);
        }
        return digits.parse::<u8>().ok();
    }

    None
}

fn parse_heading_level_from_text(text: &str) -> Option<u8> {
    let trimmed = text.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '　');
    if trimmed.starts_with("附則") {
        return Some(2);
    }
    if !trimmed.starts_with('第') {
        return None;
    }
    if contains_heading_suffix_near_start(trimmed, '編') {
        return Some(1);
    }
    if contains_heading_suffix_near_start(trimmed, '章') {
        return Some(2);
    }
    if contains_heading_suffix_near_start(trimmed, '節') {
        return Some(3);
    }
    if contains_heading_suffix_near_start(trimmed, '款') {
        return Some(4);
    }
    // Articles are matched before `目` on purpose. The `目` probe only asks
    // whether the character shows up near the start, so `第1条（目的）` — the
    // single most common article title — would otherwise be reported as a
    // `目` heading.
    if is_article_heading(trimmed) {
        return Some(6);
    }
    if contains_heading_suffix_near_start(trimmed, '目') {
        return Some(5);
    }
    None
}

fn contains_heading_suffix_near_start(text: &str, suffix: char) -> bool {
    text.char_indices()
        .find(|(_, ch)| *ch == suffix)
        .is_some_and(|(idx, _)| idx <= 18)
}

/// The section a page-setup command addresses: the one holding the caret
/// (Word's "this section"), or the final section when there is no selection.
fn page_setup_section(word_doc: &WordDocument, selection: Option<SelectionRange>) -> Option<usize> {
    selection.and_then(|range| word_doc.section_index_for_paragraph(range.focus.paragraph))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        Body, BodyElement, CommentMarker, CommentRange, Document, DocumentComment, Numbering,
        Paragraph, ParagraphProperties, RevisionKind, RevisionMark, Run, RunProperties, Table,
        TableCell, TableColumn, TableLayoutMode, TableRow,
    };
    use crate::numbering::NumberingPart;
    use crate::word_document::{DocumentPart, WordDocument};
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::HeadlessChangeCaseMode;
    use jade_docx_reader::styles::StylesPart;
    use proptest::prelude::*;
    use std::collections::HashMap;

    fn paragraph_from_text(text: &str) -> Paragraph {
        Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        }
    }

    fn run_from_text(text: &str, properties: Option<RunProperties>) -> Run {
        Run {
            text: text.to_string(),
            properties,
            image: None,
            text_box: None,
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            revision_node_id: None,
            comment_reference_ids: Vec::new(),
            footnote_reference_ids: Vec::new(),
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: None,
        }
    }

    fn sample_document(text: &str) -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph_from_text(text))],
            }),
            ..Document::default()
        })
    }

    fn sample_document_with_styled_runs(text: &str, split_offsets: &[usize]) -> WordDocument {
        let mut points = split_offsets
            .iter()
            .copied()
            .filter(|offset| *offset <= text.len() && text.is_char_boundary(*offset))
            .collect::<Vec<_>>();
        points.extend([0, text.len()]);
        points.sort_unstable();
        points.dedup();

        let runs = points
            .windows(2)
            .filter(|window| window[0] < window[1])
            .enumerate()
            .map(|(marker, window)| {
                run_from_text(
                    &text[window[0]..window[1]],
                    Some(RunProperties {
                        style: Some(std::rc::Rc::from(format!("Style{marker}"))),
                        bold: Some(marker.is_multiple_of(2)),
                        italic: Some(marker.is_multiple_of(3)),
                        font_size: Some((marker + 8) as f32),
                        ..RunProperties::default()
                    }),
                )
            })
            .collect();

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs,
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    fn run_properties_by_byte(word_doc: &WordDocument, paragraph: usize) -> Vec<RunProperties> {
        word_doc
            .paragraph(paragraph)
            .expect("paragraph exists")
            .runs
            .iter()
            .flat_map(|run| {
                std::iter::repeat_n(run.properties.clone().unwrap_or_default(), run.text.len())
            })
            .collect()
    }

    fn run_signatures(word_doc: &WordDocument, paragraph: usize) -> Vec<(String, RunProperties)> {
        word_doc
            .paragraph(paragraph)
            .expect("paragraph exists")
            .runs
            .iter()
            .map(|run| (run.text.clone(), run.properties.clone().unwrap_or_default()))
            .collect()
    }

    fn reference_case(text: &str, mode: HeadlessChangeCaseMode) -> String {
        let mut output = String::with_capacity(text.len());
        let mut word_start = true;
        let mut sentence_start = true;
        for character in text.chars() {
            match mode {
                HeadlessChangeCaseMode::Upper => output.extend(character.to_uppercase()),
                HeadlessChangeCaseMode::Lower => output.extend(character.to_lowercase()),
                HeadlessChangeCaseMode::Title if word_start && character.is_alphabetic() => {
                    output.extend(character.to_uppercase());
                }
                HeadlessChangeCaseMode::Title => output.extend(character.to_lowercase()),
                HeadlessChangeCaseMode::Sentence if sentence_start && character.is_alphabetic() => {
                    output.extend(character.to_uppercase());
                }
                HeadlessChangeCaseMode::Sentence => output.extend(character.to_lowercase()),
                // The property tests only draw the four case modes.
                _ => output.push(character),
            }
            if mode == HeadlessChangeCaseMode::Title {
                if character.is_alphabetic() {
                    word_start = false;
                }
                if character.is_whitespace() {
                    word_start = true;
                }
            }
            if mode == HeadlessChangeCaseMode::Sentence {
                if character.is_alphabetic() {
                    sentence_start = false;
                }
                if matches!(character, '.' | '!' | '?' | '。' | '！' | '？') {
                    sentence_start = true;
                }
            }
        }
        output
    }

    fn undo_all(
        word_doc: &mut WordDocument,
        executor: &mut CommandExecutor,
        selection: &mut Option<SelectionRange>,
    ) {
        while executor.can_undo() {
            let mut context = CommandContext::new(word_doc, selection);
            executor.undo(&mut context).expect("undo succeeds");
        }
    }

    fn paragraph_with_para_id(para_id: &str, text: &str) -> Paragraph {
        let mut paragraph = paragraph_from_text(text);
        paragraph.properties = Some(ParagraphProperties {
            para_id: Some(para_id.into()),
            ..ParagraphProperties::default()
        });
        paragraph
    }

    /// Build a document whose paragraphs each carry a `w14:paraId` (mirrors how
    /// the parser populates `ParagraphProperties::para_id`).
    fn document_with_para_ids(items: &[(String, String)]) -> WordDocument {
        let elements = items
            .iter()
            .map(|(para_id, text)| {
                BodyElement::from_paragraph(paragraph_with_para_id(para_id, text))
            })
            .collect();
        WordDocument::from_document(Document {
            body: Some(Body { elements }),
            ..Document::default()
        })
    }

    /// Generate `n` paragraphs with guaranteed-unique paraIds (the index prefix
    /// keeps them distinct even when the random suffix collides) and ascii text.
    fn para_id_items_strategy() -> impl Strategy<Value = Vec<(String, String)>> {
        prop::collection::vec((any::<u16>(), "[a-z]{1,12}"), 1..8).prop_map(|rows| {
            rows.into_iter()
                .enumerate()
                .map(|(i, (seed, text))| (format!("{i:03}-{seed:04X}"), text))
                .collect()
        })
    }

    fn sample_document_from_paragraphs(paragraphs: &[String]) -> WordDocument {
        let elements = paragraphs
            .iter()
            .map(|text| BodyElement::from_paragraph(paragraph_from_text(text)))
            .collect();
        WordDocument::from_document(Document {
            body: Some(Body { elements }),
            ..Document::default()
        })
    }

    fn paragraph_text(word_doc: &WordDocument, index: usize) -> String {
        let paragraph = word_doc.paragraph(index).expect("paragraph exists");
        paragraph.runs.iter().map(|run| run.text.as_str()).collect()
    }

    fn paragraph_texts(word_doc: &WordDocument) -> Vec<String> {
        (0..word_doc.paragraph_count())
            .map(|index| paragraph_text(word_doc, index))
            .collect()
    }

    fn sample_table_document() -> WordDocument {
        // Keep the fixture intentionally tiny so table command tests stay deterministic.
        let make_cell = |text: &str| TableCell {
            blocks: vec![BodyElement::from_paragraph(Paragraph {
                runs: vec![Run {
                    text: text.to_string(),
                    properties: None,
                    image: None,
                    text_box: None,
                    math: None,
                    field: None,
                    hyperlink: None,
                    node_id: None,
                    text_node_id: None,
                    revision_node_id: None,
                    comment_reference_ids: Vec::new(),
                    footnote_reference_ids: Vec::new(),
                    endnote_reference_ids: Vec::new(),
                    note_separator: None,
                    has_footnote_ref: false,
                    has_endnote_ref: false,
                    revision: None,
                }],
                properties: None,
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            })],
            style: Default::default(),
            class_name: None,
            span: None,
            vertical_merge: None,
            text_direction: None,
            margins: None,
            vertical_align: None,
            node_id: None,
        };

        let table = Table {
            rows: vec![
                TableRow {
                    cells: vec![make_cell("A1"), make_cell("B1")],
                    ..TableRow::default()
                },
                TableRow {
                    cells: vec![make_cell("A2"), make_cell("B2")],
                    ..TableRow::default()
                },
            ],
            columns: vec![
                TableColumn { width: Some(40.0) },
                TableColumn { width: Some(40.0) },
            ],
            ..Table::default()
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_table(table)],
            }),
            ..Document::default()
        })
    }

    fn sample_table_document_with_named_styles() -> WordDocument {
        let mut document = sample_table_document();
        document.styles_part = Some(
            StylesPart::from_xml(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/></w:style>
  <w:style w:type="table" w:styleId="Alpha"><w:name w:val="Alpha Style"/><w:tcPr><w:shd w:fill="AABBCC"/></w:tcPr></w:style>
  <w:style w:type="table" w:styleId="Beta"><w:name w:val="Beta Style"/><w:basedOn w:val="Alpha"/><w:tblStylePr w:type="firstRow"><w:tcPr><w:shd w:fill="112233"/></w:tcPr></w:tblStylePr></w:style>
</w:styles>"#,
            )
            .expect("named table styles"),
        );
        document
    }

    fn sample_merged_table_document() -> WordDocument {
        let make_cell = |text: &str, span: Option<u32>| TableCell {
            blocks: vec![BodyElement::from_paragraph(paragraph_from_text(text))],
            style: Default::default(),
            class_name: None,
            span,
            vertical_merge: None,
            text_direction: None,
            margins: None,
            vertical_align: None,
            node_id: None,
        };

        let table = Table {
            rows: vec![
                TableRow {
                    cells: vec![make_cell("A", Some(2)), make_cell("C", None)],
                    ..TableRow::default()
                },
                TableRow {
                    cells: vec![
                        make_cell("D", None),
                        make_cell("E", None),
                        make_cell("F", None),
                    ],
                    ..TableRow::default()
                },
            ],
            columns: vec![
                TableColumn { width: Some(20.0) },
                TableColumn { width: Some(30.0) },
                TableColumn { width: Some(40.0) },
            ],
            ..Table::default()
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_table(table)],
            }),
            ..Document::default()
        })
    }

    fn sample_comment_document() -> WordDocument {
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: "Hello world".to_string(),
                        properties: None,
                        image: None,
                        text_box: None,
                        math: None,
                        field: None,
                        hyperlink: None,
                        node_id: None,
                        text_node_id: None,
                        revision_node_id: None,
                        comment_reference_ids: Vec::new(),
                        footnote_reference_ids: Vec::new(),
                        endnote_reference_ids: Vec::new(),
                        note_separator: None,
                        has_footnote_ref: false,
                        has_endnote_ref: false,
                        revision: None,
                    }],
                    properties: None,
                    node_id: None,
                    comment_ranges: vec![CommentRange {
                        id: "1".to_string(),
                        start: 0,
                        end: 5,
                    }],
                    comment_markers: vec![CommentMarker {
                        id: "1".to_string(),
                        position: 5,
                    }],
                })],
            }),
            ..Document::default()
        });

        let mut comments = HashMap::new();
        comments.insert(
            "1".to_string(),
            DocumentComment {
                id: "1".to_string(),
                author: Some("LLM".to_string()),
                initials: Some("LM".to_string()),
                date: Some("2026-02-24T00:00:00Z".to_string()),
                resolved: None,
                thread_parent_id: None,
                text: "comment".to_string(),
            },
        );
        word_doc.comments = comments;
        word_doc
    }

    fn sample_threaded_comment_document() -> WordDocument {
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: "Hello world".to_string(),
                        properties: None,
                        image: None,
                        text_box: None,
                        math: None,
                        field: None,
                        hyperlink: None,
                        node_id: None,
                        text_node_id: None,
                        revision_node_id: None,
                        comment_reference_ids: Vec::new(),
                        footnote_reference_ids: Vec::new(),
                        endnote_reference_ids: Vec::new(),
                        note_separator: None,
                        has_footnote_ref: false,
                        has_endnote_ref: false,
                        revision: None,
                    }],
                    properties: None,
                    node_id: None,
                    comment_ranges: vec![
                        CommentRange {
                            id: "1".to_string(),
                            start: 0,
                            end: 5,
                        },
                        CommentRange {
                            id: "2".to_string(),
                            start: 6,
                            end: 11,
                        },
                    ],
                    comment_markers: vec![
                        CommentMarker {
                            id: "1".to_string(),
                            position: 5,
                        },
                        CommentMarker {
                            id: "2".to_string(),
                            position: 11,
                        },
                    ],
                })],
            }),
            ..Document::default()
        });

        let mut comments = HashMap::new();
        comments.insert(
            "1".to_string(),
            DocumentComment {
                id: "1".to_string(),
                author: Some("LLM".to_string()),
                initials: Some("LM".to_string()),
                date: Some("2026-02-24T00:00:00Z".to_string()),
                resolved: Some(false),
                thread_parent_id: None,
                text: "root".to_string(),
            },
        );
        comments.insert(
            "2".to_string(),
            DocumentComment {
                id: "2".to_string(),
                author: Some("LLM".to_string()),
                initials: Some("LM".to_string()),
                date: Some("2026-02-24T00:01:00Z".to_string()),
                resolved: Some(false),
                thread_parent_id: Some("1".to_string()),
                text: "reply".to_string(),
            },
        );
        word_doc.comments = comments;
        word_doc
    }

    fn sample_revision_document() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![
                        Run {
                            text: "Hello ".to_string(),
                            properties: None,
                            image: None,
                            text_box: None,
                            math: None,
                            field: None,
                            hyperlink: None,
                            node_id: None,
                            text_node_id: None,
                            revision_node_id: None,
                            comment_reference_ids: Vec::new(),
                            footnote_reference_ids: Vec::new(),
                            endnote_reference_ids: Vec::new(),
                            note_separator: None,
                            has_footnote_ref: false,
                            has_endnote_ref: false,
                            revision: Some(RevisionMark {
                                kind: RevisionKind::Inserted,
                                id: Some("1".to_string()),
                                author: Some("LLM".to_string()),
                                date: Some("2026-02-25T00:00:00Z".to_string()),
                                date_utc: None,
                                insertion: None,
                            }),
                        },
                        Run {
                            text: "World".to_string(),
                            properties: None,
                            image: None,
                            text_box: None,
                            math: None,
                            field: None,
                            hyperlink: None,
                            node_id: None,
                            text_node_id: None,
                            revision_node_id: None,
                            comment_reference_ids: Vec::new(),
                            footnote_reference_ids: Vec::new(),
                            endnote_reference_ids: Vec::new(),
                            note_separator: None,
                            has_footnote_ref: false,
                            has_endnote_ref: false,
                            revision: Some(RevisionMark {
                                kind: RevisionKind::Deleted,
                                id: Some("2".to_string()),
                                author: Some("LLM".to_string()),
                                date: Some("2026-02-25T00:00:00Z".to_string()),
                                date_utc: None,
                                insertion: None,
                            }),
                        },
                    ],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    fn sample_revision_comment_document(kind: RevisionKind) -> WordDocument {
        let mut revised_prefix = run_from_text("He", None);
        revised_prefix.revision = Some(RevisionMark {
            kind,
            id: Some("1".to_string()),
            author: Some("LLM".to_string()),
            date: Some("2026-08-08T00:00:00Z".to_string()),
            date_utc: None,
            insertion: None,
        });

        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![revised_prefix, run_from_text("llo", None)],
                    properties: None,
                    node_id: None,
                    comment_ranges: vec![CommentRange {
                        id: "1".to_string(),
                        start: 2,
                        end: 5,
                    }],
                    comment_markers: vec![CommentMarker {
                        id: "1".to_string(),
                        position: 5,
                    }],
                })],
            }),
            ..Document::default()
        });
        word_doc.comments.insert(
            "1".to_string(),
            DocumentComment {
                id: "1".to_string(),
                author: Some("LLM".to_string()),
                initials: None,
                date: Some("2026-08-08T00:00:00Z".to_string()),
                resolved: None,
                thread_parent_id: None,
                text: "comment".to_string(),
            },
        );
        word_doc
    }

    /// One tracked change split across three runs, as Word writes it when the
    /// inserted text carries formatting changes partway through.
    fn split_run_revision_document() -> WordDocument {
        let mark = RevisionMark {
            kind: RevisionKind::Inserted,
            id: Some("7".to_string()),
            author: Some("法務".to_string()),
            date: Some("2026-08-01T00:00:00Z".to_string()),
            date_utc: None,
            insertion: None,
        };
        let run = |text: &str| Run {
            text: text.to_string(),
            properties: None,
            image: None,
            text_box: None,
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            revision_node_id: None,
            comment_reference_ids: Vec::new(),
            footnote_reference_ids: Vec::new(),
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: Some(mark.clone()),
        };
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![run("甲は、"), run("乙に対し"), run("通知する。")],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn revision_snapshots_merge_adjacent_runs_of_one_change() {
        // The review panel lists one row per change the reviewer made, not one
        // per run the file happens to store it in. A three-run insertion has to
        // come back as a single entry spanning the whole range, or accepting
        // "the change" from the panel would only take part of it.
        let word_doc = split_run_revision_document();
        let revisions = revision_snapshots(&word_doc);

        assert_eq!(revisions.len(), 1, "adjacent runs of one mark should merge");
        let revision = &revisions[0];
        assert_eq!(revision.id.as_deref(), Some("7"));
        assert_eq!(revision.text, "甲は、乙に対し通知する。");
        assert_eq!(revision.start_offset, 0);
        assert_eq!(revision.end_offset, "甲は、乙に対し通知する。".len());
        assert_eq!(revision.author.as_deref(), Some("法務"));
    }

    #[test]
    fn one_change_split_across_element_ids_lists_and_resolves_as_one() {
        // Word gives every `w:ins` element its own id, and jade's export
        // renumbers the pieces of a change that a comment boundary or partial
        // formatting split. The panel still shows one change, and accepting
        // it by the id the panel lists takes every piece.
        let mut word_doc = split_run_revision_document();
        for (run, id) in word_doc
            .paragraph_mut(0)
            .expect("paragraph")
            .runs
            .iter_mut()
            .zip(["7", "8", "9"])
        {
            run.revision.as_mut().expect("tracked").id = Some(id.to_string());
        }

        let revisions = revision_snapshots(&word_doc);
        assert_eq!(revisions.len(), 1, "one change, three elements");
        assert_eq!(revisions[0].id.as_deref(), Some("7"));
        assert_eq!(revisions[0].text, "甲は、乙に対し通知する。");

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: Some("7".to_string()),
                revision_ids: None,
                revision_targets: None,
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("accept");
        assert!(
            revision_snapshots(&word_doc).is_empty(),
            "accepting the listed id accepts every piece"
        );
        assert_eq!(paragraph_text(&word_doc, 0), "甲は、乙に対し通知する。");
    }

    #[test]
    fn a_word_typed_while_tracking_is_rejected_by_its_listed_target() {
        // Typing records each keystroke as its own `w:ins` with its own id.
        // The panel lists the word as one change; rejecting that change by
        // the target the panel built from it has to take every keystroke,
        // not just the first one.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/hello.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let original = paragraph_text(&word_doc, 0);
        let mut executor = CommandExecutor::new();
        let mut apply = |word_doc: &mut WordDocument, command: HeadlessCommand| {
            apply_headless_commands_with_executor_to_document(
                word_doc,
                &mut executor,
                None,
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
            .expect("command applies");
        };
        apply(
            &mut word_doc,
            HeadlessCommand::StartChangeTracking {
                author: Some("jade".to_string()),
                initials: None,
                date: Some("2026-10-20T00:00:00Z".to_string()),
                date_utc: None,
            },
        );
        for (offset, key) in "new ".chars().enumerate() {
            apply(
                &mut word_doc,
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset,
                    text: key.to_string(),
                },
            );
        }
        assert_eq!(paragraph_text(&word_doc, 0), format!("new {original}"));
        let ids = word_doc
            .paragraph(0)
            .expect("paragraph")
            .runs
            .iter()
            .filter_map(|run| run.revision.as_ref())
            .filter_map(|revision| revision.id.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), 4, "one w:ins and w:id a keystroke");

        let query = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Revisions {
                start_revision: 0,
                max_revisions: 10,
                kind: None,
                author: None,
                date_from: None,
                date_to: None,
            }],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &query).expect("query succeeds");
        let HeadlessQueryResult::Revisions { revisions, .. } = &response.results[0] else {
            panic!("expected revisions result")
        };
        let listed = revisions.iter().collect::<Vec<_>>();
        assert_eq!(
            listed
                .iter()
                .map(|revision| revision.text.as_str())
                .collect::<String>(),
            "new "
        );

        apply(
            &mut word_doc,
            HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: Some(revision_targets_for(&listed)),
            },
        );
        assert_eq!(paragraph_text(&word_doc, 0), original);
        assert!(revision_snapshots(&word_doc).is_empty());
    }

    #[test]
    fn revision_snapshots_include_paragraph_marks_from_review_fixtures() {
        for (fixture, kind, expected_ids, expected_date, expected_date_utc) in [
            (
                "numbering-del2.docx",
                "deleted",
                &["0", "1", "3"][..],
                "2025-11-28T15:48:00Z",
                "2025-11-28T06:48:00Z",
            ),
            (
                "numbering-with-review.docx",
                "inserted",
                &["2", "3"][..],
                "2025-11-23T19:42:00Z",
                "2025-11-23T10:42:00Z",
            ),
        ] {
            let path = format!(
                "{}/../../public/fixtures/{fixture}",
                env!("CARGO_MANIFEST_DIR")
            );
            let bytes = std::fs::read(path).expect("read review fixture");
            let word_doc = executor::block_on(WordDocument::load(&bytes, &Options::default()))
                .expect("load review fixture");
            let marks: Vec<_> = revision_snapshots(&word_doc)
                .into_iter()
                .filter(|revision| revision.paragraph_mark)
                .collect();
            assert_eq!(
                marks
                    .iter()
                    .map(|mark| mark.id.as_deref())
                    .collect::<Vec<_>>(),
                expected_ids.iter().map(|id| Some(*id)).collect::<Vec<_>>(),
                "{fixture}"
            );
            for mark in marks {
                assert_eq!(mark.kind, kind, "{fixture}");
                assert_eq!(mark.date.as_deref(), Some(expected_date), "{fixture}");
                assert_eq!(
                    mark.date_utc.as_deref(),
                    Some(expected_date_utc),
                    "{fixture}"
                );
                assert_eq!(mark.start_offset, mark.end_offset, "{fixture}");
                assert_eq!(mark.text, "¶", "{fixture}");
                assert!(mark.end_offset <= paragraph_text(&word_doc, mark.paragraph).len());
            }
        }
    }

    fn sample_revision_filter_document() -> WordDocument {
        let revision = |kind: RevisionKind, id: &str, author: &str, date_utc: &str| {
            Some(RevisionMark {
                kind,
                id: Some(id.to_string()),
                author: Some(author.to_string()),
                date: Some(date_utc.to_string()),
                date_utc: Some(date_utc.to_string()),
                insertion: None,
            })
        };
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![
                        Run {
                            text: "A".to_string(),
                            properties: None,
                            image: None,
                            text_box: None,
                            math: None,
                            field: None,
                            hyperlink: None,
                            node_id: None,
                            text_node_id: None,
                            revision_node_id: None,
                            comment_reference_ids: Vec::new(),
                            footnote_reference_ids: Vec::new(),
                            endnote_reference_ids: Vec::new(),
                            note_separator: None,
                            has_footnote_ref: false,
                            has_endnote_ref: false,
                            revision: revision(
                                RevisionKind::Inserted,
                                "10",
                                "Alice",
                                "2026-02-24T10:00:00Z",
                            ),
                        },
                        Run {
                            text: "B".to_string(),
                            properties: None,
                            image: None,
                            text_box: None,
                            math: None,
                            field: None,
                            hyperlink: None,
                            node_id: None,
                            text_node_id: None,
                            revision_node_id: None,
                            comment_reference_ids: Vec::new(),
                            footnote_reference_ids: Vec::new(),
                            endnote_reference_ids: Vec::new(),
                            note_separator: None,
                            has_footnote_ref: false,
                            has_endnote_ref: false,
                            revision: revision(
                                RevisionKind::Deleted,
                                "11",
                                "Bob",
                                "2026-02-25T10:00:00Z",
                            ),
                        },
                        Run {
                            text: "C".to_string(),
                            properties: None,
                            image: None,
                            text_box: None,
                            math: None,
                            field: None,
                            hyperlink: None,
                            node_id: None,
                            text_node_id: None,
                            revision_node_id: None,
                            comment_reference_ids: Vec::new(),
                            footnote_reference_ids: Vec::new(),
                            endnote_reference_ids: Vec::new(),
                            note_separator: None,
                            has_footnote_ref: false,
                            has_endnote_ref: false,
                            revision: revision(
                                RevisionKind::Inserted,
                                "12",
                                "Alice",
                                "2026-02-26T10:00:00Z",
                            ),
                        },
                    ],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    fn sample_outline_document() -> WordDocument {
        let paragraph = |text: &str, style: Option<&str>| Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: Some(ParagraphProperties {
                style: style.map(Into::into),
                ..ParagraphProperties::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph("Top", Some("Heading1"))),
                    BodyElement::from_paragraph(paragraph("Sub", Some("heading2"))),
                    BodyElement::from_paragraph(paragraph("第1章 総則", None)),
                    BodyElement::from_paragraph(paragraph("Body", Some("Normal"))),
                ],
            }),
            ..Document::default()
        })
    }

    /// 契約書でよくある「第N条」を `w:numPr` の採番で持つ文書。本文ランには
    /// 条見出しの括弧書きしか入っていない (`public/fixtures/nda.docx` と同型)。
    fn sample_article_numbering_document() -> WordDocument {
        let article = |text: &str| Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: Some(ParagraphProperties {
                numbering: Some(Numbering {
                    id: Some("300".to_string()),
                    level: Some(0),
                }),
                ..ParagraphProperties::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(article("（秘密情報）")),
                    BodyElement::from_paragraph(article("（反社会的勢力の排除）")),
                    BodyElement::from_paragraph(paragraph_from_text("本文")),
                ],
            }),
            ..Document::default()
        });

        let numbering_xml = r#"
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="300">
    <w:lvl w:ilvl="0">
      <w:start w:val="1"/>
      <w:numFmt w:val="decimalFullWidth"/>
      <w:lvlText w:val="第%1条　"/>
      <w:suff w:val="nothing"/>
    </w:lvl>
  </w:abstractNum>
  <w:num w:numId="300">
    <w:abstractNumId w:val="300"/>
  </w:num>
</w:numbering>
"#;
        word_doc.numbering_part = Some(NumberingPart::from_xml(numbering_xml).expect("numbering"));
        word_doc
    }

    fn sample_numbering_document() -> WordDocument {
        let paragraph_with_numbering = |text: &str| Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: Some(ParagraphProperties {
                numbering: Some(Numbering {
                    id: Some("1".to_string()),
                    level: Some(0),
                }),
                ..ParagraphProperties::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph_with_numbering("First")),
                    BodyElement::from_paragraph(paragraph_with_numbering("Second")),
                    BodyElement::from_paragraph(paragraph_from_text("Tail")),
                ],
            }),
            ..Document::default()
        });

        let numbering_xml = r#"
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="0">
    <w:lvl w:ilvl="0">
      <w:start w:val="1"/>
      <w:numFmt w:val="decimal"/>
      <w:lvlText w:val="%1."/>
    </w:lvl>
  </w:abstractNum>
  <w:num w:numId="1">
    <w:abstractNumId w:val="0"/>
  </w:num>
</w:numbering>
"#;
        word_doc.numbering_part = Some(NumberingPart::from_xml(numbering_xml).expect("numbering"));
        word_doc
    }

    fn paste_plain_text(
        word_doc: &mut WordDocument,
        executor: &mut CommandExecutor,
        caret: TextPosition,
        text: &str,
    ) -> Option<SelectionRange> {
        executor.begin_group();
        let selection = apply_headless_commands_with_executor_to_document(
            word_doc,
            executor,
            Some(SelectionRange::collapsed(caret)),
            &HeadlessEditRequest {
                commands: plain_text_paste_commands(caret, text),
            },
        )
        .expect("paste applies");
        executor.end_group();
        selection
    }

    #[test]
    fn plain_text_paste_into_a_list_is_one_undo_step_and_keeps_blank_items() {
        // Word pastes "a\n\nb" as three list items, the middle one empty: the
        // interactive "Enter on an empty item ends the list" rule does not
        // apply to pasted line breaks. One paste is one undo step.
        let mut word_doc = sample_numbering_document();
        let mut executor = CommandExecutor::new();
        let selection = paste_plain_text(
            &mut word_doc,
            &mut executor,
            TextPosition::new(0, "First".len()),
            "a\r\n\nb",
        );
        assert_eq!(
            paragraph_texts(&word_doc),
            vec!["Firsta", "", "b", "Second", "Tail"]
        );
        let numbered = (0..word_doc.paragraph_count())
            .map(|index| {
                word_doc
                    .paragraph(index)
                    .and_then(|paragraph| paragraph.properties.as_ref())
                    .and_then(|props| props.numbering.as_ref())
                    .is_some()
            })
            .collect::<Vec<_>>();
        assert_eq!(numbered, vec![true, true, true, true, false]);
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(2, 1)))
        );
        assert_eq!(executor.undo_depth(), 1);

        let mut selection = None;
        let mut context = CommandContext::new(&mut word_doc, &mut selection);
        assert!(executor.undo(&mut context).expect("undo succeeds"));
        assert_eq!(paragraph_texts(&word_doc), vec!["First", "Second", "Tail"]);
    }

    #[test]
    fn plain_text_paste_while_tracking_records_an_insertion() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/hello.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        word_doc.set_revision_session(Some(RevisionSession {
            author: Some("jade".to_string()),
            date: None,
            date_utc: None,
        }));
        let mut executor = CommandExecutor::new();
        paste_plain_text(
            &mut word_doc,
            &mut executor,
            TextPosition::new(1, 5),
            " A\nB",
        );
        assert_eq!(paragraph_text(&word_doc, 1), "World A");
        assert_eq!(paragraph_text(&word_doc, 2), "B");
        for (index, text) in [(1usize, " A"), (2, "B")] {
            assert!(
                word_doc.paragraph(index).unwrap().runs.iter().any(|run| {
                    run.text == text
                        && run
                            .revision
                            .as_ref()
                            .is_some_and(|revision| revision.kind == RevisionKind::Inserted)
                }),
                "paragraph {index} records {text:?} as an insertion",
            );
        }
        assert_eq!(executor.undo_depth(), 1);
    }

    #[test]
    fn replace_range_replaces_selected_text() {
        let mut word_doc = sample_document("abcdef");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 2,
                end_paragraph: 0,
                end_offset: 5,
                text: "XYZ".to_string(),
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        assert_eq!(paragraph_text(&word_doc, 0), "abXYZf");
    }

    #[test]
    fn replace_placeholder_spans_split_runs_and_preserves_first_run_style() {
        let placeholder_style = RunProperties {
            bold: Some(true),
            font_family: Some("Noto Serif JP".to_string().into()),
            ..RunProperties::default()
        };
        let paragraph = Paragraph {
            runs: vec![
                run_from_text("Hello ", None),
                run_from_text("${", Some(placeholder_style.clone())),
                run_from_text(
                    "name",
                    Some(RunProperties {
                        italic: Some(true),
                        ..RunProperties::default()
                    }),
                ),
                run_from_text("}", None),
                run_from_text("!", None),
            ],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        });
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplacePlaceholder {
                placeholder: "${name}".to_string(),
                text: "Alice".to_string(),
                max_replacements: None,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        assert_eq!(paragraph_text(&word_doc, 0), "Hello Alice!");
        let replacement_run = word_doc
            .paragraph(0)
            .expect("paragraph")
            .runs
            .iter()
            .find(|run| run.text.contains("Alice"))
            .expect("replacement run");
        let replacement_props = replacement_run
            .properties
            .as_ref()
            .expect("replacement should inherit placeholder style");
        assert_eq!(replacement_props.bold, Some(true));
        assert_eq!(
            replacement_props.font_family.as_deref(),
            Some("Noto Serif JP")
        );
        assert_ne!(replacement_props.italic, Some(true));
    }

    #[test]
    fn replace_placeholder_respects_max_replacements() {
        let mut word_doc = sample_document("${name} and ${name}");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplacePlaceholder {
                placeholder: "${name}".to_string(),
                text: "Alice".to_string(),
                max_replacements: Some(1),
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        assert_eq!(paragraph_text(&word_doc, 0), "Alice and ${name}");
    }

    #[test]
    fn insert_paragraph_break_splits_paragraph() {
        let mut word_doc = sample_document("HelloWorld");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertParagraphBreak {
                paragraph: 0,
                offset: 5,
                count: 1,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        assert_eq!(word_doc.paragraph_count(), 2);
        assert_eq!(paragraph_text(&word_doc, 0), "Hello");
        assert_eq!(paragraph_text(&word_doc, 1), "World");
    }

    #[test]
    fn insert_after_heading_inserts_paragraph_after_matching_outline_item() {
        let mut heading = paragraph_from_text("Invoice Details");
        heading.properties = Some(ParagraphProperties {
            style: Some("Heading1".to_string().into()),
            ..ParagraphProperties::default()
        });
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(heading),
                    BodyElement::from_paragraph(paragraph_from_text("Existing body")),
                ],
            }),
            ..Document::default()
        });
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertAfterHeading {
                heading: "Invoice Details".to_string(),
                text: "Inserted line".to_string(),
                level: Some(1),
                occurrence: 0,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        assert_eq!(
            paragraph_texts(&word_doc),
            vec![
                "Invoice Details".to_string(),
                "Inserted line".to_string(),
                "Existing body".to_string()
            ]
        );
    }

    #[test]
    fn insert_before_heading_uses_occurrence_to_disambiguate_duplicate_headings() {
        let make_heading = || {
            let mut paragraph = paragraph_from_text("Terms");
            paragraph.properties = Some(ParagraphProperties {
                style: Some("Heading2".to_string().into()),
                ..ParagraphProperties::default()
            });
            paragraph
        };
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(make_heading()),
                    BodyElement::from_paragraph(paragraph_from_text("First body")),
                    BodyElement::from_paragraph(make_heading()),
                    BodyElement::from_paragraph(paragraph_from_text("Second body")),
                ],
            }),
            ..Document::default()
        });
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertBeforeHeading {
                heading: "Terms".to_string(),
                text: "Inserted before second".to_string(),
                level: Some(2),
                occurrence: 1,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        assert_eq!(
            paragraph_texts(&word_doc),
            vec![
                "Terms".to_string(),
                "First body".to_string(),
                "Inserted before second".to_string(),
                "Terms".to_string(),
                "Second body".to_string()
            ]
        );
    }

    #[test]
    fn insert_paragraph_break_renumbers_following_numbered_paragraphs() {
        let mut word_doc = sample_numbering_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertParagraphBreak {
                paragraph: 0,
                offset: 5,
                count: 1,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let query = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::ParagraphSlice {
                start_paragraph: 0,
                max_paragraphs: 4,
            }],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &query).expect("query succeeds");
        let HeadlessQueryResult::ParagraphSlice { paragraphs, .. } = &response.results[0] else {
            panic!("expected paragraph_slice result")
        };

        assert_eq!(paragraphs.len(), 4);
        assert_eq!(paragraphs[0].text, "First");
        assert_eq!(paragraphs[0].numbering_text.as_deref(), Some("1."));
        assert_eq!(paragraphs[1].text, "");
        assert_eq!(paragraphs[1].numbering_text.as_deref(), Some("2."));
        assert_eq!(paragraphs[2].text, "Second");
        assert_eq!(paragraphs[2].numbering_text.as_deref(), Some("3."));
        assert_eq!(paragraphs[3].text, "Tail");
        assert_eq!(paragraphs[3].numbering_text, None);
    }

    #[test]
    fn formatting_commands_apply_to_selected_range() {
        let mut word_doc = sample_document("HelloWorld");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::ApplyBold {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    enabled: true,
                },
                HeadlessCommand::ApplyItalic {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    enabled: true,
                },
                HeadlessCommand::ApplyUnderline {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    kind: Some("words".to_string()),
                    enabled: true,
                },
                HeadlessCommand::ApplyStrike {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    double: true,
                    enabled: true,
                },
                HeadlessCommand::SetFontSize {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    font_size: 18.0,
                },
                HeadlessCommand::SetFontFamily {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    font_family: "Noto Serif JP".to_string(),
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        let formatted_run = paragraph.runs.first().expect("formatted run exists");
        let props = formatted_run
            .properties
            .as_ref()
            .expect("run properties exist");

        assert_eq!(props.bold, Some(true));
        assert_eq!(props.italic, Some(true));
        assert_eq!(props.underline, Some(true));
        assert_eq!(props.underline_kind.as_deref(), Some("words"));
        assert_eq!(props.strike, Some(false));
        assert_eq!(props.double_strike, Some(true));
        assert_eq!(props.font_size, Some(18.0));
        assert_eq!(props.font_family.as_deref(), Some("Noto Serif JP"));
    }

    #[test]
    fn baseline_commands_apply_to_selected_range() {
        let mut word_doc = sample_document("HelloWorld");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::ApplySuperscript {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    enabled: true,
                },
                HeadlessCommand::ApplySubscript {
                    start_paragraph: 0,
                    start_offset: 5,
                    end_paragraph: 0,
                    end_offset: 10,
                    enabled: true,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        assert!(
            paragraph.runs.iter().any(|run| {
                run.properties.as_ref().and_then(|p| p.baseline.as_deref()) == Some("superscript")
            }),
            "superscript should be applied"
        );
        assert!(
            paragraph.runs.iter().any(|run| {
                run.properties.as_ref().and_then(|p| p.baseline.as_deref()) == Some("subscript")
            }),
            "subscript should be applied"
        );
    }

    #[test]
    fn query_paragraph_count_and_slice() {
        let word_doc = sample_document_from_paragraphs(&[
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
        ]);
        let request = HeadlessQueryRequest {
            queries: vec![
                HeadlessQuery::ParagraphCount,
                HeadlessQuery::ParagraphSlice {
                    start_paragraph: 1,
                    max_paragraphs: 5,
                },
            ],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");

        assert_eq!(response.results.len(), 2);
        assert_eq!(
            response.results[0],
            HeadlessQueryResult::ParagraphCount { paragraph_count: 3 }
        );
        assert_eq!(
            response.results[1],
            HeadlessQueryResult::ParagraphSlice {
                start_paragraph: 1,
                paragraphs: vec![
                    HeadlessParagraphText {
                        paragraph: 1,
                        text: "beta".to_string(),
                        numbering_text: None
                    },
                    HeadlessParagraphText {
                        paragraph: 2,
                        text: "gamma".to_string(),
                        numbering_text: None
                    }
                ],
                has_more: false
            }
        );
    }

    #[test]
    fn query_citation_sources_returns_the_word_catalog() {
        let mut word_doc = sample_document("Hello");
        let source = HeadlessCitationSource {
            tag: "tanaka-2026".to_string(),
            source_type: "JournalArticle".to_string(),
            author: "Tanaka, Taro".to_string(),
            title: "Jade citation round trip".to_string(),
            year: "2026".to_string(),
            publisher: Some("SuguRaku Journal".to_string()),
            url: Some("https://example.test/citation".to_string()),
        };
        word_doc.parts.insert(
            "customXml/item1.xml".to_string(),
            DocumentPart {
                path: "customXml/item1.xml".to_string(),
                relationships: Vec::new(),
                content: r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
  <b:Source>
    <b:Tag>tanaka-2026</b:Tag>
    <b:SourceType>JournalArticle</b:SourceType>
    <b:Author><b:Author><b:NameList><b:Person><b:Last>Tanaka</b:Last><b:First>Taro</b:First></b:Person></b:NameList></b:Author></b:Author>
    <b:Title>Jade citation round trip</b:Title>
    <b:Year>2026</b:Year>
    <b:JournalName>SuguRaku Journal</b:JournalName>
    <b:URL>https://example.test/citation</b:URL>
  </b:Source>
</b:Sources>"#
                    .to_string(),
                parsed_document: None,
            },
        );

        let response = execute_headless_queries_on_document(
            &word_doc,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::CitationSources],
            },
        )
        .expect("catalog query succeeds");

        assert_eq!(
            response.results,
            vec![HeadlessQueryResult::CitationSources {
                sources: vec![source]
            }]
        );
    }

    #[test]
    fn query_search_text_returns_context_and_offsets() {
        let word_doc = sample_document_from_paragraphs(&[
            "先頭の文".to_string(),
            "申請の詳細を確認する".to_string(),
            "末尾".to_string(),
        ]);
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::SearchText {
                query: "詳細".to_string(),
                start_paragraph: 0,
                max_results: 10,
                context_before_chars: 2,
                context_after_chars: 3,
                case_insensitive: false,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");

        assert_eq!(response.results.len(), 1);
        let HeadlessQueryResult::SearchText {
            query,
            matches,
            truncated,
        } = &response.results[0]
        else {
            panic!("expected search result");
        };
        assert_eq!(query, "詳細");
        assert!(!truncated);
        assert_eq!(matches.len(), 1);
        let found = &matches[0];
        assert_eq!(found.paragraph, 1);
        assert_eq!(found.matched_text, "詳細");
        assert_eq!(found.start_offset, "申請の".len());
        assert_eq!(found.end_offset, "申請の詳細".len());
        assert_eq!(found.start_char_offset, 3);
        assert_eq!(found.end_char_offset, 5);
        assert_eq!(found.context_before, "請の");
        assert_eq!(found.context_after, "を確認");
        assert_eq!(found.snippet, "請の詳細を確認");
    }

    #[test]
    fn query_search_text_rejects_empty_query() {
        let word_doc = sample_document("abc");
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::SearchText {
                query: String::new(),
                start_paragraph: 0,
                max_results: 10,
                context_before_chars: 2,
                context_after_chars: 2,
                case_insensitive: false,
            }],
        };

        let err = execute_headless_queries_on_document(&word_doc, &request)
            .expect_err("empty query should fail");
        assert!(matches!(
            err,
            HeadlessApiQueryError::InvalidQuery { index: 0, .. }
        ));
    }

    #[test]
    fn query_paragraph_returns_runs_and_style() {
        let mut word_doc = sample_document("HelloWorld");
        let format_request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetFontFamily {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: 5,
                font_family: "Noto Serif JP".to_string(),
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &format_request).expect("format");

        let query_request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Paragraph { paragraph: 0 }],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &query_request).expect("query");

        let HeadlessQueryResult::Paragraph {
            paragraph,
            value: Some(value),
        } = &response.results[0]
        else {
            panic!("expected paragraph result");
        };
        assert_eq!(*paragraph, 0);
        assert_eq!(value.text, "HelloWorld");
        assert!(value
            .runs
            .iter()
            .any(|run| run.style.font_family.as_deref() == Some("Noto Serif JP")));
    }

    #[test]
    fn query_character_style_at_returns_font_info() {
        let mut word_doc = sample_document("HelloWorld");
        let format_request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetFontSize {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: 5,
                font_size: 18.0,
            }],
        };
        apply_headless_commands_to_document(&mut word_doc, &format_request).expect("format");

        let query_request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::CharacterStyleAt {
                paragraph: 0,
                offset: 1,
            }],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &query_request).expect("query");
        let HeadlessQueryResult::CharacterStyleAt {
            value: Some(style), ..
        } = &response.results[0]
        else {
            panic!("expected character style result");
        };
        assert_eq!(style.style.font_size, Some(18.0));
    }

    #[test]
    fn query_table_count_and_table_snapshot_returns_structure() {
        let word_doc = sample_table_document();
        let request = HeadlessQueryRequest {
            queries: vec![
                HeadlessQuery::TableCount,
                HeadlessQuery::Table {
                    table_order_index: 0,
                },
            ],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");

        assert_eq!(response.results.len(), 2);
        assert_eq!(
            response.results[0],
            HeadlessQueryResult::TableCount { table_count: 1 }
        );

        let HeadlessQueryResult::Table {
            value: Some(table), ..
        } = &response.results[1]
        else {
            panic!("expected table snapshot")
        };
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[0].width, Some(40.0));
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells[0].text, "A1");
    }

    #[test]
    fn query_table_cell_returns_value_for_logical_column() {
        let word_doc = sample_table_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::TableCell {
                table_order_index: 0,
                row: 0,
                column: 1,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::TableCell {
            value: Some(cell), ..
        } = &response.results[0]
        else {
            panic!("expected table cell result")
        };
        assert_eq!(cell.text, "B1");
    }

    #[test]
    fn query_table_row_returns_logical_columns_for_merged_cells() {
        let word_doc = sample_merged_table_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::TableRow {
                table_order_index: 0,
                row: 0,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::TableRow {
            value: Some(row), ..
        } = &response.results[0]
        else {
            panic!("expected table row result")
        };
        assert_eq!(row.cells.len(), 2);
        assert_eq!(row.cells[0].column, 0);
        assert_eq!(row.cells[0].span, Some(2));
        assert_eq!(row.cells[0].text, "A");
        assert_eq!(row.cells[1].column, 2);
        assert_eq!(row.cells[1].text, "C");
    }

    #[test]
    fn query_table_column_returns_covering_cells_for_each_row() {
        let word_doc = sample_merged_table_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::TableColumn {
                table_order_index: 0,
                column: 1,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::TableColumn {
            value: Some(column),
            ..
        } = &response.results[0]
        else {
            panic!("expected table column result")
        };
        assert_eq!(column.width, Some(30.0));
        assert_eq!(column.cells.len(), 2);
        assert_eq!(column.cells[0].row, 0);
        assert_eq!(column.cells[0].column, 0);
        assert_eq!(column.cells[0].span, Some(2));
        assert_eq!(column.cells[0].text, "A");
        assert_eq!(column.cells[1].row, 1);
        assert_eq!(column.cells[1].column, 1);
        assert_eq!(column.cells[1].text, "E");
    }

    #[test]
    fn query_table_column_skips_rows_without_covering_cell() {
        let make_cell = |text: &str| TableCell {
            blocks: vec![BodyElement::from_paragraph(paragraph_from_text(text))],
            style: Default::default(),
            class_name: None,
            span: None,
            vertical_merge: None,
            text_direction: None,
            margins: None,
            vertical_align: None,
            node_id: None,
        };
        let table = Table {
            rows: vec![
                TableRow {
                    grid_before: Some(1),
                    cells: vec![make_cell("R0C1"), make_cell("R0C2")],
                    ..TableRow::default()
                },
                TableRow {
                    cells: vec![make_cell("R1C0"), make_cell("R1C1"), make_cell("R1C2")],
                    ..TableRow::default()
                },
            ],
            columns: vec![
                TableColumn { width: Some(10.0) },
                TableColumn { width: Some(20.0) },
                TableColumn { width: Some(30.0) },
            ],
            ..Table::default()
        };
        let word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_table(table)],
            }),
            ..Document::default()
        });
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::TableColumn {
                table_order_index: 0,
                column: 0,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::TableColumn {
            value: Some(column),
            ..
        } = &response.results[0]
        else {
            panic!("expected table column result")
        };
        assert_eq!(column.cells.len(), 1);
        assert_eq!(column.cells[0].row, 1);
        assert_eq!(column.cells[0].column, 0);
        assert_eq!(column.cells[0].text, "R1C0");
    }

    /// レビュー用の検索は、原稿側の大文字小文字に左右されないこと。
    ///
    /// 定義語が本文で表記ゆれしていても拾えないと、契約書レビューの起点である
    /// 「その語がどこに出てくるか」が取りこぼされる。
    #[test]
    fn query_search_text_can_ignore_ascii_case() {
        let word_doc = sample_document_from_paragraphs(&[
            "Force Majeure shall mean …".to_string(),
            "本条の force majeure には …".to_string(),
        ]);
        let search = |case_insensitive: bool| {
            let request = HeadlessQueryRequest {
                queries: vec![HeadlessQuery::SearchText {
                    query: "Force Majeure".to_string(),
                    start_paragraph: 0,
                    max_results: 10,
                    context_before_chars: 0,
                    context_after_chars: 0,
                    case_insensitive,
                }],
            };
            let response =
                execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
            let HeadlessQueryResult::SearchText { matches, .. } = &response.results[0] else {
                panic!("expected search result");
            };
            matches.clone()
        };

        assert_eq!(
            search(false).len(),
            1,
            "case-sensitive finds only the exact one"
        );

        let found = search(true);
        assert_eq!(found.len(), 2, "case-insensitive finds both spellings");
        assert_eq!(found[1].paragraph, 1);
        // The document's own casing comes back, and the offsets index it.
        assert_eq!(found[1].matched_text, "force majeure");
        let paragraph = paragraph_text(&word_doc, 1);
        assert_eq!(
            &paragraph[found[1].start_offset..found[1].end_offset],
            "force majeure",
            "offsets must index the original text, not the folded copy",
        );
    }

    /// 未解決コメントだけを、件数まで含めて取り出せること。
    #[test]
    fn query_comments_filters_by_author_and_resolved_state() {
        let mut word_doc = sample_comment_document();
        let mut comments = word_doc.comments.clone();
        comments.insert(
            "2".to_string(),
            DocumentComment {
                id: "2".to_string(),
                author: Some("Counsel".to_string()),
                initials: None,
                date: None,
                resolved: Some(true),
                thread_parent_id: None,
                text: "resolved note".to_string(),
            },
        );
        word_doc.comments = comments;

        let query = |author: Option<&str>, resolved: Option<bool>| {
            let request = HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Comments {
                    start_comment: 0,
                    max_comments: 10,
                    author: author.map(str::to_string),
                    resolved,
                }],
            };
            let response =
                execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
            let HeadlessQueryResult::Comments {
                comments,
                total_comments,
                ..
            } = &response.results[0]
            else {
                panic!("expected comments result");
            };
            (comments.clone(), *total_comments)
        };

        let (unresolved, total) = query(None, Some(false));
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].id, "1");
        assert_eq!(
            total, 1,
            "the total must count the filtered set, not the document",
        );

        let (by_author, _) = query(Some("Counsel"), None);
        assert_eq!(by_author.len(), 1);
        assert_eq!(by_author[0].id, "2");

        assert_eq!(query(Some("Nobody"), None).0.len(), 0);
    }

    /// 変更履歴を 1 件だけ採否できること。
    ///
    /// 相手方の修正を一括ではなく個別に採否するのがレビューの実務で、
    /// `revisions` が返す `id` をそのまま渡せる必要がある。
    #[test]
    fn accept_revisions_can_target_a_single_revision_id() {
        let mut word_doc = sample_revision_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: Some("1".to_string()),
                revision_ids: None,
                revision_targets: None,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        // Revision 1 (an insertion) was accepted, so its text stays as normal
        // text; revision 2 (a deletion) is untouched and still marked.
        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        let marks: Vec<_> = paragraph
            .runs
            .iter()
            .filter_map(|run| run.revision.as_ref())
            .map(|mark| mark.id.clone().unwrap_or_default())
            .collect();
        assert_eq!(marks, vec!["2".to_string()], "only revision 1 was resolved");
        assert!(paragraph
            .runs
            .iter()
            .any(|run| run.text == "Hello " && run.revision.is_none()));
    }

    /// スタイルを名指しできること。
    ///
    /// `set_paragraph_style` は `style_id` を要求するので、文書が持つ id を
    /// 一覧できなければそのコマンドは使えない。
    #[test]
    fn query_styles_lists_the_document_style_catalogue() {
        let mut word_doc = sample_document("body");
        word_doc.styles_part = Some(
            StylesPart::from_xml(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style>
  <w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/></w:style>
</w:styles>"#,
            )
            .expect("styles parse"),
        );

        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Styles { kind: None }],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::Styles { styles } = &response.results[0] else {
            panic!("expected styles result");
        };

        let paragraph_styles: Vec<&str> = styles
            .iter()
            .filter(|style| style.kind == "paragraph")
            .map(|style| style.style_id.as_str())
            .collect();
        assert!(paragraph_styles.contains(&"Heading1"), "{styles:?}");
        assert!(paragraph_styles.contains(&"Normal"), "{styles:?}");
        let heading = styles
            .iter()
            .find(|style| style.style_id == "Heading1")
            .expect("Heading1");
        assert_eq!(heading.name.as_deref(), Some("heading 1"));
        assert_eq!(heading.based_on.as_deref(), Some("Normal"));
        assert!(styles
            .iter()
            .any(|style| style.kind == "character" && style.style_id == "Strong"));

        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Styles {
                kind: Some(HeadlessStyleKindFilter::Character),
            }],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::Styles { styles } = &response.results[0] else {
            panic!("expected styles result");
        };
        assert!(styles.iter().all(|style| style.kind == "character"));
    }

    /// `set_page_*` が書いた版面を、そのまま読み返せること。
    #[test]
    fn query_page_setup_reports_what_the_page_commands_write() {
        let mut word_doc = sample_document("body");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::SetPageSize {
                    portrait_width: 595.3,
                    portrait_height: 841.9,
                },
                HeadlessCommand::SetPageMargins {
                    top: Some(72.0),
                    bottom: Some(72.0),
                    left: Some(90.0),
                    right: Some(90.0),
                    header: None,
                    footer: None,
                    gutter: None,
                },
                HeadlessCommand::SetPageColumns {
                    count: 3,
                    space: 24.0,
                    separator: true,
                },
                HeadlessCommand::SetPageBackgroundColor {
                    color: Some("#FFEEDD".to_string()),
                },
            ],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");
        word_doc.document_mut().expect("document").watermark_text = Some("社外秘".to_string());

        let query = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::PageSetup],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &query).expect("query succeeds");
        let HeadlessQueryResult::PageSetup { page_setup } = &response.results[0] else {
            panic!("expected page setup result");
        };

        assert_eq!(page_setup.width, Some(595.3));
        assert_eq!(page_setup.height, Some(841.9));
        assert_eq!(page_setup.margin_left, Some(90.0));
        assert_eq!(page_setup.margin_top, Some(72.0));
        assert_eq!(page_setup.column_count, Some(3));
        assert_eq!(page_setup.column_space, Some(24.0));
        assert_eq!(page_setup.column_equal_width, Some(true));
        assert_eq!(page_setup.column_separator, Some(true));
        assert_eq!(page_setup.background_color.as_deref(), Some("#FFEEDD"));
        assert_eq!(page_setup.watermark_text.as_deref(), Some("社外秘"));
    }

    #[test]
    fn set_watermark_requires_explicit_consistent_clear_intent() {
        for command in [
            HeadlessCommand::SetWatermark {
                clear: true,
                text: Some("draft".to_string()),
            },
            HeadlessCommand::SetWatermark {
                clear: false,
                text: None,
            },
        ] {
            let mut word_doc = sample_document("body");
            let request = HeadlessEditRequest {
                commands: vec![command],
            };
            let error = apply_headless_commands_to_document(&mut word_doc, &request)
                .expect_err("reject inconsistent watermark intent");
            assert!(error.to_string().contains("set_watermark"), "{error}");
        }
    }

    /// ヘッダー/フッターの現在値を読めること。書き戻せる形で返す。
    #[test]
    fn query_header_footer_text_round_trips_the_editing_shape() {
        let mut word_doc = sample_document("body");
        word_doc.header_lines = Some(vec![crate::word_document::HeaderLine {
            left: Some("秘密保持契約".to_string()),
            center: None,
            right: Some("Draft".to_string()),
            ..Default::default()
        }]);

        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::HeaderFooterText],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::HeaderFooterText { stories } = &response.results[0] else {
            panic!("expected header/footer result");
        };

        assert_eq!(stories.len(), 1, "only the stories that exist are reported");
        assert_eq!(stories[0].role, "header");
        assert_eq!(stories[0].variant, "default");
        assert_eq!(stories[0].lines[0].paragraph, 0);
        assert_eq!(stories[0].lines[0].left.as_deref(), Some("秘密保持契約"));
        // Tab-joined zones: the exact string set_header_paragraph_text takes.
        assert_eq!(stories[0].lines[0].text, "秘密保持契約\t\tDraft");
    }

    #[test]
    fn query_comments_returns_metadata_and_anchor() {
        let word_doc = sample_comment_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Comments {
                start_comment: 0,
                max_comments: 10,
                author: None,
                resolved: None,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::Comments {
            comments,
            has_more,
            total_comments,
            ..
        } = &response.results[0]
        else {
            panic!("expected comments result")
        };

        assert!(!has_more);
        assert_eq!(*total_comments, 1);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "1");
        assert_eq!(comments[0].author.as_deref(), Some("LLM"));
        assert_eq!(comments[0].anchors.len(), 1);
        assert_eq!(comments[0].anchors[0].paragraph, 0);
        assert_eq!(comments[0].anchors[0].start, 0);
        assert_eq!(comments[0].anchors[0].end, 5);
    }

    #[test]
    fn query_revisions_returns_revision_spans() {
        let word_doc = sample_revision_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Revisions {
                start_revision: 0,
                max_revisions: 10,
                kind: None,
                author: None,
                date_from: None,
                date_to: None,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::Revisions {
            revisions,
            total_revisions,
            ..
        } = &response.results[0]
        else {
            panic!("expected revisions result")
        };
        assert_eq!(*total_revisions, 2);
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].kind, "inserted");
        assert_eq!(revisions[0].text, "Hello ");
        assert_eq!(revisions[1].kind, "deleted");
        assert_eq!(revisions[1].text, "World");
    }

    #[test]
    fn query_revisions_reports_move_kinds() {
        // `<w:moveFrom>` / `<w:moveTo>` runs surface as distinct
        // `movedFrom` / `movedTo` revision kinds (not folded into
        // inserted/deleted). ECMA-376 Part 1 §17.13.5.20/25.
        let make_run = |text: &str, kind: RevisionKind, id: &str| Run {
            text: text.to_string(),
            properties: None,
            image: None,
            text_box: None,
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            revision_node_id: None,
            comment_reference_ids: Vec::new(),
            footnote_reference_ids: Vec::new(),
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: Some(RevisionMark {
                kind,
                id: Some(id.to_string()),
                author: Some("Reviewer".to_string()),
                date: Some("2026-05-30T00:00:00Z".to_string()),
                date_utc: None,
                insertion: None,
            }),
        };
        let word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![
                        make_run("source", RevisionKind::MovedFrom, "3"),
                        make_run("dest", RevisionKind::MovedTo, "4"),
                    ],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        });
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Revisions {
                start_revision: 0,
                max_revisions: 10,
                kind: None,
                author: None,
                date_from: None,
                date_to: None,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::Revisions { revisions, .. } = &response.results[0] else {
            panic!("expected revisions result")
        };
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].kind, "movedFrom");
        assert_eq!(revisions[0].text, "source");
        assert_eq!(revisions[1].kind, "movedTo");
        assert_eq!(revisions[1].text, "dest");
    }

    #[test]
    fn query_revisions_filters_by_kind_author_and_date() {
        let word_doc = sample_revision_filter_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Revisions {
                start_revision: 0,
                max_revisions: 10,
                kind: Some(HeadlessRevisionKindFilter::Inserted),
                author: Some("Alice".to_string()),
                date_from: Some("2026-02-25T00:00:00Z".to_string()),
                date_to: Some("2026-02-26T23:59:59Z".to_string()),
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::Revisions {
            revisions,
            total_revisions,
            ..
        } = &response.results[0]
        else {
            panic!("expected revisions result")
        };
        assert_eq!(*total_revisions, 1);
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].text, "C");
        assert_eq!(revisions[0].author.as_deref(), Some("Alice"));
        assert_eq!(revisions[0].kind, "inserted");
    }

    #[test]
    fn query_comment_threads_returns_nested_replies() {
        let word_doc = sample_threaded_comment_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::CommentThreads {
                start_thread: 0,
                max_threads: 20,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::CommentThreads {
            threads,
            total_threads,
            ..
        } = &response.results[0]
        else {
            panic!("expected comment_threads result")
        };

        assert_eq!(*total_threads, 1);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comment.id, "1");
        assert_eq!(threads[0].replies.len(), 1);
        assert_eq!(threads[0].replies[0].comment.id, "2");
        assert_eq!(
            threads[0].replies[0].comment.reply_to_comment_id.as_deref(),
            Some("1")
        );
    }

    #[test]
    fn query_document_stats_returns_counts() {
        let word_doc = sample_comment_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentStats],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::DocumentStats { stats } = &response.results[0] else {
            panic!("expected document_stats result")
        };
        assert_eq!(stats.paragraph_count, 1);
        assert_eq!(stats.table_count, 0);
        assert_eq!(stats.comment_count, 1);
        assert_eq!(stats.revision_count, 0);
    }

    #[test]
    fn headless_document_protection_command_and_query_share_the_ooxml_state() {
        let mut word_doc = sample_document("restricted");
        word_doc.parts.insert(
            "word/settings.xml".to_string(),
            DocumentPart {
                path: "word/settings.xml".to_string(),
                relationships: Vec::new(),
                content: r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/></w:settings>"#.to_string(),
                parsed_document: None,
            },
        );
        let expected = HeadlessDocumentProtection {
            edit: "readOnly".to_string(),
            enforced: true,
            hash: None,
            salt: None,
            spin_count: None,
            algorithm_sid: None,
        };
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetDocumentProtection {
                    protection: Some(expected.clone()),
                }],
            },
        )
        .expect("protection command succeeds");

        let response = execute_headless_queries_on_document(
            &word_doc,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::DocumentProtection],
            },
        )
        .expect("protection query succeeds");
        assert_eq!(
            response.results,
            vec![HeadlessQueryResult::DocumentProtection {
                protection: Some(expected),
            }]
        );
    }

    #[test]
    fn query_document_outline_returns_heading_entries() {
        let word_doc = sample_outline_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentOutline {
                start_heading: 0,
                max_headings: 20,
                max_level: None,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::DocumentOutline {
            headings,
            total_headings,
            has_more,
            ..
        } = &response.results[0]
        else {
            panic!("expected document_outline result")
        };
        assert!(!has_more);
        assert_eq!(*total_headings, 3);
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0].paragraph, 0);
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[1].paragraph, 1);
        assert_eq!(headings[1].level, 2);
        assert_eq!(headings[2].paragraph, 2);
        assert_eq!(headings[2].level, 2);
    }

    /// 契約書の「第N条」を見出しとして推定できること。半角/全角の算用数字、
    /// 漢数字、枝番、後続の全角スペースや括弧書きをすべて受け付ける。
    #[test]
    fn parse_heading_level_from_text_detects_articles() {
        for heading in [
            "第1条",
            "第１条",
            "第一条",
            "第2条の2",
            "第２条の２",
            "第１条　（秘密情報）",
            "第1条(目的)",
            "第十二条　秘密保持",
            "第2条の2　（例外）",
        ] {
            assert_eq!(
                parse_heading_level_from_text(heading),
                Some(6),
                "{heading} should be detected as an article heading"
            );
        }
    }

    /// 「第N条」に見える本文（条を引用する平文）を見出しにしないこと。
    #[test]
    fn parse_heading_level_from_text_rejects_article_references_in_body_text() {
        for body in [
            "第3条の規定にかかわらず、甲は…",
            "第2条第2項の第三者が相手方の秘密情報等を開示した場合",
            "本契約第1条に定める秘密情報",
            "第条",
        ] {
            assert_eq!(
                parse_heading_level_from_text(body),
                None,
                "{body} should not be detected as a heading"
            );
        }
    }

    /// 章/節などの既存レベル割り当ては変えない。とくに `第1条（目的）` が、
    /// 「先頭付近に『目』がある」という緩い判定で「目」レベルに落ちないこと。
    #[test]
    fn parse_heading_level_from_text_keeps_existing_hierarchy() {
        assert_eq!(parse_heading_level_from_text("第1編　総則"), Some(1));
        assert_eq!(parse_heading_level_from_text("第1章 総則"), Some(2));
        assert_eq!(parse_heading_level_from_text("第2節　定義"), Some(3));
        assert_eq!(parse_heading_level_from_text("第3款　雑則"), Some(4));
        assert_eq!(parse_heading_level_from_text("第4目　細目"), Some(5));
        assert_eq!(parse_heading_level_from_text("附則"), Some(2));
        assert_eq!(parse_heading_level_from_text("第1条（目的）"), Some(6));
    }

    /// 実務上の契約書は「第N条」を本文ランではなく `w:numPr` の採番
    /// (`lvlText`) で持つ。見出し推定は採番ラベルを含めて判定する必要がある。
    #[test]
    fn query_document_outline_detects_articles_from_numbering_labels() {
        let word_doc = sample_article_numbering_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentOutline {
                start_heading: 0,
                max_headings: 20,
                max_level: None,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::DocumentOutline {
            headings,
            total_headings,
            ..
        } = &response.results[0]
        else {
            panic!("expected document_outline result")
        };

        assert_eq!(*total_headings, 2);
        assert_eq!(headings[0].paragraph, 0);
        assert_eq!(headings[0].level, 6);
        assert_eq!(headings[0].text, "（秘密情報）");
        assert_eq!(headings[0].numbering_text.as_deref(), Some("第１条　"));
        assert_eq!(headings[1].paragraph, 1);
        assert_eq!(headings[1].numbering_text.as_deref(), Some("第２条　"));
    }

    /// `insert_after_heading` は `document_outline` と同じ検出を使うため、
    /// 採番由来の「第N条　（…）」でも見出しを指定できる。
    #[test]
    fn insert_after_heading_resolves_numbering_prefixed_article_heading() {
        let mut word_doc = sample_article_numbering_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertAfterHeading {
                heading: "第１条　（秘密情報）".to_string(),
                level: None,
                occurrence: 0,
                text: "追加本文".to_string(),
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");
        assert_eq!(paragraph_text(&word_doc, 1), "追加本文");
    }

    fn load_public_fixture(name: &str) -> WordDocument {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures")
                .join(name),
        )
        .expect("read fixture");
        executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load fixture")
    }

    /// Fixtures whose headings the navigation pane finds only through the
    /// resolved `w:outlineLvl`: a localized heading style id (`1`) in
    /// numbering-restart.docx and toc.docx, a direct `w:outlineLvl` on a
    /// Normal paragraph in toc-direct-outline-levels.docx.
    const OUTLINE_LEVEL_HEADING_FIXTURES: [&str; 3] = [
        "numbering-restart.docx",
        "toc-direct-outline-levels.docx",
        "toc.docx",
    ];

    #[test]
    fn document_outline_lists_every_navigation_pane_heading() {
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentOutline {
                start_heading: 0,
                max_headings: 1000,
                max_level: None,
            }],
        };
        for fixture in OUTLINE_LEVEL_HEADING_FIXTURES {
            let word_doc = load_public_fixture(fixture);
            let navigation = word_doc.navigation_headings();
            assert!(!navigation.is_empty(), "{fixture} has navigation headings");

            let response =
                execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
            let HeadlessQueryResult::DocumentOutline { headings, .. } = &response.results[0] else {
                panic!("expected document_outline result")
            };
            for heading in navigation {
                assert!(
                    headings
                        .iter()
                        .any(|item| item.paragraph == heading.paragraph_index
                            && item.level == heading.level
                            && item.text == heading.text.trim()),
                    "{fixture}: outline is missing navigation heading {heading:?}: {headings:?}",
                );
            }
        }
    }

    #[test]
    fn heading_inserts_resolve_navigation_pane_headings_in_fixtures() {
        // (fixture, heading text, level, occurrence, insert after)
        let cases = [
            ("numbering-restart.docx", "Article1", 1, 0, true),
            (
                "toc-direct-outline-levels.docx",
                "Direct outline paragraph",
                2,
                0,
                false,
            ),
            // toc.docx has two level-1 "Test" headings; target the second.
            ("toc.docx", "Test", 1, 1, true),
        ];
        assert_eq!(cases.len(), OUTLINE_LEVEL_HEADING_FIXTURES.len());
        for (fixture, heading, level, occurrence, after) in cases {
            let mut word_doc = load_public_fixture(fixture);
            let target = word_doc
                .navigation_headings()
                .into_iter()
                .filter(|item| item.text.trim() == heading && item.level == level)
                .nth(occurrence)
                .unwrap_or_else(|| panic!("{fixture}: navigation pane lists {heading:?}"))
                .paragraph_index;
            let inserted = format!("inserted near {heading}");
            let command = if after {
                HeadlessCommand::InsertAfterHeading {
                    heading: heading.to_string(),
                    text: inserted.clone(),
                    level: Some(level),
                    occurrence,
                }
            } else {
                HeadlessCommand::InsertBeforeHeading {
                    heading: heading.to_string(),
                    text: inserted.clone(),
                    level: Some(level),
                    occurrence,
                }
            };

            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
            .unwrap_or_else(|error| panic!("{fixture}: {heading:?} resolves: {error:?}"));

            let (inserted_at, heading_at) = if after {
                (target + 1, target)
            } else {
                (target, target + 1)
            };
            assert_eq!(
                paragraph_text(&word_doc, inserted_at),
                inserted,
                "{fixture}"
            );
            assert_eq!(paragraph_text(&word_doc, heading_at), heading, "{fixture}");
        }
    }

    /// A built-in heading style id only stands in for a missing outline level
    /// (documents built without a styles part). A resolved level wins: a
    /// direct `w:outlineLvl` makes a Normal paragraph a heading, and Word's
    /// body text level (9) takes a Heading-styled paragraph out of the outline.
    #[test]
    fn document_outline_prefers_resolved_outline_level_over_style_id() {
        let paragraph = |text: &str, style: Option<&str>, outline_level: Option<u8>| {
            let mut paragraph = paragraph_from_text(text);
            paragraph.properties = Some(ParagraphProperties {
                style: style.map(|style| style.to_string().into()),
                outline_level,
                ..ParagraphProperties::default()
            });
            paragraph
        };
        let word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph("Style only", Some("Heading2"), None)),
                    BodyElement::from_paragraph(paragraph("Direct level", Some("Normal"), Some(2))),
                    BodyElement::from_paragraph(paragraph(
                        "Demoted heading",
                        Some("Heading1"),
                        Some(9),
                    )),
                ],
            }),
            ..Document::default()
        });

        assert_eq!(
            outline_snapshots(&word_doc)
                .into_iter()
                .map(|item| (item.text, item.level))
                .collect::<Vec<_>>(),
            vec![
                ("Style only".to_string(), 2),
                ("Direct level".to_string(), 3),
            ],
        );
    }

    #[test]
    fn query_document_outline_supports_max_level_and_pagination() {
        let word_doc = sample_outline_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::DocumentOutline {
                start_heading: 1,
                max_headings: 1,
                max_level: Some(2),
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::DocumentOutline {
            headings,
            total_headings,
            has_more,
            ..
        } = &response.results[0]
        else {
            panic!("expected document_outline result")
        };
        assert_eq!(*total_headings, 3);
        assert!(has_more);
        assert_eq!(headings.len(), 1);
        assert_eq!(headings[0].text, "Sub");
    }

    #[test]
    fn query_paragraph_text_includes_numbering_text() {
        let word_doc = sample_numbering_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::ParagraphText { paragraph: 1 }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::ParagraphText {
            text: Some(text),
            numbering_text,
            ..
        } = &response.results[0]
        else {
            panic!("expected paragraph_text result")
        };
        assert_eq!(text, "Second");
        assert_eq!(numbering_text.as_deref(), Some("2."));
    }

    #[test]
    fn query_paragraph_slice_includes_numbering_text() {
        let word_doc = sample_numbering_document();
        let request = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::ParagraphSlice {
                start_paragraph: 0,
                max_paragraphs: 3,
            }],
        };

        let response =
            execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
        let HeadlessQueryResult::ParagraphSlice { paragraphs, .. } = &response.results[0] else {
            panic!("expected paragraph_slice result")
        };

        assert_eq!(paragraphs.len(), 3);
        assert_eq!(paragraphs[0].numbering_text.as_deref(), Some("1."));
        assert_eq!(paragraphs[1].numbering_text.as_deref(), Some("2."));
        assert_eq!(paragraphs[2].numbering_text, None);
    }

    #[test]
    fn table_resize_commands_update_table_dimensions() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::ResizeTableColumnBetween {
                    table_order_index: 0,
                    left_column_index: 0,
                    new_left_width: 30.0,
                    new_right_width: 50.0,
                },
                HeadlessCommand::ResizeTableRowBetween {
                    table_order_index: 0,
                    row_index: 0,
                    new_top_height: 22.0,
                    new_bottom_height: 18.0,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.columns[0].width, Some(30.0));
        assert_eq!(table.columns[1].width, Some(50.0));
        assert_eq!(
            table.table_layout,
            Some(TableLayoutMode::Fixed),
            "manual column widths must not be redistributed by autofit on repaint"
        );
        // Resize commands receive visual border-to-border heights, whereas an
        // exact OOXML row value excludes Word's effective bottom cell margin.
        // The sample has no explicit margin and no default table style, so
        // Word's built-in Normal Table margin (0pt) applies and the stored
        // value equals the visual height.
        assert_eq!(table.rows[0].height, Some(22.0));
        assert_eq!(table.rows[1].height, Some(18.0));
    }

    #[test]
    fn table_resize_commands_clamp_collapsing_dimensions() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::ResizeTableColumnBetween {
                    table_order_index: 0,
                    left_column_index: 0,
                    new_left_width: 0.0,
                    new_right_width: -4.0,
                },
                HeadlessCommand::ResizeTableRowBetween {
                    table_order_index: 0,
                    row_index: 0,
                    new_top_height: 0.0,
                    new_bottom_height: -3.0,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.columns[0].width, Some(9.0));
        assert_eq!(table.columns[1].width, Some(9.0));
        // The visual minimum remains 9pt. Stored exact heights subtract the
        // effective bottom margin, which is Word's 0pt default here, so
        // rendering reconstructs that same 9pt.
        assert_eq!(table.rows[0].height, Some(9.0));
        assert_eq!(table.rows[1].height, Some(9.0));
    }

    #[test]
    fn table_resize_commands_reject_non_finite_dimensions() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ResizeTableColumnRightEdge {
                table_order_index: 0,
                column_index: 0,
                new_width: f32::INFINITY,
            }],
        };

        let err = apply_headless_commands_to_document(&mut word_doc, &request)
            .expect_err("non-finite column width should fail");
        assert!(err
            .to_string()
            .contains("table column width must be finite"));

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.columns[0].width, Some(40.0));

        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::ResizeTableRowBottomEdge {
                table_order_index: 0,
                row_index: 0,
                new_height: f32::NAN,
            }],
        };
        let err = apply_headless_commands_to_document(&mut word_doc, &request)
            .expect_err("non-finite row height should fail");
        assert!(err.to_string().contains("table row height must be finite"));

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows[0].height, None);
    }

    #[test]
    fn table_resize_commands_reject_out_of_range_columns_without_growing_the_grid() {
        for command in [
            HeadlessCommand::ResizeTableColumnBetween {
                table_order_index: 0,
                left_column_index: 1,
                new_left_width: 30.0,
                new_right_width: 50.0,
            },
            HeadlessCommand::ResizeTableColumnRightEdge {
                table_order_index: 0,
                column_index: 2,
                new_width: 30.0,
            },
        ] {
            let mut word_doc = sample_table_document();
            let before = word_doc.table_by_order(0).expect("table exists").clone();
            let request = HeadlessEditRequest {
                commands: vec![command],
            };

            let err = apply_headless_commands_to_document(&mut word_doc, &request)
                .expect_err("an out-of-range resize must fail");
            assert!(err.to_string().contains("out of range"));
            let after = word_doc.table_by_order(0).expect("table exists");
            assert_eq!(after.columns.len(), before.columns.len());
            assert_eq!(after.columns[0].width, before.columns[0].width);
            assert_eq!(after.columns[1].width, before.columns[1].width);
            assert_eq!(after.rows.len(), before.rows.len());
            assert!(after
                .rows
                .iter()
                .zip(&before.rows)
                .all(|(after, before)| after.cells.len() == before.cells.len()));
        }
    }

    #[test]
    fn table_insert_commands_reject_out_of_range_indices_without_mutation() {
        for command in [
            HeadlessCommand::InsertTableRow {
                table_order_index: 0,
                row_index: 3,
                template_row: None,
            },
            HeadlessCommand::InsertTableColumn {
                table_order_index: 0,
                column_index: 3,
                width: Some(20.0),
                template_column: None,
            },
        ] {
            let mut word_doc = sample_table_document();
            let before = word_doc.table_by_order(0).expect("table exists").clone();
            let request = HeadlessEditRequest {
                commands: vec![command],
            };

            let err = apply_headless_commands_to_document(&mut word_doc, &request)
                .expect_err("an out-of-range insertion must fail");
            assert!(err.to_string().contains("out of range"));
            let after = word_doc.table_by_order(0).expect("table exists");
            assert_eq!(after.rows.len(), before.rows.len());
            assert_eq!(after.columns.len(), before.columns.len());
            assert_eq!(
                after
                    .columns
                    .iter()
                    .map(|column| column.width)
                    .collect::<Vec<_>>(),
                before
                    .columns
                    .iter()
                    .map(|column| column.width)
                    .collect::<Vec<_>>(),
            );
            assert!(after.rows.iter().zip(&before.rows).all(|(after, before)| {
                after.cells.len() == before.cells.len()
                    && after
                        .cells
                        .iter()
                        .zip(&before.cells)
                        .all(|(after, before)| {
                            after.span == before.span
                                && after.vertical_merge == before.vertical_merge
                        })
            }));
        }
    }

    #[test]
    fn insert_table_command_adds_table_after_body_paragraph() {
        let mut word_doc = sample_document("hello");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertTable {
                after_paragraph: 0,
                rows: 2,
                columns: 3,
                column_widths: vec![20.0, 30.0, 40.0],
                caret_offset: None,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.columns.len(), 3);
        assert_eq!(table.columns[0].width, Some(20.0));
        assert_eq!(table.columns[1].width, Some(30.0));
        assert_eq!(table.columns[2].width, Some(40.0));
    }

    #[test]
    fn set_table_width_and_column_widths_commands_update_table_model() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::SetTableWidth {
                    table_order_index: 0,
                    width: "420pt".to_string(),
                },
                HeadlessCommand::SetTableColumnWidths {
                    table_order_index: 0,
                    widths: vec![25.0, 55.0],
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(
            table.style.get("width").map(String::as_str),
            Some("420.00pt")
        );
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[0].width, Some(25.0));
        assert_eq!(table.columns[1].width, Some(55.0));
    }

    #[test]
    fn table_fit_commands_author_word_layout_and_width_modes() {
        let mut word_doc = sample_table_document();
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetTableFit {
                    table_order_index: 0,
                    mode: HeadlessTableFitMode::Window,
                }],
            },
        )
        .expect("fit to window");
        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.table_layout, Some(TableLayoutMode::Autofit));
        assert_eq!(
            table.style.get("width").map(String::as_str),
            Some("100.00%")
        );

        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetTableFit {
                    table_order_index: 0,
                    mode: HeadlessTableFitMode::Fixed,
                }],
            },
        )
        .expect("fixed width");
        assert_eq!(
            word_doc
                .table_by_order(0)
                .expect("table exists")
                .table_layout,
            Some(TableLayoutMode::Fixed)
        );
    }

    #[test]
    fn table_look_command_preserves_defaults_and_updates_conditional_regions() {
        let mut word_doc = sample_table_document();
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetTableLook {
                    table_order_index: 0,
                    first_row: None,
                    banded_rows: Some(false),
                    last_row: None,
                    first_column: None,
                    last_column: None,
                    banded_columns: None,
                }],
            },
        )
        .expect("table look");

        let table = word_doc.table_by_order(0).expect("table exists");
        // An omitted tblLook is Word's 0x04A0; only the banding switch changes.
        assert_eq!(table.look.first_row, Some(true));
        assert_eq!(table.look.first_column, Some(true));
        assert_eq!(table.look.no_h_band, Some(true));
        assert_eq!(table.look.no_v_band, Some(true));
        assert_eq!(table.class_name.as_deref(), Some("06A0"));
    }

    #[test]
    fn table_text_wrapping_command_switches_floating_model_state() {
        let mut word_doc = sample_table_document();
        for (wrapping, expected) in [
            (HeadlessTableTextWrapping::Around, Some("floating")),
            (HeadlessTableTextWrapping::None, None),
        ] {
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![HeadlessCommand::SetTableTextWrapping {
                        table_order_index: 0,
                        wrapping,
                    }],
                },
            )
            .expect("table text wrapping");
            assert_eq!(
                word_doc
                    .table_by_order(0)
                    .expect("table exists")
                    .style
                    .get("table-position")
                    .map(String::as_str),
                expected
            );
        }
    }

    #[test]
    fn delete_table_cell_shift_left_removes_only_the_target_cell() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::SetTableCellText {
                    table_order_index: 0,
                    row: 0,
                    column: 0,
                    text: "A".to_string(),
                },
                HeadlessCommand::SetTableCellText {
                    table_order_index: 0,
                    row: 0,
                    column: 1,
                    text: "B".to_string(),
                },
                HeadlessCommand::DeleteTableCell {
                    table_order_index: 0,
                    row: 0,
                    column: 0,
                    direction: HeadlessTableCellDeleteDirection::ShiftLeft,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.columns.len(), 2, "cell deletion keeps the table grid");
        assert_eq!(table.rows[0].cells.len(), 1);
        assert_eq!(table.rows[0].grid_after, Some(1));
        assert_eq!(table.rows[1].cells.len(), 2);
        let BodyElement::Paragraph(paragraph) = &table.rows[0].cells[0].blocks[0] else {
            panic!("cell must contain paragraph");
        };
        assert_eq!(
            paragraph
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "B"
        );
    }

    #[test]
    fn delete_table_cell_shift_up_pulls_the_column_and_shortens_the_last_row() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::SetTableCellText {
                    table_order_index: 0,
                    row: 1,
                    column: 0,
                    text: "C".to_string(),
                },
                HeadlessCommand::SetTableCellText {
                    table_order_index: 0,
                    row: 1,
                    column: 1,
                    text: "D".to_string(),
                },
                HeadlessCommand::DeleteTableCell {
                    table_order_index: 0,
                    row: 0,
                    column: 0,
                    direction: HeadlessTableCellDeleteDirection::ShiftUp,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.columns.len(), 2, "cell deletion keeps the table grid");
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(table.rows[1].cells.len(), 1);
        assert_eq!(table.rows[1].grid_after, Some(1));
        let texts = table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .map(|cell| {
                let BodyElement::Paragraph(paragraph) = &cell.blocks[0] else {
                    panic!("cell must contain paragraph");
                };
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(texts, ["C", "B1", "D"]);
    }

    fn table_cell_texts(word_doc: &WordDocument) -> Vec<Vec<String>> {
        let table = word_doc.table_by_order(0).expect("table exists");
        table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        let BodyElement::Paragraph(paragraph) = &cell.blocks[0] else {
                            panic!("cell must contain paragraph");
                        };
                        paragraph
                            .runs
                            .iter()
                            .map(|run| run.text.as_str())
                            .collect::<String>()
                    })
                    .collect()
            })
            .collect()
    }

    fn labelled_table_cells() -> Vec<HeadlessCommand> {
        [(0, 0, "A"), (0, 1, "B"), (1, 0, "C"), (1, 1, "D")]
            .into_iter()
            .map(|(row, column, text)| HeadlessCommand::SetTableCellText {
                table_order_index: 0,
                row,
                column,
                text: text.to_string(),
            })
            .collect()
    }

    #[test]
    fn insert_table_cell_shift_right_grows_only_the_target_row() {
        let mut word_doc = sample_table_document();
        let mut commands = labelled_table_cells();
        commands.push(HeadlessCommand::InsertTableCell {
            table_order_index: 0,
            row: 0,
            column: 0,
            direction: HeadlessTableCellInsertDirection::ShiftRight,
        });
        apply_headless_commands_to_document(&mut word_doc, &HeadlessEditRequest { commands })
            .expect("apply succeeds");

        assert_eq!(
            table_cell_texts(&word_doc),
            [vec!["", "A", "B"], vec!["C", "D"]]
        );
        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(
            table.columns.len(),
            3,
            "the grid gains the pushed-out column"
        );
        assert_eq!(table.rows[0].grid_after, None);
        assert_eq!(
            table.rows[1].grid_after,
            Some(1),
            "other rows skip the new grid column"
        );
        assert_eq!(table.columns[2].width, table.columns[0].width);
    }

    #[test]
    fn insert_table_cell_shift_right_reuses_a_trailing_grid_slot() {
        let mut word_doc = sample_table_document();
        let mut commands = labelled_table_cells();
        commands.push(HeadlessCommand::DeleteTableCell {
            table_order_index: 0,
            row: 0,
            column: 1,
            direction: HeadlessTableCellDeleteDirection::ShiftLeft,
        });
        commands.push(HeadlessCommand::InsertTableCell {
            table_order_index: 0,
            row: 0,
            column: 0,
            direction: HeadlessTableCellInsertDirection::ShiftRight,
        });
        apply_headless_commands_to_document(&mut word_doc, &HeadlessEditRequest { commands })
            .expect("apply succeeds");

        assert_eq!(table_cell_texts(&word_doc), [vec!["", "A"], vec!["C", "D"]]);
        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(
            table.columns.len(),
            2,
            "the free grid slot absorbs the cell"
        );
        assert_eq!(table.rows[0].grid_after, None);
        assert_eq!(table.rows[1].grid_after, None);
    }

    #[test]
    fn insert_table_cell_shift_down_moves_the_column_into_a_new_last_row() {
        let mut word_doc = sample_table_document();
        let mut commands = labelled_table_cells();
        commands.push(HeadlessCommand::InsertTableCell {
            table_order_index: 0,
            row: 0,
            column: 1,
            direction: HeadlessTableCellInsertDirection::ShiftDown,
        });
        apply_headless_commands_to_document(&mut word_doc, &HeadlessEditRequest { commands })
            .expect("apply succeeds");

        assert_eq!(
            table_cell_texts(&word_doc),
            [vec!["A", ""], vec!["C", "B"], vec!["", "D"]]
        );
        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.columns.len(), 2, "shifting down keeps the grid");
    }

    /// Reloads `word_doc`'s export and returns its first paragraph's run sizes.
    fn exported_run_font_sizes(word_doc: &WordDocument) -> Vec<Option<f32>> {
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        run_font_sizes(&reloaded)
    }

    fn run_font_sizes(word_doc: &WordDocument) -> Vec<Option<f32>> {
        word_doc
            .paragraph(0)
            .expect("paragraph 0")
            .runs
            .iter()
            .map(|run| run.properties.as_ref().and_then(|props| props.font_size))
            .collect()
    }

    #[test]
    fn paragraph_style_reaches_the_saved_runs_and_undo_restores_them() {
        // nda.docx's title run carries a direct 12pt size. Applying Heading 1
        // gives it the heading's size on screen, as Word does when the direct
        // formatting covers the paragraph; the saved file has to agree, or a
        // reload shows the old size again.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/nda.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let original = run_font_sizes(&word_doc);
        let mut executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetParagraphStyle {
                    start_paragraph: 0,
                    end_paragraph: 0,
                    style_id: Some("Heading1".to_string()),
                }],
            },
        )
        .expect("style applies");
        let styled = run_font_sizes(&word_doc);
        assert_ne!(styled, original, "the heading changes the title's size");
        assert_eq!(exported_run_font_sizes(&word_doc), styled);

        // Typing rebuilds the run's XML; the replaced size must not return.
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 0,
                    text: "x".to_string(),
                }],
            },
        )
        .expect("typing applies");
        assert_eq!(run_font_sizes(&word_doc), styled);
        assert_eq!(exported_run_font_sizes(&word_doc), styled);
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor.undo(&mut ctx).expect("undo typing");

        let mut selection = None;
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor.undo(&mut ctx).expect("undo succeeds");
        assert_eq!(run_font_sizes(&word_doc), original);
        assert_eq!(exported_run_font_sizes(&word_doc), original);
    }

    /// Each run's size and weight as layout resolves them: the paragraph
    /// style's formatting under the run's own.
    fn run_formatting(word_doc: &WordDocument, paragraph: usize) -> Vec<(Option<f32>, bool)> {
        let paragraph = word_doc.paragraph(paragraph).expect("paragraph");
        let style = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.style.as_deref());
        let styles = word_doc.styles_part.as_ref().expect("styles");
        paragraph
            .runs
            .iter()
            .map(|run| {
                let props = styles
                    .resolve_effective_run_properties(style, run.properties.as_ref())
                    .unwrap_or_default();
                (props.font_size, props.bold.unwrap_or(false))
            })
            .collect()
    }

    fn exported_run_formatting(
        word_doc: &WordDocument,
        paragraph: usize,
    ) -> Vec<(Option<f32>, bool)> {
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        run_formatting(&reloaded, paragraph)
    }

    #[test]
    fn runs_joined_into_another_paragraph_style_take_its_formatting() {
        // nda.docx's paragraph 6 made a Heading 2, then joined to the body
        // text above it: its runs carried the heading's 14pt bold resolved
        // into them while their saved XML leaves both to the style, so the
        // screen kept the heading look and a reload showed body text.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/nda.docx"),
        )
        .expect("read fixture");
        let joins = [
            HeadlessCommand::DeleteText {
                paragraph: 6,
                offset: 0,
                direction: HeadlessDeleteDirection::Backward,
                count: 1,
            },
            HeadlessCommand::DeleteText {
                paragraph: 5,
                offset: 0,
                direction: HeadlessDeleteDirection::Forward,
                count: 0,
            },
            HeadlessCommand::ReplaceRange {
                start_paragraph: 5,
                start_offset: 0,
                end_paragraph: 6,
                end_offset: 42,
                text: String::new(),
            },
            HeadlessCommand::ReplaceRange {
                start_paragraph: 5,
                start_offset: 0,
                end_paragraph: 6,
                end_offset: 42,
                text: "、".to_string(),
            },
        ];
        for join in joins {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let body = run_formatting(&word_doc, 5);
            let mut executor = CommandExecutor::new();
            let mut apply = |word_doc: &mut WordDocument, command: HeadlessCommand| {
                apply_headless_commands_with_executor_to_document(
                    word_doc,
                    &mut executor,
                    None,
                    &HeadlessEditRequest {
                        commands: vec![command],
                    },
                )
                .expect("command applies");
            };
            apply(
                &mut word_doc,
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: 6,
                    end_paragraph: 6,
                    style_id: Some("Heading2".to_string()),
                },
            );
            let heading = run_formatting(&word_doc, 6);
            assert_ne!(heading[0], body[0], "the heading restyles the paragraph");
            // Delete forward at the end of paragraph 5.
            let join = match join {
                HeadlessCommand::DeleteText {
                    paragraph: 5,
                    direction: HeadlessDeleteDirection::Forward,
                    ..
                } => HeadlessCommand::DeleteText {
                    paragraph: 5,
                    offset: word_doc.paragraph_text_len_at(5).expect("paragraph 5"),
                    direction: HeadlessDeleteDirection::Forward,
                    count: 1,
                },
                join => join,
            };
            apply(&mut word_doc, join.clone());
            let joined = run_formatting(&word_doc, 5);
            assert!(
                !joined.contains(&heading[0]),
                "{join:?} left heading formatting in body text: {joined:?}"
            );
            assert_eq!(exported_run_formatting(&word_doc, 5), joined, "{join:?}");

            let mut selection = None;
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.undo(&mut ctx).expect("undo succeeds");
            assert_eq!(run_formatting(&word_doc, 6), heading, "{join:?}");
            assert_eq!(exported_run_formatting(&word_doc, 6), heading, "{join:?}");
        }
    }

    #[test]
    fn normal_over_a_heading_drops_what_only_the_heading_defined() {
        // nda.docx's Normal style leaves the size and keep-with-next to the
        // document defaults. Heading 1 then Normal on body text kept the
        // heading's 16pt runs and keep-with-next on screen, while the saved
        // file, holding neither, reloaded as body text.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/nda.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let keep_next = |word_doc: &WordDocument| {
            word_doc
                .paragraph(5)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.keep_next)
        };
        let body = (run_formatting(&word_doc, 5), keep_next(&word_doc));
        let mut executor = CommandExecutor::new();
        for style_id in [Some("Heading1".to_string()), None] {
            apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &HeadlessEditRequest {
                    commands: vec![HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 5,
                        end_paragraph: 5,
                        style_id,
                    }],
                },
            )
            .expect("style applies");
        }
        assert_eq!((run_formatting(&word_doc, 5), keep_next(&word_doc)), body);
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!((run_formatting(&reloaded, 5), keep_next(&reloaded)), body);
    }

    #[test]
    fn reset_paragraph_formatting_keeps_what_the_styles_give() {
        // The cells of table-complex-pagination.docx have no w:pPr of their
        // own; their 2pt spacing after comes from the table style. Ctrl+Q
        // cleared it from the paragraphs' properties, which hold what the
        // styles give as well, so the rows shrank until a reload.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table-complex-pagination.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let spacing = |word_doc: &WordDocument| {
            (0..=27)
                .map(|index| {
                    word_doc
                        .paragraph(index)
                        .and_then(|paragraph| paragraph.properties.as_ref())
                        .and_then(|props| props.spacing.clone())
                })
                .collect::<Vec<_>>()
        };
        let before = spacing(&word_doc);
        assert!(before[10]
            .as_ref()
            .and_then(|spacing| spacing.after)
            .is_some());
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::ResetParagraphFormatting {
                    start_paragraph: 0,
                    end_paragraph: 27,
                }],
            },
        )
        .expect("reset applies");
        assert_eq!(spacing(&word_doc), before);
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(spacing(&reloaded), before);
    }

    #[test]
    fn clear_formatting_keeps_the_fonts_the_styles_give() {
        // Ctrl+Space over body text and table cells cleared the runs' font
        // and size, which hold what the styles give as well: the text drew in
        // the renderer's fallback font, and a reload showed the styles' again.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table-complex-pagination.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let fonts = |word_doc: &WordDocument| {
            (0..=37)
                .flat_map(|index| {
                    word_doc
                        .paragraph(index)
                        .map(|paragraph| {
                            paragraph
                                .runs
                                .iter()
                                .map(|run| {
                                    let props = run.properties.as_ref();
                                    (
                                        props.and_then(|props| props.font_size),
                                        props.and_then(|props| props.font_family.clone()),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        };
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::ClearFormatting {
                    start_paragraph: 0,
                    start_offset: 12,
                    end_paragraph: 37,
                    end_offset: 9,
                }],
            },
        )
        .expect("clear formatting applies");
        let live = fonts(&word_doc);
        assert!(live
            .iter()
            .all(|(size, family)| size.is_some() && family.is_some()));
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(fonts(&reloaded), live);
    }

    #[test]
    fn undoing_a_list_level_step_restores_the_saved_level() {
        // Ctrl+Shift+M on a contract.docx list item steps its w:ilvl; undo put
        // the level back in the model only, so the saved file kept the
        // undone level and its numbering.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/contract.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let level = |word_doc: &WordDocument| {
            word_doc
                .paragraph(4)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.numbering.as_ref())
                .and_then(|numbering| numbering.level)
        };
        let original = level(&word_doc);
        assert_eq!(original, Some(1));
        let mut executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::AdjustParagraphIndent {
                    start_paragraph: 4,
                    end_paragraph: 4,
                    direction: jade_docx_headless_api::HeadlessIndentDirection::Decrease,
                    target: jade_docx_headless_api::HeadlessIndentTarget::Start,
                }],
            },
        )
        .expect("decrease applies");
        assert_eq!(level(&word_doc), Some(0));
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor.undo(&mut ctx).expect("undo succeeds");
        assert_eq!(level(&word_doc), original);
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(level(&reloaded), original);
    }

    #[test]
    fn a_paragraph_style_keeps_direct_paragraph_formatting() {
        // Ctrl+0 gives the title of table-complex-pagination.docx direct
        // spacing before. Heading 2 then put the heading's spacing over it on
        // screen only: the saved w:pPr kept the direct spacing, as Word does.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table-complex-pagination.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let spacing = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.spacing.clone())
        };
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::ToggleParagraphSpaceBefore {
                        start_paragraph: 0,
                        end_paragraph: 0,
                    },
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 0,
                        end_paragraph: 0,
                        style_id: Some("Heading2".to_string()),
                    },
                ],
            },
        )
        .expect("commands apply");
        let live = spacing(&word_doc);
        assert_eq!(live.as_ref().and_then(|spacing| spacing.before), Some(12.0));
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(spacing(&reloaded), live);
    }

    fn exported_part(word_doc: &WordDocument, name: &str) -> String {
        let bytes = word_doc.export_docx().expect("export succeeds");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut archive.by_name(name).expect("part"), &mut xml)
            .expect("read part");
        xml
    }

    #[test]
    fn a_style_change_takes_the_numbering_its_style_gave() {
        // numbering-by-style.docx numbers its headings through the styles'
        // w:numPr. Turning a numbered heading Normal kept its number on
        // screen — the numbering resolved into the paragraph at load stayed
        // — while the saved paragraph had none and the list renumbered on
        // reload. The other way round, and between levels, has to follow too.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/numbering-by-style.docx"),
        )
        .expect("read fixture");
        let numbering = |word_doc: &WordDocument| {
            (0..4)
                .map(|index| {
                    word_doc
                        .paragraph(index)
                        .and_then(|paragraph| paragraph.properties.as_ref())
                        .and_then(|props| props.numbering.as_ref())
                        .map(|numbering| (numbering.id.clone(), numbering.level))
                })
                .collect::<Vec<_>>()
        };
        for (paragraph, style, expected) in [
            (1, "a", None),
            (0, "3", Some((Some("1".to_string()), Some(2)))),
            (3, "a", None),
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let mut commands = vec![HeadlessCommand::SetParagraphStyle {
                start_paragraph: paragraph,
                end_paragraph: paragraph,
                style_id: Some(style.to_string()),
            }];
            if style == "a" {
                // ... and back: the style brings its numbering again.
                commands.push(HeadlessCommand::SetParagraphStyle {
                    start_paragraph: paragraph,
                    end_paragraph: paragraph,
                    style_id: Some("3".to_string()),
                });
            }
            for (step, command) in commands.into_iter().enumerate() {
                apply_headless_commands_to_document(
                    &mut word_doc,
                    &HeadlessEditRequest {
                        commands: vec![command],
                    },
                )
                .expect("style applies");
                let live = numbering(&word_doc);
                let wanted = if step == 0 {
                    expected.clone()
                } else {
                    Some((Some("1".to_string()), Some(2)))
                };
                assert_eq!(
                    live[paragraph], wanted,
                    "¶{paragraph} → {style}, step {step}"
                );
                let reloaded = executor::block_on(WordDocument::load(
                    &word_doc.export_docx().expect("export succeeds"),
                    &Options::default(),
                ))
                .expect("reload");
                assert_eq!(
                    numbering(&reloaded),
                    live,
                    "¶{paragraph} → {style}, step {step}"
                );
            }
        }
    }

    #[test]
    fn enter_into_the_next_style_draws_the_paragraph_its_file_holds() {
        // Enter at the end of a heading starts the style's w:next. The new
        // paragraph's w:pPr is a copy of the heading's with only w:pStyle
        // changed, but the model took the next style's properties alone:
        // no numbering from that style's w:numPr (第N章 appeared only on
        // reload) and none of the copied direct formatting either.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/numbering-by-style.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let end = word_doc
            .paragraph(0)
            .expect("heading")
            .runs
            .iter()
            .map(|run| run.text.len())
            .sum::<usize>();
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertParagraphBreak {
                        paragraph: 0,
                        offset: end,
                        count: 1,
                    },
                    HeadlessCommand::InsertParagraphBreak {
                        paragraph: 1,
                        offset: 0,
                        count: 1,
                    },
                ],
            },
        )
        .expect("enters apply");
        let shape = |word_doc: &WordDocument| {
            (0..3)
                .map(|index| {
                    let props = word_doc
                        .paragraph(index)
                        .and_then(|paragraph| paragraph.properties.as_ref())
                        .expect("properties");
                    format!(
                        "{:?} {:?} {:?} {:?}",
                        props.style,
                        props.numbering.as_ref().map(|n| (n.id.clone(), n.level)),
                        props.alignment,
                        props.spacing.as_ref().and_then(|spacing| spacing.before),
                    )
                })
                .collect::<Vec<_>>()
        };
        let live = shape(&word_doc);
        // Heading 2 → Heading 1 (第N章) → Heading 2 (第N条), each numbered.
        assert!(
            live[1].contains("Some(\"10\") Some((Some(\"1\"), Some(0)))"),
            "{live:?}"
        );
        assert!(
            live[2].contains("Some(\"2\") Some((Some(\"1\"), Some(1)))"),
            "{live:?}"
        );
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(shape(&reloaded), live);
    }

    #[test]
    fn enter_in_the_last_body_paragraph_keeps_the_footnotes_editable() {
        // Splitting the last body paragraph rebuilt the paragraph list up to
        // the next body paragraph, and there is none: the footnotes after it
        // fell out of the list. The next full rebuild put them back, a join's
        // paragraph count no longer added up, and the join failed after the
        // model had changed — the screen lost a paragraph the file kept.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/footnote.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let count = word_doc.paragraph_count();
        let end = word_doc
            .paragraph(0)
            .expect("body")
            .runs
            .iter()
            .map(|run| run.text.len())
            .sum::<usize>();
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: end,
                    count: 1,
                }],
            },
        )
        .expect("enter applies");
        assert_eq!(word_doc.paragraph_count(), count + 1);
        let incremental = word_doc.paragraph_count();
        word_doc.rebuild_paragraph_surfaces();
        assert_eq!(word_doc.paragraph_count(), incremental);
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::DeleteText {
                    paragraph: 1,
                    offset: 0,
                    direction: HeadlessDeleteDirection::Backward,
                    count: 1,
                }],
            },
        )
        .expect("the join applies");
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(reloaded.paragraph_count(), word_doc.paragraph_count());
        assert_eq!(word_doc.paragraph_count(), count);
    }

    #[test]
    fn undoing_an_edit_puts_the_comment_anchors_back() {
        // An edit before a comment moves its anchors; undoing the edit put
        // the runs back but left the anchors moved, so the comment was drawn
        // over other text until the document was reopened.
        use crate::commands::{CommandContext, CommandExecutor};
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/comment.docx"),
        )
        .expect("read fixture");
        let anchors = |word_doc: &WordDocument| {
            let paragraph = word_doc.paragraph(0).expect("paragraph");
            (
                paragraph
                    .comment_ranges
                    .iter()
                    .map(|range| (range.start, range.end))
                    .collect::<Vec<_>>(),
                paragraph
                    .comment_markers
                    .iter()
                    .map(|marker| marker.position)
                    .collect::<Vec<_>>(),
            )
        };
        for edit in [
            HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "、".to_string(),
            },
            HeadlessCommand::DeleteText {
                paragraph: 0,
                offset: 0,
                direction: HeadlessDeleteDirection::Forward,
                count: 1,
            },
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let loaded = anchors(&word_doc);
            let mut history = CommandExecutor::new();
            apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut history,
                None,
                &HeadlessEditRequest {
                    commands: vec![edit.clone()],
                },
            )
            .expect("edit applies");
            let edited = anchors(&word_doc);
            let mut selection = None;
            history
                .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
                .expect("undo");
            assert_eq!(anchors(&word_doc), loaded, "{edit:?} undone");
            history
                .redo(&mut CommandContext::new(&mut word_doc, &mut selection))
                .expect("redo");
            assert_eq!(anchors(&word_doc), edited, "{edit:?} redone");
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export succeeds"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(anchors(&reloaded), edited, "{edit:?} saved");
        }
    }

    #[test]
    fn undoing_a_deletion_inside_a_comment_keeps_the_text_inside_it() {
        // Deleting the rest of a comment's text leaves its start and end
        // markers side by side in the XML. Undo re-synced the restored runs,
        // which could not tell which side of the end marker they belonged on:
        // the saved comment came back empty while the screen showed it whole.
        use crate::commands::{CommandContext, CommandExecutor};
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/comment.docx"),
        )
        .expect("read fixture");
        let anchors = |word_doc: &WordDocument| {
            let paragraph = word_doc.paragraph(0).expect("paragraph");
            (
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.clone())
                    .collect::<String>(),
                paragraph
                    .comment_ranges
                    .iter()
                    .map(|range| (range.id.clone(), range.start, range.end))
                    .collect::<Vec<_>>(),
            )
        };
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let mut history = CommandExecutor::new();
        // "Hello comment": take "comm", then the "ent" that is left.
        for (start, end) in [(6, 10), (6, 9)] {
            apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut history,
                None,
                &HeadlessEditRequest {
                    commands: vec![HeadlessCommand::ReplaceRange {
                        start_paragraph: 0,
                        start_offset: start,
                        end_paragraph: 0,
                        end_offset: end,
                        text: String::new(),
                    }],
                },
            )
            .expect("deletion applies");
        }
        let mut selection = None;
        history
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo");
        let live = anchors(&word_doc);
        assert_eq!(live.0, "Hello ent");
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(anchors(&reloaded), live);
    }

    #[test]
    fn formatting_beside_a_comment_keeps_the_comment_where_it_was() {
        // comment.docx: "Hello comment", both comments on "comment". Formatting
        // part of a run splits it, and the new piece was written after the
        // comment markers that followed the run: highlighting "o " moved the
        // comments' start in front of the space, highlighting "nt" moved one
        // comment's end in front of it — on reload only.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/comment.docx"),
        )
        .expect("read fixture");
        let anchors = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .expect("paragraph")
                .comment_ranges
                .iter()
                .map(|range| (range.id.clone(), range.start, range.end))
                .collect::<Vec<_>>()
        };
        let highlight = |start: usize, end: usize| HeadlessCommand::SetHighlightColor {
            start_paragraph: 0,
            start_offset: start,
            end_paragraph: 0,
            end_offset: end,
            color: "yellow".to_string(),
        };
        for commands in [
            vec![highlight(4, 7)],
            vec![highlight(11, 13)],
            vec![highlight(7, 9)],
            // Text typed over the comment's start: the comment follows the
            // marker the text went in front of, on screen as after a reload.
            vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 2,
                end_paragraph: 0,
                end_offset: 8,
                text: "あい".to_string(),
            }],
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: commands.clone(),
                },
            )
            .expect("edit applies");
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export succeeds"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(anchors(&reloaded), anchors(&word_doc), "{commands:?}");
        }
    }

    #[test]
    fn typing_after_enter_at_the_start_keeps_the_hyperlink() {
        // Enter at the start of a paragraph holding a w:hyperlink rebuilds
        // the link's wrapper; the wrapper renumbered the run inside, so the
        // next edit found no run in it and dropped it. The saved paragraph
        // kept the link's text and Hyperlink style but lost the link.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/hyperlink.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertParagraphBreak {
                        paragraph: 0,
                        offset: 0,
                        count: 1,
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 1,
                        offset: 0,
                        text: "あいう".to_string(),
                    },
                ],
            },
        )
        .expect("edits apply");
        let exported = word_doc.export_docx().expect("export");
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("exported zip");
        let mut xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/document.xml").expect("document part"),
            &mut xml,
        )
        .expect("read document part");
        let link = xml
            .find("<w:hyperlink")
            .expect("the hyperlink survives the save");
        let text = xml.find(">example.com<").expect("the link text is saved");
        let close = xml.find("</w:hyperlink>").expect("the hyperlink is closed");
        assert!(
            link < text && text < close,
            "the link text sits inside it: {xml}"
        );
    }

    #[test]
    fn enter_and_backspace_inside_a_text_box_are_saved() {
        // Enter inside a text box split its paragraph on screen only: the
        // saved box, in its DrawingML and VML copies alike, still held one
        // paragraph, so reopening the file put the text back together.
        use crate::commands::{CommandContext, CommandExecutor};
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/textbox.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let texts = |word_doc: &WordDocument| {
            (0..word_doc.paragraph_count())
                .map(|index| {
                    word_doc
                        .paragraph(index)
                        .map(|paragraph| {
                            paragraph.runs.iter().map(|run| run.text.as_str()).collect()
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<String>>()
        };
        let boxes = |word_doc: &WordDocument| {
            let exported = word_doc.export_docx().expect("export");
            let mut archive =
                zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("exported zip");
            let mut xml = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("word/document.xml").expect("document part"),
                &mut xml,
            )
            .expect("read document part");
            xml.split("<w:txbxContent>")
                .skip(1)
                .map(|content| {
                    let content = content.split("</w:txbxContent>").next().unwrap_or_default();
                    content
                        .split("</w:p>")
                        .filter(|paragraph| paragraph.contains("<w:p"))
                        .map(|paragraph| {
                            paragraph
                                .split("<w:t")
                                .skip(1)
                                .filter_map(|text| text.split_once('>'))
                                .map(|(_, rest)| rest.split('<').next().unwrap_or_default())
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(texts(&word_doc), ["Hello", "Hello", "World"]);

        let mut command_executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut command_executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertParagraphBreak {
                    paragraph: 1,
                    offset: 2,
                    count: 1,
                }],
            },
        )
        .expect("break applies");
        assert_eq!(texts(&word_doc), ["Hello", "He", "llo", "World"]);
        assert_eq!(boxes(&word_doc), [vec!["He", "llo"], vec!["He", "llo"]]);
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(texts(&reloaded), ["Hello", "He", "llo", "World"]);

        // Backspace joins them again, in the saved box as well: it kept the
        // second paragraph, and the file read "Hello", "llo".
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut command_executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::DeleteText {
                    paragraph: 2,
                    offset: 0,
                    direction: jade_docx_headless_api::HeadlessDeleteDirection::Backward,
                    count: 1,
                }],
            },
        )
        .expect("backspace applies");
        assert_eq!(texts(&word_doc), ["Hello", "Hello", "World"]);
        assert_eq!(boxes(&word_doc), [vec!["Hello"], vec!["Hello"]]);

        let mut selection = None;
        command_executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo the join");
        assert_eq!(boxes(&word_doc), [vec!["He", "llo"], vec!["He", "llo"]]);
        command_executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo the break");
        assert_eq!(texts(&word_doc), ["Hello", "Hello", "World"]);
        assert_eq!(boxes(&word_doc), [vec!["Hello"], vec!["Hello"]]);
    }

    #[test]
    fn a_style_keeps_the_spacing_a_paragraph_does_not_declare() {
        // A paragraph declaring only `w:spacing w:before` takes its space
        // after from its style. Applying the heading style kept the declared
        // spacing as a whole, so the style's space after was lost on screen
        // while a reload, which merges attribute by attribute, had it: the
        // table row holding the paragraph was 4pt shorter until reopened.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table-nested.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::ToggleParagraphSpaceBefore {
                        start_paragraph: 0,
                        end_paragraph: 0,
                    },
                    // `2` is the fixture's heading 2: 8pt before, 4pt after.
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 0,
                        end_paragraph: 0,
                        style_id: Some("2".to_string()),
                    },
                ],
            },
        )
        .expect("edits apply");
        let spacing = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.spacing.clone())
        };
        let live = spacing(&word_doc).expect("spacing");
        assert_eq!((live.before, live.after), (Some(12.0), Some(4.0)));
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(spacing(&reloaded), Some(live));
    }

    fn toc_document_xml(word_doc: &WordDocument) -> String {
        let exported = word_doc.export_docx().expect("export");
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("exported zip");
        let mut xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/document.xml").expect("document part"),
            &mut xml,
        )
        .expect("read document part");
        xml
    }

    /// `paragraph`'s `w:p` in the saved package.
    fn saved_paragraph_xml(xml: &str, paragraph: usize) -> &str {
        let body = &xml[xml.find("<w:body>").expect("body")..];
        body.split("</w:p>").nth(paragraph).expect("paragraph")
    }

    #[test]
    fn a_heading_keeps_its_style_colour_out_of_the_runs() {
        // toc.docx's heading 1 is the theme colour accent1. Applying it left
        // that colour in the runs' model properties, and the next edit to the
        // paragraph wrote it into their `w:rPr` as a fixed direct colour.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/toc.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 0,
                        end_paragraph: 0,
                        style_id: Some("1".to_string()),
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 0,
                        text: "x".to_string(),
                    },
                ],
            },
        )
        .expect("edits apply");
        let xml = toc_document_xml(&word_doc);
        assert!(
            !saved_paragraph_xml(&xml, 0).contains("<w:color"),
            "the heading's colour stays the style's: {}",
            saved_paragraph_xml(&xml, 0)
        );
        let colours = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .expect("heading")
                .runs
                .iter()
                .map(|run| {
                    run.properties
                        .as_ref()
                        .and_then(|props| props.color.clone())
                })
                .collect::<Vec<_>>()
        };
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(colours(&word_doc), colours(&reloaded));
    }

    #[test]
    fn enter_after_a_heading_starts_a_paragraph_as_a_reload_reads_it() {
        // Enter at the end of a heading starts its `w:next` paragraph. It had
        // only the next style's own spacing, not the document default's 8pt
        // after, and its run kept the heading run's `w:rPr`: what was typed
        // there showed in the body style and was saved in the heading colour.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/toc.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let heading_len = word_doc
            .paragraph(0)
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.len())
                    .sum::<usize>()
            })
            .expect("first paragraph");
        // The fixture's runs carry no direct colour: give the heading one, so
        // that a run copied into the next paragraph would show.
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 0,
                        end_paragraph: 0,
                        style_id: Some("1".to_string()),
                    },
                    HeadlessCommand::SetFontColor {
                        start_paragraph: 0,
                        start_offset: 0,
                        end_paragraph: 0,
                        end_offset: heading_len,
                        color: "#C00000".to_string(),
                    },
                    HeadlessCommand::InsertParagraphBreak {
                        paragraph: 0,
                        offset: heading_len,
                        count: 1,
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 1,
                        offset: 0,
                        text: "next".to_string(),
                    },
                ],
            },
        )
        .expect("edits apply");
        let xml = toc_document_xml(&word_doc);
        assert!(
            !saved_paragraph_xml(&xml, 1).contains("<w:color"),
            "typed text takes the body style: {}",
            saved_paragraph_xml(&xml, 1)
        );
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        let next = |word_doc: &WordDocument| {
            let paragraph = word_doc.paragraph(1).expect("next paragraph");
            let properties = paragraph.properties.clone().unwrap_or_default();
            (
                properties.style.map(|style| style.to_string()),
                properties.spacing,
                paragraph
                    .runs
                    .iter()
                    .map(|run| {
                        run.properties
                            .as_ref()
                            .and_then(|props| props.color.clone())
                    })
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(next(&word_doc), next(&reloaded));
        assert_eq!(
            next(&word_doc).1.and_then(|spacing| spacing.after),
            Some(8.0),
            "the document default's space after"
        );
    }

    #[test]
    fn a_text_box_moved_by_a_split_keeps_its_xml() {
        // Enter before a text box's anchor moves the text box into the new
        // paragraph. It was rebuilt from the model there: its VML fallback
        // went, its paragraph lost its style and its runs got their
        // inherited fonts written in. It now moves as written, and stays
        // editable in both copies.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/textbox.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let text_box_contents = |word_doc: &WordDocument| {
            let exported = word_doc.export_docx().expect("export");
            let mut archive =
                zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("exported zip");
            let mut xml = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("word/document.xml").expect("document part"),
                &mut xml,
            )
            .expect("read document part");
            xml.split("<w:txbxContent>")
                .skip(1)
                .map(|content| {
                    content
                        .split("</w:txbxContent>")
                        .next()
                        .unwrap_or_default()
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: 2,
                    count: 1,
                }],
            },
        )
        .expect("break applies");
        let contents = text_box_contents(&word_doc);
        assert_eq!(contents.len(), 2, "the DrawingML box and its VML fallback");
        for content in &contents {
            assert!(content.contains("<w:pStyle w:val=\"本文\"/>"), "{content}");
            assert!(!content.contains("<w:rFonts"), "{content}");
        }

        // The text box's paragraph follows its host, now paragraph 1.
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertText {
                    paragraph: 2,
                    offset: 0,
                    text: "X".to_string(),
                }],
            },
        )
        .expect("typing applies");
        let contents = text_box_contents(&word_doc);
        assert_eq!(contents.len(), 2);
        for content in &contents {
            assert!(content.contains("XHello"), "{content}");
        }
    }

    #[test]
    fn undoing_paragraph_formatting_in_a_note_or_text_box_restores_its_xml() {
        // Undo puts a paragraph's `w:pPr` back from a snapshot that only
        // reached a body paragraph's own node: a footnote's or a text box's
        // paragraph took the edit back on screen and kept it in the file.
        use crate::commands::{CommandContext, CommandExecutor};
        for (fixture, paragraph) in [("footnote.docx", 1), ("textbox.docx", 1)] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../public/fixtures")
                    .join(fixture),
            )
            .expect("read fixture");
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let formatting = |word_doc: &WordDocument| {
                let properties = word_doc
                    .paragraph(paragraph)
                    .and_then(|paragraph| paragraph.properties.clone())
                    .unwrap_or_default();
                (
                    properties.alignment.map(|alignment| alignment.to_string()),
                    properties.spacing.and_then(|spacing| spacing.line),
                )
            };
            let before = formatting(&word_doc);
            let mut command_executor = CommandExecutor::new();
            apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut command_executor,
                None,
                &HeadlessEditRequest {
                    commands: vec![
                        HeadlessCommand::SetParagraphAlignment {
                            start_paragraph: paragraph,
                            end_paragraph: paragraph,
                            alignment: "center".to_string(),
                        },
                        HeadlessCommand::SetParagraphLineSpacing {
                            start_paragraph: paragraph,
                            end_paragraph: paragraph,
                            line: Some(Some(2.0)),
                            line_rule: None,
                            before: None,
                            after: None,
                        },
                    ],
                },
            )
            .expect("formatting applies");
            let mut selection = None;
            for _ in 0..2 {
                command_executor
                    .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
                    .expect("undo");
            }
            assert_eq!(formatting(&word_doc), before, "{fixture}");
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(formatting(&reloaded), before, "{fixture}: saved");
        }
    }

    #[test]
    fn a_style_applied_in_a_note_or_text_box_is_saved_as_shown() {
        // Applying a paragraph style keeps the formatting the paragraph
        // declares and takes the style's run formatting out of its runs. Both
        // read and wrote only a body paragraph's own XML: in a footnote or a
        // text box the declared alignment was dropped on screen only, and the
        // runs' direct size stayed in the file.
        use crate::commands::{CommandContext, CommandExecutor};
        let digest = |word_doc: &WordDocument, paragraph: usize| {
            let paragraph = word_doc.paragraph(paragraph).expect("paragraph");
            let properties = paragraph.properties.clone().unwrap_or_default();
            (
                properties.style.map(|style| style.to_string()),
                properties.alignment.map(|alignment| alignment.to_string()),
                paragraph
                    .runs
                    .iter()
                    .map(|run| {
                        (
                            run.text.clone(),
                            run.properties.as_ref().and_then(|props| props.font_size),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        };
        for (fixture, paragraph, first) in [
            (
                "footnote.docx",
                1,
                HeadlessCommand::SetFontSize {
                    start_paragraph: 1,
                    start_offset: 0,
                    end_paragraph: 1,
                    end_offset: 6,
                    font_size: 20.0,
                },
            ),
            (
                "footnote.docx",
                1,
                HeadlessCommand::SetParagraphAlignment {
                    start_paragraph: 1,
                    end_paragraph: 1,
                    alignment: "center".to_string(),
                },
            ),
            (
                "textbox.docx",
                1,
                HeadlessCommand::SetFontSize {
                    start_paragraph: 1,
                    start_offset: 0,
                    end_paragraph: 1,
                    end_offset: 2,
                    font_size: 20.0,
                },
            ),
        ] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../public/fixtures")
                    .join(fixture),
            )
            .expect("read fixture");
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let mut command_executor = CommandExecutor::new();
            for command in [
                first.clone(),
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: paragraph,
                    end_paragraph: paragraph,
                    style_id: Some("Heading1".to_string()),
                },
            ] {
                apply_headless_commands_with_executor_to_document(
                    &mut word_doc,
                    &mut command_executor,
                    None,
                    &HeadlessEditRequest {
                        commands: vec![command],
                    },
                )
                .expect("edit applies");
            }
            let reload = |word_doc: &WordDocument| {
                executor::block_on(WordDocument::load(
                    &word_doc.export_docx().expect("export"),
                    &Options::default(),
                ))
                .expect("reload")
            };
            assert_eq!(
                digest(&word_doc, paragraph),
                digest(&reload(&word_doc), paragraph),
                "{fixture} after {first:?}"
            );
            let mut selection = None;
            command_executor
                .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
                .expect("undo the style");
            assert_eq!(
                digest(&word_doc, paragraph),
                digest(&reload(&word_doc), paragraph),
                "{fixture} after undoing the style"
            );
        }
    }

    #[test]
    fn a_deferred_load_keeps_inherited_fonts_out_of_edited_runs() {
        // The editor loads with the round-trip XML deferred to the first edit.
        // That pass recorded which font properties a run has as its own only
        // after the styles' fonts had been filled in, and a run it cannot tie
        // to its `w:r` (one inside a `w:moveTo`) kept them all: editing it
        // wrote the document's default fonts and size into its `w:rPr`.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/review-kinds.docx"),
        )
        .expect("read fixture");
        let mut word_doc = executor::block_on(WordDocument::load_for_initial_render(
            &bytes,
            &Options::default(),
        ))
        .expect("load");
        word_doc.ensure_roundtrip_xml().expect("round-trip XML");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetHighlightColor {
                    // "words" of the moved text, inside its `w:moveTo`.
                    start_paragraph: 2,
                    start_offset: 13,
                    end_paragraph: 2,
                    end_offset: 18,
                    color: "yellow".to_string(),
                }],
            },
        )
        .expect("highlight applies");
        let exported = word_doc.export_docx().expect("export");
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("exported zip");
        let mut xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/document.xml").expect("document part"),
            &mut xml,
        )
        .expect("read document part");
        let body = &xml[xml.find("<w:body>").expect("body")..];
        let target = body.split("</w:p>").nth(2).expect("third paragraph");
        assert!(
            target.contains("<w:highlight w:val=\"yellow\"/>"),
            "{target}"
        );
        for inherited in ["<w:rFonts", "<w:sz ", "<w:kern", "<w:w "] {
            assert!(!target.contains(inherited), "{inherited} written: {target}");
        }
    }

    #[test]
    fn undoing_a_deletion_in_a_table_of_contents_entry_keeps_its_field() {
        // A TOC entry is a hyperlink holding the heading's text, a tab and a
        // PAGEREF field. Undoing a deletion of its tab put the tab's run back
        // before its XML, and the rebuild took that run for the end of the
        // hyperlink: it and every run after it were written outside, the
        // page number twice and the field without its code.
        use crate::commands::{CommandContext, CommandExecutor};
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/toc.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let entry = |word_doc: &WordDocument| {
            word_doc
                .paragraph(2)
                .expect("first TOC entry")
                .runs
                .iter()
                .map(|run| run.text.clone())
                .collect::<String>()
        };
        assert_eq!(entry(&word_doc), "Test\t1");
        let mut command_executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut command_executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::DeleteText {
                    paragraph: 2,
                    offset: 5,
                    direction: jade_docx_headless_api::HeadlessDeleteDirection::Backward,
                    count: 2,
                }],
            },
        )
        .expect("delete applies");
        assert_eq!(entry(&word_doc), "Tes1");
        let mut selection = None;
        command_executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo");
        assert_eq!(entry(&word_doc), "Test\t1");
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(entry(&reloaded), "Test\t1");
        let field_results = |word_doc: &WordDocument| {
            word_doc
                .paragraph(2)
                .expect("first TOC entry")
                .runs
                .iter()
                .filter(|run| run.field.is_some())
                .count()
        };
        assert_eq!(
            field_results(&reloaded),
            1,
            "the page number is still a field"
        );
    }

    #[test]
    fn typing_in_a_text_box_keeps_inherited_fonts_out_of_its_runs() {
        // A text box's runs were left out of the pass that records which font
        // properties a run declares, so every font they inherit counted as
        // their own and typing in the box wrote it into their `w:rPr`.
        for deferred in [false, true] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../public/fixtures/textbox.docx"),
            )
            .expect("read fixture");
            let mut word_doc = if deferred {
                let mut word_doc = executor::block_on(WordDocument::load_for_initial_render(
                    &bytes,
                    &Options::default(),
                ))
                .expect("load");
                word_doc.ensure_roundtrip_xml().expect("round-trip XML");
                word_doc
            } else {
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load")
            };
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![HeadlessCommand::InsertText {
                        // The text box's paragraph, after its host.
                        paragraph: 1,
                        offset: 0,
                        text: "X".to_string(),
                    }],
                },
            )
            .expect("typing applies");
            let exported = word_doc.export_docx().expect("export");
            let mut archive =
                zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("exported zip");
            let mut xml = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("word/document.xml").expect("document part"),
                &mut xml,
            )
            .expect("read document part");
            let contents: Vec<&str> = xml
                .split("<w:txbxContent>")
                .skip(1)
                .map(|content| content.split("</w:txbxContent>").next().unwrap_or_default())
                .collect();
            assert!(!contents.is_empty());
            for content in contents {
                assert!(content.contains("XHello"), "{content}");
                for inherited in ["<w:rFonts", "<w:sz ", "<w:kern", "<w:w "] {
                    assert!(
                        !content.contains(inherited),
                        "deferred={deferred}: {inherited} written: {content}"
                    );
                }
            }
        }
    }

    #[test]
    fn joining_a_paragraph_that_hosts_a_text_box_keeps_both() {
        // Removing the joined paragraph also removes its text box's
        // paragraphs from the paragraph order, which the join's check did not
        // allow for: Backspace at the start of a paragraph hosting a text box
        // failed after taking the paragraph's runs, and its text and the text
        // box were gone. Delete at the end of the paragraph before it is the
        // same join from the other side.
        use crate::commands::{CommandContext, CommandExecutor};
        use jade_docx_headless_api::HeadlessDeleteDirection;
        for (paragraph, offset, direction) in [
            (1, 0, HeadlessDeleteDirection::Backward),
            (0, 2, HeadlessDeleteDirection::Forward),
        ] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../public/fixtures/textbox.docx"),
            )
            .expect("read fixture");
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let texts = |word_doc: &WordDocument| {
                (0..word_doc.paragraph_count())
                    .map(|index| {
                        word_doc
                            .paragraph(index)
                            .map(|paragraph| {
                                paragraph.runs.iter().map(|run| run.text.as_str()).collect()
                            })
                            .unwrap_or_default()
                    })
                    .collect::<Vec<String>>()
            };
            let mut command_executor = CommandExecutor::new();
            let mut apply = |word_doc: &mut WordDocument, command: HeadlessCommand| {
                apply_headless_commands_with_executor_to_document(
                    word_doc,
                    &mut command_executor,
                    None,
                    &HeadlessEditRequest {
                        commands: vec![command],
                    },
                )
                .expect("edit applies");
            };
            // "He" | "llo" + the text box's anchor.
            apply(
                &mut word_doc,
                HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: 2,
                    count: 1,
                },
            );
            assert_eq!(texts(&word_doc), ["He", "llo", "Hello", "World"]);
            apply(
                &mut word_doc,
                HeadlessCommand::DeleteText {
                    paragraph,
                    offset,
                    direction,
                    count: 1,
                },
            );
            assert_eq!(
                texts(&word_doc),
                ["Hello", "Hello", "World"],
                "{direction:?}"
            );
            // The text box is still there, and still editable.
            apply(
                &mut word_doc,
                HeadlessCommand::InsertText {
                    paragraph: 1,
                    offset: 0,
                    text: "X".to_string(),
                },
            );
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(
                texts(&reloaded),
                ["Hello", "XHello", "World"],
                "{direction:?}"
            );
            let mut selection = None;
            for _ in 0..2 {
                command_executor
                    .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
                    .expect("undo");
            }
            assert_eq!(
                texts(&word_doc),
                ["He", "llo", "Hello", "World"],
                "{direction:?}"
            );
        }
    }

    #[test]
    fn undoing_a_style_after_other_undone_edits_restores_the_run_formatting() {
        // Applying a style takes the style's run formatting out of the runs'
        // `w:rPr`, and its undo puts it back by the parent's id. Edits made
        // and undone in between rebuild those elements, so the 9pt size given
        // before the style came back on screen and was lost from the file.
        use crate::commands::{CommandContext, CommandExecutor};
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/2col.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let mut command_executor = CommandExecutor::new();
        let mut selection = None;
        let apply = |word_doc: &mut WordDocument,
                     command_executor: &mut CommandExecutor,
                     command: HeadlessCommand| {
            apply_headless_commands_with_executor_to_document(
                word_doc,
                command_executor,
                None,
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
            .expect("edit applies");
        };
        apply(
            &mut word_doc,
            &mut command_executor,
            HeadlessCommand::SetFontSize {
                start_paragraph: 0,
                start_offset: 855,
                end_paragraph: 0,
                end_offset: 2348,
                font_size: 9.0,
            },
        );
        apply(
            &mut word_doc,
            &mut command_executor,
            HeadlessCommand::SetParagraphStyle {
                start_paragraph: 0,
                end_paragraph: 0,
                style_id: Some("1".to_string()),
            },
        );
        apply(
            &mut word_doc,
            &mut command_executor,
            HeadlessCommand::InsertParagraphBreak {
                paragraph: 0,
                offset: 1272,
                count: 1,
            },
        );
        command_executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo the break");
        apply(
            &mut word_doc,
            &mut command_executor,
            HeadlessCommand::DeleteText {
                paragraph: 0,
                offset: 1987,
                direction: jade_docx_headless_api::HeadlessDeleteDirection::Backward,
                count: 1,
            },
        );
        command_executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo the deletion");
        command_executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo the style");
        let sizes = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .expect("paragraph")
                .runs
                .iter()
                .flat_map(|run| {
                    let size = run.properties.as_ref().and_then(|props| props.font_size);
                    run.text.chars().map(move |_| size)
                })
                .collect::<Vec<_>>()
        };
        let live = sizes(&word_doc);
        assert_eq!(live[900], Some(9.0), "the size given before the style");
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        let at_nine =
            |sizes: &[Option<f32>]| sizes.iter().filter(|size| **size == Some(9.0)).count();
        let saved = sizes(&reloaded);
        assert!(
            saved == live,
            "{} characters at 9pt on screen, {} saved",
            at_nine(&live),
            at_nine(&saved)
        );
    }

    #[test]
    fn undoing_enter_before_a_section_break_restores_the_sections() {
        // Undoing Enter merges the paragraphs back, which rebuilds the section
        // ranges, and only then puts the original paragraph, with its section
        // break, back: every range stayed one paragraph too long, and the
        // editor laid the document out in 2 pages that saved as 4.
        use crate::commands::{CommandContext, CommandExecutor};
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/section-break-types.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let ranges = |word_doc: &WordDocument| {
            word_doc
                .document
                .as_ref()
                .expect("document")
                .sections
                .iter()
                .map(|section| (section.start, section.end))
                .collect::<Vec<_>>()
        };
        let loaded = ranges(&word_doc);
        let mut command_executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut command_executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: 0,
                    count: 1,
                }],
            },
        )
        .expect("break applies");
        let mut selection = None;
        command_executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo");
        assert_eq!(ranges(&word_doc), loaded);
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(ranges(&reloaded), loaded);
    }

    #[test]
    fn restyled_runs_take_the_new_styles_theme_fonts() {
        // A load resolves the theme's fonts into the runs (comment.docx's
        // headings are 游ゴシック Light, its body 游明朝). A heading turned body
        // text kept the heading's fonts, compared unresolved with the old
        // style's, and a style applied again left the runs with the style's
        // unresolved theme keys: no font at all until the file was reopened.
        use crate::commands::CommandExecutor;
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/comment.docx"),
        )
        .expect("read fixture");
        let fonts = |word_doc: &WordDocument, paragraph: usize| {
            word_doc
                .paragraph(paragraph)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| {
                    let fonts = run
                        .properties
                        .as_ref()
                        .and_then(|props| props.fonts.as_ref());
                    (
                        fonts.and_then(|fonts| fonts.ascii.as_deref().map(str::to_string)),
                        fonts.and_then(|fonts| fonts.east_asia.as_deref().map(str::to_string)),
                    )
                })
                .collect::<Vec<_>>()
        };
        for second_style in [None, Some("Heading2")] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let mut command_executor = CommandExecutor::new();
            for command in [
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: 0,
                    end_paragraph: 0,
                    style_id: Some("Heading2".to_string()),
                },
                HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: 4,
                    count: 1,
                },
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: 1,
                    end_paragraph: 1,
                    style_id: second_style.map(str::to_string),
                },
            ] {
                apply_headless_commands_with_executor_to_document(
                    &mut word_doc,
                    &mut command_executor,
                    None,
                    &HeadlessEditRequest {
                        commands: vec![command],
                    },
                )
                .expect("edit applies");
            }
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export"),
                &Options::default(),
            ))
            .expect("reload");
            let live = fonts(&word_doc, 1);
            assert_eq!(live, fonts(&reloaded, 1), "then {second_style:?}");
            let expected = if second_style.is_some() {
                "游ゴシック Light"
            } else {
                "游明朝"
            };
            assert_eq!(
                live[0].1.as_deref(),
                Some(expected),
                "then {second_style:?}"
            );
        }
    }

    #[test]
    fn an_edit_in_a_text_box_keeps_its_complex_field() {
        // document-variable-fields.docx's VML text box holds a DOCVARIABLE
        // field. Formatting, a comment or italics anywhere in that paragraph
        // dropped the field's code runs, which the model knows only as empty
        // runs, and saved its stale result as plain text.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/document-variable-fields.docx"),
        )
        .expect("read fixture");
        let text = |word_doc: &WordDocument| {
            word_doc
                .paragraph(5)
                .expect("text box paragraph")
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        };
        for command in [
            HeadlessCommand::SetFontSize {
                start_paragraph: 5,
                start_offset: 8,
                end_paragraph: 5,
                end_offset: 12,
                font_size: 14.0,
            },
            HeadlessCommand::AddComment {
                start_paragraph: 5,
                start_offset: Some(9),
                end_paragraph: 5,
                end_offset: Some(12),
                text: "note".to_string(),
                author: Some("test".to_string()),
                initials: None,
                date: None,
            },
            HeadlessCommand::ApplyItalic {
                start_paragraph: 5,
                start_offset: 5,
                end_paragraph: 5,
                end_offset: 7,
                enabled: true,
            },
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            assert!(!word_doc.is_body_paragraph_surface(5));
            let label = format!("{command:?}");
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
            .expect("edit applies");
            let exported = word_doc.export_docx().expect("export succeeds");
            let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reload");
            assert_eq!(text(&reloaded), text(&word_doc), "{label}");
            assert_eq!(text(&word_doc), "Textbox channel: ", "{label}");
            let mut archive =
                zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("package");
            let mut xml = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("word/document.xml").expect("document part"),
                &mut xml,
            )
            .expect("read part");
            assert!(
                xml.contains(r#"IABLE "Release Channel" "#),
                "{label}: {xml}"
            );
        }
    }

    #[test]
    fn a_paragraph_whose_style_numbering_is_taken_off_keeps_the_styles_indent() {
        // numbering-by-style.docx's heading 3 numbers its paragraphs and
        // declares its own indent. Taking the numbering off writes
        // `w:numId w:val="0"`; the load read that as a list and dropped the
        // style's indent, so the paragraph moved left when the file was
        // reopened.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/numbering-by-style.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetNumbering {
                    start_paragraph: 1,
                    end_paragraph: 1,
                    num_id: None,
                    level: None,
                }],
            },
        )
        .expect("numbering comes off");
        let indent = |word_doc: &WordDocument| {
            word_doc
                .paragraph(1)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.indent.as_ref())
                .map(|indent| (indent.left, indent.hanging))
        };
        assert_eq!(indent(&word_doc), Some((Some(26.25), Some(15.75))));
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(indent(&reloaded), indent(&word_doc));
    }

    #[test]
    fn text_typed_into_a_shape_without_text_is_saved() {
        // shape-preset-geometries.docx's first shape has no `wps:txbx`. The
        // editor lists an empty paragraph in it to type into, with nothing in
        // the package behind it: what was typed, and the paragraph's style,
        // were lost on save.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/shape-preset-geometries.docx"),
        )
        .expect("read fixture");
        let text_boxes = |bytes: &[u8]| {
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("package");
            let mut xml = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("word/document.xml").expect("document part"),
                &mut xml,
            )
            .expect("read part");
            xml.matches("<wps:txbx>").count()
        };
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let untouched = text_boxes(&word_doc.export_docx().expect("export"));
        assert_eq!(
            untouched,
            text_boxes(&bytes),
            "nothing is added until edited"
        );
        assert!(!word_doc.is_body_paragraph_surface(2));
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 2,
                        offset: 0,
                        text: "図形".to_string(),
                    },
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 2,
                        end_paragraph: 2,
                        style_id: Some("Heading2".to_string()),
                    },
                ],
            },
        )
        .expect("edits apply");
        let paragraph = |word_doc: &WordDocument| {
            let paragraph = word_doc.paragraph(2).expect("shape paragraph");
            (
                paragraph
                    .properties
                    .as_ref()
                    .and_then(|props| props.style.as_deref().map(str::to_string)),
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
            )
        };
        let exported = word_doc.export_docx().expect("export succeeds");
        assert_eq!(text_boxes(&exported), untouched + 1);
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(paragraph(&reloaded), paragraph(&word_doc));
        assert_eq!(paragraph(&word_doc).1, "図形");
    }

    #[test]
    fn undoing_a_replacement_that_emptied_a_field_restores_its_xml() {
        // hyperlink-field-edge-cases.docx's third paragraph is text followed
        // by a HYPERLINK simple field. Replacing all but the field's last
        // three letters left one run, inside the field. Undo re-synced the
        // restored runs into that: the field kept the replacement and the
        // old text was written again after it, so the file read
        // "ab llyInternal field: Jump locally".
        use crate::commands::{CommandContext, CommandExecutor};
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/hyperlink-field-edge-cases.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let original = word_doc.export_docx().expect("export");
        let mut command_executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut command_executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::ReplaceRange {
                    start_paragraph: 2,
                    start_offset: 0,
                    end_paragraph: 2,
                    end_offset: 25,
                    text: "ab ".to_string(),
                }],
            },
        )
        .expect("replacement applies");
        let mut selection = None;
        assert!(command_executor
            .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
            .expect("undo"));
        let document_xml = |bytes: &[u8]| {
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("package");
            let mut xml = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("word/document.xml").expect("document part"),
                &mut xml,
            )
            .expect("read part");
            xml
        };
        assert_eq!(
            document_xml(&word_doc.export_docx().expect("export")),
            document_xml(&original)
        );
    }

    #[test]
    fn a_footnote_stays_after_the_half_that_references_it() {
        // A footnote is listed right after the paragraph that references it.
        // Enter after the reference kept the footnote where it was, after the
        // new second half, so the note paragraphs' indices differed from a
        // reload until the document was reopened.
        for (fixture, paragraph, offset) in [
            ("footnote.docx", 0, "Hello".len()),
            ("footnote-table-textbox-order.docx", 9, 10),
        ] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../public/fixtures")
                    .join(fixture),
            )
            .expect("read fixture");
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![HeadlessCommand::InsertParagraphBreak {
                        paragraph,
                        offset,
                        count: 1,
                    }],
                },
            )
            .expect("break applies");
            let paragraphs = |word_doc: &WordDocument| {
                (0..word_doc.paragraph_count())
                    .map(|index| {
                        let paragraph = word_doc.paragraph(index).expect("paragraph");
                        (
                            paragraph
                                .properties
                                .as_ref()
                                .and_then(|props| props.style.as_deref().map(str::to_string)),
                            paragraph
                                .runs
                                .iter()
                                .map(|run| run.text.as_str())
                                .collect::<String>(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export succeeds"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(paragraphs(&word_doc), paragraphs(&reloaded), "{fixture}");
        }
    }

    #[test]
    fn runs_moved_to_another_style_take_its_fonts_as_a_load_does() {
        // A load fills a run's fonts slot by slot, from its paragraph style
        // and the document defaults. A style change told inherited from
        // direct fonts by matching them against the old style's, which such a
        // run never does, nor did a run whose `w:rFonts` holds only a hint:
        // both kept the old style's fonts on screen only.
        let fonts = |word_doc: &WordDocument, paragraph: usize| {
            word_doc
                .paragraph(paragraph)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| {
                    let fonts = run
                        .properties
                        .as_ref()
                        .and_then(|props| props.fonts.as_ref());
                    (
                        run.text.clone(),
                        fonts.and_then(|fonts| fonts.ascii.as_deref().map(str::to_string)),
                        fonts.and_then(|fonts| fonts.east_asia.as_deref().map(str::to_string)),
                    )
                })
                .collect::<Vec<_>>()
        };
        for (fixture, command, paragraph) in [
            (
                // Heading 4 to the list style "1".
                "numbering-by-style.docx",
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: 3,
                    end_paragraph: 3,
                    style_id: Some("1".to_string()),
                },
                3,
            ),
            (
                // Joins a body paragraph into the heading before it.
                "toc.docx",
                HeadlessCommand::DeleteText {
                    paragraph: 15,
                    offset: 4,
                    direction: jade_docx_headless_api::HeadlessDeleteDirection::Forward,
                    count: 3,
                },
                15,
            ),
        ] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../public/fixtures")
                    .join(fixture),
            )
            .expect("read fixture");
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
            .expect("edit applies");
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export"),
                &Options::default(),
            ))
            .expect("reload");
            let live: Vec<_> = fonts(&word_doc, paragraph)
                .into_iter()
                .filter(|(text, _, _)| !text.is_empty())
                .collect();
            let saved: Vec<_> = fonts(&reloaded, paragraph)
                .into_iter()
                .filter(|(text, _, _)| !text.is_empty())
                .collect();
            // The reload joins runs the edit left apart; compare by text.
            let by_char = |runs: Vec<(String, Option<String>, Option<String>)>| {
                runs.into_iter()
                    .flat_map(|(text, ascii, east_asia)| {
                        text.chars()
                            .map(move |_| (ascii.clone(), east_asia.clone()))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(by_char(live), by_char(saved), "{fixture}");
        }
    }

    #[test]
    fn a_replacement_across_a_comment_reference_keeps_the_link_text_once() {
        // A comment ending inside the HYPERLINK field's result of
        // hyperlink-field-edge-cases.docx's third paragraph, then a
        // replacement from the first paragraph into that result: the
        // comment's reference run went over twice, kept with the field's
        // code and transferred as a comment reference. The second copy split
        // the link around it, and "ple link" was saved twice.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/hyperlink-field-edge-cases.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 2,
                        offset: 1,
                        text: "漢字".to_string(),
                    },
                    HeadlessCommand::AddComment {
                        start_paragraph: 0,
                        start_offset: Some(21),
                        end_paragraph: 2,
                        end_offset: Some(24),
                        text: "note".to_string(),
                        author: Some("test".to_string()),
                        initials: None,
                        date: None,
                    },
                    HeadlessCommand::ClearFormatting {
                        start_paragraph: 2,
                        start_offset: 25,
                        end_paragraph: 2,
                        end_offset: 26,
                    },
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: 0,
                        start_offset: 4,
                        end_paragraph: 2,
                        end_offset: 26,
                        text: "、".to_string(),
                    },
                ],
            },
        )
        .expect("edits apply");
        let text = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        };
        assert_eq!(text(&word_doc), "HYPE、ple link");
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(text(&reloaded), text(&word_doc));
        let references = word_doc
            .paragraph(0)
            .expect("paragraph")
            .runs
            .iter()
            .filter(|run| !run.comment_reference_ids.is_empty())
            .count();
        assert_eq!(references, 1);
    }

    #[test]
    fn a_comment_ending_inside_a_field_or_link_keeps_its_text_once() {
        // A comment boundary inside a `w:fldSimple` or `w:hyperlink` put its
        // markers beside the container while the model put the reference run
        // among the container's runs. The next edit wrote the container up to
        // the reference run, closed it, and wrote the rest again outside:
        // "Jump locally" saved as "Jump locallylly". The comment may also
        // start outside and end inside, or span paragraphs into a TOC link.
        let comment = |start_paragraph, start_offset, end_paragraph, end_offset| {
            HeadlessCommand::AddComment {
                start_paragraph,
                start_offset: Some(start_offset),
                end_paragraph,
                end_offset: Some(end_offset),
                text: "note".to_string(),
                author: Some("test".to_string()),
                initials: None,
                date: None,
            }
        };
        let cases = [
            (
                "hyperlink-field-edge-cases.docx",
                vec![
                    comment(4, 24, 4, 25),
                    HeadlessCommand::DeleteText {
                        paragraph: 4,
                        offset: 1,
                        direction: HeadlessDeleteDirection::Forward,
                        count: 2,
                    },
                ],
            ),
            (
                "hyperlink-field-edge-cases.docx",
                vec![
                    comment(4, 4, 4, 22),
                    HeadlessCommand::SetFontSize {
                        start_paragraph: 4,
                        start_offset: 2,
                        end_paragraph: 4,
                        end_offset: 19,
                        font_size: 9.0,
                    },
                ],
            ),
            (
                "toc.docx",
                vec![
                    comment(0, 2, 2, 1),
                    HeadlessCommand::InsertText {
                        paragraph: 2,
                        offset: 1,
                        text: "あ".to_string(),
                    },
                ],
            ),
        ];
        let texts = |word_doc: &WordDocument| {
            (0..word_doc.paragraph_count())
                .map(|index| {
                    word_doc
                        .paragraph(index)
                        .expect("paragraph")
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };
        for (fixture, commands) in cases {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../public/fixtures")
                    .join(fixture),
            )
            .expect("read fixture");
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let label = format!("{fixture} {commands:?}");
            apply_headless_commands_to_document(&mut word_doc, &HeadlessEditRequest { commands })
                .expect("edits apply");
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export succeeds"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(texts(&reloaded), texts(&word_doc), "{label}");
            let comments = |word_doc: &WordDocument| {
                (0..word_doc.paragraph_count())
                    .map(|index| {
                        word_doc
                            .paragraph(index)
                            .expect("paragraph")
                            .comment_ranges
                            .iter()
                            // An empty paragraph inside the range has no
                            // marker of its own to rebuild it from.
                            .filter(|range| range.start < range.end)
                            .map(|range| (range.start, range.end))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(comments(&reloaded), comments(&word_doc), "{label}");
        }
    }

    #[test]
    fn enter_in_a_paragraph_whose_text_box_holds_an_equation_lists_it_once() {
        // math.docx's paragraph 19 hosts a text box with an equation, which
        // the paragraph order lists as a math surface after the paragraph.
        // Enter in the paragraph patched the order in place, could not tell
        // that surface from the ones after the paragraph, and listed it twice:
        // one paragraph more than a reload, and every later index off by one.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/math.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertParagraphBreak {
                    paragraph: 19,
                    offset: 84,
                    count: 1,
                }],
            },
        )
        .expect("break applies");
        let surfaces = |word_doc: &WordDocument| {
            (0..word_doc.paragraph_count())
                .map(|index| {
                    word_doc.paragraph(index).map(|paragraph| {
                        paragraph
                            .runs
                            .iter()
                            .map(|run| run.text.as_str())
                            .collect::<String>()
                    })
                })
                .collect::<Vec<_>>()
        };
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(surfaces(&word_doc), surfaces(&reloaded));
    }

    #[test]
    fn joining_a_paragraph_that_references_a_note_keeps_the_note_after_it() {
        // A footnote is listed right after the paragraph that references it.
        // Backspace into the paragraph before took the reference out of the
        // joined paragraph before the list was rebuilt, so the footnote went
        // behind the separator notes: every note paragraph's index changed
        // until the document was reopened.
        for fixture in ["footnote.docx", "endnote.docx"] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../public/fixtures")
                    .join(fixture),
            )
            .expect("read fixture");
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![
                        HeadlessCommand::InsertParagraphBreak {
                            paragraph: 0,
                            offset: 3,
                            count: 1,
                        },
                        HeadlessCommand::InsertParagraphBreak {
                            paragraph: 1,
                            offset: 2,
                            count: 1,
                        },
                        HeadlessCommand::DeleteText {
                            paragraph: 2,
                            offset: 0,
                            direction: HeadlessDeleteDirection::Backward,
                            count: 1,
                        },
                    ],
                },
            )
            .expect("edits apply");
            let paragraphs = |word_doc: &WordDocument| {
                (0..word_doc.paragraph_count())
                    .map(|index| {
                        let paragraph = word_doc.paragraph(index).expect("paragraph");
                        (
                            paragraph
                                .properties
                                .as_ref()
                                .and_then(|props| props.style.as_deref().map(str::to_string)),
                            paragraph
                                .runs
                                .iter()
                                .map(|run| run.text.as_str())
                                .collect::<String>(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export succeeds"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(paragraphs(&word_doc), paragraphs(&reloaded), "{fixture}");
        }
    }

    #[test]
    fn a_style_that_removes_a_runs_size_leaves_no_complex_script_size_in_charge() {
        // Applying a style takes the size it defines out of the runs: their
        // `w:sz` went, their `w:szCs` stayed, and the reader took a lone
        // `w:szCs` as the run's size, so the text was back at the old size
        // on reload. Word sizes Latin text by `w:sz` only.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/document-property-fields.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetFontSize {
                        start_paragraph: 0,
                        start_offset: 30,
                        end_paragraph: 1,
                        end_offset: 47,
                        font_size: 14.0,
                    },
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 1,
                        end_paragraph: 3,
                        style_id: None,
                    },
                ],
            },
        )
        .expect("edits apply");
        let sizes = |word_doc: &WordDocument| {
            word_doc
                .paragraph(1)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| {
                    (
                        run.text.clone(),
                        run.properties.as_ref().and_then(|props| props.font_size),
                    )
                })
                .collect::<Vec<_>>()
        };
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(sizes(&reloaded), sizes(&word_doc));
        assert!(
            sizes(&word_doc).iter().all(|(_, size)| *size == Some(10.5)),
            "{:?}",
            sizes(&word_doc)
        );
    }

    #[test]
    fn a_tracked_deletion_kept_from_a_joined_paragraph_takes_its_style() {
        // review-kinds.docx's second paragraph opens with tracked moved-from
        // text, which a replacement across it keeps. Joined to the first
        // paragraph (style "2", 10pt), that run and the replacement typed
        // after it kept the second paragraph's 12pt on screen; a reload sized
        // them by the joined paragraph.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/review-kinds.docx"),
        )
        .expect("read fixture");
        for replacement in ["、", ""] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let replace = HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 22,
                end_paragraph: 2,
                end_offset: 1,
                text: replacement.to_string(),
            };
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![
                        HeadlessCommand::SetParagraphStyle {
                            start_paragraph: 0,
                            end_paragraph: 0,
                            style_id: Some("2".to_string()),
                        },
                        replace,
                    ],
                },
            )
            .expect("edits apply");
            let sizes = |word_doc: &WordDocument| {
                word_doc
                    .paragraph(0)
                    .expect("paragraph")
                    .runs
                    .iter()
                    .filter(|run| !run.text.is_empty())
                    .map(|run| {
                        (
                            run.text.clone(),
                            run.properties.as_ref().and_then(|props| props.font_size),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export succeeds"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(sizes(&word_doc), sizes(&reloaded), "{replacement:?}");
        }
    }

    #[test]
    fn a_text_box_paragraph_split_off_by_enter_joins_with_its_field() {
        // Enter in document-variable-fields.docx's text box left the new
        // paragraph without its `w:p`'s id. Replacing a range from the first
        // half into it then had no XML to carry over, and the DOCVARIABLE
        // field it holds was saved as its stale result in plain text.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/document-variable-fields.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertParagraphBreak {
                    paragraph: 5,
                    offset: 6,
                    count: 1,
                }],
            },
        )
        .expect("break applies");
        assert!(!word_doc.is_body_paragraph_surface(6));
        assert!(word_doc.paragraph(6).and_then(|p| p.node_id).is_some());
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::ReplaceRange {
                    start_paragraph: 5,
                    start_offset: 2,
                    end_paragraph: 6,
                    end_offset: 9,
                    text: "あ".to_string(),
                }],
            },
        )
        .expect("replace applies");
        let text = |word_doc: &WordDocument| {
            word_doc
                .paragraph(5)
                .expect("text box paragraph")
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        };
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(text(&reloaded), text(&word_doc));
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("package");
        let mut xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/document.xml").expect("document part"),
            &mut xml,
        )
        .expect("read part");
        assert!(xml.contains(r#"IABLE "Release Channel" "#), "{xml}");
    }

    #[test]
    fn enter_in_a_text_box_keeps_its_complex_field() {
        // document-variable-fields.docx's VML text box reads
        // "Textbox channel: " and a DOCVARIABLE field. Enter in that paragraph
        // rebuilt every run of both halves from the model, which knows the
        // field's code runs only as empty runs and dropped them: the field's
        // stale result was saved as plain text.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/document-variable-fields.docx"),
        )
        .expect("read fixture");
        let texts = |word_doc: &WordDocument| {
            (0..word_doc.paragraph_count())
                .map(|index| {
                    word_doc
                        .paragraph(index)
                        .expect("paragraph")
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };
        for offset in [4, "Textbox channel:".len(), "Textbox channel: ".len()] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            assert!(!word_doc.is_body_paragraph_surface(5));
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![HeadlessCommand::InsertParagraphBreak {
                        paragraph: 5,
                        offset,
                        count: 1,
                    }],
                },
            )
            .expect("break applies");
            let exported = word_doc.export_docx().expect("export succeeds");
            let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reload");
            assert_eq!(texts(&reloaded), texts(&word_doc), "at {offset}");
            let mut archive =
                zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("package");
            let mut xml = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("word/document.xml").expect("document part"),
                &mut xml,
            )
            .expect("read part");
            assert!(
                xml.contains(r#"IABLE "Release Channel" "#),
                "at {offset}: {xml}"
            );
        }
    }

    #[test]
    fn formatting_split_twice_inside_a_hyperlink_writes_its_text_once() {
        // A run inside a w:hyperlink split by one format, then split again
        // by another: the second split's new piece was appended after the
        // container's last run, behind a run not yet updated, which was then
        // written a second time. The saved link read "exampmle.com".
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/hyperlink.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetFontSize {
                        start_paragraph: 0,
                        start_offset: 8,
                        end_paragraph: 0,
                        end_offset: 15,
                        font_size: 14.0,
                    },
                    HeadlessCommand::SetFontColor {
                        start_paragraph: 0,
                        start_offset: 10,
                        end_paragraph: 0,
                        end_offset: 17,
                        color: "#C00000".to_string(),
                    },
                ],
            },
        )
        .expect("formatting applies");
        let runs = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| run.text.clone())
                .collect::<Vec<_>>()
        };
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(runs(&reloaded), runs(&word_doc));
        assert!(runs(&word_doc).concat().starts_with("Visitexample.com"));
    }

    #[test]
    fn the_next_style_turns_off_a_list_a_pending_change_records() {
        // numbering-del.docx's second paragraph had its list taken off with
        // tracking on; its `w:pPrChange` keeps the list, by which the reader
        // numbers it. Enter at the end of it, once it is a heading, starts a
        // Normal paragraph that keeps that mark and its change: unnumbered on
        // screen, numbered again on reload.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/numbering-del.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 1,
                        end_paragraph: 1,
                        style_id: Some("Heading1".to_string()),
                    },
                    HeadlessCommand::InsertParagraphBreak {
                        paragraph: 1,
                        offset: "World".len(),
                        count: 1,
                    },
                ],
            },
        )
        .expect("edits apply");
        let lists = |word_doc: &WordDocument| {
            (0..word_doc.paragraph_count())
                .map(|index| {
                    let props = word_doc
                        .paragraph(index)
                        .and_then(|p| p.properties.as_ref());
                    (
                        props.and_then(|props| props.style.as_deref().map(str::to_string)),
                        props
                            .and_then(|props| props.numbering.as_ref())
                            .and_then(|numbering| numbering.list_id().map(str::to_string)),
                    )
                })
                .collect::<Vec<_>>()
        };
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(lists(&reloaded), lists(&word_doc));
        assert_eq!(lists(&word_doc)[2].1, None, "{:?}", lists(&word_doc));
    }

    #[test]
    fn a_comment_inside_a_hyperlink_keeps_the_links_text_once() {
        // The comment's markers and reference run went beside the w:hyperlink
        // while the model put the reference run among the link's runs. The
        // next edit wrote the link up to the reference run, closed it, and
        // wrote the rest again outside: "example.com" saved as
        // "exあample.commple.com".
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/hyperlink.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::AddComment {
                        start_paragraph: 0,
                        start_offset: Some(7),
                        end_paragraph: 0,
                        end_offset: Some(8),
                        text: "note".to_string(),
                        author: Some("test".to_string()),
                        initials: None,
                        date: None,
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 7,
                        text: "あ".to_string(),
                    },
                ],
            },
        )
        .expect("edits apply");
        let text = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| run.text.clone())
                .collect::<String>()
        };
        assert!(text(&word_doc).starts_with("Visitexあample.comto"));
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(text(&reloaded), text(&word_doc));
        let paragraph = reloaded.paragraph(0).expect("paragraph");
        let comment = paragraph.comment_ranges.first().expect("comment survives");
        assert_eq!(
            &text(&reloaded)[comment.start..comment.end],
            "a",
            "the comment still covers only the letter it was added on"
        );
        let link: String = paragraph
            .runs
            .iter()
            .filter(|run| run.hyperlink.is_some())
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(link, "exあample.com");
    }

    #[test]
    fn clearing_a_character_style_gives_the_run_its_paragraphs_size() {
        // Text typed at the start of comment.docx's second paragraph joins
        // the comment reference run's character style "aa" (9pt). Clearing
        // the character style left the 9pt it had given the run, while a
        // reload sized the text by its paragraph (10.5pt).
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/comment.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 9,
                        text: "漢字".to_string(),
                    },
                    HeadlessCommand::InsertParagraphBreak {
                        paragraph: 0,
                        offset: 16,
                        count: 1,
                    },
                    HeadlessCommand::InsertParagraphBreak {
                        paragraph: 1,
                        offset: 3,
                        count: 1,
                    },
                    HeadlessCommand::DeleteText {
                        paragraph: 0,
                        offset: 16,
                        direction: HeadlessDeleteDirection::Forward,
                        count: 1,
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 1,
                        offset: 0,
                        text: "ab ".to_string(),
                    },
                    HeadlessCommand::SetCharacterStyle {
                        start_paragraph: 1,
                        start_offset: 0,
                        end_paragraph: 1,
                        end_offset: 2,
                        style_id: None,
                    },
                ],
            },
        )
        .expect("edits apply");
        let sizes = |word_doc: &WordDocument| {
            word_doc
                .paragraph(1)
                .expect("paragraph")
                .runs
                .iter()
                .filter(|run| !run.text.is_empty())
                .map(|run| {
                    let props = run.properties.as_ref();
                    (
                        run.text.clone(),
                        props.and_then(|props| props.style.as_deref().map(str::to_string)),
                        props.and_then(|props| props.font_size),
                    )
                })
                .collect::<Vec<_>>()
        };
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(sizes(&word_doc), sizes(&reloaded));
    }

    #[test]
    fn a_style_applied_again_leaves_a_character_styles_size() {
        // comment.docx's comment references are in the character style "aa"
        // (9pt). Applying Heading 1 merged the heading's 16pt over every
        // run; a change of style rebased that away, but applying the same
        // style again skips the rebase, so the runs, and text typed into
        // them, showed 16pt and went back to 9pt on reload.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/comment.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let heading = || HeadlessCommand::SetParagraphStyle {
            start_paragraph: 0,
            end_paragraph: 0,
            style_id: Some("Heading1".to_string()),
        };
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![heading(), heading()],
            },
        )
        .expect("edits apply");
        let sizes = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| {
                    let props = run.properties.as_ref();
                    (
                        run.text.clone(),
                        props.and_then(|props| props.style.as_deref().map(str::to_string)),
                        props.and_then(|props| props.font_size),
                    )
                })
                .collect::<Vec<_>>()
        };
        let live = sizes(&word_doc);
        assert!(
            live.iter()
                .filter(|(_, style, _)| style.as_deref() == Some("aa"))
                .all(|(_, _, size)| *size == Some(9.0)),
            "{live:?}"
        );
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(sizes(&reloaded), live);
    }

    #[test]
    fn a_run_given_a_latin_font_takes_its_new_paragraphs_east_asian_font() {
        // numbering-by-style.docx: Arial set on three paragraphs names the
        // runs' Latin fonts only. Replacing a range across them joins the
        // last paragraph's tail to the first, whose style gives another East
        // Asian font; the joined runs kept their old paragraph's ＭＳ 明朝,
        // and a reload gave them ＭＳ ゴシック.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/numbering-by-style.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetFontFamily {
                        start_paragraph: 0,
                        start_offset: 0,
                        end_paragraph: 2,
                        end_offset: 90,
                        font_family: "Arial".to_string(),
                    },
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: 0,
                        start_offset: 9,
                        end_paragraph: 2,
                        end_offset: 18,
                        text: "あ".to_string(),
                    },
                ],
            },
        )
        .expect("edits apply");
        let fonts = |word_doc: &WordDocument| {
            word_doc
                .paragraph(0)
                .expect("paragraph")
                .runs
                .iter()
                .filter(|run| !run.text.is_empty())
                .map(|run| {
                    let fonts = run
                        .properties
                        .as_ref()
                        .and_then(|props| props.fonts.as_ref());
                    (
                        run.text.clone(),
                        fonts.and_then(|fonts| fonts.ascii.as_deref().map(str::to_string)),
                        fonts.and_then(|fonts| fonts.east_asia.as_deref().map(str::to_string)),
                    )
                })
                .collect::<Vec<_>>()
        };
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(fonts(&word_doc), fonts(&reloaded));
    }

    #[test]
    fn a_font_the_run_names_wins_over_its_styles_theme_font() {
        // numbering-restart.docx sets "AAAAA" in ＭＳ 明朝 under Heading 2,
        // whose w:asciiTheme is the theme's major font. Filling the run's
        // fonts from its style added that theme reference beside the name,
        // and a theme reference shadows a name: the text drew in the theme's
        // 游ゴシック Light, until typing in it brought ＭＳ 明朝 back.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/numbering-restart.docx"),
        )
        .expect("read fixture");
        let word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let paragraph = (0..word_doc.paragraph_count())
            .find(|&index| {
                word_doc
                    .paragraph(index)
                    .is_some_and(|paragraph| paragraph.runs.iter().any(|run| run.text == "AAAAA"))
            })
            .expect("the AAAAA paragraph");
        let fonts = word_doc.paragraph(paragraph).expect("paragraph").runs[0]
            .properties
            .as_ref()
            .and_then(|props| props.fonts.clone())
            .expect("fonts");
        assert_eq!(fonts.ascii.as_deref(), Some("ＭＳ 明朝"));
        assert_eq!(fonts.ascii_theme, None);
        assert_eq!(fonts.h_ansi_theme, None);
    }

    #[test]
    fn typing_writes_only_the_run_formatting_the_run_declares() {
        // A run's font properties come from its styles unless its w:rPr says
        // otherwise. A run with no w:rPr, and every run of a footnote, was
        // taken to declare all of them, so typing in it wrote the inherited
        // fonts and size into the file — and a style changed afterwards no
        // longer reached the text (Heading 1 in a footnote kept its 16pt
        // after the footnote went back to its own style).
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/footnote.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let note = word_doc.footnote_paragraph_index("1").expect("footnote");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 5,
                        text: "!".to_string(),
                    },
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: note,
                        end_paragraph: note,
                        style_id: Some("1".to_string()),
                    },
                    HeadlessCommand::InsertText {
                        paragraph: note,
                        offset: 1,
                        text: "、".to_string(),
                    },
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: note,
                        end_paragraph: note,
                        style_id: Some("aa".to_string()),
                    },
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: 0,
                        end_paragraph: 0,
                        style_id: Some("1".to_string()),
                    },
                ],
            },
        )
        .expect("edits apply");
        for (part, root) in [
            ("word/document.xml", "<w:body>"),
            ("word/footnotes.xml", "w:id=\"1\""),
        ] {
            let xml = exported_part(&word_doc, part);
            let story = &xml[xml.find(root).expect("story")..];
            assert!(
                !story.contains("<w:sz ") && !story.contains("<w:rFonts "),
                "{part}: {story}"
            );
        }
        let sizes = |word_doc: &WordDocument| {
            [0, note].map(|index| {
                word_doc
                    .paragraph(index)
                    .expect("paragraph")
                    .runs
                    .iter()
                    .filter(|run| !run.text.is_empty())
                    .map(|run| run.properties.as_ref().and_then(|props| props.font_size))
                    .collect::<Vec<_>>()
            })
        };
        let live = sizes(&word_doc);
        assert!(live[0].iter().all(|size| *size == Some(16.0)), "{live:?}");
        let reloaded = executor::block_on(WordDocument::load(
            &word_doc.export_docx().expect("export succeeds"),
            &Options::default(),
        ))
        .expect("reload");
        assert_eq!(sizes(&reloaded), live);
    }

    #[test]
    fn text_typed_into_an_empty_paragraph_takes_the_theme_fonts_resolved() {
        // A paragraph with no run left to copy — an empty `<w:p/>`, or a cell
        // whose text was all deleted — makes typing build its run from the
        // defaults, whose East Asian face is `w:eastAsiaTheme`. A load
        // resolves that to the theme's face (ＭＳ 明朝 here); typing did not,
        // and the text drew in a fallback face until a reload.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table-complex-pagination.docx"),
        )
        .expect("read fixture");
        let header = "REPEAT_HEADER_0";
        for (paragraph, emptying) in [
            (16, None),
            (
                2,
                Some(HeadlessCommand::ReplaceRange {
                    start_paragraph: 2,
                    start_offset: 0,
                    end_paragraph: 2,
                    end_offset: header.len(),
                    text: String::new(),
                }),
            ),
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            let text = word_doc
                .paragraph(paragraph)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>();
            assert_eq!(text, if emptying.is_some() { header } else { "" });
            let commands = emptying
                .into_iter()
                .chain([HeadlessCommand::InsertText {
                    paragraph,
                    offset: 0,
                    text: "あ".to_string(),
                }])
                .collect();
            apply_headless_commands_to_document(&mut word_doc, &HeadlessEditRequest { commands })
                .expect("edits apply");
            let fonts = |word_doc: &WordDocument| {
                word_doc
                    .paragraph(paragraph)
                    .expect("paragraph")
                    .runs
                    .iter()
                    .map(|run| {
                        run.properties
                            .as_ref()
                            .and_then(|props| props.fonts.clone())
                    })
                    .collect::<Vec<_>>()
            };
            let live = fonts(&word_doc);
            assert!(
                live.iter()
                    .flatten()
                    .any(|fonts| fonts.east_asia.as_deref() == Some("ＭＳ 明朝")),
                "¶{paragraph}: {live:?}"
            );
            let reloaded = executor::block_on(WordDocument::load(
                &word_doc.export_docx().expect("export succeeds"),
                &Options::default(),
            ))
            .expect("reload");
            assert_eq!(fonts(&reloaded), live, "¶{paragraph}");
        }
    }

    #[test]
    fn a_paragraph_style_brings_its_theme_fonts_resolved() {
        // Heading 1 of table-complex-pagination.docx names the theme's major
        // font. Applying it left the reference unresolved in the runs, which
        // drew in their Arial fallback; a reload resolves it to the theme face.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table-complex-pagination.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        let fonts = |word_doc: &WordDocument| {
            word_doc
                .paragraph(19)
                .and_then(|paragraph| paragraph.runs.first())
                .and_then(|run| run.properties.as_ref())
                .and_then(|props| props.fonts.as_ref())
                .map(|fonts| {
                    (
                        fonts.ascii.clone(),
                        fonts.h_ansi.clone(),
                        fonts.east_asia.clone(),
                    )
                })
        };
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetParagraphStyle {
                    start_paragraph: 19,
                    end_paragraph: 19,
                    style_id: Some("Heading1".to_string()),
                }],
            },
        )
        .expect("style applies");
        let live = fonts(&word_doc);
        assert!(live.as_ref().is_some_and(|(ascii, _, _)| ascii.is_some()));
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(fonts(&reloaded), live);
    }

    #[test]
    fn new_table_cells_save_the_formatting_they_copy() {
        // A new cell copies its template cell's formatting — here the header
        // row's shading and Ctrl+0's 12pt before — but was written with only
        // its width and merges, so a reload lost the shading and the spacing.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table-complex-pagination.docx"),
        )
        .expect("read fixture");
        let formatting = |word_doc: &WordDocument| {
            let spacing = (0..word_doc.paragraph_count())
                .map(|index| {
                    word_doc
                        .paragraph(index)
                        .and_then(|paragraph| paragraph.properties.as_ref())
                        .and_then(|props| props.spacing.clone())
                        .and_then(|spacing| spacing.before)
                })
                .collect::<Vec<_>>();
            // The width is the grid's business, compared elsewhere.
            let cells = word_doc
                .table_by_order(0)
                .expect("table")
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|cell| {
                            let mut style = cell.style.clone();
                            style.remove("width");
                            (style, cell.margins.clone(), cell.vertical_align.clone())
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            (spacing, format!("{cells:?}"))
        };
        for insert in [
            HeadlessCommand::InsertTableColumn {
                table_order_index: 0,
                column_index: 4,
                width: None,
                template_column: Some(3),
            },
            HeadlessCommand::InsertTableRow {
                table_order_index: 0,
                row_index: 1,
                template_row: Some(0),
            },
            HeadlessCommand::InsertTableRow {
                table_order_index: 0,
                row_index: 2,
                template_row: Some(1),
            },
            HeadlessCommand::InsertTableCell {
                table_order_index: 0,
                row: 0,
                column: 1,
                direction: HeadlessTableCellInsertDirection::ShiftRight,
            },
            HeadlessCommand::InsertTableCell {
                table_order_index: 0,
                row: 0,
                column: 1,
                direction: HeadlessTableCellInsertDirection::ShiftDown,
            },
            HeadlessCommand::SplitTableCell {
                table_order_index: 0,
                row: 0,
                column: 1,
                columns: 3,
                rows: 2,
            },
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![
                        HeadlessCommand::ToggleParagraphSpaceBefore {
                            start_paragraph: 2,
                            end_paragraph: 40,
                        },
                        insert.clone(),
                    ],
                },
            )
            .expect("commands apply");
            let live = formatting(&word_doc);
            let exported = word_doc.export_docx().expect("export succeeds");
            let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reload");
            assert_eq!(formatting(&reloaded), live, "{insert:?}");
        }
    }

    #[test]
    fn table_column_widths_are_the_whole_twips_a_save_keeps() {
        // `w:gridCol w:w` and `w:tcW` are whole twips. A width set or divided
        // to a fraction of a twip was laid out unrounded and written rounded,
        // so the columns moved a fraction of a point on reload.
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table2x2.docx"),
        )
        .expect("read fixture");
        let widths = |word_doc: &WordDocument| {
            let table = word_doc.table_by_order(0).expect("table");
            let columns = table
                .columns
                .iter()
                .map(|column| column.width)
                .collect::<Vec<_>>();
            // A cell with no width of its own takes its grid columns'.
            let cells = table
                .rows
                .iter()
                .map(|row| {
                    let mut column = row.grid_before.unwrap_or(0) as usize;
                    row.cells
                        .iter()
                        .map(|cell| {
                            let span = cell.span.unwrap_or(1).max(1) as usize;
                            let grid: f32 = table.columns[column..column + span]
                                .iter()
                                .filter_map(|column| column.width)
                                .sum();
                            column += span;
                            cell.style
                                .get("width")
                                .cloned()
                                .unwrap_or_else(|| format!("{grid:.2}pt"))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            (columns, cells)
        };
        for command in [
            HeadlessCommand::SetTableColumnWidths {
                table_order_index: 0,
                widths: vec![100.33, 77.77],
            },
            HeadlessCommand::SplitTableCell {
                table_order_index: 0,
                row: 0,
                column: 0,
                columns: 3,
                rows: 1,
            },
            HeadlessCommand::InsertTableColumn {
                table_order_index: 0,
                column_index: 1,
                width: Some(33.33),
                template_column: Some(0),
            },
        ] {
            let mut word_doc =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
            apply_headless_commands_to_document(
                &mut word_doc,
                &HeadlessEditRequest {
                    commands: vec![command.clone()],
                },
            )
            .expect("command applies");
            let live = widths(&word_doc);
            let exported = word_doc.export_docx().expect("export succeeds");
            let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reload");
            assert_eq!(widths(&reloaded), live, "{command:?}");
        }

        // 行の高さを揃える averages the rows, which is rarely whole twips.
        let heights = |word_doc: &WordDocument| {
            let table = word_doc.table_by_order(0).expect("table");
            table.rows.iter().map(|row| row.height).collect::<Vec<_>>()
        };
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetTableRowHeightProperties {
                        table_order_index: 0,
                        row: 0,
                        mode: "atLeast".to_string(),
                        height: Some(20.0),
                    },
                    HeadlessCommand::SetTableRowHeightProperties {
                        table_order_index: 0,
                        row: 1,
                        mode: "atLeast".to_string(),
                        height: Some(30.05),
                    },
                    HeadlessCommand::DistributeTableRows {
                        table_order_index: 0,
                        height: None,
                    },
                ],
            },
        )
        .expect("commands apply");
        let live = heights(&word_doc);
        let exported = word_doc.export_docx().expect("export succeeds");
        let reloaded =
            executor::block_on(WordDocument::load(&exported, &Options::default())).expect("reload");
        assert_eq!(heights(&reloaded), live);
    }

    #[test]
    fn insert_table_cell_is_one_undo_step_and_survives_export() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/table2x2.docx"),
        )
        .expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: labelled_table_cells(),
            },
        )
        .expect("labels apply");
        let mut executor = CommandExecutor::new();
        let before = table_cell_texts(&word_doc);
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertTableCell {
                    table_order_index: 0,
                    row: 1,
                    column: 0,
                    direction: HeadlessTableCellInsertDirection::ShiftRight,
                }],
            },
        )
        .expect("insert applies");
        assert_eq!(
            table_cell_texts(&word_doc),
            [vec!["A", "B"], vec!["", "C", "D"]]
        );
        let exported = word_doc.export_docx().expect("export succeeds");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut document_xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/document.xml").expect("document part"),
            &mut document_xml,
        )
        .expect("utf-8");
        assert_eq!(document_xml.matches("<w:gridCol ").count(), 3);
        assert!(document_xml.contains("<w:gridAfter w:val=\"1\"/>"));

        let mut selection = None;
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor.undo(&mut ctx).expect("undo succeeds");
        assert_eq!(table_cell_texts(&word_doc), before);
        assert!(!executor.can_undo(), "the insertion was a single undo step");
    }

    #[test]
    fn insert_table_cell_rejects_merged_cells() {
        let mut word_doc = sample_table_document();
        let commands = vec![
            HeadlessCommand::MergeTableCells {
                table_order_index: 0,
                selected_cells: vec![
                    HeadlessTableCellSelection {
                        row: 0,
                        column: 0,
                        column_span: 1,
                    },
                    HeadlessTableCellSelection {
                        row: 0,
                        column: 1,
                        column_span: 1,
                    },
                ],
            },
            HeadlessCommand::InsertTableCell {
                table_order_index: 0,
                row: 0,
                column: 0,
                direction: HeadlessTableCellInsertDirection::ShiftRight,
            },
        ];
        assert!(apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest { commands }
        )
        .is_err());
    }

    #[test]
    fn set_table_cell_text_command_updates_target_cell() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetTableCellText {
                table_order_index: 0,
                row: 1,
                column: 0,
                text: "updated".to_string(),
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows[1].cells[0].blocks.len(), 1);
        let BodyElement::Paragraph(paragraph) = &table.rows[1].cells[0].blocks[0] else {
            panic!("cell must contain paragraph");
        };
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "updated");
    }

    #[test]
    fn fill_table_row_command_updates_logical_cells() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::FillTableRow {
                table_order_index: 0,
                row: 1,
                values: vec!["SKU-1".to_string(), "1200".to_string()],
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        let BodyElement::Paragraph(first) = &table.rows[1].cells[0].blocks[0] else {
            panic!("cell must contain paragraph");
        };
        let BodyElement::Paragraph(second) = &table.rows[1].cells[1].blocks[0] else {
            panic!("cell must contain paragraph");
        };
        assert_eq!(
            first
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "SKU-1"
        );
        assert_eq!(
            second
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "1200"
        );
    }

    #[test]
    fn clone_table_row_command_preserves_shape_and_clears_text_by_default() {
        let make_cell = |text: &str, span: Option<u32>| TableCell {
            blocks: vec![BodyElement::from_paragraph(Paragraph {
                runs: vec![run_from_text(text, None)],
                properties: None,
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            })],
            span,
            ..TableCell::default()
        };
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_table(Table {
                    rows: vec![
                        TableRow {
                            cells: vec![make_cell("A1", Some(2)), make_cell("B1", None)],
                            is_header: true,
                            height: Some(24.0),
                            ..TableRow::default()
                        },
                        TableRow {
                            cells: vec![make_cell("A2", None), make_cell("B2", None)],
                            ..TableRow::default()
                        },
                    ],
                    columns: vec![
                        TableColumn { width: Some(40.0) },
                        TableColumn { width: Some(40.0) },
                    ],
                    ..Table::default()
                })],
            }),
            ..Document::default()
        });
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::CloneTableRow {
                table_order_index: 0,
                source_row: 0,
                target_row: 1,
                clear_text: true,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[1].height, Some(24.0));
        assert!(table.rows[1].is_header);
        assert_eq!(table.rows[1].cells.len(), 2);
        assert_eq!(table.rows[1].cells[0].span, Some(2));
        for cell in &table.rows[1].cells {
            let BodyElement::Paragraph(paragraph) = &cell.blocks[0] else {
                panic!("cell must contain paragraph");
            };
            let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
            assert!(text.is_empty());
        }
    }

    #[test]
    fn insert_and_delete_table_row_column_commands_update_table_shape() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::InsertTableRow {
                    table_order_index: 0,
                    row_index: 1,
                    template_row: None,
                },
                HeadlessCommand::InsertTableColumn {
                    table_order_index: 0,
                    column_index: 1,
                    width: Some(22.0),
                    template_column: None,
                },
                HeadlessCommand::DeleteTableRow {
                    table_order_index: 0,
                    row_index: 0,
                },
                HeadlessCommand::DeleteTableColumn {
                    table_order_index: 0,
                    column_index: 0,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(table.rows[1].cells.len(), 2);
    }

    #[test]
    fn insert_table_column_command_supports_horizontally_merged_cells() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::MergeTableCells {
                    table_order_index: 0,
                    selected_cells: vec![
                        HeadlessTableCellSelection {
                            row: 0,
                            column: 0,
                            column_span: 1,
                        },
                        HeadlessTableCellSelection {
                            row: 0,
                            column: 1,
                            column_span: 1,
                        },
                    ],
                },
                HeadlessCommand::InsertTableColumn {
                    table_order_index: 0,
                    column_index: 1,
                    width: Some(30.0),
                    template_column: None,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.columns.len(), 3);
        assert_eq!(table.rows[0].cells[0].span, Some(3));
    }

    #[test]
    fn insert_table_row_command_supports_vertically_merged_cells() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::MergeTableCells {
                    table_order_index: 0,
                    selected_cells: vec![
                        HeadlessTableCellSelection {
                            row: 0,
                            column: 0,
                            column_span: 1,
                        },
                        HeadlessTableCellSelection {
                            row: 1,
                            column: 0,
                            column_span: 1,
                        },
                    ],
                },
                HeadlessCommand::InsertTableRow {
                    table_order_index: 0,
                    row_index: 1,
                    template_row: None,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows.len(), 3);
        assert_eq!(
            table.rows[1].cells[0].vertical_merge.as_deref(),
            Some("continue")
        );
    }

    #[test]
    fn inserting_below_a_vertical_merge_does_not_extend_the_chain() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::MergeTableCells {
                    table_order_index: 0,
                    selected_cells: vec![
                        HeadlessTableCellSelection {
                            row: 0,
                            column: 0,
                            column_span: 1,
                        },
                        HeadlessTableCellSelection {
                            row: 1,
                            column: 0,
                            column_span: 1,
                        },
                    ],
                },
                HeadlessCommand::InsertTableRow {
                    table_order_index: 0,
                    row_index: 2,
                    template_row: None,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(
            table.rows[0].cells[0].vertical_merge.as_deref(),
            Some("restart")
        );
        assert_eq!(
            table.rows[1].cells[0].vertical_merge.as_deref(),
            Some("continue")
        );
        assert!(table.rows[2].cells[0].vertical_merge.is_none());
    }

    #[test]
    fn deleting_the_only_vertical_merge_follower_clears_the_restart() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::MergeTableCells {
                    table_order_index: 0,
                    selected_cells: vec![
                        HeadlessTableCellSelection {
                            row: 0,
                            column: 0,
                            column_span: 1,
                        },
                        HeadlessTableCellSelection {
                            row: 1,
                            column: 0,
                            column_span: 1,
                        },
                    ],
                },
                HeadlessCommand::DeleteTableRow {
                    table_order_index: 0,
                    row_index: 1,
                },
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows.len(), 1);
        assert!(table.rows[0].cells[0].vertical_merge.is_none());
    }

    #[test]
    fn set_table_width_command_rejects_invalid_spec() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetTableWidth {
                table_order_index: 0,
                width: "invalid-width".to_string(),
            }],
        };

        let err =
            apply_headless_commands_to_document(&mut word_doc, &request).expect_err("must fail");
        let HeadlessApiApplyError::Command { source, .. } = err else {
            panic!("expected command error");
        };
        let message = source.to_string();
        assert!(message.contains("failed to parse table width"));
    }

    #[test]
    fn set_table_column_widths_command_rejects_empty_widths() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::SetTableColumnWidths {
                table_order_index: 0,
                widths: vec![],
            }],
        };

        let err =
            apply_headless_commands_to_document(&mut word_doc, &request).expect_err("must fail");
        let HeadlessApiApplyError::Command { source, .. } = err else {
            panic!("expected command error");
        };
        let message = source.to_string();
        assert!(message.contains("requires at least one width"));
    }

    #[test]
    fn merge_table_cells_command_merges_selected_cells() {
        let mut word_doc = sample_table_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::MergeTableCells {
                table_order_index: 0,
                selected_cells: vec![
                    HeadlessTableCellSelection {
                        row: 0,
                        column: 0,
                        column_span: 1,
                    },
                    HeadlessTableCellSelection {
                        row: 0,
                        column: 1,
                        column_span: 1,
                    },
                ],
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let table = word_doc.table_by_order(0).expect("table exists");
        assert_eq!(table.rows[0].cells.len(), 1);
        assert_eq!(table.rows[0].cells[0].span, Some(2));
    }

    #[test]
    fn set_paragraph_alignment_and_indent_commands_update_properties() {
        let mut word_doc = sample_document("hello");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::SetParagraphAlignment {
                    start_paragraph: 0,
                    end_paragraph: 0,
                    alignment: "center".to_string(),
                },
                HeadlessCommand::SetParagraphIndent {
                    start_paragraph: 0,
                    end_paragraph: 0,
                    left: Some(24.0),
                    right: None,
                    first_line: Some(12.0),
                    hanging: None,
                    left_chars: None,
                    right_chars: None,
                    first_line_chars: None,
                    hanging_chars: None,
                    clear_special: false,
                },
            ],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        let props = paragraph.properties.as_ref().expect("properties");
        assert_eq!(props.alignment.as_deref(), Some("center"));
        let indent = props.indent.as_ref().expect("indent");
        assert_eq!(indent.left, Some(24.0));
        assert_eq!(indent.first_line, Some(12.0));
    }

    #[test]
    fn set_bold_can_disable_bold() {
        let mut word_doc = sample_document("hello");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::SetBold {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    value: true,
                },
                HeadlessCommand::SetBold {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    value: false,
                },
            ],
        };
        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");
        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|props| props.bold),
            Some(false)
        );
    }

    #[test]
    fn start_and_stop_change_tracking_commands_toggle_revision_session() {
        let mut word_doc = sample_document("hello");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::StartChangeTracking {
                    author: Some("LLM".to_string()),
                    initials: None,
                    date: Some("2026-02-24T00:00:00Z".to_string()),
                    date_utc: Some("2026-02-24T00:00:00Z".to_string()),
                },
                HeadlessCommand::StopChangeTracking,
            ],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");
        assert!(word_doc.revision_session().is_none());
    }

    #[test]
    fn accept_revisions_command_keeps_inserted_and_drops_deleted() {
        let mut word_doc = sample_revision_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");
        assert_eq!(paragraph_text(&word_doc, 0), "Hello ");

        let query = HeadlessQueryRequest {
            queries: vec![HeadlessQuery::Revisions {
                start_revision: 0,
                max_revisions: 10,
                kind: None,
                author: None,
                date_from: None,
                date_to: None,
            }],
        };
        let response =
            execute_headless_queries_on_document(&word_doc, &query).expect("query succeeds");
        let HeadlessQueryResult::Revisions {
            total_revisions, ..
        } = &response.results[0]
        else {
            panic!("expected revisions result")
        };
        assert_eq!(*total_revisions, 0);
    }

    #[test]
    fn accept_revisions_rebases_comment_anchors_after_removed_deletion() {
        let mut word_doc = sample_revision_comment_document(RevisionKind::Deleted);
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        assert_eq!(paragraph_text(&word_doc, 0), "llo");
        assert_eq!(paragraph.comment_ranges[0].start, 0);
        assert_eq!(paragraph.comment_ranges[0].end, 3);
        assert_eq!(paragraph.comment_markers[0].position, 3);
    }

    #[test]
    fn reject_revisions_command_keeps_deleted_and_drops_inserted() {
        let mut word_doc = sample_revision_document();
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");
        assert_eq!(paragraph_text(&word_doc, 0), "World");
    }

    #[test]
    fn reject_revisions_rebases_comment_anchors_after_removed_insertion() {
        let mut word_doc = sample_revision_comment_document(RevisionKind::Inserted);
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }],
        };

        apply_headless_commands_to_document(&mut word_doc, &request).expect("apply succeeds");

        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        assert_eq!(paragraph_text(&word_doc, 0), "llo");
        assert_eq!(paragraph.comment_ranges[0].start, 0);
        assert_eq!(paragraph.comment_ranges[0].end, 3);
        assert_eq!(paragraph.comment_markers[0].position, 3);
    }

    #[test]
    fn add_comment_fails_without_raw_document() {
        let mut word_doc = sample_document("hello");
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddComment {
                start_paragraph: 0,
                start_offset: Some(0),
                end_paragraph: 0,
                end_offset: Some(5),
                text: "note".to_string(),
                author: Some("LLM".to_string()),
                initials: None,
                date: None,
            }],
        };

        let err = apply_headless_commands_to_document(&mut word_doc, &request)
            .expect_err("add_comment should fail without raw xml");
        assert!(matches!(
            err,
            HeadlessApiApplyError::Command { index: 0, .. }
        ));
    }

    #[test]
    fn header_footer_commands_fail_when_part_is_missing() {
        let mut word_doc = sample_document("hello");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::SetHeaderParagraphText {
                    paragraph: 0,
                    text: "header".to_string(),
                },
                HeadlessCommand::SetFooterParagraphText {
                    paragraph: 0,
                    text: "footer".to_string(),
                },
            ],
        };

        let err =
            apply_headless_commands_to_document(&mut word_doc, &request).expect_err("must fail");
        assert!(matches!(
            err,
            HeadlessApiApplyError::Command { index: 0, .. }
        ));
    }

    #[test]
    fn snapshot_coordinates_rebase_positions_after_a_multi_paragraph_replacement() {
        let mut word_doc = sample_document_from_paragraphs(&[
            "ab".to_string(),
            "cd".to_string(),
            "ef".to_string(),
        ]);
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 0,
                    start_offset: 1,
                    end_paragraph: 1,
                    end_offset: 1,
                    text: "X".to_string(),
                },
                HeadlessCommand::InsertText {
                    paragraph: 2,
                    offset: 1,
                    text: "Y".to_string(),
                },
                HeadlessCommand::InsertText {
                    paragraph: 1,
                    offset: 2,
                    text: "Z".to_string(),
                },
            ],
        };
        let mut executor = CommandExecutor::new();

        apply_snapshot_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &request,
        )
        .expect("non-overlapping snapshot edits");

        assert_eq!(paragraph_texts(&word_doc), ["aXdZ", "eYf"]);
    }

    #[test]
    fn snapshot_coordinates_allow_overlapping_text_range_formatting() {
        let mut word_doc = sample_document("abcdef");
        let request = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::ApplyBold {
                    start_paragraph: 0,
                    start_offset: 1,
                    end_paragraph: 0,
                    end_offset: 5,
                    enabled: true,
                },
                HeadlessCommand::ApplyItalic {
                    start_paragraph: 0,
                    start_offset: 2,
                    end_paragraph: 0,
                    end_offset: 4,
                    enabled: true,
                },
            ],
        };
        let mut executor = CommandExecutor::new();

        apply_snapshot_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &request,
        )
        .expect("formatting ranges may intentionally overlap");

        let paragraph = word_doc.paragraph(0).expect("paragraph exists");
        let mut cursor = 0usize;
        for run in &paragraph.runs {
            let end = cursor + run.text.len();
            let properties = run.properties.as_ref();
            if cursor >= 1 && end <= 5 {
                assert_eq!(properties.and_then(|props| props.bold), Some(true));
            }
            if cursor >= 2 && end <= 4 {
                assert_eq!(properties.and_then(|props| props.italic), Some(true));
            }
            cursor = end;
        }
    }

    // Property test: for single-paragraph ASCII edits, headless commands should
    // produce the same final text as a simple in-memory string model.
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            max_shrink_iters: 2000,
            .. ProptestConfig::default()
        })]

        #[test]
        fn property_snapshot_text_edits_match_a_back_to_front_string_model(
            initial in "[a-z]{1,48}",
            raw_ops in prop::collection::vec((0u8..4, any::<u8>(), any::<u8>(), any::<u8>()), 1..40),
        ) {
            #[derive(Clone)]
            struct ModelEdit {
                start: usize,
                end: usize,
                replacement: String,
                command: HeadlessCommand,
            }

            let len = initial.len();
            let mut edits: Vec<ModelEdit> = Vec::new();
            for (kind, a, b, value) in raw_ops {
                let candidate = if kind == 0 {
                    let at = usize::from(a) % (len + 1);
                    let replacement = char::from(b'A' + (value % 26)).to_string();
                    ModelEdit {
                        start: at,
                        end: at,
                        replacement: replacement.clone(),
                        command: HeadlessCommand::InsertText {
                            paragraph: 0,
                            offset: at,
                            text: replacement,
                        },
                    }
                } else {
                    let start = usize::from(a) % len;
                    let width = 1 + (usize::from(b) % (len - start));
                    let end = start + width;
                    let replacement = if kind == 1 {
                        char::from(b'A' + (value % 26)).to_string()
                    } else {
                        String::new()
                    };
                    let command = if kind == 3 {
                        HeadlessCommand::DeleteText {
                            paragraph: 0,
                            offset: start,
                            direction: HeadlessDeleteDirection::Forward,
                            count: width,
                        }
                    } else {
                        HeadlessCommand::ReplaceRange {
                            start_paragraph: 0,
                            start_offset: start,
                            end_paragraph: 0,
                            end_offset: end,
                            text: replacement.clone(),
                        }
                    };
                    ModelEdit {
                        start,
                        end,
                        replacement,
                        command,
                    }
                };

                let overlaps = edits.iter().any(|existing| {
                    if candidate.start == candidate.end && existing.start == existing.end {
                        candidate.start == existing.start
                    } else if candidate.start == candidate.end {
                        existing.start <= candidate.start && candidate.start <= existing.end
                    } else if existing.start == existing.end {
                        candidate.start <= existing.start && existing.start <= candidate.end
                    } else {
                        candidate.start < existing.end && existing.start < candidate.end
                    }
                });
                if !overlaps {
                    edits.push(candidate);
                }
            }
            prop_assume!(!edits.is_empty());

            let mut expected = initial.clone();
            let mut model_order = edits.clone();
            model_order.sort_by_key(|edit| std::cmp::Reverse(edit.start));
            for edit in model_order {
                expected.replace_range(edit.start..edit.end, &edit.replacement);
            }

            let mut word_doc = sample_document(&initial);
            let mut executor = CommandExecutor::new();
            let request = HeadlessEditRequest {
                commands: edits.into_iter().map(|edit| edit.command).collect(),
            };
            apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &request,
            )
            .expect("non-overlapping snapshot edits should apply");

            prop_assert_eq!(paragraph_text(&word_doc, 0), expected);
        }

        #[test]
        fn property_revision_decisions_rebase_comment_anchors_across_run_boundaries(
            mut run_specs in prop::collection::vec(("[a-z]{1,8}", 0u8..3), 1..12),
            raw_start in any::<usize>(),
            raw_end in any::<usize>(),
            raw_marker in any::<usize>(),
            accept in any::<bool>(),
            filter_target in any::<bool>(),
        ) {
            // Run classes: 0 is plain, 1 is the targeted revision, and 2 is
            // another revision that survives when the id filter is active.
            let has_removable_revision = run_specs.iter().any(|(_, class)| {
                *class != 0 && (!filter_target || *class == 1)
            });
            if !has_removable_revision {
                run_specs[0].1 = 1;
            }

            let total_len = run_specs.iter().map(|(text, _)| text.len()).sum::<usize>();
            let first = raw_start % (total_len + 1);
            let second = raw_end % (total_len + 1);
            let range_start = first.min(second);
            let range_end = first.max(second);
            let marker_position = raw_marker % (total_len + 1);
            let revision_kind = if accept {
                RevisionKind::Deleted
            } else {
                RevisionKind::Inserted
            };

            let mut runs = Vec::with_capacity(run_specs.len());
            let mut kept_bytes = Vec::with_capacity(total_len);
            let mut expected_text = String::new();
            let mut expected_changed_runs = 0usize;
            for (text, class) in &run_specs {
                let removed = *class != 0 && (!filter_target || *class == 1);
                kept_bytes.extend(std::iter::repeat_n(!removed, text.len()));
                if removed {
                    expected_changed_runs += 1;
                } else {
                    expected_text.push_str(text);
                }

                let mut run = run_from_text(text, None);
                if *class != 0 {
                    run.revision = Some(RevisionMark {
                        kind: revision_kind.clone(),
                        id: Some(if *class == 1 { "target" } else { "other" }.to_string()),
                        author: Some("LLM".to_string()),
                        date: Some("2026-08-08T00:00:00Z".to_string()),
                        date_utc: None,
                        insertion: None,
                    });
                }
                runs.push(run);
            }

            let expected_position = |position: usize| {
                kept_bytes[..position].iter().filter(|keep| **keep).count()
            };
            let mut word_doc = WordDocument::from_document(Document {
                body: Some(Body {
                    elements: vec![BodyElement::from_paragraph(Paragraph {
                        runs,
                        properties: None,
                        node_id: None,
                        comment_ranges: vec![CommentRange {
                            id: "comment".to_string(),
                            start: range_start,
                            end: range_end,
                        }],
                        comment_markers: vec![CommentMarker {
                            id: "comment".to_string(),
                            position: marker_position,
                        }],
                    })],
                }),
                ..Document::default()
            });
            let revision_id = filter_target.then_some("target");

            let changed_runs = if accept {
                word_doc.accept_revisions(None, None, None, revision_id)
            } else {
                word_doc.reject_revisions(None, None, None, revision_id)
            }
            .expect("revision decision should apply");

            prop_assert_eq!(changed_runs, expected_changed_runs);
            prop_assert_eq!(paragraph_text(&word_doc, 0), expected_text);
            let paragraph = word_doc.paragraph(0).expect("paragraph exists");
            prop_assert_eq!(paragraph.comment_ranges[0].start, expected_position(range_start));
            prop_assert_eq!(paragraph.comment_ranges[0].end, expected_position(range_end));
            prop_assert_eq!(
                paragraph.comment_markers[0].position,
                expected_position(marker_position)
            );
        }

        #[test]
        fn property_snapshot_delete_across_paragraphs_matches_model_and_history(
            paragraphs in prop::collection::vec("[a-z]{0,8}", 2..8),
            raw_first in any::<usize>(),
            raw_second in any::<usize>(),
            raw_start_offset in any::<usize>(),
            raw_end_offset in any::<usize>(),
            raw_insert in any::<usize>(),
            raw_insert_side in any::<bool>(),
            backward in any::<bool>(),
            reverse_input in any::<bool>(),
            inserted in "[A-Z]{1,4}",
        ) {
            let first = raw_first % (paragraphs.len() - 1);
            let second = first + 1 + raw_second % (paragraphs.len() - first - 1);
            let start_offset = raw_start_offset % (paragraphs[first].len() + 1);
            let end_offset = raw_end_offset % (paragraphs[second].len() + 1);

            let paragraph_starts = paragraphs
                .iter()
                .scan(0usize, |cursor, paragraph| {
                    let start = *cursor;
                    *cursor += paragraph.len() + 1;
                    Some(start)
                })
                .collect::<Vec<_>>();
            let flat = paragraphs.join("\n");
            let delete_start = paragraph_starts[first] + start_offset;
            let delete_end = paragraph_starts[second] + end_offset;
            let count = delete_end - delete_start;
            prop_assert!(count > 0);
            prop_assume!(delete_start > 0 || delete_end < flat.len());

            let use_before = if delete_start == 0 {
                false
            } else if delete_end == flat.len() {
                true
            } else {
                raw_insert_side
            };
            let insert_global = if use_before {
                raw_insert % delete_start
            } else {
                delete_end + 1 + raw_insert % (flat.len() - delete_end)
            };

            let global_to_position = |global: usize| {
                for (paragraph, text) in paragraphs.iter().enumerate() {
                    let start = paragraph_starts[paragraph];
                    if global <= start + text.len() {
                        return (paragraph, global - start);
                    }
                }
                let paragraph = paragraphs.len() - 1;
                (paragraph, paragraphs[paragraph].len())
            };
            let (insert_paragraph, insert_offset) = global_to_position(insert_global);
            let delete = if backward {
                HeadlessCommand::DeleteText {
                    paragraph: second,
                    offset: end_offset,
                    direction: HeadlessDeleteDirection::Backward,
                    count,
                }
            } else {
                HeadlessCommand::DeleteText {
                    paragraph: first,
                    offset: start_offset,
                    direction: HeadlessDeleteDirection::Forward,
                    count,
                }
            };
            let insert = HeadlessCommand::InsertText {
                paragraph: insert_paragraph,
                offset: insert_offset,
                text: inserted.clone(),
            };
            let commands = if reverse_input {
                vec![insert, delete]
            } else {
                vec![delete, insert]
            };

            let mut expected = flat.clone();
            if insert_global > delete_end {
                expected.insert_str(insert_global, &inserted);
                expected.replace_range(delete_start..delete_end, "");
            } else {
                expected.replace_range(delete_start..delete_end, "");
                expected.insert_str(insert_global, &inserted);
            }
            let expected = expected.split('\n').map(str::to_string).collect::<Vec<_>>();

            let mut word_doc = sample_document_from_paragraphs(&paragraphs);
            let mut command_executor = CommandExecutor::new();
            let mut selection = apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut command_executor,
                None,
                &HeadlessEditRequest { commands },
            )
            .expect("non-overlapping cross-paragraph delete and insert");
            prop_assert_eq!(paragraph_texts(&word_doc), expected.clone());

            undo_all(&mut word_doc, &mut command_executor, &mut selection);
            prop_assert_eq!(paragraph_texts(&word_doc), paragraphs.clone());

            while command_executor.can_redo() {
                let mut context = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(command_executor.redo(&mut context).expect("redo succeeds"));
            }
            prop_assert_eq!(paragraph_texts(&word_doc), expected);
        }

        #[test]
        fn property_snapshot_edit_crossing_arbitrary_runs_matches_model_and_undoes_exactly(
            initial in "[a-z]{4,48}",
            raw_split in any::<u8>(),
            raw_start in any::<u8>(),
            raw_end in any::<u8>(),
            raw_extra_splits in prop::collection::vec(any::<u8>(), 0..12),
            replacement in "[A-Z]{0,5}",
            delete_instead in any::<bool>(),
        ) {
            let len = initial.len();
            let split = 2 + usize::from(raw_split) % (len - 2);
            let start = 1 + usize::from(raw_start) % (split - 1);
            let end = split + 1 + usize::from(raw_end) % (len - split);
            let mut splits = raw_extra_splits
                .into_iter()
                .map(|offset| usize::from(offset) % (len + 1))
                .collect::<Vec<_>>();
            splits.push(split);

            let mut word_doc = sample_document_with_styled_runs(&initial, &splits);
            let original_runs = run_signatures(&word_doc, 0);
            let replacement = if delete_instead {
                String::new()
            } else {
                replacement
            };
            let edit = if delete_instead {
                HeadlessCommand::DeleteText {
                    paragraph: 0,
                    offset: start,
                    direction: HeadlessDeleteDirection::Forward,
                    count: end - start,
                }
            } else {
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 0,
                    start_offset: start,
                    end_paragraph: 0,
                    end_offset: end,
                    text: replacement.clone(),
                }
            };
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 0,
                        text: "Q".to_string(),
                    },
                    edit,
                ],
            };
            let mut executor = CommandExecutor::new();

            let mut selection = apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &request,
            )
            .expect("an edit crossing a run boundary should apply");

            let mut expected = initial.clone();
            expected.replace_range(start..end, &replacement);
            expected.insert(0, 'Q');
            prop_assert_eq!(paragraph_text(&word_doc, 0), expected);

            undo_all(&mut word_doc, &mut executor, &mut selection);
            prop_assert_eq!(run_signatures(&word_doc, 0), original_runs);
        }

        #[test]
        fn property_snapshot_overlapping_formatting_crosses_arbitrary_runs_without_style_leaks(
            initial in "[a-z]{4,48}",
            raw_split in any::<u8>(),
            raw_bold_start in any::<u8>(),
            raw_bold_end in any::<u8>(),
            raw_italic_start in any::<u8>(),
            raw_italic_end in any::<u8>(),
            raw_extra_splits in prop::collection::vec(any::<u8>(), 0..12),
        ) {
            let len = initial.len();
            let split = 2 + usize::from(raw_split) % (len - 2);
            let bold_start = 1 + usize::from(raw_bold_start) % (split - 1);
            let bold_end = split + 1 + usize::from(raw_bold_end) % (len - split);
            let italic_start = 1 + usize::from(raw_italic_start) % (split - 1);
            let italic_end = split + 1 + usize::from(raw_italic_end) % (len - split);
            let mut splits = raw_extra_splits
                .into_iter()
                .map(|offset| usize::from(offset) % (len + 1))
                .collect::<Vec<_>>();
            splits.push(split);

            let mut word_doc = sample_document_with_styled_runs(&initial, &splits);
            let original_runs = run_signatures(&word_doc, 0);
            let original_properties = run_properties_by_byte(&word_doc, 0);
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetBold {
                        start_paragraph: 0,
                        start_offset: bold_start,
                        end_paragraph: 0,
                        end_offset: bold_end,
                        value: true,
                    },
                    HeadlessCommand::ApplyItalic {
                        start_paragraph: 0,
                        start_offset: italic_start,
                        end_paragraph: 0,
                        end_offset: italic_end,
                        enabled: true,
                    },
                ],
            };
            let mut executor = CommandExecutor::new();

            let mut selection = apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &request,
            )
            .expect("overlapping formatting may cross run boundaries");

            prop_assert_eq!(paragraph_text(&word_doc, 0), initial);
            let actual_properties = run_properties_by_byte(&word_doc, 0);
            prop_assert_eq!(actual_properties.len(), original_properties.len());
            for (offset, (actual, original)) in actual_properties
                .iter()
                .zip(original_properties.iter())
                .enumerate()
            {
                let expected_bold = if (bold_start..bold_end).contains(&offset) {
                    Some(true)
                } else {
                    original.bold
                };
                let expected_italic = if (italic_start..italic_end).contains(&offset) {
                    Some(true)
                } else {
                    original.italic
                };
                prop_assert_eq!(actual.bold, expected_bold, "bold at byte {}", offset);
                prop_assert_eq!(actual.italic, expected_italic, "italic at byte {}", offset);
                prop_assert_eq!(actual.font_size, original.font_size, "font size at byte {}", offset);
            }

            undo_all(&mut word_doc, &mut executor, &mut selection);
            prop_assert_eq!(run_signatures(&word_doc, 0), original_runs);
        }

        #[test]
        fn property_every_snapshot_range_formatter_rebases_across_runs(
            initial in "[a-z]{4,48}",
            raw_split in any::<u8>(),
            raw_start in any::<u8>(),
            raw_end in any::<u8>(),
            raw_extra_splits in prop::collection::vec(any::<u8>(), 0..12),
        ) {
            let len = initial.len();
            let split = 2 + usize::from(raw_split) % (len - 2);
            let start = 1 + usize::from(raw_start) % (split - 1);
            let end = split + 1 + usize::from(raw_end) % (len - split);
            let mut splits = raw_extra_splits
                .into_iter()
                .map(|offset| usize::from(offset) % (len + 1))
                .collect::<Vec<_>>();
            splits.push(split);

            for formatter in 0u8..13 {
            let mut word_doc = sample_document_with_styled_runs(&initial, &splits);
            let original_runs = run_signatures(&word_doc, 0);
            let original_properties = run_properties_by_byte(&word_doc, 0);
            let command = match formatter {
                0 => HeadlessCommand::ApplyBold {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    enabled: true,
                },
                1 => HeadlessCommand::ApplyItalic {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    enabled: true,
                },
                2 => HeadlessCommand::ApplyUnderline {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    kind: None,
                    enabled: true,
                },
                3 => HeadlessCommand::ApplyStrike {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    double: false,
                    enabled: true,
                },
                4 => HeadlessCommand::ApplySuperscript {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    enabled: true,
                },
                5 => HeadlessCommand::ApplySubscript {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    enabled: true,
                },
                6 => HeadlessCommand::SetFontSize {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end, font_size: 19.0,
                },
                7 => HeadlessCommand::SetFontFamily {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    font_family: "Courier New".to_string(),
                },
                8 => HeadlessCommand::SetBold {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end, value: false,
                },
                9 => HeadlessCommand::SetCharacterStyle {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end, style_id: None,
                },
                10 => HeadlessCommand::SetFontColor {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    color: "#112233".to_string(),
                },
                11 => HeadlessCommand::SetHighlightColor {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                    color: "#FFFF00".to_string(),
                },
                _ => HeadlessCommand::ClearFormatting {
                    start_paragraph: 0, start_offset: start,
                    end_paragraph: 0, end_offset: end,
                },
            };
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 0,
                        text: "Q".to_string(),
                    },
                    command,
                ],
            };
            let mut executor = CommandExecutor::new();

            let mut selection = apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &request,
            )
            .expect("every supported formatter should rebase across runs");

            prop_assert_eq!(paragraph_text(&word_doc, 0), format!("Q{initial}"));
            let actual_properties = run_properties_by_byte(&word_doc, 0);
            for (original_offset, original) in original_properties.iter().enumerate() {
                let actual = &actual_properties[original_offset + 1];
                let selected = (start..end).contains(&original_offset);
                match formatter {
                    0 => prop_assert_eq!(actual.bold, selected.then_some(true).or(original.bold)),
                    1 => prop_assert_eq!(actual.italic, selected.then_some(true).or(original.italic)),
                    2 => prop_assert_eq!(actual.underline, selected.then_some(true).or(original.underline)),
                    3 => prop_assert_eq!(actual.strike, selected.then_some(true).or(original.strike)),
                    4 => prop_assert_eq!(
                        actual.baseline.as_deref(),
                        if selected { Some("superscript") } else { original.baseline.as_deref() },
                    ),
                    5 => prop_assert_eq!(
                        actual.baseline.as_deref(),
                        if selected { Some("subscript") } else { original.baseline.as_deref() },
                    ),
                    6 => prop_assert_eq!(actual.font_size, if selected { Some(19.0) } else { original.font_size }),
                    7 => prop_assert_eq!(
                        actual.font_family.as_deref(),
                        if selected { Some("Courier New") } else { original.font_family.as_deref() },
                    ),
                    8 => prop_assert_eq!(actual.bold, if selected { Some(false) } else { original.bold }),
                    9 => prop_assert_eq!(
                        actual.style.as_deref(),
                        if selected { None } else { original.style.as_deref() },
                    ),
                    10 => prop_assert_eq!(
                        actual.color.as_deref(),
                        if selected { Some("#112233") } else { original.color.as_deref() },
                    ),
                    11 => prop_assert_eq!(
                        actual.highlight.as_deref(),
                        if selected { Some("#FFFF00") } else { original.highlight.as_deref() },
                    ),
                    _ => {
                        prop_assert_eq!(actual.style.as_deref(), if selected { None } else { original.style.as_deref() });
                        prop_assert_eq!(actual.bold, if selected { None } else { original.bold });
                        prop_assert_eq!(actual.italic, if selected { None } else { original.italic });
                        prop_assert_eq!(actual.font_size, if selected { None } else { original.font_size });
                    }
                }
                if formatter != 6 && formatter != 12 {
                    prop_assert_eq!(actual.font_size, original.font_size);
                }
                if formatter != 0 && formatter != 8 && formatter != 12 {
                    prop_assert_eq!(actual.bold, original.bold);
                }
                if formatter != 1 && formatter != 12 {
                    prop_assert_eq!(actual.italic, original.italic);
                }
            }

            undo_all(&mut word_doc, &mut executor, &mut selection);
            prop_assert_eq!(run_signatures(&word_doc, 0), original_runs);
            }
        }

        #[test]
        fn property_snapshot_multibyte_edit_rebases_across_run_boundaries(
            characters in prop::collection::vec(
                prop_oneof![Just('a'), Just('あ'), Just('🙂')],
                4..24,
            ),
            raw_split in any::<u8>(),
            raw_start in any::<u8>(),
            raw_end in any::<u8>(),
        ) {
            let character_count = characters.len();
            let split_character = 2 + usize::from(raw_split) % (character_count - 2);
            let start_character =
                1 + usize::from(raw_start) % (split_character - 1);
            let end_character = split_character
                + 1
                + usize::from(raw_end) % (character_count - split_character);
            let initial = characters.iter().collect::<String>();
            let mut boundaries = initial.char_indices().map(|(offset, _)| offset).collect::<Vec<_>>();
            boundaries.push(initial.len());
            let start = boundaries[start_character];
            let split = boundaries[split_character];
            let end = boundaries[end_character];

            let mut word_doc = sample_document_with_styled_runs(&initial, &[split]);
            let original_runs = run_signatures(&word_doc, 0);
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 0,
                        text: "先".to_string(),
                    },
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: 0,
                        start_offset: start,
                        end_paragraph: 0,
                        end_offset: end,
                        text: "置換".to_string(),
                    },
                ],
            };
            let mut executor = CommandExecutor::new();

            let mut selection = apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &request,
            )
            .expect("UTF-8 byte coordinates should rebase across run boundaries");

            let mut expected = initial.clone();
            expected.replace_range(start..end, "置換");
            expected.insert(0, '先');
            prop_assert_eq!(paragraph_text(&word_doc, 0), expected);

            undo_all(&mut word_doc, &mut executor, &mut selection);
            prop_assert_eq!(run_signatures(&word_doc, 0), original_runs);
        }

        #[test]
        fn property_snapshot_paragraph_breaks_ignore_input_order(
            initial in "[a-z]{1,48}",
            raw_breaks in prop::collection::vec((any::<u8>(), 1usize..=3), 1..24),
        ) {
            let len = initial.len();
            let mut breaks = Vec::new();
            for (raw_position, count) in raw_breaks {
                let position = usize::from(raw_position) % (len + 1);
                if breaks.iter().all(|(existing, _)| *existing != position) {
                    breaks.push((position, count));
                }
            }
            prop_assume!(!breaks.is_empty());

            let commands = breaks
                .iter()
                .map(|(offset, count)| HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: *offset,
                    count: *count,
                })
                .collect();
            let mut word_doc = sample_document(&initial);
            let mut executor = CommandExecutor::new();
            apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &HeadlessEditRequest { commands },
            )
            .expect("distinct snapshot paragraph breaks should apply");

            breaks.sort_by_key(|(position, _)| *position);
            let mut expected = Vec::new();
            let mut cursor = 0usize;
            for (position, count) in breaks {
                expected.push(initial[cursor..position].to_string());
                expected.extend((1..count).map(|_| String::new()));
                cursor = position;
            }
            expected.push(initial[cursor..].to_string());

            prop_assert_eq!(paragraph_texts(&word_doc), expected);
        }

        #[test]
        fn property_snapshot_overlap_is_rejected_without_mutation(
            initial in "[a-z]{2,48}",
            raw_start in any::<u8>(),
            raw_width in any::<u8>(),
            raw_inner in any::<u8>(),
        ) {
            let start = usize::from(raw_start) % initial.len();
            let width = 1 + (usize::from(raw_width) % (initial.len() - start));
            let end = start + width;
            let inside = start + (usize::from(raw_inner) % (width + 1));
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: 0,
                        start_offset: start,
                        end_paragraph: 0,
                        end_offset: end,
                        text: String::new(),
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: inside,
                        text: "X".to_string(),
                    },
                ],
            };
            let mut word_doc = sample_document(&initial);
            let mut executor = CommandExecutor::new();
            let error = apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &request,
            )
            .expect_err("an insertion on or inside a replaced range is ambiguous");

            let error_index = match error {
                HeadlessApiApplyError::Command { index, .. } => Some(index),
                HeadlessApiApplyError::EmptyCommands => None,
            };
            prop_assert_eq!(error_index, Some(1));
            prop_assert_eq!(paragraph_text(&word_doc, 0), initial);
            prop_assert!(!executor.can_undo());
        }

        #[test]
        fn property_snapshot_formatting_tracks_original_text_through_insertions(
            initial in "[a-z]{4,48}",
            raw_start in any::<u8>(),
            raw_end in any::<u8>(),
            raw_insertions in prop::collection::vec((any::<u8>(), any::<u8>()), 0..24),
            raw_format_index in any::<u8>(),
        ) {
            let first = usize::from(raw_start) % initial.len();
            let second = usize::from(raw_end) % initial.len();
            let start = first.min(second);
            let end = first.max(second) + 1;

            let mut insertions = Vec::new();
            for (raw_position, raw_character) in raw_insertions {
                let position = usize::from(raw_position) % (initial.len() + 1);
                if position > start
                    && position < end
                    || position == start
                    || position == end
                    || insertions
                        .iter()
                        .any(|(existing, _): &(usize, char)| *existing == position)
                {
                    continue;
                }
                insertions.push((position, char::from(b'A' + (raw_character % 26))));
            }

            let mut commands = insertions
                .iter()
                .map(|(position, character)| HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: *position,
                    text: character.to_string(),
                })
                .collect::<Vec<_>>();
            let format_index = usize::from(raw_format_index) % (commands.len() + 1);
            commands.insert(
                format_index,
                HeadlessCommand::SetBold {
                    start_paragraph: 0,
                    start_offset: start,
                    end_paragraph: 0,
                    end_offset: end,
                    value: true,
                },
            );

            let mut word_doc = sample_document(&initial);
            let mut executor = CommandExecutor::new();
            apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &HeadlessEditRequest { commands },
            )
            .expect("insertions outside a formatting range are unambiguous");

            let mut expected_text = initial.clone();
            let mut model_insertions = insertions.clone();
            model_insertions.sort_by_key(|(position, _)| std::cmp::Reverse(*position));
            for (position, character) in model_insertions {
                expected_text.insert(position, character);
            }
            let shift = insertions
                .iter()
                .filter(|(position, _)| *position < start)
                .count();
            let expected_bold = (start + shift)..(end + shift);

            let paragraph = word_doc.paragraph(0).expect("paragraph exists");
            let mut actual_text = String::new();
            let mut actual_bold = Vec::new();
            for run in &paragraph.runs {
                actual_text.push_str(&run.text);
                actual_bold.extend(
                    std::iter::repeat_n(
                        run.properties
                            .as_ref()
                            .and_then(|properties| properties.bold)
                            == Some(true),
                        run.text.len(),
                    ),
                );
            }

            prop_assert_eq!(actual_text, expected_text);
            for (offset, is_bold) in actual_bold.into_iter().enumerate() {
                prop_assert_eq!(
                    is_bold,
                    expected_bold.contains(&offset),
                    "offset {}",
                    offset
                );
            }
        }

        #[test]
        fn property_snapshot_rejects_text_replacement_overlapping_formatting(
            initial in "[a-z]{2,48}",
            raw_format_start in any::<u8>(),
            raw_format_width in any::<u8>(),
            raw_replace_start in any::<u8>(),
            raw_replace_width in any::<u8>(),
        ) {
            let format_start = usize::from(raw_format_start) % initial.len();
            let format_width =
                1 + usize::from(raw_format_width) % (initial.len() - format_start);
            let format_end = format_start + format_width;
            let replace_start = format_start
                + usize::from(raw_replace_start) % (format_end - format_start);
            let replace_width =
                1 + usize::from(raw_replace_width) % (format_end - replace_start);
            let replace_end = replace_start + replace_width;
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::SetBold {
                        start_paragraph: 0,
                        start_offset: format_start,
                        end_paragraph: 0,
                        end_offset: format_end,
                        value: true,
                    },
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: 0,
                        start_offset: replace_start,
                        end_paragraph: 0,
                        end_offset: replace_end,
                        text: "X".to_string(),
                    },
                ],
            };
            let mut word_doc = sample_document(&initial);
            let mut executor = CommandExecutor::new();

            let error = apply_snapshot_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &request,
            )
            .expect_err("formatting text that another command replaces is ambiguous");

            let error_index = match error {
                HeadlessApiApplyError::Command { index, .. } => Some(index),
                HeadlessApiApplyError::EmptyCommands => None,
            };
            prop_assert_eq!(error_index, Some(1));
            prop_assert_eq!(paragraph_text(&word_doc, 0), initial);
            prop_assert!(!executor.can_undo());
        }

        #[test]
        fn property_single_paragraph_text_edits_match_model(
            ops in prop::collection::vec((0u8..4, any::<u8>(), any::<u8>()), 1..80)
        ) {
            let mut word_doc = sample_document("");
            let mut model = String::new();

            for (kind, a, b) in ops {
                let len = model.len();
                match kind {
                    0 => {
                        let pos = usize::from(a) % (len + 1);
                        let ch = char::from(b'a' + (b % 26));
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::InsertText {
                                paragraph: 0,
                                offset: pos,
                                text: ch.to_string(),
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("insert should succeed");
                        model.insert(pos, ch);
                    }
                    1 => {
                        let pos = usize::from(a) % (len + 1);
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::DeleteText {
                                paragraph: 0,
                                offset: pos,
                                direction: HeadlessDeleteDirection::Backward,
                                count: 1,
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("backward delete should succeed");
                        if pos > 0 && len > 0 {
                            model.remove(pos - 1);
                        }
                    }
                    2 => {
                        let pos = usize::from(a) % (len + 1);
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::DeleteText {
                                paragraph: 0,
                                offset: pos,
                                direction: HeadlessDeleteDirection::Forward,
                                count: 1,
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("forward delete should succeed");
                        if pos < len {
                            model.remove(pos);
                        }
                    }
                    _ => {
                        let start = usize::from(a) % (len + 1);
                        let end = usize::from(b) % (len + 1);
                        let (start, end) = if start <= end {
                            (start, end)
                        } else {
                            (end, start)
                        };
                        let repl = char::from(b'a' + ((a.wrapping_add(b)) % 26)).to_string();
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::ReplaceRange {
                                start_paragraph: 0,
                                start_offset: start,
                                end_paragraph: 0,
                                end_offset: end,
                                text: repl.clone(),
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("replace should succeed");
                        model.replace_range(start..end, &repl);
                    }
                }

                prop_assert_eq!(paragraph_text(&word_doc, 0), model.as_str());
            }
        }

        #[test]
        fn property_change_case_crosses_arbitrary_run_boundaries_without_format_loss(
            initial in "[A-Za-z .!?]{4,48}",
            raw_start in any::<u8>(),
            raw_end in any::<u8>(),
            raw_mode in any::<u8>(),
            raw_splits in prop::collection::vec(any::<u8>(), 0..14),
        ) {
            let len = initial.len();
            let first = usize::from(raw_start) % len;
            let second = 1 + usize::from(raw_end) % len;
            let (start, end) = if first < second {
                (first, second)
            } else if second < first {
                (second, first)
            } else {
                (first, (first + 1).min(len))
            };
            let mode = match raw_mode % 4 {
                0 => HeadlessChangeCaseMode::Upper,
                1 => HeadlessChangeCaseMode::Lower,
                2 => HeadlessChangeCaseMode::Title,
                _ => HeadlessChangeCaseMode::Sentence,
            };
            let splits = raw_splits
                .into_iter()
                .map(|offset| usize::from(offset) % (len + 1))
                .collect::<Vec<_>>();
            let mut word_doc = sample_document_with_styled_runs(&initial, &splits);
            let original_properties = run_properties_by_byte(&word_doc, 0);
            let request = HeadlessEditRequest {
                commands: vec![HeadlessCommand::ChangeCase {
                    start_paragraph: 0,
                    start_offset: start,
                    end_paragraph: 0,
                    end_offset: end,
                    mode,
                }],
            };

            apply_headless_commands_to_document(&mut word_doc, &request)
                .expect("case conversion should cross run boundaries");

            let mut expected = initial.clone();
            expected.replace_range(start..end, &reference_case(&initial[start..end], mode));
            prop_assert_eq!(paragraph_text(&word_doc, 0), expected);
            prop_assert_eq!(run_properties_by_byte(&word_doc, 0), original_properties);
        }

        #[test]
        fn property_unicode_case_expansion_updates_coordinates_for_later_commands(
            characters in prop::collection::vec(
                prop_oneof![
                    Just('a'), Just('B'), Just('ß'), Just('İ'), Just('Σ'),
                    Just('σ'), Just('ς'), Just('あ'), Just('🙂'), Just(' '),
                    Just('　'), Just('.'), Just('。')
                ],
                4..24,
            ),
            raw_start in any::<u8>(),
            raw_end in any::<u8>(),
            raw_mode in any::<u8>(),
            raw_splits in prop::collection::vec(any::<u8>(), 0..14),
        ) {
            let initial = characters.iter().collect::<String>();
            let mut boundaries = initial.char_indices().map(|(offset, _)| offset).collect::<Vec<_>>();
            boundaries.push(initial.len());
            let first = usize::from(raw_start) % characters.len();
            let second = 1 + usize::from(raw_end) % characters.len();
            let (start_index, end_index) = if first < second {
                (first, second)
            } else if second < first {
                (second, first)
            } else {
                (first, (first + 1).min(characters.len()))
            };
            let start = boundaries[start_index];
            let end = boundaries[end_index];
            let mode = match raw_mode % 4 {
                0 => HeadlessChangeCaseMode::Upper,
                1 => HeadlessChangeCaseMode::Lower,
                2 => HeadlessChangeCaseMode::Title,
                _ => HeadlessChangeCaseMode::Sentence,
            };
            let transformed = reference_case(&initial[start..end], mode);
            let transformed_end = start + transformed.len();
            let splits = raw_splits
                .into_iter()
                .map(|offset| boundaries[usize::from(offset) % boundaries.len()])
                .collect::<Vec<_>>();
            let mut word_doc = sample_document_with_styled_runs(&initial, &splits);
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::ChangeCase {
                        start_paragraph: 0,
                        start_offset: start,
                        end_paragraph: 0,
                        end_offset: end,
                        mode,
                    },
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: transformed_end,
                        text: "|".to_string(),
                    },
                ],
            };

            apply_headless_commands_to_document(&mut word_doc, &request)
                .expect("later command should consume the post-case-change coordinate");

            let mut expected = initial.clone();
            expected.replace_range(start..end, &transformed);
            expected.insert(transformed_end, '|');
            prop_assert_eq!(paragraph_text(&word_doc, 0), expected);
        }

        #[test]
        fn property_multi_paragraph_edits_match_model(
            ops in prop::collection::vec((0u8..6, any::<u8>(), any::<u8>(), any::<u8>()), 1..120)
        ) {
            let mut model = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
            let mut word_doc = sample_document_from_paragraphs(&model);

            for (kind, a, b, c) in ops {
                match kind {
                    // Insert a character into a random paragraph position.
                    0 => {
                        let paragraph = usize::from(a) % model.len();
                        let offset = usize::from(b) % (model[paragraph].len() + 1);
                        let ch = char::from(b'a' + (c % 26));
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::InsertText {
                                paragraph,
                                offset,
                                text: ch.to_string(),
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("insert should succeed");
                        model[paragraph].insert(offset, ch);
                    }
                    // Delete backward inside a paragraph.
                    1 => {
                        let paragraph = usize::from(a) % model.len();
                        let len = model[paragraph].len();
                        if len == 0 {
                            continue;
                        }
                        let offset = 1 + (usize::from(b) % len);
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::DeleteText {
                                paragraph,
                                offset,
                                direction: HeadlessDeleteDirection::Backward,
                                count: 1,
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("backward delete should succeed");
                        model[paragraph].remove(offset - 1);
                    }
                    // Delete forward inside a paragraph.
                    2 => {
                        let paragraph = usize::from(a) % model.len();
                        let len = model[paragraph].len();
                        if len == 0 {
                            continue;
                        }
                        let offset = usize::from(b) % len;
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::DeleteText {
                                paragraph,
                                offset,
                                direction: HeadlessDeleteDirection::Forward,
                                count: 1,
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("forward delete should succeed");
                        model[paragraph].remove(offset);
                    }
                    // Replace range inside one paragraph.
                    3 => {
                        let paragraph = usize::from(a) % model.len();
                        let len = model[paragraph].len();
                        let start = usize::from(b) % (len + 1);
                        let end = usize::from(c) % (len + 1);
                        let (start, end) = if start <= end { (start, end) } else { (end, start) };
                        let repl = char::from(b'a' + ((a.wrapping_add(b).wrapping_add(c)) % 26)).to_string();
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::ReplaceRange {
                                start_paragraph: paragraph,
                                start_offset: start,
                                end_paragraph: paragraph,
                                end_offset: end,
                                text: repl.clone(),
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("replace should succeed");
                        model[paragraph].replace_range(start..end, &repl);
                    }
                    // Insert paragraph break at a random position.
                    4 => {
                        let paragraph = usize::from(a) % model.len();
                        let offset = usize::from(b) % (model[paragraph].len() + 1);
                        let request = HeadlessEditRequest {
                            commands: vec![HeadlessCommand::InsertParagraphBreak {
                                paragraph,
                                offset,
                                count: 1,
                            }],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("paragraph break should succeed");

                        let right = model[paragraph].split_off(offset);
                        model.insert(paragraph + 1, right);
                    }
                    // Execute two commands in a single request.
                    _ => {
                        let insert_paragraph = usize::from(a) % model.len();
                        let insert_offset = usize::from(b) % (model[insert_paragraph].len() + 1);
                        let inserted = char::from(b'a' + (c % 26)).to_string();

                        let mut projected = model.clone();
                        projected[insert_paragraph].insert(insert_offset, inserted.chars().next().expect("one char"));

                        let replace_paragraph = usize::from(c) % projected.len();
                        let replace_len = projected[replace_paragraph].len();
                        let replace_start = usize::from(a.wrapping_add(c)) % (replace_len + 1);
                        let replace_end = usize::from(b.wrapping_add(c)) % (replace_len + 1);
                        let (replace_start, replace_end) = if replace_start <= replace_end {
                            (replace_start, replace_end)
                        } else {
                            (replace_end, replace_start)
                        };
                        let replacement = char::from(b'a' + (a % 26)).to_string();
                        let request = HeadlessEditRequest {
                            commands: vec![
                                HeadlessCommand::InsertText {
                                    paragraph: insert_paragraph,
                                    offset: insert_offset,
                                    text: inserted.clone(),
                                },
                                HeadlessCommand::ReplaceRange {
                                    start_paragraph: replace_paragraph,
                                    start_offset: replace_start,
                                    end_paragraph: replace_paragraph,
                                    end_offset: replace_end,
                                    text: replacement.clone(),
                                },
                            ],
                        };
                        apply_headless_commands_to_document(&mut word_doc, &request)
                            .expect("batch request should succeed");

                        model = projected;
                        model[replace_paragraph].replace_range(replace_start..replace_end, &replacement);
                    }
                }

                let actual = paragraph_texts(&word_doc);
                prop_assert_eq!(actual.as_slice(), model.as_slice());
            }
        }

        #[test]
        fn property_set_table_cell_text_writes_exact_text(
            value in "[a-zA-Z0-9 _-]{0,24}"
        ) {
            let mut word_doc = sample_table_document();
            let request = HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetTableCellText {
                    table_order_index: 0,
                    row: 1,
                    column: 0,
                    text: value.clone(),
                }],
            };

            apply_headless_commands_to_document(&mut word_doc, &request)
                .expect("set_table_cell_text should succeed");

            let table = word_doc.table_by_order(0).expect("table exists");
            let BodyElement::Paragraph(paragraph) = &table.rows[1].cells[0].blocks[0] else {
                panic!("cell must contain paragraph");
            };
            let actual: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
            prop_assert_eq!(actual, value);
        }

        #[test]
        fn property_table_column_query_resolves_horizontal_spans(
            left_span in 1usize..=4,
            query_col in 0usize..4,
        ) {
            let make_cell = |text: &str, span: Option<u32>| TableCell {
                blocks: vec![BodyElement::from_paragraph(paragraph_from_text(text))],
                style: Default::default(),
                class_name: None,
                span,
                vertical_merge: None,
                text_direction: None,
                margins: None,
                vertical_align: None,
                node_id: None,
            };

            let mut first_row_cells = vec![make_cell("TOP", Some(left_span as u32))];
            if left_span < 4 {
                let right_span = 4 - left_span;
                let trailing_span = if right_span > 1 {
                    Some(right_span as u32)
                } else {
                    None
                };
                first_row_cells.push(make_cell("TAIL", trailing_span));
            }

            let second_row_cells = (0..4)
                .map(|column| make_cell(&format!("R1C{column}"), None))
                .collect();
            let table = Table {
                rows: vec![
                    TableRow {
                        cells: first_row_cells,
                        ..TableRow::default()
                    },
                    TableRow {
                        cells: second_row_cells,
                        ..TableRow::default()
                    },
                ],
                columns: vec![
                    TableColumn { width: Some(10.0) },
                    TableColumn { width: Some(20.0) },
                    TableColumn { width: Some(30.0) },
                    TableColumn { width: Some(40.0) },
                ],
                ..Table::default()
            };
            let word_doc = WordDocument::from_document(Document {
                body: Some(Body {
                    elements: vec![BodyElement::from_table(table)],
                }),
                ..Document::default()
            });
            let request = HeadlessQueryRequest {
                queries: vec![HeadlessQuery::TableColumn {
                    table_order_index: 0,
                    column: query_col,
                }],
            };

            let response =
                execute_headless_queries_on_document(&word_doc, &request).expect("query succeeds");
            let HeadlessQueryResult::TableColumn {
                value: Some(column), ..
            } = &response.results[0]
            else {
                panic!("expected table column result");
            };

            prop_assert_eq!(column.cells.len(), 2);
            let expected_top_text = if query_col < left_span { "TOP" } else { "TAIL" };
            let expected_top_column = if query_col < left_span { 0 } else { left_span };
            let expected_second_row_text = format!("R1C{query_col}");
            prop_assert_eq!(column.cells[0].text.as_str(), expected_top_text);
            prop_assert_eq!(column.cells[0].column, expected_top_column);
            prop_assert_eq!(column.cells[1].text.as_str(), expected_second_row_text.as_str());
            prop_assert_eq!(column.cells[1].column, query_col);
        }

        /// `ParagraphByParaId` resolves each paragraph's `w14:paraId` to its
        /// current positional index and snapshot (getElementById), and reports
        /// `None` for an id that no paragraph carries.
        #[test]
        fn paragraph_by_para_id_resolves_index_and_text(items in para_id_items_strategy()) {
            let word_doc = document_with_para_ids(&items);

            for (expected_index, (para_id, text)) in items.iter().enumerate() {
                let request = HeadlessQueryRequest {
                    queries: vec![HeadlessQuery::ParagraphByParaId { para_id: para_id.clone() }],
                };
                let response = execute_headless_queries_on_document(&word_doc, &request)
                    .expect("query should succeed");
                let HeadlessQueryResult::ParagraphByParaId { paragraph, value, .. } =
                    &response.results[0]
                else {
                    panic!("expected paragraph_by_para_id result");
                };
                prop_assert_eq!(*paragraph, Some(expected_index));
                prop_assert_eq!(value.as_ref().map(|v| v.text.as_str()), Some(text.as_str()));
                prop_assert_eq!(value.as_ref().map(|v| v.paragraph), Some(expected_index));
            }

            // An id that no paragraph carries resolves to None.
            let request = HeadlessQueryRequest {
                queries: vec![HeadlessQuery::ParagraphByParaId {
                    para_id: "absent-paraid".to_string(),
                }],
            };
            let response = execute_headless_queries_on_document(&word_doc, &request)
                .expect("query should succeed");
            let HeadlessQueryResult::ParagraphByParaId { paragraph, value, .. } =
                &response.results[0]
            else {
                panic!("expected paragraph_by_para_id result");
            };
            prop_assert_eq!(*paragraph, None);
            prop_assert!(value.is_none());
        }

    }

    #[test]
    fn add_comment_by_para_id_errors_for_unknown_id() {
        let mut word_doc = document_with_para_ids(&[("AAA".to_string(), "hello".to_string())]);
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddCommentByParaId {
                para_id: "missing".to_string(),
                start_offset: 0,
                end_offset: 1,
                text: "x".to_string(),
                author: None,
                initials: None,
                date: None,
            }],
        };
        let err = apply_headless_commands_to_document(&mut word_doc, &request)
            .expect_err("an unknown paraId must error");
        let HeadlessApiApplyError::Command {
            source: CommandError::Custom(message),
            ..
        } = err
        else {
            panic!("expected a command error, got {err:?}");
        };
        assert!(
            message.contains("no paragraph found"),
            "unexpected message: {message}"
        );
    }

    /// オフセットを省略しても段落全体として解決され、add_comment まで到達すること。
    ///
    /// 合成文書には raw XML が無いので add_comment 側で必ず失敗する。そこまで届けば
    /// 「省略されたオフセットが埋められた」ことの証明になる（埋まらなければ型の時点で通らない）。
    #[test]
    fn add_comment_without_offsets_targets_the_whole_paragraph() {
        let mut word_doc = document_with_para_ids(&[("AAA".to_string(), "hello".to_string())]);
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddComment {
                start_paragraph: 0,
                start_offset: None,
                end_paragraph: 0,
                end_offset: None,
                text: "note".to_string(),
                author: None,
                initials: None,
                date: None,
            }],
        };
        let err = apply_headless_commands_to_document(&mut word_doc, &request)
            .expect_err("should reach add_comment and fail without raw xml");
        let HeadlessApiApplyError::Command {
            source: CommandError::Custom(message),
            ..
        } = err
        else {
            panic!("expected a command error, got {err:?}");
        };
        assert!(
            message.contains("raw document xml"),
            "unexpected message: {message}"
        );
    }

    /// 一致しない文字列は、オフセット計算の前に分かる形で落ちること。
    #[test]
    fn add_comment_by_text_errors_when_the_text_is_absent() {
        let mut word_doc = document_with_para_ids(&[("AAA".to_string(), "hello".to_string())]);
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddCommentByText {
                query: "goodbye".to_string(),
                occurrence: None,
                text: "note".to_string(),
                author: None,
                initials: None,
                date: None,
            }],
        };
        let err = apply_headless_commands_to_document(&mut word_doc, &request)
            .expect_err("missing text must error");
        let HeadlessApiApplyError::Command {
            source: CommandError::Custom(message),
            ..
        } = err
        else {
            panic!("expected a command error, got {err:?}");
        };
        assert!(
            message.contains("text not found") && message.contains("goodbye"),
            "探した文字列を名指しすること: {message}"
        );
    }

    /// 一致した場合は add_comment まで到達すること（= 位置が解決できている）。
    #[test]
    fn add_comment_by_text_resolves_a_match() {
        let mut word_doc = document_with_para_ids(&[("AAA".to_string(), "hello".to_string())]);
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddCommentByText {
                query: "ell".to_string(),
                occurrence: None,
                text: "note".to_string(),
                author: None,
                initials: None,
                date: None,
            }],
        };
        let err = apply_headless_commands_to_document(&mut word_doc, &request)
            .expect_err("should reach add_comment and fail without raw xml");
        let HeadlessApiApplyError::Command {
            source: CommandError::Custom(message),
            ..
        } = err
        else {
            panic!("expected a command error, got {err:?}");
        };
        assert!(
            message.contains("raw document xml"),
            "位置解決を通り抜けて add_comment に届くこと: {message}"
        );
    }

    /// occurrence で 2 個目以降を選べること。範囲は文書順に数える。
    #[test]
    fn find_nth_text_match_counts_in_document_order() {
        let word_doc = document_with_para_ids(&[
            ("A".to_string(), "foo bar".to_string()),
            ("B".to_string(), "bar baz".to_string()),
        ]);
        let first = find_nth_text_match(&word_doc, "bar", 0).expect("first match");
        assert_eq!((first.paragraph, first.start, first.end), (0, 4, 7));

        let second = find_nth_text_match(&word_doc, "bar", 1).expect("second match");
        assert_eq!((second.paragraph, second.start, second.end), (1, 0, 3));

        assert!(find_nth_text_match(&word_doc, "bar", 2).is_none());
        assert!(find_nth_text_match(&word_doc, "", 0).is_none());
    }

    #[test]
    fn add_comment_by_para_id_resolves_then_requires_raw_xml() {
        // A synthetic document has no raw XML, so a *resolvable* paraId must fail
        // inside `add_comment` (not in the resolver) — proving the id resolved to
        // a paragraph and the command delegated to the same add_comment path.
        let mut word_doc = document_with_para_ids(&[("KNOWN".to_string(), "hello".to_string())]);
        let request = HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddCommentByParaId {
                para_id: "KNOWN".to_string(),
                start_offset: 0,
                end_offset: 5,
                text: "x".to_string(),
                author: None,
                initials: None,
                date: None,
            }],
        };
        let err = apply_headless_commands_to_document(&mut word_doc, &request)
            .expect_err("should fail in add_comment without raw xml");
        let HeadlessApiApplyError::Command {
            source: CommandError::Custom(message),
            ..
        } = err
        else {
            panic!("expected a command error, got {err:?}");
        };
        assert!(
            message.contains("raw document xml"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn named_table_style_catalog_and_validation_are_document_scoped() {
        let mut word_doc = sample_table_document_with_named_styles();
        assert_eq!(
            word_doc
                .table_style_infos()
                .iter()
                .map(|style| style.style_id.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Beta"]
        );
        let err = apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::ApplyTableNamedStyle {
                    table_order_index: 0,
                    style_id: Some("Missing".to_string()),
                }],
            },
        )
        .expect_err("foreign style id must fail");
        assert!(matches!(
            err,
            HeadlessApiApplyError::Command { index: 0, .. }
        ));
        assert_eq!(word_doc.table_style_id(0).expect("table"), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 96,
            max_shrink_iters: 3000,
            .. ProptestConfig::default()
        })]

        #[test]
        fn property_named_table_style_sequences_undo_and_redo_exactly(
            operations in prop::collection::vec(0u8..3, 1..64),
        ) {
            let commands = operations
                .iter()
                .map(|operation| HeadlessCommand::ApplyTableNamedStyle {
                    table_order_index: 0,
                    style_id: match operation {
                        0 => Some("Alpha".to_string()),
                        1 => Some("Beta".to_string()),
                        _ => None,
                    },
                })
                .collect::<Vec<_>>();
            let expected = match operations.last().copied() {
                Some(0) => Some("Alpha"),
                Some(1) => Some("Beta"),
                _ => None,
            };
            let mut word_doc = sample_table_document_with_named_styles();
            let mut executor = CommandExecutor::new();
            let mut selection = apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &HeadlessEditRequest { commands },
            )
            .expect("valid table style sequence");

            let applied_style_id = word_doc.table_style_id(0).expect("table");
            prop_assert_eq!(applied_style_id.as_deref(), expected);
            let expected_fill = match expected {
                Some("Alpha") | Some("Beta") => Some("#AABBCC"),
                _ => None,
            };
            prop_assert_eq!(
                word_doc
                    .table_by_order(0)
                    .and_then(|table| table.resolved_style.as_ref())
                    .and_then(|style| style.fill.as_deref()),
                expected_fill
            );

            undo_all(&mut word_doc, &mut executor, &mut selection);
            prop_assert_eq!(word_doc.table_style_id(0).expect("table"), None);
            prop_assert!(word_doc
                .table_by_order(0)
                .and_then(|table| table.resolved_style.as_ref())
                .is_none());

            while executor.can_redo() {
                let mut context = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.redo(&mut context).expect("redo succeeds"));
            }
            let redone_style_id = word_doc.table_style_id(0).expect("table");
            prop_assert_eq!(redone_style_id.as_deref(), expected);
        }
    }

    fn generated_shape_kind(index: u8) -> HeadlessShapeKind {
        match index % 109 {
            0 => HeadlessShapeKind::Rectangle,
            1 => HeadlessShapeKind::RoundedRectangle,
            2 => HeadlessShapeKind::Ellipse,
            3 => HeadlessShapeKind::Diamond,
            4 => HeadlessShapeKind::Triangle,
            5 => HeadlessShapeKind::RightTriangle,
            6 => HeadlessShapeKind::Parallelogram,
            7 => HeadlessShapeKind::Trapezoid,
            8 => HeadlessShapeKind::Pentagon,
            9 => HeadlessShapeKind::Hexagon,
            10 => HeadlessShapeKind::Octagon,
            11 => HeadlessShapeKind::Plus,
            12 => HeadlessShapeKind::FourPointStar,
            13 => HeadlessShapeKind::FivePointStar,
            14 => HeadlessShapeKind::SixPointStar,
            15 => HeadlessShapeKind::SevenPointStar,
            16 => HeadlessShapeKind::EightPointStar,
            17 => HeadlessShapeKind::TenPointStar,
            18 => HeadlessShapeKind::TwelvePointStar,
            19 => HeadlessShapeKind::SixteenPointStar,
            20 => HeadlessShapeKind::TwentyFourPointStar,
            21 => HeadlessShapeKind::ThirtyTwoPointStar,
            22 => HeadlessShapeKind::IrregularSealOne,
            23 => HeadlessShapeKind::IrregularSealTwo,
            24 => HeadlessShapeKind::Ribbon,
            25 => HeadlessShapeKind::RibbonTwo,
            26 => HeadlessShapeKind::Wave,
            27 => HeadlessShapeKind::DoubleWave,
            28 => HeadlessShapeKind::FlowChartProcess,
            29 => HeadlessShapeKind::FlowChartAlternateProcess,
            30 => HeadlessShapeKind::FlowChartDecision,
            31 => HeadlessShapeKind::FlowChartPredefinedProcess,
            32 => HeadlessShapeKind::FlowChartInternalStorage,
            33 => HeadlessShapeKind::FlowChartDocument,
            34 => HeadlessShapeKind::FlowChartMultidocument,
            35 => HeadlessShapeKind::FlowChartTerminator,
            36 => HeadlessShapeKind::FlowChartPreparation,
            37 => HeadlessShapeKind::FlowChartManualInput,
            38 => HeadlessShapeKind::FlowChartManualOperation,
            39 => HeadlessShapeKind::FlowChartConnector,
            40 => HeadlessShapeKind::FlowChartOffpageConnector,
            41 => HeadlessShapeKind::FlowChartMagneticDisk,
            42 => HeadlessShapeKind::FlowChartMagneticDrum,
            43 => HeadlessShapeKind::FlowChartDisplay,
            44 => HeadlessShapeKind::FlowChartDelay,
            45 => HeadlessShapeKind::FlowChartOr,
            46 => HeadlessShapeKind::FlowChartCollate,
            47 => HeadlessShapeKind::FlowChartSort,
            48 => HeadlessShapeKind::FlowChartExtract,
            49 => HeadlessShapeKind::FlowChartMerge,
            50 => HeadlessShapeKind::FlowChartSummingJunction,
            51 => HeadlessShapeKind::FlowChartPunchedTape,
            52 => HeadlessShapeKind::RightArrow,
            53 => HeadlessShapeKind::LeftArrow,
            54 => HeadlessShapeKind::UpArrow,
            55 => HeadlessShapeKind::DownArrow,
            56 => HeadlessShapeKind::LeftRightArrow,
            57 => HeadlessShapeKind::UpDownArrow,
            58 => HeadlessShapeKind::NotchedRightArrow,
            59 => HeadlessShapeKind::Chevron,
            60 => HeadlessShapeKind::HomePlate,
            61 => HeadlessShapeKind::SnipOneRectangle,
            62 => HeadlessShapeKind::SnipTwoSameRectangle,
            63 => HeadlessShapeKind::SnipTwoDiagonalRectangle,
            64 => HeadlessShapeKind::SnipRoundRectangle,
            65 => HeadlessShapeKind::RoundOneRectangle,
            66 => HeadlessShapeKind::RoundTwoSameRectangle,
            67 => HeadlessShapeKind::RoundTwoDiagonalRectangle,
            68 => HeadlessShapeKind::Line,
            69 => HeadlessShapeKind::LineArrow,
            70 => HeadlessShapeKind::LineArrowDouble,
            71 => HeadlessShapeKind::LineBent,
            72 => HeadlessShapeKind::LineCurved,
            73 => HeadlessShapeKind::WedgeRectangleCallout,
            74 => HeadlessShapeKind::WedgeRoundedRectangleCallout,
            75 => HeadlessShapeKind::WedgeEllipseCallout,
            76 => HeadlessShapeKind::CloudCallout,
            77 => HeadlessShapeKind::QuadArrow,
            78 => HeadlessShapeKind::BentArrow,
            79 => HeadlessShapeKind::UturnArrow,
            80 => HeadlessShapeKind::CurvedRightArrow,
            81 => HeadlessShapeKind::StripedRightArrow,
            82 => HeadlessShapeKind::MathPlus,
            83 => HeadlessShapeKind::LeftBrace,
            84 => HeadlessShapeKind::RightBrace,
            85 => HeadlessShapeKind::BlockArc,
            86 => HeadlessShapeKind::Pie,
            87 => HeadlessShapeKind::Chord,
            88 => HeadlessShapeKind::Teardrop,
            89 => HeadlessShapeKind::Frame,
            90 => HeadlessShapeKind::HalfFrame,
            91 => HeadlessShapeKind::Corner,
            92 => HeadlessShapeKind::DiagonalStripe,
            93 => HeadlessShapeKind::Donut,
            94 => HeadlessShapeKind::NoSmoking,
            95 => HeadlessShapeKind::FoldedCorner,
            96 => HeadlessShapeKind::Bevel,
            97 => HeadlessShapeKind::Cube,
            98 => HeadlessShapeKind::Can,
            99 => HeadlessShapeKind::LightningBolt,
            100 => HeadlessShapeKind::Heart,
            101 => HeadlessShapeKind::Sun,
            102 => HeadlessShapeKind::Moon,
            103 => HeadlessShapeKind::Cloud,
            104 => HeadlessShapeKind::Arc,
            105 => HeadlessShapeKind::Plaque,
            106 => HeadlessShapeKind::SmileyFace,
            107 => HeadlessShapeKind::LeftBracket,
            _ => HeadlessShapeKind::RightBracket,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 96,
            max_shrink_iters: 4000,
            .. ProptestConfig::default()
        })]

        #[test]
        fn property_shape_insertions_follow_current_state_across_run_boundaries_and_history(
            text in "[a-zA-Z0-9ぁ-んァ-ヶ一-龠]{1,48}",
            boundary_seed in any::<usize>(),
            split_seeds in prop::collection::vec(any::<usize>(), 0..12),
            shape_index in 0u8..109,
            width in 12.0f64..700.0,
            height in 12.0f64..500.0,
            border_width in 0.0f64..12.0,
            shape_text in "[a-zA-Z0-9ぁ-んァ-ヶ一-龠]{0,32}",
        ) {
            let boundaries = text
                .char_indices()
                .map(|(offset, _)| offset)
                .chain(std::iter::once(text.len()))
                .collect::<Vec<_>>();
            let offset = boundaries[boundary_seed % boundaries.len()];
            let split_offsets = split_seeds
                .iter()
                .map(|seed| boundaries[*seed % boundaries.len()])
                .collect::<Vec<_>>();
            let mut word_doc = sample_document_with_styled_runs(&text, &split_offsets);
            let original_run_snapshot = word_doc
                .paragraph(0)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| {
                    (
                        run.text.clone(),
                        run.properties.as_ref().and_then(|props| props.bold),
                        run.properties.as_ref().and_then(|props| props.italic),
                    )
                })
                .collect::<Vec<_>>();
            let prefix = "先行🙂";
            let current_offset = prefix.len() + offset;
            let shape_kind = generated_shape_kind(shape_index);
            let is_line = matches!(
                shape_kind,
                HeadlessShapeKind::Line
                    | HeadlessShapeKind::LineArrow
                    | HeadlessShapeKind::LineArrowDouble
                    | HeadlessShapeKind::LineBent
                    | HeadlessShapeKind::LineCurved
            );
            let request = HeadlessEditRequest {
                commands: vec![
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 0,
                        text: prefix.to_string(),
                    },
                    HeadlessCommand::InsertShape {
                        paragraph: 0,
                        offset: current_offset,
                        shape: shape_kind,
                        width_px: width,
                        height_px: height,
                        fill_color: Some("#4472C4".to_string()),
                        border_color: Some("2F5496".to_string()),
                        border_width_px: border_width,
                        text: shape_text.clone(),
                    },
                ],
            };
            let mut executor = CommandExecutor::new();
            let mut selection = apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut executor,
                None,
                &request,
            )
            .expect("valid current-state shape insertion");

            let host = word_doc.paragraph(0).expect("host paragraph");
            prop_assert_eq!(
                host.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
                format!("{prefix}{text}")
            );
            let shapes = host
                .runs
                .iter()
                .filter_map(|run| run.text_box.as_ref())
                .collect::<Vec<_>>();
            prop_assert_eq!(shapes.len(), 1);
            prop_assert_eq!(
                shapes[0].text.as_str(),
                if is_line { "" } else { shape_text.as_str() }
            );
            prop_assert_eq!(
                shapes[0].fill_color.as_deref(),
                if is_line { None } else { Some("#4472C4") }
            );
            prop_assert_eq!(shapes[0].blocks.is_empty(), is_line);
            prop_assert!((shapes[0].width_px.unwrap_or_default() - width).abs() < 1e-9);
            prop_assert!((shapes[0].height_px.unwrap_or_default() - height).abs() < 1e-9);

            undo_all(&mut word_doc, &mut executor, &mut selection);
            let restored = word_doc.paragraph(0).expect("restored paragraph");
            prop_assert_eq!(
                restored
                    .runs
                    .iter()
                    .map(|run| {
                        (
                            run.text.clone(),
                            run.properties.as_ref().and_then(|props| props.bold),
                            run.properties.as_ref().and_then(|props| props.italic),
                        )
                    })
                    .collect::<Vec<_>>(),
                original_run_snapshot
            );
            prop_assert!(restored.runs.iter().all(|run| run.text_box.is_none()));

            while executor.can_redo() {
                let mut context = CommandContext::new(&mut word_doc, &mut selection);
                executor.redo(&mut context).expect("redo shape sequence");
            }
            prop_assert_eq!(
                word_doc
                    .paragraph(0)
                    .expect("redone paragraph")
                    .runs
                    .iter()
                    .filter(|run| run.text_box.is_some())
                    .count(),
                1
            );
        }
    }

    #[test]
    fn insert_shape_reports_deleted_paragraph_and_invalid_utf8_coordinates() {
        let mut word_doc =
            sample_document_from_paragraphs(&["first".to_string(), "日本語".to_string()]);
        let deleted = HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 1,
                    end_offset: "日本語".len(),
                    text: "merged".to_string(),
                },
                HeadlessCommand::InsertShape {
                    paragraph: 1,
                    offset: 0,
                    shape: HeadlessShapeKind::Ellipse,
                    width_px: 120.0,
                    height_px: 80.0,
                    fill_color: Some("4472C4".to_string()),
                    border_color: Some("2F5496".to_string()),
                    border_width_px: 1.0,
                    text: String::new(),
                },
            ],
        };
        let error = apply_headless_commands_to_document(&mut word_doc, &deleted)
            .expect_err("deleted paragraph must not be retargeted");
        assert!(matches!(
            error,
            HeadlessApiApplyError::Command { index: 1, .. }
        ));

        let mut word_doc = sample_document("日本語");
        let midpoint = HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertShape {
                paragraph: 0,
                offset: 1,
                shape: HeadlessShapeKind::Ellipse,
                width_px: 120.0,
                height_px: 80.0,
                fill_color: None,
                border_color: None,
                border_width_px: 1.0,
                text: String::new(),
            }],
        };
        let error = apply_headless_commands_to_document(&mut word_doc, &midpoint)
            .expect_err("UTF-8 midpoint must fail");
        assert!(matches!(
            error,
            HeadlessApiApplyError::Command { index: 0, .. }
        ));
    }

    /// `public/fixtures/review-kinds.docx`: one revision of every kind jade
    /// models, from two reviewers (see `scripts/generate-fixtures.mjs`).
    fn review_kinds_document() -> WordDocument {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/review-kinds.docx"),
        )
        .expect("read review-kinds fixture");
        executor::block_on(WordDocument::load(&bytes, &Options::default()))
            .expect("load review-kinds fixture")
    }

    fn revision_ids(word_doc: &WordDocument) -> Vec<String> {
        revision_snapshots(word_doc)
            .into_iter()
            .map(|revision| revision.id.unwrap_or_default())
            .collect()
    }

    fn selected_ids(word_doc: &WordDocument, selector: &RevisionSelector) -> Vec<String> {
        let snapshots = revision_snapshots(word_doc);
        select_revisions(&snapshots, selector)
            .into_iter()
            .map(|revision| revision.id.clone().unwrap_or_default())
            .collect()
    }

    fn range_selector(start: (usize, usize), end: (usize, usize)) -> RevisionSelector {
        RevisionSelector {
            range: Some((
                TextPosition::new(start.0, start.1),
                TextPosition::new(end.0, end.1),
            )),
            ..RevisionSelector::default()
        }
    }

    #[test]
    fn review_kinds_fixture_reports_every_modelled_kind() {
        let word_doc = review_kinds_document();
        let kinds: Vec<_> = revision_snapshots(&word_doc)
            .into_iter()
            .map(|revision| {
                (
                    revision.id.unwrap_or_default(),
                    revision.kind,
                    revision.author.unwrap_or_default(),
                    revision.paragraph_mark,
                )
            })
            .collect();
        let expected = [
            ("1", "inserted", "Alice", false),
            ("2", "deleted", "Bob", false),
            ("3", "movedFrom", "Alice", false),
            ("4", "movedTo", "Alice", false),
            ("5", "inserted", "Bob", false),
            ("6", "deleted", "Alice", false),
            ("7", "inserted", "Bob", true),
        ]
        .map(|(id, kind, author, mark)| {
            (id.to_string(), kind.to_string(), author.to_string(), mark)
        });
        assert_eq!(kinds, expected);
        // The two page breaks are revisions too, so their display depends on
        // the markup mode rather than being fixed layout.
        assert_eq!(paragraph_text(&word_doc, 3), "\u{c}After inserted break");
        assert_eq!(paragraph_text(&word_doc, 4), "\u{c}After deleted break");
    }

    #[test]
    fn selection_scoped_revision_selector_follows_word_this_change() {
        let word_doc = review_kinds_document();
        // "Base inserted-A mid deleted-B end": ins#1 is [5, 15), del#2 [20, 29).
        assert_eq!(
            selected_ids(&word_doc, &range_selector((0, 3), (0, 22))),
            ["1", "2"]
        );
        // Touching a revision's edge from outside does not overlap it.
        assert!(selected_ids(&word_doc, &range_selector((0, 15), (0, 20))).is_empty());
        // A caret takes the change it sits in, or the one ending right at it.
        assert_eq!(
            selected_ids(&word_doc, &range_selector((0, 7), (0, 7))),
            ["1"]
        );
        assert_eq!(
            selected_ids(&word_doc, &range_selector((0, 15), (0, 15))),
            ["1"]
        );
        assert!(selected_ids(&word_doc, &range_selector((0, 17), (0, 17))).is_empty());
        // A paragraph mark is reached only by a selection that runs past the
        // end of its paragraph, or by a caret sitting at that end.
        assert!(selected_ids(&word_doc, &range_selector((5, 0), (5, 10))).is_empty());
        assert_eq!(
            selected_ids(&word_doc, &range_selector((5, 0), (6, 0))),
            ["7"]
        );
        assert_eq!(
            selected_ids(&word_doc, &range_selector((5, 10), (5, 10))),
            ["7"]
        );
        // A selection across paragraphs takes everything in between.
        assert_eq!(
            selected_ids(&word_doc, &range_selector((0, 25), (3, 1))),
            ["2", "3", "4", "5"]
        );
    }

    #[test]
    fn revision_selector_combines_kind_and_reviewer_filters() {
        let word_doc = review_kinds_document();
        let filtered = |kinds: Option<&[&str]>, authors: Option<&[Option<&str>]>| {
            selected_ids(
                &word_doc,
                &RevisionSelector {
                    range: None,
                    kinds: kinds.map(|kinds| kinds.iter().map(|kind| kind.to_string()).collect()),
                    authors: authors.map(|authors| {
                        authors
                            .iter()
                            .map(|author| author.map(str::to_string))
                            .collect()
                    }),
                },
            )
        };
        assert_eq!(filtered(None, None).len(), 7);
        assert_eq!(filtered(Some(&["movedFrom", "movedTo"]), None), ["3", "4"]);
        assert_eq!(filtered(None, Some(&[Some("Bob")])), ["2", "5", "7"]);
        assert_eq!(
            filtered(Some(&["inserted"]), Some(&[Some("Bob")])),
            ["5", "7"]
        );
        assert_eq!(filtered(Some(&["deleted"]), Some(&[Some("Alice")])), ["6"]);
        // `None` in the reviewer list is an unattributed change; this fixture
        // has none.
        assert!(filtered(None, Some(&[None])).is_empty());
        assert!(filtered(Some(&[]), None).is_empty());
        // Filters narrow a selection scope too.
        let mut selector = range_selector((0, 0), (6, 0));
        selector.kinds = Some(vec!["deleted".to_string()]);
        assert_eq!(selected_ids(&word_doc, &selector), ["2", "6"]);
    }

    #[test]
    fn next_revision_from_position_wraps_to_the_first_change() {
        let word_doc = review_kinds_document();
        let snapshots = revision_snapshots(&word_doc);
        let all = RevisionSelector::default();
        let next = |paragraph, offset, selector: &RevisionSelector| {
            next_revision_from(&snapshots, TextPosition::new(paragraph, offset), selector)
                .and_then(|revision| revision.id.clone())
        };
        assert_eq!(next(0, 0, &all).as_deref(), Some("1"));
        assert_eq!(next(0, 16, &all).as_deref(), Some("2"));
        assert_eq!(next(5, 3, &all).as_deref(), Some("7"));
        assert_eq!(next(6, 0, &all).as_deref(), Some("1"));
        assert!(next_revision_from(&[], TextPosition::new(0, 0), &all).is_none());
    }

    /// 「反映して次へ」 under a kind or reviewer filter skips the markup the
    /// filter hides, including when it wraps around.
    #[test]
    fn next_revision_from_respects_kind_and_reviewer_filters() {
        let word_doc = review_kinds_document();
        let snapshots = revision_snapshots(&word_doc);
        let deletions = RevisionSelector {
            kinds: Some(vec!["deleted".to_string()]),
            ..RevisionSelector::default()
        };
        let bob = RevisionSelector {
            authors: Some(vec![Some("Bob".to_string())]),
            ..RevisionSelector::default()
        };
        let next = |paragraph, offset, selector: &RevisionSelector| {
            next_revision_from(&snapshots, TextPosition::new(paragraph, offset), selector)
                .and_then(|revision| revision.id.clone())
        };
        assert_eq!(next(0, 0, &deletions).as_deref(), Some("2"));
        assert_eq!(next(1, 0, &deletions).as_deref(), Some("6"));
        assert_eq!(next(4, 1, &deletions).as_deref(), Some("2"));
        assert_eq!(next(0, 21, &bob).as_deref(), Some("5"));
        let nobody = RevisionSelector {
            authors: Some(vec![None]),
            ..RevisionSelector::default()
        };
        assert!(next(0, 0, &nobody).is_none());
    }

    /// The selection a scoped decision leaves behind follows the text it
    /// removed, and a removed paragraph mark moves the next paragraph up.
    #[test]
    fn decision_rebases_positions_over_removed_text_and_joined_paragraphs() {
        let word_doc = review_kinds_document();
        let snapshots = revision_snapshots(&word_doc);
        let by_id = |ids: &[&str]| -> Vec<&HeadlessRevisionSnapshot> {
            snapshots
                .iter()
                .filter(|revision| ids.contains(&revision.id.as_deref().unwrap_or_default()))
                .collect()
        };
        let rebase = |decided: &[&HeadlessRevisionSnapshot], accept, paragraph, offset| {
            let position = rebase_position_after_decision(
                &word_doc,
                decided,
                accept,
                TextPosition::new(paragraph, offset),
            );
            (position.paragraph, position.offset)
        };
        // Rejecting ins#1 [5, 15) removes ten bytes from P0.
        let ins = by_id(&["1"]);
        assert_eq!(rebase(&ins, false, 0, 3), (0, 3));
        assert_eq!(rebase(&ins, false, 0, 7), (0, 5));
        assert_eq!(rebase(&ins, false, 0, 22), (0, 12));
        // Accepting it removes nothing.
        assert_eq!(rebase(&ins, true, 0, 22), (0, 22));
        // Accepting del#2 [20, 29) removes nine bytes.
        assert_eq!(rebase(&by_id(&["2"]), true, 0, 30), (0, 21));
        // Rejecting the inserted mark of P5 ("Split head") joins P6 onto it.
        let mark = by_id(&["7"]);
        assert_eq!(rebase(&mark, false, 5, 4), (5, 4));
        assert_eq!(rebase(&mark, false, 6, 2), (5, 12));
        // Accepting it keeps both paragraphs.
        assert_eq!(rebase(&mark, true, 6, 2), (6, 2));
        // Joined and shortened at once: reject the inserted page break of P3
        // as well, then a later paragraph keeps its index minus the join.
        let both = by_id(&["5", "7"]);
        assert_eq!(rebase(&both, false, 3, 6), (3, 5));
        assert_eq!(rebase(&both, false, 6, 0), (5, 10));
    }

    fn review_duplicate_ids_document() -> WordDocument {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/review-duplicate-ids.docx"),
        )
        .expect("read review-duplicate-ids fixture");
        executor::block_on(WordDocument::load(&bytes, &Options::default()))
            .expect("load review-duplicate-ids fixture")
    }

    fn decide_targets(accept: bool, targets: Vec<HeadlessRevisionTarget>) -> HeadlessCommand {
        let revision_targets = Some(targets);
        if accept {
            HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets,
            }
        } else {
            HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets,
            }
        }
    }

    fn revision_spans(word_doc: &WordDocument) -> Vec<(usize, usize, String)> {
        revision_snapshots(word_doc)
            .into_iter()
            .map(|revision| (revision.paragraph, revision.start_offset, revision.kind))
            .collect()
    }

    /// Some tools write `w:id="0"` on every change. A decision scoped to one
    /// change must not reach the others through the shared id.
    #[test]
    fn targeted_decision_stays_on_its_revision_when_ids_repeat() {
        let original = review_duplicate_ids_document();
        let spans = revision_spans(&original);
        assert_eq!(
            spans,
            [
                (0, 5, "inserted".to_string()),
                (0, 15, "deleted".to_string()),
                (1, 0, "inserted".to_string()),
                (2, 9, "inserted".to_string()),
            ]
        );
        assert!(revision_snapshots(&original)
            .iter()
            .all(|revision| revision.id.as_deref() == Some("0")));

        // Accept only the insertion "first" in P0, as a selection over it does.
        let mut word_doc = original.clone();
        let snapshots = revision_snapshots(&word_doc);
        let selected = select_revisions(&snapshots, &range_selector((0, 5), (0, 10)));
        assert_eq!(selected.len(), 1);
        let mut executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![decide_targets(true, revision_targets_for(&selected))],
            },
        )
        .expect("accept one of the shared-id revisions");
        assert_eq!(
            revision_spans(&word_doc),
            [
                (0, 15, "deleted".to_string()),
                (1, 0, "inserted".to_string()),
                (2, 9, "inserted".to_string()),
            ]
        );
        assert_eq!(paragraph_text(&word_doc, 0), "Keep first and second end");
        assert_eq!(executor.undo_depth(), 1);
        let mut selection = None;
        let mut context = CommandContext::new(&mut word_doc, &mut selection);
        assert!(executor.undo(&mut context).expect("undo succeeds"));
        assert_eq!(revision_spans(&word_doc), spans);

        // Reject only the inserted paragraph mark: P2 and P3 join, the three
        // run revisions sharing its id stay.
        let mut word_doc = original.clone();
        let snapshots = revision_snapshots(&word_doc);
        let selected = select_revisions(&snapshots, &range_selector((2, 9), (2, 9)));
        assert_eq!(selected.len(), 1);
        assert!(selected[0].paragraph_mark);
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![decide_targets(false, revision_targets_for(&selected))],
            },
        )
        .expect("reject the shared-id paragraph mark");
        assert_eq!(word_doc.paragraph_count(), 3);
        assert_eq!(paragraph_text(&word_doc, 2), "Mark headmark tail");
        assert_eq!(revision_spans(&word_doc), spans[..3]);

        // A kind filter (deletions) under the shared id takes only the deletion.
        let mut word_doc = original.clone();
        let snapshots = revision_snapshots(&word_doc);
        let selected = select_revisions(
            &snapshots,
            &RevisionSelector {
                kinds: Some(vec!["deleted".to_string()]),
                ..RevisionSelector::default()
            },
        );
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![decide_targets(true, revision_targets_for(&selected))],
            },
        )
        .expect("accept the shared-id deletion");
        assert_eq!(paragraph_text(&word_doc, 0), "Keep first and  end");
        assert_eq!(
            revision_spans(&word_doc),
            [
                (0, 5, "inserted".to_string()),
                (1, 0, "inserted".to_string()),
                (2, 9, "inserted".to_string()),
            ]
        );
    }

    /// A target whose id no longer matches the revision at its position (a
    /// stale target) decides nothing rather than something else.
    #[test]
    fn targeted_decision_ignores_a_span_whose_id_does_not_match() {
        let mut word_doc = review_kinds_document();
        apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![decide_targets(
                    true,
                    vec![HeadlessRevisionTarget {
                        paragraph: 0,
                        start_offset: 5,
                        end_offset: 15,
                        paragraph_mark: false,
                        id: Some("2".to_string()),
                    }],
                )],
            },
        )
        .expect("stale target applies as a no-op");
        assert_eq!(revision_ids(&word_doc).len(), 7);
        assert!(apply_headless_commands_to_document(
            &mut word_doc,
            &HeadlessEditRequest {
                commands: vec![decide_targets(
                    true,
                    vec![HeadlessRevisionTarget {
                        paragraph: 99,
                        start_offset: 0,
                        end_offset: 1,
                        paragraph_mark: false,
                        id: None,
                    }],
                )],
            },
        )
        .is_err());
    }

    fn decide_ids(accept: bool, revision_id: Option<&str>, ids: &[&str]) -> HeadlessCommand {
        let revision_id = revision_id.map(str::to_string);
        let revision_ids = Some(ids.iter().map(|id| id.to_string()).collect());
        if accept {
            HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id,
                revision_ids,
                revision_targets: None,
            }
        } else {
            HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id,
                revision_ids,
                revision_targets: None,
            }
        }
    }

    #[test]
    fn accepting_a_revision_id_set_is_one_undoable_step() {
        let mut word_doc = review_kinds_document();
        let mut executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![decide_ids(true, None, &["1", "3", "7"])],
            },
        )
        .expect("accept by id set");
        assert_eq!(revision_ids(&word_doc), ["2", "4", "5", "6"]);
        assert_eq!(
            paragraph_text(&word_doc, 0),
            "Base inserted-A mid deleted-B end"
        );
        // Accepting a move source removes the moved words from there.
        assert_eq!(paragraph_text(&word_doc, 1), " stays");
        // Accepting an inserted paragraph mark keeps both paragraphs.
        assert_eq!(word_doc.paragraph_count(), 7);
        assert_eq!(executor.undo_depth(), 1);

        let mut selection = None;
        let mut context = CommandContext::new(&mut word_doc, &mut selection);
        assert!(executor.undo(&mut context).expect("undo succeeds"));
        assert_eq!(revision_ids(&word_doc), ["1", "2", "3", "4", "5", "6", "7"]);
        assert_eq!(paragraph_text(&word_doc, 1), "moved words stays");

        let mut context = CommandContext::new(&mut word_doc, &mut selection);
        assert!(executor.redo(&mut context).expect("redo succeeds"));
        assert_eq!(revision_ids(&word_doc), ["2", "4", "5", "6"]);
        assert_eq!(paragraph_text(&word_doc, 1), " stays");
    }

    #[test]
    fn rejecting_a_revision_id_set_joins_an_inserted_paragraph_mark() {
        let mut word_doc = review_kinds_document();
        let mut executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![decide_ids(false, None, &["2", "5", "7"])],
            },
        )
        .expect("reject by id set");
        assert_eq!(revision_ids(&word_doc), ["1", "3", "4", "6"]);
        // Rejecting a deletion restores the text as plain text.
        assert_eq!(
            paragraph_text(&word_doc, 0),
            "Base inserted-A mid deleted-B end"
        );
        // Rejecting an inserted page break removes it.
        assert_eq!(paragraph_text(&word_doc, 3), "After inserted break");
        // Rejecting an inserted paragraph mark joins the paragraphs.
        assert_eq!(word_doc.paragraph_count(), 6);
        assert_eq!(paragraph_text(&word_doc, 5), "Split headsplit tail");
        assert_eq!(executor.undo_depth(), 1);

        let mut selection = None;
        let mut context = CommandContext::new(&mut word_doc, &mut selection);
        assert!(executor.undo(&mut context).expect("undo succeeds"));
        assert_eq!(word_doc.paragraph_count(), 7);
        assert_eq!(revision_ids(&word_doc), ["1", "2", "3", "4", "5", "6", "7"]);
    }

    #[test]
    fn revision_id_filters_intersect_and_an_empty_set_records_nothing() {
        let mut word_doc = review_kinds_document();
        let mut executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![
                    decide_ids(true, None, &[]),
                    decide_ids(true, Some("1"), &["2"]),
                ],
            },
        )
        .expect("empty decisions succeed");
        assert_eq!(revision_ids(&word_doc).len(), 7);
        assert_eq!(
            executor.undo_depth(),
            0,
            "no-op decisions leave no undo step"
        );

        apply_headless_commands_with_executor_to_document(
            &mut word_doc,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![decide_ids(true, Some("1"), &["1", "2"])],
            },
        )
        .expect("intersection succeeds");
        assert_eq!(revision_ids(&word_doc), ["2", "3", "4", "5", "6", "7"]);
    }

    #[test]
    fn revision_ids_deserialize_as_an_optional_field() {
        let command: HeadlessCommand = serde_json::from_str(
            r#"{"type":"accept_revisions","kind":null,"start_paragraph":null,"end_paragraph":null}"#,
        )
        .expect("legacy command parses");
        assert!(matches!(
            command,
            HeadlessCommand::AcceptRevisions {
                revision_ids: None,
                revision_targets: None,
                ..
            }
        ));
        let command: HeadlessCommand = serde_json::from_str(
            r#"{"type":"reject_revisions","kind":null,"start_paragraph":null,"end_paragraph":null,"revision_ids":["3","4"]}"#,
        )
        .expect("id list parses");
        assert!(matches!(
            command,
            HeadlessCommand::RejectRevisions {
                revision_ids: Some(ref ids),
                revision_targets: None,
                ..
            } if ids == &["3".to_string(), "4".to_string()]
        ));
    }

    fn fixture_document(name: &str) -> WordDocument {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures")
                .join(name),
        )
        .expect("read fixture");
        executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load fixture")
    }

    fn apply_json_commands(
        word_doc: &mut WordDocument,
        executor: &mut CommandExecutor,
        commands: &str,
    ) -> Result<Option<SelectionRange>, HeadlessApiApplyError> {
        let request: HeadlessEditRequest =
            serde_json::from_str(&format!(r#"{{"commands":{commands}}}"#)).expect("request parses");
        apply_headless_commands_with_executor_to_document(word_doc, executor, None, &request)
    }

    /// Paragraph indices `paragraph()` resolves to nothing.
    fn unresolved_paragraphs(word_doc: &WordDocument) -> Vec<usize> {
        (0..word_doc.paragraph_count())
            .filter(|&index| word_doc.paragraph(index).is_none())
            .collect()
    }

    /// The host run of the text box that holds paragraph surface `index`.
    fn text_box_host(word_doc: &WordDocument, index: usize) -> Option<(usize, usize)> {
        word_doc
            .text_box_host_for_surface(index)
            .map(|target| (target.paragraph_index, target.run_index))
    }

    /// `textbox.docx`: body paragraph 0 hosts a text box whose paragraph is
    /// surface 1. Striking part of the host splits its runs and moves the box
    /// to another run index; the box's surface has to follow it, or every
    /// later command that walks a paragraph range over the box — accepting or
    /// rejecting all changes, for one — fails with `ParagraphUnavailable`.
    #[test]
    fn formatting_a_text_box_host_keeps_the_box_paragraph_addressable() {
        for decision in ["accept_revisions", "reject_revisions"] {
            let mut word_doc = fixture_document("textbox.docx");
            assert_eq!(text_box_host(&word_doc, 1), Some((0, 1)));
            let mut executor = CommandExecutor::new();
            apply_json_commands(
                &mut word_doc,
                &mut executor,
                r#"[{"type":"apply_strike","start_paragraph":0,"start_offset":0,"end_paragraph":0,"end_offset":2}]"#,
            )
            .expect("strike succeeds");

            let host_runs: Vec<_> = word_doc
                .paragraph(0)
                .expect("host paragraph")
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect();
            assert_eq!(host_runs, ["He", "llo", ""], "the strike split the host");
            assert_eq!(text_box_host(&word_doc, 1), Some((0, 2)));
            assert_eq!(unresolved_paragraphs(&word_doc), Vec::<usize>::new());
            assert_eq!(paragraph_text(&word_doc, 1), "Hello");

            apply_json_commands(
                &mut word_doc,
                &mut executor,
                &format!(r#"[{{"type":"{decision}"}}]"#),
            )
            .unwrap_or_else(|error| panic!("{decision} fails: {error:?}"));

            undo_all(&mut word_doc, &mut executor, &mut None);
            assert_eq!(text_box_host(&word_doc, 1), Some((0, 1)));
            assert_eq!(unresolved_paragraphs(&word_doc), Vec::<usize>::new());
        }
    }

    /// One selection from the host, through the box's paragraph, into the
    /// next body paragraph: the strike splits the host's runs first and then
    /// has to find the box's paragraph at its new run index, in the same
    /// command — and so does its undo.
    #[test]
    fn a_strike_across_a_text_box_host_reaches_the_box_paragraph() {
        let mut word_doc = fixture_document("textbox.docx");
        let mut executor = CommandExecutor::new();
        apply_json_commands(
            &mut word_doc,
            &mut executor,
            r#"[{"type":"apply_strike","start_paragraph":0,"start_offset":2,"end_paragraph":2,"end_offset":3}]"#,
        )
        .expect("strike succeeds");

        let struck = |word_doc: &WordDocument, index: usize| -> Vec<(String, bool)> {
            word_doc
                .paragraph(index)
                .expect("paragraph resolves")
                .runs
                .iter()
                .filter(|run| run.text_box.is_none())
                .map(|run| {
                    let strike = run
                        .properties
                        .as_ref()
                        .and_then(|properties| properties.strike);
                    (run.text.clone(), strike == Some(true))
                })
                .collect()
        };
        let text = |text: &str, strike: bool| (text.to_string(), strike);
        assert_eq!(struck(&word_doc, 0), [text("He", false), text("llo", true)]);
        assert_eq!(struck(&word_doc, 1), [text("Hello", true)]);
        assert_eq!(struck(&word_doc, 2), [text("Wor", true), text("ld", false)]);

        undo_all(&mut word_doc, &mut executor, &mut None);
        assert_eq!(text_box_host(&word_doc, 1), Some((0, 1)));
        assert_eq!(struck(&word_doc, 0), [text("Hello", false)]);
        assert_eq!(struck(&word_doc, 1), [text("Hello", false)]);
        assert_eq!(struck(&word_doc, 2), [text("World", false)]);
    }

    /// `math.docx` has block equations (`m:oMathPara`) between paragraphs.
    /// Each holds a paragraph index with no paragraph behind it, which a
    /// decision over the whole document — and its undo — has to step over
    /// rather than refuse.
    #[test]
    fn deciding_every_revision_steps_over_block_equations() {
        for (decision, keeps_insertion) in [("accept_revisions", true), ("reject_revisions", false)]
        {
            let mut word_doc = fixture_document("math.docx");
            let equations = unresolved_paragraphs(&word_doc);
            assert!(!equations.is_empty(), "the fixture has block equations");
            assert!(equations
                .iter()
                .all(|&index| word_doc.is_math_surface(index)));
            let original = paragraph_text(&word_doc, 0);

            let mut executor = CommandExecutor::new();
            apply_json_commands(
                &mut word_doc,
                &mut executor,
                r#"[{"type":"start_change_tracking","author":"Reviewer"},{"type":"insert_text","paragraph":0,"offset":0,"text":"X"}]"#,
            )
            .expect("tracked insertion succeeds");
            assert_eq!(revision_ids(&word_doc).len(), 1);

            apply_json_commands(
                &mut word_doc,
                &mut executor,
                &format!(r#"[{{"type":"{decision}"}}]"#),
            )
            .unwrap_or_else(|error| panic!("{decision} fails: {error:?}"));
            assert!(revision_ids(&word_doc).is_empty());
            let decided = if keeps_insertion {
                format!("X{original}")
            } else {
                original.clone()
            };
            assert_eq!(paragraph_text(&word_doc, 0), decided, "{decision}");

            let mut selection = None;
            let mut context = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut context).expect("undo succeeds"));
            assert_eq!(revision_ids(&word_doc).len(), 1);
            assert_eq!(paragraph_text(&word_doc, 0), format!("X{original}"));
            assert_eq!(unresolved_paragraphs(&word_doc), equations);
        }
    }
}
