//! Word 97–2003 binary documents (`.doc`), read as [MS-DOC] lays them out.
//!
//! A `.doc` is a compound file. Its `WordDocument` stream starts with the
//! File Information Block (FIB), which says where everything else is: the
//! piece table (CLX) mapping character positions (CPs) to the text's bytes,
//! the formatted-disk-page tables (FKPs) giving each run's character
//! properties (CHPX) and each paragraph's (PAPX) as property modifiers
//! (sprms), the section table (PlcfSed), the header stories (PlcfHdd), the
//! style sheet (STSH), the font table (SttbfFfn) and the list tables
//! (PlfLst, PlfLfo) — most of them in the `0Table` or `1Table` stream.
//! Inline pictures are in the `Data` stream; floating ones in the
//! OfficeArt drawing (DggInfo) with their blips in the blip store.
//!
//! What is read goes into [`jade_docx_convert::flow`]: the text in its
//! paragraphs, character formatting, tables with merged cells, sections
//! with their pages, headers and footers, lists and pictures. Fields read
//! as their results; a `HYPERLINK` field as a link.

use std::collections::HashMap;
use std::io::Read;

use jade_docx_convert::flow::*;

/// Why a `.doc` could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocError {
    /// Not a compound file, or one with no WordDocument stream.
    NotWord,
    /// Encrypted with a password (or obfuscated): Word asks for it.
    Encrypted,
    /// Word 6 or 95, or older: a format before Word 97.
    TooOld,
    /// The file is damaged where it matters.
    Damaged(String),
}

impl std::fmt::Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocError::NotWord => write!(f, "not a Word 97–2003 document"),
            DocError::Encrypted => write!(f, "the document is password-protected"),
            DocError::TooOld => write!(f, "the document is from a Word older than Word 97"),
            DocError::Damaged(what) => write!(f, "the document is damaged: {what}"),
        }
    }
}

impl std::error::Error for DocError {}

const COMPOUND_SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Whether `bytes` are a compound file with a Word document in it — a
/// `.doc`, as opposed to an encrypted `.docx` (which is a compound file
/// too) or an Excel or PowerPoint binary.
pub fn is_doc(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&COMPOUND_SIGNATURE) {
        return false;
    }
    cfb::CompoundFile::open(std::io::Cursor::new(bytes))
        .map(|file| file.is_stream("/WordDocument"))
        .unwrap_or(false)
}

/// Reads a `.doc`.
pub fn read_doc(bytes: &[u8]) -> Result<FlowDocument, DocError> {
    if !bytes.starts_with(&COMPOUND_SIGNATURE) {
        return Err(DocError::NotWord);
    }
    let mut file =
        cfb::CompoundFile::open(std::io::Cursor::new(bytes)).map_err(|_| DocError::NotWord)?;
    let mut stream = |name: &str| -> Option<Vec<u8>> {
        let mut stream = file.open_stream(name).ok()?;
        let mut out = Vec::new();
        stream.read_to_end(&mut out).ok()?;
        Some(out)
    };
    let word = stream("/WordDocument").ok_or(DocError::NotWord)?;
    let fib = Fib::read(&word)?;
    let table_name = if fib.which_table {
        "/1Table"
    } else {
        "/0Table"
    };
    let table = stream(table_name).ok_or_else(|| DocError::Damaged("no table stream".into()))?;
    let data = stream("/Data").unwrap_or_default();
    let reader = DocReader::new(fib, word, table, data)?;
    Ok(reader.document())
}

// ---- bytes -----------------------------------------------------------------

fn u8_at(bytes: &[u8], at: usize) -> u8 {
    bytes.get(at).copied().unwrap_or(0)
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    match bytes.get(at..at + 2) {
        Some(b) => u16::from_le_bytes([b[0], b[1]]),
        None => 0,
    }
}

fn i16_at(bytes: &[u8], at: usize) -> i16 {
    u16_at(bytes, at) as i16
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    match bytes.get(at..at + 4) {
        Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        None => 0,
    }
}

fn i32_at(bytes: &[u8], at: usize) -> i32 {
    u32_at(bytes, at) as i32
}

fn slice(bytes: &[u8], at: usize, length: usize) -> &[u8] {
    let start = at.min(bytes.len());
    let end = at.saturating_add(length).min(bytes.len());
    &bytes[start..end]
}

// ---- the FIB ---------------------------------------------------------------

struct Fib {
    which_table: bool,
    ccp_text: u32,
    ccp_ftn: u32,
    /// (fc, lcb) by index into FibRgFcLcb97.
    pairs: Vec<(u32, u32)>,
}

const FC_STSHF: usize = 1;
const FC_PLCF_SED: usize = 6;
const FC_PLCF_HDD: usize = 11;
const FC_PLCF_BTE_CHPX: usize = 12;
const FC_PLCF_BTE_PAPX: usize = 13;
const FC_STTBF_FFN: usize = 15;
const FC_CLX: usize = 33;
const FC_PLCSPA_MOM: usize = 40;
const FC_DGG_INFO: usize = 50;
const FC_PLF_LST: usize = 73;
const FC_PLF_LFO: usize = 74;

impl Fib {
    fn read(word: &[u8]) -> Result<Self, DocError> {
        if word.len() < 154 {
            return Err(DocError::Damaged("the FIB is cut short".into()));
        }
        let ident = u16_at(word, 0);
        if ident != 0xA5EC {
            return Err(if ident == 0xA5DC {
                DocError::TooOld
            } else {
                DocError::NotWord
            });
        }
        let n_fib = u16_at(word, 2);
        if n_fib < 0x00C1 && n_fib != 0x0065 {
            // 0x0065 is how Word 97 in a few languages labels itself.
            return Err(DocError::TooOld);
        }
        let flags = u16_at(word, 0x0A);
        if flags & 0x0100 != 0 {
            return Err(DocError::Encrypted);
        }
        let which_table = flags & 0x0200 != 0;
        // FibRgW: csw at 32, then csw u16s; FibRgLw: cslw then cslw u32s.
        let csw = u16_at(word, 32) as usize;
        let rg_lw = 34 + csw * 2;
        let cslw = u16_at(word, rg_lw) as usize;
        let lw = rg_lw + 2;
        let ccp_text = u32_at(word, lw + 3 * 4);
        let ccp_ftn = u32_at(word, lw + 4 * 4);
        let fc_lcb = lw + cslw * 4;
        let count = u16_at(word, fc_lcb) as usize;
        let pairs = (0..count)
            .map(|index| {
                let at = fc_lcb + 2 + index * 8;
                (u32_at(word, at), u32_at(word, at + 4))
            })
            .collect();
        Ok(Self {
            which_table,
            ccp_text,
            ccp_ftn,
            pairs,
        })
    }

    fn pair(&self, index: usize) -> (usize, usize) {
        self.pairs
            .get(index)
            .map(|(fc, lcb)| (*fc as usize, *lcb as usize))
            .unwrap_or((0, 0))
    }
}

// ---- sprms -----------------------------------------------------------------

/// Each sprm in a grpprl with its operand.
fn sprms(grpprl: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 2 <= grpprl.len() {
        let sprm = u16_at(grpprl, at);
        at += 2;
        let length = match sprm >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            _ => {
                // Variable: a byte of length, but two for these.
                if sprm == 0xD608 || sprm == 0xD606 {
                    let length = u16_at(grpprl, at) as usize;
                    at += 2;
                    // sprmTDefTable's length counts itself minus one.
                    length.saturating_sub(1)
                } else if sprm == 0xC615 {
                    // sprmPChgTabs: 255 means a longer form.
                    let length = u8_at(grpprl, at) as usize;
                    at += 1;
                    if length == 255 {
                        let deletes = u8_at(grpprl, at) as usize;
                        let adds_at = at + 1 + deletes * 4;
                        let adds = u8_at(grpprl, adds_at) as usize;
                        1 + deletes * 4 + 1 + adds * 3
                    } else {
                        length
                    }
                } else {
                    let length = u8_at(grpprl, at) as usize;
                    at += 1;
                    length
                }
            }
        };
        let end = (at + length).min(grpprl.len());
        out.push((sprm, &grpprl[at..end]));
        at += length;
    }
    out
}

/// Character properties as a run has them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Chp {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    small_caps: bool,
    caps: bool,
    hidden: bool,
    vertical: u8,
    size: u16,
    font: Option<u16>,
    east_asian_font: Option<u16>,
    color: Option<String>,
    highlight: Option<u8>,
    shading: Option<String>,
    special: bool,
    picture_location: Option<u32>,
    data: bool,
    object: bool,
    symbol: Option<(u16, u16)>,
    style: Option<u16>,
}

/// Paragraph properties as a paragraph has them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pap {
    alignment: Option<Alignment>,
    left: i32,
    right: i32,
    first: i32,
    before: Option<u32>,
    after: Option<u32>,
    line: Option<LineSpacing>,
    in_table: bool,
    row_end: bool,
    depth: i32,
    inner_cell: bool,
    inner_row_end: bool,
    list_level: u8,
    list_format: i16,
    keep_with_next: bool,
    page_break_before: bool,
    outline: Option<u8>,
    shading: Option<String>,
    table_definition: Option<Vec<u8>>,
    table_header: bool,
    table_borders: bool,
}

fn toggle(value: u8, base: bool) -> bool {
    match value {
        0 => false,
        1 => true,
        0x80 => base,
        0x81 => !base,
        _ => base,
    }
}

const ICO: [&str; 17] = [
    "", "000000", "0000FF", "00FFFF", "00FF00", "FF00FF", "FF0000", "FFFF00", "FFFFFF", "000080",
    "008080", "008000", "800080", "800000", "808000", "808080", "C0C0C0",
];

fn colorref(operand: &[u8]) -> Option<String> {
    if operand.len() < 4 || operand[3] == 0xFF {
        return None;
    }
    Some(format!(
        "{:02X}{:02X}{:02X}",
        operand[0], operand[1], operand[2]
    ))
}

fn apply_chpx(grpprl: &[u8], chp: &mut Chp, base: &Chp) {
    for (sprm, operand) in sprms(grpprl) {
        let byte = u8_at(operand, 0);
        match sprm {
            0x0835 => chp.bold = toggle(byte, base.bold),
            0x0836 => chp.italic = toggle(byte, base.italic),
            0x0837 | 0x2A53 => chp.strike = toggle(byte, base.strike),
            0x083A => chp.small_caps = toggle(byte, base.small_caps),
            0x083B => chp.caps = toggle(byte, base.caps),
            0x083C => chp.hidden = toggle(byte, base.hidden),
            0x2A3E => chp.underline = byte != 0,
            0x2A42 => {
                chp.color = ICO
                    .get(byte as usize)
                    .filter(|color| !color.is_empty())
                    .map(|color| color.to_string())
            }
            0x6870 => chp.color = colorref(operand),
            0x4A43 => chp.size = u16_at(operand, 0),
            0x2A48 => chp.vertical = byte,
            0x4A4F => chp.font = Some(u16_at(operand, 0)),
            0x4A50 => chp.east_asian_font = Some(u16_at(operand, 0)),
            0x2A0C => chp.highlight = (byte != 0).then_some(byte),
            0xCA71 => {
                // SHDOperand: cb, then cvFore, cvBack, ipat.
                chp.shading = colorref(slice(operand, 4, 4)).filter(|color| color != "FFFFFF");
            }
            0x0855 => chp.special = byte != 0,
            0x6A03 => chp.picture_location = Some(u32_at(operand, 0)),
            0x0806 => chp.data = byte != 0,
            0x0856 => chp.object = byte != 0,
            0x6A09 => chp.symbol = Some((u16_at(operand, 0), u16_at(operand, 2))),
            0x4A30 => chp.style = Some(u16_at(operand, 0)),
            0x2A33 => {
                // sprmCPlain: back to the paragraph style's properties.
                let style = chp.style;
                *chp = base.clone();
                chp.style = style;
            }
            _ => {}
        }
    }
}

fn apply_papx(grpprl: &[u8], pap: &mut Pap) {
    for (sprm, operand) in sprms(grpprl) {
        let byte = u8_at(operand, 0);
        match sprm {
            0x2403 | 0x2461 => {
                pap.alignment = Some(match byte {
                    1 => Alignment::Center,
                    2 => Alignment::Right,
                    3..=5 => Alignment::Justify,
                    _ => Alignment::Left,
                })
            }
            0x840F | 0x845E => pap.left = i16_at(operand, 0) as i32,
            0x840E | 0x845D => pap.right = i16_at(operand, 0) as i32,
            0x8411 | 0x8460 => pap.first = i16_at(operand, 0) as i32,
            0xA413 => pap.before = Some(u16_at(operand, 0) as u32),
            0xA414 => pap.after = Some(u16_at(operand, 0) as u32),
            0x6412 => {
                let line = i16_at(operand, 0);
                let multiple = i16_at(operand, 2) != 0;
                pap.line = Some(if multiple {
                    LineSpacing::Multiple(line.unsigned_abs() as u32)
                } else if line < 0 {
                    LineSpacing::Exact(line.unsigned_abs() as u32)
                } else {
                    LineSpacing::AtLeast(line as u32)
                });
            }
            0x2416 => pap.in_table = byte != 0,
            0x2417 => pap.row_end = byte != 0,
            0x6649 => pap.depth = i32_at(operand, 0),
            0x244B => pap.inner_cell = byte != 0,
            0x244C => pap.inner_row_end = byte != 0,
            0x260A => pap.list_level = byte.min(8),
            0x460B => pap.list_format = i16_at(operand, 0),
            0x2406 => pap.keep_with_next = byte != 0,
            0x2407 => pap.page_break_before = byte != 0,
            0x2640 => pap.outline = (byte < 9).then_some(byte),
            0xC64D => {
                pap.shading = colorref(slice(operand, 5, 4)).filter(|color| color != "FFFFFF");
            }
            0xD608 => pap.table_definition = Some(operand.to_vec()),
            0x3404 => pap.table_header = byte != 0,
            0xD605 | 0xD613 => {
                pap.table_borders = operand.iter().any(|byte| *byte != 0 && *byte != 0xFF)
            }
            _ => {}
        }
    }
}

// ---- styles, fonts, lists --------------------------------------------------

#[derive(Clone, Debug, Default)]
struct Style {
    name: String,
    sti: u16,
    base: Option<u16>,
    papx: Vec<u8>,
    chpx: Vec<u8>,
}

fn read_styles(table: &[u8], (fc, lcb): (usize, usize)) -> (Vec<Option<Style>>, [u16; 3]) {
    let stsh = slice(table, fc, lcb);
    let mut fonts = [0u16; 3];
    if stsh.len() < 4 {
        return (Vec::new(), fonts);
    }
    let cb_stshi = u16_at(stsh, 0) as usize;
    let stshi = slice(stsh, 2, cb_stshi);
    let count = u16_at(stshi, 0) as usize;
    let base_size = u16_at(stshi, 2) as usize;
    for (index, font) in fonts.iter_mut().enumerate() {
        *font = u16_at(stshi, 12 + index * 2);
    }
    let mut styles = Vec::with_capacity(count);
    let mut at = 2 + cb_stshi;
    for _ in 0..count {
        let cb_std = u16_at(stsh, at) as usize;
        at += 2;
        if cb_std == 0 {
            styles.push(None);
            continue;
        }
        let std = slice(stsh, at, cb_std);
        at += cb_std;
        let sti = u16_at(std, 0) & 0x0FFF;
        let stk = u16_at(std, 2) & 0x000F;
        let base = u16_at(std, 2) >> 4;
        let mut cursor = base_size.max(10);
        // xstzName: a count of UTF-16 units, the units, a terminating zero.
        let length = u16_at(std, cursor) as usize;
        cursor += 2;
        let name: String =
            char::decode_utf16((0..length).map(|index| u16_at(std, cursor + index * 2)))
                .map(|ch| ch.unwrap_or('\u{fffd}'))
                .collect();
        cursor += length * 2 + 2;
        let upx = |cursor: &mut usize| -> Vec<u8> {
            let size = u16_at(std, *cursor) as usize;
            *cursor += 2;
            let data = slice(std, *cursor, size).to_vec();
            *cursor += size + (size & 1);
            data
        };
        let mut style = Style {
            name,
            sti,
            base: (base != 0x0FFF).then_some(base),
            ..Style::default()
        };
        match stk {
            1 => {
                let papx = upx(&mut cursor);
                // The UpxPapx starts with the istd.
                style.papx = papx.get(2..).map(<[u8]>::to_vec).unwrap_or_default();
                style.chpx = upx(&mut cursor);
            }
            2 => style.chpx = upx(&mut cursor),
            _ => {}
        }
        styles.push(Some(style));
    }
    (styles, fonts)
}

fn read_fonts(table: &[u8], (fc, lcb): (usize, usize)) -> Vec<String> {
    let sttb = slice(table, fc, lcb);
    let mut out = Vec::new();
    if sttb.len() < 4 {
        return out;
    }
    let count = u16_at(sttb, 0) as usize;
    let mut at = 4;
    for _ in 0..count {
        let length = u8_at(sttb, at) as usize;
        let ffn = slice(sttb, at + 1, length);
        at += 1 + length;
        let units: Vec<u16> = (39..ffn.len())
            .step_by(2)
            .map(|offset| u16_at(ffn, offset))
            .take_while(|unit| *unit != 0)
            .collect();
        out.push(String::from_utf16_lossy(&units));
    }
    out
}

/// Each list's levels' number formats by its lsid, and each list format
/// override's lsid, 1-based as paragraphs name them.
fn read_lists(table: &[u8], fib: &Fib) -> (HashMap<i32, Vec<u8>>, Vec<i32>) {
    let mut lists = HashMap::new();
    let (fc, lcb) = fib.pair(FC_PLF_LST);
    let plf = slice(table, fc, lcb);
    if plf.len() >= 2 {
        let count = i16_at(plf, 0).max(0) as usize;
        let mut lstfs = Vec::new();
        for index in 0..count {
            let at = 2 + index * 28;
            let lsid = i32_at(plf, at);
            let simple = u8_at(plf, at + 26) & 1 != 0;
            lstfs.push((lsid, if simple { 1 } else { 9 }));
        }
        // The levels follow the LSTFs, one list after another.
        let mut at = 2 + count * 28;
        for (lsid, levels) in lstfs {
            let mut formats = Vec::new();
            for _ in 0..levels {
                let lvlf = slice(plf, at, 28);
                if lvlf.len() < 28 {
                    break;
                }
                formats.push(u8_at(lvlf, 4));
                let chpx = u8_at(lvlf, 24) as usize;
                let papx = u8_at(lvlf, 25) as usize;
                at += 28 + papx + chpx;
                let text = u16_at(plf, at) as usize;
                at += 2 + text * 2;
            }
            lists.insert(lsid, formats);
        }
    }
    let (fc, lcb) = fib.pair(FC_PLF_LFO);
    let plf = slice(table, fc, lcb);
    let mut overrides = Vec::new();
    if plf.len() >= 4 {
        let count = i32_at(plf, 0).max(0) as usize;
        for index in 0..count.min(plf.len() / 16) {
            overrides.push(i32_at(plf, 4 + index * 16));
        }
    }
    (lists, overrides)
}

// ---- the reader ------------------------------------------------------------

struct Piece {
    cp_start: u32,
    cp_end: u32,
    fc: u32,
    compressed: bool,
}

struct DocReader {
    fib: Fib,
    word: Vec<u8>,
    table: Vec<u8>,
    data: Vec<u8>,
    /// Every character of every story, with the FC it is stored at.
    chars: Vec<(u16, u32)>,
    chpx: Vec<(u32, u32, Vec<u8>)>,
    papx: Vec<(u32, u32, u16, Vec<u8>)>,
    styles: Vec<Option<Style>>,
    resolved: HashMap<u16, (Pap, Chp)>,
    fonts: Vec<String>,
    default_fonts: [u16; 3],
    lists: HashMap<i32, Vec<u8>>,
    overrides: Vec<i32>,
    floating: HashMap<u32, Image>,
}

impl DocReader {
    fn new(fib: Fib, word: Vec<u8>, table: Vec<u8>, data: Vec<u8>) -> Result<Self, DocError> {
        let pieces = read_pieces(&table, fib.pair(FC_CLX))?;
        let mut chars = Vec::new();
        for piece in &pieces {
            for index in 0..(piece.cp_end.saturating_sub(piece.cp_start)) {
                if piece.compressed {
                    let fc = piece.fc / 2 + index;
                    let byte = u8_at(&word, fc as usize);
                    chars.push((windows_1252(byte), fc));
                } else {
                    let fc = piece.fc + index * 2;
                    chars.push((u16_at(&word, fc as usize), fc));
                }
            }
        }
        let chpx = read_chpx(&word, &table, fib.pair(FC_PLCF_BTE_CHPX));
        let papx = read_papx(&word, &table, fib.pair(FC_PLCF_BTE_PAPX));
        let (styles, default_fonts) = read_styles(&table, fib.pair(FC_STSHF));
        let fonts = read_fonts(&table, fib.pair(FC_STTBF_FFN));
        let (lists, overrides) = read_lists(&table, &fib);
        let mut reader = Self {
            fib,
            word,
            table,
            data,
            chars,
            chpx,
            papx,
            styles,
            resolved: HashMap::new(),
            fonts,
            default_fonts,
            lists,
            overrides,
            floating: HashMap::new(),
        };
        reader.floating = reader.floating_pictures();
        Ok(reader)
    }

    fn default_chp(&self) -> Chp {
        Chp {
            size: 20,
            font: Some(self.default_fonts[0]),
            east_asian_font: Some(self.default_fonts[1]),
            ..Chp::default()
        }
    }

    /// The paragraph and character properties style `istd` gives, its
    /// bases' first.
    fn style(&mut self, istd: u16) -> (Pap, Chp) {
        if let Some(resolved) = self.resolved.get(&istd) {
            return resolved.clone();
        }
        let mut chain = Vec::new();
        let mut next = Some(istd);
        while let Some(id) = next {
            if chain.contains(&id) || chain.len() > 12 {
                break;
            }
            let Some(Some(style)) = self.styles.get(id as usize) else {
                break;
            };
            chain.push(id);
            next = style.base;
        }
        let mut pap = Pap::default();
        let mut chp = self.default_chp();
        for id in chain.iter().rev() {
            if let Some(Some(style)) = self.styles.get(*id as usize) {
                let base = chp.clone();
                apply_papx(&style.papx, &mut pap);
                apply_chpx(&style.chpx, &mut chp, &base);
            }
        }
        self.resolved.insert(istd, (pap.clone(), chp.clone()));
        (pap, chp)
    }

    fn papx_at(&self, fc: u32) -> Option<&(u32, u32, u16, Vec<u8>)> {
        let index = self.papx.partition_point(|(start, _, _, _)| *start <= fc);
        let entry = self.papx.get(index.checked_sub(1)?)?;
        (fc < entry.1).then_some(entry)
    }

    fn chpx_at(&self, fc: u32) -> Option<&[u8]> {
        let index = self.chpx.partition_point(|(start, _, _)| *start <= fc);
        let entry = self.chpx.get(index.checked_sub(1)?)?;
        (fc < entry.1).then_some(entry.2.as_slice())
    }

    fn paragraph_properties(&mut self, fc: u32) -> (Pap, Chp, String, u16) {
        let (istd, grpprl) = self
            .papx_at(fc)
            .map(|(_, _, istd, grpprl)| (*istd, grpprl.clone()))
            .unwrap_or((0, Vec::new()));
        let (mut pap, chp) = self.style(istd);
        apply_papx(&grpprl, &mut pap);
        let name = self
            .styles
            .get(istd as usize)
            .and_then(|style| style.as_ref())
            .map(|style| style.name.clone())
            .unwrap_or_default();
        let sti = self
            .styles
            .get(istd as usize)
            .and_then(|style| style.as_ref())
            .map(|style| style.sti)
            .unwrap_or(0);
        (pap, chp, name, sti)
    }

    fn char_properties(&mut self, fc: u32, paragraph: &Chp) -> Chp {
        let grpprl = self.chpx_at(fc).map(<[u8]>::to_vec).unwrap_or_default();
        let mut chp = paragraph.clone();
        // A character style goes under the run's own formatting.
        if let Some(style) = sprms(&grpprl)
            .iter()
            .find(|(sprm, _)| *sprm == 0x4A30)
            .map(|(_, operand)| u16_at(operand, 0))
        {
            if let Some(Some(character)) = self.styles.get(style as usize).cloned() {
                let base = chp.clone();
                apply_chpx(&character.chpx, &mut chp, &base);
            }
        }
        let base = chp.clone();
        apply_chpx(&grpprl, &mut chp, &base);
        chp
    }

    fn font_name(&self, index: Option<u16>) -> Option<String> {
        self.fonts
            .get(index? as usize)
            .filter(|name| !name.is_empty())
            .cloned()
    }

    fn format(&self, chp: &Chp, link: Option<&str>) -> CharFormat {
        let default = self.default_chp();
        let font = self.font_name(chp.font);
        let default_font = self.font_name(default.font);
        let east_asian = self.font_name(chp.east_asian_font);
        let default_east_asian = self.font_name(default.east_asian_font);
        CharFormat {
            bold: chp.bold,
            italic: chp.italic,
            underline: chp.underline,
            strike: chp.strike,
            superscript: chp.vertical == 1,
            subscript: chp.vertical == 2,
            small_caps: chp.small_caps,
            all_caps: chp.caps,
            hidden: chp.hidden,
            size: (chp.size != 0).then_some(chp.size as u32),
            font: font.filter(|font| Some(font) != default_font.as_ref()),
            east_asian_font: east_asian.filter(|font| Some(font) != default_east_asian.as_ref()),
            color: chp.color.clone(),
            highlight: chp
                .highlight
                .and_then(|ico| ICO.get(ico as usize))
                .and_then(|color| highlight_named(color))
                .map(str::to_string),
            shading: chp.shading.clone(),
            link: link.map(str::to_string),
        }
    }

    fn document(mut self) -> FlowDocument {
        let main_end = self.fib.ccp_text.min(self.chars.len() as u32);
        let sections = read_sections(&self.table, self.fib.pair(FC_PLCF_SED), &self.word);
        let header_start = self.fib.ccp_text + self.fib.ccp_ftn;
        let headers = read_header_ranges(&self.table, self.fib.pair(FC_PLCF_HDD), header_start);
        let mut document = FlowDocument {
            default_font: self.font_name(Some(self.default_fonts[0])),
            default_size: Some(20),
            ..FlowDocument::default()
        };
        if self
            .chars
            .iter()
            .take(main_end as usize)
            .any(|(ch, _)| matches!(*ch as u32, 0x3040..=0x30ff | 0x4e00..=0x9fff))
        {
            document.language = Some("ja-JP".into());
            document.default_font = self
                .font_name(Some(self.default_fonts[1]))
                .or(document.default_font);
        }
        let boundaries: Vec<u32> = sections.iter().map(|(start, _, _)| *start).collect();
        let sections_info = if sections.is_empty() {
            vec![(0, main_end, PageSetup::letter())]
        } else {
            sections
        };
        let mut previous_header: Vec<Block> = Vec::new();
        let mut previous_footer: Vec<Block> = Vec::new();
        for (index, (start, end, page)) in sections_info.iter().enumerate() {
            let end = (*end).min(main_end);
            let blocks = self.story(*start, end, &boundaries);
            let story = |slot: usize| headers.get(6 + index * 6 + slot).copied();
            let header = match story(1) {
                Some((start, end)) if end > start => {
                    let blocks = self.story(start, end, &[]);
                    previous_header = blocks.clone();
                    blocks
                }
                _ => previous_header.clone(),
            };
            let footer = match story(3) {
                Some((start, end)) if end > start => {
                    let blocks = self.story(start, end, &[]);
                    previous_footer = blocks.clone();
                    blocks
                }
                _ => previous_footer.clone(),
            };
            document.sections.push(FlowSection {
                page: *page,
                header: trim_story(header),
                footer: trim_story(footer),
                blocks,
            });
        }
        if document.sections.is_empty() {
            document.sections.push(FlowSection::default());
        }
        document
    }

    /// The blocks of the text from `start` to `end`.
    fn story(&mut self, start: u32, end: u32, section_ends: &[u32]) -> Vec<Block> {
        let mut blocks: Vec<Block> = Vec::new();
        let mut paragraph = Paragraph::default();
        let mut table: Option<TableBuilder> = None;
        let mut fields: Vec<Field> = Vec::new();
        let mut paragraph_chp: Option<(u32, Chp)> = None;
        let end = end.min(self.chars.len() as u32);
        let mut cp = start;
        while cp < end {
            let (unit, fc) = self.chars[cp as usize];
            // The paragraph a character is in gives the style its run
            // starts from: found at the paragraph's mark.
            if paragraph_chp.is_none() {
                let mark = self.paragraph_mark_fc(cp, end);
                let (_, chp, _, _) = self.paragraph_properties(mark);
                paragraph_chp = Some((mark, chp));
            }
            let base = paragraph_chp
                .as_ref()
                .map(|(_, chp)| chp.clone())
                .unwrap_or_default();
            let chp = self.char_properties(fc, &base);
            let link = fields.iter().rev().find_map(|field| field.link.clone());
            let in_instruction = fields.iter().any(|field| !field.in_result);
            match unit {
                0x13 => {
                    fields.push(Field::default());
                }
                0x14 if !fields.is_empty() => {
                    if let Some(field) = fields.last_mut() {
                        field.in_result = true;
                        field.link = hyperlink_target(&field.instruction);
                    }
                }
                0x15 if !fields.is_empty() => {
                    fields.pop();
                }
                _ if in_instruction => {
                    if let Some(field) = fields.iter_mut().rev().find(|field| !field.in_result) {
                        if let Some(ch) = char::from_u32(unit as u32) {
                            field.instruction.push(ch);
                        }
                    }
                }
                0x0D | 0x07 | 0x0C
                    if unit != 0x0C || section_ends.contains(&(cp + 1)) || cp + 1 == end =>
                {
                    let (pap, _, name, sti) = self.paragraph_properties(fc);
                    let mut finished = std::mem::take(&mut paragraph);
                    self.finish_paragraph(&mut finished, &pap, &name, sti);
                    paragraph_chp = None;
                    let cell_end = unit == 0x07;
                    if pap.in_table || pap.row_end || cell_end && table.is_some() {
                        let builder = table.get_or_insert_with(TableBuilder::default);
                        if pap.row_end && (cell_end || pap.depth <= 1) {
                            builder.end_row(&pap);
                        } else if pap.inner_row_end {
                            // A nested table's row: its text is already in the cell.
                        } else {
                            builder.cell.push(Block::Paragraph(finished));
                            if cell_end && !pap.inner_cell {
                                builder.end_cell();
                            }
                        }
                    } else {
                        if let Some(builder) = table.take() {
                            if let Some(table) = builder.build() {
                                blocks.push(Block::Table(table));
                            }
                        }
                        blocks.push(Block::Paragraph(finished));
                    }
                }
                0x0C => paragraph.inlines.push(Inline::PageBreak),
                0x0B | 0x0E => paragraph.inlines.push(Inline::LineBreak),
                0x09 => paragraph.inlines.push(Inline::Tab),
                0x01 if chp.special => {
                    if let Some(image) = chp
                        .picture_location
                        .and_then(|location| self.inline_picture(location))
                    {
                        paragraph.inlines.push(Inline::Image(image));
                    }
                }
                0x08 if chp.special => {
                    if let Some(image) = self.floating.get(&cp).cloned() {
                        paragraph.inlines.push(Inline::Image(image));
                    }
                }
                0x1E => paragraph.push_text("\u{2011}", &self.format(&chp, link.as_deref())),
                0x00..=0x1F => {}
                _ => {
                    let text = match chp.symbol {
                        Some((_, code)) if chp.special => {
                            let code = if code >= 0xF000 { code - 0xF000 } else { code };
                            char::from_u32(code as u32)
                                .map(String::from)
                                .unwrap_or_default()
                        }
                        _ => char::from_u32(unit as u32)
                            .map(String::from)
                            .unwrap_or_else(|| {
                                // A surrogate pair: decode it with the next unit.
                                let next = self.chars.get(cp as usize + 1).map(|(unit, _)| *unit);
                                match next {
                                    Some(low) if (0xDC00..0xE000).contains(&low) => {
                                        cp += 1;
                                        String::from_utf16_lossy(&[unit, low])
                                    }
                                    _ => String::new(),
                                }
                            }),
                    };
                    let format = self.format(&chp, link.as_deref());
                    paragraph.push_text(&text, &format);
                }
            }
            cp += 1;
        }
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(paragraph));
        }
        if let Some(builder) = table.take() {
            if let Some(table) = builder.build() {
                blocks.push(Block::Table(table));
            }
        }
        blocks
    }

    /// The FC of the mark ending the paragraph `cp` is in.
    fn paragraph_mark_fc(&self, cp: u32, end: u32) -> u32 {
        let mut at = cp;
        while at < end {
            let (unit, fc) = self.chars[at as usize];
            if matches!(unit, 0x0D | 0x07) {
                return fc;
            }
            at += 1;
        }
        self.chars
            .get(end.saturating_sub(1) as usize)
            .map(|(_, fc)| *fc)
            .unwrap_or(0)
    }

    fn finish_paragraph(&self, paragraph: &mut Paragraph, pap: &Pap, name: &str, sti: u16) {
        paragraph.kind = match sti {
            1..=9 => ParagraphKind::Heading(sti as u8),
            62 => ParagraphKind::Title,
            _ => {
                let lower = name.to_ascii_lowercase();
                if let Some(level) = lower
                    .strip_prefix("heading ")
                    .and_then(|l| l.trim().parse::<u8>().ok())
                {
                    ParagraphKind::Heading(level.clamp(1, 9))
                } else if let Some(level) = name
                    .strip_prefix("見出し")
                    .and_then(|l| l.trim().parse::<u8>().ok())
                {
                    ParagraphKind::Heading(level.clamp(1, 9))
                } else {
                    match pap.outline {
                        Some(level) => ParagraphKind::Heading(level + 1),
                        None => ParagraphKind::Normal,
                    }
                }
            }
        };
        paragraph.alignment = pap.alignment;
        paragraph.indent_left = pap.left;
        paragraph.indent_right = pap.right;
        paragraph.indent_first_line = pap.first;
        paragraph.space_before = pap.before;
        paragraph.space_after = pap.after;
        paragraph.line_spacing = pap.line;
        paragraph.keep_with_next = pap.keep_with_next;
        paragraph.page_break_before = pap.page_break_before;
        paragraph.shading = pap.shading.clone();
        if pap.list_format > 0 {
            let lsid = self.overrides.get(pap.list_format as usize - 1).copied();
            let format = lsid
                .and_then(|lsid| self.lists.get(&lsid))
                .and_then(|levels| levels.get(pap.list_level as usize).or(levels.first()))
                .copied();
            if format != Some(255) {
                paragraph.list = Some(ListItem {
                    ordered: format != Some(23),
                    level: pap.list_level,
                });
                paragraph.indent_left = 0;
                paragraph.indent_first_line = 0;
            }
        }
    }

    /// The picture whose PICF is at `location` in the Data stream.
    fn inline_picture(&self, location: u32) -> Option<Image> {
        let at = location as usize;
        let total = u32_at(&self.data, at) as usize;
        let header = u16_at(&self.data, at + 4) as usize;
        if total < header || header < 0x44 {
            return None;
        }
        let picf = slice(&self.data, at, total);
        let mm = u16_at(picf, 6);
        let goal_width = i16_at(picf, 28).max(0) as u64;
        let goal_height = i16_at(picf, 30).max(0) as u64;
        let scale_x = u16_at(picf, 32).max(1) as u64;
        let scale_y = u16_at(picf, 34).max(1) as u64;
        let mut start = header;
        if mm == 0x66 {
            start += 1 + u8_at(picf, header) as usize;
        }
        let mut image = blip_in(slice(picf, start, picf.len()))?;
        if goal_width > 0 && goal_height > 0 {
            image.width = goal_width * scale_x / 1000 * EMU_PER_TWIP;
            image.height = goal_height * scale_y / 1000 * EMU_PER_TWIP;
        }
        Some(image)
    }

    /// The floating pictures, by the CP of the character anchoring each:
    /// the shapes' placements (PlcSpaMom) matched to their blips through the
    /// drawing's shape properties and the blip store.
    fn floating_pictures(&self) -> HashMap<u32, Image> {
        let mut out = HashMap::new();
        let (fc, lcb) = self.fib.pair(FC_PLCSPA_MOM);
        let plc = slice(&self.table, fc, lcb);
        if plc.len() < 4 {
            return out;
        }
        // (n + 1) CPs then n FSPAs of 26 bytes.
        let count = (plc.len() - 4) / 30;
        let (fc, lcb) = self.fib.pair(FC_DGG_INFO);
        let dgg = slice(&self.table, fc, lcb);
        let blips = blip_store(dgg, &self.word);
        let shapes = shape_blips(dgg);
        for index in 0..count {
            let cp = u32_at(plc, index * 4);
            let fspa = slice(plc, (count + 1) * 4 + index * 26, 26);
            let spid = u32_at(fspa, 0);
            let width = (i32_at(fspa, 12) - i32_at(fspa, 4)).max(0) as u64;
            let height = (i32_at(fspa, 16) - i32_at(fspa, 8)).max(0) as u64;
            let Some(pib) = shapes.get(&spid) else {
                continue;
            };
            let Some(Some(image)) = blips.get((*pib as usize).wrapping_sub(1)) else {
                continue;
            };
            let mut image = image.clone();
            if width > 0 && height > 0 {
                image.width = width * EMU_PER_TWIP;
                image.height = height * EMU_PER_TWIP;
            }
            out.insert(cp, image);
        }
        out
    }
}

#[derive(Default)]
struct Field {
    instruction: String,
    in_result: bool,
    link: Option<String>,
}

fn hyperlink_target(instruction: &str) -> Option<String> {
    let rest = instruction.trim().strip_prefix("HYPERLINK")?.trim();
    if let Some(anchor) = rest.strip_prefix("\\l") {
        let anchor = anchor.trim().trim_matches('"');
        return Some(format!("#{anchor}"));
    }
    let target = if let Some(quoted) = rest.strip_prefix('"') {
        quoted.split('"').next().unwrap_or("")
    } else {
        rest.split_whitespace().next().unwrap_or("")
    };
    (!target.is_empty()).then(|| target.to_string())
}

fn trim_story(mut blocks: Vec<Block>) -> Vec<Block> {
    // A header story ends with a guard paragraph mark of its own.
    while matches!(blocks.last(), Some(Block::Paragraph(paragraph)) if paragraph.is_empty())
        && blocks.len() > 1
    {
        blocks.pop();
    }
    if matches!(blocks.as_slice(), [Block::Paragraph(paragraph)] if paragraph.is_empty()) {
        blocks.clear();
    }
    blocks
}

#[derive(Default)]
struct TableBuilder {
    rows: Vec<Row>,
    row_cells: Vec<Vec<Block>>,
    cell: Vec<Block>,
    boundaries: Vec<i32>,
    borders: bool,
    /// Each cell's (row, index, left, right).
    edges: Vec<(usize, usize, i32, i32)>,
}

impl TableBuilder {
    fn end_cell(&mut self) {
        let cell = std::mem::take(&mut self.cell);
        self.row_cells.push(cell);
    }

    fn end_row(&mut self, pap: &Pap) {
        if !self.cell.is_empty() {
            self.end_cell();
        }
        let cells = std::mem::take(&mut self.row_cells);
        let mut row = Row {
            cells: Vec::new(),
            header: pap.table_header,
        };
        let definition = pap.table_definition.clone().unwrap_or_default();
        let count = u8_at(&definition, 0) as usize;
        let centers: Vec<i32> = (0..=count)
            .map(|index| i16_at(&definition, 1 + index * 2) as i32)
            .collect();
        let tcs_at = 1 + (count + 1) * 2;
        if pap.table_borders {
            self.borders = true;
        }
        for (index, blocks) in cells.into_iter().enumerate() {
            let tc = slice(&definition, tcs_at + index * 20, 20);
            let flags = u16_at(tc, 0);
            // Any border in the cell's TC80 rules the table.
            if tc.len() >= 20
                && (4..20)
                    .step_by(4)
                    .any(|at| u8_at(tc, at + 1) != 0 && u8_at(tc, at + 1) != 0xFF)
            {
                self.borders = true;
            }
            let merged_across = flags & 0x0002 != 0;
            let vertical_merge = if flags & 0x0040 != 0 {
                VerticalMerge::Restart
            } else if flags & 0x0020 != 0 {
                VerticalMerge::Continue
            } else {
                VerticalMerge::None
            };
            let (left, right) = (
                centers.get(index).copied().unwrap_or(0),
                centers.get(index + 1).copied().unwrap_or(0),
            );
            if merged_across {
                if let Some(last) = row.cells.last_mut() {
                    last.span += 1;
                    continue;
                }
            }
            if right > left {
                for boundary in [left, right] {
                    if !self.boundaries.contains(&boundary) {
                        self.boundaries.push(boundary);
                    }
                }
            }
            row.cells.push(Cell {
                blocks,
                span: 1,
                vertical_merge,
                shading: None,
            });
            // The cell's edges give its span once the grid is known.
            self.edges
                .push((self.rows.len(), row.cells.len() - 1, left, right));
        }
        self.rows.push(row);
    }

    fn build(mut self) -> Option<Table> {
        if !self.row_cells.is_empty() || !self.cell.is_empty() {
            self.end_row(&Pap::default());
        }
        let edges = std::mem::take(&mut self.edges);
        if self.rows.is_empty() {
            return None;
        }
        self.boundaries.sort_unstable();
        let mut columns = Vec::new();
        for pair in self.boundaries.windows(2) {
            columns.push((pair[1] - pair[0]).max(1) as u32);
        }
        for (row, cell, left, right) in edges {
            let span = self
                .boundaries
                .iter()
                .filter(|boundary| **boundary > left && **boundary <= right)
                .count()
                .max(1) as u32;
            if let Some(cell) = self
                .rows
                .get_mut(row)
                .and_then(|row| row.cells.get_mut(cell))
            {
                cell.span = cell.span.max(1) - 1 + span;
            }
        }
        let mut table = Table {
            columns,
            rows: self.rows,
            borders: self.borders,
        };
        if table.columns.len() != table.grid_width() {
            table.columns.clear();
        }
        Some(table)
    }
}

trait Letter {
    fn letter() -> Self;
}

impl Letter for PageSetup {
    /// What a section with no page sprms has: Letter, with Word 97's
    /// 1.25-inch side margins.
    fn letter() -> Self {
        PageSetup {
            width: 12240,
            height: 15840,
            top: 1440,
            bottom: 1440,
            left: 1800,
            right: 1800,
            header: 720,
            footer: 720,
        }
    }
}

fn read_pieces(table: &[u8], (fc, lcb): (usize, usize)) -> Result<Vec<Piece>, DocError> {
    let clx = slice(table, fc, lcb);
    let mut at = 0;
    // Prcs, each a grpprl, come before the piece table.
    while u8_at(clx, at) == 0x01 {
        let size = i16_at(clx, at + 1).max(0) as usize;
        at += 3 + size;
    }
    if u8_at(clx, at) != 0x02 {
        return Err(DocError::Damaged("no piece table".into()));
    }
    let size = u32_at(clx, at + 1) as usize;
    let plc = slice(clx, at + 5, size);
    if plc.len() < 4 {
        return Err(DocError::Damaged("an empty piece table".into()));
    }
    let count = (plc.len() - 4) / 12;
    let mut pieces = Vec::with_capacity(count);
    for index in 0..count {
        let cp_start = u32_at(plc, index * 4);
        let cp_end = u32_at(plc, (index + 1) * 4);
        let pcd = slice(plc, (count + 1) * 4 + index * 8, 8);
        let fc_compressed = u32_at(pcd, 2);
        pieces.push(Piece {
            cp_start,
            cp_end,
            fc: fc_compressed & 0x3FFF_FFFF,
            compressed: fc_compressed & 0x4000_0000 != 0,
        });
    }
    Ok(pieces)
}

/// A byte of a compressed piece: Windows-1252, as [MS-DOC] 2.4.1 maps it.
fn windows_1252(byte: u8) -> u16 {
    if !(0x80..0xA0).contains(&byte) {
        return byte as u16;
    }
    let text = jade_docx_convert::text::TextEncoding::Windows1252.decode(&[byte]);
    text.encode_utf16().next().unwrap_or(byte as u16)
}

/// The bin table's FKP pages: (n + 1) FCs then n page numbers.
fn fkp_pages(table: &[u8], (fc, lcb): (usize, usize)) -> Vec<usize> {
    let plc = slice(table, fc, lcb);
    if plc.len() < 4 {
        return Vec::new();
    }
    let count = (plc.len() - 4) / 8;
    (0..count)
        .map(|index| (u32_at(plc, (count + 1) * 4 + index * 4) & 0x003F_FFFF) as usize)
        .collect()
}

fn read_chpx(word: &[u8], table: &[u8], pair: (usize, usize)) -> Vec<(u32, u32, Vec<u8>)> {
    let mut out = Vec::new();
    for page in fkp_pages(table, pair) {
        let fkp = slice(word, page * 512, 512);
        if fkp.len() < 512 {
            continue;
        }
        let count = u8_at(fkp, 511) as usize;
        for index in 0..count {
            let start = u32_at(fkp, index * 4);
            let end = u32_at(fkp, (index + 1) * 4);
            let offset = u8_at(fkp, (count + 1) * 4 + index) as usize * 2;
            let grpprl = if offset == 0 {
                Vec::new()
            } else {
                let size = u8_at(fkp, offset) as usize;
                slice(fkp, offset + 1, size).to_vec()
            };
            out.push((start, end, grpprl));
        }
    }
    out.sort_by_key(|(start, _, _)| *start);
    out
}

fn read_papx(word: &[u8], table: &[u8], pair: (usize, usize)) -> Vec<(u32, u32, u16, Vec<u8>)> {
    let mut out = Vec::new();
    for page in fkp_pages(table, pair) {
        let fkp = slice(word, page * 512, 512);
        if fkp.len() < 512 {
            continue;
        }
        let count = u8_at(fkp, 511) as usize;
        for index in 0..count {
            let start = u32_at(fkp, index * 4);
            let end = u32_at(fkp, (index + 1) * 4);
            // BX: a byte offset (in words) and a 12-byte PHE.
            let offset = u8_at(fkp, (count + 1) * 4 + index * 13) as usize * 2;
            if offset == 0 {
                out.push((start, end, 0, Vec::new()));
                continue;
            }
            let mut cb = u8_at(fkp, offset) as usize;
            let mut at = offset + 1;
            let size = if cb == 0 {
                cb = u8_at(fkp, at) as usize;
                at += 1;
                cb * 2
            } else {
                cb * 2 - 1
            };
            let papx = slice(fkp, at, size);
            let istd = u16_at(papx, 0);
            out.push((
                start,
                end,
                istd,
                papx.get(2..).map(<[u8]>::to_vec).unwrap_or_default(),
            ));
        }
    }
    out.sort_by_key(|(start, _, _, _)| *start);
    out
}

/// Each section's CP range and page.
fn read_sections(
    table: &[u8],
    (fc, lcb): (usize, usize),
    word: &[u8],
) -> Vec<(u32, u32, PageSetup)> {
    let plc = slice(table, fc, lcb);
    if plc.len() < 4 {
        return Vec::new();
    }
    let count = (plc.len() - 4) / 16;
    let mut out = Vec::new();
    for index in 0..count {
        let start = u32_at(plc, index * 4);
        let end = u32_at(plc, (index + 1) * 4);
        let sed = slice(plc, (count + 1) * 4 + index * 12, 12);
        let fc_sepx = i32_at(sed, 2);
        let mut page = PageSetup::letter();
        if fc_sepx >= 0 {
            let at = fc_sepx as usize;
            let size = i16_at(word, at).max(0) as usize;
            for (sprm, operand) in sprms(slice(word, at + 2, size)) {
                let value = u16_at(operand, 0) as u32;
                match sprm {
                    0xB01F => page.width = value,
                    0xB020 => page.height = value,
                    0xB021 => page.left = value,
                    0xB022 => page.right = value,
                    0x9023 => page.top = (i16_at(operand, 0) as i32).unsigned_abs(),
                    0x9024 => page.bottom = (i16_at(operand, 0) as i32).unsigned_abs(),
                    0xB017 => page.header = value,
                    0xB018 => page.footer = value,
                    _ => {}
                }
            }
        }
        out.push((start, end, page));
    }
    out
}

/// The header stories' CP ranges, in PlcfHdd's order.
fn read_header_ranges(table: &[u8], (fc, lcb): (usize, usize), base: u32) -> Vec<(u32, u32)> {
    let plc = slice(table, fc, lcb);
    let count = plc.len() / 4;
    let cps: Vec<u32> = (0..count)
        .map(|index| base + u32_at(plc, index * 4))
        .collect();
    cps.windows(2).map(|pair| (pair[0], pair[1])).collect()
}

// ---- pictures --------------------------------------------------------------

/// The first blip among the OfficeArt records in `bytes`.
fn blip_in(bytes: &[u8]) -> Option<Image> {
    let mut found = None;
    walk_records(bytes, &mut |kind, instance, body| {
        if found.is_none() {
            found = blip_record(kind, instance, body);
        }
    });
    found.or_else(|| {
        // No record structure: look for a picture's signature.
        let png = bytes.windows(4).position(|window| window == b"\x89PNG")?;
        Image::new(bytes[png..].to_vec(), None)
    })
}

/// Visits each OfficeArt record, descending into containers.
fn walk_records(bytes: &[u8], visit: &mut dyn FnMut(u16, u16, &[u8])) {
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let ver_instance = u16_at(bytes, at);
        let kind = u16_at(bytes, at + 2);
        let length = u32_at(bytes, at + 4) as usize;
        if !(0xF000..=0xF200).contains(&kind) {
            break;
        }
        let body = slice(bytes, at + 8, length);
        if ver_instance & 0x000F == 0x000F {
            walk_records(body, visit);
        } else {
            visit(kind, ver_instance >> 4, body);
            if kind == 0xF007 {
                // An FBSE: its blip follows its 36-byte header and name.
                let name = u8_at(body, 33) as usize;
                walk_records(slice(body, 36 + name, body.len()), visit);
            }
        }
        at += 8 + length;
    }
}

fn blip_record(kind: u16, instance: u16, body: &[u8]) -> Option<Image> {
    let uids = if instance & 1 == 1 { 32 } else { 16 };
    match kind {
        // JPEG, PNG, DIB, TIFF, CMYK JPEG: the UIDs, a tag byte, the data.
        0xF01D | 0xF01E | 0xF01F | 0xF029 | 0xF02A => {
            let data = slice(body, uids + 1, body.len()).to_vec();
            let data = if kind == 0xF01F {
                dib_to_bmp(&data)?
            } else {
                data
            };
            Image::new(data, None)
        }
        // EMF, WMF: the UIDs, a 34-byte metafile header, then the data,
        // deflated unless the header says otherwise.
        0xF01A | 0xF01B => {
            let header = slice(body, uids, 34);
            let compressed = u8_at(header, 32) == 0;
            let raw = slice(body, uids + 34, body.len());
            let data = if compressed {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(raw)
                    .read_to_end(&mut out)
                    .ok()?;
                out
            } else {
                raw.to_vec()
            };
            let mut data = data;
            if kind == 0xF01B && !data.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
                data = placeable_wmf(&data, header);
            }
            Image::new(data, None)
        }
        _ => None,
    }
}

/// A WMF with the placeable header Office strips, made from the blip's
/// bounds, so readers know its size.
fn placeable_wmf(data: &[u8], header: &[u8]) -> Vec<u8> {
    let left = i32_at(header, 4) as i16;
    let top = i32_at(header, 8) as i16;
    let right = i32_at(header, 12) as i16;
    let bottom = i32_at(header, 16) as i16;
    let mut out = vec![0xD7, 0xCD, 0xC6, 0x9A, 0, 0];
    for value in [left, top, right, bottom] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&1440u16.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    let checksum = out.chunks(2).take(10).fold(0u16, |sum, pair| {
        sum ^ u16::from_le_bytes([pair[0], pair[1]])
    });
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(data);
    out
}

fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let header = u32_at(dib, 0) as usize;
    let bits = u16_at(dib, 14) as usize;
    let used = u32_at(dib, 32) as usize;
    let palette = if used > 0 {
        used
    } else if bits <= 8 {
        1 << bits
    } else {
        0
    };
    let offset = 14 + header + palette * 4;
    let mut out = Vec::with_capacity(dib.len() + 14);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((dib.len() + 14) as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// The blip store's pictures, in order: embedded in each FBSE, or for a
/// delayed one at its offset in the WordDocument stream.
fn blip_store(dgg: &[u8], word: &[u8]) -> Vec<Option<Image>> {
    let mut out = Vec::new();
    // The drawing group container comes first; its blip store is a child.
    let mut at = 0;
    while at + 8 <= dgg.len() {
        let kind = u16_at(dgg, at + 2);
        let length = u32_at(dgg, at + 4) as usize;
        let body = slice(dgg, at + 8, length);
        if kind == 0xF000 {
            let mut inner = 0;
            while inner + 8 <= body.len() {
                let inner_kind = u16_at(body, inner + 2);
                let inner_length = u32_at(body, inner + 4) as usize;
                if inner_kind == 0xF001 {
                    let store = slice(body, inner + 8, inner_length);
                    let mut entry = 0;
                    while entry + 8 <= store.len() {
                        let entry_length = u32_at(store, entry + 4) as usize;
                        let fbse = slice(store, entry + 8, entry_length);
                        let size = u32_at(fbse, 20) as usize;
                        let delay = u32_at(fbse, 28) as usize;
                        let name = u8_at(fbse, 33) as usize;
                        let embedded = slice(fbse, 36 + name, fbse.len());
                        let image = if !embedded.is_empty() {
                            blip_in(embedded)
                        } else if size > 0 {
                            blip_in(slice(word, delay, size))
                        } else {
                            None
                        };
                        out.push(image);
                        entry += 8 + entry_length;
                    }
                }
                inner += 8 + inner_length;
            }
            break;
        }
        at += 8 + length;
    }
    out
}

/// Each shape's blip (its `pib` property) by shape id.
fn shape_blips(dgg: &[u8]) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    collect_shapes(dgg, &mut out);
    out
}

fn collect_shapes(bytes: &[u8], out: &mut HashMap<u32, u32>) {
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let ver_instance = u16_at(bytes, at);
        let kind = u16_at(bytes, at + 2);
        let length = u32_at(bytes, at + 4) as usize;
        if !(0xF000..=0xF200).contains(&kind) {
            break;
        }
        let body = slice(bytes, at + 8, length);
        if kind == 0xF004 {
            // A shape: its FSP (0xF00A) names it, its FOPT (0xF00B) its blip.
            let mut spid = None;
            let mut pib = None;
            let mut inner = 0;
            while inner + 8 <= body.len() {
                let inner_instance = u16_at(body, inner) >> 4;
                let inner_kind = u16_at(body, inner + 2);
                let inner_length = u32_at(body, inner + 4) as usize;
                let record = slice(body, inner + 8, inner_length);
                match inner_kind {
                    0xF00A => spid = Some(u32_at(record, 0)),
                    0xF00B => {
                        for index in 0..inner_instance as usize {
                            let id = u16_at(record, index * 6) & 0x3FFF;
                            if id == 0x0104 {
                                pib = Some(u32_at(record, index * 6 + 2));
                            }
                        }
                    }
                    _ => {}
                }
                inner += 8 + inner_length;
            }
            if let (Some(spid), Some(pib)) = (spid, pib) {
                out.insert(spid, pib);
            }
        } else if ver_instance & 0x000F == 0x000F {
            collect_shapes(body, out);
        }
        at += 8 + length;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprms_split_by_their_operand_sizes() {
        // sprmCFBold on, sprmCHps 24, sprmCCv red.
        let grpprl = [
            0x35, 0x08, 0x01, 0x43, 0x4A, 24, 0, 0x70, 0x68, 0xFF, 0, 0, 0,
        ];
        let list = sprms(&grpprl);
        assert_eq!(list.len(), 3);
        let mut chp = Chp::default();
        apply_chpx(&grpprl, &mut chp, &Chp::default());
        assert!(chp.bold);
        assert_eq!(chp.size, 24);
        assert_eq!(chp.color.as_deref(), Some("FF0000"));
    }

    #[test]
    fn a_file_that_is_not_a_compound_file_is_refused() {
        assert_eq!(read_doc(b"PK\x03\x04").unwrap_err(), DocError::NotWord);
        assert!(!is_doc(b"{\\rtf1}"));
    }

    #[test]
    fn hyperlink_fields_name_their_targets() {
        assert_eq!(
            hyperlink_target(" HYPERLINK \"http://a.example/\" ").as_deref(),
            Some("http://a.example/")
        );
        assert_eq!(
            hyperlink_target("HYPERLINK \\l \"top\"").as_deref(),
            Some("#top")
        );
    }
}
