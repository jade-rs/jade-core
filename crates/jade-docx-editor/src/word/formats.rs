//! The file formats a Word document opens from and saves to, beyond
//! `.docx`: Word 97–2003 (`.doc`), RTF, plain text in any encoding, ODT,
//! HTML and — natively — PDF. Both shells open and save through here: a
//! file in another format is converted to `.docx` for the editor when it
//! opens (`jade-docx-convert`, `jade-doc-reader`), and the editor's
//! `.docx` is converted back when it is saved in that format.
//!
//! A template (`.dotx`, `.dotm`) is the same package with the template's
//! main content type, which [`template_from_document`] declares.

use jade_docx_convert::text::TextOptions;
pub use jade_docx_convert::text::{LineEnding, TextEncoding};
use jade_docx_convert::{html, odt, rtf, text, FlowDocument};
use jade_opc::OpcPackage;

/// A format Word reads or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WordFormat {
    Docx,
    Docm,
    Dotx,
    Dotm,
    /// Word 97–2004 文書.
    Doc,
    /// Word 97–2004 テンプレート (`.dot`): read as a `.doc`.
    Dot,
    Rtf,
    Text,
    Odt,
    Html,
    Pdf,
}

impl WordFormat {
    /// Every format, in the order Word's ファイル形式 menu lists them.
    pub const ALL: [WordFormat; 11] = [
        WordFormat::Docx,
        WordFormat::Docm,
        WordFormat::Dotx,
        WordFormat::Dotm,
        WordFormat::Doc,
        WordFormat::Dot,
        WordFormat::Rtf,
        WordFormat::Text,
        WordFormat::Odt,
        WordFormat::Html,
        WordFormat::Pdf,
    ];

    /// The format a file name's extension names.
    pub fn of_name(name: &str) -> Option<Self> {
        let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match extension.as_str() {
            "docx" => Self::Docx,
            "docm" => Self::Docm,
            "dotx" => Self::Dotx,
            "dotm" => Self::Dotm,
            "doc" | "wbk" => Self::Doc,
            "dot" => Self::Dot,
            "rtf" => Self::Rtf,
            "txt" | "text" => Self::Text,
            "odt" | "fodt" => Self::Odt,
            "htm" | "html" | "xhtml" => Self::Html,
            "pdf" => Self::Pdf,
            _ => return None,
        })
    }

    /// What `bytes` are, read from their first bytes rather than a name:
    /// a `.doc` saved with a `.docx` name, or RTF called `.doc`, as Word
    /// itself writes them when asked to.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"PK") {
            let package = OpcPackage::open(bytes).ok()?;
            if package
                .part("mimetype")
                .is_some_and(|mime| mime.starts_with(b"application/vnd.oasis.opendocument.text"))
            {
                return Some(Self::Odt);
            }
            let types = package.part_str("[Content_Types].xml").ok().flatten()?;
            return Some(if types.contains("macroEnabledTemplate.main") {
                Self::Dotm
            } else if types.contains("template.main") {
                Self::Dotx
            } else if types.contains("macroEnabled.main") {
                Self::Docm
            } else {
                Self::Docx
            });
        }
        if jade_doc_reader::is_doc(bytes) {
            return Some(Self::Doc);
        }
        let head = bytes.trim_ascii_start();
        if head.starts_with(b"{\\rtf") {
            return Some(Self::Rtf);
        }
        if head.starts_with(b"%PDF") {
            return Some(Self::Pdf);
        }
        let lower = String::from_utf8_lossy(&head[..head.len().min(512)]).to_ascii_lowercase();
        if lower.starts_with("<!doctype html")
            || lower.starts_with("<html")
            || lower.contains("<html")
        {
            return Some(Self::Html);
        }
        if lower.starts_with("<?xml") && lower.contains("opendocument") {
            return Some(Self::Odt);
        }
        None
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Docm => "docm",
            Self::Dotx => "dotx",
            Self::Dotm => "dotm",
            Self::Doc => "doc",
            Self::Dot => "dot",
            Self::Rtf => "rtf",
            Self::Text => "txt",
            Self::Odt => "odt",
            Self::Html => "html",
            Self::Pdf => "pdf",
        }
    }

    /// The name Word for Mac's ファイル形式 menu gives it.
    pub fn label(self, japanese: bool) -> &'static str {
        match (self, japanese) {
            (Self::Docx, true) => "Word 文書 (.docx)",
            (Self::Docx, false) => "Word Document (.docx)",
            (Self::Docm, true) => "Word マクロ有効文書 (.docm)",
            (Self::Docm, false) => "Word Macro-Enabled Document (.docm)",
            (Self::Dotx, true) => "Word テンプレート (.dotx)",
            (Self::Dotx, false) => "Word Template (.dotx)",
            (Self::Dotm, true) => "Word マクロ有効テンプレート (.dotm)",
            (Self::Dotm, false) => "Word Macro-Enabled Template (.dotm)",
            (Self::Doc, true) => "Word 97-2004 文書 (.doc)",
            (Self::Doc, false) => "Word 97-2004 Document (.doc)",
            (Self::Dot, true) => "Word 97-2004 テンプレート (.dot)",
            (Self::Dot, false) => "Word 97-2004 Template (.dot)",
            (Self::Rtf, true) => "リッチ テキスト形式 (.rtf)",
            (Self::Rtf, false) => "Rich Text Format (.rtf)",
            (Self::Text, true) => "書式なし (.txt)",
            (Self::Text, false) => "Plain Text (.txt)",
            (Self::Odt, true) => "OpenDocument テキスト (.odt)",
            (Self::Odt, false) => "OpenDocument Text (.odt)",
            (Self::Html, true) => "Web ページ (.htm)",
            (Self::Html, false) => "Web Page (.htm)",
            (Self::Pdf, true) => "PDF",
            (Self::Pdf, false) => "PDF",
        }
    }

    /// Whether the editor's own package is the file: nothing to convert.
    pub fn is_package(self) -> bool {
        matches!(self, Self::Docx | Self::Docm | Self::Dotx | Self::Dotm)
    }

    /// Whether saving in this format keeps less than the editor has, so
    /// Word warns first: text keeps only the words, the others lose what
    /// they have no place for.
    pub fn loses_features(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Rtf | Self::Odt | Self::Html | Self::Doc | Self::Dot
        )
    }

    /// Whether this is one of the formats a Word document opens from.
    pub fn opens(self) -> bool {
        !matches!(self, Self::Pdf)
    }

    /// Whether a document in this format opens in 互換モード: the binary
    /// formats are Word 2003's.
    pub fn is_legacy(self) -> bool {
        matches!(self, Self::Doc | Self::Dot)
    }
}

/// Why a file could not be opened or saved, for the shells to say in
/// their own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// Not a format a Word document opens from.
    Unsupported,
    /// Protected by a password.
    Encrypted,
    /// A Word older than Word 97.
    TooOld,
    /// The file is damaged: what failed.
    Damaged(String),
    /// The conversion failed: why.
    Failed(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Unsupported => write!(f, "not a format Word opens"),
            FormatError::Encrypted => write!(f, "the document is password-protected"),
            FormatError::TooOld => write!(f, "the document is from a Word older than Word 97"),
            FormatError::Damaged(what) => write!(f, "the document is damaged: {what}"),
            FormatError::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// How a file in another format is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// A text file's encoding, or `None` to detect it.
    pub encoding: Option<TextEncoding>,
}

/// A file opened as a `.docx` for the editor.
#[derive(Clone, Debug)]
pub struct Opened {
    pub docx: Vec<u8>,
    /// What the file was.
    pub format: WordFormat,
    /// A text file's encoding, as read.
    pub encoding: Option<TextEncoding>,
    /// What repairing a damaged document did, when it was damaged.
    pub repairs: Vec<super::repair::RepairNote>,
}

/// Reads `bytes`, a file named `name`, as a `.docx`. A package is handed
/// back as it is; everything else is converted. `resolve` gives the bytes
/// of a picture an HTML page names by a relative path.
pub fn open_as_docx(
    name: &str,
    bytes: &[u8],
    options: OpenOptions,
    resolve: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<Opened, FormatError> {
    // What the bytes are wins over what the name says, as in Word.
    let named = WordFormat::of_name(name);
    let format = match (WordFormat::sniff(bytes), named) {
        (Some(sniffed), _) => sniffed,
        (None, Some(named)) => named,
        (None, None) => return Err(FormatError::Unsupported),
    };
    // A text file is whatever its name says, even when it looks like HTML.
    let format = if named == Some(WordFormat::Text) && matches!(format, WordFormat::Html) {
        WordFormat::Text
    } else {
        format
    };
    let mut encoding = None;
    let flow: FlowDocument = match format {
        _ if format.is_package() => {
            // A damaged document is repaired as it opens, and says so.
            if super::repair::needs_repair(bytes) {
                if let Ok(repaired) = super::repair::repair(bytes) {
                    return Ok(Opened {
                        docx: repaired.docx,
                        format,
                        encoding: None,
                        repairs: repaired.notes,
                    });
                }
            }
            return Ok(Opened {
                docx: bytes.to_vec(),
                format,
                encoding: None,
                repairs: Vec::new(),
            });
        }
        WordFormat::Doc | WordFormat::Dot => {
            jade_doc_reader::read_doc(bytes).map_err(|error| match error {
                jade_doc_reader::DocError::NotWord => FormatError::Unsupported,
                jade_doc_reader::DocError::Encrypted => FormatError::Encrypted,
                jade_doc_reader::DocError::TooOld => FormatError::TooOld,
                jade_doc_reader::DocError::Damaged(what) => FormatError::Damaged(what),
            })?
        }
        WordFormat::Rtf => rtf::read_rtf(bytes).map_err(FormatError::Damaged)?,
        WordFormat::Odt => odt::read_odt(bytes).map_err(FormatError::Damaged)?,
        WordFormat::Html => html::read_html(bytes, resolve),
        WordFormat::Text => {
            let chosen = options
                .encoding
                .unwrap_or_else(|| text::detect_encoding(bytes));
            encoding = Some(chosen);
            text::read_text(bytes, Some(chosen))
        }
        WordFormat::Pdf => read_pdf(bytes)?,
        WordFormat::Docx | WordFormat::Docm | WordFormat::Dotx | WordFormat::Dotm => unreachable!(),
    };
    let docx = jade_docx_convert::write_docx(&flow).map_err(FormatError::Failed)?;
    Ok(Opened {
        docx,
        format,
        encoding,
        repairs: Vec::new(),
    })
}

/// A PDF as an editable document (natively: the browser build has no PDF
/// reader).
fn read_pdf(_bytes: &[u8]) -> Result<FlowDocument, FormatError> {
    Err(FormatError::Unsupported)
}

/// How a file in another format is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveOptions {
    /// A text file's encoding; UTF-8 when `None`.
    pub encoding: Option<TextEncoding>,
    pub line_ending: LineEnding,
}

/// The editor's `.docx` bytes as a file in `format`. PDF is the shells'
/// own: they print the pages.
pub fn save_from_docx(
    format: WordFormat,
    docx: &[u8],
    options: SaveOptions,
) -> Result<Vec<u8>, FormatError> {
    match format {
        WordFormat::Docx | WordFormat::Docm => {
            document_from_template_types(docx, format == WordFormat::Docm)
        }
        WordFormat::Dotx | WordFormat::Dotm => {
            template_from_document(docx, format == WordFormat::Dotm)
        }
        WordFormat::Doc | WordFormat::Dot => {
            let flow = jade_docx_convert::read_docx(docx).map_err(FormatError::Failed)?;
            if format == WordFormat::Dot {
                jade_doc_writer::write_dot(&flow)
            } else {
                jade_doc_writer::write_doc(&flow)
            }
            .map_err(FormatError::Failed)
        }
        WordFormat::Pdf => Err(FormatError::Failed(
            "a PDF is printed, not converted".into(),
        )),
        WordFormat::Rtf | WordFormat::Text | WordFormat::Odt | WordFormat::Html => {
            let flow = jade_docx_convert::read_docx(docx).map_err(FormatError::Failed)?;
            Ok(match format {
                WordFormat::Rtf => rtf::write_rtf(&flow).into_bytes(),
                WordFormat::Odt => odt::write_odt(&flow).map_err(FormatError::Failed)?,
                WordFormat::Html => html::write_html(&flow).into_bytes(),
                _ => {
                    let written = text::write_text(
                        &flow,
                        TextOptions {
                            line_ending: options.line_ending,
                            include_hidden: false,
                        },
                    );
                    options
                        .encoding
                        .unwrap_or(TextEncoding::Utf8)
                        .encode(&written)
                        .0
                }
            })
        }
    }
}

/// The characters of `docx`'s text `encoding` cannot carry: what Word's
/// ファイルの変換 dialog warns will be lost, in red.
pub fn unencodable_characters(docx: &[u8], encoding: TextEncoding) -> Vec<char> {
    let Ok(flow) = jade_docx_convert::read_docx(docx) else {
        return Vec::new();
    };
    encoding.unencodable(&text::write_text(&flow, TextOptions::default()))
}

/// The text of a text file as `encoding` reads it, for the dialog's
/// preview: the first lines only.
pub fn text_preview(bytes: &[u8], encoding: TextEncoding, lines: usize) -> String {
    encoding
        .decode(&bytes[..bytes.len().min(16 * 1024)])
        .lines()
        .take(lines)
        .collect::<Vec<_>>()
        .join("\n")
}

const TEMPLATE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
const DOCUMENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const MACRO_TEMPLATE: &str = "application/vnd.ms-word.template.macroEnabledTemplate.main+xml";
const MACRO_DOCUMENT: &str = "application/vnd.ms-word.document.macroEnabled.main+xml";

fn retype(docx: &[u8], from: &[&str], to: &str) -> Result<Vec<u8>, FormatError> {
    let mut package =
        OpcPackage::open(docx).map_err(|error| FormatError::Failed(error.to_string()))?;
    let types = package
        .part_str("[Content_Types].xml")
        .map_err(|error| FormatError::Failed(error.to_string()))?
        .ok_or_else(|| FormatError::Failed("the document has no content types".into()))?;
    let mut declared = types.clone();
    for kind in from {
        declared = declared.replace(kind, to);
    }
    if declared == types {
        return Ok(docx.to_vec());
    }
    package.replace("[Content_Types].xml", declared.into_bytes());
    package
        .write()
        .map_err(|error| FormatError::Failed(error.to_string()))
}

/// `docx` declared a template (`.dotx`, or `.dotm` keeping its macros), as
/// Word's 名前を付けて保存 → Word テンプレート writes it.
pub fn template_from_document(docx: &[u8], macros: bool) -> Result<Vec<u8>, FormatError> {
    if macros {
        retype(docx, &[MACRO_DOCUMENT, DOCUMENT, TEMPLATE], MACRO_TEMPLATE)
    } else {
        retype(docx, &[MACRO_DOCUMENT, MACRO_TEMPLATE, DOCUMENT], TEMPLATE)
    }
}

/// `docx` declared a document again, whatever it was opened as.
fn document_from_template_types(docx: &[u8], macros: bool) -> Result<Vec<u8>, FormatError> {
    if macros {
        retype(docx, &[MACRO_TEMPLATE, TEMPLATE, DOCUMENT], MACRO_DOCUMENT)
    } else {
        retype(docx, &[MACRO_TEMPLATE, TEMPLATE, MACRO_DOCUMENT], DOCUMENT)
    }
}

/// The file name `name` takes saved as `format`: its stem with the
/// format's extension.
pub fn renamed(name: &str, format: WordFormat) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    format!("{stem}.{}", format.extension())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word::new::{blank_document, Blank};

    #[test]
    fn names_and_contents_say_what_a_file_is() {
        assert_eq!(WordFormat::of_name("a.RTF"), Some(WordFormat::Rtf));
        assert_eq!(WordFormat::of_name("a.htm"), Some(WordFormat::Html));
        assert_eq!(WordFormat::of_name("a.xlsx"), None);
        assert_eq!(WordFormat::sniff(b"{\\rtf1 x}"), Some(WordFormat::Rtf));
        assert_eq!(WordFormat::sniff(b"%PDF-1.7"), Some(WordFormat::Pdf));
        assert_eq!(
            WordFormat::sniff(b"<!DOCTYPE html><p>x"),
            Some(WordFormat::Html)
        );
        let blank = blank_document(Blank::Japanese);
        assert_eq!(WordFormat::sniff(&blank), Some(WordFormat::Docx));
        let template = template_from_document(&blank, false).unwrap();
        assert_eq!(WordFormat::sniff(&template), Some(WordFormat::Dotx));
        let macro_template = template_from_document(&blank, true).unwrap();
        assert_eq!(WordFormat::sniff(&macro_template), Some(WordFormat::Dotm));
        // Saving it as a document again declares it one.
        let back = save_from_docx(WordFormat::Docx, &template, SaveOptions::default()).unwrap();
        assert_eq!(WordFormat::sniff(&back), Some(WordFormat::Docx));
        assert_eq!(renamed("report.docx", WordFormat::Rtf), "report.rtf");
    }

    #[test]
    fn rtf_called_doc_opens_as_rtf() {
        let opened = open_as_docx(
            "memo.doc",
            b"{\\rtf1\\ansi hello\\par}",
            OpenOptions::default(),
            &|_| None,
        )
        .unwrap();
        assert_eq!(opened.format, WordFormat::Rtf);
        assert_eq!(WordFormat::sniff(&opened.docx), Some(WordFormat::Docx));
    }

    #[test]
    fn a_shift_jis_text_file_opens_and_saves_back_in_shift_jis() {
        let (bytes, _) = TextEncoding::ShiftJis.encode("一行目\r\n二行目\r\n");
        let opened = open_as_docx("memo.txt", &bytes, OpenOptions::default(), &|_| None).unwrap();
        assert_eq!(opened.encoding, Some(TextEncoding::ShiftJis));
        let saved = save_from_docx(
            WordFormat::Text,
            &opened.docx,
            SaveOptions {
                encoding: Some(TextEncoding::ShiftJis),
                line_ending: LineEnding::CrLf,
            },
        )
        .unwrap();
        assert_eq!(saved, bytes);
        assert_eq!(text_preview(&bytes, TextEncoding::ShiftJis, 1), "一行目");
        assert!(unencodable_characters(&opened.docx, TextEncoding::Windows1252).contains(&'一'));
    }

    /// Word 97-2004 文書 and テンプレート are written as Word 97–2003 binary
    /// files, which open again with what was written.
    #[test]
    fn a_doc_and_a_dot_are_written() {
        let blank = blank_document(Blank::Japanese);
        let mut flow = jade_docx_convert::read_docx(&blank).unwrap();
        flow.sections[0].blocks = vec![jade_docx_convert::flow::Block::Paragraph(
            jade_docx_convert::flow::Paragraph::with_text("互換形式で保存"),
        )];
        let docx = jade_docx_convert::docx_write::write_docx(&flow).unwrap();
        for format in [WordFormat::Doc, WordFormat::Dot] {
            let bytes = save_from_docx(format, &docx, SaveOptions::default()).unwrap();
            assert_eq!(WordFormat::sniff(&bytes), Some(WordFormat::Doc));
            // fDot marks the template.
            let word = {
                let mut file = cfb::CompoundFile::open(std::io::Cursor::new(&bytes)).unwrap();
                let mut stream = file.open_stream("/WordDocument").unwrap();
                let mut word = Vec::new();
                std::io::Read::read_to_end(&mut stream, &mut word).unwrap();
                word
            };
            assert_eq!(word[10] & 1 == 1, format == WordFormat::Dot);
            let name = format!("copy.{}", format.extension());
            let opened = open_as_docx(&name, &bytes, OpenOptions::default(), &|_| None).unwrap();
            let back = jade_docx_convert::read_docx(&opened.docx).unwrap();
            assert_eq!(back.plain_text().trim(), "互換形式で保存");
        }
    }

    #[test]
    fn every_writable_format_reads_back() {
        let blank = blank_document(Blank::Japanese);
        for format in [
            WordFormat::Rtf,
            WordFormat::Odt,
            WordFormat::Html,
            WordFormat::Text,
        ] {
            let saved = save_from_docx(format, &blank, SaveOptions::default()).unwrap();
            let opened = open_as_docx(
                &format!("a.{}", format.extension()),
                &saved,
                OpenOptions::default(),
                &|_| None,
            )
            .unwrap();
            assert_eq!(opened.format, format);
        }
    }
}
