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

/// Where Office keeps the faces it ships, in the order they are searched.
/// `JADE_OFFICE_FONT_DIR` (a list separated by `:`) comes first.
pub fn office_font_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Ok(list) = std::env::var("JADE_OFFICE_FONT_DIR") {
        directories.extend(
            list.split(':')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(PathBuf::from),
        );
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

/// The families a browser on a Mac gives CSS's generic names, which jade's
/// fallback stacks end with.
const GENERIC_FAMILIES: &[(&str, &str)] = &[
    ("serif", "Times"),
    ("sans-serif", "Helvetica"),
    ("monospace", "Courier"),
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
}

/// Installed faces by every name they answer to.
#[derive(Default)]
pub struct FontLibrary {
    by_family: HashMap<String, Vec<usize>>,
    by_face_name: HashMap<String, usize>,
    faces: Vec<InstalledFace>,
}

impl FontLibrary {
    /// The faces in Office's directories and the system's, indexed once per
    /// process.
    pub fn system() -> &'static FontLibrary {
        static LIBRARY: OnceLock<FontLibrary> = OnceLock::new();
        LIBRARY.get_or_init(|| {
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
                    Some((families, face_names))
                })
                .flatten();
            let Some((families, face_names)) = names else {
                continue;
            };
            let Some(family) = info.families.first().map(|(name, _)| name.to_lowercase()) else {
                continue;
            };
            let id = library.faces.len();
            library.faces.push(InstalledFace {
                path: path.clone(),
                index: info.index,
                weight: info.weight.0,
                italic: !matches!(info.style, fontdb::Style::Normal),
                family,
            });
            for name in families {
                library.by_family.entry(name).or_default().push(id);
            }
            for name in face_names {
                library.by_face_name.entry(name).or_insert(id);
            }
        }
        library
    }

    /// The faces `name` finds, each with the weight it answers for: every
    /// face of a family of that name (the first found of each weight and
    /// style), else the one face of that full name as the regular, with the
    /// heavier face of its family nearest bold as the bold.
    pub fn find(&self, name: &str) -> Vec<(&InstalledFace, u16)> {
        let key = name.trim().to_lowercase();
        if let Some(ids) = self.by_family.get(&key) {
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
        self.by_family.contains_key(&key) || self.by_face_name.contains_key(&key)
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
/// slots and font table, DrawingML's typefaces (theme fonts among them, a
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

    let named = families_named_in(package);
    let mut families = BTreeSet::new();
    for stack in [
        DEFAULT_LATIN_FONT_STACK,
        DEFAULT_EAST_ASIA_FONT_STACK,
        DEFAULT_COMPLEX_FONT_STACK,
    ] {
        stack_families(stack, &mut families);
    }
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
    use super::*;

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
