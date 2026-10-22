//! Open Packaging Conventions: the zip container and relationship parts every
//! OOXML format shares.
//!
//! `.xlsx`, `.pptx` and `.docx` all wrap their parts the same way — a zip whose
//! entries reference each other through `.rels` parts — so the container is
//! worth owning once. `jade-docx-reader` still has its own copy with a
//! DOCX-shaped relationship-type enum; folding it into this crate is a separate
//! change from introducing this one.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::{HashMap, HashSet};
use thiserror::Error;
pub mod compound_file;
pub mod encryption;
pub mod package_limits;
pub mod signature;
mod snapshot;
pub use package_limits::{PackageLimitError, PackageLoadLimits};
pub use snapshot::{ArchiveSnapshot, SnapshotStore};

/// Everything that can go wrong opening a package or reading a relationship
/// part.
#[derive(Debug, Error)]
pub enum OpcError {
    #[error("archive: {0}")]
    Archive(#[from] zip::result::ZipError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("xml: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("attribute: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("{part} is not valid UTF-8")]
    Encoding { part: String },
    #[error("required part `{0}` is missing from the archive")]
    MissingPart(String),
    #[error(transparent)]
    Limit(#[from] PackageLimitError),
}

pub type Result<T> = std::result::Result<T, OpcError>;

// zip 8.6 returns a Result here; existing SDK consumers can still lock 8.2.
trait ArchiveCommentResult {
    fn into_save_result(self) -> Result<()>;
}
impl ArchiveCommentResult for () {
    fn into_save_result(self) -> Result<()> {
        Ok(())
    }
}
impl ArchiveCommentResult for zip::result::ZipResult<()> {
    fn into_save_result(self) -> Result<()> {
        self.map_err(Into::into)
    }
}

const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// The content type a package gives every part with `extension`, for the
/// extensions whose parts are typed by extension alone.
fn default_content_type(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "rels" => "application/vnd.openxmlformats-package.relationships+xml",
        "png" => "image/png",
        "jpeg" | "jpg" | "jpe" | "jfif" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "wdp" => "image/vnd.ms-photo",
        "vml" => "application/vnd.openxmlformats-officedocument.vmlDrawing",
        "bin" => "application/vnd.openxmlformats-officedocument.oleObject",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "wmv" => "video/x-ms-wmv",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "fntdata" => "application/x-fontdata",
        "odttf" => "application/vnd.openxmlformats-officedocument.obfuscatedFont",
        _ => return None,
    })
}

/// Every part of the archive, keyed by its full name (`xl/workbook.xml`).
///
/// The whole archive is read up front: the parts are small next to the document
/// body, and holding them lets the parsers stay borrow-free.
pub struct OpcPackage {
    parts: HashMap<String, Vec<u8>>,
    /// Part names in the order the archive held them.
    ///
    /// Writing the package back in a different order would produce a file that
    /// is equivalent but not comparable: a diff of two saves would be noise.
    /// Excel also expects `[Content_Types].xml` first, which the original
    /// order gives for free.
    order: Vec<String>,
    modified: HashSet<String>,
    origin: std::sync::Arc<()>,
}

/// An editing package borrowing its original ZIP for unchanged compressed data.
pub struct PreservingOpcPackage<'a> {
    package: OpcPackage,
    original: &'a [u8],
    origin: std::sync::Arc<()>,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PackageWriteStats {
    pub copied_parts: usize,
    pub copied_compressed_bytes: u64,
    pub recompressed_parts: usize,
    pub recompressed_source_bytes: usize,
}
impl std::ops::Deref for PreservingOpcPackage<'_> {
    type Target = OpcPackage;
    fn deref(&self) -> &Self::Target {
        &self.package
    }
}
impl std::ops::DerefMut for PreservingOpcPackage<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.package
    }
}
impl PreservingOpcPackage<'_> {
    pub fn write(&self) -> Result<Vec<u8>> {
        self.write_with_stats().map(|(bytes, _)| bytes)
    }
    pub fn write_with_stats(&self) -> Result<(Vec<u8>, PackageWriteStats)> {
        if !std::sync::Arc::ptr_eq(&self.origin, &self.package.origin) {
            // A caller replaced the entire dereferenced package; its old ZIP
            // cannot supply the new package's unchanged entries.
            let stats = PackageWriteStats {
                recompressed_parts: self.package.parts.len(),
                recompressed_source_bytes: self.package.parts.values().map(Vec::len).sum(),
                ..Default::default()
            };
            return self.package.write().map(|bytes| (bytes, stats));
        }
        if self.package.modified.is_empty() {
            return Ok((self.original.to_vec(), PackageWriteStats::default()));
        }
        let mut stats = PackageWriteStats::default();
        let mut original = zip::ZipArchive::new(std::io::Cursor::new(self.original))?;
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            writer
                .set_raw_comment(original.comment().into())
                .into_save_result()?;
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let content_types = self.package.content_types_covering_new_parts();
            for name in &self.package.order {
                let Some(contents) = self.package.parts.get(name) else {
                    continue;
                };
                if name == CONTENT_TYPES_PART {
                    if let Some(patched) = &content_types {
                        stats.recompressed_parts += 1;
                        stats.recompressed_source_bytes += patched.len();
                        writer.start_file(name, options)?;
                        std::io::Write::write_all(&mut writer, patched)?;
                        continue;
                    }
                }
                if !self.package.modified.contains(name) {
                    let entry = original.by_name(name)?;
                    stats.copied_parts += 1;
                    stats.copied_compressed_bytes += entry.compressed_size();
                    writer.raw_copy_file(entry)?;
                } else {
                    stats.recompressed_parts += 1;
                    stats.recompressed_source_bytes += contents.len();
                    writer.start_file(name, options)?;
                    std::io::Write::write_all(&mut writer, contents)?;
                }
            }
            writer.finish()?;
        }
        Ok((buffer, stats))
    }
}

impl OpcPackage {
    /// Open a package within the default [`PackageLoadLimits`].
    pub fn open(bytes: &[u8]) -> Result<Self> {
        Self::open_with_limits(bytes, PackageLoadLimits::default())
    }

    /// Open a package within explicit resource budgets.
    ///
    /// Every entry is inflated up front, so without a bound a small hostile
    /// archive (a ZIP bomb) exhausts memory before any parser sees it. The
    /// declared sizes are checked first and the decompressor's actual output
    /// is capped while reading; neither trusts the other.
    pub fn open_with_limits(bytes: &[u8], limits: PackageLoadLimits) -> Result<Self> {
        let mut archive = package_limits::open_checked_archive(bytes, limits)?;
        let mut parts = HashMap::new();
        let mut order = Vec::new();
        let mut total = 0u64;
        for index in 0..archive.len() {
            let entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_string();
            // Not `Vec::with_capacity(entry.size())`: that size comes from the
            // archive and would let it pick our allocation.
            let contents = package_limits::read_part(entry, limits, &mut total)?;
            if parts.insert(name.clone(), contents).is_none() {
                order.push(name);
            }
        }
        Ok(Self {
            parts,
            order,
            modified: HashSet::new(),
            origin: std::sync::Arc::new(()),
        })
    }

    /// Open for editing without retaining another copy of the input archive.
    pub fn open_preserving(bytes: &[u8]) -> Result<PreservingOpcPackage<'_>> {
        let package = Self::open(bytes)?;
        Ok(PreservingOpcPackage {
            origin: package.origin.clone(),
            package,
            original: bytes,
        })
    }

    /// Part names in archive order.
    pub fn part_names(&self) -> &[String] {
        &self.order
    }

    /// Replaces a part, or adds it at the end when it is new.
    pub fn replace(&mut self, name: &str, contents: Vec<u8>) {
        if self
            .parts
            .get(name)
            .is_some_and(|current| *current == contents)
        {
            return;
        }
        self.modified.insert(name.to_string());
        if self.parts.insert(name.to_string(), contents).is_none() {
            self.order.push(name.to_string());
        }
    }

    /// Removes a part, if it is there. The rest of the package — including
    /// any relationship or content type that named it — is the caller's to
    /// keep consistent.
    pub fn remove(&mut self, name: &str) -> bool {
        if self.parts.remove(name).is_some() {
            self.modified.insert(name.to_string());
            self.order.retain(|part| part != name);
            true
        } else {
            false
        }
    }

    /// Writes the package back out.
    ///
    /// Every part that was not replaced goes back byte for byte, in the order
    /// it came in — which is what makes a save that changed one cell show up as
    /// a change to one part rather than to the whole file.
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let content_types = self.content_types_covering_new_parts();
            for name in &self.order {
                let Some(mut contents) = self.parts.get(name).map(Vec::as_slice) else {
                    continue;
                };
                if name == CONTENT_TYPES_PART {
                    if let Some(patched) = &content_types {
                        contents = patched;
                    }
                }
                writer.start_file(name, options)?;
                std::io::Write::write_all(&mut writer, contents)?;
            }
            writer.finish()?;
        }
        Ok(buffer)
    }

    /// `[Content_Types].xml` with a `Default` added for each extension of a
    /// part written here that it gives no type — a relationships part or a
    /// picture added to a package that declares every part by `Override`,
    /// as some writers do — or `None` when every part has one already.
    /// Office refuses, or repairs, a package with a part it cannot type.
    fn content_types_covering_new_parts(&self) -> Option<Vec<u8>> {
        let xml = std::str::from_utf8(self.parts.get(CONTENT_TYPES_PART)?).ok()?;
        let attribute = |element: &str, key: &str| -> Option<String> {
            let at = element.find(&format!("{key}=\""))? + key.len() + 2;
            let end = element[at..].find('"')?;
            Some(element[at..at + end].to_string())
        };
        let mut defaults = HashSet::new();
        let mut overrides = HashSet::new();
        for element in xml.split('<').skip(1) {
            if element.starts_with("Default") {
                if let Some(extension) = attribute(element, "Extension") {
                    defaults.insert(extension.to_ascii_lowercase());
                }
            } else if element.starts_with("Override") {
                if let Some(part) = attribute(element, "PartName") {
                    overrides.insert(part.to_ascii_lowercase());
                }
            }
        }
        let mut added = Vec::new();
        for name in self
            .order
            .iter()
            .filter(|name| self.modified.contains(*name))
        {
            if name == CONTENT_TYPES_PART
                || overrides.contains(&format!("/{}", name.to_ascii_lowercase()))
            {
                continue;
            }
            let Some((_, extension)) = name.rsplit_once('.') else {
                continue;
            };
            let extension = extension.to_ascii_lowercase();
            if defaults.contains(&extension) {
                continue;
            }
            if let Some(content_type) = default_content_type(&extension) {
                defaults.insert(extension.clone());
                added.push(format!(
                    "<Default Extension=\"{extension}\" ContentType=\"{content_type}\"/>"
                ));
            }
        }
        if added.is_empty() {
            return None;
        }
        let close = xml.rfind("</")?;
        let mut patched = String::with_capacity(xml.len() + 100 * added.len());
        patched.push_str(&xml[..close]);
        for default in added {
            patched.push_str(&default);
        }
        patched.push_str(&xml[close..]);
        Some(patched.into_bytes())
    }

    pub fn part(&self, name: &str) -> Option<&[u8]> {
        self.parts.get(name).map(Vec::as_slice)
    }

    /// The part as text, or `None` when the archive has no such part.
    pub fn part_str(&self, name: &str) -> Result<Option<String>> {
        match self.part(name) {
            None => Ok(None),
            Some(bytes) => {
                String::from_utf8(bytes.to_vec())
                    .map(Some)
                    .map_err(|_| OpcError::Encoding {
                        part: name.to_string(),
                    })
            }
        }
    }

    pub fn required_part_str(&self, name: &str) -> Result<String> {
        self.part_str(name)?
            .ok_or_else(|| OpcError::MissingPart(name.to_string()))
    }
}

/// One entry of a `.rels` part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    pub relationship_type: String,
    pub target: String,
    pub external: bool,
}

/// The relationships a `.rels` part declares, keyed by id.
#[derive(Debug, Clone, Default)]
pub struct Relationships(HashMap<String, Relationship>);

impl Relationships {
    pub fn relationship(&self, id: &str) -> Option<&Relationship> {
        self.0.get(id)
    }

    pub fn target(&self, id: &str) -> Option<&str> {
        self.relationship(id).map(|entry| entry.target.as_str())
    }

    /// The first target whose type ends with `suffix` (`/styles`,
    /// `/sharedStrings`, ...). Types are namespaced URIs that differ between
    /// the strict and transitional conventions, and only the last segment is
    /// stable across them.
    pub fn target_by_type_suffix(&self, suffix: &str) -> Option<&str> {
        let mut matches: Vec<&Relationship> = self
            .0
            .values()
            .filter(|entry| entry.relationship_type.ends_with(suffix))
            .collect();
        // HashMap iteration order is not stable, so pick deterministically.
        matches.sort_by(|a, b| a.id.cmp(&b.id));
        matches.first().map(|entry| entry.target.as_str())
    }

    /// Every relationship whose type ends with `suffix`, in id order — a
    /// worksheet may have several tables, a drawing several charts.
    pub fn by_type_suffix(&self, suffix: &str) -> Vec<&Relationship> {
        let mut matches: Vec<&Relationship> = self
            .0
            .values()
            .filter(|entry| entry.relationship_type.ends_with(suffix))
            .collect();
        matches.sort_by(|a, b| a.id.cmp(&b.id));
        matches
    }

    /// Every relationship, in id order.
    pub fn all(&self) -> Vec<&Relationship> {
        let mut entries: Vec<&Relationship> = self.0.values().collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries
    }
}

/// Reads a `.rels` part.
pub fn parse_relationships(xml: &str) -> Result<Relationships> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut relationships = HashMap::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) | Event::Empty(ref element)
                if local_name(element.name().as_ref()) == b"Relationship" =>
            {
                let mut id = None;
                let mut relationship_type = None;
                let mut target = None;
                let mut external = false;
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    match local_name(attribute.key.as_ref()) {
                        b"Id" => id = Some(attribute.unescape_value()?.into_owned()),
                        b"Type" => {
                            relationship_type = Some(attribute.unescape_value()?.into_owned())
                        }
                        b"Target" => target = Some(attribute.unescape_value()?.into_owned()),
                        b"TargetMode" => {
                            external = attribute.unescape_value()?.as_ref() == "External"
                        }
                        _ => {}
                    }
                }
                if let (Some(id), Some(target)) = (id, target) {
                    relationships.insert(
                        id.clone(),
                        Relationship {
                            id,
                            relationship_type: relationship_type.unwrap_or_default(),
                            target,
                            external,
                        },
                    );
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(Relationships(relationships))
}

/// The `.rels` part that describes `part_name`.
pub fn rels_part_name(part_name: &str) -> String {
    match part_name.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part_name}.rels"),
    }
}

/// Resolves a relationship target against the part that declared it.
///
/// Targets are usually relative (`worksheets/sheet1.xml` from `xl/workbook.xml`)
/// but Excel is allowed to write them absolute (`/xl/worksheets/sheet1.xml`),
/// and files in the wild do both.
pub fn resolve_target(base_part: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_string();
    }
    let base_dir = base_part.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let mut segments: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for segment in target.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

/// The tag name without its namespace prefix.
pub fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|byte| *byte == b':') {
        Some(index) => &name[index + 1..],
        None => name,
    }
}

#[cfg(test)]
mod archive_comment_result_tests {
    use super::*;
    #[test]
    fn both_zip_api_returns_preserve_success_and_report_failure() {
        assert!(().into_save_result().is_ok());
        assert!(Ok::<(), zip::result::ZipError>(())
            .into_save_result()
            .is_ok());
        assert!(matches!(
            Err::<(), _>(zip::result::ZipError::FileNotFound).into_save_result(),
            Err(OpcError::Archive(zip::result::ZipError::FileNotFound))
        ));
    }
}
