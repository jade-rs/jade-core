use crate::{line_wrap::ScriptCategory, units::points_to_px};
use jade_docx_reader::document::{FontHint, RunProperties};
use rustc_hash::{FxHashMap, FxHasher};
use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

const FONT_FAMILY_CACHE_CAPACITY: usize = 256;
const RUN_STYLE_CACHE_CAPACITY: usize = 256;

thread_local! {
    static RESOLVED_FONT_FAMILIES_CACHE: RefCell<FxHashMap<u64, Rc<ResolvedFontFamilies>>> =
        RefCell::new(FxHashMap::default());
    static RESOLVED_RUN_STYLE_CACHE: RefCell<FxHashMap<u64, Rc<ResolvedRunStyle>>> =
        RefCell::new(FxHashMap::default());
    /// `word/fontTable.xml` substitutes for the current document, keyed by
    /// the normalized (lowercased, trimmed) family name.
    static DOCUMENT_FONT_ALT_NAMES: RefCell<FxHashMap<String, Vec<String>>> =
        RefCell::new(FxHashMap::default());
    /// Families the host registered as stand-ins for Word fonts; see
    /// [`set_host_font_fallbacks`].
    static HOST_FONT_FALLBACKS: RefCell<Vec<HostFontFallback>> =
        const { RefCell::new(Vec::new()) };
}

/// Faces a host registered under its own family name to stand in for Word
/// fonts it cannot supply, such as the browser SDK's bundled metric-compatible
/// Yu Mincho substitute.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostFontFallback {
    /// CSS family the host registered the substitute faces under.
    pub family: String,
    /// Word font names the family stands in for, e.g. `游明朝` / `Yu Mincho`.
    pub substitutes: Vec<String>,
}

/// Installs this thread's host font fallbacks. A run that asks for one of a
/// fallback's `substitutes` gets the fallback family right after the last of
/// those names in its stack: an installed copy of the Word font still wins,
/// and the fallback outranks the platform look-alikes after it (macOS's
/// YuMincho, Hiragino, Noto).
/// Runs asking for other fonts are unaffected even when their alias stack
/// mentions a substituted name. Resolver caches are cleared on change.
pub fn set_host_font_fallbacks<I>(fallbacks: I)
where
    I: IntoIterator<Item = HostFontFallback>,
{
    let normalized: Vec<HostFontFallback> = fallbacks
        .into_iter()
        .filter_map(|fallback| {
            let family = fallback.family.trim().to_string();
            let substitutes: Vec<String> = fallback
                .substitutes
                .iter()
                .map(|name| normalize_font_identifier(name))
                .filter(|name| !name.is_empty())
                .collect();
            (!family.is_empty() && !substitutes.is_empty()).then_some(HostFontFallback {
                family,
                substitutes,
            })
        })
        .collect();
    HOST_FONT_FALLBACKS.with(|registry| {
        let mut registry = registry.borrow_mut();
        if *registry == normalized {
            return;
        }
        *registry = normalized;
        RESOLVED_FONT_FAMILIES_CACHE.with(|cache| cache.borrow_mut().clear());
        RESOLVED_RUN_STYLE_CACHE.with(|cache| cache.borrow_mut().clear());
    });
}

/// Installs the current document's `w:altName` substitution map (from
/// `word/fontTable.xml`) for this thread's font resolution. Call with an
/// empty iterator when the document has no font table so a previously loaded
/// document's substitutes cannot leak. Resolver caches are cleared because
/// the same run properties now resolve to different stacks.
pub fn set_document_font_alt_names<I>(alt_names: I)
where
    I: IntoIterator<Item = (String, Vec<String>)>,
{
    let normalized: FxHashMap<String, Vec<String>> = alt_names
        .into_iter()
        .filter_map(|(name, substitutes)| {
            let key = name.trim().to_ascii_lowercase();
            let substitutes: Vec<String> = substitutes
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
            (!key.is_empty() && !substitutes.is_empty()).then_some((key, substitutes))
        })
        .collect();
    DOCUMENT_FONT_ALT_NAMES.with(|registry| {
        let mut registry = registry.borrow_mut();
        if *registry == normalized {
            return;
        }
        *registry = normalized;
        RESOLVED_FONT_FAMILIES_CACHE.with(|cache| cache.borrow_mut().clear());
        RESOLVED_RUN_STYLE_CACHE.with(|cache| cache.borrow_mut().clear());
    });
}

fn document_font_alt_names(name: &str) -> Vec<String> {
    let key = name.trim().to_ascii_lowercase();
    DOCUMENT_FONT_ALT_NAMES
        .with(|registry| registry.borrow().get(&key).cloned().unwrap_or_default())
}

/// Fallback CSS font stack for Latin script.
pub const DEFAULT_LATIN_FONT_STACK: &str = "'Times New Roman', serif";
/// Fallback CSS font stack optimized for East Asian glyphs.
pub const DEFAULT_EAST_ASIA_FONT_STACK: &str = "'MS Mincho', 'Yu Mincho', 'Hiragino Mincho ProN', 'Hiragino Mincho Pro', 'Noto Serif CJK JP', serif";
/// Capture fallback stack for complex scripts with limited document hints.
pub const DEFAULT_COMPLEX_FONT_STACK: &str = "'Times New Roman', serif";

/// Resolved font stacks for each script family, deduped and normalized.
/// Word distinguishes ASCII (w:ascii) and HAnsi (w:hAnsi) even within Latin.
/// We keep both stacks so rendering can follow Word's per-codepoint choice.
#[derive(Clone, Debug)]
pub struct ResolvedFontFamilies {
    latin_ascii: String,
    latin_hansi: String,
    east_asia: String,
    complex: String,
}

/// CSS font descriptors, including computed font-size and style prefix.
#[derive(Clone, Debug)]
pub struct FontDescriptorSet {
    latin_ascii: Rc<str>,
    latin_hansi: Rc<str>,
    east_asia: Rc<str>,
    complex: Rc<str>,
}

impl FontDescriptorSet {
    pub fn for_script(&self, script: ScriptCategory) -> &str {
        match script {
            ScriptCategory::Latin => &self.latin_ascii,
            ScriptCategory::EastAsia => &self.east_asia,
            ScriptCategory::Complex => &self.complex,
        }
    }

    pub fn latin_ascii(&self) -> &str {
        &self.latin_ascii
    }

    pub fn latin_hansi(&self) -> &str {
        &self.latin_hansi
    }

    pub fn for_script_rc(&self, script: ScriptCategory) -> Rc<str> {
        match script {
            ScriptCategory::Latin => Rc::clone(&self.latin_ascii),
            ScriptCategory::EastAsia => Rc::clone(&self.east_asia),
            ScriptCategory::Complex => Rc::clone(&self.complex),
        }
    }

    pub fn latin_ascii_rc(&self) -> Rc<str> {
        Rc::clone(&self.latin_ascii)
    }

    pub fn latin_hansi_rc(&self) -> Rc<str> {
        Rc::clone(&self.latin_hansi)
    }

    pub fn descriptor_for_grapheme(
        &self,
        grapheme: &str,
        script: ScriptCategory,
        hint: Option<&FontHint>,
    ) -> &str {
        match apply_hint_to_script(script, hint, grapheme) {
            ScriptCategory::Latin => {
                if grapheme.is_ascii() {
                    self.latin_ascii()
                } else {
                    self.latin_hansi()
                }
            }
            ScriptCategory::EastAsia => self.for_script(ScriptCategory::EastAsia),
            ScriptCategory::Complex => self.for_script(ScriptCategory::Complex),
        }
    }
}

impl ResolvedFontFamilies {
    pub fn for_script(&self, script: ScriptCategory) -> &str {
        match script {
            ScriptCategory::Latin => &self.latin_ascii,
            ScriptCategory::EastAsia => &self.east_asia,
            ScriptCategory::Complex => &self.complex,
        }
    }

    pub fn latin_ascii_stack(&self) -> &str {
        &self.latin_ascii
    }

    pub fn latin_hansi_stack(&self) -> &str {
        &self.latin_hansi
    }

    pub fn latin_stack(&self) -> &str {
        &self.latin_ascii
    }

    pub fn east_asia_stack(&self) -> &str {
        &self.east_asia
    }

    pub fn complex_stack(&self) -> &str {
        &self.complex
    }

    pub fn to_descriptor_set(&self, style_prefix: &str, font_size_pt: f32) -> FontDescriptorSet {
        self.to_descriptor_set_with_sizes(style_prefix, font_size_pt, font_size_pt)
    }

    /// Builds the per-script descriptors with a separate complex-script size:
    /// `w:szCs` sizes complex-script glyphs independently of `w:sz`.
    pub fn to_descriptor_set_with_sizes(
        &self,
        style_prefix: &str,
        font_size_pt: f32,
        complex_font_size_pt: f32,
    ) -> FontDescriptorSet {
        self.to_descriptor_set_with_styles(
            style_prefix,
            style_prefix,
            font_size_pt,
            complex_font_size_pt,
        )
    }

    /// Builds the per-script descriptors with the complex-script face styled
    /// on its own: `w:bCs`/`w:iCs` bold and italicize only complex-script
    /// glyphs, `w:b`/`w:i` the others (ECMA-376 Part 1 §17.3.2.2, §17.3.2.6).
    pub fn to_descriptor_set_with_styles(
        &self,
        style_prefix: &str,
        complex_style_prefix: &str,
        font_size_pt: f32,
        complex_font_size_pt: f32,
    ) -> FontDescriptorSet {
        FontDescriptorSet {
            latin_ascii: Rc::from(format!(
                "{}{:0.2}pt {}",
                style_prefix, font_size_pt, self.latin_ascii
            )),
            latin_hansi: Rc::from(format!(
                "{}{:0.2}pt {}",
                style_prefix, font_size_pt, self.latin_hansi
            )),
            east_asia: Rc::from(format!(
                "{}{:0.2}pt {}",
                style_prefix, font_size_pt, self.east_asia
            )),
            complex: Rc::from(format!(
                "{}{:0.2}pt {}",
                complex_style_prefix, complex_font_size_pt, self.complex
            )),
        }
    }
}

fn format_font_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains('"') || trimmed.contains('\'') {
        trimmed.to_string()
    } else if trimmed.contains(',') || trimmed.contains(' ') {
        // Quote comma-containing names too: emitting a bare comma inside
        // one entry would corrupt the generated CSS font list.
        format!("'{}'", trimmed)
    } else {
        trimmed.to_string()
    }
}

fn normalize_font_identifier(name: &str) -> String {
    name.trim()
        .trim_matches('\'')
        .trim_matches('"')
        .trim()
        .to_ascii_lowercase()
}

/// Places each host fallback for one of `requested` right after the last of
/// its substituted names already in `parts`.
fn insert_host_font_fallbacks(
    parts: &mut Vec<String>,
    seen: &mut HashSet<String>,
    requested: &[String],
) {
    HOST_FONT_FALLBACKS.with(|registry| {
        for fallback in registry.borrow().iter() {
            if !requested
                .iter()
                .any(|name| fallback.substitutes.contains(name))
            {
                continue;
            }
            if !seen.insert(normalize_font_identifier(&fallback.family)) {
                continue;
            }
            let position = parts
                .iter()
                .rposition(|part| {
                    fallback
                        .substitutes
                        .contains(&normalize_font_identifier(part))
                })
                .map_or(parts.len(), |index| index + 1);
            parts.insert(position, format_font_name(&fallback.family));
        }
    });
}

/// Normalizes font lists by quoting multi-word names and deduplicating aliases.
fn normalize_font_stack(value: Option<&str>, fallback: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut push_name = |raw: &str| {
        let formatted = format_font_name(raw);
        if formatted.is_empty() {
            return;
        }
        let key = normalize_font_identifier(&formatted);
        if seen.insert(key) {
            parts.push(formatted);
        }
    };

    if let Some(candidate) = value
        .map(|v| v.trim())
        .filter(|v| !v.is_empty() && !v.starts_with('+'))
    {
        push_name(candidate);
        // The document's own `w:altName` substitutes outrank the built-in
        // alias table: they are what Word on the authoring machine actually
        // fell back to.
        let alt_names = document_font_alt_names(candidate);
        for alt in &alt_names {
            push_name(alt);
        }
        for alias in font_aliases(candidate) {
            push_name(alias);
        }
        let requested: Vec<String> = std::iter::once(candidate)
            .chain(alt_names.iter().map(String::as_str))
            .map(normalize_font_identifier)
            .collect();
        insert_host_font_fallbacks(&mut parts, &mut seen, &requested);
    }

    if !fallback.trim().is_empty() {
        let fallback_key = normalize_font_identifier(fallback);
        if seen.insert(fallback_key) {
            parts.push(fallback.to_string());
        }
    }

    parts.join(", ")
}

fn hash_font_hint(hint: Option<&FontHint>, hasher: &mut FxHasher) {
    let tag = match hint {
        Some(FontHint::EastAsia) => 1_u8,
        Some(FontHint::Complex) => 2_u8,
        Some(FontHint::Default) => 3_u8,
        None => 0_u8,
    };
    tag.hash(hasher);
}

fn hash_run_properties(properties: Option<&RunProperties>, hasher: &mut FxHasher) {
    // Japanese missing-glyph fallback differs between Word platforms.
    (crate::line_metrics::word_layout_platform() == crate::line_metrics::WordLayoutPlatform::Mac)
        .hash(hasher);
    if let Some(props) = properties {
        props.style.hash(hasher);
        props.bold.hash(hasher);
        props.italic.hash(hasher);
        props.underline.hash(hasher);
        props.strike.hash(hasher);
        props.baseline.hash(hasher);
        props.char_spacing.map(f32::to_bits).hash(hasher);
        props.kerning.map(f32::to_bits).hash(hasher);
        props.text_scale.map(f32::to_bits).hash(hasher);
        props
            .fit_text
            .as_ref()
            .map(|fit| (fit.val, fit.id))
            .hash(hasher);
        props.font_size.map(f32::to_bits).hash(hasher);
        props.font_size_complex.map(f32::to_bits).hash(hasher);
        props.bold_complex.hash(hasher);
        props.italic_complex.hash(hasher);
        props.font_family.hash(hasher);
        props.language.hash(hasher);
        props.east_asia_language.hash(hasher);
        props.color.hash(hasher);
        opentype_features_token(props, false).hash(hasher);
        if let Some(fonts) = props.fonts.as_ref() {
            fonts.ascii.hash(hasher);
            fonts.h_ansi.hash(hasher);
            fonts.east_asia.hash(hasher);
            fonts.complex.hash(hasher);
            // Theme keys participate so two runs differing only by
            // `asciiTheme` cannot collide once theme-aware resolution lands.
            fonts.ascii_theme.hash(hasher);
            fonts.h_ansi_theme.hash(hasher);
            fonts.east_asia_theme.hash(hasher);
            fonts.complex_theme.hash(hasher);
            hash_font_hint(fonts.hint.as_ref(), hasher);
        } else {
            0_u8.hash(hasher);
        }
    } else {
        0_u8.hash(hasher);
    }
}

fn compute_font_families_cache_key(properties: Option<&RunProperties>) -> u64 {
    let mut hasher = FxHasher::default();
    hash_run_properties(properties, &mut hasher);
    hasher.finish()
}

fn compute_run_style_cache_key(
    properties: Option<&RunProperties>,
    doc_grid_char_spacing_px: Option<f64>,
) -> u64 {
    let mut hasher = FxHasher::default();
    hash_run_properties(properties, &mut hasher);
    doc_grid_char_spacing_px.map(f64::to_bits).hash(&mut hasher);
    hasher.finish()
}

fn cache_insert_bounded<T: Clone>(
    cache: &mut FxHashMap<u64, T>,
    capacity: usize,
    key: u64,
    value: T,
) {
    if cache.len() >= capacity {
        if let Some(evict_key) = cache.keys().next().copied() {
            cache.remove(&evict_key);
        }
    }
    cache.insert(key, value);
}

/// Returns known aliases for Japanese font names to provide fallbacks.
fn font_aliases(name: &str) -> &'static [&'static str] {
    let trimmed = name.trim();
    match trimmed {
        // Word for Mac 16.113 draws `Mincho`, the face's old name, in MS Mincho.
        "ＭＳ 明朝" | "MS 明朝" | "MS Mincho" | "ＭＳ明朝" | "ＭＳ Ｐ明朝" | "MS P明朝"
        | "ＭＳＰ明朝" | "MS PMincho" | "Mincho" => &[
            "MS Mincho",
            "MS PMincho",
            "Hiragino Mincho ProN W3",
            "Hiragino Mincho ProN W6",
            "Hiragino Mincho Pro W3",
            "Hiragino Mincho Pro W6",
            "Hiragino Mincho ProN",
            "Hiragino Mincho Pro",
            "Yu Mincho",
            "YuMincho",
            // IPA Mincho keeps MS Mincho's fixed half-width Latin and digits;
            // Noto Serif CJK's proportional digits widen every number.
            "IPAMincho",
            "Noto Serif CJK JP",
        ],
        "游明朝" | "Yu Mincho" | "YuMincho" => &[
            "Yu Mincho",
            "YuMincho",
            "Hiragino Mincho ProN W3",
            "Hiragino Mincho ProN W6",
            "Hiragino Mincho Pro W3",
            "Hiragino Mincho Pro W6",
            "Hiragino Mincho ProN",
            "Hiragino Mincho Pro",
            "MS Mincho",
            "Noto Serif CJK JP",
        ],
        "ＭＳ ゴシック"
        | "MS ゴシック"
        | "MS Gothic"
        | "ＭＳゴシック"
        | "ＭＳ Ｐゴシック"
        | "MS P Gothic"
        | "ＭＳＰゴシック"
        | "MS PGothic" => &[
            "MS Gothic",
            "MS PGothic",
            "Hiragino Sans W3",
            "Hiragino Sans W6",
            "Hiragino Kaku Gothic ProN W3",
            "Hiragino Kaku Gothic ProN W6",
            "Hiragino Sans",
            "Hiragino Kaku Gothic ProN",
            "Yu Gothic",
            "YuGothic",
            "Meiryo",
            // IPA Gothic keeps MS Gothic's fixed half-width Latin and digits.
            "IPAGothic",
            "Noto Sans CJK JP",
        ],
        "メイリオ" | "Meiryo" | "Meiryo UI" | "MS UI Gothic" => &[
            "Meiryo",
            "Hiragino Sans W3",
            "Hiragino Sans W6",
            "Hiragino Kaku Gothic ProN W3",
            "Hiragino Kaku Gothic ProN W6",
            "Hiragino Sans",
            "Hiragino Kaku Gothic ProN",
            "Yu Gothic",
            "YuGothic",
            "Noto Sans CJK JP",
        ],
        "游ゴシック" | "Yu Gothic" | "YuGothic" => &[
            "Yu Gothic",
            "YuGothic",
            "Hiragino Sans W3",
            "Hiragino Sans W6",
            "Hiragino Kaku Gothic ProN W3",
            "Hiragino Kaku Gothic ProN W6",
            "Hiragino Sans",
            "Hiragino Kaku Gothic ProN",
            "Meiryo",
            "Noto Sans CJK JP",
        ],
        "Noto Sans" => &[
            "Noto Sans JP",
            "Noto Sans CJK JP",
            "Noto Sans KR",
            "Noto Sans CJK KR",
            "Noto Sans SC",
            "Noto Sans CJK SC",
            "Noto Sans TC",
            "Noto Sans CJK TC",
            "Noto Sans HK",
            "Noto Sans CJK HK",
        ],
        "Noto Serif" => &[
            "Noto Serif JP",
            "Noto Serif CJK JP",
            "Noto Serif KR",
            "Noto Serif CJK KR",
            "Noto Serif SC",
            "Noto Serif CJK SC",
            "Noto Serif TC",
            "Noto Serif CJK TC",
            "Noto Serif HK",
            "Noto Serif CJK HK",
        ],
        "Noto Sans JP" => &["Noto Sans CJK JP"],
        "Noto Sans CJK JP" => &["Noto Sans JP"],
        "Noto Sans KR" => &["Noto Sans CJK KR"],
        "Noto Sans CJK KR" => &["Noto Sans KR"],
        "Noto Sans SC" => &["Noto Sans CJK SC"],
        "Noto Sans CJK SC" => &["Noto Sans SC"],
        "Noto Sans TC" => &["Noto Sans CJK TC"],
        "Noto Sans CJK TC" => &["Noto Sans TC"],
        "Noto Sans HK" => &["Noto Sans CJK HK"],
        "Noto Sans CJK HK" => &["Noto Sans HK"],
        "Noto Serif JP" => &["Noto Serif CJK JP"],
        "Noto Serif CJK JP" => &["Noto Serif JP"],
        "Noto Serif KR" => &["Noto Serif CJK KR"],
        "Noto Serif CJK KR" => &["Noto Serif KR"],
        "Noto Serif SC" => &["Noto Serif CJK SC"],
        "Noto Serif CJK SC" => &["Noto Serif SC"],
        "Noto Serif TC" => &["Noto Serif CJK TC"],
        "Noto Serif CJK TC" => &["Noto Serif TC"],
        "Noto Serif HK" => &["Noto Serif CJK HK"],
        "Noto Serif CJK HK" => &["Noto Serif HK"],
        "ヒラギノ角ゴ ProN W3" => {
            &["Hiragino Kaku Gothic ProN W3", "Hiragino Kaku Gothic ProN"]
        }
        "ヒラギノ角ゴ ProN W6" => {
            &["Hiragino Kaku Gothic ProN W6", "Hiragino Kaku Gothic ProN"]
        }
        "Arial" => &[
            "Arial Unicode MS",
            "Hiragino Sans W3",
            "Hiragino Sans W6",
            "Hiragino Sans",
            "Noto Sans JP",
            "Noto Sans CJK JP",
        ],
        "Arial Unicode MS" => &[
            "Arial",
            "Hiragino Sans W3",
            "Hiragino Sans W6",
            "Hiragino Sans",
            "Noto Sans JP",
            "Noto Sans CJK JP",
        ],
        "Calibri" => &[
            "Hiragino Sans W3",
            "Hiragino Sans W6",
            "Hiragino Sans",
            "Noto Sans JP",
            "Noto Sans CJK JP",
        ],
        _ => &[],
    }
}

/// Builds a `ResolvedFontFamilies` instance from the run's font preferences and hints.
pub fn resolve_font_families(properties: Option<&RunProperties>) -> Rc<ResolvedFontFamilies> {
    let cache_key = compute_font_families_cache_key(properties);
    if let Some(cached) =
        RESOLVED_FONT_FAMILIES_CACHE.with(|cache| cache.borrow().get(&cache_key).cloned())
    {
        return cached;
    }
    let resolved = Rc::new(resolve_font_families_uncached(properties));
    RESOLVED_FONT_FAMILIES_CACHE.with(|cache| {
        cache_insert_bounded(
            &mut cache.borrow_mut(),
            FONT_FAMILY_CACHE_CAPACITY,
            cache_key,
            resolved.clone(),
        );
    });
    resolved
}

fn resolve_font_families_uncached(properties: Option<&RunProperties>) -> ResolvedFontFamilies {
    let mut latin_candidate: Option<&str> = None;
    let mut ascii_candidate: Option<&str> = None;
    let mut hansi_candidate: Option<&str> = None;
    let mut fonts_ref = None;
    if let Some(props) = properties {
        if let Some(fonts) = props.fonts.as_ref() {
            // Prefer explicit rFonts overrides first (Word behavior).
            ascii_candidate = fonts.ascii.as_deref();
            hansi_candidate = fonts.h_ansi.as_deref();
            latin_candidate = fonts.preferred_latin();
            fonts_ref = Some(fonts);
        }
        if latin_candidate.is_none() {
            latin_candidate = props.font_family.as_deref();
        }
        if ascii_candidate.is_none() {
            // Fall back to a single font_family if explicit ascii is missing.
            ascii_candidate = props.font_family.as_deref();
        }
        if hansi_candidate.is_none() {
            // Fall back to a single font_family if explicit hAnsi is missing.
            hansi_candidate = props.font_family.as_deref();
        }
    }

    if ascii_candidate.is_none() {
        // Word treats missing ascii/hAnsi as aliases of each other.
        ascii_candidate = hansi_candidate;
    }
    if hansi_candidate.is_none() {
        hansi_candidate = ascii_candidate;
    }

    let latin_ascii_stack = normalize_font_stack(ascii_candidate, DEFAULT_LATIN_FONT_STACK);
    let latin_hansi_stack = normalize_font_stack(hansi_candidate, DEFAULT_LATIN_FONT_STACK);

    let hint = fonts_ref.and_then(|fonts| fonts.hint.as_ref());

    let east_candidate = fonts_ref
        .and_then(|fonts| fonts.preferred_east_asia())
        .or(match hint {
            Some(FontHint::EastAsia) => latin_candidate,
            _ => None,
        })
        .or(latin_candidate);
    // Noto Sans is a Latin face, not a locale-specific Noto CJK face. Word
    // for Mac's Japanese Noto PDF uses MS Mincho for its missing CJK glyphs;
    // treating Noto Sans JP as an interchangeable alias changes the box.
    // Explicit Noto Sans JP/CJK requests and other languages keep their stacks.
    let japanese = properties
        .and_then(|props| {
            props
                .east_asia_language
                .as_deref()
                .or(props.language.as_deref())
        })
        .is_some_and(|language| {
            language.eq_ignore_ascii_case("ja")
                || language
                    .get(..3)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("ja-"))
        });
    let east_candidate = if japanese
        && crate::line_metrics::word_layout_platform()
            == crate::line_metrics::WordLayoutPlatform::Mac
        && east_candidate.is_some_and(|family| family.trim() == "Noto Sans")
    {
        Some("MS Mincho")
    } else {
        east_candidate
    };
    let east_stack = normalize_font_stack(east_candidate, DEFAULT_EAST_ASIA_FONT_STACK);

    let complex_candidate = fonts_ref
        .and_then(|fonts| fonts.preferred_complex())
        .or(match hint {
            Some(FontHint::Complex) => latin_candidate,
            _ => None,
        })
        .or(latin_candidate);
    let complex_stack = normalize_font_stack(complex_candidate, DEFAULT_COMPLEX_FONT_STACK);

    ResolvedFontFamilies {
        latin_ascii: latin_ascii_stack,
        latin_hansi: latin_hansi_stack,
        east_asia: east_stack,
        complex: complex_stack,
    }
}

/// The OpenType features a run's Word 2010 typography asks for, as the
/// descriptor token [`jade_painter::font_features`] defines — `None` when the
/// run asks for none, so ordinary text keeps its descriptor (and every cache
/// keyed on it) unchanged.
///
/// Word applies only what the フォント dialog's 詳細設定 tab turns on: with
/// any of it set, the shaper's own defaults (`liga`, `clig`, `calt`, and
/// `kern` without `w:kern`) are turned off so they do not join or shift
/// letters Word leaves alone.
pub fn opentype_features_token(props: &RunProperties, pair_kerning: bool) -> Option<String> {
    use jade_painter::font_features::{token, FontFeature};
    let effects = props.text_effects.as_deref()?;
    let on = |tag: &[u8; 4]| FontFeature {
        tag: *tag,
        value: 1,
    };
    let off = |tag: &[u8; 4]| FontFeature {
        tag: *tag,
        value: 0,
    };
    let mut features: Vec<FontFeature> = Vec::new();
    let ligatures = effects.ligatures.as_deref().unwrap_or("none");
    let all = ligatures == "all";
    let lower = ligatures.to_ascii_lowercase();
    let standard = all || lower.contains("standard");
    let contextual = all || lower.contains("contextual");
    let historical = all || lower.contains("historical");
    let discretional = all || lower.contains("discretional");
    if standard {
        features.push(on(b"liga"));
    }
    if contextual {
        features.push(on(b"clig"));
    }
    if historical {
        features.push(on(b"hlig"));
    }
    if discretional {
        features.push(on(b"dlig"));
    }
    match effects.number_form.as_deref() {
        Some("lining") => features.push(on(b"lnum")),
        Some("oldStyle") => features.push(on(b"onum")),
        _ => {}
    }
    match effects.number_spacing.as_deref() {
        Some("proportional") => features.push(on(b"pnum")),
        Some("tabular") => features.push(on(b"tnum")),
        _ => {}
    }
    if let Some(set) = effects.stylistic_set.filter(|set| (1..=20).contains(set)) {
        let tag = format!("ss{set:02}");
        let bytes = tag.as_bytes();
        features.push(on(&[bytes[0], bytes[1], bytes[2], bytes[3]]));
    }
    let alternates = effects.contextual_alternates == Some(true);
    if alternates {
        features.push(on(b"calt"));
    }
    if features.is_empty() {
        return None;
    }
    if !standard {
        features.push(off(b"liga"));
    }
    if !contextual {
        features.push(off(b"clig"));
    }
    if !alternates {
        features.push(off(b"calt"));
    }
    if !pair_kerning {
        features.push(off(b"kern"));
    }
    token(&features)
}

/// Whether a resolved style's descriptors carry features that join letters
/// (ligatures, contextual alternates), so a pair of letters can measure other
/// than the sum of the two.
pub fn descriptor_joins_characters(descriptor: &str) -> bool {
    jade_painter::font_features::has_features(descriptor)
        && jade_painter::font_features::joins_characters(&jade_painter::font_features::parse(
            descriptor,
        ))
}

/// Returns a `ResolvedRunStyle` containing descriptors and letter spacing for layout pipelines.
pub fn resolve_run_style_for_wrap(
    properties: Option<&RunProperties>,
    doc_grid_char_spacing_px: Option<f64>,
) -> Rc<ResolvedRunStyle> {
    let cache_key = compute_run_style_cache_key(properties, doc_grid_char_spacing_px);
    if let Some(cached) =
        RESOLVED_RUN_STYLE_CACHE.with(|cache| cache.borrow().get(&cache_key).cloned())
    {
        return cached;
    }
    let resolved = Rc::new(resolve_run_style_for_wrap_uncached(
        properties,
        doc_grid_char_spacing_px,
    ));
    RESOLVED_RUN_STYLE_CACHE.with(|cache| {
        cache_insert_bounded(
            &mut cache.borrow_mut(),
            RUN_STYLE_CACHE_CAPACITY,
            cache_key,
            resolved.clone(),
        );
    });
    resolved
}

fn resolve_run_style_for_wrap_uncached(
    properties: Option<&RunProperties>,
    doc_grid_char_spacing_px: Option<f64>,
) -> ResolvedRunStyle {
    let mut font_style = String::new();
    let mut complex_font_style = String::new();
    let mut font_size_pt = 12.0_f32;
    let mut font_size_complex_pt: Option<f32> = None;
    let mut letter_spacing_px = 0.0_f64;
    let mut text_scale_x = 1.0_f64;
    let mut enable_pair_kerning = false;

    if let Some(props) = properties {
        if props.bold.unwrap_or(false) {
            font_style.push_str("bold ");
        }
        if props.bold_complex.unwrap_or(false) {
            complex_font_style.push_str("bold ");
        }

        if props.italic.unwrap_or(false) {
            font_style.push_str("italic ");
        }
        if props.italic_complex.unwrap_or(false) {
            complex_font_style.push_str("italic ");
        }

        if let Some(size) = props.font_size {
            font_size_pt = size;
        }

        // `w:szCs` sizes complex-script glyphs independently of `w:sz`.
        if let Some(size) = props.font_size_complex {
            font_size_complex_pt = Some(size);
        }

        if let Some(spacing) = props.char_spacing {
            letter_spacing_px = points_to_px(spacing);
        }

        if let Some(scale) = props.text_scale {
            if scale.is_finite() && scale > 0.0 {
                text_scale_x = (scale as f64 / 100.0).clamp(0.01, 10.0);
            }
        }

        if let Some(min_points) = props.kerning {
            if min_points.is_finite() && min_points > 0.0 {
                enable_pair_kerning = (font_size_pt as f64) >= min_points as f64;
            }
        }

        if let Some(token) = opentype_features_token(props, enable_pair_kerning) {
            font_style.insert_str(0, &format!("{token} "));
            complex_font_style.insert_str(0, &format!("{token} "));
        }

        if let Some("superscript" | "super" | "subscript" | "sub") = props.baseline.as_deref() {
            let families = resolve_font_families(properties);
            let stack = families.latin_ascii_stack();
            font_size_pt = crate::line_metrics::word_shifted_script_size_pt(font_size_pt, stack);
            font_size_complex_pt = font_size_complex_pt
                .map(|size| crate::line_metrics::word_shifted_script_size_pt(size, stack));
        }
    }

    if letter_spacing_px.abs() < f64::EPSILON {
        if let Some(spacing) = doc_grid_char_spacing_px {
            letter_spacing_px = spacing;
        }
    }

    let families = resolve_font_families(properties);
    let descriptors = families.to_descriptor_set_with_styles(
        &font_style,
        &complex_font_style,
        font_size_pt,
        font_size_complex_pt.unwrap_or(font_size_pt),
    );

    ResolvedRunStyle::new(
        descriptors.latin_ascii_rc(),
        descriptors.latin_hansi_rc(),
        descriptors.for_script_rc(crate::line_wrap::ScriptCategory::EastAsia),
        descriptors.for_script_rc(crate::line_wrap::ScriptCategory::Complex),
        letter_spacing_px,
        text_scale_x,
        enable_pair_kerning,
        properties
            .as_ref()
            .and_then(|props| props.fonts.as_ref())
            .and_then(|fonts| fonts.hint.clone()),
    )
}

/// One of the four resolved font descriptors carried by a run style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedFontSlot {
    LatinAscii,
    LatinHansi,
    EastAsia,
    Complex,
}

#[derive(Clone, Debug)]
struct CachedAsciiMetrics {
    descriptor: Rc<str>,
    metrics: Option<crate::line_metrics::WordAsciiMetrics>,
}

/// Runtime style metadata exposed to `LineWrapContext`.
#[derive(Clone, Debug)]
pub struct ResolvedRunStyle {
    pub font_descriptor_latin_ascii: Rc<str>,
    pub font_descriptor_latin_hansi: Rc<str>,
    pub font_descriptor_east_asia: Rc<str>,
    pub font_descriptor_complex: Rc<str>,
    pub font_descriptor_latin_ascii_hash: u64,
    pub font_descriptor_latin_hansi_hash: u64,
    pub font_descriptor_east_asia_hash: u64,
    pub font_descriptor_complex_hash: u64,
    /// Renderer-owned measurement-cache ids. Generic layout users leave these
    /// unset; renderers with a shared descriptor interner attach them once when
    /// resolving a style so per-grapheme cache hits can skip interning entirely.
    pub font_descriptor_latin_ascii_id: Option<u32>,
    pub font_descriptor_latin_hansi_id: Option<u32>,
    pub font_descriptor_east_asia_id: Option<u32>,
    pub font_descriptor_complex_id: Option<u32>,
    pub letter_spacing_px: f64,
    pub text_scale_x: f64,
    pub enable_pair_kerning: bool,
    pub font_hint: Option<FontHint>,
    /// Whether the descriptors ask for ligatures or contextual alternates
    /// (see [`descriptor_joins_characters`]): letter pairs are then measured
    /// as kerning is, so a ligature's width reaches layout.
    pub joins_characters: bool,
    fixed_ascii_metrics: std::cell::OnceCell<CachedAsciiMetrics>,
    fixed_east_asia_descriptor: Option<Rc<str>>,
    fixed_east_asia_em_px: f64,
}

impl ResolvedRunStyle {
    /// Whether adjacent letters are measured as a pair: for `w:kern`, or for
    /// OpenType features that join letters.
    #[inline]
    pub fn measures_letter_pairs(&self) -> bool {
        self.enable_pair_kerning || self.joins_characters
    }

    #[inline]
    pub fn font_slot_for_grapheme(
        &self,
        grapheme: &str,
        script: ScriptCategory,
    ) -> ResolvedFontSlot {
        match apply_hint_to_script(script, self.font_hint.as_ref(), grapheme) {
            ScriptCategory::Latin if grapheme.is_ascii() => ResolvedFontSlot::LatinAscii,
            ScriptCategory::Latin => ResolvedFontSlot::LatinHansi,
            ScriptCategory::EastAsia => ResolvedFontSlot::EastAsia,
            ScriptCategory::Complex => ResolvedFontSlot::Complex,
        }
    }

    #[inline]
    pub fn descriptor_for_slot(&self, slot: ResolvedFontSlot) -> &Rc<str> {
        match slot {
            ResolvedFontSlot::LatinAscii => &self.font_descriptor_latin_ascii,
            ResolvedFontSlot::LatinHansi => &self.font_descriptor_latin_hansi,
            ResolvedFontSlot::EastAsia => &self.font_descriptor_east_asia,
            ResolvedFontSlot::Complex => &self.font_descriptor_complex,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        font_descriptor_latin_ascii: impl Into<Rc<str>>,
        font_descriptor_latin_hansi: impl Into<Rc<str>>,
        font_descriptor_east_asia: impl Into<Rc<str>>,
        font_descriptor_complex: impl Into<Rc<str>>,
        letter_spacing_px: f64,
        text_scale_x: f64,
        enable_pair_kerning: bool,
        font_hint: Option<FontHint>,
    ) -> Self {
        let font_descriptor_latin_ascii: Rc<str> = font_descriptor_latin_ascii.into();
        let font_descriptor_latin_hansi: Rc<str> = font_descriptor_latin_hansi.into();
        let font_descriptor_east_asia: Rc<str> = font_descriptor_east_asia.into();
        let font_descriptor_complex: Rc<str> = font_descriptor_complex.into();
        let joins_characters = descriptor_joins_characters(&font_descriptor_latin_ascii)
            || descriptor_joins_characters(&font_descriptor_latin_hansi)
            || descriptor_joins_characters(&font_descriptor_east_asia);
        let font_descriptor_latin_ascii_hash = hash_font_descriptor(&font_descriptor_latin_ascii);
        let font_descriptor_latin_hansi_hash = hash_font_descriptor(&font_descriptor_latin_hansi);
        let font_descriptor_east_asia_hash = hash_font_descriptor(&font_descriptor_east_asia);
        let font_descriptor_complex_hash = hash_font_descriptor(&font_descriptor_complex);
        let (fixed_east_asia_descriptor, fixed_east_asia_em_px) = if enable_pair_kerning {
            (
                Some(Rc::clone(&font_descriptor_east_asia)),
                crate::line_metrics::word_fixed_east_asian_em_px(&font_descriptor_east_asia)
                    .unwrap_or(0.0),
            )
        } else {
            (None, 0.0)
        };
        Self {
            font_descriptor_latin_ascii,
            font_descriptor_latin_hansi,
            font_descriptor_east_asia,
            font_descriptor_complex,
            font_descriptor_latin_ascii_hash,
            font_descriptor_latin_hansi_hash,
            font_descriptor_east_asia_hash,
            font_descriptor_complex_hash,
            font_descriptor_latin_ascii_id: None,
            font_descriptor_latin_hansi_id: None,
            font_descriptor_east_asia_id: None,
            font_descriptor_complex_id: None,
            letter_spacing_px,
            text_scale_x,
            enable_pair_kerning,
            font_hint,
            joins_characters,
            fixed_ascii_metrics: std::cell::OnceCell::new(),
            fixed_east_asia_descriptor,
            fixed_east_asia_em_px,
        }
    }

    #[inline]
    pub(crate) fn fixed_east_asia_em_px(&self) -> Option<f64> {
        if !self.enable_pair_kerning {
            return None;
        }
        if self
            .fixed_east_asia_descriptor
            .as_ref()
            .is_some_and(|descriptor| Rc::ptr_eq(descriptor, &self.font_descriptor_east_asia))
        {
            (self.fixed_east_asia_em_px > 0.0).then_some(self.fixed_east_asia_em_px)
        } else {
            // Public descriptor/kerning changes may leave missing or stale metadata.
            crate::line_metrics::word_fixed_east_asian_em_px(&self.font_descriptor_east_asia)
        }
    }

    /// Retain the descriptor allocation so pointer identity is safe across
    /// style clones. Public descriptor replacement still resolves fresh metrics.
    #[inline]
    pub(crate) fn ascii_advance_px(&self, ascii: u8) -> Option<f64> {
        self.ascii_metrics()
            .and_then(|metrics| metrics.advance_px(ascii))
    }

    #[inline]
    pub(crate) fn ascii_pair_kerning_px(&self, left: u8, right: u8) -> Option<f64> {
        self.ascii_metrics()
            .and_then(|metrics| metrics.pair_kerning_px(left, right))
    }

    #[inline]
    fn ascii_metrics(&self) -> Option<crate::line_metrics::WordAsciiMetrics> {
        let metrics = self.fixed_ascii_metrics.get_or_init(|| CachedAsciiMetrics {
            descriptor: Rc::clone(&self.font_descriptor_latin_ascii),
            metrics: crate::line_metrics::word_ascii_metrics(&self.font_descriptor_latin_ascii),
        });
        if Rc::ptr_eq(&metrics.descriptor, &self.font_descriptor_latin_ascii) {
            metrics.metrics
        } else {
            crate::line_metrics::word_ascii_metrics(&self.font_descriptor_latin_ascii)
        }
    }

    /// Attaches ids allocated by the measurement cache shared by the renderer.
    /// The ids are deliberately optional because `jade-docx-layout` is also used by
    /// contexts that do not have an interner.
    pub fn with_font_descriptor_ids(
        mut self,
        latin_ascii: u32,
        latin_hansi: u32,
        east_asia: u32,
        complex: u32,
    ) -> Self {
        self.font_descriptor_latin_ascii_id = Some(latin_ascii);
        self.font_descriptor_latin_hansi_id = Some(latin_hansi);
        self.font_descriptor_east_asia_id = Some(east_asia);
        self.font_descriptor_complex_id = Some(complex);
        self
    }

    pub fn descriptor_for_grapheme(&self, grapheme: &str, script: ScriptCategory) -> &str {
        self.descriptor_and_hash_for_grapheme(grapheme, script).0
    }

    /// Returns a cheap `Rc::clone` of the font descriptor for the given grapheme.
    #[inline]
    pub fn descriptor_for_grapheme_rc(&self, grapheme: &str, script: ScriptCategory) -> Rc<str> {
        Rc::clone(self.descriptor_for_slot(self.font_slot_for_grapheme(grapheme, script)))
    }

    pub fn descriptor_and_hash_for_grapheme(
        &self,
        grapheme: &str,
        script: ScriptCategory,
    ) -> (&str, u64) {
        let (descriptor, hash, _) = self.descriptor_key_for_grapheme(grapheme, script);
        (descriptor, hash)
    }

    /// Resolves the descriptor, its stable hash, and an optional renderer-owned
    /// measurement-cache id in one script-selection pass.
    pub fn descriptor_key_for_grapheme(
        &self,
        grapheme: &str,
        script: ScriptCategory,
    ) -> (&str, u64, Option<u32>) {
        let (_, descriptor, hash, id) = self.font_key_for_grapheme(grapheme, script);
        (descriptor, hash, id)
    }

    /// Resolves the font slot together with its descriptor cache key so callers
    /// can carry the already-resolved slot into later rendering stages.
    #[inline]
    pub fn font_key_for_grapheme(
        &self,
        grapheme: &str,
        script: ScriptCategory,
    ) -> (ResolvedFontSlot, &str, u64, Option<u32>) {
        let slot = self.font_slot_for_grapheme(grapheme, script);
        match slot {
            ResolvedFontSlot::LatinAscii => (
                slot,
                &self.font_descriptor_latin_ascii,
                self.font_descriptor_latin_ascii_hash,
                self.font_descriptor_latin_ascii_id,
            ),
            ResolvedFontSlot::LatinHansi => (
                slot,
                &self.font_descriptor_latin_hansi,
                self.font_descriptor_latin_hansi_hash,
                self.font_descriptor_latin_hansi_id,
            ),
            ResolvedFontSlot::EastAsia => (
                slot,
                &self.font_descriptor_east_asia,
                self.font_descriptor_east_asia_hash,
                self.font_descriptor_east_asia_id,
            ),
            ResolvedFontSlot::Complex => (
                slot,
                &self.font_descriptor_complex,
                self.font_descriptor_complex_hash,
                self.font_descriptor_complex_id,
            ),
        }
    }
}

fn hash_font_descriptor(font_descriptor: &str) -> u64 {
    let mut hasher = FxHasher::default();
    font_descriptor.hash(&mut hasher);
    hasher.finish()
}

/// Applies `w:rFonts/@w:hint` to a grapheme's font slot.
///
/// The hint only settles characters whose slot is shared between scripts.
/// Basic Latin (U+0000–U+007F) always takes the `w:ascii` font (ECMA-376 Part 1
/// §17.3.2.26), so `AAAAAA` in a `w:hint="eastAsia"` run is set in Times New
/// Roman, not in the East Asian face: `system-dev-contract.docx` in Word.
pub fn apply_hint_to_script(
    script: ScriptCategory,
    hint: Option<&FontHint>,
    grapheme: &str,
) -> ScriptCategory {
    if grapheme.is_ascii() {
        return script;
    }
    match (script, hint) {
        (ScriptCategory::Latin, Some(FontHint::EastAsia)) => ScriptCategory::EastAsia,
        (ScriptCategory::Latin, Some(FontHint::Complex)) => ScriptCategory::Complex,
        _ => script,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_docx_reader::document::RunFonts;
    use std::hint::black_box;
    use std::time::Instant;

    #[test]
    fn noto_jp_ascii_metrics_keep_known_pdf_widths_and_unobserved_faces() {
        for family in ["Noto Sans JP", "NotoSansJP"] {
            let face = format!("10.50pt '{family}', sans-serif");
            let style = ResolvedRunStyle::new(
                face.as_str(),
                face.as_str(),
                face.as_str(),
                face.as_str(),
                0.0,
                1.0,
                false,
                None,
            );
            for (character, units) in [(b'2', 555.0), (b'(', 338.0), (b',', 278.0), (b' ', 224.0)] {
                let advance = style
                    .ascii_advance_px(character)
                    .expect("Noto JP requested width");
                assert!((advance - 14.0 * units / 1000.0).abs() < 1e-9);
            }
            assert_eq!(style.ascii_advance_px(b'A'), None);
            for other_face in [
                format!("bold {face}"),
                format!("italic {face}"),
                "10.50pt 'Unknown Sans'".into(),
            ] {
                let mut other = style.clone();
                other.font_descriptor_latin_ascii = Rc::from(other_face);
                assert_eq!(other.ascii_advance_px(b'2'), None);
            }
            let mut latin_noto = style.clone();
            latin_noto.font_descriptor_latin_ascii = Rc::from("10.50pt 'Noto Sans'");
            assert_eq!(
                latin_noto.ascii_advance_px(b'2'),
                Some(14.0 * 1128.0 / 2048.0)
            );
            assert_eq!(latin_noto.ascii_advance_px(b'A'), None);
        }
    }

    #[test]
    fn fixed_ascii_metrics_follow_cloned_styles_and_replaced_descriptors() {
        let face = "10.50pt 'MS Mincho', serif";
        let style = ResolvedRunStyle::new(face, face, face, face, 0.0, 1.0, false, None);
        assert_eq!(style.ascii_advance_px(b'H'), Some(7.0));
        let mut clone = style.clone();
        clone.font_descriptor_latin_ascii = Rc::from("11.00pt 'Times New Roman', serif");
        assert_eq!(clone.ascii_advance_px(b'H'), None);
        clone.font_descriptor_latin_ascii = Rc::from("12.00pt 'MS Mincho', serif");
        assert_eq!(clone.ascii_advance_px(b'H'), Some(8.0));
        clone.font_descriptor_latin_ascii = Rc::from("10.50pt 'Yu Mincho', serif");
        assert_eq!(clone.ascii_advance_px(b'H'), Some(14.0 * 1686.0 / 2048.0));
        assert_eq!(clone.ascii_advance_px(b'q'), None);
        assert_eq!(style.ascii_advance_px(b'H'), Some(7.0));
    }

    #[test]
    fn document_alt_names_extend_the_stack_and_reset_invalidates_caches() {
        let props = RunProperties {
            fonts: Some(RunFonts {
                ascii: Some("HGP創英角ﾎﾟｯﾌﾟ体".into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };

        set_document_font_alt_names([(
            "HGP創英角ﾎﾟｯﾌﾟ体".to_string(),
            vec!["HGPSoeiKakupoptai".to_string()],
        )]);
        let resolved = resolve_font_families(Some(&props));
        assert!(
            resolved.latin_ascii_stack().contains("HGPSoeiKakupoptai"),
            "the fontTable alt name must join the stack: {}",
            resolved.latin_ascii_stack()
        );
        let primary_at = resolved
            .latin_ascii_stack()
            .find("HGP創英角ﾎﾟｯﾌﾟ体")
            .expect("primary");
        let alt_at = resolved
            .latin_ascii_stack()
            .find("HGPSoeiKakupoptai")
            .expect("alt");
        assert!(
            primary_at < alt_at,
            "the primary face stays first; the alt name is its fallback"
        );

        // Loading the next document resets the registry, and the cached stack
        // for the same run properties must not leak the previous document's
        // alt names.
        set_document_font_alt_names(std::iter::empty());
        let resolved = resolve_font_families(Some(&props));
        assert!(
            !resolved.latin_ascii_stack().contains("HGPSoeiKakupoptai"),
            "a cleared registry must invalidate cached stacks: {}",
            resolved.latin_ascii_stack()
        );
    }

    fn ascii_font(name: &str) -> RunProperties {
        RunProperties {
            fonts: Some(RunFonts {
                ascii: Some(name.into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        }
    }

    fn stack_position(stack: &str, name: &str) -> usize {
        stack
            .find(name)
            .unwrap_or_else(|| panic!("{name} missing from {stack}"))
    }

    #[test]
    fn hiragino_japanese_face_names_resolve_to_their_installed_english_alias() {
        for (requested, alias) in [
            ("ヒラギノ角ゴ ProN W3", "Hiragino Kaku Gothic ProN W3"),
            ("ヒラギノ角ゴ ProN W6", "Hiragino Kaku Gothic ProN W6"),
        ] {
            let props = RunProperties {
                font_family: Some(requested.into()),
                ..RunProperties::default()
            };
            let resolved = resolve_font_families(Some(&props));
            let stack = resolved.latin_ascii_stack();
            assert!(stack.contains(alias), "{stack}");
            assert!(stack_position(stack, requested) < stack_position(stack, alias));
        }
    }

    #[test]
    fn host_font_fallback_follows_the_word_font_before_platform_substitutes() {
        set_host_font_fallbacks([HostFontFallback {
            family: "Jade Word Fallback Yu Mincho".into(),
            substitutes: vec!["Yu Mincho".into(), "游明朝".into()],
        }]);
        for requested in ["游明朝", "Yu Mincho"] {
            let resolved = resolve_font_families(Some(&ascii_font(requested)));
            let stack = resolved.latin_ascii_stack();
            let fallback = stack_position(stack, "'Jade Word Fallback Yu Mincho'");
            // An installed Yu Mincho, under either name, keeps priority.
            assert!(stack_position(stack, requested) < fallback, "{stack}");
            assert!(stack_position(stack, "'Yu Mincho'") < fallback, "{stack}");
            // The metric-compatible stand-in beats platform look-alikes,
            // including macOS's own YuMincho (游明朝体), which paginates
            // differently from Word's Yu Mincho.
            assert!(fallback < stack_position(stack, "YuMincho"), "{stack}");
            assert!(
                fallback < stack_position(stack, "'Hiragino Mincho ProN W3'"),
                "{stack}"
            );
            assert_eq!(stack.matches("Jade Word Fallback").count(), 1, "{stack}");
        }

        set_host_font_fallbacks(std::iter::empty());
        let resolved = resolve_font_families(Some(&ascii_font("游明朝")));
        assert!(
            !resolved.latin_ascii_stack().contains("Jade Word Fallback"),
            "clearing the fallbacks must invalidate cached stacks: {}",
            resolved.latin_ascii_stack()
        );
    }

    #[test]
    fn host_font_fallback_stays_out_of_other_word_fonts() {
        set_host_font_fallbacks([HostFontFallback {
            family: "Jade Word Fallback Yu Mincho".into(),
            substitutes: vec!["Yu Mincho".into(), "游明朝".into()],
        }]);
        // MS Mincho's alias stack names Yu Mincho as a late substitute; the
        // Yu Mincho stand-in was not measured for MS Mincho text.
        let resolved = resolve_font_families(Some(&ascii_font("ＭＳ 明朝")));
        assert!(resolved.latin_ascii_stack().contains("'Yu Mincho'"));
        assert!(
            !resolved.latin_ascii_stack().contains("Jade Word Fallback"),
            "{}",
            resolved.latin_ascii_stack()
        );
        set_host_font_fallbacks(std::iter::empty());
    }

    #[test]
    fn host_font_fallback_applies_through_document_alt_names() {
        set_host_font_fallbacks([HostFontFallback {
            family: "Jade Word Fallback Yu Mincho".into(),
            substitutes: vec!["Yu Mincho".into(), "游明朝".into()],
        }]);
        set_document_font_alt_names([("Custom Body".to_string(), vec!["游明朝".to_string()])]);
        let resolved = resolve_font_families(Some(&ascii_font("Custom Body")));
        let stack = resolved.latin_ascii_stack();
        assert!(
            stack_position(stack, "游明朝")
                < stack_position(stack, "'Jade Word Fallback Yu Mincho'"),
            "{stack}"
        );
        set_document_font_alt_names(std::iter::empty());
        set_host_font_fallbacks(std::iter::empty());
    }

    #[test]
    fn resolve_font_families_distinguishes_ascii_and_hansi() {
        let props = RunProperties {
            fonts: Some(RunFonts {
                ascii: Some("AsciiFont".into()),
                h_ansi: Some("AnsiFont".into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };

        let resolved = resolve_font_families(Some(&props));
        assert!(
            resolved.latin_ascii_stack().contains("AsciiFont"),
            "expected ascii font stack to include AsciiFont: {}",
            resolved.latin_ascii_stack()
        );
        assert!(
            resolved.latin_hansi_stack().contains("AnsiFont"),
            "expected hAnsi font stack to include AnsiFont: {}",
            resolved.latin_hansi_stack()
        );
    }

    #[test]
    fn resolve_run_style_carries_font_hint() {
        let props = RunProperties {
            fonts: Some(RunFonts {
                hint: Some(FontHint::EastAsia),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };

        let style = resolve_run_style_for_wrap(Some(&props), None);
        assert_eq!(style.font_hint, Some(FontHint::EastAsia));
    }

    #[test]
    fn unkerned_style_does_not_parse_or_retain_fixed_cjk_metadata() {
        use crate::line_metrics::WORD_FIXED_CJK_DESCRIPTOR_PARSES;
        WORD_FIXED_CJK_DESCRIPTOR_PARSES.with(|count| count.set(0));
        let descriptor: Rc<str> = Rc::from("10.50pt 'MS Mincho', serif");
        let mut style = ResolvedRunStyle::new(
            Rc::clone(&descriptor),
            Rc::clone(&descriptor),
            Rc::clone(&descriptor),
            Rc::clone(&descriptor),
            0.0,
            1.0,
            false,
            None,
        );
        assert_eq!(
            WORD_FIXED_CJK_DESCRIPTOR_PARSES.with(|count| count.get()),
            0
        );
        assert_eq!(Rc::strong_count(&descriptor), 5);
        let clone = style.clone();
        assert_eq!(Rc::strong_count(&descriptor), 9);
        assert_eq!(clone.fixed_east_asia_em_px(), None);
        assert_eq!(
            WORD_FIXED_CJK_DESCRIPTOR_PARSES.with(|count| count.get()),
            0
        );
        drop(clone);
        // Public flag changes must still resolve current metadata safely.
        style.enable_pair_kerning = true;
        assert_eq!(style.fixed_east_asia_em_px(), Some(14.0));
        style.font_descriptor_east_asia = Rc::from("12.00pt 'Yu Mincho', serif");
        assert_eq!(style.fixed_east_asia_em_px(), Some(16.0));
        assert_eq!(
            WORD_FIXED_CJK_DESCRIPTOR_PARSES.with(|count| count.get()),
            2
        );
        style.enable_pair_kerning = false;
        assert_eq!(style.fixed_east_asia_em_px(), None);
        assert_eq!(
            WORD_FIXED_CJK_DESCRIPTOR_PARSES.with(|count| count.get()),
            2
        );
    }

    #[test]
    fn resolve_run_style_applies_text_scale_and_kerning_threshold() {
        let props = RunProperties {
            font_size: Some(12.0),
            text_scale: Some(125.0),
            kerning: Some(11.0),
            ..RunProperties::default()
        };

        let style = resolve_run_style_for_wrap(Some(&props), None);
        assert!((style.text_scale_x - 1.25).abs() < 1e-9);
        assert!(style.enable_pair_kerning);

        let small_props = RunProperties {
            font_size: Some(10.0),
            kerning: Some(11.0),
            ..RunProperties::default()
        };
        let small_style = resolve_run_style_for_wrap(Some(&small_props), None);
        assert!(!small_style.enable_pair_kerning);
    }

    #[test]
    fn resolved_style_routes_measure_cache_ids_with_font_selection() {
        let style = ResolvedRunStyle::new(
            "ascii",
            "hansi",
            "east-asia",
            "complex",
            0.0,
            1.0,
            false,
            None,
        )
        .with_font_descriptor_ids(11, 22, 33, 44);

        assert_eq!(
            style.descriptor_key_for_grapheme("A", ScriptCategory::Latin),
            ("ascii", style.font_descriptor_latin_ascii_hash, Some(11))
        );
        assert_eq!(
            style.descriptor_key_for_grapheme("é", ScriptCategory::Latin),
            ("hansi", style.font_descriptor_latin_hansi_hash, Some(22))
        );
        assert_eq!(
            style.descriptor_key_for_grapheme("漢", ScriptCategory::EastAsia),
            ("east-asia", style.font_descriptor_east_asia_hash, Some(33))
        );
        assert_eq!(
            style.descriptor_key_for_grapheme("ش", ScriptCategory::Complex),
            ("complex", style.font_descriptor_complex_hash, Some(44))
        );
        assert_eq!(
            style.font_slot_for_grapheme("A", ScriptCategory::Latin),
            ResolvedFontSlot::LatinAscii
        );
        assert_eq!(
            style.font_slot_for_grapheme("é", ScriptCategory::Latin),
            ResolvedFontSlot::LatinHansi
        );
        assert_eq!(
            style
                .descriptor_for_slot(ResolvedFontSlot::EastAsia)
                .as_ref(),
            "east-asia"
        );

        let hinted = ResolvedRunStyle {
            font_hint: Some(FontHint::EastAsia),
            ..style
        };
        // The hint settles shared-range Latin only; Basic Latin keeps ascii.
        assert_eq!(
            hinted.descriptor_key_for_grapheme("é", ScriptCategory::Latin),
            ("east-asia", hinted.font_descriptor_east_asia_hash, Some(33))
        );
        assert_eq!(
            hinted.font_slot_for_grapheme("é", ScriptCategory::Latin),
            ResolvedFontSlot::EastAsia
        );
        assert_eq!(
            hinted.font_slot_for_grapheme("A", ScriptCategory::Latin),
            ResolvedFontSlot::LatinAscii
        );
    }

    #[test]
    fn mac_japanese_noto_sans_uses_word_cjk_fallback_without_changing_latin() {
        use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
        let mut props = RunProperties {
            fonts: Some(RunFonts {
                ascii: Some("Noto Sans".into()),
                h_ansi: Some("Noto Sans".into()),
                east_asia: Some("Noto Sans".into()),
                ..RunFonts::default()
            }),
            east_asia_language: Some("ja-JP".into()),
            ..RunProperties::default()
        };
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let windows = resolve_run_style_for_wrap(Some(&props), None);
        assert!(windows.font_descriptor_east_asia.contains("'Noto Sans'"));
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mac = resolve_run_style_for_wrap(Some(&props), None);
        assert!(mac
            .font_descriptor_latin_ascii
            .starts_with("12.00pt 'Noto Sans'"));
        assert!(mac
            .font_descriptor_east_asia
            .starts_with("12.00pt 'MS Mincho'"));
        props.east_asia_language = Some("zh-CN".into());
        let chinese = resolve_run_style_for_wrap(Some(&props), None);
        assert!(chinese
            .font_descriptor_east_asia
            .starts_with("12.00pt 'Noto Sans'"));
        props.fonts.as_mut().unwrap().east_asia = Some("Noto Sans JP".into());
        props.east_asia_language = Some("ja-JP".into());
        let jp = resolve_run_style_for_wrap(Some(&props), None);
        assert!(jp
            .font_descriptor_east_asia
            .starts_with("12.00pt 'Noto Sans JP'"));
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn resolve_font_families_expands_noto_sans_locale_aliases() {
        let props = RunProperties {
            fonts: Some(RunFonts {
                ascii: Some("Noto Sans".into()),
                h_ansi: Some("Noto Sans".into()),
                east_asia: Some("Noto Sans".into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };

        let resolved = resolve_font_families(Some(&props));
        assert!(
            resolved.latin_ascii_stack().contains("'Noto Sans JP'"),
            "expected ascii stack to include Noto Sans JP alias: {}",
            resolved.latin_ascii_stack()
        );
        assert!(
            resolved.east_asia_stack().contains("'Noto Sans KR'"),
            "expected east Asia stack to include Noto Sans KR alias: {}",
            resolved.east_asia_stack()
        );
        assert!(
            resolved.east_asia_stack().contains("'Noto Sans CJK SC'"),
            "expected east Asia stack to include Noto Sans CJK SC alias: {}",
            resolved.east_asia_stack()
        );
    }

    #[test]
    fn resolve_font_families_bridges_noto_cjk_variant_names() {
        let props = RunProperties {
            fonts: Some(RunFonts {
                east_asia: Some("Noto Sans CJK TC".into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };

        let resolved = resolve_font_families(Some(&props));
        assert!(
            resolved.east_asia_stack().contains("'Noto Sans TC'"),
            "expected east Asia stack to include Noto Sans TC alias: {}",
            resolved.east_asia_stack()
        );
    }

    #[test]
    fn resolve_font_families_bridges_arial_to_east_asian_sans_fallbacks() {
        let props = RunProperties {
            fonts: Some(RunFonts {
                east_asia: Some("Arial".into()),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };

        let resolved = resolve_font_families(Some(&props));
        assert!(
            resolved.east_asia_stack().contains("'Arial Unicode MS'"),
            "expected east Asia stack to include Arial Unicode MS: {}",
            resolved.east_asia_stack()
        );
        assert!(
            resolved.east_asia_stack().contains("'Noto Sans JP'"),
            "expected east Asia stack to include Noto Sans JP: {}",
            resolved.east_asia_stack()
        );
    }

    #[test]
    #[ignore]
    fn benchmark_font_resolver_cache_hot_path() {
        const ITERS: usize = 50_000;
        let props = Some(RunProperties {
            bold: Some(true),
            italic: Some(true),
            baseline: Some("superscript".into()),
            char_spacing: Some(0.5),
            kerning: Some(12.0),
            text_scale: Some(95.0),
            font_size: Some(11.0),
            font_family: Some("Calibri".into()),
            fonts: Some(RunFonts {
                ascii: Some("Times New Roman".into()),
                h_ansi: Some("Calibri".into()),
                east_asia: Some("MS Mincho".into()),
                complex: Some("Arial".into()),
                hint: Some(FontHint::EastAsia),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        });

        let uncached_families_start = Instant::now();
        for _ in 0..ITERS {
            black_box(resolve_font_families_uncached(props.as_ref()));
        }
        let uncached_families_elapsed = uncached_families_start.elapsed();

        let cached_families_start = Instant::now();
        for _ in 0..ITERS {
            black_box(resolve_font_families(props.as_ref()));
        }
        let cached_families_elapsed = cached_families_start.elapsed();

        let uncached_style_start = Instant::now();
        for _ in 0..ITERS {
            black_box(resolve_run_style_for_wrap_uncached(
                props.as_ref(),
                Some(0.25),
            ));
        }
        let uncached_style_elapsed = uncached_style_start.elapsed();

        let cached_style_start = Instant::now();
        for _ in 0..ITERS {
            black_box(resolve_run_style_for_wrap(props.as_ref(), Some(0.25)));
        }
        let cached_style_elapsed = cached_style_start.elapsed();

        println!(
            "font resolver benchmark: families_uncached={}us families_cached={}us style_uncached={}us style_cached={}us iterations={}",
            uncached_families_elapsed.as_micros(),
            cached_families_elapsed.as_micros(),
            uncached_style_elapsed.as_micros(),
            cached_style_elapsed.as_micros(),
            ITERS,
        );
    }
}
