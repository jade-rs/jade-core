/// Default tab width (points) applied when paragraph tabs are absent.
pub(crate) const DEFAULT_TAB_WIDTH_PT: f32 = 36.0;

/// Heuristic expansion for SBCS glyphs when `balanceSingleByteDoubleByteWidth` is enabled.
///
/// 0.23 keeps wrap positions closer to Word for fallback fonts (Yu/Noto families)
/// where SBCS glyphs are narrower than MS Mincho.
pub(crate) const BALANCED_SBCS_EXTRA_RATIO: f64 = 0.23;

/// Additional visual boost for EastAsian <-> Latin/Number auto spacing.
///
/// This value intentionally over-indexes readability on legal-text lines where
/// Word's implicit boundary spacing appears larger than browser defaults.
pub(crate) const AUTO_SPACE_SIDE_BOOST: f64 = 1.5;

/// Word 365 treats missing autoSpaceDE/autoSpaceDN as enabled and uses the
/// same visual profile as explicit ON for EastAsian<->Latin/Number boundaries.
pub(crate) const IMPLICIT_NUMERIC_GAP_SIDE_RATIO_DEFAULT_ENABLED: f64 =
    (0.80 / 3.0) * 1.10 * AUTO_SPACE_SIDE_BOOST;

/// Explicit autoSpaceDN ON profile.
pub(crate) const IMPLICIT_NUMERIC_GAP_SIDE_RATIO_EXPLICIT_AUTO_DN: f64 =
    (0.80 / 3.0) * 1.10 * AUTO_SPACE_SIDE_BOOST;

/// Minimum visual gap per boundary in CSS px.
///
/// MS Mincho's half-width digits measure 7 px at the fixture's 14 px CJK em.
/// Word leaves a quarter-em (3.5 px) at each number/East-Asian boundary. The
/// floor matters for narrow fixed-pitch digits; wider proportional digits keep
/// using the ratio above.
pub(crate) const IMPLICIT_NUMERIC_GAP_SIDE_MIN_PX_DEFAULT: f64 = 3.5;

/// Minimum gap for explicit autoSpaceDN ON.
pub(crate) const IMPLICIT_NUMERIC_GAP_SIDE_MIN_PX_EXPLICIT: f64 = 3.5;

/// Share of the East Asian-to-Latin auto-spacing gap that Word leaves on the
/// other side, from a Latin letter or digit back to East Asian text.
///
/// Word for Mac prints `system-dev-contract.docx` (10.5pt Yu Mincho/Times New
/// Roman) with 2.625pt (1/4 em) before `A`, `B` and every number and 2.1pt
/// (1/5 em) after them. Windows Word COM traces show the same asymmetry on its
/// 0.75pt grid: 3.0pt before a number, 2.25pt after it (`yokohama_page1`).
pub const AUTO_SPACE_LATIN_TO_EAST_ASIAN_SHARE: f64 = 0.8;

/// Ratio for implicit Latin <-> EastAsia gap (autoSpaceDE).
///
/// Word's `autoSpaceDE` adds roughly 1/4 em at an East Asian/Latin boundary.
/// The ratio remains useful for wide proportional glyphs; the minimum below
/// supplies the quarter-em floor for the 10.5 pt legal-text fixtures.
pub const IMPLICIT_LATIN_GAP_SIDE_RATIO: f64 = (0.80 / 3.0) * 1.10;

/// Minimum gap per boundary in CSS px for Latin <-> EastAsia spacing.
///
/// The `contract.docx` Word trace records about 3.0-3.8 px of additional
/// advance on each side of `てasdasdasdい`. Using the former 2.2 px floor
/// let the following `違` fit even though Word wraps it. A 10.5 pt CJK em is
/// 14 CSS px, so 3.5 px is the corresponding quarter-em value and matches the
/// same floor used for `autoSpaceDN`.
pub const IMPLICIT_LATIN_GAP_SIDE_MIN_PX: f64 = 3.5;

/// Shared East Asian advance scale for wrap/draw width calculations.
///
/// Keep this close to 1.0 so line breaks are primarily determined by measured
/// glyph widths and content width.
pub const EAST_ASIA_ADVANCE_SCALE: f64 = 1.0;
pub const EAST_ASIA_FE_ADVANCE_SCALE: f64 = 1.01;

/// Safety margin for wrap decisions.
///
/// Keep zero so measured glyph widths and content width determine boundaries.
pub(crate) const WRAP_SAFETY_PX: f64 = 0.0;
