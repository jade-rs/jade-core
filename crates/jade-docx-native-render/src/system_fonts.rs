//! The fonts installed on this machine, found by the names documents use.
//!
//! Word, Excel and PowerPoint lay text out in the faces a document names —
//! Calibri, Century, 游明朝, ＭＳ Ｐゴシック — and a line breaks where those
//! faces' advances make it break. A native shell that drew every document in
//! one bundled face could never break lines, or fit cells and text boxes, as
//! Office does. So the shells look the names up here: in the faces Office
//! ships inside its own app bundles first (they are what Office itself draws
//! with, and are not installed system-wide), then the system's fonts.
//!
//! Only an index of names is built up front; a face's file is read when a
//! document asks for its family.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::document::FontFace;

/// The default equation face when a document does not specify `m:mathFont`.
pub const MATH_FAMILY: &str = "Cambria Math";

/// Selects upright faces for the document's math family and the default
/// fallback. Register under the requested names, including their casing, so
/// the renderer can find the same family it read from the document.
pub fn math_faces_for(faces: &[FontFace], configured_family: Option<&str>) -> Vec<FontFace> {
    let configured = configured_family
        .filter(|family| !family.trim().is_empty())
        .unwrap_or(MATH_FAMILY);
    let mut selected = Vec::new();
    for family in [configured, MATH_FAMILY] {
        if selected
            .iter()
            .any(|face: &FontFace| face.family.eq_ignore_ascii_case(family))
        {
            continue;
        }
        if let Some(face) = faces
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(family))
            .min_by_key(|face| (face.italic, face.weight.abs_diff(400)))
        {
            let mut face = face.clone();
            face.family = family.to_string();
            selected.push(face);
        }
    }
    selected
}

/// Where Office keeps the faces it ships, in the order they are searched.
/// `JADE_OFFICE_FONT_DIR` (a platform path list: `;` on Windows, `:` elsewhere) comes first.
pub fn office_font_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(list) = std::env::var_os("JADE_OFFICE_FONT_DIR") {
            directories
                .extend(std::env::split_paths(&list).filter(|path| !path.as_os_str().is_empty()));
        }
        if jade_docx_layout::line_metrics::word_layout_platform()
            == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
        {
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                let cache = PathBuf::from(local).join("Microsoft").join("FontCache");
                directories.extend(windows_cloud_font_directories(&cache));
            }
        }
    }
    directories.extend(
        [
            "/Applications/Microsoft Word.app/Contents/Resources/DFonts",
            "/Applications/Microsoft Excel.app/Contents/Resources/DFonts",
            "/Applications/Microsoft PowerPoint.app/Contents/Resources/DFonts",
            "/Library/Fonts/Microsoft",
        ]
        .map(PathBuf::from),
    );
    directories
}

/// Office's cache generation is independent of its Word product version.
/// Discover existing CloudFonts directories rather than assuming generation 4
/// or recognizing individual font names. Preview fonts are not document fonts.
#[cfg(any(target_os = "windows", test))]
fn windows_cloud_font_directories(cache: &std::path::Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let unversioned = cache.join("CloudFonts");
    if unversioned.is_dir() {
        directories.push(unversioned);
    }
    if let Ok(entries) = std::fs::read_dir(cache) {
        directories.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("CloudFonts"))
                .filter(|path| path.is_dir()),
        );
    }
    directories.sort();
    directories.dedup();
    directories
}

/// The families a browser on a Mac gives CSS's generic names, which jade's
/// fallback stacks end with.
#[cfg(not(target_os = "windows"))]
const GENERIC_FAMILIES: &[(&str, &str)] = &[
    ("serif", "Times"),
    ("sans-serif", "Helvetica"),
    ("monospace", "Courier"),
];

// Windows generic faces must resolve to fonts available to Word on Windows.
#[cfg(target_os = "windows")]
const GENERIC_FAMILIES: &[(&str, &str)] = &[
    ("serif", "Times New Roman"),
    ("sans-serif", "Arial"),
    ("monospace", "Courier New"),
];

/// One installed face.
#[derive(Clone, Debug)]
pub struct InstalledFace {
    pub path: PathBuf,
    pub index: u32,
    pub weight: u16,
    pub italic: bool,
    /// The family it belongs to, lowercased: the typographic family name,
    /// else the family name — how CoreText groups a family's faces.
    family: String,
    /// The legacy family used when retaining implicit Windows reference faces.
    windows_family: String,
}

/// Installed faces by every name they answer to.
#[derive(Default)]
pub struct FontLibrary {
    by_family: HashMap<String, Vec<usize>>,
    windows_by_family: HashMap<String, Vec<usize>>,
    by_face_name: HashMap<String, usize>,
    faces: Vec<InstalledFace>,
}

impl FontLibrary {
    fn system_cache() -> &'static OnceLock<FontLibrary> {
        static LIBRARY: OnceLock<FontLibrary> = OnceLock::new();
        #[cfg(target_os = "windows")]
        {
            static WINDOWS_LIBRARY: OnceLock<FontLibrary> = OnceLock::new();
            if jade_docx_layout::line_metrics::word_layout_platform()
                == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
            {
                return &WINDOWS_LIBRARY;
            }
        }
        &LIBRARY
    }

    /// Office and system faces, indexed once for each layout platform. Mac
    /// replays on Windows retain their original inventory even if a Windows
    /// document initialized Office's cloud-font inventory first.
    pub fn system() -> &'static FontLibrary {
        Self::system_cache().get_or_init(|| {
            let mut database = fontdb::Database::new();
            // Office's first: where a family is in both, its faces win, as
            // they are the ones Office draws with.
            for directory in office_font_directories() {
                if directory.is_dir() {
                    database.load_fonts_dir(&directory);
                }
            }
            database.load_system_fonts();
            Self::index(&database)
        })
    }

    /// The faces in `directories` only.
    pub fn in_directories(directories: &[PathBuf]) -> Self {
        let mut database = fontdb::Database::new();
        for directory in directories {
            database.load_fonts_dir(directory);
        }
        Self::index(&database)
    }

    fn index(database: &fontdb::Database) -> Self {
        let mut library = Self::default();
        for info in database.faces() {
            let fontdb::Source::File(path) = &info.source else {
                continue;
            };
            let names = database
                .with_face_data(info.id, |data, index| {
                    let face = ttf_parser::Face::parse(data, index).ok()?;
                    let named = |id: u16| -> BTreeSet<String> {
                        face.names()
                            .into_iter()
                            .filter(|name| name.name_id == id)
                            .filter_map(|name| name.to_string())
                            .map(|name| name.to_lowercase())
                            .collect()
                    };
                    let mut families: BTreeSet<String> = info
                        .families
                        .iter()
                        .map(|(family, _)| family.to_lowercase())
                        .collect();
                    let typographic = named(ttf_parser::name_id::TYPOGRAPHIC_FAMILY);
                    let windows_families = named(ttf_parser::name_id::FAMILY);
                    let mut face_names = named(ttf_parser::name_id::FULL_NAME);
                    face_names.extend(named(ttf_parser::name_id::POST_SCRIPT_NAME));
                    face_names.insert(info.post_script_name.to_lowercase());
                    if typographic.is_empty() {
                        families.extend(named(ttf_parser::name_id::FAMILY));
                    } else {
                        // `Yu Gothic Medium` is a face of `Yu Gothic` to
                        // CoreText, but documents name it as a family.
                        families.extend(typographic);
                        face_names.extend(named(ttf_parser::name_id::FAMILY));
                    }
                    Some((families, windows_families, face_names))
                })
                .flatten();
            let Some((families, windows_families, face_names)) = names else {
                continue;
            };
            let Some(family) = info.families.first().map(|(name, _)| name.to_lowercase()) else {
                continue;
            };
            let id = library.faces.len();
            let windows_family = windows_families
                .iter()
                .next()
                .cloned()
                .unwrap_or_else(|| family.clone());
            library.faces.push(InstalledFace {
                path: path.clone(),
                index: info.index,
                weight: info.weight.0,
                italic: !matches!(info.style, fontdb::Style::Normal),
                family,
                windows_family,
            });
            for name in families {
                library.by_family.entry(name).or_default().push(id);
            }
            for name in windows_families {
                library.windows_by_family.entry(name).or_default().push(id);
            }
            for name in face_names {
                library.by_face_name.entry(name).or_insert(id);
            }
        }
        library
    }

    fn family_ids(&self, name: &str) -> Option<&[usize]> {
        // Windows Word groups regular/bold/italic by the legacy family.
        // Aptos Display's typographic family is Aptos; treating its legacy
        // name as one full face could register a bold font as regular when
        // that file happened to be indexed first. Preserve CoreText grouping
        // for Mac, including Mac layout replays on a Windows host.
        let ids = if jade_docx_layout::line_metrics::word_layout_platform()
            == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
        {
            self.windows_by_family
                .get(name)
                .or_else(|| self.by_family.get(name))
        } else {
            self.by_family.get(name)
        };
        ids.map(Vec::as_slice)
    }

    /// The faces `name` finds, each with the weight it answers for: every
    /// face of a family of that name (the first found of each weight and
    /// style), else the one face of that full name as the regular, with the
    /// heavier face of its family nearest bold as the bold.
    pub fn find(&self, name: &str) -> Vec<(&InstalledFace, u16)> {
        let key = name.trim().to_lowercase();
        if let Some(ids) = self.family_ids(&key) {
            let mut seen = BTreeSet::new();
            return ids
                .iter()
                .map(|&id| &self.faces[id])
                .filter(|face| seen.insert((face.weight, face.italic)))
                .map(|face| (face, face.weight))
                .collect();
        }
        let Some(&id) = self.by_face_name.get(&key) else {
            return Vec::new();
        };
        let face = &self.faces[id];
        let bold = self
            .faces
            .iter()
            .filter(|other| {
                other.family == face.family
                    && other.italic == face.italic
                    && other.weight > face.weight
            })
            .min_by_key(|other| (i32::from(other.weight) - 700).abs());
        std::iter::once((face, 400))
            .chain(bold.map(|bold| (bold, 700)))
            .collect()
    }

    /// Whether any face answers to `name`.
    pub fn has(&self, name: &str) -> bool {
        let key = name.trim().to_lowercase();
        self.family_ids(&key).is_some() || self.by_face_name.contains_key(&key)
    }

    /// The faces to register for `families`, each under the name it was asked
    /// by, and the names nothing installed answers to. CSS's generic names
    /// are found as a Mac browser finds them.
    pub fn faces_for<'a>(
        &self,
        families: impl IntoIterator<Item = &'a str>,
    ) -> (Vec<FontFace>, Vec<String>) {
        let mut faces = Vec::new();
        let mut missing = Vec::new();
        let mut done = BTreeSet::new();
        for name in families {
            if !done.insert(name.to_lowercase()) {
                continue;
            }
            let lookup = GENERIC_FAMILIES
                .iter()
                .find(|(generic, _)| generic.eq_ignore_ascii_case(name))
                .map_or(name, |(_, family)| *family);
            let found = self.find(lookup);
            if found.is_empty() {
                missing.push(name.to_string());
                continue;
            }
            faces.extend(found.into_iter().map(|(face, weight)| FontFace {
                family: name.to_string(),
                weight,
                italic: face.italic,
                path: face.path.clone(),
                index: face.index,
            }));
        }
        (faces, missing)
    }

    /// Retain every supplied font program for a reference replay, including
    /// implicit fallback faces not named in the DOCX. Named aliases come last
    /// so their established family/style preference remains authoritative.
    /// This does not expand the default system-font selection.
    pub fn reference_faces_for<'a>(
        &self,
        families: impl IntoIterator<Item = &'a str>,
    ) -> (Vec<FontFace>, Vec<String>) {
        let (named, missing) = self.faces_for(families);
        let mut all: Vec<_> = {
            let selected: BTreeSet<_> = named
                .iter()
                .map(|face| (face.path.as_path(), face.index))
                .collect();
            self.faces
                .iter()
                .filter(|face| !selected.contains(&(face.path.as_path(), face.index)))
                .map(|face| FontFace {
                    // Extra subsets must retain their own Windows family too:
                    // a Display subset registered under typographic `Aptos`
                    // can otherwise shadow a correctly selected Aptos alias.
                    family: if jade_docx_layout::line_metrics::word_layout_platform()
                        == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
                    {
                        face.windows_family.clone()
                    } else {
                        face.family.clone()
                    },
                    weight: face.weight,
                    italic: face.italic,
                    path: face.path.clone(),
                    index: face.index,
                })
                .collect()
        };
        all.extend(named);
        (all, missing)
    }
}

/// The faces to open a Word document with: `fallback` first — the bundled
/// faces text falls back to when no installed face covers it — then the
/// installed faces of every family the document may be drawn in.
pub fn word_document_faces(package: &[u8], fallback: &[FontFace]) -> Vec<FontFace> {
    let families = word_families(package);
    let (installed, _missing) =
        FontLibrary::system().faces_for(families.iter().map(String::as_str));
    fallback.iter().cloned().chain(installed).collect()
}

/// Every font family an Office package's parts name: Word's `w:rFonts`
/// slots, math settings and font table, DrawingML's typefaces (theme fonts among them, a
/// theme's per-script list only for Japanese), and SpreadsheetML's font and
/// rich-run names.
pub fn families_named_in(package: &[u8]) -> BTreeSet<String> {
    let mut named = BTreeSet::new();
    let Ok(package) = jade_opc::OpcPackage::open(package) else {
        return named;
    };
    for part in package.part_names() {
        if !part.ends_with(".xml") {
            continue;
        }
        if let Ok(Some(xml)) = package.part_str(part) {
            font_names_in_xml(&xml, part.ends_with("fontTable.xml"), &mut named);
            if part.ends_with("fontTable.xml")
                && jade_docx_layout::line_metrics::word_layout_platform()
                    == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
            {
                if let Ok(table) = jade_docx_reader::FontTablePart::from_xml(&xml) {
                    named.extend(
                        table
                            .windows_fallback_names()
                            .map(|(_, fallback)| fallback.to_string()),
                    );
                }
            }
        }
    }
    named
}

/// The families a Word document's text may be drawn in: those it names, and
/// those jade's font resolution falls back to for each of them.
pub fn word_families(package: &[u8]) -> BTreeSet<String> {
    use jade_docx_layout::font_resolver::{
        resolve_font_families, DEFAULT_COMPLEX_FONT_STACK, DEFAULT_EAST_ASIA_FONT_STACK,
        DEFAULT_LATIN_FONT_STACK,
    };
    use jade_docx_reader::document::{RunFonts, RunProperties};
    use std::rc::Rc;

    let mut named = families_named_in(package);
    // An altChunk's HTML is set in Japanese Word's web font, which no part
    // of the package names.
    named.insert(jade_docx_editor::word_document::HTML_ALT_CHUNK_FONT.to_string());
    let mut families = BTreeSet::new();
    for stack in [
        DEFAULT_LATIN_FONT_STACK,
        DEFAULT_EAST_ASIA_FONT_STACK,
        DEFAULT_COMPLEX_FONT_STACK,
    ] {
        stack_families(stack, &mut families);
    }
    // Retain the default math face as well as a document's named math font.
    families.insert(MATH_FAMILY.to_string());
    // Word for Mac measures a numbering label's suffix space in Arial, named
    // or not; without it the space takes the fallback face's width.
    stack_families(
        jade_docx_layout::line_metrics::WORD_MAC_NUMBERING_SUFFIX_SPACE_STACK,
        &mut families,
    );
    for name in &named {
        let name: Rc<str> = Rc::from(name.as_str());
        for language in [None, Some("ja-JP")] {
            let properties = RunProperties {
                fonts: Some(RunFonts {
                    ascii: Some(Rc::clone(&name)),
                    h_ansi: Some(Rc::clone(&name)),
                    east_asia: Some(Rc::clone(&name)),
                    complex: Some(Rc::clone(&name)),
                    ..RunFonts::default()
                }),
                language: language.map(Rc::from),
                east_asia_language: language.map(Rc::from),
                ..RunProperties::default()
            };
            let resolved = resolve_font_families(Some(&properties));
            for stack in [
                resolved.latin_stack(),
                resolved.east_asia_stack(),
                resolved.complex_stack(),
            ] {
                stack_families(stack, &mut families);
            }
        }
        families.insert(name.to_string());
    }
    families
}

/// The family names of a CSS font stack.
fn stack_families(stack: &str, out: &mut BTreeSet<String>) {
    for family in stack.split(',') {
        let family = family.trim().trim_matches(|c| c == '\'' || c == '"').trim();
        if !family.is_empty() {
            out.insert(family.to_string());
        }
    }
}

/// Collects the font names in one part's XML.
fn font_names_in_xml(xml: &str, font_table: bool, out: &mut BTreeSet<String>) {
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('>') else {
            break;
        };
        let tag = &rest[..close];
        rest = &rest[close..];
        if tag.starts_with('/') || tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name);
        let attributes: &[&str] = match local {
            "rFonts" => &["ascii", "hAnsi", "eastAsia", "cs"],
            "mathFont" => &["val"],
            "font" if name.starts_with("w:") && font_table => &["name"],
            // A theme lists a face for each of some thirty scripts; only the
            // Japanese one is drawn here.
            "font" if name.starts_with("a:") => {
                if attribute(tag, "script").as_deref() == Some("Jpan") {
                    &["typeface"]
                } else {
                    &[]
                }
            }
            "latin" | "ea" | "cs" | "sym" | "buFont" => &["typeface"],
            // SpreadsheetML: `<font><name val=…/>` and rich runs' `<rFont>`.
            "name" | "rFont" if !name.contains(':') || name.starts_with("x:") => &["val"],
            _ => &[],
        };
        for key in attributes {
            if let Some(value) = attribute(tag, key) {
                let value = value.trim();
                if !value.is_empty() && !value.starts_with('+') {
                    out.insert(value.to_string());
                }
            }
        }
    }
}

/// The unescaped value of `key` (unprefixed or with any prefix) in a tag.
fn attribute(tag: &str, key: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(at) = rest.find(key) {
        let before = rest[..at].chars().last();
        let after = &rest[at + key.len()..];
        rest = after;
        if !matches!(before, Some(c) if c.is_whitespace() || c == ':') {
            continue;
        }
        let Some(after) = after.trim_start().strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let value = &after[1..];
        let end = value.find(quote)?;
        return Some(unescape_xml(&value[..end]));
    }
    None
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    #[test]
    fn cloud_font_discovery_handles_cache_generations_without_preview_faces() {
        let root = std::env::temp_dir().join(format!(
            "jade-cloud-font-discovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(!root.exists());
        assert!(windows_cloud_font_directories(&root).is_empty());
        let paths = [
            "3/CloudFonts",
            "4/CloudFonts",
            "4/PreviewFont",
            "next/CloudFonts",
        ];
        for name in paths {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        assert_eq!(
            windows_cloud_font_directories(&root),
            ["3/CloudFonts", "4/CloudFonts", "next/CloudFonts"].map(|name| root.join(name))
        );
        for name in paths {
            std::fs::remove_dir(root.join(name)).unwrap();
        }
        for name in ["3", "4", "next"] {
            std::fs::remove_dir(root.join(name)).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_cloud_inventory_cache_cannot_replace_the_mac_cache() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mac = FontLibrary::system_cache();
        let before = office_font_directories();
        set_word_layout_platform(WordLayoutPlatform::Windows);
        assert!(!std::ptr::eq(mac, FontLibrary::system_cache()));
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert!(std::ptr::eq(mac, FontLibrary::system_cache()));
        assert_eq!(office_font_directories(), before);
        set_word_layout_platform(previous);
    }

    #[test]
    fn windows_legacy_family_keeps_bold_and_regular_faces_without_changing_mac() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        let mut library = FontLibrary::default();
        for (path, weight) in [("display-bold.ttf", 700), ("display-regular.ttf", 400)] {
            library.faces.push(InstalledFace {
                path: PathBuf::from(path),
                index: 0,
                weight,
                italic: false,
                family: "aptos".into(),
                windows_family: "aptos display".into(),
            });
        }
        library.by_family.insert("aptos".into(), vec![0, 1]);
        library.by_face_name.insert("aptos display".into(), 0);
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mac_before = library.faces_for(["Aptos Display"]).0;
        library
            .windows_by_family
            .insert("aptos display".into(), vec![0, 1]);
        assert_eq!(library.faces_for(["Aptos Display"]).0, mac_before);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let windows = library.faces_for(["Aptos Display"]).0;
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].weight, 700);
        assert_eq!(windows[0].path, PathBuf::from("display-bold.ttf"));
        assert_eq!(windows[1].weight, 400);
        assert_eq!(windows[1].path, PathBuf::from("display-regular.ttf"));
        assert!(library.has("Aptos Display"));
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert_eq!(library.faces_for(["Aptos Display"]).0, mac_before);
        set_word_layout_platform(previous);
    }

    #[test]
    fn windows_reference_subsets_keep_legacy_families_and_mac_grouping() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        let previous = word_layout_platform();
        let mut library = FontLibrary::default();
        for (path, family) in [
            ("aptos.ttf", "aptos"),
            ("display.ttf", "aptos display"),
            ("aptos-subset.ttf", "aptos"),
            ("display-subset.ttf", "aptos display"),
        ] {
            library.faces.push(InstalledFace {
                path: PathBuf::from(path),
                index: 0,
                weight: 400,
                italic: false,
                family: "aptos".into(),
                windows_family: family.into(),
            });
        }
        library.by_family.insert("aptos".into(), vec![0, 1, 2, 3]);
        library.by_face_name.insert("aptos display".into(), 1);
        library.windows_by_family.insert("aptos".into(), vec![0, 2]);
        library
            .windows_by_family
            .insert("aptos display".into(), vec![1, 3]);
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mac_before = library.reference_faces_for(["Aptos", "Aptos Display"]);
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let (windows, missing) = library.reference_faces_for(["Aptos", "Aptos Display"]);
        assert!(missing.is_empty());
        assert_eq!(windows.len(), 4);
        assert_eq!(windows[0].family, "aptos");
        assert_eq!(windows[0].path, PathBuf::from("aptos-subset.ttf"));
        assert_eq!(windows[1].family, "aptos display");
        assert_eq!(windows[1].path, PathBuf::from("display-subset.ttf"));
        assert_eq!(windows[2].family, "Aptos");
        assert_eq!(windows[3].family, "Aptos Display");
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert_eq!(
            library.reference_faces_for(["Aptos", "Aptos Display"]),
            mac_before
        );
        set_word_layout_platform(previous);
    }

    #[test]
    fn reference_faces_retain_implicit_and_same_style_subset_programs() {
        use super::*;
        let mut library = FontLibrary::default();
        for (path, family) in [
            ("named.ttf", "requested"),
            ("second-subset.ttf", "requested"),
            ("implicit-emoji.ttf", "implicit emoji"),
        ] {
            library.faces.push(InstalledFace {
                path: PathBuf::from(path),
                index: 0,
                weight: 400,
                italic: false,
                family: family.into(),
                windows_family: family.into(),
            });
        }
        library.by_family.insert("requested".into(), vec![0, 1]);
        library.by_family.insert("implicit emoji".into(), vec![2]);
        let (ordinary, _) = library.faces_for(["Requested"]);
        assert_eq!(ordinary.len(), 1);
        let (reference, missing) = library.reference_faces_for(["Requested", "Missing"]);
        assert_eq!(missing, ["Missing"]);
        assert_eq!(reference.len(), 3);
        assert_eq!(reference[0].path, PathBuf::from("second-subset.ttf"));
        assert_eq!(reference[1].path, PathBuf::from("implicit-emoji.ttf"));
        assert_eq!(reference[2], ordinary[0]);
        assert_eq!(library.faces_for(["Requested"]).0, ordinary);
    }

    use super::*;

    #[test]
    fn math_faces_keep_the_requested_name_and_prefer_regular_upright() {
        let face = |family: &str, weight, italic| FontFace {
            family: family.to_string(),
            weight,
            italic,
            path: PathBuf::from("unused.ttf"),
            index: 0,
        };
        let faces = [
            face("host math", 400, true),
            face("host math", 700, false),
            face("host math", 400, false),
            face(MATH_FAMILY, 400, false),
            face("Arial", 400, false),
        ];
        let selected = math_faces_for(&faces, Some("Host Math"));
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].family, "Host Math");
        assert_eq!(selected[0].weight, 400);
        assert!(!selected[0].italic);
        assert_eq!(selected[1].family, MATH_FAMILY);
        for family in [
            None,
            Some(""),
            Some("  "),
            Some(MATH_FAMILY),
            Some("Missing"),
        ] {
            assert_eq!(math_faces_for(&faces, family), [faces[3].clone()]);
        }
    }

    #[test]
    fn names_in_word_drawingml_and_spreadsheetml_parts() {
        let mut out = BTreeSet::new();
        font_names_in_xml(
            r#"<w:rPr><w:rFonts w:ascii="Century" w:eastAsia="ＭＳ 明朝" w:hAnsi="+mj-lt"/></w:rPr>
               <a:latin typeface="Calibri"/><a:ea typeface=""/>
               <a:font script="Jpan" typeface="游ゴシック"/><a:font script="Arab" typeface="Times New Roman"/>
               <font><name val="Yu Gothic"/></font><r><rPr><rFont val="Meiryo &amp; Co"/></rPr></r>"#,
            false,
            &mut out,
        );
        let expected: BTreeSet<String> = [
            "Century",
            "ＭＳ 明朝",
            "Calibri",
            "游ゴシック",
            "Yu Gothic",
            "Meiryo & Co",
        ]
        .map(String::from)
        .into();
        assert_eq!(out, expected);
    }

    #[test]
    fn the_font_table_names_count_only_there() {
        let mut out = BTreeSet::new();
        font_names_in_xml(r#"<w:font w:name="Cambria"/>"#, false, &mut out);
        assert!(out.is_empty());
        font_names_in_xml(r#"<w:font w:name="Cambria"/>"#, true, &mut out);
        assert!(out.contains("Cambria"));
    }

    #[test]
    fn a_settings_only_math_font_is_discovered() {
        use std::io::Write;
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            zip.start_file(
                "word/settings.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(br#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:mathPr><m:mathFont m:val="Host Math"/></m:mathPr></w:settings>"#)
                .unwrap();
            zip.finish().unwrap();
        }
        let bytes = bytes.into_inner();
        assert!(families_named_in(&bytes).contains("Host Math"));
        let families = word_families(&bytes);
        assert!(families.contains("Host Math"));
        assert!(families.contains(MATH_FAMILY));
    }

    /// `alt-chunk-html` names Times New Roman and Arial Unicode MS only, and
    /// Word for Mac sets its imported HTML in MS-PGothic.
    #[test]
    fn a_document_is_opened_with_the_alt_chunk_web_font() {
        let package = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/alt-chunk-html.docx"),
        )
        .unwrap();
        let font = jade_docx_editor::word_document::HTML_ALT_CHUNK_FONT;
        assert!(!families_named_in(&package).contains(font));
        assert!(word_families(&package).contains(font));
    }

    /// `word-mac-probe-indent` names Century and ＭＳ 明朝 only, and Word for
    /// Mac still sets its `①` labels' suffix spaces at Arial's 0.278 em.
    #[test]
    fn a_document_is_opened_with_the_numbering_suffix_space_face() {
        let package = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/word-mac-probe-indent.docx"),
        )
        .unwrap();
        assert!(!families_named_in(&package).contains("Arial"));
        assert!(word_families(&package).contains("Arial"));
    }

    #[test]
    fn a_bundled_face_is_found_by_its_family_and_its_full_name() {
        let fonts =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fonts/Noto_Sans_JP/static");
        let library = FontLibrary::in_directories(&[fonts]);
        let family = library.find("noto sans jp");
        assert!(family
            .iter()
            .any(|(face, weight)| *weight == 400 && !face.italic));
        assert!(family.iter().any(|(_, weight)| *weight == 700));
        let (faces, missing) = library.faces_for(["Noto Sans JP", "No Such Face"]);
        assert!(faces.iter().all(|face| face.family == "Noto Sans JP"));
        assert_eq!(missing, ["No Such Face"]);
        // A face's full name finds it, with its family's bold beside it.
        let by_name = library.find("Noto Sans JP Regular");
        assert_eq!(by_name.first().map(|(_, weight)| *weight), Some(400));
    }
}
