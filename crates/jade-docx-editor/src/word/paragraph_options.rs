//! The paragraph settings Word keeps in dialogs of their own, and the ruler:
//!
//! - タブとリーダー: tab stops with their alignment and leader, set, cleared
//!   and moved — from the dialog or by clicking and dragging on the ruler —
//!   and the default tab interval (既定値);
//! - 線種とページ罫線と網かけの設定: the border box, line by line, and the
//!   shading's pattern and colours;
//! - 段落's switches: 同じスタイルの場合は段落間にスペースを追加しない, 行番号を
//!   表示しない, 段落の自動ハイフネーションを行わない;
//! - ドロップキャップ, and 均等割り付け (distributed alignment).
//!
//! Both shells drive these through the same calls; each lands as one undo
//! step and is written as Word writes it (see the engine's
//! `SetParagraphOptions`).

use crate::document::{ParagraphProperties, TabStop};
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessDropCapKind, HeadlessParagraphBorderBox, HeadlessParagraphBorderLine,
    HeadlessParagraphShading, HeadlessTabStop,
};

use super::package::{edit_package, PartKind};
use super::xml::Element;
use super::WordSession;

/// How text lines up on a tab stop (配置).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TabAlignment {
    Left,
    Center,
    Right,
    Decimal,
    Bar,
}

impl TabAlignment {
    pub const ALL: [TabAlignment; 5] = [
        TabAlignment::Left,
        TabAlignment::Center,
        TabAlignment::Right,
        TabAlignment::Decimal,
        TabAlignment::Bar,
    ];

    pub fn token(self) -> &'static str {
        match self {
            TabAlignment::Left => "left",
            TabAlignment::Center => "center",
            TabAlignment::Right => "right",
            TabAlignment::Decimal => "decimal",
            TabAlignment::Bar => "bar",
        }
    }

    pub fn from_token(token: &str) -> TabAlignment {
        match token {
            "center" => TabAlignment::Center,
            "right" | "end" => TabAlignment::Right,
            "decimal" => TabAlignment::Decimal,
            "bar" => TabAlignment::Bar,
            _ => TabAlignment::Left,
        }
    }

    /// The next on the ruler's tab selector, which clicking steps through as
    /// Word's does.
    pub fn next(self) -> TabAlignment {
        let index = TabAlignment::ALL
            .iter()
            .position(|a| *a == self)
            .unwrap_or(0);
        TabAlignment::ALL[(index + 1) % TabAlignment::ALL.len()]
    }
}

/// What fills the space before a tab stop (リーダー).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TabLeader {
    None,
    Dot,
    Hyphen,
    Underscore,
    MiddleDot,
}

impl TabLeader {
    pub const ALL: [TabLeader; 5] = [
        TabLeader::None,
        TabLeader::Dot,
        TabLeader::Hyphen,
        TabLeader::Underscore,
        TabLeader::MiddleDot,
    ];

    pub fn token(self) -> &'static str {
        match self {
            TabLeader::None => "none",
            TabLeader::Dot => "dot",
            TabLeader::Hyphen => "hyphen",
            TabLeader::Underscore => "underscore",
            TabLeader::MiddleDot => "middleDot",
        }
    }

    pub fn from_token(token: Option<&str>) -> TabLeader {
        match token {
            Some("dot") => TabLeader::Dot,
            Some("hyphen") => TabLeader::Hyphen,
            Some("underscore") | Some("heavy") => TabLeader::Underscore,
            Some("middleDot") => TabLeader::MiddleDot,
            _ => TabLeader::None,
        }
    }

    /// How the dialog shows it: Word's own sample of each.
    pub fn sample(self) -> &'static str {
        match self {
            TabLeader::None => "",
            TabLeader::Dot => "……",
            TabLeader::Hyphen => "------",
            TabLeader::Underscore => "______",
            TabLeader::MiddleDot => "・・・・",
        }
    }
}

/// One tab stop, its position in points from the margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tab {
    pub position: f32,
    pub alignment: TabAlignment,
    pub leader: TabLeader,
}

impl Tab {
    pub fn new(position: f32, alignment: TabAlignment, leader: TabLeader) -> Self {
        Tab {
            position,
            alignment,
            leader,
        }
    }

    fn headless(&self) -> HeadlessTabStop {
        HeadlessTabStop {
            position: self.position,
            alignment: Some(self.alignment.token().to_string()),
            leader: Some(self.leader.token().to_string()),
        }
    }
}

/// The on/off switches of the 段落 dialog this module sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParagraphSwitch {
    /// 同じスタイルの場合は段落間にスペースを追加しない.
    ContextualSpacing,
    /// 行番号を表示しない.
    SuppressLineNumbers,
    /// 段落の自動ハイフネーションを行わない.
    DontHyphenate,
}

/// One line of the border box.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderLine {
    /// An `ST_Border` token (`single`, `double`, `dotted`, …).
    pub style: String,
    /// Points.
    pub width: f32,
    /// `RRGGBB`, `None` for 自動.
    pub color: Option<String>,
    /// Distance from the text, points.
    pub space: f32,
}

impl BorderLine {
    pub fn new(style: &str, width: f32, color: Option<&str>) -> Self {
        BorderLine {
            style: style.to_string(),
            width,
            color: color.map(|color| color.trim_start_matches('#').to_ascii_uppercase()),
            space: 1.0,
        }
    }

    fn headless(&self) -> HeadlessParagraphBorderLine {
        HeadlessParagraphBorderLine {
            style: self.style.clone(),
            width: self.width,
            color: self.color.as_ref().map(|color| format!("#{color}")),
            space: Some(self.space),
        }
    }

    fn of(edge: &crate::document::BorderEdge) -> Option<BorderLine> {
        if !edge.is_visible() {
            return None;
        }
        Some(BorderLine {
            style: edge.val.clone().unwrap_or_else(|| "single".to_string()),
            width: edge.size_eighths.unwrap_or(4) as f32 / 8.0,
            color: edge
                .color
                .as_deref()
                .filter(|color| !color.eq_ignore_ascii_case("auto"))
                .map(|color| color.trim_start_matches('#').to_ascii_uppercase()),
            space: edge.space.unwrap_or(0) as f32,
        })
    }
}

/// The border box: each side, and the line between paragraphs that share it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BorderBox {
    pub top: Option<BorderLine>,
    pub bottom: Option<BorderLine>,
    pub left: Option<BorderLine>,
    pub right: Option<BorderLine>,
    pub between: Option<BorderLine>,
}

impl BorderBox {
    /// 囲む: the four sides in one line.
    pub fn boxed(line: BorderLine) -> Self {
        BorderBox {
            top: Some(line.clone()),
            bottom: Some(line.clone()),
            left: Some(line.clone()),
            right: Some(line),
            between: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self == &BorderBox::default()
    }

    fn headless(&self) -> HeadlessParagraphBorderBox {
        let line = |line: &Option<BorderLine>| line.as_ref().map(BorderLine::headless);
        HeadlessParagraphBorderBox {
            top: line(&self.top),
            bottom: line(&self.bottom),
            left: line(&self.left),
            right: line(&self.right),
            between: line(&self.between),
        }
    }
}

/// 網かけ: a fill colour under a pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadingSpec {
    /// `clear` for the fill alone, `pct10`, `solid`, `horzStripe`, …
    pub pattern: String,
    /// The pattern's colour, `RRGGBB`; `None` for 自動.
    pub color: Option<String>,
    /// The fill, `RRGGBB`; `None` for 色なし.
    pub fill: Option<String>,
}

/// What the dialogs show for the paragraph at the caret.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphOptionsState {
    pub tabs: Vec<Tab>,
    pub contextual_spacing: bool,
    pub suppress_line_numbers: bool,
    pub dont_hyphenate: bool,
    pub borders: BorderBox,
    pub shading: Option<ShadingSpec>,
    /// `left`, `center`, `right`, `both`, `distribute`, …
    pub alignment: Option<String>,
    /// The drop cap, and how many lines it drops.
    pub drop_cap: Option<(DropCapPosition, u8)>,
    /// The document's default tab interval, points (既定値).
    pub default_tab: f32,
}

/// Where a drop cap sits (ドロップキャップ's 本文内に表示 / 余白に表示).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropCapPosition {
    Dropped,
    InMargin,
}

/// A paragraph's effective tab stops, as the layout reads them.
pub fn effective_tabs(tabs: &[TabStop]) -> Vec<Tab> {
    let mut out: Vec<Tab> = Vec::new();
    for tab in tabs {
        let Some(position) = tab.position else {
            continue;
        };
        out.retain(|existing| (existing.position - position).abs() > 0.01);
        if tab.kind.as_deref() == Some("clear") {
            continue;
        }
        out.push(Tab {
            position,
            alignment: TabAlignment::from_token(tab.kind.as_deref().unwrap_or("left")),
            leader: TabLeader::from_token(tab.leader.as_deref()),
        });
    }
    out.sort_by(|a, b| a.position.total_cmp(&b.position));
    out
}

/// What the dialogs read from a paragraph's properties.
pub fn options_of(props: &ParagraphProperties) -> ParagraphOptionsState {
    let borders = props
        .borders
        .as_ref()
        .map(|borders| {
            let line =
                |edge: &Option<crate::document::BorderEdge>| edge.as_ref().and_then(BorderLine::of);
            BorderBox {
                top: line(&borders.top),
                bottom: line(&borders.bottom),
                left: line(&borders.left),
                right: line(&borders.right),
                between: line(&borders.between),
            }
        })
        .unwrap_or_default();
    let hex = |value: Option<&str>| {
        value
            .filter(|value| !value.eq_ignore_ascii_case("auto"))
            .map(|value| value.trim_start_matches('#').to_ascii_uppercase())
    };
    let shading = props.shading.as_ref().and_then(|shading| {
        let spec = ShadingSpec {
            pattern: shading.val.clone().unwrap_or_else(|| "clear".to_string()),
            color: hex(shading.color.as_deref()),
            fill: hex(shading.fill.as_deref()),
        };
        (spec.fill.is_some() || spec.pattern != "clear").then_some(spec)
    });
    ParagraphOptionsState {
        tabs: effective_tabs(&props.tabs),
        contextual_spacing: props.contextual_spacing.unwrap_or(false),
        suppress_line_numbers: props.suppress_line_numbers.unwrap_or(false),
        dont_hyphenate: props.suppress_auto_hyphens.unwrap_or(false),
        borders,
        shading,
        alignment: props.alignment.as_deref().map(str::to_string),
        drop_cap: props.drop_cap.map(|drop_cap| {
            (
                match drop_cap.kind {
                    crate::document::DropCapKind::Drop => DropCapPosition::Dropped,
                    crate::document::DropCapKind::Margin => DropCapPosition::InMargin,
                },
                drop_cap.lines,
            )
        }),
        default_tab: 0.0,
    }
}

/// A paragraph's indents as the ruler shows them, in points from the
/// margins: the left edge, the right edge, and the first line relative to
/// the left edge (negative for a hanging indent).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RulerIndents {
    pub left: f32,
    pub right: f32,
    pub first_line: f32,
}

/// The indents of `props`, a character being `char_width` points.
pub fn ruler_indents_of(props: &ParagraphProperties, char_width: f32) -> RulerIndents {
    let Some(indent) = props.indent.as_ref() else {
        return RulerIndents::default();
    };
    let chars = |value: Option<i32>| value.map(|hundredths| hundredths as f32 / 100.0 * char_width);
    let left = chars(indent.left_chars.or(indent.start_chars))
        .or(indent.physical_left(false))
        .unwrap_or(0.0);
    let right = chars(indent.right_chars.or(indent.end_chars))
        .or(indent.end.or(indent.right))
        .unwrap_or(0.0);
    let first_line = chars(indent.first_line_chars)
        .or(indent.first_line)
        .or_else(|| {
            chars(indent.hanging_chars)
                .or(indent.hanging)
                .map(|hanging| -hanging)
        })
        .unwrap_or(0.0);
    RulerIndents {
        left,
        right,
        first_line,
    }
}

/// The paragraph commands of this module, for every session.
pub trait WordParagraphOptions: WordSession {
    /// The paragraphs the selection touches, or the caret's.
    fn option_span(&self) -> (usize, usize) {
        match self.selection_range() {
            Some(range) if !range.is_collapsed() => {
                let (start, end) = range.normalized();
                (start.paragraph, end.paragraph)
            }
            Some(range) => (range.focus.paragraph, range.focus.paragraph),
            None => (0, 0),
        }
    }

    /// What the dialogs show: the first paragraph of the selection.
    fn paragraph_options(&self) -> ParagraphOptionsState {
        let (start, _) = self.option_span();
        let mut state = self
            .word_document()
            .paragraph(start)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .map(options_of)
            .unwrap_or_default();
        state.default_tab = self.default_tab_stop();
        state
    }

    /// The document's default tab interval in points (Word's 36 when the
    /// settings name none).
    fn default_tab_stop(&self) -> f32 {
        self.word_document()
            .document
            .as_ref()
            .and_then(|document| document.default_tab_stop)
            .filter(|value| *value > 0.0)
            .unwrap_or(36.0)
    }

    fn options_command(&mut self, build: impl FnOnce(&mut OptionsCommand)) -> Result<(), String> {
        let (start, end) = self.option_span();
        let mut command = OptionsCommand::default();
        build(&mut command);
        self.apply_command(
            HeadlessCommand::SetParagraphOptions {
                start_paragraph: start,
                end_paragraph: end,
                tabs: command.tabs,
                contextual_spacing: command.contextual_spacing,
                suppress_line_numbers: command.suppress_line_numbers,
                suppress_auto_hyphens: command.suppress_auto_hyphens,
                borders: command.borders,
                shading: command.shading,
            },
            true,
        )
    }

    /// The tab stops the dialog ends with, replacing the paragraphs'.
    fn set_tabs(&mut self, tabs: &[Tab]) -> Result<(), String> {
        let tabs: Vec<HeadlessTabStop> = tabs.iter().map(Tab::headless).collect();
        self.options_command(|command| command.tabs = Some(tabs))
    }

    /// 設定: adds a stop, or changes the one at that position.
    fn set_tab(&mut self, tab: Tab) -> Result<(), String> {
        let mut tabs = self.paragraph_options().tabs;
        tabs.retain(|existing| (existing.position - tab.position).abs() > 0.5);
        tabs.push(tab);
        self.set_tabs(&tabs)
    }

    /// クリア: takes the stop at `position` off.
    fn clear_tab(&mut self, position: f32) -> Result<(), String> {
        let mut tabs = self.paragraph_options().tabs;
        tabs.retain(|existing| (existing.position - position).abs() > 0.5);
        self.set_tabs(&tabs)
    }

    /// すべてクリア.
    fn clear_all_tabs(&mut self) -> Result<(), String> {
        self.set_tabs(&[])
    }

    /// A stop dragged along the ruler: off the ruler (`to: None`) it is
    /// cleared, as in Word.
    fn move_tab(&mut self, from: f32, to: Option<f32>) -> Result<(), String> {
        let mut tabs = self.paragraph_options().tabs;
        let Some(index) = tabs
            .iter()
            .position(|tab| (tab.position - from).abs() <= 0.5)
        else {
            return Err("no tab stop there".to_string());
        };
        match to {
            Some(to) => tabs[index].position = to.max(0.0),
            None => {
                tabs.remove(index);
            }
        }
        self.set_tabs(&tabs)
    }

    fn set_paragraph_switch(&mut self, switch: ParagraphSwitch, on: bool) -> Result<(), String> {
        self.options_command(|command| match switch {
            ParagraphSwitch::ContextualSpacing => command.contextual_spacing = Some(on),
            ParagraphSwitch::SuppressLineNumbers => command.suppress_line_numbers = Some(on),
            ParagraphSwitch::DontHyphenate => command.suppress_auto_hyphens = Some(on),
        })
    }

    /// The border box; an empty one takes the borders off (罫線なし).
    fn set_border_box(&mut self, borders: &BorderBox) -> Result<(), String> {
        let borders = borders.headless();
        self.options_command(|command| command.borders = Some(borders))
    }

    /// 網かけ, or none.
    fn set_shading_spec(&mut self, shading: Option<&ShadingSpec>) -> Result<(), String> {
        let shading = shading.map(|shading| HeadlessParagraphShading {
            pattern: Some(shading.pattern.clone()),
            color: shading.color.as_ref().map(|color| format!("#{color}")),
            fill: shading.fill.as_ref().map(|fill| format!("#{fill}")),
        });
        self.options_command(|command| command.shading = Some(shading))
    }

    /// ドロップキャップ: `None` takes it off (なし).
    fn set_drop_cap(&mut self, drop_cap: Option<(DropCapPosition, u8)>) -> Result<(), String> {
        let (start, end) = self.option_span();
        self.apply_command(
            HeadlessCommand::SetDropCap {
                start_paragraph: start,
                end_paragraph: end,
                kind: drop_cap.map(|(position, _)| match position {
                    DropCapPosition::Dropped => HeadlessDropCapKind::Drop,
                    DropCapPosition::InMargin => HeadlessDropCapKind::Margin,
                }),
                lines: drop_cap.map_or(3, |(_, lines)| lines.clamp(1, 10)),
            },
            true,
        )
    }

    /// The caret paragraph's indents, for the ruler.
    fn ruler_indents(&self, char_width: f32) -> RulerIndents {
        let (start, _) = self.option_span();
        self.word_document()
            .paragraph(start)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .map(|props| ruler_indents_of(props, char_width))
            .unwrap_or_default()
    }

    /// An indent marker dragged on the ruler: the paragraphs take these
    /// indents, in points.
    fn set_ruler_indents(&mut self, indents: RulerIndents) -> Result<(), String> {
        let (start, end) = self.option_span();
        let (first_line, hanging) = if indents.first_line < 0.0 {
            (None, Some(-indents.first_line))
        } else {
            (Some(indents.first_line), None)
        };
        self.apply_command(
            HeadlessCommand::SetParagraphMetrics {
                start_paragraph: start,
                end_paragraph: end,
                left: Some(indents.left),
                right: Some(indents.right),
                before: None,
                after: None,
                first_line,
                hanging,
                clear_special: first_line == Some(0.0),
            },
            true,
        )
    }

    /// 既定値: the document's default tab interval, in points.
    fn set_default_tab_stop(&mut self, points: f32) -> Result<(), String> {
        if !points.is_finite() || !(1.0..=1584.0).contains(&points) {
            return Err("the default tab interval must be between 1 and 1584 points".into());
        }
        let twips = ((points * 20.0).round() as i64).to_string();
        edit_package(
            self,
            |package| {
                package.edit(PartKind::Settings, |settings| {
                    settings.put(
                        Element::with_val("w:defaultTabStop", &twips),
                        SETTINGS_ORDER,
                    );
                    Ok(())
                })
            },
            Vec::new(),
        )
    }
}

impl<T: WordSession + ?Sized> WordParagraphOptions for T {}

/// The fields of the engine's `SetParagraphOptions`, filled in by a call.
#[derive(Default)]
pub struct OptionsCommand {
    pub tabs: Option<Vec<HeadlessTabStop>>,
    pub contextual_spacing: Option<bool>,
    pub suppress_line_numbers: Option<bool>,
    pub suppress_auto_hyphens: Option<bool>,
    pub borders: Option<HeadlessParagraphBorderBox>,
    pub shading: Option<Option<HeadlessParagraphShading>>,
}

/// `CT_Settings`' sequence, as far as the settings this editor writes go.
pub(crate) const SETTINGS_ORDER: &[&str] = &[
    "w:writeProtection",
    "w:view",
    "w:zoom",
    "w:removePersonalInformation",
    "w:removeDateAndTime",
    "w:doNotDisplayPageBoundaries",
    "w:displayBackgroundShape",
    "w:printPostScriptOverText",
    "w:printFractionalCharacterWidth",
    "w:printFormsData",
    "w:embedTrueTypeFonts",
    "w:embedSystemFonts",
    "w:saveSubsetFonts",
    "w:saveFormsData",
    "w:mirrorMargins",
    "w:alignBordersAndEdges",
    "w:bordersDoNotSurroundHeader",
    "w:bordersDoNotSurroundFooter",
    "w:gutterAtTop",
    "w:hideSpellingErrors",
    "w:hideGrammaticalErrors",
    "w:activeWritingStyle",
    "w:proofState",
    "w:formsDesign",
    "w:attachedTemplate",
    "w:linkStyles",
    "w:stylePaneFormatFilter",
    "w:stylePaneSortMethod",
    "w:documentType",
    "w:mailMerge",
    "w:revisionView",
    "w:trackRevisions",
    "w:doNotTrackMoves",
    "w:doNotTrackFormatting",
    "w:documentProtection",
    "w:autoFormatOverride",
    "w:styleLockTheme",
    "w:styleLockQFSet",
    "w:defaultTabStop",
    "w:autoHyphenation",
    "w:consecutiveHyphenLimit",
    "w:hyphenationZone",
    "w:doNotHyphenateCaps",
    "w:showEnvelope",
    "w:summaryLength",
    "w:clickAndTypeStyle",
    "w:defaultTableStyle",
    "w:evenAndOddHeaders",
    "w:bookFoldRevPrinting",
    "w:bookFoldPrinting",
    "w:bookFoldPrintingSheets",
    "w:drawingGridHorizontalSpacing",
    "w:drawingGridVerticalSpacing",
    "w:displayHorizontalDrawingGridEvery",
    "w:displayVerticalDrawingGridEvery",
    "w:doNotUseMarginsForDrawingGridOrigin",
    "w:drawingGridHorizontalOrigin",
    "w:drawingGridVerticalOrigin",
    "w:doNotShadeFormData",
    "w:noPunctuationKerning",
    "w:characterSpacingControl",
    "w:printTwoOnOne",
    "w:strictFirstAndLastChars",
    "w:noLineBreaksAfter",
    "w:noLineBreaksBefore",
    "w:savePreviewPicture",
    "w:doNotValidateAgainstSchema",
    "w:saveInvalidXml",
    "w:ignoreMixedContent",
    "w:alwaysShowPlaceholderText",
    "w:doNotDemarcateInvalidXml",
    "w:saveXmlDataOnly",
    "w:useXSLTWhenSaving",
    "w:saveThroughXslt",
    "w:showXMLTags",
    "w:alwaysMergeEmptyNamespace",
    "w:updateFields",
    "w:hdrShapeDefaults",
    "w:footnotePr",
    "w:endnotePr",
    "w:compat",
    "w:docVars",
    "w:rsids",
    "m:mathPr",
    "w:attachedSchema",
    "w:themeFontLang",
    "w:clrSchemeMapping",
    "w:doNotIncludeSubdocsInStats",
    "w:doNotAutoCompressPictures",
    "w:forceUpgrade",
    "w:captions",
    "w:readModeInkLockDown",
    "w:smartTagType",
    "sl:schemaLibrary",
    "w:shapeDefaults",
    "w:doNotEmbedSmartTags",
    "w:decimalSymbol",
    "w:listSeparator",
];
