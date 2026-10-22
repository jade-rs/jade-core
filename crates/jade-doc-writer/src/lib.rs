//! Word 97–2003 documents (`.doc`), written as [MS-DOC] lays them out.
//!
//! The document's text goes into the `WordDocument` stream as one UTF-16
//! piece after the File Information Block; its runs' character properties
//! and its paragraphs' properties go into formatted disk pages (CHPX and
//! PAPX FKPs) after it, and each section's properties (SEPX) after those.
//! The `1Table` stream holds what ties them together: the style sheet
//! (Normal and Heading 1–9, by their built-in identifiers), the font table,
//! the section table, the bin tables of the FKPs, the header stories'
//! table, the fields' table, the document properties and the piece table.
//! Pictures go into the `Data` stream as Word 2000 writes them: a PICF and
//! an OfficeArt shape with its blip.
//!
//! What the format has a place for is written — paragraphs with their
//! alignment, indents and spacing, headings, runs with their formatting,
//! tables with their spans and vertical merges, sections with their page,
//! headers and footers, hyperlinks as HYPERLINK fields, and PNG and JPEG
//! pictures; lists are written as their markers and a hanging indent.

use jade_docx_convert::flow::*;

use jade_opc::compound_file;

const FIB_SIZE: usize = 900;
/// Where the text starts in the WordDocument stream.
const TEXT_START: usize = 0x400;
const PAGE: usize = 512;

/// Writes `document` as a `.doc`.
pub fn write_doc(document: &FlowDocument) -> Result<Vec<u8>, String> {
    write(document, false)
}

/// Writes `document` as a Word 97–2003 template (`.dot`): a `.doc` whose
/// FIB says it is a template.
pub fn write_dot(document: &FlowDocument) -> Result<Vec<u8>, String> {
    write(document, true)
}

fn write(document: &FlowDocument, template: bool) -> Result<Vec<u8>, String> {
    let mut builder = Builder::new(document);
    builder.template = template;
    let empty = FlowSection::default();
    let sections: Vec<&FlowSection> = if document.sections.is_empty() {
        vec![&empty]
    } else {
        document.sections.iter().collect()
    };
    // The main text, section by section; each section but the last ends
    // with a section mark.
    let mut section_ends = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        let last = index + 1 == sections.len();
        builder.blocks(&section.blocks, section.page.text_width(), false);
        // A section ends in a paragraph: a table at its end gets one after.
        if section.blocks.is_empty() || matches!(section.blocks.last(), Some(Block::Table(_))) {
            builder.paragraph(&Paragraph::default(), section.page.text_width(), false);
        }
        if !last {
            builder.end_section();
        }
        section_ends.push(builder.text.len() as u32);
    }
    let ccp_text = builder.text.len() as u32;
    builder.in_main = false;
    // The header stories: six for the footnote separators (none here), then
    // six for each section, of which the odd-page header and footer are
    // written.
    let any_story = sections
        .iter()
        .any(|section| !section.header.is_empty() || !section.footer.is_empty());
    let mut stories: Vec<u32> = Vec::new();
    if any_story {
        for _ in 0..6 {
            stories.push(builder.text.len() as u32 - ccp_text);
        }
        for section in &sections {
            for slot in 0..6 {
                stories.push(builder.text.len() as u32 - ccp_text);
                let blocks = match slot {
                    1 => &section.header,
                    3 => &section.footer,
                    _ => continue,
                };
                if !blocks.is_empty() {
                    builder.blocks(blocks, section.page.text_width(), false);
                    if matches!(blocks.last(), Some(Block::Table(_))) {
                        builder.paragraph(&Paragraph::default(), section.page.text_width(), false);
                    }
                }
            }
        }
        stories.push(builder.text.len() as u32 - ccp_text);
        // The guard mark ending the header document.
        builder.mark(0x0D, 0, Vec::new());
    }
    let ccp_hdd = builder.text.len() as u32 - ccp_text;
    if ccp_hdd > 0 {
        // A document with subdocuments ends with one more paragraph mark.
        builder.mark(0x0D, 0, Vec::new());
    }
    builder.finish(
        document,
        &sections,
        &section_ends,
        ccp_text,
        ccp_hdd,
        &stories,
    )
}

struct Builder {
    text: Vec<u16>,
    /// Each run of characters with the same character properties: its end
    /// (exclusive) in CPs and its grpprl.
    runs: Vec<(u32, Vec<u8>)>,
    /// Each paragraph: its end (after its mark) in CPs, style and grpprl.
    paragraphs: Vec<(u32, u16, Vec<u8>)>,
    fonts: Vec<(String, u8)>,
    /// The main document's field characters: CP and FLD.
    fields: Vec<(u32, [u8; 2])>,
    data: Vec<u8>,
    in_main: bool,
    has_pictures: bool,
    japanese: bool,
    shapes: u32,
    /// One list for each run of list paragraphs: whether each of its nine
    /// levels is numbered (`None` for a level no paragraph uses).
    lists: Vec<[Option<bool>; 9]>,
    /// The list the paragraphs being written continue.
    current_list: Option<usize>,
    /// fDot: the file is a template.
    template: bool,
}

fn push_sprm(grpprl: &mut Vec<u8>, sprm: u16, operand: &[u8]) {
    grpprl.extend_from_slice(&sprm.to_le_bytes());
    grpprl.extend_from_slice(operand);
}

const ICO_COLORS: [(u8, [u8; 3]); 16] = [
    (1, [0, 0, 0]),
    (2, [0, 0, 255]),
    (3, [0, 255, 255]),
    (4, [0, 255, 0]),
    (5, [255, 0, 255]),
    (6, [255, 0, 0]),
    (7, [255, 255, 0]),
    (8, [255, 255, 255]),
    (9, [0, 0, 128]),
    (10, [0, 128, 128]),
    (11, [0, 128, 0]),
    (12, [128, 0, 128]),
    (13, [128, 0, 0]),
    (14, [128, 128, 0]),
    (15, [128, 128, 128]),
    (16, [192, 192, 192]),
];

fn hex_rgb(color: &str) -> Option<[u8; 3]> {
    let color = color.trim_start_matches('#');
    if color.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&color[range], 16).ok();
    Some([channel(0..2)?, channel(2..4)?, channel(4..6)?])
}

/// The nearest of Word 97's sixteen colours.
fn ico(rgb: [u8; 3]) -> u8 {
    ICO_COLORS
        .iter()
        .min_by_key(|(_, color)| {
            color
                .iter()
                .zip(rgb.iter())
                .map(|(a, b)| (*a as i32 - *b as i32).pow(2))
                .sum::<i32>()
        })
        .map(|(index, _)| *index)
        .unwrap_or(1)
}

impl Builder {
    fn new(document: &FlowDocument) -> Self {
        let japanese = document.language.as_deref().is_some_and(|lang| lang.starts_with("ja"))
            || document.sections.iter().any(|section| {
                section.blocks.iter().any(|block| {
                    matches!(block, Block::Paragraph(p) if p.text().chars().any(|ch| matches!(ch as u32, 0x3040..=0x30ff | 0x4e00..=0x9fff)))
                })
            });
        let body = document
            .default_font
            .clone()
            .unwrap_or_else(|| "Times New Roman".to_string());
        let mut fonts = vec![(body.clone(), charset_of(&body))];
        let east_asian = if japanese {
            "ＭＳ 明朝"
        } else {
            "Times New Roman"
        };
        if !fonts.iter().any(|(name, _)| name == east_asian) {
            fonts.push((east_asian.to_string(), charset_of(east_asian)));
        }
        Self {
            text: Vec::new(),
            runs: Vec::new(),
            paragraphs: Vec::new(),
            fonts,
            fields: Vec::new(),
            data: Vec::new(),
            in_main: true,
            has_pictures: false,
            japanese,
            shapes: 0,
            lists: Vec::new(),
            current_list: None,
            template: false,
        }
    }

    fn font(&mut self, name: &str) -> u16 {
        if let Some(index) = self.fonts.iter().position(|(known, _)| known == name) {
            return index as u16;
        }
        self.fonts.push((name.to_string(), charset_of(name)));
        (self.fonts.len() - 1) as u16
    }

    /// Characters in one run of `grpprl`.
    fn chars(&mut self, units: &[u16], grpprl: Vec<u8>) {
        if units.is_empty() {
            return;
        }
        self.text.extend_from_slice(units);
        let end = self.text.len() as u32;
        match self.runs.last_mut() {
            Some((last_end, last)) if *last == grpprl && *last_end == end - units.len() as u32 => {
                *last_end = end;
            }
            _ => self.runs.push((end, grpprl)),
        }
    }

    /// A paragraph's mark — 0x0D, a cell's 0x07, or a section's 0x0C —
    /// ending the paragraph with style `istd` and `grpprl`.
    fn mark(&mut self, mark: u16, istd: u16, grpprl: Vec<u8>) {
        self.chars(&[mark], Vec::new());
        self.paragraphs.push((self.text.len() as u32, istd, grpprl));
    }

    /// The last paragraph's mark becomes a section mark.
    fn end_section(&mut self) {
        if let Some(last) = self.text.last_mut() {
            *last = 0x0C;
        }
    }

    fn char_grpprl(&mut self, format: &CharFormat) -> Vec<u8> {
        let mut grpprl = Vec::new();
        if format.bold {
            push_sprm(&mut grpprl, 0x0835, &[1]);
        }
        if format.italic {
            push_sprm(&mut grpprl, 0x0836, &[1]);
        }
        if format.strike {
            push_sprm(&mut grpprl, 0x0837, &[1]);
        }
        if format.small_caps {
            push_sprm(&mut grpprl, 0x083A, &[1]);
        }
        if format.all_caps {
            push_sprm(&mut grpprl, 0x083B, &[1]);
        }
        if format.hidden {
            push_sprm(&mut grpprl, 0x083C, &[1]);
        }
        if format.underline || format.link.is_some() {
            push_sprm(&mut grpprl, 0x2A3E, &[1]);
        }
        if format.superscript {
            push_sprm(&mut grpprl, 0x2A48, &[1]);
        } else if format.subscript {
            push_sprm(&mut grpprl, 0x2A48, &[2]);
        }
        if let Some(size) = format.size {
            push_sprm(
                &mut grpprl,
                0x4A43,
                &(size.clamp(2, 3276) as u16).to_le_bytes(),
            );
        }
        if let Some(font) = &format.font {
            let index = self.font(font);
            push_sprm(&mut grpprl, 0x4A4F, &index.to_le_bytes());
            push_sprm(&mut grpprl, 0x4A51, &index.to_le_bytes());
        }
        if let Some(font) = &format.east_asian_font {
            let index = self.font(font);
            push_sprm(&mut grpprl, 0x4A50, &index.to_le_bytes());
        }
        let color = format
            .color
            .as_deref()
            .and_then(hex_rgb)
            .or_else(|| format.link.as_ref().map(|_| [0x05, 0x63, 0xC1]));
        if let Some(rgb) = color {
            push_sprm(&mut grpprl, 0x2A42, &[ico(rgb)]);
            push_sprm(&mut grpprl, 0x6870, &[rgb[0], rgb[1], rgb[2], 0]);
        }
        if let Some(highlight) = format
            .highlight
            .as_deref()
            .and_then(highlight_color)
            .and_then(hex_rgb)
        {
            push_sprm(&mut grpprl, 0x2A0C, &[ico(highlight)]);
        }
        grpprl
    }

    /// The list format override (1-based) of a list paragraph, starting a
    /// list at the first of a run of them; other paragraphs end the run.
    fn list_format(&mut self, paragraph: &Paragraph) -> Option<u16> {
        let Some(item) = paragraph.list else {
            self.current_list = None;
            return None;
        };
        let index = match self.current_list {
            Some(index) => index,
            None => {
                self.lists.push([None; 9]);
                self.lists.len() - 1
            }
        };
        self.current_list = Some(index);
        let level = &mut self.lists[index][item.level.min(8) as usize];
        level.get_or_insert(item.ordered);
        Some(index as u16 + 1)
    }

    fn paragraph_grpprl(
        paragraph: &Paragraph,
        list: Option<u16>,
        in_table: bool,
    ) -> (u16, Vec<u8>) {
        let mut grpprl = Vec::new();
        let istd = match paragraph.kind {
            ParagraphKind::Heading(level) => level.clamp(1, 9) as u16,
            _ => 0,
        };
        if let Some(alignment) = paragraph.alignment {
            let value = match alignment {
                Alignment::Left => 0,
                Alignment::Center => 1,
                Alignment::Right => 2,
                Alignment::Justify => 3,
            };
            push_sprm(&mut grpprl, 0x2403, &[value]);
        }
        let (left, first) = match paragraph.list {
            Some(item) => (360 + 360 * item.level as i32, -360),
            None => (paragraph.indent_left, paragraph.indent_first_line),
        };
        let clamp = |value: i32| value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        if left != 0 {
            push_sprm(&mut grpprl, 0x840F, &clamp(left).to_le_bytes());
        }
        if paragraph.indent_right != 0 {
            push_sprm(
                &mut grpprl,
                0x840E,
                &clamp(paragraph.indent_right).to_le_bytes(),
            );
        }
        if first != 0 {
            push_sprm(&mut grpprl, 0x8411, &clamp(first).to_le_bytes());
        }
        if let Some(before) = paragraph.space_before {
            push_sprm(
                &mut grpprl,
                0xA413,
                &(before.min(31680) as u16).to_le_bytes(),
            );
        }
        if let Some(after) = paragraph.space_after {
            push_sprm(
                &mut grpprl,
                0xA414,
                &(after.min(31680) as u16).to_le_bytes(),
            );
        }
        if let Some(line) = paragraph.line_spacing {
            let (value, multiple) = match line {
                LineSpacing::Multiple(line) => (line.min(32767) as i16, 1i16),
                LineSpacing::Exact(line) => (-(line.min(32767) as i16), 0),
                LineSpacing::AtLeast(line) => (line.min(32767) as i16, 0),
            };
            let mut operand = value.to_le_bytes().to_vec();
            operand.extend_from_slice(&multiple.to_le_bytes());
            push_sprm(&mut grpprl, 0x6412, &operand);
        }
        if paragraph.keep_with_next {
            push_sprm(&mut grpprl, 0x2406, &[1]);
        }
        if paragraph.page_break_before {
            push_sprm(&mut grpprl, 0x2407, &[1]);
        }
        if let (Some(ilfo), Some(item)) = (list, paragraph.list) {
            // The level first, as Word writes them: a reader applying the
            // list as it meets ilfo takes the level it already has.
            push_sprm(&mut grpprl, 0x260A, &[item.level.min(8)]);
            push_sprm(&mut grpprl, 0x460B, &ilfo.to_le_bytes());
        }
        if in_table {
            push_sprm(&mut grpprl, 0x2416, &[1]);
        }
        (istd, grpprl)
    }

    fn blocks(&mut self, blocks: &[Block], width: u32, in_table: bool) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => self.paragraph(paragraph, width, in_table),
                Block::Table(table) => {
                    self.current_list = None;
                    if in_table {
                        // A table in a cell: its cells' text, paragraph by paragraph.
                        for row in &table.rows {
                            for cell in &row.cells {
                                self.blocks(&cell.blocks, width, true);
                            }
                        }
                    } else {
                        self.table(table, width);
                    }
                }
            }
        }
    }

    /// A paragraph's inlines and mark.
    fn paragraph(&mut self, paragraph: &Paragraph, width: u32, in_table: bool) {
        let list = self.list_format(paragraph);
        self.inlines(paragraph, width);
        let (istd, grpprl) = Self::paragraph_grpprl(paragraph, list, in_table);
        self.mark(0x0D, istd, grpprl);
    }

    fn inlines(&mut self, paragraph: &Paragraph, width: u32) {
        let mut open_link: Option<String> = None;
        for inline in &paragraph.inlines {
            let link = match inline {
                Inline::Text(_, format) => format.link.clone(),
                _ => None,
            };
            if link != open_link {
                if open_link.is_some() {
                    self.field_char(0x15, 0x80);
                }
                if let Some(target) = &link {
                    if self.in_main {
                        self.field_char(0x13, 88);
                        let instruction = match target.strip_prefix('#') {
                            Some(anchor) => format!(" HYPERLINK \\l \"{anchor}\" "),
                            None => format!(" HYPERLINK \"{target}\" "),
                        };
                        let units: Vec<u16> = instruction.encode_utf16().collect();
                        self.chars(&units, Vec::new());
                        self.field_char(0x14, 0);
                    }
                }
                open_link = link.filter(|_| self.in_main);
            }
            match inline {
                Inline::Text(text, format) => {
                    let grpprl = self.char_grpprl(format);
                    let units: Vec<u16> = text
                        .chars()
                        .map(|ch| match ch {
                            '\n' => '\u{b}',
                            '\r' => ' ',
                            other => other,
                        })
                        .collect::<String>()
                        .encode_utf16()
                        // Control characters but tab, line and page break stand for
                        // the text's structure in a .doc; they are left out.
                        .filter(|unit| !matches!(*unit, 0x01..=0x08 | 0x0D..=0x1F))
                        .collect();
                    self.chars(&units, grpprl);
                }
                Inline::Tab => self.chars(&[0x09], Vec::new()),
                Inline::LineBreak => self.chars(&[0x0B], Vec::new()),
                Inline::PageBreak => self.chars(&[0x0C], Vec::new()),
                Inline::Image(image) => self.picture(image, width),
            }
        }
        if open_link.is_some() {
            self.field_char(0x15, 0x80);
        }
    }

    fn field_char(&mut self, ch: u16, flags: u8) {
        let mut grpprl = Vec::new();
        push_sprm(&mut grpprl, 0x0855, &[1]);
        let cp = self.text.len() as u32;
        self.chars(&[ch], grpprl);
        self.fields.push((cp, [ch as u8, flags]));
    }

    fn picture(&mut self, image: &Image, width: u32) {
        let (kind, instance, bt) = match image.format {
            ImageFormat::Png => (0xF01Eu16, 0x6E0u16, 6u8),
            ImageFormat::Jpeg => (0xF01D, 0x46A, 5),
            // No blip of its own in this format: its description stands in.
            _ => {
                let text = image
                    .description
                    .clone()
                    .unwrap_or_else(|| "[picture]".to_string());
                let units: Vec<u16> = text.encode_utf16().collect();
                self.chars(&units, Vec::new());
                return;
            }
        };
        let mut image = image.clone();
        image.fit_width(width as u64 * EMU_PER_TWIP);
        let goal_width = (image.width / EMU_PER_TWIP).min(31680) as i16;
        let goal_height = (image.height / EMU_PER_TWIP).min(31680) as i16;
        let uid = uid_of(&image.data);
        let record = |out: &mut Vec<u8>, version: u16, instance: u16, kind: u16, body: &[u8]| {
            out.extend_from_slice(&(version | (instance << 4)).to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(body);
        };
        let mut blip_body = uid.to_vec();
        blip_body.push(0xFF);
        blip_body.extend_from_slice(&image.data);
        let mut blip = Vec::new();
        record(&mut blip, 0, instance, kind, &blip_body);
        let mut fbse_body = vec![bt, bt];
        fbse_body.extend_from_slice(&uid);
        fbse_body.extend_from_slice(&0x00FFu16.to_le_bytes());
        fbse_body.extend_from_slice(&(blip.len() as u32).to_le_bytes());
        fbse_body.extend_from_slice(&1u32.to_le_bytes());
        fbse_body.extend_from_slice(&0u32.to_le_bytes());
        fbse_body.extend_from_slice(&[0, 0, 0, 0]);
        fbse_body.extend_from_slice(&blip);
        let mut fbse = Vec::new();
        record(&mut fbse, 2, bt as u16, 0xF007, &fbse_body);
        self.shapes += 1;
        let mut fsp_body = (1024 + self.shapes).to_le_bytes().to_vec();
        fsp_body.extend_from_slice(&0x0000_0A00u32.to_le_bytes());
        let mut shape = Vec::new();
        record(&mut shape, 2, 75, 0xF00A, &fsp_body);
        let mut fopt_body = 0x4104u16.to_le_bytes().to_vec();
        fopt_body.extend_from_slice(&1u32.to_le_bytes());
        record(&mut shape, 3, 1, 0xF00B, &fopt_body);
        let mut container = Vec::new();
        record(&mut container, 0xF, 0, 0xF004, &shape);
        container.extend_from_slice(&fbse);
        let mut picf = Vec::with_capacity(68);
        picf.extend_from_slice(&((68 + container.len()) as u32).to_le_bytes());
        picf.extend_from_slice(&0x44u16.to_le_bytes());
        // mfpf: MM_SHAPE, then the extents and handle Word leaves zero.
        picf.extend_from_slice(&0x0064u16.to_le_bytes());
        picf.extend_from_slice(&[0; 6]);
        picf.extend_from_slice(&[0; 14]);
        picf.extend_from_slice(&goal_width.to_le_bytes());
        picf.extend_from_slice(&goal_height.to_le_bytes());
        picf.extend_from_slice(&1000u16.to_le_bytes());
        picf.extend_from_slice(&1000u16.to_le_bytes());
        picf.extend_from_slice(&[0; 8]);
        picf.extend_from_slice(&[0; 2]);
        picf.extend_from_slice(&[0; 16]);
        picf.extend_from_slice(&[0; 4]);
        picf.extend_from_slice(&0u16.to_le_bytes());
        debug_assert_eq!(picf.len(), 68);
        // A PICF starts at an even offset.
        if self.data.len() % 2 == 1 {
            self.data.push(0);
        }
        let location = self.data.len() as u32;
        self.data.extend_from_slice(&picf);
        self.data.extend_from_slice(&container);
        let mut grpprl = Vec::new();
        push_sprm(&mut grpprl, 0x0855, &[1]);
        push_sprm(&mut grpprl, 0x6A03, &location.to_le_bytes());
        self.chars(&[0x01], grpprl);
        self.has_pictures = true;
    }

    fn table(&mut self, table: &Table, width: u32) {
        let grid = table.grid_width().max(1);
        let columns = if table.columns.len() == grid {
            table.columns.clone()
        } else {
            Table::even_columns(grid, width)
        };
        for row in &table.rows {
            let mut boundaries = vec![0i16];
            let mut column = 0usize;
            let mut tcs = Vec::new();
            for cell in &row.cells {
                let span = cell.span.max(1) as usize;
                let cell_width: u32 = columns.iter().skip(column).take(span).sum();
                column += span;
                let right = (*boundaries.last().unwrap_or(&0) as i32 + cell_width as i32)
                    .min(i16::MAX as i32);
                boundaries.push(right as i16);
                // TC80: merge flags, width, four borders.
                let mut flags = 0u16;
                match cell.vertical_merge {
                    VerticalMerge::Restart => flags |= 0x0020 | 0x0040,
                    VerticalMerge::Continue => flags |= 0x0020,
                    VerticalMerge::None => {}
                }
                flags |= 3 << 9;
                let mut tc = flags.to_le_bytes().to_vec();
                tc.extend_from_slice(&(cell_width.min(65535) as u16).to_le_bytes());
                let border: [u8; 4] = if table.borders {
                    [4, 1, 1, 0]
                } else {
                    [0, 0, 0, 0]
                };
                for _ in 0..4 {
                    tc.extend_from_slice(&border);
                }
                tcs.push(tc);
                // The cell's paragraphs; its last ends with the cell mark.
                let blocks: &[Block] = &cell.blocks;
                let (body, last) = match blocks.split_last() {
                    Some((Block::Paragraph(last), body)) => (body, Some(last.clone())),
                    _ => (blocks, None),
                };
                self.blocks(body, cell_width, true);
                let last = last.unwrap_or_default();
                let list = self.list_format(&last);
                if cell.vertical_merge != VerticalMerge::Continue {
                    self.inlines(&last, cell_width);
                }
                let (istd, grpprl) = Self::paragraph_grpprl(&last, list, true);
                self.mark(0x07, istd, grpprl);
            }
            // The row's end: a mark of its own carrying the row's layout.
            let mut grpprl = Vec::new();
            push_sprm(&mut grpprl, 0x2416, &[1]);
            push_sprm(&mut grpprl, 0x2417, &[1]);
            if row.header {
                push_sprm(&mut grpprl, 0x3404, &[1]);
            }
            let mut definition = vec![row.cells.len().min(63) as u8];
            for boundary in &boundaries {
                definition.extend_from_slice(&boundary.to_le_bytes());
            }
            for tc in tcs.iter().take(63) {
                definition.extend_from_slice(tc);
            }
            grpprl.extend_from_slice(&0xD608u16.to_le_bytes());
            grpprl.extend_from_slice(&((definition.len() + 1) as u16).to_le_bytes());
            grpprl.extend_from_slice(&definition);
            self.mark(0x07, 0, grpprl);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        mut self,
        document: &FlowDocument,
        sections: &[&FlowSection],
        section_ends: &[u32],
        ccp_text: u32,
        ccp_hdd: u32,
        stories: &[u32],
    ) -> Result<Vec<u8>, String> {
        let total = self.text.len() as u32;
        // The WordDocument stream: the FIB, the text, the FKPs, the SEPXs.
        let mut word = vec![0u8; TEXT_START];
        for unit in &self.text {
            word.extend_from_slice(&unit.to_le_bytes());
        }
        let fc_of = |cp: u32| TEXT_START as u32 + cp * 2;
        pad_to_page(&mut word);
        // Character runs, every character covered.
        let mut runs: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        let mut start = 0;
        for (end, grpprl) in std::mem::take(&mut self.runs) {
            if end > start {
                runs.push((fc_of(start), fc_of(end), grpprl));
            }
            start = end;
        }
        if start < total {
            runs.push((fc_of(start), fc_of(total), Vec::new()));
        }
        let (chpx_pages, chpx_bounds) = chpx_fkps(&runs);
        let first_chpx_page = word.len() / PAGE;
        for page in &chpx_pages {
            word.extend_from_slice(page);
        }
        let mut paragraphs: Vec<(u32, u32, u16, Vec<u8>)> = Vec::new();
        let mut start = 0;
        for (end, istd, grpprl) in &self.paragraphs {
            paragraphs.push((fc_of(start), fc_of(*end), *istd, grpprl.clone()));
            start = *end;
        }
        if start < total {
            paragraphs.push((fc_of(start), fc_of(total), 0, Vec::new()));
        }
        let (papx_pages, papx_bounds) = papx_fkps(&paragraphs);
        let first_papx_page = word.len() / PAGE;
        for page in &papx_pages {
            word.extend_from_slice(page);
        }
        // Section properties.
        let mut sepx_offsets = Vec::new();
        for (index, section) in sections.iter().enumerate() {
            let page = &section.page;
            let mut grpprl = Vec::new();
            if index > 0 {
                push_sprm(&mut grpprl, 0x3009, &[2]);
            }
            if page.landscape() {
                push_sprm(&mut grpprl, 0x301D, &[2]);
            }
            let clamp = |value: u32| value.min(31680) as u16;
            push_sprm(&mut grpprl, 0xB01F, &clamp(page.width).to_le_bytes());
            push_sprm(&mut grpprl, 0xB020, &clamp(page.height).to_le_bytes());
            push_sprm(&mut grpprl, 0xB021, &clamp(page.left).to_le_bytes());
            push_sprm(&mut grpprl, 0xB022, &clamp(page.right).to_le_bytes());
            push_sprm(&mut grpprl, 0x9023, &(clamp(page.top) as i16).to_le_bytes());
            push_sprm(
                &mut grpprl,
                0x9024,
                &(clamp(page.bottom) as i16).to_le_bytes(),
            );
            push_sprm(&mut grpprl, 0xB017, &clamp(page.header).to_le_bytes());
            push_sprm(&mut grpprl, 0xB018, &clamp(page.footer).to_le_bytes());
            if word.len() % 2 == 1 {
                word.push(0);
            }
            sepx_offsets.push(word.len() as u32);
            word.extend_from_slice(&(grpprl.len() as i16).to_le_bytes());
            word.extend_from_slice(&grpprl);
        }
        let cb_mac = word.len() as u32;

        // The table stream.
        let mut table: Vec<u8> = Vec::new();
        let mut pairs = [(0u32, 0u32); 93];
        let put =
            |table: &mut Vec<u8>, pairs: &mut [(u32, u32); 93], index: usize, bytes: &[u8]| {
                pairs[index] = (table.len() as u32, bytes.len() as u32);
                table.extend_from_slice(bytes);
            };
        let stsh = self.style_sheet(document);
        put(&mut table, &mut pairs, 1, &stsh);
        // fcStshfOrig names the same style sheet.
        let mut plcf_sed = Vec::new();
        plcf_sed.extend_from_slice(&0u32.to_le_bytes());
        for end in section_ends {
            plcf_sed.extend_from_slice(&end.to_le_bytes());
        }
        for offset in &sepx_offsets {
            plcf_sed.extend_from_slice(&0i16.to_le_bytes());
            plcf_sed.extend_from_slice(&(*offset as i32).to_le_bytes());
            plcf_sed.extend_from_slice(&0i16.to_le_bytes());
            plcf_sed.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        }
        put(&mut table, &mut pairs, 6, &plcf_sed);
        if ccp_hdd > 0 {
            let mut plcf_hdd = Vec::new();
            for cp in stories {
                plcf_hdd.extend_from_slice(&cp.to_le_bytes());
            }
            // The last CP: past the guard.
            plcf_hdd.extend_from_slice(&(stories.last().copied().unwrap_or(0) + 1).to_le_bytes());
            put(&mut table, &mut pairs, 11, &plcf_hdd);
        }
        put(
            &mut table,
            &mut pairs,
            12,
            &bin_table(&chpx_bounds, first_chpx_page),
        );
        put(
            &mut table,
            &mut pairs,
            13,
            &bin_table(&papx_bounds, first_papx_page),
        );
        put(&mut table, &mut pairs, 15, &self.font_table());
        if !self.fields.is_empty() {
            let mut plcf_fld = Vec::new();
            for (cp, _) in &self.fields {
                plcf_fld.extend_from_slice(&cp.to_le_bytes());
            }
            plcf_fld.extend_from_slice(&ccp_text.to_le_bytes());
            for (_, fld) in &self.fields {
                plcf_fld.extend_from_slice(fld);
            }
            put(&mut table, &mut pairs, 16, &plcf_fld);
        }
        put(&mut table, &mut pairs, 31, &dop());
        if !self.lists.is_empty() {
            let (lstfs, levels) = self.list_tables();
            put(&mut table, &mut pairs, 73, &lstfs);
            // The levels follow the LSTFs, outside lcbPlfLst.
            table.extend_from_slice(&levels);
            put(&mut table, &mut pairs, 74, &self.list_overrides());
        }
        let mut clx = vec![0x02];
        let mut plc_pcd = Vec::new();
        plc_pcd.extend_from_slice(&0u32.to_le_bytes());
        plc_pcd.extend_from_slice(&total.to_le_bytes());
        plc_pcd.extend_from_slice(&0u16.to_le_bytes());
        plc_pcd.extend_from_slice(&(TEXT_START as u32).to_le_bytes());
        plc_pcd.extend_from_slice(&0u16.to_le_bytes());
        clx.extend_from_slice(&(plc_pcd.len() as u32).to_le_bytes());
        clx.extend_from_slice(&plc_pcd);
        put(&mut table, &mut pairs, 33, &clx);
        pairs[0] = pairs[1];

        // The FIB.
        let mut fib = Vec::with_capacity(FIB_SIZE);
        fib.extend_from_slice(&0xA5ECu16.to_le_bytes());
        fib.extend_from_slice(&0x00C1u16.to_le_bytes());
        fib.extend_from_slice(&0u16.to_le_bytes());
        let lid: u16 = if self.japanese { 0x0411 } else { 0x0409 };
        fib.extend_from_slice(&lid.to_le_bytes());
        fib.extend_from_slice(&0u16.to_le_bytes());
        // fWhichTblStm and fExtChar, and fHasPic when there are pictures.
        let pictures: u16 = if self.has_pictures { 0x0008 } else { 0 };
        let flags: u16 = 0x0200 | 0x1000 | pictures | u16::from(self.template);
        fib.extend_from_slice(&flags.to_le_bytes());
        fib.extend_from_slice(&0x00BFu16.to_le_bytes());
        fib.extend_from_slice(&0u32.to_le_bytes());
        fib.extend_from_slice(&[0, 0]);
        fib.extend_from_slice(&0u16.to_le_bytes());
        fib.extend_from_slice(&0u16.to_le_bytes());
        fib.extend_from_slice(&0u32.to_le_bytes());
        fib.extend_from_slice(&0u32.to_le_bytes());
        debug_assert_eq!(fib.len(), 32);
        fib.extend_from_slice(&14u16.to_le_bytes());
        for index in 0..14 {
            // lidFE is the last.
            let value: u16 = if index == 13 { lid } else { 0 };
            fib.extend_from_slice(&value.to_le_bytes());
        }
        fib.extend_from_slice(&22u16.to_le_bytes());
        let mut longs = [0u32; 22];
        longs[0] = cb_mac;
        longs[3] = ccp_text;
        longs[5] = ccp_hdd;
        for value in longs {
            fib.extend_from_slice(&value.to_le_bytes());
        }
        fib.extend_from_slice(&93u16.to_le_bytes());
        for (fc, lcb) in pairs {
            fib.extend_from_slice(&fc.to_le_bytes());
            fib.extend_from_slice(&lcb.to_le_bytes());
        }
        fib.extend_from_slice(&0u16.to_le_bytes());
        debug_assert_eq!(fib.len(), FIB_SIZE);
        word[..fib.len()].copy_from_slice(&fib);

        let mut streams: Vec<(&str, &[u8])> = vec![("/WordDocument", &word), ("/1Table", &table)];
        if !self.data.is_empty() {
            streams.push(("/Data", &self.data));
        }
        compound_file::write(&streams)
    }

    /// The style sheet: Normal and Heading 1–9, Default Paragraph Font,
    /// and the reserved slots up to 15.
    fn style_sheet(&self, document: &FlowDocument) -> Vec<u8> {
        let mut styles: Vec<Vec<u8>> = Vec::new();
        let size = document
            .default_size
            .unwrap_or(if self.japanese { 21 } else { 24 });
        let east_asian = if self.fonts.len() > 1 { 1u16 } else { 0 };
        // Normal: the body face and size.
        let mut normal_chpx = Vec::new();
        push_sprm(&mut normal_chpx, 0x4A4F, &0u16.to_le_bytes());
        push_sprm(&mut normal_chpx, 0x4A50, &east_asian.to_le_bytes());
        push_sprm(&mut normal_chpx, 0x4A51, &0u16.to_le_bytes());
        push_sprm(&mut normal_chpx, 0x4A43, &(size as u16).to_le_bytes());
        styles.push(paragraph_style(0, 0x0FFF, 0, "Normal", &[], &normal_chpx));
        let heading_sizes = [32u16, 28, 26, 24, 22, 22, 21, 21, 21];
        for level in 1..=9u16 {
            let mut papx = Vec::new();
            push_sprm(&mut papx, 0x2406, &[1]);
            push_sprm(&mut papx, 0xA413, &240u16.to_le_bytes());
            push_sprm(&mut papx, 0xA414, &60u16.to_le_bytes());
            push_sprm(&mut papx, 0x2640, &[(level - 1) as u8]);
            let mut chpx = Vec::new();
            push_sprm(&mut chpx, 0x0835, &[1]);
            push_sprm(
                &mut chpx,
                0x4A43,
                &heading_sizes[level as usize - 1].to_le_bytes(),
            );
            styles.push(paragraph_style(
                level,
                0,
                0,
                &format!("heading {level}"),
                &papx,
                &chpx,
            ));
        }
        styles.push(character_style(65, "Default Paragraph Font"));
        for _ in 11..15 {
            styles.push(Vec::new());
        }
        let mut out = Vec::new();
        // Stshi: 18 bytes.
        let mut stshi = Vec::new();
        stshi.extend_from_slice(&(styles.len() as u16).to_le_bytes());
        stshi.extend_from_slice(&10u16.to_le_bytes());
        stshi.extend_from_slice(&1u16.to_le_bytes());
        stshi.extend_from_slice(&0x005Bu16.to_le_bytes());
        stshi.extend_from_slice(&15u16.to_le_bytes());
        stshi.extend_from_slice(&0u16.to_le_bytes());
        stshi.extend_from_slice(&0u16.to_le_bytes());
        stshi.extend_from_slice(&east_asian.to_le_bytes());
        stshi.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(stshi.len() as u16).to_le_bytes());
        out.extend_from_slice(&stshi);
        for style in styles {
            out.extend_from_slice(&(style.len() as u16).to_le_bytes());
            out.extend_from_slice(&style);
        }
        out
    }

    /// The PlfLst — an LSTF for each list — and the nine LVLs of each.
    fn list_tables(&self) -> (Vec<u8>, Vec<u8>) {
        let mut lstfs = (self.lists.len() as i16).to_le_bytes().to_vec();
        let mut levels = Vec::new();
        for (index, list) in self.lists.iter().enumerate() {
            lstfs.extend_from_slice(&list_id(index).to_le_bytes());
            lstfs.extend_from_slice(&list_id(index).to_le_bytes());
            for _ in 0..9 {
                lstfs.extend_from_slice(&0x0FFFu16.to_le_bytes());
            }
            // Nine levels (not fSimpleList), and fHybrid off.
            lstfs.push(0);
            lstfs.push(0);
            for (level, ordered) in list.iter().enumerate() {
                levels.extend_from_slice(&level_of(level, ordered.unwrap_or(true)));
            }
        }
        (lstfs, levels)
    }

    /// The PlfLfo: an LFO naming each list, then each one's LFOData.
    fn list_overrides(&self) -> Vec<u8> {
        let mut out = (self.lists.len() as u32).to_le_bytes().to_vec();
        for index in 0..self.lists.len() {
            out.extend_from_slice(&list_id(index).to_le_bytes());
            out.extend_from_slice(&[0; 12]);
        }
        for _ in &self.lists {
            out.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        }
        out
    }

    fn font_table(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.fonts.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        for (name, charset) in &self.fonts {
            let mut ffn = Vec::new();
            // prq variable, TrueType, family roman.
            ffn.push(0x02 | 0x04 | 0x10);
            ffn.extend_from_slice(&400i16.to_le_bytes());
            ffn.push(*charset);
            ffn.push(0);
            ffn.extend_from_slice(&[0; 10]);
            ffn.extend_from_slice(&[0; 24]);
            for unit in name.encode_utf16() {
                ffn.extend_from_slice(&unit.to_le_bytes());
            }
            ffn.extend_from_slice(&0u16.to_le_bytes());
            out.push(ffn.len() as u8);
            out.extend_from_slice(&ffn);
        }
        out
    }
}

/// A list's identifier: any number but -1 and 0, unique in the document.
fn list_id(index: usize) -> i32 {
    0x1A2B_0000 + index as i32
}

/// A list level's LVL: a decimal number followed by its punctuation —
/// `1.`, `(a)`-style numbering is Word's own — or a bullet, hanging from an
/// indent a quarter inch deeper than the level above.
fn level_of(level: usize, ordered: bool) -> Vec<u8> {
    let text: Vec<u16> = if ordered {
        vec![level as u16, '.' as u16]
    } else {
        vec!['•' as u16]
    };
    let mut papx = Vec::new();
    push_sprm(&mut papx, 0x840F, &(360 + 360 * level as i16).to_le_bytes());
    push_sprm(&mut papx, 0x8411, &(-360i16).to_le_bytes());
    let mut lvl = Vec::new();
    lvl.extend_from_slice(&1i32.to_le_bytes());
    lvl.push(if ordered { 0 } else { 23 });
    lvl.push(0);
    let mut placeholders = [0u8; 9];
    if ordered {
        placeholders[0] = 1;
    }
    lvl.extend_from_slice(&placeholders);
    // ixchFollow: a tab.
    lvl.push(0);
    lvl.extend_from_slice(&0i32.to_le_bytes());
    lvl.extend_from_slice(&0u32.to_le_bytes());
    lvl.push(0);
    lvl.push(papx.len() as u8);
    lvl.push(0);
    lvl.push(0);
    debug_assert_eq!(lvl.len(), 28);
    lvl.extend_from_slice(&papx);
    lvl.extend_from_slice(&(text.len() as u16).to_le_bytes());
    for unit in text {
        lvl.extend_from_slice(&unit.to_le_bytes());
    }
    lvl
}

/// A paragraph style's STD: sti, base, next, name, and its PAPX and CHPX.
fn paragraph_style(
    sti: u16,
    base: u16,
    next: u16,
    name: &str,
    papx: &[u8],
    chpx: &[u8],
) -> Vec<u8> {
    let mut std = Vec::new();
    std.extend_from_slice(&(sti & 0x0FFF).to_le_bytes());
    std.extend_from_slice(&(1 | (base << 4)).to_le_bytes());
    std.extend_from_slice(&(2 | (next << 4)).to_le_bytes());
    std.extend_from_slice(&0u16.to_le_bytes());
    std.extend_from_slice(&0u16.to_le_bytes());
    push_name(&mut std, name);
    // UpxPapx: the style's istd, then its sprms.
    let mut upx = sti.to_le_bytes().to_vec();
    upx.extend_from_slice(papx);
    push_upx(&mut std, &upx);
    push_upx(&mut std, chpx);
    let length = std.len() as u16;
    std[6..8].copy_from_slice(&length.to_le_bytes());
    std
}

fn character_style(sti: u16, name: &str) -> Vec<u8> {
    let mut std = Vec::new();
    std.extend_from_slice(&(sti & 0x0FFF).to_le_bytes());
    std.extend_from_slice(&(2u16 | (0x0FFF << 4)).to_le_bytes());
    std.extend_from_slice(&(1u16 | (10 << 4)).to_le_bytes());
    std.extend_from_slice(&0u16.to_le_bytes());
    std.extend_from_slice(&0u16.to_le_bytes());
    push_name(&mut std, name);
    push_upx(&mut std, &[]);
    let length = std.len() as u16;
    std[6..8].copy_from_slice(&length.to_le_bytes());
    std
}

fn push_name(std: &mut Vec<u8>, name: &str) {
    let units: Vec<u16> = name.encode_utf16().collect();
    std.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for unit in units {
        std.extend_from_slice(&unit.to_le_bytes());
    }
    std.extend_from_slice(&0u16.to_le_bytes());
}

fn push_upx(std: &mut Vec<u8>, upx: &[u8]) {
    std.extend_from_slice(&(upx.len() as u16).to_le_bytes());
    std.extend_from_slice(upx);
    if upx.len() % 2 == 1 {
        std.push(0);
    }
}

fn charset_of(font: &str) -> u8 {
    if !font.is_ascii()
        || [
            "MS Mincho",
            "MS Gothic",
            "MS PMincho",
            "MS PGothic",
            "Meiryo",
            "Yu Mincho",
            "Yu Gothic",
        ]
        .contains(&font)
    {
        128
    } else {
        0
    }
}

fn uid_of(data: &[u8]) -> [u8; 16] {
    let mut state: u64 = 0xcbf2_9ce4_8422_2325;
    let mut out = [0u8; 16];
    for (index, byte) in data.iter().enumerate() {
        state ^= *byte as u64;
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
        out[index % 16] ^= (state >> 24) as u8;
    }
    out
}

fn pad_to_page(bytes: &mut Vec<u8>) {
    let padded = bytes.len().div_ceil(PAGE) * PAGE;
    bytes.resize(padded, 0);
}

/// Character runs in CHPX FKPs: the pages, and each page's first FC (with
/// the last page's end after them).
fn chpx_fkps(runs: &[(u32, u32, Vec<u8>)]) -> (Vec<Vec<u8>>, Vec<u32>) {
    let mut pages = Vec::new();
    let mut bounds = Vec::new();
    let mut index = 0;
    while index < runs.len() {
        let mut page = vec![0u8; PAGE];
        let mut count = 0usize;
        let mut free_end = 511usize;
        let mut offsets: Vec<u8> = Vec::new();
        let start = index;
        while index < runs.len() {
            let grpprl = &runs[index].2;
            let entry = if grpprl.is_empty() {
                0
            } else {
                1 + grpprl.len()
            };
            let header = 4 * (count + 2) + (count + 1);
            let place = if entry == 0 {
                free_end
            } else {
                (free_end - entry) & !1
            };
            if place < header || count >= 101 {
                break;
            }
            if entry > 0 {
                page[place] = grpprl.len() as u8;
                page[place + 1..place + 1 + grpprl.len()].copy_from_slice(grpprl);
                free_end = place;
                offsets.push((place / 2) as u8);
            } else {
                offsets.push(0);
            }
            count += 1;
            index += 1;
        }
        for (slot, run) in runs[start..index].iter().enumerate() {
            page[slot * 4..slot * 4 + 4].copy_from_slice(&run.0.to_le_bytes());
        }
        page[count * 4..count * 4 + 4].copy_from_slice(&runs[index - 1].1.to_le_bytes());
        let rgb = (count + 1) * 4;
        page[rgb..rgb + count].copy_from_slice(&offsets);
        page[511] = count as u8;
        bounds.push(runs[start].0);
        pages.push(page);
    }
    if let Some(last) = runs.last() {
        bounds.push(last.1);
    }
    (pages, bounds)
}

/// Paragraphs in PAPX FKPs, as [`chpx_fkps`].
fn papx_fkps(paragraphs: &[(u32, u32, u16, Vec<u8>)]) -> (Vec<Vec<u8>>, Vec<u32>) {
    let mut pages = Vec::new();
    let mut bounds = Vec::new();
    let mut index = 0;
    while index < paragraphs.len() {
        let mut page = vec![0u8; PAGE];
        let mut count = 0usize;
        let mut free_end = 511usize;
        let mut offsets: Vec<u8> = Vec::new();
        let start = index;
        while index < paragraphs.len() {
            let (_, _, istd, grpprl) = &paragraphs[index];
            let length = 2 + grpprl.len();
            let mut entry = Vec::new();
            if length % 2 == 1 {
                entry.push(length.div_ceil(2) as u8);
            } else {
                entry.push(0);
                entry.push((length / 2) as u8);
            }
            entry.extend_from_slice(&istd.to_le_bytes());
            entry.extend_from_slice(grpprl);
            let header = 4 * (count + 2) + 13 * (count + 1);
            let place = (free_end.saturating_sub(entry.len())) & !1;
            if place < header || entry.len() > 480 {
                break;
            }
            page[place..place + entry.len()].copy_from_slice(&entry);
            free_end = place;
            offsets.push((place / 2) as u8);
            count += 1;
            index += 1;
        }
        if count == 0 {
            // A paragraph too big for a page: written with no properties.
            let mut page = vec![0u8; PAGE];
            page[0..4].copy_from_slice(&paragraphs[index].0.to_le_bytes());
            page[4..8].copy_from_slice(&paragraphs[index].1.to_le_bytes());
            page[511] = 1;
            bounds.push(paragraphs[index].0);
            pages.push(page);
            index += 1;
            continue;
        }
        for (slot, paragraph) in paragraphs[start..index].iter().enumerate() {
            page[slot * 4..slot * 4 + 4].copy_from_slice(&paragraph.0.to_le_bytes());
        }
        page[count * 4..count * 4 + 4].copy_from_slice(&paragraphs[index - 1].1.to_le_bytes());
        let rgbx = (count + 1) * 4;
        for (slot, offset) in offsets.iter().enumerate() {
            page[rgbx + slot * 13] = *offset;
        }
        page[511] = count as u8;
        bounds.push(paragraphs[start].0);
        pages.push(page);
    }
    if let Some(last) = paragraphs.last() {
        bounds.push(last.1);
    }
    (pages, bounds)
}

/// A bin table: the FKPs' first FCs and the last end, then their page
/// numbers.
fn bin_table(bounds: &[u32], first_page: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for bound in bounds {
        out.extend_from_slice(&bound.to_le_bytes());
    }
    for page in 0..bounds.len().saturating_sub(1) {
        out.extend_from_slice(&((first_page + page) as u32).to_le_bytes());
    }
    out
}

/// The document properties (Dop97): widow control on and a half-inch
/// default tab stop; the rest Word's defaults of zero.
fn dop() -> Vec<u8> {
    let mut dop = vec![0u8; 500];
    // fWidowControl.
    dop[0] = 0x02;
    // nFtn = 1.
    dop[2..4].copy_from_slice(&(1u16 << 2).to_le_bytes());
    // dxaTab.
    dop[10..12].copy_from_slice(&720u16.to_le_bytes());
    dop
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FlowDocument {
        let mut heading = Paragraph::with_text("第一章 概要");
        heading.kind = ParagraphKind::Heading(1);
        let mut body = Paragraph {
            alignment: Some(Alignment::Justify),
            indent_left: 720,
            indent_first_line: 360,
            space_after: Some(120),
            ..Paragraph::default()
        };
        body.push_text("通常の文と ", &CharFormat::default());
        body.push_text(
            "太字",
            &CharFormat {
                bold: true,
                ..CharFormat::default()
            },
        );
        body.push_text(" と ", &CharFormat::default());
        body.push_text(
            "red italic",
            &CharFormat {
                italic: true,
                color: Some("FF0000".into()),
                size: Some(28),
                font: Some("Arial".into()),
                ..CharFormat::default()
            },
        );
        body.inlines.push(Inline::Tab);
        body.push_text(
            "link",
            &CharFormat {
                link: Some("https://example.com/".into()),
                ..CharFormat::default()
            },
        );
        let table = Table {
            columns: vec![2000, 2000, 2000],
            rows: vec![
                Row {
                    cells: vec![
                        Cell {
                            span: 2,
                            ..Cell::with_text("wide")
                        },
                        Cell {
                            vertical_merge: VerticalMerge::Restart,
                            ..Cell::with_text("tall")
                        },
                    ],
                    header: true,
                },
                Row {
                    cells: vec![
                        Cell::with_text("a"),
                        Cell::with_text("b"),
                        Cell {
                            vertical_merge: VerticalMerge::Continue,
                            ..Cell::default()
                        },
                    ],
                    header: false,
                },
            ],
            borders: true,
        };
        FlowDocument {
            title: None,
            author: None,
            default_font: Some("Times New Roman".into()),
            default_size: Some(21),
            language: Some("ja-JP".into()),
            sections: vec![
                FlowSection {
                    blocks: vec![
                        Block::Paragraph(heading),
                        Block::Paragraph(body),
                        Block::Table(table),
                    ],
                    header: vec![Block::Paragraph(Paragraph::with_text("ヘッダー"))],
                    footer: vec![Block::Paragraph(Paragraph::with_text("フッター"))],
                    ..FlowSection::default()
                },
                FlowSection {
                    blocks: vec![Block::Paragraph(Paragraph::with_text("横向きの節"))],
                    page: PageSetup {
                        width: 16838,
                        height: 11906,
                        ..PageSetup::default()
                    },
                    ..FlowSection::default()
                },
            ],
        }
    }

    #[test]
    fn fkps_hold_their_runs_and_point_back_to_them() {
        let runs: Vec<(u32, u32, Vec<u8>)> = (0..300u32)
            .map(|index| {
                (
                    1024 + index * 2,
                    1026 + index * 2,
                    if index % 2 == 0 {
                        vec![0x35, 0x08, 1]
                    } else {
                        Vec::new()
                    },
                )
            })
            .collect();
        let (pages, bounds) = chpx_fkps(&runs);
        assert!(pages.len() > 1, "300 runs take more than one page");
        assert_eq!(bounds.len(), pages.len() + 1);
        assert_eq!(*bounds.last().unwrap(), 1026 + 299 * 2);
        let total: usize = pages.iter().map(|page| page[511] as usize).sum();
        assert_eq!(total, 300);
    }

    /// The browser build writes `.doc` files on `wasm32-unknown-unknown`,
    /// where asking for the time panics: the file carries no time at all,
    /// and still reads back.
    #[test]
    fn a_document_is_written_without_asking_the_clock_and_reads_back() {
        let bytes = write_doc(&sample()).unwrap();
        let file = cfb::CompoundFile::open(std::io::Cursor::new(bytes.clone())).unwrap();
        // A zero CFB time, 1601-01-01.
        let no_time = std::time::UNIX_EPOCH - std::time::Duration::from_secs(11_644_473_600);
        for entry in file.walk() {
            assert_eq!(entry.created(), no_time, "{}", entry.path().display());
            assert_eq!(entry.modified(), no_time, "{}", entry.path().display());
        }
        let read = jade_doc_reader::read_doc(&bytes).unwrap();
        assert_eq!(read.paragraphs()[0].text(), "第一章 概要");
    }

    #[test]
    fn a_written_document_reads_back() {
        let document = sample();
        let bytes = write_doc(&document).unwrap();
        assert!(jade_doc_reader::is_doc(&bytes));
        let read = jade_doc_reader::read_doc(&bytes).unwrap();
        assert_eq!(read.sections.len(), 2);
        assert!(read.sections[1].page.landscape());
        let paragraphs = read.paragraphs();
        assert_eq!(paragraphs[0].kind, ParagraphKind::Heading(1));
        assert_eq!(paragraphs[0].text(), "第一章 概要");
        assert_eq!(paragraphs[1].text(), "通常の文と 太字 と red italic\tlink");
        assert_eq!(paragraphs[1].alignment, Some(Alignment::Justify));
        assert_eq!(paragraphs[1].indent_left, 720);
        let formats: Vec<(String, CharFormat)> = paragraphs[1]
            .inlines
            .iter()
            .filter_map(|inline| match inline {
                Inline::Text(text, format) => Some((text.clone(), format.clone())),
                _ => None,
            })
            .collect();
        assert!(formats
            .iter()
            .any(|(text, format)| text == "太字" && format.bold));
        let red = formats
            .iter()
            .find(|(text, _)| text == "red italic")
            .unwrap();
        assert!(red.1.italic);
        assert_eq!(red.1.color.as_deref(), Some("FF0000"));
        assert_eq!(red.1.size, Some(28));
        assert_eq!(red.1.font.as_deref(), Some("Arial"));
        let link = formats.iter().find(|(text, _)| text == "link").unwrap();
        assert_eq!(link.1.link.as_deref(), Some("https://example.com/"));
        let table = read.sections[0]
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Table(table) => Some(table),
                _ => None,
            })
            .expect("the table reads back");
        assert_eq!(table.rows.len(), 2);
        assert!(table.borders);
        assert_eq!(table.rows[0].cells[0].span, 2);
        assert_eq!(
            table.rows[0].cells[1].vertical_merge,
            VerticalMerge::Restart
        );
        assert_eq!(
            table.rows[1].cells[2].vertical_merge,
            VerticalMerge::Continue
        );
        let text = |blocks: &[Block]| FlowDocument::single(blocks.to_vec()).plain_text();
        assert_eq!(text(&read.sections[0].header).trim(), "ヘッダー");
        assert_eq!(text(&read.sections[0].footer).trim(), "フッター");
    }
}
