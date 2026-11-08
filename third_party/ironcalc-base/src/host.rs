//! What the application around the engine knows and the engine's own model
//! does not, for CELL and INFO during one synchronous calculation: where the
//! workbook is saved, what the window shows, the machine it runs on, and —
//! when the cells' formats live outside the engine — each cell's number
//! format, alignment and protection and each column's width.
use std::cell::RefCell;
use std::sync::OnceLock;

use crate::types::HorizontalAlignment;

/// The workbook as the application has it open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    /// The file's name with its extension (`Budget.xlsx`); `None` before
    /// it is first saved.
    pub file_name: Option<String>,
    /// The folder it is saved in, ending with its separator.
    pub directory: Option<String>,
    /// The one-based row and column of the top-left cell the window shows.
    pub origin: Option<(i32, i32)>,
    /// Whether the workbook recalculates only when asked.
    pub manual: bool,
}

/// How a cell is formatted, as CELL reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct CellLook {
    pub number_format: String,
    pub horizontal: HorizontalAlignment,
    pub locked: bool,
    /// Whether active worksheet protection hides this cell's formula.
    pub formula_hidden: bool,
}

/// A column's width in characters of the default font, and whether it is
/// the sheet's default width.
pub type ColumnWidth = (f64, bool);

/// Cell formats and column widths kept outside the engine's model.
pub struct Looks {
    pub cell: Box<dyn Fn(u32, i32, i32) -> Option<CellLook>>,
    pub column: Box<dyn Fn(u32, i32) -> Option<ColumnWidth>>,
}

/// The width Excel shows for a column whose file width is `width` (the
/// file's unit, padding included): `9.140625` shows as 8.43 characters.
pub fn characters_of_width(width: f64) -> f64 {
    let pixels = ((256.0 * width + (128.0f64 / 7.0).trunc()) / 256.0 * 7.0).trunc();
    (((pixels - 5.0) / 7.0) * 100.0 + 0.5).trunc() / 100.0
}

thread_local! {
    static DOCUMENT: RefCell<Option<Document>> = const { RefCell::new(None) };
    static LOOKS: RefCell<Option<Looks>> = const { RefCell::new(None) };
}

fn scoped<S: 'static, T>(
    key: &'static std::thread::LocalKey<RefCell<Option<S>>>,
    value: S,
    operation: impl FnOnce() -> T,
) -> T {
    struct Restore<S: 'static>(
        &'static std::thread::LocalKey<RefCell<Option<S>>>,
        Option<S>,
    );
    impl<S: 'static> Drop for Restore<S> {
        fn drop(&mut self) {
            let previous = self.1.take();
            self.0.with(|slot| *slot.borrow_mut() = previous);
        }
    }
    let previous = key.with(|slot| slot.borrow_mut().replace(value));
    let _restore = Restore(key, previous);
    operation()
}

/// Runs `operation` with `document` visible to CELL and INFO, restoring
/// what was there before, however `operation` ends.
pub fn with_document<T>(document: Document, operation: impl FnOnce() -> T) -> T {
    scoped(&DOCUMENT, document, operation)
}

/// Runs `operation` with `looks` answering CELL's questions about formats
/// and widths in place of the engine's own styles.
pub fn with_looks<T>(looks: Looks, operation: impl FnOnce() -> T) -> T {
    scoped(&LOOKS, looks, operation)
}

pub(crate) fn document() -> Option<Document> {
    DOCUMENT.with(|slot| slot.borrow().clone())
}

pub(crate) fn cell_look(sheet: u32, row: i32, column: i32) -> Option<Option<CellLook>> {
    LOOKS.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|looks| (looks.cell)(sheet, row, column))
    })
}

pub(crate) fn column_width(sheet: u32, column: i32) -> Option<Option<ColumnWidth>> {
    LOOKS.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|looks| (looks.column)(sheet, column))
    })
}

static DIRECTORY: OnceLock<std::sync::Mutex<Option<String>>> = OnceLock::new();

/// Sets the folder INFO("directory") gives for a workbook not yet saved:
/// the one the application last opened or saved a file in.
pub fn set_current_directory(directory: Option<String>) {
    let slot = DIRECTORY.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut slot) = slot.lock() {
        *slot = directory;
    }
}

pub(crate) fn current_directory() -> Option<String> {
    DIRECTORY
        .get()
        .and_then(|slot| slot.lock().ok().and_then(|slot| slot.clone()))
}

/// The operating system's name and version as INFO("osversion") gives it.
pub(crate) fn os_version() -> String {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(read_os_version).clone()
}

#[cfg(target_os = "macos")]
fn read_os_version() -> String {
    // Excel for Mac: "Macintosh (ARM) Version 15.1 (Build 24B83)".
    let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
        .unwrap_or_default();
    let value = |key: &str| {
        let after = plist.split(&format!("<key>{key}</key>")).nth(1)?;
        let start = after.find("<string>")? + "<string>".len();
        let end = after[start..].find("</string>")? + start;
        Some(after[start..end].trim().to_string())
    };
    let processor = if cfg!(target_arch = "aarch64") {
        "ARM"
    } else {
        "Intel"
    };
    match (value("ProductVersion"), value("ProductBuildVersion")) {
        (Some(version), Some(build)) => {
            format!("Macintosh ({processor}) Version {version} (Build {build})")
        }
        (Some(version), None) => format!("Macintosh ({processor}) Version {version}"),
        _ => format!("Macintosh ({processor})"),
    }
}

#[cfg(target_os = "windows")]
fn read_os_version() -> String {
    if cfg!(target_pointer_width = "64") {
        "Windows (64-bit) NT 10.00".to_string()
    } else {
        "Windows (32-bit) NT 10.00".to_string()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_os_version() -> String {
    if cfg!(target_arch = "wasm32") {
        "Web".to_string()
    } else {
        let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        format!("{} {}", std::env::consts::OS, release.trim())
            .trim()
            .to_string()
    }
}
