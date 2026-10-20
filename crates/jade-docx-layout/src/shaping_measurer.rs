//! Browser-free text measurement backed by OpenType shaping (`rustybuzz`).
//!
//! The browser renderer measures advance widths with canvas `measureText`,
//! which is Chromium's HarfBuzz + Skia pipeline. A native shell has no canvas,
//! so this module reproduces the same widths in-process.
//!
//! Plain OpenType shaping is *not* byte-identical to Chromium. Three corrections
//! close the gap; each one was derived empirically by measuring Chromium and
//! this backend over the same font file (see `tests/chromium_parity.rs`, which
//! locks the result against recorded Chromium output):
//!
//! 1. **Full-width punctuation trimming** — Chromium collapses adjacent
//!    full-width punctuation by 0.5em ([`ShapingOptions::trim_fullwidth_punctuation`]).
//! 2. **`kern` scope** — Chromium does not apply the font's kana kerning pairs
//!    ([`ShapingOptions::disable_kern_for_cjk`]).
//! 3. **Font-size quantization** — Chromium truncates the px font size to two
//!    decimals, so 14pt shapes at 18.66px rather than 18.6667px
//!    ([`ShapingOptions::quantize_px_font_size`]).
//!
//! Font fallback and vertical metrics follow the same rule-by-measurement
//! approach; see `tests/chromium_parity.rs` and `tests/chromium_vertical.rs`.

use crate::font_measurer::{
    parse_font_descriptor, pick_font_index, ParsedFontDescriptor, RegisteredFontStyle,
};
use crate::font_resolver::{resolve_run_style_for_wrap, ResolvedRunStyle};
use crate::layout_cache::{TextMeasureCache, TextMeasurer};
use crate::line_wrap::LineWrapContext;
use crate::units::points_to_px;
use jade_docx_reader::RunProperties;
use rustybuzz::ttf_parser::Tag;
use rustybuzz::{Face, Feature, UnicodeBuffer};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::rc::Rc;

/// Full-width punctuation whose *left* half is blank (opening brackets).
/// Chromium trims that half whenever another full-width punctuation precedes it.
const FULLWIDTH_OPEN: &[char] = &[
    '「', '『', '（', '［', '｛', '〈', '《', '【', '〔', '〖', '〘', '〚', '｟', '〝',
];

/// Full-width punctuation whose *right* half is blank (closing brackets, plus
/// the ideographic comma and full stop). Chromium trims that half whenever
/// another full-width punctuation follows.
const FULLWIDTH_CLOSE: &[char] = &[
    '」', '』', '）', '］', '｝', '〉', '》', '】', '〕', '〗', '〙', '〛', '｠', '〟', '、', '。',
    '，', '．',
];

/// Full-width punctuation that participates in trimming as a neighbour but
/// never triggers a trim on its own side.
const FULLWIDTH_NEUTRAL: &[char] = &['\u{3000}', '：', '・'];

/// The `kern` GPOS feature, disabled on CJK runs to match Chromium.
const KERN: &[u8; 4] = b"kern";

/// Sample the browser renderer measures for font-level vertical metrics. Kept
/// identical so the two backends derive line heights from the same glyphs.
const VERTICAL_METRICS_SAMPLE: &str = "Hg\u{3042}";

/// Which browser-compatibility corrections the shaper applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapingOptions {
    /// Collapse adjacent full-width punctuation by 0.5em.
    pub trim_fullwidth_punctuation: bool,
    /// Turn the `kern` feature off for CJK runs (Latin runs keep it).
    pub disable_kern_for_cjk: bool,
    /// Truncate the px font size to two decimals before shaping.
    pub quantize_px_font_size: bool,
}

impl ShapingOptions {
    /// Reproduces Chromium canvas `measureText`, i.e. what the browser renderer
    /// measures today. This is the setting to use when native layout must agree
    /// with the existing browser layout.
    pub const CHROMIUM_PARITY: Self = Self {
        trim_fullwidth_punctuation: true,
        disable_kern_for_cjk: true,
        quantize_px_font_size: true,
    };

    /// Plain OpenType shaping with no browser-specific corrections.
    pub const PLAIN_OPENTYPE: Self = Self {
        trim_fullwidth_punctuation: false,
        disable_kern_for_cjk: false,
        quantize_px_font_size: false,
    };
}

impl Default for ShapingOptions {
    fn default() -> Self {
        Self::CHROMIUM_PARITY
    }
}

struct RegisteredFace {
    family: String,
    weight: u16,
    style: RegisteredFontStyle,
    // SAFETY INVARIANT: `face` borrows `bytes`. Declared first so it is dropped
    // before the buffer it points into, and `bytes` is never mutated or
    // reallocated after construction, so the `'static` lifetime is never
    // observable outside this struct.
    face: Face<'static>,
    _bytes: Box<[u8]>,
}

/// A font stack resolved once per descriptor: the faces named by the descriptor
/// in priority order, plus the face to use for characters none of them cover.
#[derive(Debug)]
struct ResolvedStack {
    /// Face indices for the descriptor's families, in stack order. Family names
    /// with no registered face are skipped, matching Chromium.
    faces: Vec<usize>,
    /// Face used when no family in the stack has a glyph. Chromium would reach
    /// for a system font here; with only registered faces available this is the
    /// closest match by family/weight/style.
    fallback: usize,
    size_pt: f32,
}

/// `rustybuzz`-backed [`TextMeasurer`] that can reproduce Chromium advance widths.
pub struct ShapingMeasurer {
    default_font_size_pt: f32,
    options: ShapingOptions,
    faces: Vec<RegisteredFace>,
    descriptor_cache: RefCell<HashMap<String, Rc<ResolvedStack>>>,
    uncovered: RefCell<BTreeSet<char>>,
}

impl ShapingMeasurer {
    pub fn new(default_font_size_pt: f32) -> Self {
        Self::with_options(default_font_size_pt, ShapingOptions::default())
    }

    pub fn with_options(default_font_size_pt: f32, options: ShapingOptions) -> Self {
        Self {
            default_font_size_pt,
            options,
            faces: Vec::new(),
            descriptor_cache: RefCell::new(HashMap::new()),
            uncovered: RefCell::new(BTreeSet::new()),
        }
    }

    pub fn options(&self) -> ShapingOptions {
        self.options
    }

    pub fn register_font_path<P: AsRef<Path>>(
        &mut self,
        family: impl Into<String>,
        weight: u16,
        style: RegisteredFontStyle,
        path: P,
    ) -> Result<(), Box<dyn Error>> {
        self.register_font_path_with_collection_index(family, weight, style, path, 0)
    }

    pub fn register_font_path_with_collection_index<P: AsRef<Path>>(
        &mut self,
        family: impl Into<String>,
        weight: u16,
        style: RegisteredFontStyle,
        path: P,
        collection_index: u32,
    ) -> Result<(), Box<dyn Error>> {
        let bytes = fs::read(path)?;
        self.register_font_bytes_with_collection_index(
            family,
            weight,
            style,
            &bytes,
            collection_index,
        )
    }

    pub fn register_font_bytes(
        &mut self,
        family: impl Into<String>,
        weight: u16,
        style: RegisteredFontStyle,
        bytes: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        self.register_font_bytes_with_collection_index(family, weight, style, bytes, 0)
    }

    pub fn register_font_bytes_with_collection_index(
        &mut self,
        family: impl Into<String>,
        weight: u16,
        style: RegisteredFontStyle,
        bytes: &[u8],
        collection_index: u32,
    ) -> Result<(), Box<dyn Error>> {
        let owned: Box<[u8]> = bytes.to_vec().into_boxed_slice();
        // SAFETY: the slice points into `owned`, which is heap-allocated and
        // moves into the `RegisteredFace` below without reallocating. The face
        // is dropped before the buffer (field order), so it never outlives it.
        let borrowed: &'static [u8] =
            unsafe { std::slice::from_raw_parts(owned.as_ptr(), owned.len()) };
        let face = Face::from_slice(borrowed, collection_index).ok_or_else(|| {
            format!("failed to parse font face (collection index {collection_index})")
        })?;
        self.faces.push(RegisteredFace {
            family: family.into(),
            weight,
            style,
            face,
            _bytes: owned,
        });
        self.descriptor_cache.borrow_mut().clear();
        Ok(())
    }

    /// Full line box height, matching the browser's
    /// `fontBoundingBoxAscent + fontBoundingBoxDescent`.
    ///
    /// Chromium derives those from the primary face's `hhea` ascender and
    /// descender, **rounds each to a whole pixel**, and ignores the line gap.
    /// Rounding before summing matters: the two halves are rounded
    /// independently, so this is not `round(ascent + descent)`.
    pub fn measure_line_box_height(&self, font_descriptor: &str) -> Option<f64> {
        let (ascent, descent) = self.font_bounding_box(font_descriptor)?;
        Some(ascent + descent)
    }

    /// The two halves of [`Self::measure_line_box_height`], separately.
    ///
    /// A canvas `TextMetrics` carries `fontBoundingBoxAscent` and
    /// `fontBoundingBoxDescent` as distinct fields, so a host answering
    /// measurement from this measurer needs both rather than their sum.
    pub fn font_bounding_box(&self, font_descriptor: &str) -> Option<(f64, f64)> {
        let (face, size_pt) = self.resolve_face(font_descriptor)?;
        let scale = self.font_size_px(size_pt) / f64::from(face.units_per_em());
        let ascent = (f64::from(face.ascender()) * scale).round();
        let descent = (f64::from(face.descender()).abs() * scale).round();
        Some((ascent, descent))
    }

    /// Ascent/descent in px, matching the browser's
    /// `actualBoundingBoxAscent/Descent` for the shared sample string.
    ///
    /// Unlike the font bounding box this is *ink* extent and it does follow
    /// per-character fallback, so a face lacking the sample's kana contributes
    /// only its Latin glyphs. Falls back to the font bounding box when the
    /// sample yields no ink, mirroring the browser path.
    pub fn measure_font_vertical_metrics(&self, font_descriptor: &str) -> Option<(f64, f64)> {
        let stack = self.resolve_stack(font_descriptor)?;
        let size_px = self.font_size_px(stack.size_pt);

        let mut ascent = f64::MIN;
        let mut descent = f64::MIN;
        for ch in VERTICAL_METRICS_SAMPLE.chars() {
            let index = self.face_for_char(&stack, ch);
            let Some(entry) = self.faces.get(index) else {
                continue;
            };
            let Some(bounds) = entry
                .face
                .glyph_index(ch)
                .and_then(|glyph| entry.face.glyph_bounding_box(glyph))
            else {
                continue;
            };
            let scale = size_px / f64::from(entry.face.units_per_em());
            ascent = ascent.max(f64::from(bounds.y_max) * scale);
            descent = descent.max(-(f64::from(bounds.y_min) * scale));
        }

        if ascent > 0.0 && descent >= 0.0 {
            return Some((ascent, descent));
        }
        self.font_bounding_box(font_descriptor)
    }

    /// Converts pt to px the way Chromium does. Blink shapes at a px size that
    /// has been truncated to two decimals, so 14pt (18.6667px) actually shapes
    /// at 18.66px — a 3.6e-4 relative narrowing that is absent at sizes whose px
    /// value terminates (9pt, 10.5pt, 12pt).
    fn font_size_px(&self, size_pt: f32) -> f64 {
        let px = points_to_px(size_pt);
        if self.options.quantize_px_font_size {
            (px * 100.0).floor() / 100.0
        } else {
            px
        }
    }

    /// Primary face for descriptor-level queries that are not per-character
    /// (vertical metrics). This is the first family in the stack that resolves.
    fn resolve_face(&self, descriptor: &str) -> Option<(&Face<'static>, f32)> {
        let stack = self.resolve_stack(descriptor)?;
        let index = stack.faces.first().copied().unwrap_or(stack.fallback);
        self.faces
            .get(index)
            .map(|entry| (&entry.face, stack.size_pt))
    }

    fn resolve_stack(&self, descriptor: &str) -> Option<Rc<ResolvedStack>> {
        if self.faces.is_empty() {
            return None;
        }
        if let Some(cached) = self.descriptor_cache.borrow().get(descriptor) {
            return Some(Rc::clone(cached));
        }
        let spec = parse_font_descriptor(descriptor, self.default_font_size_pt);
        let faces = spec
            .families
            .iter()
            .filter_map(|family| self.best_face_for_family(family, &spec))
            .collect();
        let stack = Rc::new(ResolvedStack {
            faces,
            fallback: self.pick_index(&spec).unwrap_or(0),
            size_pt: spec.size_pt,
        });
        self.descriptor_cache
            .borrow_mut()
            .insert(descriptor.to_string(), Rc::clone(&stack));
        Some(stack)
    }

    /// Best registered face carrying `family`, scored on weight and style only —
    /// stack order is already handled by the caller.
    fn best_face_for_family(&self, family: &str, spec: &ParsedFontDescriptor) -> Option<usize> {
        let mut best: Option<(usize, i32)> = None;
        for (index, face) in self.faces.iter().enumerate() {
            if !face.family.eq_ignore_ascii_case(family) {
                continue;
            }
            let mut score = -(spec.weight as i32 - face.weight as i32).abs();
            if spec.style == face.style {
                score += 100;
            }
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((index, score));
            }
        }
        best.map(|(index, _)| index)
    }

    /// Face for one character: the stack's first that has it, else the
    /// fallback face, else any registered face that has it.
    ///
    /// Chromium reaches for a system font once the named families run out;
    /// the nearest a measurer built from registered faces has is another
    /// registered face carrying the character — Noto Sans Thai for Thai in a
    /// Japanese run. Characters no registered face covers would silently
    /// measure as `.notdef`, so they are recorded, letting a native shell
    /// surface "this document needs a font you have not registered" instead.
    fn face_for_char(&self, stack: &ResolvedStack, ch: char) -> usize {
        let selected = select_face(&stack.faces, stack.fallback, |index| self.covers(index, ch));
        if self.covers(selected, ch) {
            return selected;
        }
        match (0..self.faces.len()).find(|index| self.covers(*index, ch)) {
            Some(covering) => covering,
            None => {
                self.uncovered.borrow_mut().insert(ch);
                selected
            }
        }
    }

    /// Characters seen so far that none of their descriptor's families covered.
    /// A non-empty result means measurement may diverge from the browser, which
    /// would have substituted a system font.
    pub fn uncovered_characters(&self) -> Vec<char> {
        self.uncovered.borrow().iter().copied().collect()
    }

    /// Clears the recorded coverage gaps.
    pub fn clear_uncovered_characters(&self) {
        self.uncovered.borrow_mut().clear();
    }

    /// True when the face at `index` has a real glyph for `ch`.
    fn covers(&self, index: usize, ch: char) -> bool {
        self.faces
            .get(index)
            .and_then(|entry| entry.face.glyph_index(ch))
            .is_some_and(|glyph| glyph.0 != 0)
    }

    fn pick_index(&self, spec: &ParsedFontDescriptor) -> Option<usize> {
        pick_font_index(
            self.faces
                .iter()
                .map(|entry| (entry.family.as_str(), entry.weight, entry.style)),
            spec,
        )
    }
}

/// True for scripts where Chromium leaves the font's `kern` pairs unapplied.
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3000..=0x303F   // CJK symbols and punctuation
        | 0x3040..=0x309F // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xFF00..=0xFFEF // Halfwidth and Fullwidth Forms
    )
}

fn is_fullwidth_punctuation(ch: char) -> bool {
    FULLWIDTH_OPEN.contains(&ch) || FULLWIDTH_CLOSE.contains(&ch) || FULLWIDTH_NEUTRAL.contains(&ch)
}

/// Chromium collapses the blank half of a full-width punctuation mark when it
/// sits next to another one. Both conditions can hold at a single boundary
/// (`」「`), but the collapse is still a single 0.5em — never 1.0em.
fn trims_between(left: char, right: char) -> bool {
    if !is_fullwidth_punctuation(left) || !is_fullwidth_punctuation(right) {
        return false;
    }
    FULLWIDTH_OPEN.contains(&right) || FULLWIDTH_CLOSE.contains(&left)
}

fn fullwidth_trim_count(text: &str) -> usize {
    let mut count = 0;
    let mut previous: Option<char> = None;
    for ch in text.chars() {
        if let Some(left) = previous {
            if trims_between(left, ch) {
                count += 1;
            }
        }
        previous = Some(ch);
    }
    count
}

/// Shapes one run and returns its advance in font design units.
fn shape_run_units(face: &Face<'_>, text: &str, kern_enabled: bool) -> i32 {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let features = if kern_enabled {
        Vec::new()
    } else {
        vec![Feature::new(Tag::from_bytes(KERN), 0, ..)]
    };
    let shaped = rustybuzz::shape(face, &features, buffer);
    shaped
        .glyph_positions()
        .iter()
        .map(|position| position.x_advance)
        .sum()
}

/// Picks the face for one character: the first family in the stack that has a
/// glyph, else `fallback`. Chromium resolves fallback per character rather than
/// per run, so a stack whose first font lacks Japanese still measures Latin with
/// that first font.
fn select_face(stack: &[usize], fallback: usize, covers: impl Fn(usize) -> bool) -> usize {
    stack
        .iter()
        .copied()
        .find(|index| covers(*index))
        .unwrap_or(fallback)
}

impl ShapingMeasurer {
    /// Advance of one run in px. Each face carries its own units-per-em, so the
    /// scale has to be applied per run rather than once per string.
    fn run_advance_px(&self, text: &str, face_index: usize, cjk: bool, size_px: f64) -> f64 {
        let Some(entry) = self.faces.get(face_index) else {
            return 0.0;
        };
        let kern_enabled = !(cjk && self.options.disable_kern_for_cjk);
        let units = shape_run_units(&entry.face, text, kern_enabled);
        f64::from(units) / f64::from(entry.face.units_per_em()) * size_px
    }
}

impl TextMeasurer for ShapingMeasurer {
    type Error = Box<dyn Error>;

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        let Some(stack) = self.resolve_stack(font_descriptor) else {
            return Ok(0.0);
        };
        let size_px = self.font_size_px(stack.size_pt);

        // Split into maximal runs sharing a face and a kern policy. Chromium
        // shapes each fallback run independently, so no kerning crosses a font
        // boundary and none is applied inside CJK runs.
        let mut width = 0.0_f64;
        let mut run_start = 0_usize;
        let mut current: Option<(usize, bool)> = None;
        for (offset, ch) in text.char_indices() {
            let key = (self.face_for_char(&stack, ch), is_cjk(ch));
            match current {
                Some(previous) if previous == key => {}
                Some((face_index, cjk)) => {
                    width +=
                        self.run_advance_px(&text[run_start..offset], face_index, cjk, size_px);
                    run_start = offset;
                    current = Some(key);
                }
                None => current = Some(key),
            }
        }
        if let Some((face_index, cjk)) = current {
            width += self.run_advance_px(&text[run_start..], face_index, cjk, size_px);
        }

        if self.options.trim_fullwidth_punctuation {
            width -= 0.5 * size_px * fullwidth_trim_count(text) as f64;
        }
        Ok(width)
    }
}

/// [`LineWrapContext`] that wraps a [`ShapingMeasurer`] with the shared
/// measurement cache, mirroring `FontdueLineWrapContext`.
pub struct ShapingLineWrapContext {
    measurer: ShapingMeasurer,
    cache: RefCell<TextMeasureCache>,
    doc_grid_char_spacing_px: Option<f64>,
}

impl ShapingLineWrapContext {
    pub fn new(measurer: ShapingMeasurer, doc_grid_char_spacing_px: Option<f64>) -> Self {
        Self {
            measurer,
            cache: RefCell::new(TextMeasureCache::default()),
            doc_grid_char_spacing_px,
        }
    }

    pub fn measurer(&self) -> &ShapingMeasurer {
        &self.measurer
    }
}

impl LineWrapContext for ShapingLineWrapContext {
    type Error = Box<dyn Error>;

    fn resolve_run_style(
        &self,
        properties: Option<&RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        Ok(resolve_run_style_for_wrap(
            properties,
            self.doc_grid_char_spacing_px,
        ))
    }

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        let mut cache = self.cache.borrow_mut();
        cache.measure_with(&self.measurer, text, font_descriptor)
    }

    #[inline]
    fn measure_text_with_hash(
        &self,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
    ) -> Result<f64, Self::Error> {
        let mut cache = self.cache.borrow_mut();
        cache.measure_with_hash(&self.measurer, text, font_descriptor, font_descriptor_hash)
    }

    fn measure_line_box_height(&self, font_descriptor: &str) -> Result<Option<f64>, Self::Error> {
        Ok(self.measurer.measure_line_box_height(font_descriptor))
    }

    fn measure_font_vertical_metrics(
        &self,
        font_descriptor: &str,
    ) -> Result<Option<(f64, f64)>, Self::Error> {
        Ok(self.measurer.measure_font_vertical_metrics(font_descriptor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn bundled_font_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_JP/static")
            .join(name)
    }

    fn measurer(options: ShapingOptions) -> ShapingMeasurer {
        let mut measurer = ShapingMeasurer::with_options(12.0, options);
        measurer
            .register_font_path(
                "Noto Sans JP",
                400,
                RegisteredFontStyle::Normal,
                bundled_font_path("NotoSansJP-Regular.ttf"),
            )
            .expect("register bundled Noto Sans JP");
        measurer
    }

    const DESC: &str = "10.50pt 'Noto Sans JP'";
    const EM: f64 = 14.0; // 10.5pt -> 14px

    #[test]
    fn closing_then_opening_punctuation_collapses_by_half_em_only_once() {
        let measurer = measurer(ShapingOptions::CHROMIUM_PARITY);

        // Both trim rules fire at this boundary, but the collapse stays 0.5em.
        let width = measurer.measure_text("」「", DESC).expect("measure");

        assert!((width - 1.5 * EM).abs() < 1e-6, "got {width}");
    }

    #[test]
    fn opening_punctuation_collapses_after_any_fullwidth_punctuation() {
        let measurer = measurer(ShapingOptions::CHROMIUM_PARITY);

        for pair in ["、「", "。（", "：【", "・〈", "\u{3000}『"] {
            let width = measurer.measure_text(pair, DESC).expect("measure");
            assert!((width - 1.5 * EM).abs() < 1e-6, "{pair}: got {width}");
        }
    }

    #[test]
    fn closing_punctuation_collapses_before_any_fullwidth_punctuation() {
        let measurer = measurer(ShapingOptions::CHROMIUM_PARITY);

        for pair in ["」、", "。・", "）：", "］\u{3000}", "、。"] {
            let width = measurer.measure_text(pair, DESC).expect("measure");
            assert!((width - 1.5 * EM).abs() < 1e-6, "{pair}: got {width}");
        }
    }

    #[test]
    fn punctuation_next_to_ideographs_and_excluded_marks_is_not_collapsed() {
        let measurer = measurer(ShapingOptions::CHROMIUM_PARITY);

        // `！？ー〜` are full-width but outside Chromium's trimming classes, and
        // an ideograph on either side never triggers a trim.
        for pair in ["あ、", "、あ", "日「", "！「", "、！", "ー「", "」〜"] {
            let width = measurer.measure_text(pair, DESC).expect("measure");
            assert!((width - 2.0 * EM).abs() < 1e-6, "{pair}: got {width}");
        }
    }

    #[test]
    fn trimming_is_applied_per_boundary_across_longer_runs() {
        let measurer = measurer(ShapingOptions::CHROMIUM_PARITY);

        // 「「「 has two collapsing boundaries.
        let width = measurer.measure_text("「「「", DESC).expect("measure");

        assert!((width - 2.0 * EM).abs() < 1e-6, "got {width}");
    }

    #[test]
    fn disabling_the_trim_option_restores_plain_shaping_widths() {
        let plain = measurer(ShapingOptions::PLAIN_OPENTYPE);

        let width = plain.measure_text("」「", DESC).expect("measure");

        assert!((width - 2.0 * EM).abs() < 1e-6, "got {width}");
    }

    #[test]
    fn kern_stays_enabled_for_latin_and_disabled_for_kana() {
        let parity = measurer(ShapingOptions::CHROMIUM_PARITY);
        let plain = measurer(ShapingOptions::PLAIN_OPENTYPE);

        // Latin keeps its kerning pairs, so `AV` is narrower than `A` + `V`.
        let av = parity.measure_text("AV", DESC).expect("measure");
        let a = parity.measure_text("A", DESC).expect("measure");
        let v = parity.measure_text("V", DESC).expect("measure");
        assert!(av < a + v - 1e-6, "AV={av} A+V={}", a + v);

        // `えて` carries a kana kern pair that Chromium does not apply.
        let parity_kana = parity.measure_text("えて", DESC).expect("measure");
        let plain_kana = plain.measure_text("えて", DESC).expect("measure");
        assert!((parity_kana - 2.0 * EM).abs() < 1e-6, "got {parity_kana}");
        assert!(plain_kana < parity_kana - 1e-6);
    }

    #[test]
    fn px_font_size_is_truncated_to_two_decimals_like_chromium() {
        let parity = measurer(ShapingOptions::CHROMIUM_PARITY);
        let plain = measurer(ShapingOptions::PLAIN_OPENTYPE);

        // 14pt is 18.6667px; Chromium shapes at 18.66px.
        let quantized = parity.measure_text("あ", "14.00pt 'Noto Sans JP'").unwrap();
        let exact = plain.measure_text("あ", "14.00pt 'Noto Sans JP'").unwrap();
        assert!((quantized - 18.66).abs() < 1e-6, "got {quantized}");
        assert!(exact > quantized);

        // Sizes whose px value terminates are unaffected.
        let terminating = parity.measure_text("あ", DESC).unwrap();
        assert!((terminating - EM).abs() < 1e-6, "got {terminating}");
    }

    #[test]
    fn select_face_walks_the_stack_and_falls_back_when_nothing_covers() {
        // First covering family in stack order wins, regardless of position.
        assert_eq!(select_face(&[3, 1, 2], 9, |index| index == 1), 1);
        assert_eq!(select_face(&[3, 1, 2], 9, |index| index != 3), 1);
        assert_eq!(select_face(&[3, 1, 2], 9, |_| true), 3);
        // Nothing in the stack covers the character.
        assert_eq!(select_face(&[3, 1, 2], 9, |_| false), 9);
        // Empty stack (no named family resolved) always uses the fallback.
        assert_eq!(select_face(&[], 9, |_| true), 9);
    }

    #[test]
    fn unregistered_family_names_are_skipped_like_chromium() {
        let measurer = measurer(ShapingOptions::CHROMIUM_PARITY);

        let with_missing_first = measurer
            .measure_text("Aあ", "10.50pt 'No Such Font', 'Noto Sans JP'")
            .expect("measure");
        let direct = measurer.measure_text("Aあ", DESC).expect("measure");

        assert!((with_missing_first - direct).abs() < 1e-9);
    }

    #[test]
    fn characters_no_registered_family_covers_are_recorded() {
        let measurer = measurer(ShapingOptions::CHROMIUM_PARITY);

        // Noto Sans JP carries no Hangul, so the browser would substitute a
        // system font here and the native width cannot match.
        measurer.measure_text("日本語한", DESC).expect("measure");

        assert_eq!(measurer.uncovered_characters(), vec!['한']);

        measurer.clear_uncovered_characters();
        measurer.measure_text("日本語", DESC).expect("measure");
        assert!(measurer.uncovered_characters().is_empty());
    }

    /// Cross-font fallback needs two faces with different coverage, so it uses a
    /// macOS system Arial. Ignored rather than skipped so it never passes
    /// silently on a machine without that file. Expected widths are the ones
    /// Chromium reported for the same two faces.
    #[test]
    #[ignore = "requires macOS /System/Library/Fonts/Supplemental/Arial.ttf"]
    fn per_character_fallback_matches_chromium_across_two_faces() {
        let arial = PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf");
        let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
        measurer
            .register_font_path("TestLatin", 400, RegisteredFontStyle::Normal, &arial)
            .expect("register Arial");
        measurer
            .register_font_path(
                "TestJP",
                400,
                RegisteredFontStyle::Normal,
                bundled_font_path("NotoSansJP-Regular.ttf"),
            )
            .expect("register Noto");

        let latin_first = "10.50pt 'TestLatin', 'TestJP'";
        let jp_first = "10.50pt 'TestJP', 'TestLatin'";
        for (descriptor, text, expected) in [
            (latin_first, "A", 9.3379),    // covered by both, stack order wins
            (latin_first, "あ", 14.0),     // Arial has no kana, falls through
            (latin_first, "Aあ", 23.3379), // fallback resolves per character
            (latin_first, "ا", 2.8984),    // Arial covers Arabic, Noto does not
            (jp_first, "A", 8.5120),
            (jp_first, "Aあ", 22.5120),
        ] {
            let actual = measurer.measure_text(text, descriptor).expect("measure");
            assert!(
                (actual - expected).abs() < 0.01,
                "{text:?} @{descriptor}: expected {expected}, got {actual}"
            );
        }
    }

    /// A character no family in the descriptor has falls back to a
    /// registered face that has it, as Chromium reaches for a system font:
    /// Thai in a Japanese run measures as Noto Sans Thai's glyphs, not as
    /// `.notdef` boxes, and is not reported as uncovered.
    #[test]
    fn a_character_no_named_family_has_falls_back_to_any_registered_face() {
        let mut measurer = measurer(ShapingOptions::CHROMIUM_PARITY);
        measurer
            .register_font_path(
                "Noto Sans Thai",
                400,
                RegisteredFontStyle::Normal,
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fonts/Noto_Sans_Thai/NotoSansThai-VariableFont_wdth,wght.ttf"),
            )
            .expect("register Noto Sans Thai");
        let japanese = "10.50pt 'Noto Sans JP'";
        let latin = measurer.measure_text("Total ", japanese).expect("measure");
        let thai = measurer
            .measure_text("บาท", "10.50pt 'Noto Sans Thai'")
            .expect("measure");
        let mixed = measurer
            .measure_text("Total บาท", japanese)
            .expect("measure");
        assert!(
            (mixed - (latin + thai)).abs() < 0.01,
            "{mixed} vs {latin} + {thai}"
        );
        assert!(measurer.uncovered_characters().is_empty());
        // A character no registered face has is still reported.
        measurer.measure_text("한", japanese).expect("measure");
        assert_eq!(measurer.uncovered_characters(), ['한']);
    }

    #[test]
    fn measuring_without_registered_fonts_yields_zero() {
        let measurer = ShapingMeasurer::new(12.0);

        assert_eq!(measurer.measure_text("あ", DESC).expect("measure"), 0.0);
    }
}
