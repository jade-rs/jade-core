//! Plain text (`.txt`), in and out, in the encodings Word's ファイルの変換
//! dialog offers: Unicode in its forms, the Japanese ones (Shift_JIS, EUC,
//! JIS) and the Western single-byte ones.

use encoding_rs::Encoding;

use crate::flow::*;

/// An encoding a text file can be read or written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    /// UTF-8 with no byte order mark.
    Utf8,
    /// UTF-8 starting with a byte order mark, as Windows Notepad writes it.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// 日本語 (シフト JIS): Windows' code page 932.
    ShiftJis,
    /// 日本語 (EUC)
    EucJp,
    /// 日本語 (JIS): ISO-2022-JP.
    Iso2022Jp,
    /// 西ヨーロッパ (Windows): code page 1252.
    Windows1252,
    /// 西ヨーロッパ (Mac).
    MacRoman,
    /// 簡体字中国語 (GB18030).
    Gb18030,
    /// 繁体字中国語 (Big5).
    Big5,
    /// 韓国語.
    EucKr,
}

impl TextEncoding {
    /// Every encoding, in the order the dialog lists them.
    pub const ALL: [TextEncoding; 12] = [
        TextEncoding::Utf8,
        TextEncoding::Utf8Bom,
        TextEncoding::ShiftJis,
        TextEncoding::EucJp,
        TextEncoding::Iso2022Jp,
        TextEncoding::Utf16Le,
        TextEncoding::Utf16Be,
        TextEncoding::Windows1252,
        TextEncoding::MacRoman,
        TextEncoding::Gb18030,
        TextEncoding::Big5,
        TextEncoding::EucKr,
    ];

    /// The name Word's dialog lists it under, in Japanese.
    pub fn label_ja(self) -> &'static str {
        match self {
            Self::Utf8 => "Unicode (UTF-8)",
            Self::Utf8Bom => "Unicode (UTF-8、BOM 付き)",
            Self::Utf16Le => "Unicode (UTF-16 LE)",
            Self::Utf16Be => "Unicode (UTF-16 BE)",
            Self::ShiftJis => "日本語 (シフト JIS)",
            Self::EucJp => "日本語 (EUC)",
            Self::Iso2022Jp => "日本語 (JIS)",
            Self::Windows1252 => "西ヨーロッパ言語 (Windows)",
            Self::MacRoman => "西ヨーロッパ言語 (Mac)",
            Self::Gb18030 => "簡体字中国語 (GB18030)",
            Self::Big5 => "繁体字中国語 (Big5)",
            Self::EucKr => "韓国語",
        }
    }

    /// The name English Word lists it under.
    pub fn label_en(self) -> &'static str {
        match self {
            Self::Utf8 => "Unicode (UTF-8)",
            Self::Utf8Bom => "Unicode (UTF-8 with BOM)",
            Self::Utf16Le => "Unicode (UTF-16 LE)",
            Self::Utf16Be => "Unicode (UTF-16 BE)",
            Self::ShiftJis => "Japanese (Shift JIS)",
            Self::EucJp => "Japanese (EUC)",
            Self::Iso2022Jp => "Japanese (JIS)",
            Self::Windows1252 => "Western European (Windows)",
            Self::MacRoman => "Western European (Mac)",
            Self::Gb18030 => "Chinese Simplified (GB18030)",
            Self::Big5 => "Chinese Traditional (Big5)",
            Self::EucKr => "Korean",
        }
    }

    /// A short stable key, for settings and the browser's page.
    pub fn key(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf8Bom => "utf-8-bom",
            Self::Utf16Le => "utf-16le",
            Self::Utf16Be => "utf-16be",
            Self::ShiftJis => "shift_jis",
            Self::EucJp => "euc-jp",
            Self::Iso2022Jp => "iso-2022-jp",
            Self::Windows1252 => "windows-1252",
            Self::MacRoman => "macintosh",
            Self::Gb18030 => "gb18030",
            Self::Big5 => "big5",
            Self::EucKr => "euc-kr",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|encoding| encoding.key().eq_ignore_ascii_case(key))
    }

    /// The encoding a `charset` label (HTML's, RTF's code page names) means.
    pub fn from_label(label: &str) -> Option<Self> {
        let encoding = Encoding::for_label(label.trim().as_bytes())?;
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.encoding() == encoding && *candidate != Self::Utf8Bom)
    }

    fn encoding(self) -> &'static Encoding {
        match self {
            Self::Utf8 | Self::Utf8Bom => encoding_rs::UTF_8,
            Self::Utf16Le => encoding_rs::UTF_16LE,
            Self::Utf16Be => encoding_rs::UTF_16BE,
            Self::ShiftJis => encoding_rs::SHIFT_JIS,
            Self::EucJp => encoding_rs::EUC_JP,
            Self::Iso2022Jp => encoding_rs::ISO_2022_JP,
            Self::Windows1252 => encoding_rs::WINDOWS_1252,
            Self::MacRoman => encoding_rs::MACINTOSH,
            Self::Gb18030 => encoding_rs::GB18030,
            Self::Big5 => encoding_rs::BIG5,
            Self::EucKr => encoding_rs::EUC_KR,
        }
    }

    /// `bytes` as text. A byte order mark is dropped; characters that do
    /// not decode become U+FFFD.
    pub fn decode(self, bytes: &[u8]) -> String {
        let bytes = match self {
            Self::Utf8 | Self::Utf8Bom => bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes),
            Self::Utf16Le => bytes.strip_prefix(b"\xff\xfe").unwrap_or(bytes),
            Self::Utf16Be => bytes.strip_prefix(b"\xfe\xff").unwrap_or(bytes),
            _ => bytes,
        };
        let (text, _) = self.encoding().decode_without_bom_handling(bytes);
        text.into_owned()
    }

    /// `text` in this encoding, with how many characters it cannot carry
    /// (each written as `?`, as Word writes them).
    pub fn encode(self, text: &str) -> (Vec<u8>, usize) {
        match self {
            Self::Utf8 => (text.as_bytes().to_vec(), 0),
            Self::Utf8Bom => {
                let mut out = b"\xef\xbb\xbf".to_vec();
                out.extend_from_slice(text.as_bytes());
                (out, 0)
            }
            // encoding_rs only writes UTF-8 for these; they are written here.
            Self::Utf16Le => {
                let mut out = vec![0xff, 0xfe];
                for unit in text.encode_utf16() {
                    out.extend_from_slice(&unit.to_le_bytes());
                }
                (out, 0)
            }
            Self::Utf16Be => {
                let mut out = vec![0xfe, 0xff];
                for unit in text.encode_utf16() {
                    out.extend_from_slice(&unit.to_be_bytes());
                }
                (out, 0)
            }
            _ => {
                let mut encoder = self.encoding().new_encoder();
                let mut out = Vec::with_capacity(text.len() * 2);
                let mut lost = 0;
                let mut buffer = [0u8; 16];
                for ch in text.chars() {
                    let mut utf8 = [0u8; 4];
                    let piece = ch.encode_utf8(&mut utf8);
                    let (result, _, written) =
                        encoder.encode_from_utf8_without_replacement(piece, &mut buffer, false);
                    match result {
                        encoding_rs::EncoderResult::InputEmpty => {
                            out.extend_from_slice(&buffer[..written])
                        }
                        _ => {
                            out.extend_from_slice(&buffer[..written]);
                            out.push(b'?');
                            lost += 1;
                        }
                    }
                }
                let (_, _, written) =
                    encoder.encode_from_utf8_without_replacement("", &mut buffer, true);
                out.extend_from_slice(&buffer[..written]);
                (out, lost)
            }
        }
    }

    /// Characters of `text` this encoding cannot carry, each once, in the
    /// order they first appear: what the dialog warns will be lost.
    pub fn unencodable(self, text: &str) -> Vec<char> {
        let mut out = Vec::new();
        if matches!(
            self,
            Self::Utf8 | Self::Utf8Bom | Self::Utf16Le | Self::Utf16Be | Self::Gb18030
        ) {
            return out;
        }
        for ch in text.chars() {
            if out.contains(&ch) {
                continue;
            }
            let (_, lost) = self.encode(&ch.to_string());
            if lost > 0 {
                out.push(ch);
            }
        }
        out
    }
}

/// The encoding `bytes` are most likely in: a byte order mark says; text
/// that is valid UTF-8 is UTF-8; then the Japanese encodings by which
/// decodes cleanly (as Word guesses a Japanese file); then Windows-1252.
pub fn detect_encoding(bytes: &[u8]) -> TextEncoding {
    if bytes.starts_with(b"\xef\xbb\xbf") {
        return TextEncoding::Utf8Bom;
    }
    if bytes.starts_with(b"\xff\xfe") {
        return TextEncoding::Utf16Le;
    }
    if bytes.starts_with(b"\xfe\xff") {
        return TextEncoding::Utf16Be;
    }
    // UTF-16 without a mark: every other byte zero in ASCII text.
    if bytes.len() >= 4 {
        let sample = &bytes[..bytes.len().min(512) & !1];
        let odd_zeros = sample
            .iter()
            .skip(1)
            .step_by(2)
            .filter(|byte| **byte == 0)
            .count();
        let even_zeros = sample.iter().step_by(2).filter(|byte| **byte == 0).count();
        let half = sample.len() / 2;
        if odd_zeros * 10 > half * 6 && even_zeros * 10 < half {
            return TextEncoding::Utf16Le;
        }
        if even_zeros * 10 > half * 6 && odd_zeros * 10 < half {
            return TextEncoding::Utf16Be;
        }
    }
    // JIS is seven-bit, so valid UTF-8 too: its escapes give it away.
    if bytes.contains(&0x1b) && bytes.windows(3).any(|w| w == b"\x1b$B" || w == b"\x1b$@") {
        return TextEncoding::Iso2022Jp;
    }
    if std::str::from_utf8(bytes).is_ok() {
        return TextEncoding::Utf8;
    }
    let clean = |encoding: TextEncoding| {
        let (_, had_errors) = encoding.encoding().decode_without_bom_handling(bytes);
        !had_errors
    };
    let shift_jis = clean(TextEncoding::ShiftJis);
    let euc = clean(TextEncoding::EucJp);
    match (shift_jis, euc) {
        (true, false) => TextEncoding::ShiftJis,
        (false, true) => TextEncoding::EucJp,
        // Both decode: the one with more kana and kanji is the one meant.
        (true, true) => {
            let score = |encoding: TextEncoding| {
                encoding
                    .decode(bytes)
                    .chars()
                    .filter(|ch| matches!(*ch as u32, 0x3040..=0x30ff | 0x4e00..=0x9fff))
                    .count()
            };
            if score(TextEncoding::EucJp) > score(TextEncoding::ShiftJis) {
                TextEncoding::EucJp
            } else {
                TextEncoding::ShiftJis
            }
        }
        (false, false) => TextEncoding::Windows1252,
    }
}

/// How lines end in a written text file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// CR LF, as Windows and Word write.
    #[default]
    CrLf,
    Lf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CrLf => "\r\n",
            Self::Lf => "\n",
            Self::Cr => "\r",
        }
    }
}

/// A text file as a document: a paragraph per line, a form feed a page
/// break, in the face Word opens text in.
pub fn read_text(bytes: &[u8], encoding: Option<TextEncoding>) -> FlowDocument {
    let encoding = encoding.unwrap_or_else(|| detect_encoding(bytes));
    let text = encoding.decode(bytes);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = Vec::new();
    let mut page_break = false;
    for line in text.split('\n') {
        let mut paragraph = Paragraph {
            page_break_before: std::mem::take(&mut page_break),
            ..Paragraph::default()
        };
        let mut pieces = line.split('\u{c}').peekable();
        while let Some(piece) = pieces.next() {
            for (index, part) in piece.split('\t').enumerate() {
                if index > 0 {
                    paragraph.inlines.push(Inline::Tab);
                }
                paragraph.push_text(part, &CharFormat::default());
            }
            if pieces.peek().is_some() {
                paragraph.inlines.push(Inline::PageBreak);
            }
        }
        blocks.push(Block::Paragraph(paragraph));
    }
    // The newline ending the last line ends no paragraph of its own.
    if text.ends_with('\n') && blocks.len() > 1 {
        blocks.pop();
    }
    let mut document = FlowDocument::single(blocks);
    // Word opens a text file in the body face of a new document.
    if matches!(
        encoding,
        TextEncoding::ShiftJis | TextEncoding::EucJp | TextEncoding::Iso2022Jp
    ) || text
        .chars()
        .any(|ch| matches!(ch as u32, 0x3040..=0x30ff | 0x4e00..=0x9fff))
    {
        document.language = Some("ja-JP".into());
    }
    document
}

/// What a text file is written with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextOptions {
    pub line_ending: LineEnding,
    /// Whether hidden text is written too.
    pub include_hidden: bool,
}

/// A document as plain text, as Word's テキストのみ保存 writes it: a line
/// per paragraph, list numbers and bullets in front, table cells separated
/// by tabs and rows by lines, a form feed between sections.
pub fn write_text(document: &FlowDocument, options: TextOptions) -> String {
    let newline = options.line_ending.as_str();
    let mut out = String::new();
    let mut counters = [0usize; 9];
    for (index, section) in document.sections.iter().enumerate() {
        if index > 0 {
            out.push('\u{c}');
        }
        for block in &section.blocks {
            write_block(block, options, newline, &mut counters, &mut out);
        }
    }
    out
}

fn write_block(
    block: &Block,
    options: TextOptions,
    newline: &str,
    counters: &mut [usize; 9],
    out: &mut String,
) {
    match block {
        Block::Paragraph(paragraph) => {
            match paragraph.list {
                Some(item) => {
                    let level = item.level.min(8) as usize;
                    if item.ordered {
                        counters[level] += 1;
                        for deeper in counters.iter_mut().skip(level + 1) {
                            *deeper = 0;
                        }
                        out.push_str(&format!("{}.\t", counters[level]));
                    } else {
                        out.push_str("•\t");
                    }
                }
                None => *counters = [0; 9],
            }
            out.push_str(&paragraph_text(paragraph, options, newline));
            out.push_str(newline);
        }
        Block::Table(table) => {
            *counters = [0; 9];
            for row in &table.rows {
                let cells: Vec<String> = row
                    .cells
                    .iter()
                    .map(|cell| {
                        let mut text = String::new();
                        let mut inner = [0usize; 9];
                        for block in &cell.blocks {
                            write_block(block, options, " ", &mut inner, &mut text);
                        }
                        text.trim_end().to_string()
                    })
                    .collect();
                out.push_str(&cells.join("\t"));
                out.push_str(newline);
            }
        }
    }
}

fn paragraph_text(paragraph: &Paragraph, options: TextOptions, newline: &str) -> String {
    let mut out = String::new();
    for inline in &paragraph.inlines {
        match inline {
            Inline::Text(text, format) => {
                if format.hidden && !options.include_hidden {
                    continue;
                }
                if format.all_caps {
                    out.push_str(&text.to_uppercase());
                } else {
                    out.push_str(text);
                }
            }
            Inline::Tab => out.push('\t'),
            Inline::LineBreak => out.push_str(newline),
            Inline::PageBreak => out.push('\u{c}'),
            Inline::Image(_) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_jis_text_is_recognised_and_read() {
        let (bytes, lost) = TextEncoding::ShiftJis.encode("日本語のテキスト\r\n二行目\tタブ");
        assert_eq!(lost, 0);
        assert_eq!(detect_encoding(&bytes), TextEncoding::ShiftJis);
        let document = read_text(&bytes, None);
        let paragraphs = document.paragraphs();
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(paragraphs[0].text(), "日本語のテキスト");
        assert_eq!(paragraphs[1].text(), "二行目\tタブ");
        assert_eq!(document.language.as_deref(), Some("ja-JP"));
    }

    #[test]
    fn each_encoding_round_trips_what_it_can_carry() {
        for encoding in TextEncoding::ALL {
            let text = if matches!(
                encoding,
                TextEncoding::Windows1252
                    | TextEncoding::MacRoman
                    | TextEncoding::Big5
                    | TextEncoding::EucKr
                    | TextEncoding::Gb18030
            ) {
                "Caf\u{e9} cr\u{e8}me"
            } else {
                "テキスト text"
            };
            let text = if matches!(
                encoding,
                TextEncoding::Big5 | TextEncoding::EucKr | TextEncoding::Gb18030
            ) {
                "plain text"
            } else {
                text
            };
            let (bytes, lost) = encoding.encode(text);
            assert_eq!(lost, 0, "{encoding:?}");
            assert_eq!(encoding.decode(&bytes), text, "{encoding:?}");
        }
        assert_eq!(
            detect_encoding(&TextEncoding::Utf16Le.encode("abc").0),
            TextEncoding::Utf16Le
        );
        assert_eq!(
            detect_encoding(&TextEncoding::EucJp.encode("日本語のかな").0),
            TextEncoding::EucJp
        );
        assert_eq!(
            detect_encoding(&TextEncoding::Iso2022Jp.encode("日本語").0),
            TextEncoding::Iso2022Jp
        );
    }

    #[test]
    fn what_an_encoding_cannot_carry_is_named_and_written_as_a_question_mark() {
        assert_eq!(TextEncoding::ShiftJis.unencodable("あ😀あ"), vec!['😀']);
        let (bytes, lost) = TextEncoding::ShiftJis.encode("あ😀");
        assert_eq!(lost, 1);
        assert_eq!(TextEncoding::ShiftJis.decode(&bytes), "あ?");
    }

    #[test]
    fn written_text_numbers_lists_and_tabs_tables() {
        let mut first = Paragraph::with_text("first");
        first.list = Some(ListItem {
            ordered: true,
            level: 0,
        });
        let mut second = Paragraph::with_text("second");
        second.list = Some(ListItem {
            ordered: true,
            level: 0,
        });
        let mut hidden = Paragraph::default();
        hidden.push_text("shown", &CharFormat::default());
        hidden.push_text(
            " hidden",
            &CharFormat {
                hidden: true,
                ..CharFormat::default()
            },
        );
        let table = Table {
            rows: vec![Row {
                cells: vec![Cell::with_text("a"), Cell::with_text("b")],
                header: false,
            }],
            ..Table::default()
        };
        let document = FlowDocument::single(vec![
            Block::Paragraph(first),
            Block::Paragraph(second),
            Block::Paragraph(hidden),
            Block::Table(table),
        ]);
        let text = write_text(&document, TextOptions::default());
        assert_eq!(text, "1.\tfirst\r\n2.\tsecond\r\nshown\r\na\tb\r\n");
        let text = write_text(
            &document,
            TextOptions {
                line_ending: LineEnding::Lf,
                include_hidden: true,
            },
        );
        assert!(text.contains("shown hidden\n"));
    }
}
