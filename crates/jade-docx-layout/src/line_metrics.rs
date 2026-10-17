use jade_docx_reader::document::{
    BorderEdge, DocGrid, ParagraphProperties, Run, RunProperties, Spacing,
};

const WORD_DEFAULT_LINE_SPACING_MULTIPLIER: f32 = 1.15;

/// Per-line sizing for Mac paragraphs snapped to an active line grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WordGridLineSizing {
    pitch_pt: f64,
    multiple: f64,
}

impl WordGridLineSizing {
    pub fn new(
        spacing: Option<&Spacing>,
        snap: bool,
        grid: Option<&DocGrid>,
        section_content_height_pt: Option<f32>,
        base_font_size: f32,
    ) -> Option<Self> {
        if !word_grid_line_centres_text(snap, grid) {
            return None;
        }
        let pitch_pt = f64::from(effective_doc_grid_line_pitch_pt(
            grid,
            section_content_height_pt,
            base_font_size,
        )?);
        let multiple = f64::from(proportional_line_multiple(spacing)?);
        (pitch_pt.is_finite() && pitch_pt > 0.0 && multiple.is_finite() && multiple > 0.0)
            .then_some(Self { pitch_pt, multiple })
    }

    pub fn line_height_px(self, single_px: Option<f64>, fallback_px: f64) -> f64 {
        let Some(single_px) = single_px.filter(|px| px.is_finite() && *px > 0.0) else {
            return fallback_px;
        };
        let pitch_px = self.pitch_pt * 96.0 / 72.0;
        ((single_px * self.multiple / pitch_px).ceil().max(1.0)) * pitch_px
    }
}
/// Each auto-spaced Mac line uses its own known font box. Unknown faces,
/// exact/at-least spacing and snapped grids retain their other sizing paths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WordProportionalLineSizing {
    multiple: f64,
}

impl WordProportionalLineSizing {
    pub fn new(spacing: Option<&Spacing>, snap: bool, all_faces_known: bool) -> Option<Self> {
        if word_layout_platform() != WordLayoutPlatform::Mac || snap || !all_faces_known {
            return None;
        }
        let multiple = f64::from(proportional_line_multiple(spacing)?);
        (multiple.is_finite() && multiple > 0.0).then_some(Self { multiple })
    }

    pub fn line_height_px(self, single_px: Option<f64>, fallback_px: f64) -> f64 {
        single_px
            .filter(|px| px.is_finite() && *px > 0.0)
            .map_or(fallback_px, |single| {
                (single * self.multiple).max(96.0 / 72.0)
            })
    }
}

pub const WORD_EAST_ASIAN_FE_LINE_SPACING_MULTIPLIER: f32 = 1.1375;
/// Word's FE-layout auto line box for an otherwise unformatted empty paragraph
/// is slightly taller than the 10.5pt `docDefaults` font in common Japanese
/// contract documents. Keep this shared floor below the 12pt visible-text
/// calibration so repeated blank paragraphs neither collapse nor over-expand.
pub const WORD_EAST_ASIAN_FE_EMPTY_PARAGRAPH_MIN_FONT_SIZE_PT: f32 = 11.0;

/// `w:widowControl` に従って、段落を現在ページでどう分割するかを表す。
///
/// ECMA-376 Part 1 §17.3.1.44 では、先頭行または末尾行だけを別ページに
/// 孤立させてはならない。`w:widowControl` がスタイル階層のどこにも指定
/// されていない場合も既定値は有効である。
///
/// 判定にはグリフのインク境界ではなく、段落配置が実際に消費する行boxの
/// 個数を渡す。字面だけが本文領域に入ることを理由に行boxを無視すると、
/// 事前判定では2行配置可能でも、後続の1行単位の配置処理が2行目を次ページへ
/// 送ってしまい、禁止されている先頭1行の孤立を作るためである。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidowOrphanDecision {
    /// 通常の1行単位の配置を続ける。有効な2行以上の分割を含む。
    FlowNormally,
    /// 現在ページには有効な分割位置がないため、段落全体を次ページへ送る。
    MoveWholeParagraph,
    /// 末尾側に2行を残すため、この行番号の直前で改ページする。
    BreakBeforeLine(usize),
}

/// 行box基準で `w:widowControl` の分割位置を解決する。
pub fn resolve_widow_orphan_decision(
    line_count: usize,
    complete_line_boxes_fit: usize,
) -> WidowOrphanDecision {
    if line_count <= 1 || complete_line_boxes_fit == 0 || complete_line_boxes_fit >= line_count {
        return WidowOrphanDecision::FlowNormally;
    }

    let max_lines_on_current = line_count.saturating_sub(2);
    if complete_line_boxes_fit < 2 || max_lines_on_current < 2 {
        WidowOrphanDecision::MoveWholeParagraph
    } else if complete_line_boxes_fit > max_lines_on_current {
        WidowOrphanDecision::BreakBeforeLine(max_lines_on_current)
    } else {
        WidowOrphanDecision::FlowNormally
    }
}

/// Resolves the implicit FE font-size floor used to construct a paragraph line
/// box. An empty paragraph normally uses Word's narrower paragraph-mark floor,
/// but an explicit font face on `w:pPr/w:rPr` makes the paragraph mark take part
/// in font-metric selection just like visible text.
pub fn word_east_asian_fe_font_size_floor(
    has_visible_run: bool,
    mark_run_properties: Option<&RunProperties>,
) -> f32 {
    let mark_has_explicit_font_face = mark_run_properties.is_some_and(|properties| {
        properties.font_family.is_some()
            || properties
                .fonts
                .as_ref()
                .is_some_and(|fonts| !fonts.is_empty())
    });
    if has_visible_run || mark_has_explicit_font_face {
        12.0
    } else {
        WORD_EAST_ASIAN_FE_EMPTY_PARAGRAPH_MIN_FONT_SIZE_PT
    }
}

/// The part of a paragraph's `w:spacing/@w:before` that Word adds below the
/// previous paragraph's `w:spacing/@w:after` when the two meet: the gap is the
/// larger of the two, not their sum. Word for Mac 16.113 on
/// `system-dev-contract.docx` (body after 6pt, heading before 12pt) puts the
/// heading 12pt below the body; changing the after to 0pt leaves it there, 20pt
/// moves it 8pt down, and after 20pt with before 30pt moves it 18pt — in both
/// compatibility modes 12 and 15.
pub fn collapsed_spacing_before(before: f64, preceding_after: f64) -> f64 {
    (before - preceding_after.max(0.0)).max(0.0)
}

/// Select the preceding paragraph's already-consumed after spacing. Mac
/// section starts retain before spacing, so their adjacent after/before pair
/// still collapses across the section's page/column transition. Ordinary flow
/// breaks and Windows retain their physical-continuity requirement.
pub fn resolve_preceding_body_spacing_after(
    trailing: Option<(usize, usize, f64, f64)>,
    page: usize,
    column: usize,
    y: f64,
    at_later_section_flow_start: impl FnOnce() -> bool,
) -> f64 {
    trailing
        .filter(|&(trailing_page, trailing_column, trailing_y, _)| {
            (trailing_page == page && trailing_column == column && (trailing_y - y).abs() <= 0.01)
                || (word_layout_platform() == WordLayoutPlatform::Mac
                    && at_later_section_flow_start())
        })
        .map_or(0.0, |(_, _, _, after)| after)
}

/// Mac body paragraphs omit before spacing at a page/column flow start,
/// except the first block of a section. Inline page breaks retain their
/// separate compatibility contract; cell and auxiliary callers use normal
/// collapsed spacing instead.
pub fn resolve_body_paragraph_spacing_before(
    before: f64,
    preceding_after: f64,
    at_flow_start: bool,
    section_start: bool,
    contains_inline_page_break: bool,
) -> f64 {
    if word_layout_platform() == WordLayoutPlatform::Mac
        && at_flow_start
        && !section_start
        && !contains_inline_page_break
    {
        0.0
    } else {
        collapsed_spacing_before(before, preceding_after)
    }
}

/// Which Word the layout reproduces where Word for Mac and Word for Windows
/// lay the same document out differently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordLayoutPlatform {
    /// Word for Windows: the reference for the Word COM traces.
    #[default]
    Windows,
    /// Word for Mac.
    Mac,
}

thread_local! {
    static WORD_LAYOUT_PLATFORM: std::cell::Cell<WordLayoutPlatform> =
        const { std::cell::Cell::new(WordLayoutPlatform::Windows) };
}

/// Sets which Word this thread's layout reproduces. Hosts call it once,
/// before the first document is laid out: cached layouts are not invalidated.
pub fn set_word_layout_platform(platform: WordLayoutPlatform) {
    WORD_LAYOUT_PLATFORM.with(|cell| cell.set(platform));
}

pub fn word_layout_platform() -> WordLayoutPlatform {
    WORD_LAYOUT_PLATFORM.with(std::cell::Cell::get)
}

/// An auto-spacing gap (autoSpaceDE/DN), in px, as the target Word places it.
/// Word for Windows lays the gap out in whole 96 dpi pixels: its COM traces
/// put 3.0pt (4px) before every number and 2.25pt (3px) after it at 10.5pt,
/// where Word for Mac prints the exact 1/4 em and 1/5 em (2.625pt, 2.1pt) of
/// `system-dev-contract.docx`.
pub fn word_auto_space_gap_px(gap_px: f64) -> f64 {
    match word_layout_platform() {
        WordLayoutPlatform::Windows => gap_px.round(),
        WordLayoutPlatform::Mac => gap_px,
    }
}

/// The auto-spacing (autoSpaceDE/DN) Word leaves around a glyph set in a face
/// whose metrics it is known for.
///
/// Word for Mac 16.113 puts 1/4 em of the glyph's size between East Asian text
/// and a following Latin letter or digit, whatever the faces, and half the
/// Latin face's OS/2 `xAvgCharWidth` between the letter or digit and following
/// East Asian text: at 10.5pt 2.1pt after Times New Roman, 2.29pt after
/// Arial, 2.41pt after Century, 2.70pt after Calibri and 3.19pt after
/// Cambria (3.0pt and 2.41pt before/after 12pt Times New Roman). East Asian
/// faces setting Latin text leave 1/4 em on both sides (MS Mincho and Yu
/// Mincho digits in `yokohama_page6`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WordAutoSpace {
    /// The glyph's font size, in px.
    pub em_px: f64,
    /// The gap after the glyph, before East Asian text, as a share of 1/4 em.
    pub trailing_share: f64,
}

impl WordAutoSpace {
    /// The gap between East Asian text and this glyph, in px.
    pub fn leading_gap_px(self) -> f64 {
        self.em_px / 4.0
    }

    /// The gap between this glyph and following East Asian text, in px.
    /// `w:balanceSingleByteDoubleByteWidth` makes it 1/4 em like the leading
    /// gap: Word for Mac 16.113 then leaves 2.58pt after 10.5pt Century and
    /// Times New Roman instead of 2.41pt and 2.07pt, and keeps every glyph's
    /// own advance.
    pub fn trailing_gap_px(self, balance_single_byte_double_byte_width: bool) -> f64 {
        if balance_single_byte_double_byte_width {
            self.leading_gap_px()
        } else {
            self.em_px / 4.0 * self.trailing_share
        }
    }
}

/// Whether `w:balanceSingleByteDoubleByteWidth` widens single-byte glyphs.
/// Word for Mac 16.113 draws every Latin glyph at its own advance with it on;
/// the widening stays for Word for Windows, whose traces it was fitted to.
pub fn word_widens_balanced_single_byte_glyphs(platform: WordLayoutPlatform) -> bool {
    platform == WordLayoutPlatform::Windows
}

/// Word for Mac gives the initial authored ASCII space in a note paragraph
/// half an em. Corpus PDFs show 5.25pt at 10.5pt after automatic references in
/// five note fixtures and at the start of 39 subsequent paragraphs in the long
/// note. Ordinary spaces within those runs keep their font advance.
pub fn word_automatic_note_separator_advance_px(
    platform: WordLayoutPlatform,
    font_descriptor: &str,
) -> Option<f64> {
    if platform != WordLayoutPlatform::Mac {
        return None;
    }
    let (size_pt, _) = split_font_descriptor(font_descriptor)?;
    (size_pt.is_finite() && size_pt > 0.0).then_some(f64::from(size_pt) * 96.0 / 72.0 * 0.5)
}

/// Whether `compressPunctuation` narrows East Asian punctuation inside a
/// line. Word for Mac 16.113 keeps `、` before `地` and `」、（` at the full 10.5pt
/// em; the narrowing stays for Word for Windows, whose traces it was fitted to.
pub fn word_compresses_mid_line_punctuation(platform: WordLayoutPlatform) -> bool {
    platform == WordLayoutPlatform::Windows
}

/// Whether `w:balanceSingleByteDoubleByteWidth` turns off the auto-spacing
/// around digits. Word for Mac 16.113 keeps 1/4 em on both sides of `25` in
/// `yokohama_page1`; Word for Windows traces were fitted without it.
pub fn word_balance_suppresses_numeric_auto_space(platform: WordLayoutPlatform) -> bool {
    platform == WordLayoutPlatform::Windows
}

/// Whether a run whose fonts resolve to the Default hint (a direct `w:rFonts`
/// without `w:hint="eastAsia"` or an East Asian face) loses the auto-spacing
/// around its digits. Word for Mac 16.113 keeps 1/4 em on both sides of `7`
/// in `別紙7の` set with `<w:rFonts w:ascii="ＭＳ 明朝"/>`
/// (`trust-beneficiary-sale-keep-next`); the suppression stays for Word for
/// Windows, whose traces it was fitted to.
pub fn word_default_hint_suppresses_numeric_auto_space(platform: WordLayoutPlatform) -> bool {
    platform == WordLayoutPlatform::Windows
}

/// The East Asian advance scale for `compressPunctuation` paragraphs without
/// a grid. Word for Mac 16.113 sets every 10.5pt MS Mincho glyph, punctuation
/// included, at 10.5pt; the 1% stays for Word for Windows, whose traces it was
/// fitted to.
pub fn word_compressed_punctuation_east_asian_advance_scale() -> f64 {
    match word_layout_platform() {
        WordLayoutPlatform::Windows => crate::constants::EAST_ASIA_FE_ADVANCE_SCALE,
        WordLayoutPlatform::Mac => 1.0,
    }
}

/// [`WordAutoSpace`] for a glyph drawn with `font_descriptor` in Word for Mac,
/// or `None` for a face whose `xAvgCharWidth` is not known. Word for Windows
/// keeps its COM-traced gaps: 2.25pt after an MS Mincho digit in
/// `yokohama_page1`, where Word for Mac leaves 1/4 em.
pub fn word_auto_space_metrics(font_descriptor: &str) -> Option<WordAutoSpace> {
    word_auto_space_metrics_with_balance(font_descriptor, false)
}

/// Balanced SBCS/DBCS spacing uses a quarter em on both sides, so it does
/// not require the Latin face's average-width metric. Unbalanced unknown
/// faces retain the ordinary host fallback.
pub fn word_auto_space_metrics_with_balance(
    font_descriptor: &str,
    balance_single_byte_double_byte_width: bool,
) -> Option<WordAutoSpace> {
    if word_layout_platform() != WordLayoutPlatform::Mac {
        return None;
    }
    WORD_AUTO_SPACE_METRICS.with(|cell| {
        let mut cached = cell.borrow_mut();
        if let Some((descriptor, balance, metrics)) = cached.as_ref() {
            if descriptor == font_descriptor && *balance == balance_single_byte_double_byte_width {
                return *metrics;
            }
        }
        let metrics =
            parse_word_auto_space_metrics(font_descriptor, balance_single_byte_double_byte_width);
        if let Some((descriptor, balance, cached_metrics)) = cached.as_mut() {
            descriptor.clear();
            descriptor.push_str(font_descriptor);
            *balance = balance_single_byte_double_byte_width;
            *cached_metrics = metrics;
        } else {
            *cached = Some((
                font_descriptor.to_owned(),
                balance_single_byte_double_byte_width,
                metrics,
            ));
        }
        metrics
    })
}

fn parse_word_auto_space_metrics(
    font_descriptor: &str,
    balance_single_byte_double_byte_width: bool,
) -> Option<WordAutoSpace> {
    #[cfg(test)]
    WORD_AUTO_SPACE_DESCRIPTOR_PARSES.with(|count| count.set(count.get() + 1));
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    let em_px = f64::from(size_pt) * 96.0 / 72.0;
    if balance_single_byte_double_byte_width {
        return (em_px.is_finite() && em_px > 0.0).then_some(WordAutoSpace {
            em_px,
            trailing_share: 1.0,
        });
    }
    if word_east_asian_font_vertical_units(family).is_some() {
        return Some(WordAutoSpace {
            em_px,
            trailing_share: 1.0,
        });
    }
    let (bold, italic) = font_descriptor_style(font_descriptor);
    let family = family
        .trim()
        .trim_matches(|ch| ch == '\'' || ch == '"')
        .trim();
    let average_units = match (family, bold, italic) {
        ("Times New Roman", true, _) => 874.0,
        ("Times New Roman", false, true) => 823.0,
        ("Times New Roman", false, false) => 821.0,
        ("Arial", true, _) => 980.0,
        ("Arial", false, _) => 904.0,
        ("Century", false, false) => 951.0,
        // Official Aptos 2.01 OS/2 average width also matches original Word i→CJK origins.
        ("Aptos", false, false) => 1148.0,
        // Mac Noto PDF's repeated digit→CJK origins leave 0.28515625em,
        // the same quarter-em rule with 1168/2048 average character units.
        ("Noto Sans", false, false) => 1168.0,
        ("Calibri", true, _) => 1098.0,
        ("Calibri", false, _) => 1067.0,
        ("Calibri Light", false, _) => 1065.0,
        ("Cambria", true, _) => 1228.0,
        ("Cambria", false, true) => 1111.0,
        ("Cambria", false, false) => 1260.0,
        ("Verdana", true, _) => 1163.0,
        ("Verdana", false, _) => 1041.0,
        _ => return None,
    };
    Some(WordAutoSpace {
        em_px,
        trailing_share: 2.0 * average_units / 2048.0,
    })
}

thread_local! {
    // Own the descriptor because callers can mutate their font buffer. Keep
    // one entry per thread, including misses, so cache size cannot grow with
    // document fonts. Balance affects unknown-face fallback and is part of
    // the key; the platform is checked before consulting these Mac metrics.
    static WORD_AUTO_SPACE_METRICS: std::cell::RefCell<Option<(String, bool, Option<WordAutoSpace>)>> =
        const { std::cell::RefCell::new(None) };
    #[cfg(test)]
    static WORD_AUTO_SPACE_DESCRIPTOR_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    // Own the descriptor: callers reuse buffers between runs, so pointer keys
    // could return a previous font's width. One entry also bounds cache memory.
    static WORD_FIXED_ASCII_ADVANCE: std::cell::RefCell<Option<(String, WordLayoutPlatform, Option<WordAsciiMetrics>)>> =
        const { std::cell::RefCell::new(None) };
    #[cfg(test)]
    static WORD_FIXED_ASCII_DESCRIPTOR_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    #[cfg(test)]
    pub(crate) static WORD_FIXED_ASCII_DESCRIPTOR_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// MS Mincho and MS Gothic's printable ASCII advance is half an em, even when the host
/// substitutes a proportional face. Word's committed Yokohama PDF references
/// put adjacent 10.5pt digits, parentheses and commas 5.25pt apart.
/// Synthetic bold adds 1/50 em on Mac. Proportional MS PMincho/PGothic and
/// unrelated fallback families keep host metrics.
#[inline]
pub fn word_fixed_ascii_advance_px(font_descriptor: &str) -> Option<f64> {
    match word_ascii_metrics(font_descriptor)? {
        WordAsciiMetrics::Fixed(advance) => Some(advance),
        WordAsciiMetrics::YuMincho(_)
        | WordAsciiMetrics::YuMinchoBold(_)
        | WordAsciiMetrics::YuGothicLight(_)
        | WordAsciiMetrics::Century(_)
        | WordAsciiMetrics::NotoSansJp(_)
        | WordAsciiMetrics::NotoSans(_)
        | WordAsciiMetrics::Calibri(_, _)
        | WordAsciiMetrics::Aptos(_, _)
        | WordAsciiMetrics::Bookman(_) => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum WordAsciiMetrics {
    Fixed(f64),
    // Pixels per font unit, resolved once rather than divided per glyph.
    YuMincho(f64),
    YuMinchoBold(f64),
    YuGothicLight(f64),
    Century(f64),
    NotoSansJp(f64),
    NotoSans(f64),
    Calibri(f64, bool),
    Aptos(f64, bool),
    Bookman(f64),
}

// Numeric hmtx facts only: 52 regular / 55 bold ASCII mappings in 42 / 11
// unchanged Word Mac PDFs. Index is printable ASCII minus space. Zero marks
// an unobserved glyph and keeps host fallback, not a zero-width character.
const WORD_CALIBRI_ASCII_UNITS: [u16; 95] = [
    463, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 627, 0, 791, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 1185, 1114, 1092, 1260, 1000, 941, 1292, 1276, 516, 0, 1064, 861, 1751, 1322, 1356,
    1058, 1378, 1112, 941, 998, 0, 1162, 1822, 0, 998, 959, 0, 0, 0, 0, 0, 0, 981, 1076, 866, 1076,
    1019, 625, 964, 1076, 470, 490, 931, 470, 1636, 1076, 1080, 1076, 1076, 714, 801, 686, 1076,
    925, 1464, 887, 927, 809, 0, 0, 0, 0,
];
const WORD_CALIBRI_BOLD_ASCII_UNITS: [u16; 95] = [
    463, 0, 0, 0, 0, 0, 0, 0, 638, 638, 0, 0, 528, 627, 547, 0, 0, 1038, 1038, 1038, 1038, 0, 0, 0,
    0, 0, 565, 0, 0, 0, 0, 0, 0, 1241, 1148, 1084, 1291, 999, 940, 1305, 1292, 546, 0, 0, 866,
    1790, 1349, 1385, 1090, 0, 1153, 968, 1014, 1337, 1211, 0, 0, 0, 0, 0, 880, 0, 0, 1020, 0,
    1011, 1099, 857, 1099, 1031, 648, 971, 1099, 503, 0, 983, 503, 1666, 1099, 1101, 1099, 0, 728,
    817, 710, 1099, 969, 1526, 941, 970, 0, 0, 0, 0, 0,
];

// Source-matched Calibri Title runs in 40 original Word PDFs, at 22/24pt.
// Subtract the observed hmtx and source tracking from adjacent origins;
// round median design units, treating <2 units as PDF positioning noise.
// The 304 observed pairs include zero adjustments; missing pairs keep the
// host path. A fixed compile-time table avoids per-glyph maps/allocations.
static WORD_CALIBRI_PAIR_UNITS: [i16; 95 * 95] = {
    let pairs = [
        (b' ', b'/', 0),
        (b' ', b'A', 0),
        (b' ', b'B', 0),
        (b' ', b'C', 0),
        (b' ', b'D', 0),
        (b' ', b'E', 0),
        (b' ', b'F', 0),
        (b' ', b'H', 0),
        (b' ', b'I', 0),
        (b' ', b'M', 0),
        (b' ', b'N', 0),
        (b' ', b'O', 0),
        (b' ', b'P', 0),
        (b' ', b'R', 0),
        (b' ', b'S', 0),
        (b' ', b'T', 0),
        (b' ', b'V', 0),
        (b' ', b'W', 0),
        (b' ', b'Z', 0),
        (b' ', b'a', 0),
        (b'-', b'L', 0),
        (b'-', b'P', 0),
        (b'-', b'W', 0),
        (b'/', b' ', 0),
        (b'/', b'F', 0),
        (b'A', b'B', 0),
        (b'A', b'G', -16),
        (b'A', b'R', 0),
        (b'A', b'l', 0),
        (b'A', b'n', 0),
        (b'A', b'u', 0),
        (b'B', b'L', 0),
        (b'B', b'o', 0),
        (b'C', b' ', 0),
        (b'C', b'O', -19),
        (b'C', b'P', 0),
        (b'C', b'T', 9),
        (b'C', b'V', 0),
        (b'C', b'a', 0),
        (b'C', b'e', 0),
        (b'C', b'o', 0),
        (b'C', b'r', 0),
        (b'D', b'O', 0),
        (b'D', b'e', 0),
        (b'D', b'i', 0),
        (b'E', b' ', 0),
        (b'E', b'C', -24),
        (b'E', b'Q', -31),
        (b'E', b'R', 0),
        (b'E', b'S', -20),
        (b'E', b'd', -30),
        (b'E', b'n', 0),
        (b'F', b' ', 0),
        (b'F', b'i', 0),
        (b'F', b'l', 0),
        (b'F', b'o', -29),
        (b'G', b'E', 0),
        (b'H', b'Y', 0),
        (b'H', b'e', 0),
        (b'H', b'i', 0),
        (b'H', b'o', 0),
        (b'I', b'A', 0),
        (b'I', b'F', 0),
        (b'I', b'N', 0),
        (b'I', b'O', 0),
        (b'I', b'm', 0),
        (b'I', b'n', 0),
        (b'K', b' ', 0),
        (b'L', b'E', 0),
        (b'L', b'I', 0),
        (b'L', b'e', 0),
        (b'M', b'P', 0),
        (b'M', b'a', 0),
        (b'M', b'e', 0),
        (b'N', b'K', 0),
        (b'N', b'P', 0),
        (b'N', b'o', 0),
        (b'N', b'u', 0),
        (b'O', b'C', 0),
        (b'O', b'M', 0),
        (b'O', b'N', 0),
        (b'O', b'P', 0),
        (b'O', b'b', 0),
        (b'O', b'f', 0),
        (b'O', b'r', 0),
        (b'O', b'u', 0),
        (b'O', b'v', 0),
        (b'P', b'A', -152),
        (b'P', b'E', 0),
        (b'P', b'R', 0),
        (b'P', b'a', -45),
        (b'P', b'o', -41),
        (b'P', b'r', 0),
        (b'Q', b' ', 0),
        (b'R', b'E', 0),
        (b'R', b'I', 0),
        (b'R', b'L', 0),
        (b'R', b'O', -21),
        (b'R', b'T', -21),
        (b'R', b'e', -36),
        (b'R', b'o', -42),
        (b'S', b' ', 0),
        (b'S', b'E', 0),
        (b'S', b'e', 0),
        (b'S', b'h', 0),
        (b'S', b'i', 0),
        (b'S', b'o', 0),
        (b'S', b'p', 0),
        (b'S', b'q', 0),
        (b'T', b'I', 0),
        (b'T', b'O', -58),
        (b'T', b'Y', 0),
        (b'T', b'a', -160),
        (b'T', b'e', -182),
        (b'T', b'h', 0),
        (b'T', b'i', 0),
        (b'T', b'o', -182),
        (b'V', b'A', -95),
        (b'V', b'e', -102),
        (b'V', b'i', 0),
        (b'W', b'i', 0),
        (b'W', b'o', -86),
        (b'W', b'r', -61),
        (b'Y', b' ', 0),
        (b'Y', b'P', 0),
        (b'Z', b' ', 0),
        (b'a', b'b', 0),
        (b'a', b'c', 0),
        (b'a', b'd', 0),
        (b'a', b'g', 0),
        (b'a', b'l', 0),
        (b'a', b'n', 0),
        (b'a', b'p', 0),
        (b'a', b'r', 0),
        (b'a', b's', 0),
        (b'a', b't', -20),
        (b'a', b'u', 0),
        (b'a', b'v', -34),
        (b'b', b'e', 0),
        (b'b', b'i', 0),
        (b'b', b'j', 0),
        (b'b', b'l', 0),
        (b'c', b'a', -18),
        (b'c', b'e', 0),
        (b'c', b'h', 0),
        (b'c', b'i', 0),
        (b'c', b't', 0),
        (b'c', b'y', 0),
        (b'd', b' ', 0),
        (b'd', b'd', 0),
        (b'd', b'e', 0),
        (b'd', b'g', 0),
        (b'd', b'i', 0),
        (b'd', b't', 0),
        (b'd', b'y', 0),
        (b'e', b' ', 0),
        (b'e', b'-', 0),
        (b'e', b'a', 0),
        (b'e', b'c', 0),
        (b'e', b'd', 0),
        (b'e', b'f', -19),
        (b'e', b'g', 0),
        (b'e', b'h', 0),
        (b'e', b'i', 0),
        (b'e', b'l', 0),
        (b'e', b'n', 0),
        (b'e', b'p', 0),
        (b'e', b'q', 0),
        (b'e', b'r', 0),
        (b'e', b's', 0),
        (b'e', b't', -10),
        (b'e', b'v', -10),
        (b'e', b'x', -31),
        (b'f', b'a', -40),
        (b'f', b'e', -50),
        (b'f', b'f', -19),
        (b'f', b'i', 0),
        (b'f', b'l', 0),
        (b'f', b's', -27),
        (b'g', b' ', 0),
        (b'g', b'a', -37),
        (b'g', b'e', -17),
        (b'g', b'h', 0),
        (b'g', b'n', 0),
        (b'g', b'o', -15),
        (b'h', b' ', 0),
        (b'h', b'a', 0),
        (b'h', b'e', 0),
        (b'h', b'o', 0),
        (b'h', b'r', 0),
        (b'h', b's', 0),
        (b'h', b't', -18),
        (b'i', b'V', 0),
        (b'i', b'a', 0),
        (b'i', b'b', 0),
        (b'i', b'c', 0),
        (b'i', b'd', 0),
        (b'i', b'e', 0),
        (b'i', b'g', 0),
        (b'i', b'l', 0),
        (b'i', b'm', 0),
        (b'i', b'n', 0),
        (b'i', b'o', 0),
        (b'i', b'r', 0),
        (b'i', b's', 0),
        (b'i', b't', 0),
        (b'i', b'v', 0),
        (b'i', b'x', 0),
        (b'i', b'z', 0),
        (b'j', b'e', 0),
        (b'k', b'e', -66),
        (b'l', b' ', 0),
        (b'l', b'a', 0),
        (b'l', b'e', 0),
        (b'l', b'i', 0),
        (b'l', b'l', 0),
        (b'l', b'o', 0),
        (b'l', b't', 0),
        (b'l', b'y', 0),
        (b'm', b' ', 0),
        (b'm', b'a', 0),
        (b'm', b'b', 0),
        (b'm', b'e', 0),
        (b'm', b'p', 0),
        (b'n', b' ', 0),
        (b'n', b'a', 0),
        (b'n', b'c', 0),
        (b'n', b'd', 0),
        (b'n', b'e', 0),
        (b'n', b'f', -12),
        (b'n', b'g', 0),
        (b'n', b'l', 0),
        (b'n', b'm', 0),
        (b'n', b's', 0),
        (b'n', b't', -20),
        (b'o', b'-', 0),
        (b'o', b'a', 0),
        (b'o', b'd', 0),
        (b'o', b'f', 0),
        (b'o', b'l', 0),
        (b'o', b'm', 0),
        (b'o', b'n', 0),
        (b'o', b'o', 0),
        (b'o', b'p', 0),
        (b'o', b'r', 0),
        (b'o', b's', 0),
        (b'o', b't', 0),
        (b'o', b'u', 0),
        (b'o', b'w', -8),
        (b'o', b'x', -40),
        (b'p', b' ', 0),
        (b'p', b'a', 0),
        (b'p', b'e', 0),
        (b'p', b'l', 0),
        (b'p', b'r', 0),
        (b'p', b't', -9),
        (b'q', b'u', 0),
        (b'r', b' ', 0),
        (b'r', b'/', 0),
        (b'r', b'T', 0),
        (b'r', b'a', -42),
        (b'r', b'c', -30),
        (b'r', b'd', -28),
        (b'r', b'e', -27),
        (b'r', b'g', -28),
        (b'r', b'i', 0),
        (b'r', b'k', 0),
        (b'r', b'l', 0),
        (b'r', b'o', -34),
        (b'r', b'r', 0),
        (b'r', b's', -34),
        (b'r', b't', 0),
        (b's', b'e', 0),
        (b's', b'i', 0),
        (b's', b't', -23),
        (b's', b'u', 0),
        (b't', b' ', 0),
        (b't', b'a', -24),
        (b't', b'e', -22),
        (b't', b'h', 0),
        (b't', b'i', 0),
        (b't', b'l', 0),
        (b't', b'o', -20),
        (b't', b'r', 0),
        (b't', b's', 0),
        (b't', b't', -29),
        (b't', b'u', 0),
        (b't', b'y', 0),
        (b'u', b'a', 0),
        (b'u', b'e', 0),
        (b'u', b'g', 0),
        (b'u', b'l', 0),
        (b'u', b'm', 0),
        (b'u', b'r', 0),
        (b'u', b't', 0),
        (b'v', b'e', -21),
        (b'w', b' ', 0),
        (b'w', b's', -17),
        (b'x', b' ', 0),
        (b'x', b'e', -55),
        (b'x', b't', 7),
        (b'y', b' ', 0),
        (b'y', b'g', -25),
        (b'z', b'o', -45),
    ];
    let mut units = [i16::MIN; 95 * 95];
    let mut index = 0;
    while index < pairs.len() {
        let (left, right, adjustment) = pairs[index];
        units[(left - b' ') as usize * 95 + (right - b' ') as usize] = adjustment;
        index += 1;
    }
    units
};

// Numeric hmtx facts: 506 original Word PDF advances agree with Aptos 2.01
// regular/Display units within 0.009pt. Only 21/5 observed characters are used;
// unobserved characters/styles retain the host path.
const WORD_APTOS_ASCII_UNITS: [u16; 95] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 585, 0, 0, 1094, 1094, 1094, 1094, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 1207, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1618, 0, 0, 0, 0, 0, 0, 980, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1088, 1149, 1075, 1148, 1079, 0, 0, 1129, 489, 0, 0, 0, 1747, 1129,
    0, 0, 0, 0, 995, 662, 1144, 926, 0, 0, 0, 0, 0, 0, 0, 0,
];
const WORD_APTOS_DISPLAY_ASCII_UNITS: [u16; 95] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 977, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 1062, 0, 0, 1012, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 948, 622, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0,
];

impl WordAsciiMetrics {
    /// Source-matched pair origins in the original Word PDFs. For example,
    /// Yu Mincho's 10.5pt W/o pitch is 2013 - 123 design units. Unobserved
    /// pairs and faces retain host kerning until their requested-face value
    /// is known; confirmed zero adjustments also bypass host measurement.
    #[inline]
    pub(crate) fn pair_kerning_px(self, left: u8, right: u8) -> Option<f64> {
        match (self, left, right) {
            (Self::Aptos(unit_px, display), left, right) => {
                let units = if display {
                    match (left, right) {
                        (b'T', b'e') => -164,
                        (b'e', b's') => -10,
                        (b's', b't') => -12,
                        (b'b', b'b') => 0,
                        _ => return None,
                    }
                } else {
                    match (left, right) {
                        (b'T', b'e') => -164,
                        (b'T', b'a') => -154,
                        (b'e', b's') | (b't', b'e') => -10,
                        (b's', b't') => -12,
                        (b't', b's') => -8,
                        (b'v', b'a') => -7,
                        (b'T', b't')
                        | (b't', b't')
                        | (b'A', b'd')
                        | (b'M', b'm')
                        | (b'a', b'e')
                        | (b'a', b'h')
                        | (b'a', b'n')
                        | (b'c', b'e')
                        | (b'd', b'v')
                        | (b'e', b'a')
                        | (b'e', b'u')
                        | (b'h', b't')
                        | (b'h', b'u')
                        | (b'm', b'm')
                        | (b'n', b'c')
                        | (b't', b'h')
                        | (b't', b'u')
                        | (b'u', b'e')
                        | (b'u', b'u') => 0,
                        _ => return None,
                    }
                };
                Some(unit_px * f64::from(units))
            }
            (Self::Calibri(unit_px, false), left, right)
                if (b' '..=b'~').contains(&left) && (b' '..=b'~').contains(&right) =>
            {
                let units = WORD_CALIBRI_PAIR_UNITS
                    [usize::from(left - b' ') * 95 + usize::from(right - b' ')];
                (units != i16::MIN).then_some(unit_px * f64::from(units))
            }
            (Self::YuMincho(unit_px), left, right)
                if (b' '..=b'~').contains(&left) && (b' '..=b'~').contains(&right) =>
            {
                let units = WORD_YU_MINCHO_PAIR_UNITS
                    [usize::from(left - b' ') * 95 + usize::from(right - b' ')];
                (units != i16::MIN).then_some(unit_px * f64::from(units))
            }
            (Self::YuMinchoBold(unit_px), b'W', b'o') => Some(-123.0 * unit_px),
            (Self::YuMinchoBold(_), left, right)
                if matches!(
                    (left, right),
                    (b'H', b'e')
                        | (b'e', b'l')
                        | (b'l', b'l')
                        | (b'l', b'o')
                        | (b'o', b' ')
                        | (b' ', b'W')
                        | (b'o', b'r')
                        | (b'r', b'l')
                        | (b'l', b'd')
                        | (b'd', b' ')
                        | (b' ', b'H')
                ) =>
            {
                Some(0.0)
            }
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn advance_px(self, ascii: u8) -> Option<f64> {
        if !(b' '..=b'~').contains(&ascii) {
            return None;
        }
        match self {
            Self::Fixed(advance) => Some(advance),
            Self::YuGothicLight(unit_px) => {
                // Numeric hmtx facts in the unchanged numbering Word PDF.
                // Unobserved characters and other Yu Gothic faces keep fallback.
                let units = match ascii {
                    b' ' => 584.0,
                    b's' => 969.0,
                    b'o' => 1061.0,
                    b'm' => 1686.0,
                    b'e' => 1055.0,
                    b't' => 608.0,
                    b'h' | b'n' => 1106.0,
                    b'i' => 457.0,
                    b'g' => 1047.0,
                    _ => return None,
                };
                Some(unit_px * units)
            }
            Self::Aptos(unit_px, display) => {
                let units = if display {
                    WORD_APTOS_DISPLAY_ASCII_UNITS[usize::from(ascii - b' ')]
                } else {
                    WORD_APTOS_ASCII_UNITS[usize::from(ascii - b' ')]
                };
                (units != 0).then_some(unit_px * f64::from(units))
            }
            Self::Calibri(unit_px, bold) => {
                let units = if bold {
                    WORD_CALIBRI_BOLD_ASCII_UNITS[usize::from(ascii - b' ')]
                } else {
                    WORD_CALIBRI_ASCII_UNITS[usize::from(ascii - b' ')]
                };
                (units != 0).then_some(unit_px * f64::from(units))
            }
            Self::Bookman(unit_px) => {
                // Fourteen source-matched hmtx mappings in Word Mac's math
                // PDF. Other characters and style variants keep host widths.
                let units = match ascii {
                    b' ' | b'.' | b'f' => 655.0,
                    b'0' | b'1' | b'2' | b'3' => 1270.0,
                    b'M' => 1884.0,
                    b'c' | b's' => 1065.0,
                    b'i' => 614.0,
                    b'o' => 1147.0,
                    b'r' => 901.0,
                    b't' => 778.0,
                    _ => return None,
                };
                Some(unit_px * units)
            }
            Self::NotoSans(unit_px) => {
                // Fifteen scalar mappings in the unchanged Mac Noto Sans PDF.
                // These 2048-upem advances differ from Noto Sans JP's subset.
                let units = match ascii {
                    b'0'..=b'9' => 1128.0,
                    b'(' | b')' => 724.0,
                    b',' => 512.0,
                    b'.' => 549.0,
                    b' ' => 532.0,
                    _ => return None,
                };
                Some(unit_px * units)
            }
            Self::NotoSansJp(unit_px) => {
                // Numeric hmtx facts from the 15 observed ASCII characters in
                // the unchanged Noto JP Word PDF (1000 units/em). ToUnicode
                // maps the subset's character codes to these Unicode values.
                let units = match ascii {
                    b'0'..=b'9' => 555.0,
                    b'(' | b')' => 338.0,
                    b',' | b'.' => 278.0,
                    b' ' => 224.0,
                    _ => return None,
                };
                Some(unit_px * units)
            }
            Self::Century(unit_px) => {
                let units = CENTURY_ASCII_UNITS[usize::from(ascii - b' ')];
                (units > 0).then_some(unit_px * f64::from(units))
            }
            Self::YuMincho(unit_px) | Self::YuMinchoBold(unit_px) => {
                let units = YU_MINCHO_ASCII_UNITS[usize::from(ascii - b' ')];
                let synthetic_units = if matches!(self, Self::YuMinchoBold(_)) {
                    2048.0 * 0.02
                } else {
                    0.0
                };
                (units > 0).then_some(unit_px * (f64::from(units) + synthetic_units))
            }
        }
    }
}

// Source-resolved regular Yu Mincho origins in 20 original Word PDFs. Only
// horizontal, unscaled words with zero tracking and enabled kerning are used.
// Retain the previously observed space-boundary facts as well. Unknown pairs
// keep host measurement; the read-only lookup allocates nothing at runtime.
static WORD_YU_MINCHO_PAIR_UNITS: [i16; 95 * 95] = {
    let pairs: [(u8, u8, i16); 193] = [
        (b' ', b'H', 0),
        (b' ', b'W', 0),
        (b'0', b'.', 0),
        (b'1', b'.', 0),
        (b'1', b'0', 0),
        (b'1', b'1', 0),
        (b'1', b'2', 0),
        (b'1', b'3', 0),
        (b'1', b'4', 0),
        (b'1', b'5', 0),
        (b'1', b'6', 0),
        (b'1', b'7', 0),
        (b'1', b'8', 0),
        (b'1', b'9', 0),
        (b'2', b'.', 0),
        (b'2', b'0', 0),
        (b'2', b'1', 0),
        (b'2', b'2', 0),
        (b'2', b'3', 0),
        (b'2', b'4', 0),
        (b'2', b'5', 0),
        (b'2', b'6', 0),
        (b'2', b'7', 0),
        (b'2', b'8', 0),
        (b'2', b'9', 0),
        (b'3', b'.', 0),
        (b'3', b'0', 0),
        (b'3', b'1', 0),
        (b'3', b'2', 0),
        (b'3', b'3', 0),
        (b'3', b'4', 0),
        (b'3', b'5', 0),
        (b'3', b'6', 0),
        (b'3', b'7', 0),
        (b'3', b'8', 0),
        (b'3', b'9', 0),
        (b'4', b'.', 0),
        (b'4', b'0', 0),
        (b'4', b'1', 0),
        (b'4', b'2', 0),
        (b'4', b'3', 0),
        (b'4', b'4', 0),
        (b'4', b'5', 0),
        (b'4', b'6', 0),
        (b'4', b'7', 0),
        (b'4', b'8', 0),
        (b'4', b'9', 0),
        (b'5', b'.', 0),
        (b'5', b'0', 0),
        (b'5', b'1', 0),
        (b'5', b'2', 0),
        (b'5', b'3', 0),
        (b'5', b'4', 0),
        (b'5', b'5', 0),
        (b'5', b'6', 0),
        (b'5', b'7', 0),
        (b'5', b'8', 0),
        (b'5', b'9', 0),
        (b'6', b'.', 0),
        (b'6', b'0', 0),
        (b'7', b'.', 0),
        (b'8', b'.', 0),
        (b'9', b'.', 0),
        (b'A', b'A', 0),
        (b'B', b'B', 0),
        (b'B', b'o', 0),
        (b'C', b'e', 0),
        (b'E', b'n', 0),
        (b'F', b'o', -61),
        (b'H', b'e', 0),
        (b'H', b'o', 0),
        (b'I', b'n', 0),
        (b'L', b'o', 0),
        (b'N', b'o', 0),
        (b'P', b'a', -92),
        (b'R', b'i', 0),
        (b'R', b'o', 0),
        (b'S', b'e', 0),
        (b'T', b'a', -174),
        (b'W', b'o', -123),
        (b'a', b'b', 0),
        (b'a', b'c', 0),
        (b'a', b'f', 0),
        (b'a', b'g', 0),
        (b'a', b'i', 0),
        (b'a', b'm', 0),
        (b'a', b'p', 0),
        (b'a', b'r', 0),
        (b'a', b's', 0),
        (b'b', b'e', 0),
        (b'b', b'l', 0),
        (b'b', b'o', 0),
        (b'c', b'c', 0),
        (b'c', b'e', 0),
        (b'c', b'h', 0),
        (b'c', b't', 0),
        (b'c', b'u', 0),
        (b'd', b' ', 0),
        (b'd', b'a', 0),
        (b'd', b'i', 0),
        (b'd', b'n', 0),
        (b'd', b'y', 0),
        (b'e', b'.', 0),
        (b'e', b'a', 0),
        (b'e', b'c', 0),
        (b'e', b'd', 0),
        (b'e', b'f', 0),
        (b'e', b'l', 0),
        (b'e', b'n', 0),
        (b'e', b'r', 0),
        (b'e', b'x', 0),
        (b'f', b'e', 0),
        (b'f', b'i', 0),
        (b'f', b'o', 0),
        (b'f', b't', 0),
        (b'g', b'e', 0),
        (b'g', b'h', 0),
        (b'g', b'r', 0),
        (b'h', b'e', 0),
        (b'h', b'i', 0),
        (b'h', b't', 0),
        (b'i', b'c', 0),
        (b'i', b'g', 0),
        (b'i', b'l', 0),
        (b'i', b'n', 0),
        (b'i', b'o', 0),
        (b'i', b's', 0),
        (b'i', b't', 0),
        (b'l', b'a', 0),
        (b'l', b'd', 0),
        (b'l', b'e', 0),
        (b'l', b'i', 0),
        (b'l', b'l', 0),
        (b'l', b'o', 0),
        (b'l', b'y', 0),
        (b'm', b'a', 0),
        (b'm', b'b', 0),
        (b'm', b'o', 0),
        (b'n', b'c', 0),
        (b'n', b'd', 0),
        (b'n', b'e', 0),
        (b'n', b'g', 0),
        (b'n', b'l', 0),
        (b'n', b'o', 0),
        (b'n', b't', 0),
        (b'n', b'u', 0),
        (b'o', b' ', 0),
        (b'o', b'c', 0),
        (b'o', b'd', 0),
        (b'o', b'f', 0),
        (b'o', b'l', 0),
        (b'o', b'n', 0),
        (b'o', b'o', 0),
        (b'o', b'r', 0),
        (b'o', b's', 0),
        (b'o', b't', 0),
        (b'o', b'u', 0),
        (b'o', b'w', 0),
        (b'o', b'x', 0),
        (b'p', b'a', 0),
        (b'p', b'h', 0),
        (b'p', b'o', 0),
        (b'p', b'r', 0),
        (b'p', b'y', 0),
        (b'r', b'a', 0),
        (b'r', b'e', 0),
        (b'r', b'i', 0),
        (b'r', b'l', 0),
        (b'r', b'o', 0),
        (b'r', b'y', 0),
        (b's', b'd', 0),
        (b's', b'e', 0),
        (b's', b'i', 0),
        (b's', b'o', 0),
        (b's', b'p', 0),
        (b's', b't', 0),
        (b't', b'a', 0),
        (b't', b'e', 0),
        (b't', b'h', 0),
        (b't', b'i', 0),
        (b't', b'n', 0),
        (b't', b'o', 0),
        (b't', b'r', 0),
        (b't', b's', 0),
        (b't', b'w', 0),
        (b'u', b'm', 0),
        (b'u', b'n', 0),
        (b'u', b'p', 0),
        (b'w', b'i', 0),
        (b'w', b'o', 0),
        (b'w', b's', 0),
        (b'x', b'i', 0),
        (b'y', b',', -113),
    ];
    let mut units = [i16::MIN; 95 * 95];
    let mut index = 0;
    while index < pairs.len() {
        let (left, right, adjustment) = pairs[index];
        units[(left - b' ') as usize * 95 + (right - b' ') as usize] = adjustment;
        index += 1;
    }
    units
};

// Numeric hmtx facts from YuMincho-Regular subsets embedded in the 38
// unchanged Word PDFs in mac-glyph-corpus-20261010; every observed value agrees.
// Zero means unobserved, so those characters retain host measurement. No font
// program or glyph outlines are included. Index is printable ASCII minus 0x20.
const YU_MINCHO_ASCII_UNITS: [u16; 95] = [
    532, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 559, 0, 559, 0, 1110, 1110, 1110, 1110, 1110, 1110, 1110,
    1110, 1110, 1110, 0, 0, 0, 0, 0, 0, 0, 1448, 1411, 1495, 1706, 1313, 1235, 1653, 1686, 733, 0,
    0, 1309, 0, 1645, 1735, 1358, 0, 1405, 1145, 1487, 0, 0, 2013, 0, 0, 0, 0, 0, 0, 0, 0, 0, 981,
    1141, 948, 1149, 1004, 682, 1073, 1159, 588, 0, 1036, 549, 1743, 1190, 1096, 1163, 0, 795, 838,
    709, 1157, 993, 1477, 969, 997, 0, 0, 0, 0, 0,
];

// Numeric hmtx facts from regular Century's MacRomanEncoding subsets in the
// unchanged math, scroll-offset-repro, trust-beneficiary-sale-keep-next and
// wrong-num Word PDFs. All 35 observed advances agree; zero keeps host metrics.
const CENTURY_ASCII_UNITS: [u16; 95] = [
    569, 0, 0, 0, 0, 0, 0, 0, 682, 682, 0, 1241, 0, 0, 569, 0, 1139, 1139, 1139, 1139, 1139, 1139,
    1139, 1139, 1139, 1139, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1479, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1933, 1669,
    0, 0, 0, 1479, 0, 1366, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1139, 1139, 909, 0, 1024, 682, 0,
    1251, 645, 0, 0, 0, 1821, 1251, 1024, 1176, 0, 909, 948, 797, 0, 0, 1593, 0, 0, 0, 0, 0, 0, 0,
];

#[inline]
pub(crate) fn word_ascii_metrics(font_descriptor: &str) -> Option<WordAsciiMetrics> {
    #[cfg(test)]
    WORD_FIXED_ASCII_DESCRIPTOR_LOOKUPS.with(|count| count.set(count.get() + 1));
    WORD_FIXED_ASCII_ADVANCE.with(|cell| {
        let mut cached = cell.borrow_mut();
        let platform = word_layout_platform();
        if let Some((descriptor, cached_platform, advance)) = cached.as_ref() {
            if descriptor == font_descriptor && *cached_platform == platform {
                return *advance;
            }
        }
        let advance = parse_word_ascii_metrics(font_descriptor);
        if let Some((descriptor, cached_platform, cached_advance)) = cached.as_mut() {
            descriptor.clear();
            descriptor.push_str(font_descriptor);
            *cached_platform = platform;
            *cached_advance = advance;
        } else {
            *cached = Some((font_descriptor.to_owned(), platform, advance));
        }
        advance
    })
}

fn parse_word_ascii_metrics(font_descriptor: &str) -> Option<WordAsciiMetrics> {
    #[cfg(test)]
    WORD_FIXED_ASCII_DESCRIPTOR_PARSES.with(|count| count.set(count.get() + 1));
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    let family = family.trim_matches(|ch| ch == '\'' || ch == '"');
    if !size_pt.is_finite() || size_pt <= 0.0 {
        return None;
    }
    let em_px = f64::from(size_pt) * 96.0 / 72.0;
    let (bold, italic) = font_descriptor_style(font_descriptor);
    match family {
        "ＭＳ 明朝"
        | "MS 明朝"
        | "MS Mincho"
        | "ＭＳ明朝"
        | "Mincho"
        | "ＭＳ ゴシック"
        | "MS ゴシック"
        | "MS Gothic"
        | "ＭＳゴシック" => Some(WordAsciiMetrics::Fixed(
            em_px * if bold { 0.52 } else { 0.5 },
        )),
        "游明朝" | "Yu Mincho" | "YuMincho" | "YuMincho-Regular" => Some(if bold {
            WordAsciiMetrics::YuMinchoBold(em_px / 2048.0)
        } else {
            WordAsciiMetrics::YuMincho(em_px / 2048.0)
        }),
        "游ゴシック Light" | "Yu Gothic Light" | "YuGothic-Light" if !bold && !italic => {
            Some(WordAsciiMetrics::YuGothicLight(em_px / 2048.0))
        }
        "Century" if !bold && !italic => Some(WordAsciiMetrics::Century(em_px / 2048.0)),
        "Bookman Old Style" if !bold && !italic => Some(WordAsciiMetrics::Bookman(em_px / 2048.0)),
        "Calibri" if !italic => Some(WordAsciiMetrics::Calibri(em_px / 2048.0, bold)),
        "Aptos" if !bold && !italic && word_layout_platform() == WordLayoutPlatform::Mac => {
            Some(WordAsciiMetrics::Aptos(em_px / 2048.0, false))
        }
        "Aptos Display"
            if !bold && !italic && word_layout_platform() == WordLayoutPlatform::Mac =>
        {
            Some(WordAsciiMetrics::Aptos(em_px / 2048.0, true))
        }
        "Noto Sans" if !bold && !italic => Some(WordAsciiMetrics::NotoSans(em_px / 2048.0)),
        "Noto Sans JP" | "NotoSansJP" if !bold && !italic => {
            Some(WordAsciiMetrics::NotoSansJp(em_px / 1000.0))
        }
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static WORD_FIXED_CJK_DESCRIPTOR_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Full-em CJK faces whose default wide glyphs retain fixed advance in Word.
/// Proportional faces and unknown descriptors keep their host shaping path.
pub(crate) fn word_fixed_east_asian_em_px(font_descriptor: &str) -> Option<f64> {
    #[cfg(test)]
    WORD_FIXED_CJK_DESCRIPTOR_PARSES.with(|count| count.set(count.get() + 1));
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    if !size_pt.is_finite() || size_pt <= 0.0 {
        return None;
    }
    let family = family.trim_matches(|ch| ch == '\'' || ch == '"');
    matches!(
        family,
        "ＭＳ 明朝"
            | "MS 明朝"
            | "MS Mincho"
            | "ＭＳ明朝"
            | "Mincho"
            | "ＭＳ ゴシック"
            | "MS ゴシック"
            | "MS Gothic"
            | "ＭＳゴシック"
            | "游明朝"
            | "Yu Mincho"
            | "YuMincho"
            | "YuMincho-Regular"
    )
    .then_some(f64::from(size_pt) * 96.0 / 72.0)
}

/// Numeric non-ASCII advances verified in original Word Mac PDF subsets.
/// The regular MS Mincho and Yu Gothic Light U+2026 glyphs both have a 1em
/// hmtx advance, despite selecting the HAnsi slot (General Punctuation).
/// Calibri Bold's U+2014 has 1854/2048em in both original section headings.
/// This is an observed face metric, not a Unicode-wide width assumption.
/// Unknown characters/faces/styles retain the host measurement.
#[inline]
pub fn word_observed_unicode_advance_px(font_descriptor: &str, grapheme: &str) -> Option<f64> {
    if !matches!(grapheme, "…" | "—") {
        return None;
    }
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    if !size_pt.is_finite() || size_pt <= 0.0 {
        return None;
    }
    let (bold, italic) = font_descriptor_style(font_descriptor);
    if grapheme == "—" {
        return (bold && !italic && family.trim_matches(|ch| ch == '\'' || ch == '"') == "Calibri")
            .then_some(f64::from(size_pt) * 96.0 / 72.0 * 1854.0 / 2048.0);
    }
    if bold || italic {
        return None;
    }
    match family.trim_matches(|ch| ch == '\'' || ch == '"') {
        "ＭＳ 明朝"
        | "MS 明朝"
        | "MS Mincho"
        | "ＭＳ明朝"
        | "MS-Mincho"
        | "游ゴシック Light"
        | "Yu Gothic Light"
        | "YuGothic-Light" => Some(f64::from(size_pt) * 96.0 / 72.0),
        _ => None,
    }
}

/// Whether a CSS font shorthand asks for a bold and an italic face.
fn font_descriptor_style(font_descriptor: &str) -> (bool, bool) {
    let mut bold = false;
    let mut italic = false;
    for token in font_descriptor.split_whitespace() {
        if token.ends_with("pt") && token[..token.len() - 2].parse::<f32>().is_ok() {
            break;
        }
        match token {
            "bold" | "bolder" | "700" | "800" | "900" => bold = true,
            "italic" | "oblique" => italic = true,
            _ => {}
        }
    }
    (bold, italic)
}

// A leading ASCII size token terminates the original style scan immediately.
// Keep Unicode whitespace and arbitrary prefixes on the existing parser path.
#[inline]
fn synthetic_bold_descriptor_needs_style_scan(font_descriptor: &str) -> bool {
    let bytes = font_descriptor.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return true;
    }
    let end = bytes
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(bytes.len());
    let first = &font_descriptor[..end];
    !(first.is_ascii() && first.ends_with("pt"))
}

/// Extra advance, in px, that Word for Mac gives a glyph it emboldens itself:
/// 1/50 em for a bold run in a face without a bold style (Yu Mincho, MS
/// Mincho, MS Gothic). Word for Mac 16.113 sets bold 10.5pt `第1条（目的）`
/// in either face at a 10.71pt pitch and the 16pt title at 16.32pt; Word for
/// Windows keeps 10.5pt (`wrong-num` Word COM trace), so this is 0 there.
pub fn word_synthetic_bold_extra_advance_px(font_descriptor: &str) -> f64 {
    if word_layout_platform() != WordLayoutPlatform::Mac {
        return 0.0;
    }
    if !synthetic_bold_descriptor_needs_style_scan(font_descriptor) {
        return 0.0;
    }
    let bold = font_descriptor
        .split_whitespace()
        .take_while(|token| !token.ends_with("pt"))
        .any(|token| token == "bold" || token == "700");
    if !bold {
        return 0.0;
    }
    let Some((size_pt, family)) = split_font_descriptor(font_descriptor) else {
        return 0.0;
    };
    let family = family
        .trim()
        .trim_matches(|ch| ch == '\'' || ch == '"')
        .trim();
    let synthesized = matches!(
        family,
        "游明朝"
            | "Yu Mincho"
            | "YuMincho"
            | "ＭＳ 明朝"
            | "MS 明朝"
            | "MS Mincho"
            | "ＭＳ明朝"
            | "ＭＳ Ｐ明朝"
            | "MS P明朝"
            | "ＭＳＰ明朝"
            | "MS PMincho"
            | "Mincho"
            | "ＭＳ ゴシック"
            | "MS ゴシック"
            | "MS Gothic"
            | "ＭＳゴシック"
            | "ＭＳ Ｐゴシック"
            | "MS P Gothic"
            | "ＭＳＰゴシック"
            | "MS PGothic"
    );
    if synthesized {
        f64::from(size_pt) * 96.0 / 72.0 * 0.02
    } else {
        0.0
    }
}

/// Word gives a line set in an East Asian font 1.3 times the font's own
/// ascent+descent, splitting the extra 30% evenly above and below the glyphs
/// and rounding each half to whole font units
/// ([`word_east_asian_font_half_leading_units`]).
/// Measured on Word for Mac 16.113 print output with no document grid, 10.5,
/// 12 and 20pt: MS Mincho/MS Gothic (1.0 em box) → 1.298×, Yu Mincho/Yu Gothic
/// (1.102 em box) → 1.434×, while Times New Roman and Century lines keep their
/// own 1.15×/1.20× box. MS Mincho at 10.5pt is where this meets the older
/// `12pt × 1.1375 = 13.65pt` FE floor, which therefore stays as a floor.
pub const WORD_EAST_ASIAN_FONT_LINE_BOX_MULTIPLIER: f32 = 1.3;

/// The `hhea` ascent, descent and units per em of the East Asian faces Word
/// ships.
///
/// Word's line box depends on the requested face, not on whichever local
/// face or metric-compatible stand-in ends up drawing it, so these are kept as
/// shared layout input instead of being measured from the host. Values come
/// from the fonts bundled with Word for Mac (`yumin.ttf`, `YuGoth*.ttc`,
/// `msmincho.ttc`, `msgothic.ttc`).
pub fn word_east_asian_font_vertical_units(family: &str) -> Option<(f32, f32, f32)> {
    let family = family
        .trim()
        .trim_matches(|ch| ch == '\'' || ch == '"')
        .trim();
    match family {
        // Source-matched Arial Unicode MS subsets in the unchanged Word Mac
        // rich-paste-cr and unicode-segmentation PDFs: hhea 2189/-555,
        // 2048 units/em, no line gap; also present in the macOS font.
        "Arial Unicode MS" | "ArialUnicodeMS"
            if word_layout_platform() == WordLayoutPlatform::Mac =>
        {
            Some((2189.0, 555.0, 2048.0))
        }
        // Noto Sans JP subset in Word for Mac's unchanged Noto JP PDF:
        // hhea 1160/-288, 1000 units/em, no line gap.
        "Noto Sans JP" | "NotoSansJP" if word_layout_platform() == WordLayoutPlatform::Mac => {
            Some((1160.0, 288.0, 1000.0))
        }
        // macOS Hiragino Kaku Gothic ProN W3: hhea 880/-120, 1000 units/em.
        // Word's East Asian box excludes this face's 500-unit Latin line gap.
        "ヒラギノ角ゴ ProN W3" | "Hiragino Kaku Gothic ProN W3" | "Hiragino Kaku Gothic ProN"
            if word_layout_platform() == WordLayoutPlatform::Mac => Some((880.0, 120.0, 1000.0)),
        // Source-matched HG Maru subsets in the unchanged Word Mac math PDF:
        // hhea 220/-36, 256 units/em, no line gap.
        "HG丸ｺﾞｼｯｸM-PRO" | "HGMaruGothicMPRO"
            if word_layout_platform() == WordLayoutPlatform::Mac => Some((220.0, 36.0, 256.0)),
        "游明朝"
        | "Yu Mincho"
        | "YuMincho"
        | "游明朝 Light"
        | "Yu Mincho Light"
        | "游明朝 Demibold"
        | "Yu Mincho Demibold"
        | "游ゴシック"
        | "Yu Gothic"
        | "YuGothic"
        | "游ゴシック Light"
        | "Yu Gothic Light"
        | "游ゴシック Medium"
        | "Yu Gothic Medium" => Some((1802.0, 455.0, 2048.0)),
        "ＭＳ 明朝"
        | "MS 明朝"
        | "MS Mincho"
        | "ＭＳ明朝"
        | "ＭＳ Ｐ明朝"
        | "MS P明朝"
        | "ＭＳＰ明朝"
        | "MS PMincho"
        // Word for Mac 16.113 sets `w:eastAsia="Mincho"` in MS Mincho
        // (`trust-beneficiary-sale-keep-next`).
        | "Mincho"
        | "ＭＳ ゴシック"
        | "MS ゴシック"
        | "MS Gothic"
        | "ＭＳゴシック"
        | "ＭＳ Ｐゴシック"
        | "MS P Gothic"
        | "ＭＳＰゴシック"
        | "MS PGothic" => Some((220.0, 36.0, 256.0)),
        _ => None,
    }
}

/// Word's single line height, in points, for East Asian text drawn with
/// `font_descriptor` (a CSS font shorthand such as `bold 10.50pt '游明朝', …`).
/// `None` when the requested face is not one Word's metrics are known for.
pub fn word_east_asian_font_line_height_pt(font_descriptor: &str) -> Option<f32> {
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    let (ascent, descent, units_per_em) = word_east_asian_font_vertical_units(family)?;
    let half_leading = word_east_asian_font_half_leading_units(ascent, descent);
    Some(size_pt * (ascent + descent + 2.0 * half_leading) / units_per_em)
}

/// The leading Word adds above, and again below, an East Asian face's
/// glyphs: 15% of its ascent+descent, rounded to whole font units. Word for
/// Mac 16.113 advances 30 single-spaced lines of MS Mincho by 1.29688em
/// (332/256: 38 units each side) at every size from 9 to 16pt, and of Yu
/// Mincho/Yu Gothic by 1.43311em (2935/2048: 339 units).
fn word_east_asian_font_half_leading_units(ascent: f32, descent: f32) -> f32 {
    ((ascent + descent) * (WORD_EAST_ASIAN_FONT_LINE_BOX_MULTIPLIER - 1.0) / 2.0).round()
}

/// The ascent and descent, in px, of Word's line box for a glyph drawn with
/// `font_descriptor`: the face's own ascent and descent, each widened by half
/// of the extra 30% (see [`WORD_EAST_ASIAN_FONT_LINE_BOX_MULTIPLIER`]). Word
/// for Mac puts a 10.5pt Yu Mincho baseline 10.97pt below the line top, and
/// an MS Mincho one 10.6pt below. `None` for faces without known metrics.
pub fn word_east_asian_font_vertical_metrics_px(font_descriptor: &str) -> Option<(f64, f64)> {
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    let (ascent, descent, units_per_em) = word_east_asian_font_vertical_units(family)?;
    let half_leading = word_east_asian_font_half_leading_units(ascent, descent);
    let size_px = f64::from(size_pt) * 96.0 / 72.0 / f64::from(units_per_em);
    Some((
        f64::from(ascent + half_leading) * size_px,
        f64::from(descent + half_leading) * size_px,
    ))
}

/// A list label contributes above the baseline; the body's descent remains.
/// Meiryo's source-matched Mac contract subsets supply hhea 2171/-901 at
/// 2048 units/em. Keep this label observation separate from body line sizing.
pub fn word_numbering_font_ascent_px(font_descriptor: &str) -> Option<f64> {
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    if word_layout_platform() == WordLayoutPlatform::Mac
        && matches!(family.trim_matches(['\'', '"']), "Meiryo" | "メイリオ")
    {
        let half_leading = word_east_asian_font_half_leading_units(2171.0, 901.0);
        return Some(f64::from(size_pt) * 96.0 / 72.0 * f64::from(2171.0 + half_leading) / 2048.0);
    }
    word_font_vertical_metrics_px(font_descriptor).map(|(ascent, _)| ascent)
}

/// Combine a first-line label with body metrics without importing its descent.
pub fn line_font_box_with_numbering_ascent(
    body: (f64, f64),
    label_ascent: Option<f64>,
) -> (f64, f64) {
    label_ascent
        .filter(|ascent| ascent.is_finite() && *ascent >= 0.0)
        .map_or(body, |ascent| (body.0.max(ascent), body.1))
}

/// Whether a paragraph's lines take Word's East Asian font line box: the
/// paragraph has auto single spacing, no document grid it snaps to, and Word
/// sizes East Asian lines from their faces ([`word_sizes_east_asian_lines_from_faces`]).
pub fn word_east_asian_font_line_box_applies(
    spacing: Option<&Spacing>,
    snap_to_grid: bool,
    document_uses_east_asian_layout: bool,
) -> bool {
    word_sizes_east_asian_lines_from_faces(document_uses_east_asian_layout)
        && !snap_to_grid
        && !spacing_sets_line_height(spacing)
}

/// Whether Word sizes lines of East Asian text from the face's own 1.3× line
/// box ([`WORD_EAST_ASIAN_FONT_LINE_BOX_MULTIPLIER`]).
///
/// Word for Mac 16.113 always does: with `w:useFELayout` removed (and
/// compatibility mode 15) an 11pt MS Mincho line is still 14.24pt, and
/// `yokohama_page6` (10.5pt MS Mincho, no `useFELayout`) prints 13.65pt
/// lines. Word for Windows only does with `w:useFELayout`: without it
/// `yokohama_page6` only reaches its Word page 3 content at 13.8pt lines.
pub fn word_sizes_east_asian_lines_from_faces(document_uses_east_asian_layout: bool) -> bool {
    document_uses_east_asian_layout || word_layout_platform() == WordLayoutPlatform::Mac
}

/// The single line height, in em, Word gives a line set in a Latin face it
/// ships: the face's `hhea` ascent + descent + line gap, which Word for Mac
/// 16.113 prints for single, 1.15 and 1.5 auto spacing at 11 and 12pt
/// (Calibri 13.44pt at 11pt, Cambria 12.88pt, Arial and Times New Roman
/// 12.64pt, Century 13.2pt). Word for Windows reads the same height from
/// `usWin*` plus the external leading for these faces; Aptos is Mac-only
/// because its Windows metrics give a taller box Word for Windows has not
/// been traced for.
///
/// Like [`word_east_asian_font_vertical_units`], this depends on the
/// requested face, not on the host's stand-in for it.
pub fn word_latin_font_line_height_em(family: &str) -> Option<f32> {
    let (ascent, descent) = word_latin_font_vertical_units(family)?;
    Some((ascent + descent) / 2048.0)
}

/// The ascent (`hhea` ascender + line gap) and descent, in 2048-unit em, of
/// the Latin faces in [`word_latin_font_line_height_em`]. Word sets the line
/// gap above the glyphs: the first baseline of a Times New Roman page is
/// 1912/2048 em below the top margin, not 1825/2048.
fn word_latin_font_vertical_units(family: &str) -> Option<(f32, f32)> {
    let family = family
        .trim()
        .trim_matches(|ch| ch == '\'' || ch == '"')
        .trim();
    match family {
        "Calibri" | "Calibri Light" => Some((1950.0, 550.0)),
        "Cambria" => Some((1946.0, 455.0)),
        "Arial" => Some((1921.0, 434.0)),
        "Times New Roman" => Some((1912.0, 443.0)),
        "Century" => Some((2019.0, 443.0)),
        // Unchanged Word Mac Noto PDF: hhea 2189/-600, no gap, 2048 upem.
        "Noto Sans" if word_layout_platform() == WordLayoutPlatform::Mac => Some((2189.0, 600.0)),
        "Verdana" => Some((2059.0, 430.0)),
        "Aptos" | "Aptos Display" if word_layout_platform() == WordLayoutPlatform::Mac => {
            Some((1923.0, 577.0))
        }
        _ => None,
    }
}

/// The ascent and descent, in px, of Word's line box for a glyph drawn with
/// `font_descriptor`: an East Asian face's
/// ([`word_east_asian_font_vertical_metrics_px`]) or a Latin face's
/// ([`word_latin_font_vertical_units`]). `None` for faces without known
/// metrics, which keep the drawing face's.
pub fn word_font_vertical_metrics_px(font_descriptor: &str) -> Option<(f64, f64)> {
    if let Some(metrics) = word_east_asian_font_vertical_metrics_px(font_descriptor) {
        return Some(metrics);
    }
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    let (ascent, descent) = word_latin_font_vertical_units(family)?;
    let size_px = f64::from(size_pt) * 96.0 / 72.0 / 2048.0;
    Some((f64::from(ascent) * size_px, f64::from(descent) * size_px))
}

/// Glyph ascent/descent without the paragraph leading used by horizontal lines.
/// Upright vertical glyphs occupy an em cell; its baseline uses the face's hhea.
pub fn word_vertical_glyph_metrics_px(font_descriptor: &str) -> Option<(f64, f64)> {
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    if let Some((ascent, descent, units)) = word_east_asian_font_vertical_units(family) {
        let scale = f64::from(size_pt) * 96.0 / 72.0 / f64::from(units);
        return Some((f64::from(ascent) * scale, f64::from(descent) * scale));
    }
    word_font_vertical_metrics_px(font_descriptor)
}

/// [`word_latin_font_line_height_em`] for a CSS font shorthand, in points.
pub fn word_latin_font_line_height_pt(font_descriptor: &str) -> Option<f32> {
    let (size_pt, family) = split_font_descriptor(font_descriptor)?;
    Some(size_pt * word_latin_font_line_height_em(family)?)
}

/// The ascent and descent, in points, Word gives text set in
/// `font_descriptor`'s face: a Latin face's ([`word_latin_font_vertical_units`])
/// or, when `east_asian_faces_size_lines`, an East Asian face's 1.3× box
/// ([`word_east_asian_font_vertical_metrics_px`]) even when it sets Latin
/// text: Word for Mac 16.113 advances `Abc` set in 10.5pt MS Mincho 13.6138pt
/// a line, like `あいう`.
pub fn word_face_vertical_extent_pt(
    font_descriptor: &str,
    east_asian_faces_size_lines: bool,
) -> Option<(f32, f32)> {
    let east_asian = east_asian_faces_size_lines
        .then(|| word_east_asian_font_vertical_metrics_px(font_descriptor))
        .flatten();
    let (ascent_px, descent_px) = match east_asian {
        Some(metrics) => metrics,
        None => {
            let (size_pt, family) = split_font_descriptor(font_descriptor)?;
            let (ascent, descent) = word_latin_font_vertical_units(family)?;
            let size_px = f64::from(size_pt) * 96.0 / 72.0 / 2048.0;
            (f64::from(ascent) * size_px, f64::from(descent) * size_px)
        }
    };
    Some(((ascent_px * 0.75) as f32, (descent_px * 0.75) as f32))
}

/// Word's single line for text set in `font_descriptor`'s face
/// ([`word_face_vertical_extent_pt`]), in points.
pub fn word_face_line_height_pt(
    font_descriptor: &str,
    east_asian_faces_size_lines: bool,
) -> Option<f32> {
    let (ascent, descent) =
        word_face_vertical_extent_pt(font_descriptor, east_asian_faces_size_lines)?;
    Some(ascent + descent)
}

/// How far a character border (`w:bdr`) pushes the line out above and below
/// the run's text, in points: its width plus its `w:space`. Word for Mac
/// 16.113 grows a 7pt Century line from 8.42pt by 1pt (0.5pt border), 3pt
/// (1.5pt border) and 7pt (0.5pt border, 3pt space), and lowers its baseline
/// by the same amount per side.
pub fn word_run_border_extent_pt(properties: Option<&RunProperties>) -> f32 {
    properties
        .and_then(|properties| properties.border.as_ref())
        .filter(|border| border.is_visible())
        .map_or(0.0, |border| {
            border.size_eighths.unwrap_or(0) as f32 / 8.0 + border.space.unwrap_or(0) as f32
        })
}

/// How far a paragraph border edge (`w:pBdr`) pushes the text after it away,
/// in points: its `w:space` plus its width. Word for Mac 16.113 sets a 10pt
/// Calibri paragraph with a 1pt top border 4pt from it 5pt lower, and the one
/// after a 1pt bottom border 6pt from it 7pt lower.
pub fn word_paragraph_border_extent_pt(edge: Option<&BorderEdge>) -> f32 {
    edge.filter(|edge| edge.is_visible()).map_or(0.0, |edge| {
        edge.size_eighths.unwrap_or(0) as f32 / 8.0 + edge.space.unwrap_or(0) as f32
    })
}

/// How far a `w:between` border pushes apart two paragraphs that share a
/// border box ([`word_paragraphs_share_border_box`]), in points: its width plus
/// its `w:space` on both sides. Word for Mac 16.113 leaves 5pt for a 1pt
/// border 2pt away, 9pt for one 4pt away and 3pt for a 3pt one 0pt away.
pub fn word_paragraph_between_border_extent_pt(edge: Option<&BorderEdge>) -> f32 {
    edge.filter(|edge| edge.is_visible()).map_or(0.0, |edge| {
        edge.size_eighths.unwrap_or(0) as f32 / 8.0 + 2.0 * edge.space.unwrap_or(0) as f32
    })
}

/// Whether Word draws two adjacent paragraphs in one border box: both have
/// the same `w:pBdr` and the same side indents. The upper paragraph then
/// leaves out its bottom border and the lower one its top border, with the
/// `w:between` border, if any, between them. Word for Mac 16.113 sets the
/// second of two boxed paragraphs at the plain pitch, and one indented by
/// 0.5in in a box of its own.
pub fn word_paragraphs_share_border_box(
    upper: Option<&ParagraphProperties>,
    lower: Option<&ParagraphProperties>,
) -> bool {
    let (Some(upper), Some(lower)) = (upper, lower) else {
        return false;
    };
    let Some(borders) = upper.borders.as_ref() else {
        return false;
    };
    if lower.borders.as_ref() != Some(borders) {
        return false;
    }
    let sides = |properties: &ParagraphProperties| {
        let indent = properties.indent.as_ref();
        (
            indent.and_then(|indent| indent.start.or(indent.left)),
            indent.and_then(|indent| indent.end.or(indent.right)),
            indent.and_then(|indent| indent.start_chars.or(indent.left_chars)),
            indent.and_then(|indent| indent.end_chars.or(indent.right_chars)),
        )
    };
    sides(upper) == sides(lower)
}

/// The ascent and descent, in points, Word gives a run: the largest of its
/// Latin slot face's ([`word_face_vertical_extent_pt`]) when it draws a visible
/// character outside the East Asian slot, and of its East Asian face's when it
/// draws one in it and `east_asian_faces_size_lines`
/// ([`word_sizes_east_asian_lines_from_faces`]), each pushed out by
/// `border_extent_pt` ([`word_run_border_extent_pt`]).
///
/// `Ok(None)` when the run draws nothing visible; `Err(())` when a face it
/// draws with has no known Word metrics, so the caller keeps its estimate.
#[allow(clippy::result_unit_err)]
pub fn word_run_vertical_extent_pt(
    slots: (bool, bool),
    font_descriptor_latin: &str,
    font_descriptor_east_asia: &str,
    east_asian_faces_size_lines: bool,
    border_extent_pt: f32,
) -> Result<Option<(f32, f32)>, ()> {
    WordRunFaces::new(
        font_descriptor_latin,
        font_descriptor_east_asia,
        east_asian_faces_size_lines,
    )
    .run_vertical_extent_pt(slots, border_extent_pt)
}

/// A run style's Latin and East Asian face metrics, as
/// [`word_run_vertical_extent_pt`] and [`word_east_asian_font_line_height_pt`]
/// read them, parsed from its font descriptors once so a renderer can keep
/// them per style instead of per run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WordRunFaces {
    east_asian_faces_size_lines: bool,
    latin: Option<(f32, f32)>,
    east_asia: Option<(f32, f32)>,
    /// [`word_east_asian_font_line_height_pt`] of the East Asian face.
    pub east_asian_line_pt: Option<f32>,
}

impl WordRunFaces {
    pub fn new(
        font_descriptor_latin: &str,
        font_descriptor_east_asia: &str,
        east_asian_faces_size_lines: bool,
    ) -> Self {
        Self {
            east_asian_faces_size_lines,
            latin: word_face_vertical_extent_pt(font_descriptor_latin, east_asian_faces_size_lines),
            east_asia: east_asian_faces_size_lines
                .then(|| word_face_vertical_extent_pt(font_descriptor_east_asia, true))
                .flatten(),
            east_asian_line_pt: word_east_asian_font_line_height_pt(font_descriptor_east_asia),
        }
    }

    /// [`word_run_vertical_extent_pt`] for a run drawing `slots` in these faces.
    #[allow(clippy::result_unit_err)]
    pub fn run_vertical_extent_pt(
        &self,
        (uses_latin, uses_east_asian): (bool, bool),
        border_extent_pt: f32,
    ) -> Result<Option<(f32, f32)>, ()> {
        let mut extent: Option<(f32, f32)> = None;
        let mut include = |metrics: Option<(f32, f32)>| -> Result<(), ()> {
            let (ascent, descent) = metrics.ok_or(())?;
            extent =
                Some(extent.map_or((ascent, descent), |(a, d)| (a.max(ascent), d.max(descent))));
            Ok(())
        };
        if uses_latin {
            include(self.latin)?;
        }
        if uses_east_asian {
            if !self.east_asian_faces_size_lines {
                return Err(());
            }
            include(self.east_asia)?;
        }
        Ok(extent.map(|(ascent, descent)| (ascent + border_extent_pt, descent + border_extent_pt)))
    }
}

/// Word's single line for a paragraph's runs ([`word_run_vertical_extent_pt`]):
/// the largest ascent plus the largest descent, which need not come from the
/// same run. Word for Mac 16.113 sets 10.5pt MS Mincho `ああ` next to 7pt
/// Century digits with a 1.5pt border 2pt out 15.6pt tall: MS Mincho's
/// 10.58pt ascent over the digits' 5.01pt descent. `None` when any visible run
/// draws with a face Word's metrics are not known for.
pub fn word_tallest_single_line_height_pt(
    runs: impl IntoIterator<Item = Result<Option<(f32, f32)>, ()>>,
) -> Option<f32> {
    let mut extent: Option<(f32, f32)> = None;
    for run in runs {
        if let Some((ascent, descent)) = run.ok()? {
            extent =
                Some(extent.map_or((ascent, descent), |(a, d)| (a.max(ascent), d.max(descent))));
        }
    }
    extent.map(|(ascent, descent)| ascent + descent)
}

/// Whether a paragraph's font-sized lines can change its line height:
/// through the East Asian font line box (`word_font_line_box`, see
/// [`apply_word_east_asian_font_line_height`]) or a multiple of its tallest
/// face's single line (see [`apply_word_font_proportional_line_height`]).
/// Exact, at-least and grid-snapped lines outside the East Asian box need not
/// measure their runs' faces.
pub fn word_font_lines_affect_line_height(
    word_font_line_box: bool,
    spacing: Option<&Spacing>,
    snap_to_grid: bool,
) -> bool {
    word_font_line_box || (!snap_to_grid && proportional_line_multiple(spacing).is_some())
}

/// Word's line height for an auto-spaced line whose faces all have known
/// metrics: `line × single`, where `single_line_height_pt` is the tallest
/// run's single line ([`word_tallest_single_line_height_pt`]). Word for Mac
/// 16.113 advances 1.15, 1.5 and 2.0 multiples of every face's own single
/// line (Calibri 10.5pt at 2.0 → 25.64pt, MS Mincho 12pt at 1.5 → 23.35pt),
/// not of the font size, and puts the extra below the text. Exact, at-least
/// and grid-snapped lines, and lines with an unknown face, keep
/// `line_height_pt`.
pub fn apply_word_font_proportional_line_height(
    line_height_pt: f32,
    spacing: Option<&Spacing>,
    snap_to_grid: bool,
    single_line_height_pt: Option<f32>,
) -> f32 {
    let Some(single) = single_line_height_pt else {
        return line_height_pt;
    };
    if snap_to_grid {
        return line_height_pt;
    }
    let Some(multiple) = proportional_line_multiple(spacing) else {
        return line_height_pt;
    };
    (single * multiple).max(1.0)
}

/// The auto line multiple `spacing` asks for (1.0 when it sets none), or
/// `None` for exact and at-least line spacing.
fn proportional_line_multiple(spacing: Option<&Spacing>) -> Option<f32> {
    let Some(spacing) = spacing else {
        return Some(1.0);
    };
    let Some(line) = spacing.line else {
        return Some(1.0);
    };
    match spacing.line_rule.as_deref() {
        Some("auto") | None => Some(line.max(0.0)),
        _ => None,
    }
}

/// Word's line height, in points, for an empty paragraph whose mark is set in
/// `font_descriptor`'s Latin face, where its metrics are known
/// ([`word_latin_font_line_height_em`]). Word for Mac 16.113 sets the empty 10.5pt paragraph after the table of
/// `system-dev-contract.docx` 12.07pt tall, not on the 12.51pt East Asian
/// empty-paragraph floor ([`WORD_EAST_ASIAN_FE_EMPTY_PARAGRAPH_MIN_FONT_SIZE_PT`]).
pub fn word_empty_paragraph_line_height_pt(font_descriptor: &str) -> Option<f32> {
    word_latin_font_line_height_pt(font_descriptor)
}

/// Whether `text` has a visible character Word draws with the East Asian font
/// slot. Only such runs lend their East Asian face to the line height: a
/// Latin-only run in a document whose East Asian font is Yu Mincho keeps the
/// Latin box, and a run of ideographic spaces before 7pt digits does not raise
/// the line either ([`crate::line_wrap::wrapped_line_max_font_ascent`]).
pub fn text_uses_east_asian_font(text: &str) -> bool {
    let mut buffer = [0_u8; 4];
    text.chars().any(|ch| {
        !ch.is_ascii()
            && !ch.is_whitespace()
            && crate::line_wrap::classify_script(ch.encode_utf8(&mut buffer))
                == crate::line_wrap::ScriptCategory::EastAsia
    })
}

/// Font slots used by a run, including neutral inline note rules. In East
/// Asian documents those rules use the East Asian face's line box, even
/// though they have no text glyphs; ordinary empty runs remain empty.
#[inline]
pub fn run_font_slots(run: &Run, east_asian_document: bool) -> (bool, bool) {
    if run.note_separator.is_none() {
        return text_font_slots(&run.text);
    }
    let (latin, east_asian) = text_font_slots(&run.text);
    (
        latin || !east_asian_document,
        east_asian || east_asian_document,
    )
}

/// Which font slots the visible characters of `text` use, in one pass:
/// `(Latin, East Asian)`: a visible character outside or inside the East Asian
/// slot ([`text_uses_east_asian_font`]). Stops once both are found.
pub fn text_font_slots(text: &str) -> (bool, bool) {
    let mut buffer = [0_u8; 4];
    let (mut latin, mut east_asian) = (false, false);
    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_control() {
            continue;
        }
        let in_east_asian_slot = !ch.is_ascii()
            && (matches!(ch as u32, 0x3040..=0x30FF | 0x4E00..=0x9FFF)
                || crate::line_wrap::classify_script(ch.encode_utf8(&mut buffer))
                    == crate::line_wrap::ScriptCategory::EastAsia);
        if in_east_asian_slot {
            east_asian = true;
        } else {
            latin = true;
        }
        if latin && east_asian {
            break;
        }
    }
    (latin, east_asian)
}

/// Splits a CSS font shorthand into its point size and first family.
fn split_font_descriptor(font_descriptor: &str) -> Option<(f32, &str)> {
    let mut rest = font_descriptor.trim_start();
    loop {
        let token_end = rest.find(char::is_whitespace)?;
        let token = &rest[..token_end];
        rest = rest[token_end..].trim_start();
        if let Some(size) = token.strip_suffix("pt") {
            let size_pt = size.parse::<f32>().ok()?;
            let family = rest.split(',').next()?.trim();
            return (!family.is_empty()).then_some((size_pt, family));
        }
    }
}

/// Whether glyphs set in a Word East Asian face take Word's line box
/// ascent/descent ([`word_east_asian_font_vertical_metrics_px`]): wherever
/// Word sizes the line from its fonts: auto single and multiple spacing, and
/// lines snapped to a document grid (they are then centred in the grid pitch,
/// see [`word_grid_line_centres_text`]). An 11pt Yu Mincho baseline sits the
/// same 11.4–11.6pt below the line top at single, 1.15 and 1.5 spacing.
pub fn word_east_asian_font_metrics_apply(
    spacing: Option<&Spacing>,
    document_uses_east_asian_layout: bool,
) -> bool {
    word_sizes_east_asian_lines_from_faces(document_uses_east_asian_layout)
        && proportional_line_multiple(spacing).is_some()
}

/// Whether Word centres a line's font line box in its grid pitch: lines
/// snapped to an active line grid. Word for Mac 16.113 puts a 10.5pt MS
/// Mincho baseline 12.83pt below the top of an 18pt grid line (half of
/// 18 − 13.61 plus its 10.58pt ascent), a Yu Mincho one 12.59pt, and 16pt text
/// that needs two grid lines 23.87pt below the top of the 36pt pair. Word for
/// Windows has not been traced for this (jlsi/jade#437), so its lines keep
/// their leading below the text.
pub fn word_grid_line_centres_text(snap_to_grid: bool, doc_grid: Option<&DocGrid>) -> bool {
    word_layout_platform() == WordLayoutPlatform::Mac
        && snap_to_grid
        && doc_grid_has_active_line_grid(doc_grid)
}

/// Raises an auto single line height to Word's East Asian font line box when
/// `applies` ([`word_east_asian_font_line_box_applies`]).
pub fn apply_word_east_asian_font_line_height(
    line_height_pt: f32,
    applies: bool,
    east_asian_font_line_height_pt: Option<f32>,
) -> f32 {
    if !applies {
        return line_height_pt;
    }
    match east_asian_font_line_height_pt {
        Some(height) if height.is_finite() && height > line_height_pt => height,
        _ => line_height_pt,
    }
}

fn spacing_sets_line_height(spacing: Option<&Spacing>) -> bool {
    let Some(spacing) = spacing else {
        return false;
    };
    let Some(line) = spacing.line else {
        return false;
    };
    let single_auto =
        matches!(spacing.line_rule.as_deref(), Some("auto") | None) && (line - 1.0).abs() < 0.0001;
    !single_auto
}

pub fn doc_grid_has_active_line_grid(doc_grid: Option<&DocGrid>) -> bool {
    let Some(grid) = doc_grid else {
        return false;
    };
    if grid.line_pitch.is_none() {
        return false;
    }
    match grid.grid_type.as_deref() {
        Some(kind) => !kind.eq_ignore_ascii_case("default"),
        // Mac treats an omitted type as saved, inactive grid metadata: the
        // Noto JP Word PDF uses its 19.761pt face line, not body_height/30.5.
        // Keep the existing Windows legacy FE-grid behavior until traced there.
        None => word_layout_platform() == WordLayoutPlatform::Windows && grid.char_space.is_some(),
    }
}

/// When `w:linePitch` is present but tiny (e.g. `100` → 5pt), Word does not use that value as the
/// real line height; it derives pitch from the section body height and the page line grid. We
/// approximate with **body height / N** where **N** matches typical Japanese A4 body line counts.
///
/// IMPORTANT: `30.5` is still a heuristic derived from the bundled Noto Sans JP fixture and
/// Playwright parity checks. We should remove this fallback once we understand how Word derives the
/// effective section line grid for `w:docGrid w:linePitch="100"` style documents and can compute it
/// directly from parsed DOCX settings instead of tuning a constant.
const WORD_SNAP_LINES_PER_PAGE_DEFAULT: f32 = 30.5;

fn effective_doc_grid_line_pitch_pt(
    doc_grid: Option<&DocGrid>,
    section_content_height_pt: Option<f32>,
    base_font_size: f32,
) -> Option<f32> {
    let raw_pitch = doc_grid.and_then(|g| g.line_pitch)?;
    let min_credible = (base_font_size * 1.12).max(8.0);
    if raw_pitch >= min_credible {
        return Some(raw_pitch);
    }
    let body = section_content_height_pt?;
    if body <= 0.0 {
        return None;
    }
    let derived = body / WORD_SNAP_LINES_PER_PAGE_DEFAULT;
    Some(derived.max(raw_pitch))
}

pub fn doc_grid_has_active_char_grid(doc_grid: Option<&DocGrid>) -> bool {
    let Some(grid) = doc_grid else {
        return false;
    };
    match grid.grid_type.as_deref() {
        Some(kind) if kind.eq_ignore_ascii_case("linesAndChars") => grid.char_space.is_some(),
        Some(kind) if kind.eq_ignore_ascii_case("snapToChars") => true,
        _ => false,
    }
}

/// Character pitch Word uses to convert `w:*Chars` indents when an active
/// character grid applies to the paragraph: the paragraph font size widened by
/// the grid's extra character spacing (`w:docGrid/@w:charSpace`, stored in
/// points). The `indent-with-grid` Word trace records `w:firstLineChars="200"`
/// resolving to 40pt on a 12pt/8pt-spacing grid (字送り20PT).
///
/// Returns `None` when no char grid applies; the point twin recorded by Word
/// alongside the character value stays authoritative there.
pub fn char_grid_indent_char_width_pt(
    doc_grid: Option<&DocGrid>,
    snap_to_grid: bool,
    base_font_size: f32,
) -> Option<f64> {
    if !snap_to_grid || !doc_grid_has_active_char_grid(doc_grid) {
        return None;
    }
    let char_space = doc_grid.and_then(|grid| grid.char_space).unwrap_or(0.0);
    Some((base_font_size + char_space.max(0.0)) as f64)
}

pub fn resolve_effective_snap_to_grid(
    paragraph_snap_to_grid: Option<bool>,
    document_snap_to_grid_default: bool,
    doc_grid: Option<&DocGrid>,
) -> bool {
    if let Some(value) = paragraph_snap_to_grid {
        return value;
    }
    let section_grid_is_active =
        doc_grid_has_active_line_grid(doc_grid) || doc_grid_has_active_char_grid(doc_grid);
    if document_snap_to_grid_default {
        return section_grid_is_active;
    }
    section_grid_is_active
}

/// Resolves paragraph line height in points.
///
/// `section_content_height_pt` is the section body height in points (page height minus top and
/// bottom margins). It is used when `w:linePitch` is a placeholder and Word snaps to the
/// document grid so we can derive a credible line pitch from the body area.
pub fn resolve_line_height_points(
    base_font_size: f32,
    spacing: Option<&Spacing>,
    doc_grid: Option<&DocGrid>,
    snap_to_grid: bool,
    section_content_height_pt: Option<f32>,
    implicit_line_box_pt: Option<f32>,
) -> f32 {
    resolve_line_height_points_with_default_multiplier(
        base_font_size,
        spacing,
        doc_grid,
        snap_to_grid,
        section_content_height_pt,
        implicit_line_box_pt,
        WORD_DEFAULT_LINE_SPACING_MULTIPLIER,
    )
}

pub fn resolve_line_height_points_with_default_multiplier(
    base_font_size: f32,
    spacing: Option<&Spacing>,
    doc_grid: Option<&DocGrid>,
    snap_to_grid: bool,
    section_content_height_pt: Option<f32>,
    implicit_line_box_pt: Option<f32>,
    default_line_spacing_multiplier: f32,
) -> f32 {
    let mut line_height = base_font_size;
    let mut spacing_explicit = false;
    let single_auto_spacing = spacing.is_some_and(|value| {
        value.line_rule.as_deref() == Some("auto")
            && value.line.is_some_and(|line| (line - 1.0).abs() < 0.0001)
    });

    if let Some(spacing) = spacing {
        if let Some(rule) = spacing.line_rule.as_deref() {
            match rule {
                "atLeast" => {
                    if let Some(line) = spacing.line {
                        line_height = line_height.max(line);
                        spacing_explicit = true;
                    }
                }
                "exact" | "exactly" => {
                    if let Some(line) = spacing.line {
                        line_height = line.max(1.0);
                        spacing_explicit = true;
                    }
                }
                "auto" => {
                    if let Some(line) = spacing.line {
                        if !single_auto_spacing {
                            let candidate = base_font_size * line;
                            line_height = candidate.max(1.0);
                            spacing_explicit = true;
                        }
                    }
                }
                _ => {
                    if let Some(line) = spacing.line {
                        let candidate = base_font_size * line;
                        line_height = candidate.max(1.0);
                        spacing_explicit = true;
                    }
                }
            }
        } else if let Some(line) = spacing.line {
            let candidate = base_font_size * line;
            line_height = candidate.max(1.0);
            spacing_explicit = true;
        }
    }

    if !spacing_explicit {
        let implicit = (base_font_size * default_line_spacing_multiplier).max(base_font_size);
        line_height = line_height.max(implicit);
        if snap_to_grid {
            if let Some(line_box) =
                implicit_line_box_pt.filter(|value| value.is_finite() && *value > 0.0)
            {
                // Keep the active font's full line box as a floor only for grid-snapped documents.
                // Applying it to ordinary paragraphs over-expands legacy fixtures like yokohama_page6.
                line_height = line_height.max(line_box);
            }
        }
    }

    let exact_spacing = spacing
        .and_then(|value| value.line_rule.as_deref())
        .is_some_and(|rule| matches!(rule, "exact" | "exactly"));
    if snap_to_grid && !exact_spacing {
        if let Some(pitch) =
            effective_doc_grid_line_pitch_pt(doc_grid, section_content_height_pt, base_font_size)
        {
            if pitch > 0.0 {
                // Word keeps document-grid line pitch as a floor for auto/atLeast paragraphs when
                // snap-to-grid applies to an active line grid.
                line_height = line_height.max(pitch);
                if word_layout_platform() == WordLayoutPlatform::Mac
                    && doc_grid_has_active_line_grid(doc_grid)
                {
                    // A font box taller than one grid step occupies the next
                    // whole step. Word's 14pt MS Mincho box is 18.15625pt, so
                    // an 18pt grid gives it 36pt, centred by the line geometry.
                    line_height = (line_height / pitch).ceil() * pitch;
                }
            }
        }
    }

    line_height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preceding_after_crosses_only_mac_section_starts_and_defers_context_checks() {
        let calls = std::cell::Cell::new(0);
        let context = |section_start| {
            let calls = &calls;
            move || {
                calls.set(calls.get() + 1);
                section_start
            }
        };
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let trailing = Some((0, 0, 100.0, 13.333333));
        assert_eq!(
            resolve_preceding_body_spacing_after(trailing, 0, 0, 100.0, context(false)),
            13.333333
        );
        assert_eq!(
            resolve_preceding_body_spacing_after(None, 1, 0, 0.0, context(true)),
            0.0
        );
        assert_eq!(
            calls.get(),
            0,
            "ordinary continuity and missing snapshots need no section/flow query"
        );
        assert_eq!(
            resolve_preceding_body_spacing_after(trailing, 1, 0, 0.0, context(false)),
            0.0
        );
        assert_eq!(
            resolve_preceding_body_spacing_after(trailing, 1, 0, 0.0, context(true)),
            13.333333
        );
        assert_eq!(calls.get(), 2);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(
            resolve_preceding_body_spacing_after(trailing, 1, 0, 0.0, || panic!(
                "Windows must not query Mac section context"
            )),
            0.0
        );
    }

    #[test]
    fn calibri_bold_em_dash_uses_original_pdf_hmtx_without_extending_other_faces() {
        let expected = 15.0 * 96.0 / 72.0 * 1854.0 / 2048.0;
        assert_eq!(
            word_observed_unicode_advance_px("bold 15.00pt Calibri", "—"),
            Some(expected)
        );
        for descriptor in [
            "15.00pt Calibri",
            "italic bold 15.00pt Calibri",
            "bold 15.00pt Arial",
        ] {
            assert_eq!(word_observed_unicode_advance_px(descriptor, "—"), None);
        }
        assert_eq!(
            word_observed_unicode_advance_px("bold 15.00pt Calibri", "–"),
            None
        );
    }

    #[test]
    fn mac_body_before_spacing_preserves_section_and_inline_break_contracts() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert_eq!(
            resolve_body_paragraph_spacing_before(32.0, 0.0, true, false, false),
            0.0
        );
        assert_eq!(
            resolve_body_paragraph_spacing_before(32.0, 0.0, true, true, false),
            32.0
        );
        assert_eq!(
            resolve_body_paragraph_spacing_before(32.0, 0.0, true, false, true),
            32.0
        );
        assert_eq!(
            resolve_body_paragraph_spacing_before(32.0, 8.0, false, false, false),
            24.0
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(
            resolve_body_paragraph_spacing_before(32.0, 8.0, true, false, false),
            24.0
        );
    }

    #[test]
    fn mac_grid_sizes_each_lines_face_instead_of_the_paragraph_maximum() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let grid = doc_grid("linesAndChars", Some(14.95), Some(3.0));
        let sizing = WordGridLineSizing::new(None, true, Some(&grid), Some(650.0), 11.0).unwrap();
        // numbering-restart's MS Mincho first line uses one grid step;
        // the following Yu Gothic line needs two, in the same paragraph.
        for (face, steps) in [("11.00pt MS Mincho", 1.0), ("11.00pt Yu Gothic", 2.0)] {
            let single =
                word_east_asian_font_line_height_pt(face).map(|pt| f64::from(pt) * 96.0 / 72.0);
            let actual = sizing.line_height_px(single, 29.9 * 96.0 / 72.0);
            assert!(
                (actual - 14.95 * steps * 96.0 / 72.0).abs() < 1e-5,
                "{face}: {actual}"
            );
        }
        assert_eq!(sizing.line_height_px(None, 40.0), 40.0);
        assert_eq!(sizing.line_height_px(Some(f64::NAN), 40.0), 40.0);
        let tiny = doc_grid("lines", Some(5.0), None);
        let effective =
            WordGridLineSizing::new(None, true, Some(&tiny), Some(610.0), 11.0).unwrap();
        assert!((effective.line_height_px(Some(18.0), 40.0) - 20.0 * 96.0 / 72.0).abs() < 1e-5);
        let exact = Spacing {
            line: Some(12.0),
            line_rule: Some("exact".into()),
            ..Spacing::default()
        };
        assert!(
            WordGridLineSizing::new(Some(&exact), true, Some(&grid), Some(650.0), 11.0).is_none()
        );
        assert!(WordGridLineSizing::new(None, false, Some(&grid), Some(650.0), 11.0).is_none());
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!(WordGridLineSizing::new(None, true, Some(&grid), Some(650.0), 11.0).is_none());
    }

    fn bordered(space: u32, indent_left: Option<f32>) -> ParagraphProperties {
        let edge = BorderEdge {
            val: Some("single".to_string()),
            size_eighths: Some(8),
            space: Some(space),
            color: None,
        };
        ParagraphProperties {
            borders: Some(jade_docx_reader::document::ParagraphBorders {
                top: Some(edge.clone()),
                bottom: Some(edge),
                ..Default::default()
            }),
            indent: indent_left.map(|left| jade_docx_reader::document::Indent {
                left: Some(left),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn mincho_takes_ms_mincho_line_box() {
        assert_eq!(
            word_east_asian_font_vertical_units("Mincho"),
            word_east_asian_font_vertical_units("ＭＳ 明朝")
        );
        assert!(word_east_asian_font_vertical_units("Mincho").is_some());
    }

    #[test]
    fn paragraph_border_extent_is_space_plus_width() {
        let properties = bordered(4, None);
        let borders = properties.borders.as_ref().unwrap();
        assert_eq!(word_paragraph_border_extent_pt(borders.top.as_ref()), 5.0);
        assert_eq!(word_paragraph_border_extent_pt(None), 0.0);
        let between = BorderEdge {
            val: Some("single".to_string()),
            size_eighths: Some(8),
            space: Some(2),
            color: None,
        };
        assert_eq!(word_paragraph_between_border_extent_pt(Some(&between)), 5.0);
        let nil = BorderEdge {
            val: Some("nil".to_string()),
            ..between
        };
        assert_eq!(word_paragraph_border_extent_pt(Some(&nil)), 0.0);
    }

    #[test]
    fn paragraphs_share_border_box_only_with_same_borders_and_indents() {
        let boxed = bordered(4, None);
        assert!(word_paragraphs_share_border_box(Some(&boxed), Some(&boxed)));
        assert!(!word_paragraphs_share_border_box(
            Some(&boxed),
            Some(&bordered(4, Some(36.0)))
        ));
        assert!(!word_paragraphs_share_border_box(
            Some(&boxed),
            Some(&bordered(6, None))
        ));
        let plain = ParagraphProperties::default();
        assert!(!word_paragraphs_share_border_box(
            Some(&plain),
            Some(&plain)
        ));
        assert!(!word_paragraphs_share_border_box(Some(&boxed), None));
    }

    fn doc_grid(grid_type: &str, line_pitch: Option<f32>, char_space: Option<f32>) -> DocGrid {
        DocGrid {
            grid_type: Some(grid_type.to_string()),
            line_pitch,
            char_space,
        }
    }

    #[test]
    fn default_doc_grid_type_is_inactive() {
        let grid = doc_grid("default", Some(24.0), Some(1.0));
        assert!(!doc_grid_has_active_line_grid(Some(&grid)));
        assert!(!doc_grid_has_active_char_grid(Some(&grid)));
    }

    #[test]
    fn missing_doc_grid_type_with_line_pitch_only_is_inactive() {
        let grid = DocGrid {
            grid_type: None,
            line_pitch: Some(5.0),
            char_space: None,
        };
        assert!(!doc_grid_has_active_line_grid(Some(&grid)));
        assert!(!resolve_effective_snap_to_grid(None, false, Some(&grid)));
        assert!(!resolve_effective_snap_to_grid(None, true, Some(&grid)));
    }

    #[test]
    fn mac_missing_doc_grid_type_does_not_activate_saved_grid_settings() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        for pitch in [5.0, 18.0, 24.0] {
            let grid = DocGrid {
                grid_type: None,
                line_pitch: Some(pitch),
                char_space: Some(1.5),
            };
            assert!(!doc_grid_has_active_line_grid(Some(&grid)));
            assert!(!resolve_effective_snap_to_grid(None, false, Some(&grid)));
            assert!(!resolve_effective_snap_to_grid(None, true, Some(&grid)));
        }
        assert!(doc_grid_has_active_line_grid(Some(&doc_grid(
            "lines",
            Some(24.0),
            None
        ))));
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn missing_doc_grid_type_with_line_and_character_pitch_keeps_legacy_fe_grid() {
        let grid = DocGrid {
            grid_type: None,
            line_pitch: Some(5.0),
            char_space: Some(1.5),
        };
        assert!(doc_grid_has_active_line_grid(Some(&grid)));
        assert!(resolve_effective_snap_to_grid(None, false, Some(&grid)));
    }

    #[test]
    fn effective_snap_to_grid_defaults_on_when_grid_exists() {
        let grid = doc_grid("lines", Some(24.0), None);
        assert!(resolve_effective_snap_to_grid(None, false, Some(&grid)));
        assert!(resolve_effective_snap_to_grid(None, true, Some(&grid)));
        assert!(!resolve_effective_snap_to_grid(
            Some(false),
            false,
            Some(&grid)
        ));
    }

    #[test]
    fn default_doc_grid_does_not_enable_snap_by_itself() {
        let grid = doc_grid("default", Some(5.0), Some(1.5));
        assert!(!resolve_effective_snap_to_grid(None, false, Some(&grid)));
    }

    #[test]
    fn mac_tall_font_line_uses_whole_document_grid_steps() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let grid = doc_grid("lines", Some(18.0), None);
        // Word's 18pt grid puts 14pt MS Mincho in two grid steps: its
        // single line is 18.15625pt. The scroll-offset heading baseline is
        // 122.205pt, about nine points lower than a one-step line.
        for (size, expected) in [(10.5, 18.0), (14.0, 36.0), (16.0, 36.0)] {
            let descriptor = format!("{size:.2}pt 'MS Mincho'");
            let single = word_east_asian_font_line_height_pt(&descriptor);
            let height =
                resolve_line_height_points(size, None, Some(&grid), true, Some(650.0), single);
            assert!(
                (height - expected).abs() < 1e-5,
                "{size}: {height} vs {expected}"
            );
        }
        let exact = Spacing {
            line: Some(12.0),
            line_rule: Some("exact".into()),
            ..Spacing::default()
        };
        assert_eq!(
            resolve_line_height_points(
                16.0,
                Some(&exact),
                Some(&grid),
                true,
                Some(650.0),
                Some(20.75)
            ),
            12.0
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let height =
            resolve_line_height_points(16.0, None, Some(&grid), true, Some(650.0), Some(20.75));
        assert!((height - 20.75).abs() < 1e-5);
    }

    #[test]
    fn line_pitch_applies_to_auto_spacing_with_active_grid() {
        let grid = doc_grid("lines", Some(24.0), None);
        let line_height = resolve_line_height_points(10.0, None, Some(&grid), true, None, None);
        assert!((line_height - 24.0).abs() < 1e-6);
    }

    #[test]
    fn exact_spacing_overrides_doc_grid_pitch() {
        let grid = doc_grid("lines", Some(24.0), None);
        let spacing = Spacing {
            line: Some(12.0),
            line_rule: Some("exact".to_string()),
            ..Spacing::default()
        };
        let line_height =
            resolve_line_height_points(10.0, Some(&spacing), Some(&grid), true, None, None);
        assert!((line_height - 12.0).abs() < 1e-6);
    }

    #[test]
    fn active_line_grid_derives_pitch_from_section_body_when_placeholder_is_tiny() {
        let grid = doc_grid("lines", Some(5.0), Some(1.5));
        assert!(resolve_effective_snap_to_grid(None, false, Some(&grid)));
        let line_height =
            resolve_line_height_points(10.5, None, Some(&grid), true, Some(607.7), None);
        assert!((line_height - (607.7 / 30.5)).abs() < 0.05);
    }

    #[test]
    fn implicit_line_box_is_ignored_without_grid_snap() {
        let line_height = resolve_line_height_points(10.5, None, None, false, None, Some(15.0));
        assert_eq!(line_height, 10.5 * WORD_DEFAULT_LINE_SPACING_MULTIPLIER);
    }

    #[test]
    fn synthetic_bold_size_prefix_skips_only_the_original_empty_style_scan() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        for descriptor in [
            "10.50pt '游明朝'",
            "10.50pt\t'游明朝'",
            "0pt bold '游明朝'",
            "700pt '游明朝'",
            "10.50pt",
        ] {
            assert!(!synthetic_bold_descriptor_needs_style_scan(descriptor));
            assert_eq!(word_synthetic_bold_extra_advance_px(descriptor), 0.0);
        }
        for (descriptor, expected) in [
            ("bold 10.50pt '游明朝'", 0.28),
            ("700 10.50pt '游明朝'", 0.28),
            ("bold\u{2003}10.50pt '游明朝'", 0.28),
            ("10.50pt\u{2003}bold '游明朝'", 0.0),
            ("\t10.50pt '游明朝'", 0.0),
            ("italic 10.50pt '游明朝'", 0.0),
            ("800 10.50pt '游明朝'", 0.0),
            ("not a font", 0.0),
        ] {
            assert!(synthetic_bold_descriptor_needs_style_scan(descriptor));
            assert!((word_synthetic_bold_extra_advance_px(descriptor) - expected).abs() < 1e-9);
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(
            word_synthetic_bold_extra_advance_px("bold 10.50pt '游明朝'"),
            0.0
        );
    }

    #[test]
    fn word_for_mac_widens_synthetic_bold_only() {
        let bold_yu = "bold 10.50pt '游明朝', 'Yu Mincho', serif";
        assert_eq!(word_synthetic_bold_extra_advance_px(bold_yu), 0.0);
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert!((word_synthetic_bold_extra_advance_px(bold_yu) - 0.28).abs() < 1e-9);
        assert!(
            (word_synthetic_bold_extra_advance_px("bold 16.00pt 'ＭＳ 明朝'")
                - 16.0 * 4.0 / 3.0 * 0.02)
                .abs()
                < 1e-9
        );
        assert_eq!(
            word_synthetic_bold_extra_advance_px("10.50pt '游明朝'"),
            0.0
        );
        // Times New Roman and Yu Gothic have real bold faces.
        assert_eq!(
            word_synthetic_bold_extra_advance_px("bold 10.50pt 'Times New Roman'"),
            0.0
        );
        assert_eq!(
            word_synthetic_bold_extra_advance_px("bold 10.50pt '游ゴシック'"),
            0.0
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn note_rules_use_the_document_font_slot_without_inventing_text() {
        use jade_docx_reader::document::NoteSeparatorKind;
        let mut run: Run = serde_json::from_value(serde_json::json!({
            "text": "", "properties": null, "image": null,
            "text_box": null, "field": null
        }))
        .unwrap();
        assert_eq!(run.note_separator, None, "old run JSON remains valid");
        assert_eq!(run_font_slots(&run, true), (false, false));
        for kind in [
            NoteSeparatorKind::Separator,
            NoteSeparatorKind::ContinuationSeparator,
        ] {
            run.note_separator = Some(kind);
            assert_eq!(run_font_slots(&run, false), (true, false));
            assert_eq!(run_font_slots(&run, true), (false, true));
            run.text = "A".into();
            assert_eq!(run_font_slots(&run, true), (true, true));
            run.text.clear();
        }
        assert!(run.text.is_empty());
    }

    #[test]
    fn arial_unicode_east_asian_line_uses_the_requested_mac_font_box() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let descriptor = "12.00pt 'Arial Unicode MS', 'Arial', sans-serif";
        // rich-paste-cr's source selects this East Asian face at 12pt. The
        // unchanged Word PDF subset and macOS font both have hhea 2189/-555
        // and 2048 units/em. The existing rounded half-leading is 412 units.
        assert_eq!(
            word_east_asian_font_vertical_units("Arial Unicode MS"),
            Some((2189.0, 555.0, 2048.0))
        );
        assert_eq!(
            word_east_asian_font_line_height_pt(descriptor),
            Some(20.90625)
        );
        let (ascent, descent) = word_east_asian_font_vertical_metrics_px(descriptor).unwrap();
        assert!((ascent * 0.75 - 15.240234375).abs() < 1e-9);
        assert!((descent * 0.75 - 5.666015625).abs() < 1e-9);
        assert_eq!(
            word_empty_paragraph_line_height_pt("12.00pt 'Times New Roman'"),
            Some(13.798828)
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(
            word_east_asian_font_vertical_units("Arial Unicode MS"),
            None
        );
        assert_eq!(
            word_east_asian_font_vertical_units("Unregistered CJK face"),
            None
        );
    }

    #[test]
    fn empty_paragraph_takes_its_mark_latin_font_line() {
        let tnr = word_empty_paragraph_line_height_pt("10.50pt 'Times New Roman', serif")
            .expect("Times New Roman metrics");
        assert!((tnr - 12.07).abs() < 0.01, "{tnr}");
        assert_eq!(
            word_empty_paragraph_line_height_pt("10.50pt 'Noto Serif'"),
            None
        );
    }

    #[test]
    fn word_latin_font_line_follows_the_requested_face() {
        // Word for Mac 16.113 single-spaced pitches, quantized to 0.24pt.
        for (descriptor, word_pt) in [
            ("11.00pt Calibri, sans-serif", 13.44),
            ("11.00pt Cambria, serif", 12.88),
            ("11.00pt Arial, sans-serif", 12.64),
            ("11.00pt 'Times New Roman', serif", 12.64),
            ("12.00pt Century, serif", 14.40),
        ] {
            let height = word_latin_font_line_height_pt(descriptor).expect(descriptor);
            assert!((height - word_pt).abs() < 0.13, "{descriptor}: {height}");
        }
        assert_eq!(word_latin_font_line_height_pt("11.00pt Aptos"), None);
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let aptos = word_latin_font_line_height_pt("11.00pt Aptos").expect("Aptos on Mac");
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!((aptos - 13.44).abs() < 0.13, "{aptos}");
    }

    #[test]
    fn mac_aptos_display_uses_the_original_word_pdf_vertical_metrics() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let height = word_latin_font_line_height_pt("20.00pt 'Aptos Display'");
        let metrics = word_font_vertical_metrics_px("20.00pt 'Aptos Display'");
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let expected_height = 20.0 * (1923.0 + 577.0) / 2048.0;
        assert!(
            height.is_some(),
            "original Word font hhea is1923/-577,2048upem"
        );
        assert!((height.unwrap() - expected_height).abs() < 1e-5);
        let (ascent, descent) = metrics.expect("original Aptos Display metrics");
        assert!((ascent - 20.0 * 96.0 / 72.0 * 1923.0 / 2048.0).abs() < 1e-8);
        assert!((descent - 20.0 * 96.0 / 72.0 * 577.0 / 2048.0).abs() < 1e-8);
        assert_eq!(
            word_latin_font_line_height_pt("20.00pt 'Aptos Display'"),
            None
        );
    }

    #[test]
    fn word_auto_space_follows_the_latin_face_average_width() {
        assert_eq!(word_auto_space_metrics("10.50pt Century"), None);
        set_word_layout_platform(WordLayoutPlatform::Mac);
        // Word for Mac 16.113, 10.5pt, MS Mincho East Asian text: the gap
        // after a letter or digit, before East Asian text.
        for (descriptor, word_pt) in [
            ("10.50pt 'Times New Roman'", 2.07),
            ("10.50pt Arial", 2.29),
            ("10.50pt Century", 2.41),
            ("10.50pt Calibri", 2.70),
            ("10.50pt Cambria", 3.19),
            ("10.50pt 'ＭＳ 明朝'", 2.60),
            ("10.50pt '游明朝'", 2.59),
            ("12.00pt 'Times New Roman'", 2.41),
        ] {
            let metrics = word_auto_space_metrics(descriptor).expect(descriptor);
            let trailing = metrics.trailing_gap_px(false) * 0.75;
            let balanced = metrics.trailing_gap_px(true) * 0.75;
            assert!((balanced - metrics.leading_gap_px() * 0.75).abs() < 1e-9);
            assert!(
                (trailing - word_pt).abs() < 0.05,
                "{descriptor}: {trailing}"
            );
        }
        let tnr = word_auto_space_metrics("12.00pt 'Times New Roman'").unwrap();
        assert!((tnr.leading_gap_px() * 0.75 - 3.0).abs() < 1e-9);
        assert_eq!(word_auto_space_metrics("10.50pt 'Noto Serif'"), None);
        let bold = word_auto_space_metrics("bold 10.50pt 'Times New Roman'").unwrap();
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!((bold.trailing_share - 874.0 / 1024.0).abs() < 1e-9);
    }

    #[test]
    fn aptos_regular_auto_space_uses_the_official_average_width() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let metrics = word_auto_space_metrics("12.00pt Aptos");
        let original_origin_delta = 78.229 - 72.0;
        let natural_i_advance_pt = 12.0 * 489.0 / 2048.0;
        let original_gap_pt = original_origin_delta - natural_i_advance_pt;
        let actual = metrics.map(|value| value.trailing_gap_px(false) * 0.75);
        let balanced = metrics.map(|value| value.trailing_gap_px(true) * 0.75);
        let bold = word_auto_space_metrics("bold 12.00pt Aptos");
        let italic = word_auto_space_metrics("italic 12.00pt Aptos");
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!(
            (actual.expect("corroborated Aptos average width") - original_gap_pt).abs() < 0.001
        );
        assert_eq!(balanced, Some(3.0));
        assert_eq!(bold, None);
        assert_eq!(italic, None);
        assert_eq!(word_auto_space_metrics("12.00pt Aptos"), None);
    }

    #[test]
    fn word_latin_line_sets_the_line_gap_above_the_glyphs() {
        // Word for Mac 16.113 puts the first 12pt Times New Roman baseline
        // 10.96–11.20pt below the top margin (0.24pt quantization): 1912/2048 em.
        let (ascent, descent) =
            word_font_vertical_metrics_px("12.00pt 'Times New Roman'").expect("TNR metrics");
        assert!((ascent * 0.75 - 11.203).abs() < 0.01, "{ascent}");
        assert!(
            ((ascent + descent) * 0.75 - 13.799).abs() < 0.01,
            "{descent}"
        );
        let (ascent, _) = word_font_vertical_metrics_px("10.50pt 'ＭＳ 明朝'").expect("MS Mincho");
        assert!((ascent * 0.75 - 10.582).abs() < 0.01, "{ascent}");
        assert_eq!(word_font_vertical_metrics_px("12.00pt 'Noto Serif'"), None);
    }

    #[test]
    fn word_run_line_takes_the_faces_its_characters_use() {
        let latin = "11.00pt Calibri";
        let east_asia = "11.00pt '游明朝'";
        let height =
            |run: Result<Option<(f32, f32)>, ()>| word_tallest_single_line_height_pt([run]);
        let run = |text: &str, latin: &str, faces: bool| {
            word_run_vertical_extent_pt(text_font_slots(text), latin, east_asia, faces, 0.0)
        };
        let calibri = height(run("Abc", latin, true)).unwrap();
        assert!((calibri - 13.43).abs() < 0.01, "{calibri}");
        // Word for Mac: あいう in 11pt Yu Mincho advances 15.7655pt.
        let yu = height(run("あいう", latin, true)).unwrap();
        assert!((yu - 15.7655).abs() < 0.01, "{yu}");
        assert_eq!(run("あいう", latin, false), Err(()));
        assert_eq!(height(run("あいAbc", latin, true)), Some(yu));
        assert_eq!(run("Abc", "11.00pt 'Noto Sans'", true), Err(()));
        // Latin text set in an East Asian face takes that face's box.
        assert_eq!(height(run("Abc", east_asia, true)), Some(yu));
        assert_eq!(run("Abc", east_asia, false), Err(()));
        // Whitespace, ideographic spaces included, sizes nothing.
        assert_eq!(run("  　", latin, true), Ok(None));
        assert_eq!(
            word_tallest_single_line_height_pt([run("Abc", latin, true), Err(())]),
            None
        );
    }

    #[test]
    fn word_line_takes_the_largest_ascent_and_descent() {
        // Word for Mac 16.113, trust-beneficiary styles: 10.5pt MS Mincho
        // `ああ` beside 7pt Century digits with a 1.5pt border 2pt out sets a
        // 15.6pt line; the bordered digits alone 15.42pt, without the border
        // 8.42pt.
        let border = |size_eighths, space| RunProperties {
            border: Some(jade_docx_reader::document::BorderEdge {
                val: Some("single".to_string()),
                size_eighths: Some(size_eighths),
                space: Some(space),
                color: None,
            }),
            ..RunProperties::default()
        };
        let digits = |extent| {
            word_run_vertical_extent_pt(
                text_font_slots("2024"),
                "7.00pt Century",
                "7.00pt 'ＭＳ 明朝'",
                true,
                extent,
            )
        };
        let plain = word_tallest_single_line_height_pt([digits(0.0)]).unwrap();
        assert!((plain - 8.42).abs() < 0.01, "{plain}");
        let extent = word_run_border_extent_pt(Some(&border(12, 2)));
        assert_eq!(extent, 3.5);
        let bordered = word_tallest_single_line_height_pt([digits(extent)]).unwrap();
        assert!((bordered - 15.42).abs() < 0.01, "{bordered}");
        let kana = word_run_vertical_extent_pt(
            text_font_slots("ああ"),
            "10.50pt Century",
            "10.50pt 'ＭＳ 明朝'",
            true,
            0.0,
        );
        let mixed = word_tallest_single_line_height_pt([kana, digits(extent)]).unwrap();
        assert!((mixed - 15.6).abs() < 0.05, "{mixed}");
        assert_eq!(word_run_border_extent_pt(Some(&border(4, 0))), 0.5);
        assert_eq!(word_run_border_extent_pt(None), 0.0);
    }

    #[test]
    fn observed_yu_mincho_pairs_keep_source_resolved_word_origins() {
        let regular = word_ascii_metrics("10.50pt 'Yu Mincho'").expect("Yu Mincho metrics");
        for (pair, units) in [
            (b"Ro", 0.0),
            (b"ow", 0.0),
            (b"No", 0.0),
            (b"te", 0.0),
            (b"Fo", -61.0),
            (b"y,", -113.0),
        ] {
            assert_eq!(
                regular.pair_kerning_px(pair[0], pair[1]),
                Some(14.0 * units / 2048.0)
            );
        }
        assert_eq!(regular.pair_kerning_px(b'~', b'~'), None);
        let bold = word_ascii_metrics("bold 10.50pt 'Yu Mincho'").expect("bold metrics");
        assert_eq!(
            bold.pair_kerning_px(b'R', b'o'),
            None,
            "the new audit contains only regular faces"
        );
        assert_eq!(
            bold.pair_kerning_px(b'W', b'o'),
            Some(-14.0 * 123.0 / 2048.0)
        );
    }

    #[test]
    fn word_auto_multiple_scales_the_face_line() {
        let multiple = |line: f32| Spacing {
            line: Some(line),
            line_rule: Some("auto".to_string()),
            ..Spacing::default()
        };
        // Word for Mac 16.113: Calibri 11pt at 1.15 → 15.44pt, at 1.5 → 20.08pt.
        let single = word_latin_font_line_height_pt("11.00pt Calibri");
        let at_115 = apply_word_font_proportional_line_height(
            11.0 * 1.15,
            Some(&multiple(1.15)),
            false,
            single,
        );
        assert!((at_115 - 15.44).abs() < 0.03, "{at_115}");
        let at_150 =
            apply_word_font_proportional_line_height(16.5, Some(&multiple(1.5)), false, single);
        assert!((at_150 - 20.08).abs() < 0.13, "{at_150}");
        let unset = apply_word_font_proportional_line_height(12.65, None, false, single);
        assert!((unset - 13.43).abs() < 0.01, "{unset}");
        // The face's line replaces the FE floor (12pt × 1.1375 = 13.65pt):
        // Word for Mac advances 10.5pt MS Mincho lines 13.6138pt.
        let ms = word_east_asian_font_line_height_pt("10.50pt 'ＭＳ 明朝'");
        let fe = apply_word_font_proportional_line_height(13.65, None, false, ms);
        assert!((fe - 13.6138).abs() < 0.01, "{fe}");
        // Exact, at-least, grid-snapped lines and unknown faces keep their height.
        let exact = Spacing {
            line: Some(12.0),
            line_rule: Some("exact".to_string()),
            ..Spacing::default()
        };
        assert_eq!(
            apply_word_font_proportional_line_height(12.0, Some(&exact), false, single),
            12.0
        );
        assert_eq!(
            apply_word_font_proportional_line_height(18.0, Some(&multiple(1.5)), true, single),
            18.0
        );
        assert_eq!(
            apply_word_font_proportional_line_height(16.5, Some(&multiple(1.5)), false, None),
            16.5
        );
    }

    #[test]
    fn word_for_mac_sizes_east_asian_lines_without_fe_layout() {
        assert!(!word_sizes_east_asian_lines_from_faces(false));
        assert!(word_sizes_east_asian_lines_from_faces(true));
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mac = word_sizes_east_asian_lines_from_faces(false);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!(mac);
    }

    #[test]
    fn adjacent_paragraph_spacing_takes_the_larger_value() {
        assert_eq!(collapsed_spacing_before(12.0, 6.0), 6.0);
        assert_eq!(collapsed_spacing_before(12.0, 0.0), 12.0);
        assert_eq!(collapsed_spacing_before(12.0, 20.0), 0.0);
        assert_eq!(collapsed_spacing_before(30.0, 20.0), 10.0);
    }

    #[test]
    fn noto_sans_auto_space_matches_repeated_word_pdf_numeric_boundaries() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let gap = word_auto_space_metrics("10.50pt 'Noto Sans'").expect("Noto Sans gaps");
        assert!((gap.leading_gap_px() * 0.75 - 2.625).abs() < 0.0001);
        // The Word PDF's 25年, 8月, 11日 and 34号 boundaries all leave 2.994pt.
        assert!((gap.trailing_gap_px(false) * 0.75 - 2.994140625).abs() < 0.0001);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(word_auto_space_metrics("10.50pt 'Noto Sans'"), None);
    }

    #[test]
    fn yu_gothic_light_ascii_keeps_its_observed_face_advances() {
        for face in ["游ゴシック Light", "Yu Gothic Light", "YuGothic-Light"] {
            for size_pt in [10.5, 11.0, 20.0] {
                let descriptor = format!("{size_pt:.2}pt '{face}'");
                let metrics = word_ascii_metrics(&descriptor).expect("observed light face");
                for (character, units) in [
                    (b' ', 584.0),
                    (b's', 969.0),
                    (b'o', 1061.0),
                    (b'm', 1686.0),
                    (b'e', 1055.0),
                    (b't', 608.0),
                    (b'h', 1106.0),
                    (b'i', 457.0),
                    (b'n', 1106.0),
                    (b'g', 1047.0),
                ] {
                    let expected = size_pt * 96.0 / 72.0 * units / 2048.0;
                    assert!((metrics.advance_px(character).unwrap() - expected).abs() < 1e-9);
                }
                assert_eq!(metrics.advance_px(b'0'), None);
                assert_eq!(metrics.pair_kerning_px(b's', b'o'), None);
            }
        }
        assert!(word_ascii_metrics("bold 11.00pt 'Yu Gothic Light'").is_none());
        assert!(word_ascii_metrics("italic 11.00pt 'Yu Gothic Light'").is_none());
        assert!(word_ascii_metrics("11.00pt 'Yu Gothic Medium'").is_none());
        assert!(word_ascii_metrics("11.00pt 'Yu Gothic'").is_none());
    }

    #[test]
    fn calibri_regular_pair_adjustments_match_source_matched_word_titles() {
        let metrics = word_ascii_metrics("22.00pt 'Calibri'").unwrap();
        let unit_px = 22.0 * 96.0 / 72.0 / 2048.0;
        for (left, right, units) in [
            (b'R', b'e', -36.0),
            (b'T', b'a', -160.0),
            (b'T', b'e', -182.0),
            (b'H', b'e', 0.0),
            (b'e', b'a', 0.0),
        ] {
            let adjustment = metrics.pair_kerning_px(left, right).expect("observed pair");
            assert!((adjustment - units * unit_px).abs() < 1e-9);
        }
        assert_eq!(metrics.pair_kerning_px(b'~', b'~'), None);
        let bold = word_ascii_metrics("bold 22.00pt 'Calibri'").unwrap();
        assert_eq!(bold.pair_kerning_px(b'R', b'e'), None);
    }

    #[test]
    fn yu_mincho_pa_uses_original_word_requested_pair_adjustment() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let metrics = word_ascii_metrics("10.50pt 'Yu Mincho'").unwrap();
        let pair = metrics.pair_kerning_px(b'P', b'a');
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(pair, Some(-92.0 * 10.5 * 96.0 / 72.0 / 2048.0));
    }

    #[test]
    fn aptos_regular_advances_and_pairs_use_original_word_numeric_metrics() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        for (descriptor, character, units) in [
            ("12.00pt Aptos", b'T', 980.0),
            ("12.00pt Aptos", b'e', 1079.0),
            ("12.00pt Aptos", b'1', 1094.0),
            ("20.00pt 'Aptos Display'", b'T', 977.0),
            ("20.00pt 'Aptos Display'", b'b', 1062.0),
        ] {
            let metrics = word_ascii_metrics(descriptor).expect("original Word Aptos face");
            let size = if descriptor.starts_with("20") {
                20.0
            } else {
                12.0
            };
            assert!(
                (metrics.advance_px(character).unwrap() - size * 96.0 / 72.0 * units / 2048.0)
                    .abs()
                    < 1e-9
            );
            assert_eq!(metrics.advance_px(b'~'), None);
            assert_eq!(metrics.advance_px(b' '), None);
            assert_eq!(metrics.pair_kerning_px(b'~', b'~'), None);
            assert!(
                (metrics.pair_kerning_px(b'T', b'e').unwrap()
                    + size * 96.0 / 72.0 * 164.0 / 2048.0)
                    .abs()
                    < 1e-9
            );
        }
        assert!(word_ascii_metrics("bold 12.00pt Aptos").is_none());
        assert!(word_ascii_metrics("italic 12.00pt Aptos").is_none());
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!(word_ascii_metrics("12.00pt Aptos").is_none());
        assert!(word_ascii_metrics("20.00pt 'Aptos Display'").is_none());
    }

    #[test]
    fn calibri_ascii_retains_observed_regular_and_bold_word_advances() {
        for (descriptor, character, units) in [
            ("22.00pt 'Calibri'", b'R', 1112.0),
            ("22.00pt 'Calibri'", b'e', 1019.0),
            ("22.00pt 'Calibri'", b'H', 1276.0),
            ("22.00pt 'Calibri'", b' ', 463.0),
            ("22.00pt 'Calibri'", b'd', 1076.0),
            ("bold 22.00pt 'Calibri'", b'R', 1153.0),
            ("bold 22.00pt 'Calibri'", b'd', 1099.0),
            ("bold 22.00pt 'Calibri'", b'1', 1038.0),
        ] {
            let metrics = word_ascii_metrics(descriptor).expect("observed Calibri face");
            assert!(
                (metrics.advance_px(character).unwrap() - 22.0 * 96.0 / 72.0 * units / 2048.0)
                    .abs()
                    < 1e-9
            );
        }
        let regular = word_ascii_metrics("22.00pt 'Calibri'").unwrap();
        assert_eq!(
            regular.advance_px(b'0'),
            None,
            "unobserved regular glyph keeps fallback"
        );
        assert!(word_ascii_metrics("italic 22.00pt 'Calibri'").is_none());
        assert!(word_ascii_metrics("22.00pt 'Calibri Light'").is_none());
    }

    #[test]
    fn noto_sans_ascii_keeps_observed_word_mac_advances() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let metrics =
            word_ascii_metrics("10.50pt 'Noto Sans', sans-serif").expect("Noto Sans ASCII");
        for (ascii, units) in [
            (b'0', 1128.0),
            (b'9', 1128.0),
            (b'(', 724.0),
            (b')', 724.0),
            (b',', 512.0),
            (b'.', 549.0),
            (b' ', 532.0),
        ] {
            assert!((metrics.advance_px(ascii).unwrap() - 14.0 * units / 2048.0).abs() < 0.0001);
        }
        assert_eq!(metrics.advance_px(b'A'), None);
        assert!(word_ascii_metrics("bold 10.50pt 'Noto Sans'").is_none());
        assert!(word_ascii_metrics("italic 10.50pt 'Noto Sans'").is_none());
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn noto_sans_latin_box_matches_the_word_mac_pdf_subset() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let descriptor = "10.50pt 'Noto Sans', sans-serif";
        let (ascent, descent) = word_font_vertical_metrics_px(descriptor).expect("Noto Sans box");
        assert!((ascent * 0.75 - 10.5 * 2189.0 / 2048.0).abs() < 0.0001);
        assert!((descent * 0.75 - 10.5 * 600.0 / 2048.0).abs() < 0.0001);
        assert!((word_latin_font_line_height_pt(descriptor).unwrap() - 14.299072).abs() < 0.0001);
        assert_eq!(word_east_asian_font_line_height_pt(descriptor), None);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(word_font_vertical_metrics_px(descriptor), None);
    }

    #[test]
    fn noto_sans_jp_line_box_matches_the_word_pdf_subset_metrics() {
        // The unchanged Noto JP Word PDF embeds hhea 1160/-288 at 1000
        // units/em. Its 10.5pt lines advance 19.68/19.92pt in the print grid.
        set_word_layout_platform(WordLayoutPlatform::Mac);
        for family in ["Noto Sans JP", "NotoSansJP"] {
            let descriptor = format!("10.50pt '{family}', sans-serif");
            assert_eq!(
                word_east_asian_font_vertical_units(family),
                Some((1160.0, 288.0, 1000.0))
            );
            let height = word_east_asian_font_line_height_pt(&descriptor).expect("Noto JP metrics");
            assert!((height - 19.761).abs() < 0.001, "{height}");
            let (ascent, descent) =
                word_font_vertical_metrics_px(&descriptor).expect("Noto JP baseline");
            assert!((ascent * 0.75 - 14.4585).abs() < 0.001);
            assert!((descent * 0.75 - 5.3025).abs() < 0.001);
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(word_east_asian_font_vertical_units("Noto Sans JP"), None);
    }

    #[test]
    fn auto_space_metrics_parse_repeated_descriptors_once_and_keep_inputs_independent() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mut descriptor = String::new();
        for (font, balance, expected) in [
            ("10.50pt 'Bookman Old Style', serif", false, None),
            (
                "10.50pt 'Bookman Old Style', serif",
                true,
                Some((14.0, 1.0)),
            ),
            (
                "10.50pt 'Times New Roman', serif",
                false,
                Some((14.0, 821.0 / 1024.0)),
            ),
            (
                "bold 10.50pt 'Times New Roman', serif",
                false,
                Some((14.0, 874.0 / 1024.0)),
            ),
            ("12.00pt 'Times New Roman', serif", true, Some((16.0, 1.0))),
            ("not a font", true, None),
            ("0.00pt 'Bookman Old Style', serif", true, None),
            ("10.50pt 'Bookman Old Style', serif", false, None),
        ] {
            // Callers can reuse the same String allocation for another face.
            descriptor.clear();
            descriptor.push_str(font);
            WORD_AUTO_SPACE_DESCRIPTOR_PARSES.with(|count| count.set(0));
            for _ in 0..1_000 {
                assert_eq!(
                    word_auto_space_metrics_with_balance(&descriptor, balance)
                        .map(|metrics| (metrics.em_px, metrics.trailing_share)),
                    expected
                );
            }
            assert_eq!(
                WORD_AUTO_SPACE_DESCRIPTOR_PARSES.with(std::cell::Cell::get),
                1
            );
        }
        // Leave a known Mac result in the cache so Windows isolation cannot
        // pass merely because both platforms return None for an unknown face.
        descriptor.clear();
        descriptor.push_str("10.50pt 'Times New Roman', serif");
        let mac_metrics = word_auto_space_metrics_with_balance(&descriptor, false);
        assert_eq!(
            mac_metrics.map(|metrics| (metrics.em_px, metrics.trailing_share)),
            Some((14.0, 821.0 / 1024.0))
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
        WORD_AUTO_SPACE_DESCRIPTOR_PARSES.with(|count| count.set(0));
        assert_eq!(
            word_auto_space_metrics_with_balance(&descriptor, false),
            None
        );
        assert_eq!(
            WORD_AUTO_SPACE_DESCRIPTOR_PARSES.with(std::cell::Cell::get),
            0
        );
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert_eq!(
            word_auto_space_metrics_with_balance(&descriptor, false),
            mac_metrics
        );
        assert_eq!(
            WORD_AUTO_SPACE_DESCRIPTOR_PARSES.with(std::cell::Cell::get),
            0
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn bookman_regular_ascii_widths_match_the_source_matched_word_subset() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let descriptor = "10.50pt 'Bookman Old Style', serif";
        assert!(
            word_auto_space_metrics(descriptor).is_none(),
            "unbalanced unknown average width retains fallback"
        );
        let metrics = word_ascii_metrics(descriptor).expect("Bookman metrics");
        for (character, units) in [
            (b' ', 655.0),
            (b'.', 655.0),
            (b'0', 1270.0),
            (b'1', 1270.0),
            (b'2', 1270.0),
            (b'3', 1270.0),
            (b'M', 1884.0),
            (b'c', 1065.0),
            (b'f', 655.0),
            (b'i', 614.0),
            (b'o', 1147.0),
            (b'r', 901.0),
            (b's', 1065.0),
            (b't', 778.0),
        ] {
            assert!(
                (metrics.advance_px(character).expect("observed glyph") - 14.0 * units / 2048.0)
                    .abs()
                    < 1e-9
            );
        }
        assert_eq!(metrics.advance_px(b'9'), None);
        assert_eq!(
            word_ascii_metrics("bold 10.50pt 'Bookman Old Style', serif")
                .map(|m| m.advance_px(b'M')),
            None
        );
        assert_eq!(
            word_ascii_metrics("italic 10.50pt 'Bookman Old Style', serif")
                .map(|m| m.advance_px(b'M')),
            None
        );
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn hg_maru_gothic_uses_the_requested_word_face_vertical_metrics() {
        set_word_layout_platform(WordLayoutPlatform::Mac);
        for family in ["HG丸ｺﾞｼｯｸM-PRO", "HGMaruGothicMPRO"] {
            assert_eq!(
                word_east_asian_font_vertical_units(family),
                Some((220.0, 36.0, 256.0))
            );
            let descriptor = format!("12.00pt '{family}', sans-serif");
            let (ascent, descent) =
                word_font_vertical_metrics_px(&descriptor).expect("HG Maru font metrics");
            // The existing Word rule adds round(256 * 0.15) = 38 font
            // units on each side of the glyph box.
            assert!((ascent * 0.75 - 12.09375).abs() < 0.0001);
            assert!((descent * 0.75 - 3.46875).abs() < 0.0001);
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert_eq!(word_east_asian_font_vertical_units("HG丸ｺﾞｼｯｸM-PRO"), None);
    }

    #[test]
    fn hiragino_kaku_pro_n_baseline_matches_word_mac_hello_reference() {
        // hello's Word PDF puts both 11pt baselines at 68.16pt below a 56.7pt margin.
        set_word_layout_platform(WordLayoutPlatform::Mac);
        for family in [
            "ヒラギノ角ゴ ProN W3",
            "Hiragino Kaku Gothic ProN W3",
            "Hiragino Kaku Gothic ProN",
        ] {
            let descriptor = format!("11.00pt '{family}', serif");
            let (ascent, _) = word_font_vertical_metrics_px(&descriptor).expect("Hiragino metrics");
            assert!((ascent * 0.75 - (68.16 - 56.7)).abs() <= 0.375);
        }
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn fixed_ascii_metrics_parse_each_repeated_font_once_including_proportional_faces() {
        word_fixed_ascii_advance_px("0.00pt 'reset before metric work budget'");
        for (descriptor, expected) in [
            ("10.50pt 'MS Mincho', serif", Some(7.0)),
            ("11.00pt 'Times New Roman', serif", None),
            ("12.00pt 'MS Mincho', serif", Some(8.0)),
            ("bold italic 10.50pt 'ＭＳ 明朝', serif", Some(7.28)),
            ("10.50pt 'MS PMincho', serif", None),
            ("10.50pt 'MS Gothic', serif", Some(7.0)),
            ("bold 12.00pt 'ＭＳ ゴシック', serif", Some(8.32)),
            ("10.50pt 'MS PGothic', serif", None),
            ("10.50pt 'MS Mincho', serif", Some(7.0)),
        ] {
            WORD_FIXED_ASCII_DESCRIPTOR_PARSES.with(|count| count.set(0));
            for _ in 0..1_000 {
                assert_eq!(word_fixed_ascii_advance_px(descriptor), expected);
            }
            assert_eq!(
                WORD_FIXED_ASCII_DESCRIPTOR_PARSES.with(std::cell::Cell::get),
                1
            );
        }
    }

    #[test]
    fn word_east_asian_font_line_height_follows_the_requested_face() {
        // Word for Mac 16.113, 30 single-spaced lines: Yu Mincho 10.5pt
        // advances 15.0455pt a line, MS Mincho 12pt 15.5669pt and 10.5pt
        // 13.6138pt (each ±0.01pt after 0.24pt quantization).
        let yu = word_east_asian_font_line_height_pt("10.50pt '游明朝', 'Yu Mincho', serif")
            .expect("Yu Mincho metrics");
        assert!((yu - 15.0455).abs() < 0.01, "{yu}");
        let ms = word_east_asian_font_line_height_pt("bold 12.00pt 'ＭＳ 明朝', serif")
            .expect("MS Mincho metrics");
        assert!((ms - 15.5669).abs() < 0.01, "{ms}");
        let ms = word_east_asian_font_line_height_pt("10.50pt 'ＭＳ 明朝'").expect("MS Mincho");
        assert!((ms - 13.6138).abs() < 0.01, "{ms}");
        assert_eq!(
            word_east_asian_font_line_height_pt("10.50pt 'Times New Roman', serif"),
            None
        );
    }

    #[test]
    fn east_asian_font_line_height_yields_to_explicit_spacing_and_grids() {
        let exact = Spacing {
            line: Some(12.0),
            line_rule: Some("exact".to_string()),
            ..Spacing::default()
        };
        let single = Spacing {
            line: Some(1.0),
            line_rule: Some("auto".to_string()),
            ..Spacing::default()
        };
        assert!(word_east_asian_font_line_box_applies(None, false, true));
        assert!(word_east_asian_font_line_box_applies(
            Some(&single),
            false,
            true
        ));
        assert!(!word_east_asian_font_line_box_applies(
            Some(&exact),
            false,
            true
        ));
        assert!(!word_east_asian_font_line_box_applies(None, true, true));
        assert!(!word_east_asian_font_line_box_applies(None, false, false));
        assert_eq!(
            apply_word_east_asian_font_line_height(13.65, true, Some(15.04)),
            15.04
        );
        assert_eq!(
            apply_word_east_asian_font_line_height(13.65, false, Some(15.04)),
            13.65
        );
        assert_eq!(
            apply_word_east_asian_font_line_height(15.6, true, Some(15.04)),
            15.6
        );
    }

    #[test]
    fn east_asian_fe_default_line_spacing_can_use_narrower_pitch() {
        let line_height = resolve_line_height_points_with_default_multiplier(
            12.0,
            None,
            None,
            false,
            None,
            None,
            WORD_EAST_ASIAN_FE_LINE_SPACING_MULTIPLIER,
        );
        assert!((line_height - 13.65).abs() < 0.001);
    }

    #[test]
    fn explicit_paragraph_mark_font_face_uses_visible_fe_line_box_floor() {
        let mark = RunProperties {
            font_family: Some("ＭＳ 明朝".into()),
            ..RunProperties::default()
        };

        assert_eq!(word_east_asian_fe_font_size_floor(false, Some(&mark)), 12.0);
        assert_eq!(word_east_asian_fe_font_size_floor(false, None), 11.0);
    }

    #[test]
    fn single_auto_spacing_uses_implicit_word_single_height() {
        let spacing = Spacing {
            line: Some(1.0),
            line_rule: Some("auto".to_string()),
            ..Spacing::default()
        };
        let line_height = resolve_line_height_points(10.5, Some(&spacing), None, false, None, None);
        assert_eq!(line_height, 10.5 * WORD_DEFAULT_LINE_SPACING_MULTIPLIER);
    }

    #[test]
    fn implicit_line_box_sets_floor_for_grid_snapped_single_spacing() {
        let grid = doc_grid("lines", Some(12.0), None);
        let line_height =
            resolve_line_height_points(10.5, None, Some(&grid), true, None, Some(15.0));
        assert_eq!(line_height, 15.0);
    }

    #[test]
    fn widow_control_moves_paragraph_when_only_its_first_line_box_fits() {
        assert_eq!(
            resolve_widow_orphan_decision(4, 1),
            WidowOrphanDecision::MoveWholeParagraph
        );
    }

    #[test]
    fn widow_control_keeps_two_lines_on_each_page() {
        assert_eq!(
            resolve_widow_orphan_decision(4, 2),
            WidowOrphanDecision::FlowNormally
        );
        assert_eq!(
            resolve_widow_orphan_decision(4, 3),
            WidowOrphanDecision::BreakBeforeLine(2)
        );
    }
}
