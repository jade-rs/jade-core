use crate::font_resolver::{resolve_run_style_for_wrap, ResolvedRunStyle};
use crate::layout_cache::{TextMeasureCache, TextMeasurer};
use crate::line_wrap::LineWrapContext;
use crate::units::points_to_px;
use fontdue::{Font, FontSettings, LineMetrics};
use jade_docx_reader::RunProperties;
use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisteredFontStyle {
    Normal,
    Italic,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParsedFontDescriptor {
    pub(crate) families: Vec<String>,
    pub(crate) weight: u16,
    pub(crate) style: RegisteredFontStyle,
    pub(crate) size_pt: f32,
}

#[derive(Clone)]
struct RegisteredFont {
    family: String,
    weight: u16,
    style: RegisteredFontStyle,
    font: Font,
}

/// Deterministic `fontdue`-backed measurer for `jade-docx-layout`.
///
/// This exists so Rust golden tests and browser rendering can share the same
/// line-height API shape (`measure_line_box_height`) even though the underlying
/// measurement backend differs.
#[derive(Clone, Default)]
pub struct FontdueMeasurer {
    default_font_size_pt: f32,
    fonts: Vec<RegisteredFont>,
    descriptor_cache: RefCell<HashMap<String, usize>>,
}

impl FontdueMeasurer {
    pub fn new(default_font_size_pt: f32) -> Self {
        Self {
            default_font_size_pt,
            fonts: Vec::new(),
            descriptor_cache: RefCell::new(HashMap::new()),
        }
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
        let font = Font::from_bytes(
            bytes,
            FontSettings {
                collection_index,
                ..FontSettings::default()
            },
        )?;
        self.fonts.push(RegisteredFont {
            family: family.into(),
            weight,
            style,
            font,
        });
        self.descriptor_cache.borrow_mut().clear();
        Ok(())
    }

    pub fn measure_line_box_height(
        &self,
        font_descriptor: &str,
    ) -> Result<Option<f64>, Box<dyn Error>> {
        let Some((font, size_pt)) = self.resolve_font(font_descriptor)? else {
            return Ok(None);
        };
        let px_size = points_to_px(size_pt) as f32;
        let metrics = font
            .horizontal_line_metrics(px_size)
            .or_else(|| fallback_line_metrics(font, px_size));
        Ok(metrics.map(|metrics| metrics.new_line_size as f64))
    }

    pub fn measure_font_vertical_metrics(
        &self,
        font_descriptor: &str,
    ) -> Result<Option<(f64, f64)>, Box<dyn Error>> {
        let Some((font, size_pt)) = self.resolve_font(font_descriptor)? else {
            return Ok(None);
        };
        let px_size = points_to_px(size_pt) as f32;
        let metrics = font
            .horizontal_line_metrics(px_size)
            .or_else(|| fallback_line_metrics(font, px_size));
        Ok(metrics.map(|metrics| (metrics.ascent as f64, metrics.descent.abs() as f64)))
    }

    fn resolve_font(&self, descriptor: &str) -> Result<Option<(&Font, f32)>, Box<dyn Error>> {
        if self.fonts.is_empty() {
            return Ok(None);
        }

        let spec = self.parse_descriptor(descriptor);
        if let Some(index) = self.descriptor_cache.borrow().get(descriptor).copied() {
            return Ok(self
                .fonts
                .get(index)
                .map(|entry| (&entry.font, spec.size_pt)));
        }

        let index = self.pick_font_index(&spec).unwrap_or(0);
        self.descriptor_cache
            .borrow_mut()
            .insert(descriptor.to_string(), index);
        Ok(self
            .fonts
            .get(index)
            .map(|entry| (&entry.font, spec.size_pt)))
    }

    fn pick_font_index(&self, spec: &ParsedFontDescriptor) -> Option<usize> {
        pick_font_index(
            self.fonts
                .iter()
                .map(|font| (font.family.as_str(), font.weight, font.style)),
            spec,
        )
    }

    fn parse_descriptor(&self, descriptor: &str) -> ParsedFontDescriptor {
        parse_font_descriptor(descriptor, self.default_font_size_pt)
    }
}

/// Parses a CSS-style font shorthand (`"bold 10.50pt 'Noto Sans JP', serif"`)
/// into the family list, weight, style, and size that measurement backends need.
/// Shared so every backend resolves the same descriptor string identically.
pub(crate) fn parse_font_descriptor(
    descriptor: &str,
    default_font_size_pt: f32,
) -> ParsedFontDescriptor {
    let mut size_pt = default_font_size_pt.max(1.0);
    let mut weight = 400_u16;
    let mut style = RegisteredFontStyle::Normal;
    let mut families = Vec::new();

    let tokens: Vec<&str> = descriptor.split_whitespace().collect();
    let mut size_idx = None;
    for (idx, token) in tokens.iter().enumerate() {
        if let Some(value) = token.strip_suffix("pt") {
            if let Ok(size) = value.parse::<f32>() {
                size_pt = size;
                size_idx = Some(idx);
                break;
            }
        }
    }

    if let Some(idx) = size_idx {
        let prefix = tokens[..idx].join(" ");
        let suffix = tokens[idx + 1..].join(" ");
        let prefix_lower = prefix.to_ascii_lowercase();
        if prefix_lower.contains("bold") {
            weight = 700;
        }
        if prefix_lower.contains("italic") {
            style = RegisteredFontStyle::Italic;
        }
        families.extend(parse_family_list(&suffix));
    } else {
        families.extend(parse_family_list(descriptor));
    }

    ParsedFontDescriptor {
        families,
        weight,
        style,
        size_pt,
    }
}

/// Scores registered faces against a parsed descriptor and returns the best
/// match. Earlier families in the stack win, then weight distance, then style.
pub(crate) fn pick_font_index<'a>(
    candidates: impl IntoIterator<Item = (&'a str, u16, RegisteredFontStyle)>,
    spec: &ParsedFontDescriptor,
) -> Option<usize> {
    let mut best: Option<(usize, i32)> = None;
    for (index, (family, weight, style)) in candidates.into_iter().enumerate() {
        let family_position = spec
            .families
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(family));
        let Some(family_position) = family_position else {
            continue;
        };
        let mut score = 10_000 - (family_position as i32 * 1_000);
        score -= (spec.weight as i32 - weight as i32).abs();
        if spec.style == style {
            score += 100;
        }
        if best
            .as_ref()
            .is_none_or(|(_, best_score)| score > *best_score)
        {
            best = Some((index, score));
        }
    }
    best.map(|(index, _)| index)
}

impl TextMeasurer for FontdueMeasurer {
    type Error = Box<dyn Error>;

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        let Some((font, size_pt)) = self.resolve_font(font_descriptor)? else {
            return Ok(0.0);
        };
        let px_size = points_to_px(size_pt) as f32;
        let mut width = 0.0_f64;
        for ch in text.chars() {
            let metrics = font.metrics(ch, px_size);
            width += metrics.advance_width as f64;
        }
        Ok(width)
    }
}

pub struct FontdueLineWrapContext {
    measurer: FontdueMeasurer,
    cache: RefCell<TextMeasureCache>,
    doc_grid_char_spacing_px: Option<f64>,
}

impl FontdueLineWrapContext {
    pub fn new(measurer: FontdueMeasurer, doc_grid_char_spacing_px: Option<f64>) -> Self {
        Self {
            measurer,
            cache: RefCell::new(TextMeasureCache::default()),
            doc_grid_char_spacing_px,
        }
    }
}

impl LineWrapContext for FontdueLineWrapContext {
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
        self.measurer.measure_line_box_height(font_descriptor)
    }

    fn measure_font_vertical_metrics(
        &self,
        font_descriptor: &str,
    ) -> Result<Option<(f64, f64)>, Self::Error> {
        self.measurer.measure_font_vertical_metrics(font_descriptor)
    }
}

pub(crate) fn parse_family_list(families: &str) -> Vec<String> {
    families
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\''))
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

fn fallback_line_metrics(font: &Font, px_size: f32) -> Option<LineMetrics> {
    let sample = font.metrics('あ', px_size);
    if sample.bounds.height > 0.0 {
        Some(LineMetrics {
            ascent: sample.bounds.height,
            descent: 0.0,
            line_gap: 0.0,
            new_line_size: sample.bounds.height,
        })
    } else {
        None
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

    #[test]
    fn parse_descriptor_extracts_size_weight_style_and_families() {
        let measurer = FontdueMeasurer::new(12.0);

        let parsed = measurer.parse_descriptor(r#"italic bold 14pt "Noto Sans JP", serif"#);

        assert_eq!(parsed.size_pt, 14.0);
        assert_eq!(parsed.weight, 700);
        assert_eq!(parsed.style, RegisteredFontStyle::Italic);
        assert_eq!(parsed.families, vec!["Noto Sans JP", "serif"]);
    }

    #[test]
    fn bold_descriptor_without_a_bold_face_measures_like_the_regular_face() {
        // Chromium's fake bold (see tests/fixtures/chromium_fake_bold.json)
        // does not change advance widths, so the bold->regular fallback must
        // return byte-identical measurements rather than synthesizing wider
        // advances.
        let bytes =
            fs::read(bundled_font_path("NotoSansJP-Regular.ttf")).expect("read bundled font");
        let mut measurer = FontdueMeasurer::new(12.0);
        measurer
            .register_font_bytes("Fake Bold Probe", 400, RegisteredFontStyle::Normal, &bytes)
            .expect("register regular face");

        let regular = measurer
            .measure_text("Hello 混在 1,000", "10.50pt 'Fake Bold Probe'")
            .expect("measure regular");
        let bold = measurer
            .measure_text("Hello 混在 1,000", "bold 10.50pt 'Fake Bold Probe'")
            .expect("measure bold fallback");
        assert!(
            (regular - bold).abs() < 1e-9,
            "bold with no bold face must fall back to regular advances: {regular} vs {bold}"
        );
        assert!(regular > 0.0);
    }

    #[test]
    fn pick_font_index_prefers_earlier_matching_family_name() {
        let bytes =
            fs::read(bundled_font_path("NotoSansJP-Regular.ttf")).expect("read bundled font");
        let mut measurer = FontdueMeasurer::new(12.0);
        measurer
            .register_font_bytes("Fallback Family", 400, RegisteredFontStyle::Normal, &bytes)
            .expect("register fallback family");
        measurer
            .register_font_bytes("Noto Sans JP", 400, RegisteredFontStyle::Normal, &bytes)
            .expect("register preferred family");

        let spec = measurer.parse_descriptor(r#"12pt "Noto Sans JP", "Fallback Family""#);

        assert_eq!(measurer.pick_font_index(&spec), Some(1));
    }

    #[test]
    fn measure_line_box_height_returns_positive_for_registered_font() {
        let mut measurer = FontdueMeasurer::new(12.0);
        measurer
            .register_font_path(
                "Noto Sans JP",
                400,
                RegisteredFontStyle::Normal,
                bundled_font_path("NotoSansJP-Regular.ttf"),
            )
            .expect("register bundled Noto Sans JP");

        let height = measurer
            .measure_line_box_height(r#"12pt "Noto Sans JP""#)
            .expect("measure line box height");

        assert!(height.is_some_and(|value| value > 0.0));
    }
}
