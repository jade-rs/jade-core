//! Which Word a shell lays documents out like.
//!
//! Word for Mac and Word for Windows break and place the same document
//! differently in a handful of places (auto-spacing gaps, line-start
//! squeezing, justification, float wrap); `jade-docx-layout` reproduces
//! either. The choice is a thread-local the layout reads, set once by the
//! host before its first document is laid out: cached layouts are not
//! redone when it changes.
//!
//! The native app follows the machine it runs on. The browser builds follow
//! a setting, which `"auto"` (or anything unrecognised) turns into the
//! browser's own platform, as jlsi/jade's `EditorOptions.wordPlatform` does.

pub use jade_docx_layout::line_metrics::WordLayoutPlatform;

/// Word for Mac on macOS, Word for Windows anywhere else.
pub fn native_word_layout_platform() -> WordLayoutPlatform {
    if cfg!(target_os = "macos") {
        WordLayoutPlatform::Mac
    } else {
        WordLayoutPlatform::Windows
    }
}

/// `setting` (`"mac"`, `"windows"` or `"auto"`) as a platform. `"auto"`, and
/// anything else, follows `browser_platform` — `navigator.userAgentData
/// .platform`, `navigator.platform` or the user agent, whichever the browser
/// has: a Mac's is `"macOS"`, `"MacIntel"` or contains `Macintosh`.
pub fn word_layout_platform_for(setting: &str, browser_platform: &str) -> WordLayoutPlatform {
    let setting = setting.trim();
    if setting.eq_ignore_ascii_case("mac") {
        WordLayoutPlatform::Mac
    } else if setting.eq_ignore_ascii_case("windows") {
        WordLayoutPlatform::Windows
    } else if browser_platform.to_ascii_lowercase().contains("mac") {
        WordLayoutPlatform::Mac
    } else {
        WordLayoutPlatform::Windows
    }
}

/// Lays this thread's documents out like `platform`'s Word from here on.
/// Call it before the first document is opened.
pub fn set_word_layout_platform(platform: WordLayoutPlatform) {
    jade_docx_layout::line_metrics::set_word_layout_platform(platform);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_names_the_platform_or_follows_the_browser() {
        assert_eq!(
            word_layout_platform_for("mac", "Win32"),
            WordLayoutPlatform::Mac
        );
        assert_eq!(
            word_layout_platform_for("Windows", "MacIntel"),
            WordLayoutPlatform::Windows
        );
        for mac in [
            "macOS",
            "MacIntel",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        ] {
            assert_eq!(
                word_layout_platform_for("auto", mac),
                WordLayoutPlatform::Mac
            );
        }
        for other in ["Windows", "Win32", "Linux x86_64", ""] {
            assert_eq!(
                word_layout_platform_for("", other),
                WordLayoutPlatform::Windows
            );
        }
    }
}
