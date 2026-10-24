//! Draw command module for separating render (layout) from draw (painting).
//!
//! This module provides an intermediate representation for drawing operations.
//! Instead of directly calling Canvas API during paragraph rendering, we generate
//! a list of `DrawCommand`s that can be:
//!
//! 1. **Cached**: If a paragraph hasn't changed, we can replay its cached commands
//! 2. **Tested**: Commands can be inspected without a real canvas
//! 3. **Batched**: Commands can be optimized before execution
//!
//! ## Usage
//!
//! ```ignore
//! // During layout phase, collect commands
//! let commands = layout_paragraph(paragraph, context)?;
//!
//! // Cache the commands if the paragraph is unchanged
//! cache.insert(paragraph_index, commands.clone());
//!
//! // During draw phase, execute commands
//! execute_commands(&commands, canvas_context)?;
//! ```

pub mod executor;

use crate::document::FontHint;
use jade_docx_layout::layout_cache::ContentHash;
use serde::{Deserialize, Serialize};
use std::rc::Rc;

/// Text style information needed for drawing text.
/// This is a serializable version of AppliedRunStyle.
///
/// String fields use `Rc<str>` so that `DrawCommand::clone()` (used heavily
/// in cache replay and `with_y_offset`) is a cheap refcount bump instead of
/// a deep string copy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    /// Font size in points
    pub font_size_pt: f32,
    /// Letter spacing in CSS pixels
    pub letter_spacing_px: f64,
    /// Horizontal text scale ratio (`w:w`), where `1.0` is 100%.
    #[serde(default = "default_text_scale_x")]
    pub text_scale_x: f64,
    /// Whether pair kerning should be applied while measuring/drawing.
    #[serde(default)]
    pub enable_pair_kerning: bool,
    /// Baseline shift in CSS pixels (for superscript/subscript)
    pub baseline_shift_px: f64,
    /// Whether to draw strikethrough
    pub strike: bool,
    /// Whether to draw underline
    pub underline: bool,
    /// Whether to draw the glyphs with a drop shadow (`w:shadow`,
    /// ECMA-376 Part 1 §17.3.2.31).
    #[serde(default)]
    pub shadow: bool,
    /// Whether to draw the glyphs as hollow stroked outlines (`w:outline`,
    /// ECMA-376 Part 1 §17.3.2.23).
    #[serde(default)]
    pub outline: bool,
    /// Whether to draw the glyphs embossed/raised (`w:emboss`,
    /// ECMA-376 Part 1 §17.3.2.13).
    #[serde(default)]
    pub emboss: bool,
    /// Whether to draw the glyphs imprinted/engraved (`w:imprint`,
    /// ECMA-376 Part 1 §17.3.2.18).
    #[serde(default)]
    pub imprint: bool,
    /// CSS font descriptor for Latin (ASCII) script.
    /// Word uses w:ascii for codepoints in 0x00-0x7F.
    #[serde(default, alias = "font_descriptor_latin")]
    pub font_descriptor_latin_ascii: Rc<str>,
    /// CSS font descriptor for Latin (hAnsi) script.
    /// Word uses w:hAnsi for non-ASCII Latin codepoints.
    #[serde(default)]
    pub font_descriptor_latin_hansi: Rc<str>,
    /// CSS font descriptor for East Asian script
    pub font_descriptor_east_asia: Rc<str>,
    /// CSS font descriptor for Complex script
    pub font_descriptor_complex: Rc<str>,
    /// Optional font hint to override Latin script classification (w:hint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_hint: Option<FontHint>,
    /// Fill color (CSS color string)
    pub fill_color: Rc<str>,
    /// Highlight/background color (optional)
    pub highlight_color: Option<Rc<str>>,
    /// Word 2010's glow and coloured text outline (`w14:glow`,
    /// `w14:textOutline`), when the run has either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Box<TextEffectStyle>>,
}

/// `w14:glow` and `w14:textOutline` as the painter draws them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextEffectStyle {
    /// The glow's CSS colour (with its opacity) and blur radius in px.
    pub glow: Option<(Rc<str>, f64)>,
    /// The outline's CSS colour and line width in px, stroked over the fill.
    pub outline: Option<(Rc<str>, f64)>,
    /// 組み文字 / 割注 (`w:eastAsianLayout w:combine`): the run's text drawn
    /// as two lines, the first raised and the second lowered by these px,
    /// in the room the layout squeezed the run into.
    #[serde(default)]
    pub combine: Option<CombinedCharacters>,
    /// 囲い文字: an enclosure (○, □, △, ◇) drawn round the run's character,
    /// which is drawn smaller inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enclosure: Option<Rc<str>>,
}

/// How a 組み文字 / 割注 run is drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CombinedCharacters {
    /// px the first line's baseline is raised.
    pub raise: f64,
    /// px the second line's baseline is lowered.
    pub lower: f64,
    /// 割注's brackets, drawn at twice the run's (half) size round the lines.
    pub brackets: Option<(Rc<str>, Rc<str>)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GlyphRunStep {
    /// UTF-8 byte range within the owning `DrawCommand::TextRun::text`.
    pub text_start: u32,
    pub text_end: u32,
    pub font_index: u16,
    pub x_offset: f64,
}

impl GlyphRunStep {
    #[inline]
    pub fn text<'a>(&self, run_text: &'a str) -> Option<&'a str> {
        run_text.get(self.text_start as usize..self.text_end as usize)
    }
}

fn default_text_scale_x() -> f64 {
    1.0
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size_pt: 12.0,
            letter_spacing_px: 0.0,
            text_scale_x: 1.0,
            enable_pair_kerning: false,
            baseline_shift_px: 0.0,
            strike: false,
            underline: false,
            shadow: false,
            outline: false,
            emboss: false,
            imprint: false,
            font_descriptor_latin_ascii: Rc::from("12pt 'Times New Roman', serif"),
            font_descriptor_latin_hansi: Rc::from("12pt 'Times New Roman', serif"),
            font_descriptor_east_asia: Rc::from("12pt 'MS Mincho', serif"),
            font_descriptor_complex: Rc::from("12pt 'Times New Roman', serif"),
            fill_color: Rc::from("#000000"),
            highlight_color: None,
            font_hint: None,
            effects: None,
        }
    }
}

/// Tab leader style for drawing tab leader characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TabLeaderStyle {
    #[default]
    None,
    Dot,
    Hyphen,
    Underscore,
    Heavy,
    MiddleDot,
}

impl TabLeaderStyle {
    /// Returns the character used to draw this leader style.
    pub fn character(&self) -> &'static str {
        match self {
            TabLeaderStyle::Dot => ".",
            TabLeaderStyle::Hyphen => "-",
            TabLeaderStyle::Underscore => "_",
            TabLeaderStyle::Heavy => "—",
            TabLeaderStyle::MiddleDot => "·",
            TabLeaderStyle::None => "",
        }
    }

    /// Returns the name of this leader style (for serialization).
    pub fn name(&self) -> &'static str {
        match self {
            TabLeaderStyle::Dot => "dot",
            TabLeaderStyle::Hyphen => "hyphen",
            TabLeaderStyle::Underscore => "underscore",
            TabLeaderStyle::Heavy => "heavy",
            TabLeaderStyle::MiddleDot => "middleDot",
            TabLeaderStyle::None => "none",
        }
    }
}

/// A single drawing command that can be executed on a canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DrawCommand {
    /// Draw a positioned glyph run with a shared style.
    TextRun {
        text: Rc<str>,
        x: f64,
        y: f64,
        style: TextStyle,
        fonts: Vec<Rc<str>>,
        glyphs: Vec<GlyphRunStep>,
    },

    /// Draw text at a specific position.
    Text {
        text: Rc<str>,
        x: f64,
        y: f64,
        style: TextStyle,
    },

    /// Draw text with letter spacing.
    TextWithSpacing {
        text: Rc<str>,
        x: f64,
        y: f64,
        letter_spacing_px: f64,
        style: TextStyle,
    },

    /// Draw a strikethrough line. When `double` is true two parallel lines are
    /// drawn (`w:dstrike`, ECMA-376 Part 1 §17.3.2.9).
    StrikeThrough {
        x: f64,
        y: f64,
        width: f64,
        font_size_pt: f32,
        color: Rc<str>,
        #[serde(default)]
        double: bool,
    },

    /// Draw an underline. `kind` carries the `w:u@w:val` flavor ("double",
    /// "dotted", "wave", …); `None` draws the plain single rule.
    Underline {
        x: f64,
        y: f64,
        width: f64,
        font_size_pt: f32,
        color: Rc<str>,
        #[serde(default)]
        kind: Option<Rc<str>>,
    },

    /// Draw a wavy underline ("squiggly") used by lint-style decoration
    /// plugins. Geometry parameters mirror `Underline`; the executor
    /// picks wavelength / amplitude from `font_size_pt`.
    SquigglyUnderline {
        x: f64,
        y: f64,
        width: f64,
        font_size_pt: f32,
        color: Rc<str>,
    },

    /// Draw a single emphasis mark (`w:em`, ECMA-376 Part 1 §17.3.2.12) — a
    /// small circle centered at (`x`, `y`). `filled` selects a solid dot
    /// (dot/comma/underDot) versus an open ring (circle).
    EmphasisMark {
        x: f64,
        y: f64,
        radius: f64,
        filled: bool,
        color: Rc<str>,
    },

    /// Fill a rectangle with a color.
    FillRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: Rc<str>,
    },

    /// Stroke an open polyline (wavy and stroked border rules).
    StrokePolyline {
        points: Rc<[(f64, f64)]>,
        line_width: f64,
        color: Rc<str>,
    },

    /// Clear a rectangle back to transparent.
    ClearRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },

    /// Draw a selection highlight rectangle.
    SelectionHighlight {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },

    /// Draw a comment highlight rectangle.
    CommentHighlight {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },

    /// Draw tab leader characters between two x positions.
    TabLeader {
        start_x: f64,
        end_x: f64,
        baseline: f64,
        leader: TabLeaderStyle,
        glyph_positions: Rc<[f64]>,
        style: TextStyle,
    },

    /// Draw a formatting control mark (like paragraph end, line break).
    ControlMark {
        symbol: Rc<str>,
        x: f64,
        baseline: f64,
        style: TextStyle,
    },

    /// Draw a revision highlight (inserted/deleted text background).
    RevisionHighlight {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        is_inserted: bool,
    },

    /// Save the canvas state.
    Save,

    /// Clip subsequent drawing to a rectangle until a matching restore.
    ClipRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },

    /// Restore the canvas state.
    Restore,

    /// Set the fill style.
    SetFillStyle { color: Rc<str> },

    /// Set the font.
    SetFont { font: Rc<str> },
}

impl DrawCommand {
    /// Returns a new DrawCommand with the y coordinate offset by the given amount.
    /// This is useful for replaying cached commands at a different vertical position.
    pub fn with_y_offset(&self, offset: f64) -> Self {
        if offset.abs() < f64::EPSILON {
            return self.clone();
        }
        match self {
            DrawCommand::TextRun {
                text,
                x,
                y,
                style,
                fonts,
                glyphs,
            } => DrawCommand::TextRun {
                text: text.clone(),
                x: *x,
                y: *y + offset,
                style: style.clone(),
                fonts: fonts.clone(),
                glyphs: glyphs.clone(),
            },
            DrawCommand::Text { text, x, y, style } => DrawCommand::Text {
                text: text.clone(),
                x: *x,
                y: *y + offset,
                style: style.clone(),
            },
            DrawCommand::TextWithSpacing {
                text,
                x,
                y,
                letter_spacing_px,
                style,
            } => DrawCommand::TextWithSpacing {
                text: text.clone(),
                x: *x,
                y: *y + offset,
                letter_spacing_px: *letter_spacing_px,
                style: style.clone(),
            },
            DrawCommand::StrikeThrough {
                x,
                y,
                width,
                font_size_pt,
                color,
                double,
            } => DrawCommand::StrikeThrough {
                x: *x,
                y: *y + offset,
                width: *width,
                font_size_pt: *font_size_pt,
                color: color.clone(),
                double: *double,
            },
            DrawCommand::Underline {
                x,
                y,
                width,
                font_size_pt,
                color,
                kind,
            } => DrawCommand::Underline {
                x: *x,
                y: *y + offset,
                width: *width,
                font_size_pt: *font_size_pt,
                color: color.clone(),
                kind: kind.clone(),
            },
            DrawCommand::SquigglyUnderline {
                x,
                y,
                width,
                font_size_pt,
                color,
            } => DrawCommand::SquigglyUnderline {
                x: *x,
                y: *y + offset,
                width: *width,
                font_size_pt: *font_size_pt,
                color: color.clone(),
            },
            DrawCommand::EmphasisMark {
                x,
                y,
                radius,
                filled,
                color,
            } => DrawCommand::EmphasisMark {
                x: *x,
                y: *y + offset,
                radius: *radius,
                filled: *filled,
                color: color.clone(),
            },
            DrawCommand::FillRect {
                x,
                y,
                width,
                height,
                color,
            } => DrawCommand::FillRect {
                x: *x,
                y: *y + offset,
                width: *width,
                height: *height,
                color: color.clone(),
            },
            DrawCommand::StrokePolyline {
                points,
                line_width,
                color,
            } => DrawCommand::StrokePolyline {
                points: points.iter().map(|&(x, y)| (x, y + offset)).collect(),
                line_width: *line_width,
                color: color.clone(),
            },
            DrawCommand::ClearRect {
                x,
                y,
                width,
                height,
            } => DrawCommand::ClearRect {
                x: *x,
                y: *y + offset,
                width: *width,
                height: *height,
            },
            DrawCommand::SelectionHighlight {
                x,
                y,
                width,
                height,
            } => DrawCommand::SelectionHighlight {
                x: *x,
                y: *y + offset,
                width: *width,
                height: *height,
            },
            DrawCommand::CommentHighlight {
                x,
                y,
                width,
                height,
            } => DrawCommand::CommentHighlight {
                x: *x,
                y: *y + offset,
                width: *width,
                height: *height,
            },
            DrawCommand::TabLeader {
                start_x,
                end_x,
                baseline,
                leader,
                glyph_positions,
                style,
            } => DrawCommand::TabLeader {
                start_x: *start_x,
                end_x: *end_x,
                baseline: *baseline + offset,
                leader: *leader,
                glyph_positions: Rc::clone(glyph_positions),
                style: style.clone(),
            },
            DrawCommand::ControlMark {
                symbol,
                x,
                baseline,
                style,
            } => DrawCommand::ControlMark {
                symbol: symbol.clone(),
                x: *x,
                baseline: *baseline + offset,
                style: style.clone(),
            },
            DrawCommand::RevisionHighlight {
                x,
                y,
                width,
                height,
                is_inserted,
            } => DrawCommand::RevisionHighlight {
                x: *x,
                y: *y + offset,
                width: *width,
                height: *height,
                is_inserted: *is_inserted,
            },
            // State commands don't have y coordinates
            DrawCommand::Save => DrawCommand::Save,
            DrawCommand::ClipRect {
                x,
                y,
                width,
                height,
            } => DrawCommand::ClipRect {
                x: *x,
                y: *y + offset,
                width: *width,
                height: *height,
            },
            DrawCommand::Restore => DrawCommand::Restore,
            DrawCommand::SetFillStyle { color } => DrawCommand::SetFillStyle {
                color: color.clone(),
            },
            DrawCommand::SetFont { font } => DrawCommand::SetFont { font: font.clone() },
        }
    }
}

/// Result of laying out a paragraph, containing all commands needed to draw it.
#[derive(Clone, Debug, Default)]
pub struct ParagraphDrawCommands {
    /// The drawing commands for this paragraph.
    pub commands: Vec<DrawCommand>,
    /// The height consumed by this paragraph in CSS pixels.
    pub height: f64,
    /// The ending Y position after this paragraph.
    pub end_y: f64,
    /// The page number where this paragraph ends.
    pub end_page: usize,
}

impl ParagraphDrawCommands {
    /// Creates a new empty command list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a command list with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            commands: Vec::with_capacity(capacity),
            height: 0.0,
            end_y: 0.0,
            end_page: 0,
        }
    }

    /// Adds a command to the list.
    pub fn push(&mut self, command: DrawCommand) {
        self.commands.push(command);
    }

    /// Returns the number of commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Returns true if there are no commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Extends this command list with another.
    pub fn extend(&mut self, other: ParagraphDrawCommands) {
        self.commands.extend(other.commands);
    }

    /// Returns an iterator over the commands.
    pub fn iter(&self) -> impl Iterator<Item = &DrawCommand> {
        self.commands.iter()
    }
}

/// A command buffer that collects drawing commands during layout.
///
/// This is used during the layout phase to collect all drawing operations
/// without immediately executing them.
#[derive(Clone, Debug, Default)]
pub struct DrawCommandBuffer {
    commands: Vec<DrawCommand>,
}

impl DrawCommandBuffer {
    /// Creates a new empty command buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a buffer with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            commands: Vec::with_capacity(capacity),
        }
    }

    /// Adds a text drawing command.
    pub fn draw_text(&mut self, text: impl Into<Rc<str>>, x: f64, y: f64, style: TextStyle) {
        self.commands.push(DrawCommand::Text {
            text: text.into(),
            x,
            y,
            style,
        });
    }

    /// Adds a text with spacing drawing command.
    pub fn draw_text_with_spacing(
        &mut self,
        text: impl Into<Rc<str>>,
        x: f64,
        y: f64,
        letter_spacing_px: f64,
        style: TextStyle,
    ) {
        self.commands.push(DrawCommand::TextWithSpacing {
            text: text.into(),
            x,
            y,
            letter_spacing_px,
            style,
        });
    }

    /// Adds a strikethrough drawing command.
    pub fn draw_strike_through(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        font_size_pt: f32,
        color: impl Into<Rc<str>>,
        double: bool,
    ) {
        self.commands.push(DrawCommand::StrikeThrough {
            x,
            y,
            width,
            font_size_pt,
            color: color.into(),
            double,
        });
    }

    /// Adds an underline drawing command.
    pub fn draw_underline(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        font_size_pt: f32,
        color: impl Into<Rc<str>>,
    ) {
        self.commands.push(DrawCommand::Underline {
            x,
            y,
            width,
            font_size_pt,
            color: color.into(),
            kind: None,
        });
    }

    /// Adds a fill rectangle command.
    pub fn fill_rect(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: impl Into<Rc<str>>,
    ) {
        self.commands.push(DrawCommand::FillRect {
            x,
            y,
            width,
            height,
            color: color.into(),
        });
    }

    /// Adds a selection highlight command.
    pub fn draw_selection_highlight(&mut self, x: f64, y: f64, width: f64, height: f64) {
        self.commands.push(DrawCommand::SelectionHighlight {
            x,
            y,
            width,
            height,
        });
    }

    /// Adds a comment highlight command.
    pub fn draw_comment_highlight(&mut self, x: f64, y: f64, width: f64, height: f64) {
        self.commands.push(DrawCommand::CommentHighlight {
            x,
            y,
            width,
            height,
        });
    }

    /// Adds a tab leader command.
    pub fn draw_tab_leader(
        &mut self,
        span: (f64, f64),
        baseline: f64,
        leader: TabLeaderStyle,
        style: TextStyle,
        glyph_positions: Rc<[f64]>,
    ) {
        self.commands.push(DrawCommand::TabLeader {
            start_x: span.0,
            end_x: span.1,
            baseline,
            leader,
            glyph_positions,
            style,
        });
    }

    /// Adds a control mark command.
    pub fn draw_control_mark(
        &mut self,
        symbol: impl Into<Rc<str>>,
        x: f64,
        baseline: f64,
        style: TextStyle,
    ) {
        self.commands.push(DrawCommand::ControlMark {
            symbol: symbol.into(),
            x,
            baseline,
            style,
        });
    }

    /// Adds a revision highlight command.
    pub fn draw_revision_highlight(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        is_inserted: bool,
    ) {
        self.commands.push(DrawCommand::RevisionHighlight {
            x,
            y,
            width,
            height,
            is_inserted,
        });
    }

    /// Adds a save command.
    pub fn save(&mut self) {
        self.commands.push(DrawCommand::Save);
    }

    /// Adds a restore command.
    pub fn restore(&mut self) {
        self.commands.push(DrawCommand::Restore);
    }

    /// Sets the fill style.
    pub fn set_fill_style(&mut self, color: impl Into<Rc<str>>) {
        self.commands.push(DrawCommand::SetFillStyle {
            color: color.into(),
        });
    }

    /// Sets the font.
    pub fn set_font(&mut self, font: impl Into<Rc<str>>) {
        self.commands
            .push(DrawCommand::SetFont { font: font.into() });
    }

    /// Converts this buffer into a ParagraphDrawCommands.
    pub fn into_paragraph_commands(
        self,
        height: f64,
        end_y: f64,
        end_page: usize,
    ) -> ParagraphDrawCommands {
        ParagraphDrawCommands {
            commands: self.commands,
            height,
            end_y,
            end_page,
        }
    }

    /// Returns the commands without consuming the buffer.
    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    /// Returns the number of commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Returns true if there are no commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Clears all commands.
    pub fn clear(&mut self) {
        self.commands.clear();
    }
}

// =============================================================================
// Paragraph Draw Command Cache
// =============================================================================

/// Cached drawing commands for a paragraph.
///
/// This structure stores the draw commands along with the content hash
/// that was used to generate them. When re-rendering, if the content hash
/// matches, the cached commands can be replayed instead of re-computing.
#[derive(Clone, Debug)]
pub struct CachedParagraphCommands {
    /// The content hash when these commands were generated.
    pub content_hash: ContentHash,
    /// The drawing commands.
    pub commands: Vec<DrawCommand>,
    /// The Y position where this paragraph starts.
    pub start_y: f64,
    /// The Y position where this paragraph ends.
    pub end_y: f64,
    /// The page number where this paragraph starts.
    pub start_page: usize,
    /// The page number where this paragraph ends.
    pub end_page: usize,
}

impl CachedParagraphCommands {
    /// Creates a new cached paragraph commands entry.
    pub fn new(
        content_hash: ContentHash,
        commands: Vec<DrawCommand>,
        start_y: f64,
        end_y: f64,
        start_page: usize,
        end_page: usize,
    ) -> Self {
        Self {
            content_hash,
            commands,
            start_y,
            end_y,
            start_page,
            end_page,
        }
    }

    /// Returns the height of this paragraph.
    pub fn height(&self) -> f64 {
        self.end_y - self.start_y
    }
}

/// Cache for paragraph draw commands.
///
/// This cache stores draw commands indexed by paragraph index. When a paragraph
/// is re-rendered with the same content hash and position, the cached commands
/// can be replayed instead of regenerating them.
#[derive(Debug, Default)]
pub struct ParagraphCommandCache {
    /// Cached commands indexed by paragraph index.
    entries: Vec<Option<CachedParagraphCommands>>,
    /// Statistics
    hits: u64,
    misses: u64,
}

impl ParagraphCommandCache {
    /// Creates a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a cache with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            hits: 0,
            misses: 0,
        }
    }

    /// Attempts to get cached commands for a paragraph.
    ///
    /// Returns `Some` if:
    /// 1. An entry exists for the paragraph index
    /// 2. The content hash matches
    /// 3. The start Y position matches (no upstream changes)
    pub fn get(
        &mut self,
        paragraph_index: usize,
        content_hash: ContentHash,
        expected_start_y: f64,
    ) -> Option<&CachedParagraphCommands> {
        if paragraph_index >= self.entries.len() {
            self.misses += 1;
            return None;
        }

        if let Some(ref entry) = self.entries[paragraph_index] {
            const EPSILON: f64 = 0.001;
            if entry.content_hash == content_hash
                && (entry.start_y - expected_start_y).abs() < EPSILON
            {
                self.hits += 1;
                return Some(entry);
            }
        }

        self.misses += 1;
        None
    }

    /// Stores cached commands for a paragraph.
    pub fn insert(&mut self, paragraph_index: usize, entry: CachedParagraphCommands) {
        // Ensure vector is large enough
        if paragraph_index >= self.entries.len() {
            self.entries.resize(paragraph_index + 1, None);
        }
        self.entries[paragraph_index] = Some(entry);
    }

    /// Stores cached commands for a paragraph with individual arguments.
    pub fn insert_with_args(
        &mut self,
        paragraph_index: usize,
        content_hash: ContentHash,
        start_y: f64,
        commands: Vec<DrawCommand>,
    ) {
        let end_y = start_y; // Will be updated by caller if needed
        let entry = CachedParagraphCommands::new(content_hash, commands, start_y, end_y, 0, 0);
        self.insert(paragraph_index, entry);
    }

    /// Invalidates all entries from the given paragraph index onwards.
    pub fn invalidate_from(&mut self, paragraph_index: usize) {
        if paragraph_index < self.entries.len() {
            for entry in &mut self.entries[paragraph_index..] {
                *entry = None;
            }
        }
    }

    /// Clears all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns the number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }

    /// Returns true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(|e| e.is_none())
    }

    /// Returns cache statistics.
    pub fn stats(&self) -> ParagraphCommandCacheStats {
        ParagraphCommandCacheStats {
            hits: self.hits,
            misses: self.misses,
            cached_entries: self.len(),
            total_capacity: self.entries.len(),
        }
    }

    /// Resets statistics.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    /// Attempts to get cached commands for a paragraph with y offset adjustment.
    ///
    /// Returns `Some((commands, y_offset))` if:
    /// 1. An entry exists for the paragraph index
    /// 2. The content hash matches
    ///
    /// The y_offset is the difference between current_y and the cached start_y,
    /// which should be applied to all commands before execution.
    pub fn get_with_offset(
        &mut self,
        paragraph_index: usize,
        content_hash: ContentHash,
        current_y: f64,
    ) -> Option<(Vec<DrawCommand>, f64)> {
        if paragraph_index >= self.entries.len() {
            self.misses += 1;
            return None;
        }

        if let Some(ref entry) = self.entries[paragraph_index] {
            if entry.content_hash == content_hash {
                self.hits += 1;
                let y_offset = current_y - entry.start_y;
                let adjusted_commands: Vec<DrawCommand> = entry
                    .commands
                    .iter()
                    .map(|cmd| cmd.with_y_offset(y_offset))
                    .collect();
                return Some((adjusted_commands, entry.height()));
            }
        }

        self.misses += 1;
        None
    }

    /// Returns cached draw commands as a borrowed slice plus y_offset and height,
    /// avoiding the clone+collect overhead of `get_with_offset`.
    /// Callers should apply `y_offset` at draw time (e.g. via `context.translate`).
    pub fn get_ref_with_offset(
        &mut self,
        paragraph_index: usize,
        content_hash: ContentHash,
        current_y: f64,
    ) -> Option<(&[DrawCommand], f64, f64)> {
        if paragraph_index >= self.entries.len() {
            self.misses += 1;
            return None;
        }

        if let Some(ref entry) = self.entries[paragraph_index] {
            if entry.content_hash == content_hash {
                self.hits += 1;
                let y_offset = current_y - entry.start_y;
                return Some((&entry.commands, y_offset, entry.height()));
            }
        }

        self.misses += 1;
        None
    }
}

/// Statistics for paragraph command cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParagraphCommandCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub cached_entries: usize,
    pub total_capacity: usize,
}

impl ParagraphCommandCacheStats {
    /// Returns the cache hit rate as a value between 0.0 and 1.0.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_style_default() {
        let style = TextStyle::default();
        assert_eq!(style.font_size_pt, 12.0);
        assert_eq!(style.letter_spacing_px, 0.0);
        assert!(!style.strike);
        assert!(!style.underline);
    }

    #[test]
    fn test_tab_leader_character() {
        assert_eq!(TabLeaderStyle::Dot.character(), ".");
        assert_eq!(TabLeaderStyle::Hyphen.character(), "-");
        assert_eq!(TabLeaderStyle::Underscore.character(), "_");
        assert_eq!(TabLeaderStyle::None.character(), "");
    }

    #[test]
    fn test_command_buffer_basic() {
        let mut buffer = DrawCommandBuffer::new();
        assert!(buffer.is_empty());

        buffer.draw_text("Hello", 10.0, 20.0, TextStyle::default());
        assert_eq!(buffer.len(), 1);

        buffer.fill_rect(0.0, 0.0, 100.0, 50.0, "#ff0000");
        assert_eq!(buffer.len(), 2);

        let commands = buffer.into_paragraph_commands(30.0, 50.0, 1);
        assert_eq!(commands.len(), 2);
        assert_eq!(commands.height, 30.0);
        assert_eq!(commands.end_y, 50.0);
    }

    #[test]
    fn test_command_buffer_text_with_spacing() {
        let mut buffer = DrawCommandBuffer::new();
        buffer.draw_text_with_spacing("Test", 0.0, 10.0, 2.0, TextStyle::default());

        let cmd = &buffer.commands()[0];
        match cmd {
            DrawCommand::TextWithSpacing {
                text,
                letter_spacing_px,
                ..
            } => {
                assert_eq!(&**text, "Test");
                assert_eq!(*letter_spacing_px, 2.0);
            }
            _ => panic!("Expected TextWithSpacing command"),
        }
    }

    #[test]
    fn test_command_buffer_decorations() {
        let mut buffer = DrawCommandBuffer::new();
        buffer.draw_strike_through(10.0, 20.0, 50.0, 12.0, "#000000", false);
        buffer.draw_underline(10.0, 20.0, 50.0, 12.0, "#0000ff");

        assert_eq!(buffer.len(), 2);
        assert!(matches!(
            buffer.commands()[0],
            DrawCommand::StrikeThrough { .. }
        ));
        assert!(matches!(
            buffer.commands()[1],
            DrawCommand::Underline { .. }
        ));
    }

    #[test]
    fn test_command_buffer_highlights() {
        let mut buffer = DrawCommandBuffer::new();
        buffer.draw_selection_highlight(0.0, 0.0, 100.0, 20.0);
        buffer.draw_comment_highlight(0.0, 20.0, 100.0, 20.0);
        buffer.draw_revision_highlight(0.0, 40.0, 100.0, 20.0, true);

        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn test_command_buffer_save_restore() {
        let mut buffer = DrawCommandBuffer::new();
        buffer.save();
        buffer.set_fill_style("#ff0000");
        buffer.fill_rect(0.0, 0.0, 10.0, 10.0, "#ff0000");
        buffer.restore();

        assert_eq!(buffer.len(), 4);
        assert!(matches!(buffer.commands()[0], DrawCommand::Save));
        assert!(matches!(buffer.commands()[3], DrawCommand::Restore));
    }

    #[test]
    fn test_draw_command_with_y_offset_clip_rect() {
        let cmd = DrawCommand::ClipRect {
            x: 10.0,
            y: 100.0,
            width: 50.0,
            height: 20.0,
        };

        let offset_cmd = cmd.with_y_offset(-30.0);
        match offset_cmd {
            DrawCommand::ClipRect { y, .. } => assert_eq!(y, 70.0),
            _ => panic!("Expected ClipRect command"),
        }
    }

    #[test]
    fn test_paragraph_draw_commands_extend() {
        let mut commands1 = ParagraphDrawCommands::new();
        commands1.push(DrawCommand::FillRect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            color: Rc::from("#000000"),
        });

        let mut commands2 = ParagraphDrawCommands::new();
        commands2.push(DrawCommand::FillRect {
            x: 10.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            color: Rc::from("#ff0000"),
        });

        commands1.extend(commands2);
        assert_eq!(commands1.len(), 2);
    }

    #[test]
    fn test_paragraph_command_cache_basic() {
        let mut cache = ParagraphCommandCache::new();
        assert!(cache.is_empty());

        let hash = ContentHash::new(12345);
        let commands = vec![DrawCommand::FillRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 20.0,
            color: Rc::from("#000000"),
        }];

        let entry = CachedParagraphCommands::new(hash, commands, 100.0, 120.0, 1, 1);
        cache.insert(0, entry);

        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());

        // Cache hit with matching hash and position
        let result = cache.get(0, hash, 100.0);
        assert!(result.is_some());
        assert_eq!(result.unwrap().commands.len(), 1);

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 0);
    }

    #[test]
    fn test_paragraph_command_cache_miss_wrong_hash() {
        let mut cache = ParagraphCommandCache::new();

        let hash1 = ContentHash::new(12345);
        let hash2 = ContentHash::new(67890);
        let commands = vec![];

        let entry = CachedParagraphCommands::new(hash1, commands, 100.0, 120.0, 1, 1);
        cache.insert(0, entry);

        // Cache miss with different hash
        let result = cache.get(0, hash2, 100.0);
        assert!(result.is_none());

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_paragraph_command_cache_miss_wrong_position() {
        let mut cache = ParagraphCommandCache::new();

        let hash = ContentHash::new(12345);
        let commands = vec![];

        let entry = CachedParagraphCommands::new(hash, commands, 100.0, 120.0, 1, 1);
        cache.insert(0, entry);

        // Cache miss with different Y position (upstream content changed)
        let result = cache.get(0, hash, 150.0);
        assert!(result.is_none());

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_paragraph_command_cache_invalidate() {
        let mut cache = ParagraphCommandCache::new();

        let hash = ContentHash::new(12345);

        // Insert entries for paragraphs 0, 1, 2
        for i in 0..3 {
            let entry = CachedParagraphCommands::new(
                hash,
                vec![],
                (i * 20) as f64,
                ((i + 1) * 20) as f64,
                1,
                1,
            );
            cache.insert(i, entry);
        }

        assert_eq!(cache.len(), 3);

        // Invalidate from paragraph 1 onwards
        cache.invalidate_from(1);

        assert_eq!(cache.len(), 1);
        assert!(cache.get(0, hash, 0.0).is_some());
        assert!(cache.get(1, hash, 20.0).is_none());
        assert!(cache.get(2, hash, 40.0).is_none());
    }

    #[test]
    fn test_cached_paragraph_commands_height() {
        let entry = CachedParagraphCommands::new(ContentHash::new(0), vec![], 100.0, 150.0, 1, 1);
        assert_eq!(entry.height(), 50.0);
    }

    #[test]
    fn test_paragraph_command_cache_stats_hit_rate() {
        let mut cache = ParagraphCommandCache::new();
        let hash = ContentHash::new(12345);

        let entry = CachedParagraphCommands::new(hash, vec![], 0.0, 20.0, 1, 1);
        cache.insert(0, entry);

        // 2 hits
        cache.get(0, hash, 0.0);
        cache.get(0, hash, 0.0);

        // 1 miss (wrong index)
        cache.get(1, hash, 20.0);

        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert!((stats.hit_rate() - 0.666666).abs() < 0.001);
    }

    #[test]
    fn test_draw_command_with_y_offset() {
        let cmd = DrawCommand::Text {
            text: Rc::from("Hello"),
            x: 10.0,
            y: 100.0,
            style: TextStyle::default(),
        };

        let offset_cmd = cmd.with_y_offset(50.0);
        match offset_cmd {
            DrawCommand::Text { y, .. } => assert_eq!(y, 150.0),
            _ => panic!("Expected Text command"),
        }
    }

    #[test]
    fn test_draw_command_text_run_with_y_offset() {
        let cmd = DrawCommand::TextRun {
            text: Rc::from("A"),
            x: 10.0,
            y: 100.0,
            style: TextStyle::default(),
            fonts: vec![Rc::from("12pt Latin")],
            glyphs: vec![GlyphRunStep {
                text_start: 0,
                text_end: 1,
                font_index: 0,
                x_offset: 5.0,
            }],
        };

        let offset_cmd = cmd.with_y_offset(25.0);
        match offset_cmd {
            DrawCommand::TextRun {
                text, y, glyphs, ..
            } => {
                assert_eq!(y, 125.0);
                assert_eq!(glyphs.len(), 1);
                assert_eq!(glyphs[0].text(&text), Some("A"));
            }
            _ => panic!("Expected TextRun command"),
        }
    }

    #[test]
    fn test_draw_command_with_y_offset_zero() {
        let cmd = DrawCommand::Text {
            text: Rc::from("Hello"),
            x: 10.0,
            y: 100.0,
            style: TextStyle::default(),
        };

        // Zero offset should return clone
        let offset_cmd = cmd.with_y_offset(0.0);
        assert_eq!(cmd, offset_cmd);
    }

    #[test]
    fn test_draw_command_with_y_offset_revision() {
        let cmd = DrawCommand::RevisionHighlight {
            x: 10.0,
            y: 100.0,
            width: 50.0,
            height: 20.0,
            is_inserted: true,
        };

        let offset_cmd = cmd.with_y_offset(-30.0);
        match offset_cmd {
            DrawCommand::RevisionHighlight { y, .. } => assert_eq!(y, 70.0),
            _ => panic!("Expected RevisionHighlight command"),
        }
    }

    #[test]
    fn test_paragraph_command_cache_get_with_offset() {
        let mut cache = ParagraphCommandCache::new();
        let hash = ContentHash::new(12345);

        let commands = vec![
            DrawCommand::Text {
                text: Rc::from("Test"),
                x: 10.0,
                y: 100.0,
                style: TextStyle::default(),
            },
            DrawCommand::RevisionHighlight {
                x: 10.0,
                y: 100.0,
                width: 50.0,
                height: 20.0,
                is_inserted: true,
            },
        ];
        let entry = CachedParagraphCommands::new(hash, commands, 90.0, 120.0, 1, 1);
        cache.insert(0, entry);

        // Get with different y position (offset of 10)
        let result = cache.get_with_offset(0, hash, 100.0);
        assert!(result.is_some());

        let (adjusted_commands, height) = result.unwrap();
        assert_eq!(height, 30.0); // 120.0 - 90.0
        assert_eq!(adjusted_commands.len(), 2);

        // Check that y coordinates are offset by 10
        match adjusted_commands[0].clone() {
            DrawCommand::Text { y, .. } => assert_eq!(y, 110.0), // 100 + 10
            _ => panic!("Expected Text command"),
        }
        match adjusted_commands[1].clone() {
            DrawCommand::RevisionHighlight { y, .. } => assert_eq!(y, 110.0), // 100 + 10
            _ => panic!("Expected RevisionHighlight command"),
        }
    }
}
