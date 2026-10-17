use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use std::iter::FromIterator;

use thiserror::Error;
use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

mod mce;

pub use mce::{
    RepairedRootTag, is_wordprocessing_xml_part, repair_markup_compatibility_prefixes,
    repair_markup_compatibility_root_tag,
};

/// エクスポート時に発生するエラー。
#[derive(Debug, Error)]
pub enum DocxWriteError {
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid OPC metadata: {0}")]
    Opc(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedPart {
    pub path: String,
    pub data: Vec<u8>,
    pub content_type: String,
    pub relationship: Option<AddedPartRelationship>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedPartRelationship {
    pub source_part: String,
    pub id: String,
    pub relationship_type: String,
    pub target_mode: Option<String>,
}

/// Adds package parts together with the OPC declarations that make them
/// reachable and type-safe when Word reopens the document.
pub fn write_docx_with_added_parts(
    original_files: &HashMap<String, Vec<u8>>,
    overrides: &HashMap<String, Vec<u8>>,
    added_parts: &[AddedPart],
) -> Result<Vec<u8>, DocxWriteError> {
    let mut merged = overrides.clone();
    for part in added_parts {
        validate_part_path(&part.path)?;
        merged.insert(part.path.clone(), part.data.clone());
    }
    if !added_parts.is_empty() {
        let mut content_types =
            effective_utf8_part(original_files, &merged, "[Content_Types].xml")?;
        for part in added_parts {
            let part_name = format!("/{}", part.path);
            if !xml_attribute_value_exists(&content_types, "PartName", &part_name) {
                let declaration = format!(
                    "<Override PartName=\"{}\" ContentType=\"{}\"/>",
                    xml_escape_attribute(&part_name),
                    xml_escape_attribute(&part.content_type),
                );
                insert_before_closing_tag(&mut content_types, "Types", &declaration)?;
            }
        }
        merged.insert(
            "[Content_Types].xml".to_string(),
            content_types.into_bytes(),
        );

        for part in added_parts {
            let Some(relationship) = part.relationship.as_ref() else {
                continue;
            };
            validate_part_path(&relationship.source_part)?;
            let relationship_path = relationship_part_path(&relationship.source_part)?;
            let mut relationships = match effective_optional_utf8_part(
                original_files,
                &merged,
                &relationship_path,
            )? {
                Some(xml) => xml,
                None => "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".to_string(),
            };
            if xml_attribute_value_exists(&relationships, "Id", &relationship.id) {
                return Err(DocxWriteError::Opc(format!(
                    "relationship id {} already exists in {}",
                    relationship.id, relationship_path
                )));
            }
            let target = relative_part_target(&relationship.source_part, &part.path)?;
            let target_mode = relationship
                .target_mode
                .as_deref()
                .map(|mode| format!(" TargetMode=\"{}\"", xml_escape_attribute(mode)))
                .unwrap_or_default();
            let declaration = format!(
                "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
                xml_escape_attribute(&relationship.id),
                xml_escape_attribute(&relationship.relationship_type),
                xml_escape_attribute(&target),
                target_mode,
            );
            insert_before_closing_tag(&mut relationships, "Relationships", &declaration)?;
            merged.insert(relationship_path, relationships.into_bytes());
        }
    }
    write_docx(original_files, &merged)
}

fn effective_utf8_part(
    original_files: &HashMap<String, Vec<u8>>,
    overrides: &HashMap<String, Vec<u8>>,
    path: &str,
) -> Result<String, DocxWriteError> {
    effective_optional_utf8_part(original_files, overrides, path)?
        .ok_or_else(|| DocxWriteError::Opc(format!("required package part {path} is missing")))
}

fn effective_optional_utf8_part(
    original_files: &HashMap<String, Vec<u8>>,
    overrides: &HashMap<String, Vec<u8>>,
    path: &str,
) -> Result<Option<String>, DocxWriteError> {
    overrides
        .get(path)
        .or_else(|| original_files.get(path))
        .map(|bytes| {
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| DocxWriteError::Opc(format!("package part {path} is not UTF-8 XML")))
        })
        .transpose()
}

fn validate_part_path(path: &str) -> Result<(), DocxWriteError> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(DocxWriteError::Opc(format!(
            "invalid package part path {path:?}"
        )));
    }
    Ok(())
}

fn relationship_part_path(source_part: &str) -> Result<String, DocxWriteError> {
    let (directory, filename) = source_part.rsplit_once('/').unwrap_or(("", source_part));
    if filename.is_empty() {
        return Err(DocxWriteError::Opc(
            "relationship source has no filename".to_string(),
        ));
    }
    Ok(if directory.is_empty() {
        format!("_rels/{filename}.rels")
    } else {
        format!("{directory}/_rels/{filename}.rels")
    })
}

fn relative_part_target(source_part: &str, target_part: &str) -> Result<String, DocxWriteError> {
    let mut source = source_part.split('/').collect::<Vec<_>>();
    source.pop();
    let target = target_part.split('/').collect::<Vec<_>>();
    let common = source
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = Vec::new();
    relative.extend(std::iter::repeat_n(
        "..",
        source.len().saturating_sub(common),
    ));
    relative.extend(target[common..].iter().copied());
    if relative.is_empty() {
        return Err(DocxWriteError::Opc(
            "relationship target cannot equal its source directory".to_string(),
        ));
    }
    Ok(relative.join("/"))
}

fn insert_before_closing_tag(
    xml: &mut String,
    root_name: &str,
    element: &str,
) -> Result<(), DocxWriteError> {
    let closing = format!("</{root_name}>");
    if let Some(index) = xml.rfind(&closing) {
        xml.insert_str(index, element);
        return Ok(());
    }
    let self_closing = xml.rfind("/>").ok_or_else(|| {
        DocxWriteError::Opc(format!("{root_name} XML has no closing root element"))
    })?;
    let root_start = xml[..self_closing]
        .rfind('<')
        .ok_or_else(|| DocxWriteError::Opc(format!("{root_name} XML root is malformed")))?;
    let name = xml[root_start + 1..]
        .split(|ch: char| ch.is_ascii_whitespace() || ch == '/' || ch == '>')
        .next()
        .unwrap_or("");
    if name != root_name {
        return Err(DocxWriteError::Opc(format!(
            "expected {root_name} XML root, found {name:?}"
        )));
    }
    xml.replace_range(self_closing..self_closing + 2, ">");
    xml.push_str(element);
    xml.push_str(&closing);
    Ok(())
}

fn xml_attribute_value_exists(xml: &str, name: &str, value: &str) -> bool {
    let double_quoted = format!("{name}=\"{}\"", xml_escape_attribute(value));
    let single_quoted = format!(
        "{name}='{}'",
        value.replace('&', "&amp;").replace('\'', "&apos;")
    );
    xml.contains(&double_quoted) || xml.contains(&single_quoted)
}

fn xml_escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 既存のファイル群を元に DOCX パッケージを再生成する。
///
/// `original_files` には元の OPC パッケージに含まれていたすべてのファイルを渡す。
/// `overrides` には更新したいパスとファイル内容を指定する。存在しないパスを渡すと
/// 新規ファイルとしてパッケージに追加される。
pub fn write_docx(
    original_files: &HashMap<String, Vec<u8>>,
    overrides: &HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>, DocxWriteError> {
    write_docx_with_removed(original_files, overrides, &HashSet::new())
}

/// Rebuilds an existing DOCX while omitting package parts in `removed`.
///
/// Deleting an object from `word/document.xml` is not enough when its media
/// part was present in the source archive: without an explicit tombstone the
/// immutable source map would put that orphan back into every export.
pub fn write_docx_with_removed(
    original_files: &HashMap<String, Vec<u8>>,
    overrides: &HashMap<String, Vec<u8>>,
    removed: &HashSet<String>,
) -> Result<Vec<u8>, DocxWriteError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut keys = Vec::from_iter(original_files.keys().cloned());
    for key in overrides.keys() {
        if !original_files.contains_key(key) {
            keys.push(key.clone());
        }
    }
    keys.sort();
    keys.dedup();

    for key in keys {
        if removed.contains(&key) {
            continue;
        }
        writer.start_file(&key, options)?;
        if let Some(data) = overrides.get(&key) {
            write_part(&mut writer, &key, data)?;
        } else if let Some(data) = original_files.get(&key) {
            write_part(&mut writer, &key, data)?;
        } else {
            // overrides に追加されていて original には存在しなかった場合は
            // 上の if ブロックで処理済みなのでここには到達しないが、将来の変更に備えて空ファイルを書き込む。
            writer.write_all(&[])?;
        }
    }

    let cursor = writer.finish()?;
    Ok(cursor.into_inner())
}

/// Word refuses a package whose `mc:Ignorable` names an undeclared prefix, so
/// every WordprocessingML part is checked on the way out — including parts
/// jade never edited, because the source document may already be broken that
/// way (see [`mce`]). A consistent part keeps its exact bytes, and a repaired
/// one is streamed around its rewritten root tag rather than copied: exports
/// run on every save and on every hash a test takes, and `document.xml` can
/// be several megabytes.
fn write_part<W: Write>(writer: &mut W, path: &str, data: &[u8]) -> std::io::Result<()> {
    let repaired = if is_wordprocessing_xml_part(path) {
        repair_markup_compatibility_root_tag(data)
    } else {
        None
    };
    match repaired {
        Some(root) => {
            writer.write_all(&data[..root.range.start])?;
            writer.write_all(&root.tag)?;
            writer.write_all(&data[root.range.end..])
        }
        None => writer.write_all(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use zip::ZipArchive;

    #[test]
    fn write_docx_replaces_overridden_files() {
        let mut original = HashMap::new();
        original.insert(
            "word/document.xml".to_string(),
            b"<doc>original</doc>".to_vec(),
        );
        original.insert("docProps/app.xml".to_string(), b"app".to_vec());

        let mut overrides = HashMap::new();
        overrides.insert(
            "word/document.xml".to_string(),
            b"<doc>updated</doc>".to_vec(),
        );

        let bytes = write_docx(&original, &overrides).expect("write docx");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip archive");

        let mut updated = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut updated)
            .expect("read document");
        assert_eq!(updated, "<doc>updated</doc>");

        let mut app = String::new();
        archive
            .by_name("docProps/app.xml")
            .expect("app.xml")
            .read_to_string(&mut app)
            .expect("read app");
        assert_eq!(app, "app");
    }

    #[test]
    fn write_docx_adds_new_entries() {
        let original = HashMap::new();
        let mut overrides = HashMap::new();
        overrides.insert("custom/file.txt".to_string(), b"hello".to_vec());

        let bytes = write_docx(&original, &overrides).expect("write docx");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip archive");

        let mut file = String::new();
        archive
            .by_name("custom/file.txt")
            .expect("custom file")
            .read_to_string(&mut file)
            .expect("read file");
        assert_eq!(file, "hello");
    }

    #[test]
    fn added_part_updates_content_types_and_source_relationships() {
        let original = HashMap::from([
            (
                "[Content_Types].xml".to_string(),
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_vec(),
            ),
            (
                "word/_rels/document.xml.rels".to_string(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#.to_vec(),
            ),
        ]);
        let added = AddedPart {
            path: "word/media/image1.png".to_string(),
            data: b"png".to_vec(),
            content_type: "image/png".to_string(),
            relationship: Some(AddedPartRelationship {
                source_part: "word/document.xml".to_string(),
                id: "rId7".to_string(),
                relationship_type:
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
                        .to_string(),
                target_mode: None,
            }),
        };

        let bytes = write_docx_with_added_parts(&original, &HashMap::new(), &[added])
            .expect("write package with added part");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip archive");
        let mut content_types = String::new();
        archive
            .by_name("[Content_Types].xml")
            .expect("content types")
            .read_to_string(&mut content_types)
            .expect("read content types");
        assert!(content_types.contains("PartName=\"/word/media/image1.png\""));
        assert!(content_types.contains("ContentType=\"image/png\""));

        let mut relationships = String::new();
        archive
            .by_name("word/_rels/document.xml.rels")
            .expect("document relationships")
            .read_to_string(&mut relationships)
            .expect("read relationships");
        assert!(relationships.contains("Id=\"rId7\""));
        assert!(relationships.contains("Target=\"media/image1.png\""));
    }

    #[test]
    fn write_docx_omits_removed_source_entries_and_removed_overrides() {
        let mut original = HashMap::new();
        original.insert("word/document.xml".to_string(), b"document".to_vec());
        original.insert("word/media/ink.png".to_string(), b"old".to_vec());

        let mut overrides = HashMap::new();
        overrides.insert("word/media/ink.png".to_string(), b"new".to_vec());
        let removed = HashSet::from(["word/media/ink.png".to_string()]);

        let bytes = write_docx_with_removed(&original, &overrides, &removed).expect("write docx");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip archive");
        assert!(archive.by_name("word/document.xml").is_ok());
        assert!(archive.by_name("word/media/ink.png").is_err());
    }

    #[test]
    fn write_docx_declares_ignorable_prefixes_in_untouched_and_overridden_parts() {
        let broken = "<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"w14 w15\"/>";
        let mut original = HashMap::new();
        original.insert("word/styles.xml".to_string(), broken.as_bytes().to_vec());
        original.insert("word/document.xml".to_string(), b"<w:document/>".to_vec());
        original.insert(
            "docProps/app.xml".to_string(),
            b"<Properties mc:Ignorable=\"w14\"/>".to_vec(),
        );
        let mut overrides = HashMap::new();
        overrides.insert(
            "word/document.xml".to_string(),
            broken.replace("w:styles", "w:document").into_bytes(),
        );

        let bytes = write_docx(&original, &overrides).expect("write docx");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip archive");
        let read = |archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str| {
            let mut text = String::new();
            archive
                .by_name(name)
                .expect(name)
                .read_to_string(&mut text)
                .expect("read part");
            text
        };

        for part in ["word/styles.xml", "word/document.xml"] {
            let text = read(&mut archive, part);
            assert!(
                text.contains(
                    " xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\""
                ),
                "{part}: {text}"
            );
            assert!(
                text.contains(
                    " xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\""
                ),
                "{part}: {text}"
            );
        }
        // Parts outside the WordprocessingML story are copied byte for byte.
        assert_eq!(
            read(&mut archive, "docProps/app.xml"),
            "<Properties mc:Ignorable=\"w14\"/>"
        );
    }
}
