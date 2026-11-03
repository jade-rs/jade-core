//! Word compatibility target shared by loading and layout.
//! Hosts select it before opening a document; the model remains layout-free.

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
