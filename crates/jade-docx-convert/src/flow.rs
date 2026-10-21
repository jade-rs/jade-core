//! The flow model every converter reads into and writes out of: what a
//! document is when the format it came in no longer matters — sections of
//! paragraphs and tables, runs of formatted text, pictures with their bytes.
//!
//! Lengths are in twentieths of a point (twips), as Word's own settings and
//! RTF count them; picture sizes are in EMU (914 400 to the inch), as
//! DrawingML counts them.

/// A document: its sections, in order, and what describes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlowDocument {
    pub title: Option<String>,
    pub author: Option<String>,
    /// The face body text is set in, where the source names one.
    pub default_font: Option<String>,
    /// The body text size, in half-points.
    pub default_size: Option<u32>,
    /// The language body text is in, as a BCP 47 tag (`ja-JP`).
    pub language: Option<String>,
    pub sections: Vec<FlowSection>,
}

impl FlowDocument {
    /// A document of one section on the default page.
    pub fn single(blocks: Vec<Block>) -> Self {
        Self {
            sections: vec![FlowSection {
                blocks,
                ..FlowSection::default()
            }],
            ..Self::default()
        }
    }

    /// Every paragraph's text, a line each, tables cell by cell: what a
    /// test looks for.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            for block in &section.blocks {
                block.push_plain_text(&mut out);
            }
        }
        out
    }

    /// Every block of every section, in order.
    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.sections
            .iter()
            .flat_map(|section| section.blocks.iter())
    }

    /// Every paragraph, tables and all, in reading order.
    pub fn paragraphs(&self) -> Vec<&Paragraph> {
        let mut out = Vec::new();
        for block in self.blocks() {
            block.collect_paragraphs(&mut out);
        }
        out
    }
}

/// A run of pages with one page setup, and its header and footer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlowSection {
    pub page: PageSetup,
    pub header: Vec<Block>,
    pub footer: Vec<Block>,
    pub blocks: Vec<Block>,
}

/// The paper and margins, in twips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSetup {
    pub width: u32,
    pub height: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
    pub header: u32,
    pub footer: u32,
}

impl Default for PageSetup {
    /// A4 with Word's 25.4 mm margins.
    fn default() -> Self {
        Self {
            width: 11906,
            height: 16838,
            top: 1440,
            right: 1440,
            bottom: 1440,
            left: 1440,
            header: 720,
            footer: 720,
        }
    }
}

impl PageSetup {
    pub fn landscape(&self) -> bool {
        self.width > self.height
    }

    /// The width text runs across, between the margins.
    pub fn text_width(&self) -> u32 {
        self.width.saturating_sub(self.left + self.right).max(720)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

impl Block {
    fn push_plain_text(&self, out: &mut String) {
        match self {
            Block::Paragraph(paragraph) => {
                out.push_str(&paragraph.text());
                out.push('\n');
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        for block in &cell.blocks {
                            block.push_plain_text(out);
                        }
                    }
                }
            }
        }
    }

    fn collect_paragraphs<'a>(&'a self, out: &mut Vec<&'a Paragraph>) {
        match self {
            Block::Paragraph(paragraph) => out.push(paragraph),
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        for block in &cell.blocks {
                            block.collect_paragraphs(out);
                        }
                    }
                }
            }
        }
    }
}

/// What kind of paragraph it is, which the formats name differently: a
/// heading is `h2` in HTML, `text:h` in ODT, `heading 2` in Word.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ParagraphKind {
    #[default]
    Normal,
    /// Heading 1–9.
    Heading(u8),
    Title,
    /// Preformatted text: a fixed-pitch face, spaces kept.
    Preformatted,
    Quote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Justify,
}

/// Line spacing: a multiple of single (240 = single), or an exact or
/// minimum height in twips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineSpacing {
    Multiple(u32),
    Exact(u32),
    AtLeast(u32),
}

/// A paragraph in a list: bulleted or numbered, and how deep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub ordered: bool,
    /// 0 for the outermost level.
    pub level: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paragraph {
    pub kind: ParagraphKind,
    pub alignment: Option<Alignment>,
    /// Twips; the first line's is relative to the left indent, negative for
    /// a hanging indent.
    pub indent_left: i32,
    pub indent_right: i32,
    pub indent_first_line: i32,
    pub space_before: Option<u32>,
    pub space_after: Option<u32>,
    pub line_spacing: Option<LineSpacing>,
    pub list: Option<ListItem>,
    pub page_break_before: bool,
    pub keep_with_next: bool,
    /// The paragraph's background, `RRGGBB`.
    pub shading: Option<String>,
    pub inlines: Vec<Inline>,
}

impl Paragraph {
    pub fn with_text(text: &str) -> Self {
        Self {
            inlines: vec![Inline::Text(text.to_string(), CharFormat::default())],
            ..Self::default()
        }
    }

    /// The paragraph's text: tabs as tabs, breaks as newlines, pictures as
    /// nothing.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for inline in &self.inlines {
            match inline {
                Inline::Text(text, _) => out.push_str(text),
                Inline::Tab => out.push('\t'),
                Inline::LineBreak => out.push('\n'),
                Inline::PageBreak => out.push('\u{c}'),
                Inline::Image(_) => {}
            }
        }
        out
    }

    /// Adds text in `format`, joined to the run before when that run is
    /// formatted the same.
    pub fn push_text(&mut self, text: &str, format: &CharFormat) {
        if text.is_empty() {
            return;
        }
        if let Some(Inline::Text(last, last_format)) = self.inlines.last_mut() {
            if last_format == format {
                last.push_str(text);
                return;
            }
        }
        self.inlines
            .push(Inline::Text(text.to_string(), format.clone()));
    }

    pub fn is_empty(&self) -> bool {
        self.inlines.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String, CharFormat),
    Tab,
    LineBreak,
    PageBreak,
    Image(Image),
}

/// A run's character formatting. `None` and `false` are what the paragraph
/// gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharFormat {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub small_caps: bool,
    pub all_caps: bool,
    /// Hidden text (`w:vanish`).
    pub hidden: bool,
    /// Half-points.
    pub size: Option<u32>,
    /// The Latin face.
    pub font: Option<String>,
    /// The East Asian face, where it differs.
    pub east_asian_font: Option<String>,
    /// `RRGGBB`.
    pub color: Option<String>,
    /// One of Word's highlight names (`yellow`, `green`, …).
    pub highlight: Option<String>,
    /// A background colour that is not a highlight, `RRGGBB`.
    pub shading: Option<String>,
    /// The hyperlink the run belongs to: a URL, or `#bookmark`.
    pub link: Option<String>,
}

impl CharFormat {
    pub fn is_plain(&self) -> bool {
        *self == CharFormat::default()
    }
}

/// A picture, its bytes as they were stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub data: Vec<u8>,
    pub format: ImageFormat,
    /// The displayed size, EMU.
    pub width: u64,
    pub height: u64,
    /// Alternative text.
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Tiff,
    Emf,
    Wmf,
}

impl ImageFormat {
    /// What the bytes are, read from their signature.
    pub fn sniff(data: &[u8]) -> Option<Self> {
        if data.starts_with(b"\x89PNG") {
            Some(Self::Png)
        } else if data.starts_with(&[0xff, 0xd8]) {
            Some(Self::Jpeg)
        } else if data.starts_with(b"GIF8") {
            Some(Self::Gif)
        } else if data.starts_with(b"BM") {
            Some(Self::Bmp)
        } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
            Some(Self::Tiff)
        } else if data.len() > 44 && data[0..4] == [1, 0, 0, 0] && &data[40..44] == b" EMF" {
            Some(Self::Emf)
        } else if data.starts_with(&[0xd7, 0xcd, 0xc6, 0x9a]) || data.starts_with(&[1, 0, 9, 0]) {
            Some(Self::Wmf)
        } else {
            None
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Emf => "emf",
            Self::Wmf => "wmf",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
            Self::Tiff => "image/tiff",
            Self::Emf => "image/x-emf",
            Self::Wmf => "image/x-wmf",
        }
    }

    /// The size a picture's own header gives, in pixels, where it is one
    /// the header can be read from cheaply.
    pub fn pixel_size(data: &[u8]) -> Option<(u32, u32)> {
        match Self::sniff(data)? {
            Self::Png if data.len() >= 24 => Some((
                u32::from_be_bytes(data[16..20].try_into().ok()?),
                u32::from_be_bytes(data[20..24].try_into().ok()?),
            )),
            Self::Gif if data.len() >= 10 => Some((
                u16::from_le_bytes([data[6], data[7]]) as u32,
                u16::from_le_bytes([data[8], data[9]]) as u32,
            )),
            Self::Bmp if data.len() >= 26 => Some((
                i32::from_le_bytes(data[18..22].try_into().ok()?).unsigned_abs(),
                i32::from_le_bytes(data[22..26].try_into().ok()?).unsigned_abs(),
            )),
            Self::Jpeg => jpeg_size(data),
            _ => None,
        }
    }
}

fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 9 < data.len() {
        if data[at] != 0xff {
            at += 1;
            continue;
        }
        let marker = data[at + 1];
        if marker == 0xd8 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            at += 2;
            continue;
        }
        let length = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let height = u16::from_be_bytes([data[at + 5], data[at + 6]]) as u32;
            let width = u16::from_be_bytes([data[at + 7], data[at + 8]]) as u32;
            return Some((width, height));
        }
        at += 2 + length;
    }
    None
}

/// EMU per pixel at 96 to the inch.
pub const EMU_PER_PIXEL: u64 = 9525;
/// EMU per twip.
pub const EMU_PER_TWIP: u64 = 635;

impl Image {
    /// A picture at its own pixel size (96 to the inch), or an inch square
    /// when the header does not say.
    pub fn new(data: Vec<u8>, description: Option<String>) -> Option<Self> {
        let format = ImageFormat::sniff(&data)?;
        let (width, height) = ImageFormat::pixel_size(&data).unwrap_or((96, 96));
        Some(Self {
            data,
            format,
            width: width.max(1) as u64 * EMU_PER_PIXEL,
            height: height.max(1) as u64 * EMU_PER_PIXEL,
            description,
        })
    }

    /// Shrinks the picture, keeping its proportions, to no wider than
    /// `width` EMU.
    pub fn fit_width(&mut self, width: u64) {
        if self.width > width && self.width > 0 {
            self.height = self.height * width / self.width;
            self.width = width;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// Each grid column's width, twips.
    pub columns: Vec<u32>,
    pub rows: Vec<Row>,
    /// Whether the cells are ruled, as a grid.
    pub borders: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// Repeats at the top of each page.
    pub header: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub blocks: Vec<Block>,
    /// How many grid columns the cell spans.
    pub span: u32,
    pub vertical_merge: VerticalMerge,
    /// `RRGGBB`.
    pub shading: Option<String>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            span: 1,
            vertical_merge: VerticalMerge::None,
            shading: None,
        }
    }
}

impl Cell {
    pub fn with_text(text: &str) -> Self {
        Self {
            blocks: vec![Block::Paragraph(Paragraph::with_text(text))],
            ..Self::default()
        }
    }
}

/// A cell merged with the ones below it: the first starts the merge, the
/// rest continue it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerticalMerge {
    #[default]
    None,
    Restart,
    Continue,
}

impl Table {
    /// Columns of equal width across `width` twips, for a source that does
    /// not say how wide they are.
    pub fn even_columns(count: usize, width: u32) -> Vec<u32> {
        let count = count.max(1) as u32;
        vec![width / count; count as usize]
    }

    /// How many grid columns the widest row spans.
    pub fn grid_width(&self) -> usize {
        self.rows
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.span.max(1) as usize).sum())
            .max()
            .unwrap_or(0)
    }
}

/// The highlight colours Word knows, by name and colour.
pub const HIGHLIGHTS: &[(&str, &str)] = &[
    ("yellow", "FFFF00"),
    ("green", "00FF00"),
    ("cyan", "00FFFF"),
    ("magenta", "FF00FF"),
    ("blue", "0000FF"),
    ("red", "FF0000"),
    ("darkBlue", "000080"),
    ("darkCyan", "008080"),
    ("darkGreen", "008000"),
    ("darkMagenta", "800080"),
    ("darkRed", "800000"),
    ("darkYellow", "808000"),
    ("darkGray", "808080"),
    ("lightGray", "C0C0C0"),
    ("black", "000000"),
    ("white", "FFFFFF"),
];

/// The colour of a highlight name.
pub fn highlight_color(name: &str) -> Option<&'static str> {
    HIGHLIGHTS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, color)| *color)
}

/// The highlight whose colour is `color`, if there is one.
pub fn highlight_named(color: &str) -> Option<&'static str> {
    let color = color.trim_start_matches('#');
    HIGHLIGHTS
        .iter()
        .find(|(_, known)| known.eq_ignore_ascii_case(color))
        .map(|(name, _)| *name)
}
