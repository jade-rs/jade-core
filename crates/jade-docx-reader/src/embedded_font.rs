//! Fonts the author embedded in the package (`word/fonts/*.odttf`).
//!
//! An embedded font is the document's own glyphs and advances, so it lets a
//! host without the face lay the document out as it was written, under the
//! licence the font itself grants for embedding.
//!
//! ECMA-376 Part 1 §17.8.1 obfuscates the part: the first 32 bytes of the
//! font file are XORed with the 16-byte key spelled by `w:fontKey`, a GUID
//! whose hex digits are read in reverse byte order. Undoing that gives the
//! original TrueType/OpenType file. The package part itself is never rewritten:
//! saving round-trips the obfuscated bytes untouched.

use crate::common::{OpenXmlPackage, RelationshipTypes};
use crate::font_table::{EmbeddedFontStyle, FontTablePart};

/// Number of leading font bytes the obfuscation covers (§17.8.1).
const OBFUSCATED_PREFIX_LEN: usize = 32;

/// A deobfuscated embedded font, ready to register with a font host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedFont {
    /// The `w:font/@w:name` family the file supplies.
    pub family: String,
    pub style: EmbeddedFontStyle,
    /// `w:subsetted`: only the glyphs the document uses are present.
    pub subsetted: bool,
    /// The TrueType/OpenType file.
    pub data: Vec<u8>,
}

/// Why an embedded font part was not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddedFontError {
    /// `w:fontKey` is not a GUID.
    InvalidFontKey,
    /// The part is shorter than the obfuscated prefix.
    Truncated,
    /// The (deobfuscated) bytes are not a TrueType/OpenType file.
    NotAFont,
    /// The font's OS/2 `fsType` does not allow it to be used from a document.
    EmbeddingNotPermitted(EmbeddingPermission),
}

impl std::fmt::Display for EmbeddedFontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFontKey => write!(f, "w:fontKey is not a GUID"),
            Self::Truncated => write!(f, "embedded font part is truncated"),
            Self::NotAFont => write!(f, "embedded font part is not a TrueType/OpenType font"),
            Self::EmbeddingNotPermitted(permission) => {
                write!(f, "font embedding is not permitted ({permission:?})")
            }
        }
    }
}

impl std::error::Error for EmbeddedFontError {}

/// The embedding licence an OpenType `OS/2.fsType` grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingPermission {
    /// No OS/2 table: the TrueType default, which places no restriction.
    Unrestricted,
    /// Bits 0-3 clear: installable embedding.
    Installable,
    /// Bit 2 (`0x0004`): preview and print.
    PreviewAndPrint,
    /// Bit 3 (`0x0008`): editable embedding.
    Editable,
    /// Bits 0-3 equal to `0x0002`: the font must not be embedded or used.
    Restricted,
    /// Bit 9 (`0x0200`): only bitmaps may be embedded, never the outlines.
    BitmapOnly,
}

impl EmbeddingPermission {
    /// Whether a viewer may use the font's outlines to lay out and draw the
    /// document it came with.
    pub fn allows_use(self) -> bool {
        !matches!(self, Self::Restricted | Self::BitmapOnly)
    }
}

/// The 16-byte key spelled by a `w:fontKey` GUID, in the order §17.8.1 applies
/// it: the GUID's 32 hex digits read as bytes from the last pair to the first.
pub fn font_key_bytes(font_key: &str) -> Option<[u8; 16]> {
    let hex: Vec<u8> = font_key
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .bytes()
        .filter(|byte| *byte != b'-')
        .collect();
    if hex.len() != 32 {
        return None;
    }
    let mut key = [0u8; 16];
    for (index, pair) in hex.chunks_exact(2).rev().enumerate() {
        let digits = std::str::from_utf8(pair).ok()?;
        key[index] = u8::from_str_radix(digits, 16).ok()?;
    }
    Some(key)
}

/// Undoes (or applies — the transform is its own inverse) the §17.8.1
/// obfuscation of an embedded font part.
pub fn deobfuscate_font(part: &[u8], font_key: &str) -> Result<Vec<u8>, EmbeddedFontError> {
    let key = font_key_bytes(font_key).ok_or(EmbeddedFontError::InvalidFontKey)?;
    if part.len() < OBFUSCATED_PREFIX_LEN {
        return Err(EmbeddedFontError::Truncated);
    }
    let mut font = part.to_vec();
    for (index, byte) in font[..OBFUSCATED_PREFIX_LEN].iter_mut().enumerate() {
        *byte ^= key[index % key.len()];
    }
    Ok(font)
}

/// The embedding licence of a TrueType/OpenType file, or `None` when the bytes
/// are not one (or its table directory is out of bounds).
pub fn embedding_permission(font: &[u8]) -> Option<EmbeddingPermission> {
    let version = font.get(0..4)?;
    if !matches!(version, [0x00, 0x01, 0x00, 0x00] | b"true" | b"OTTO") {
        return None;
    }
    let table_count = read_u16(font, 4)? as usize;
    for index in 0..table_count {
        let record = 12 + index * 16;
        let tag = font.get(record..record + 4)?;
        if tag != b"OS/2" {
            continue;
        }
        let offset = read_u32(font, record + 8)? as usize;
        let fs_type = read_u16(font, offset.checked_add(8)?)?;
        return Some(permission_from_fs_type(fs_type));
    }
    Some(EmbeddingPermission::Unrestricted)
}

fn permission_from_fs_type(fs_type: u16) -> EmbeddingPermission {
    if fs_type & 0x0200 != 0 {
        return EmbeddingPermission::BitmapOnly;
    }
    // Bits 0-3 are meant to be exclusive; when several are set the least
    // restrictive one applies (OpenType OS/2 `fsType`).
    let usage = fs_type & 0x000F;
    if usage & 0x0008 != 0 {
        EmbeddingPermission::Editable
    } else if usage & 0x0004 != 0 {
        EmbeddingPermission::PreviewAndPrint
    } else if usage == 0x0002 {
        EmbeddingPermission::Restricted
    } else {
        EmbeddingPermission::Installable
    }
}

/// Reads one embedded font part into a usable font file: deobfuscated when it
/// carries a `w:fontKey`, and refused when the font forbids embedding.
pub fn decode_embedded_font(
    part: &[u8],
    font_key: Option<&str>,
) -> Result<Vec<u8>, EmbeddedFontError> {
    let font = match font_key {
        Some(key) => deobfuscate_font(part, key)?,
        None => part.to_vec(),
    };
    let permission = embedding_permission(&font).ok_or(EmbeddedFontError::NotAFont)?;
    if !permission.allows_use() {
        return Err(EmbeddedFontError::EmbeddingNotPermitted(permission));
    }
    Ok(font)
}

/// Every usable embedded font in `font_table`, whose part lives at
/// `font_table_path` inside `package`. Parts that are missing, malformed or
/// not licensed for embedding are skipped; the document then falls back to
/// the installed faces exactly as it would without them.
pub fn embedded_fonts(
    package: &OpenXmlPackage,
    font_table_path: &str,
    font_table: &FontTablePart,
) -> Vec<EmbeddedFont> {
    if font_table.fonts.iter().all(|font| font.embedded.is_empty()) {
        return Vec::new();
    }
    let Some(relationships) = package
        .get(&jade_opc::rels_part_name(font_table_path))
        .and_then(|bytes| jade_opc::parse_relationships(&String::from_utf8_lossy(bytes)).ok())
    else {
        return Vec::new();
    };
    let mut fonts = Vec::new();
    for declaration in &font_table.fonts {
        for reference in &declaration.embedded {
            let Some(relationship) = relationships.relationship(&reference.relationship_id) else {
                continue;
            };
            if relationship.external {
                continue;
            }
            let part_name = jade_opc::resolve_target(font_table_path, &relationship.target);
            let Some(part) = package.get(&part_name) else {
                continue;
            };
            if let Ok(data) = decode_embedded_font(part, reference.font_key.as_deref()) {
                fonts.push(EmbeddedFont {
                    family: declaration.name.clone(),
                    style: reference.style,
                    subsetted: reference.subsetted,
                    data,
                });
            }
        }
    }
    fonts
}

/// [`embedded_fonts`] for a whole package: follows the main document part's
/// fontTable relationship. Cheap for a document without embedded fonts — only
/// the fontTable part is parsed.
pub fn package_embedded_fonts(package: &OpenXmlPackage) -> Vec<EmbeddedFont> {
    let Some(document_path) = package
        .relationships
        .iter()
        .find(|rel| rel.r#type == RelationshipTypes::OfficeDocument.as_str())
        .map(|rel| jade_opc::resolve_target("", &rel.target))
    else {
        return Vec::new();
    };
    let Some(document_relationships) = package
        .get(&jade_opc::rels_part_name(&document_path))
        .and_then(|bytes| jade_opc::parse_relationships(&String::from_utf8_lossy(bytes)).ok())
    else {
        return Vec::new();
    };
    let Some(font_table_path) = document_relationships
        .target_by_type_suffix("/fontTable")
        .map(|target| jade_opc::resolve_target(&document_path, target))
    else {
        return Vec::new();
    };
    let Some(font_table) = package
        .get(&font_table_path)
        .and_then(|bytes| FontTablePart::from_xml(&String::from_utf8_lossy(bytes)).ok())
    else {
        return Vec::new();
    };
    embedded_fonts(package, &font_table_path, &font_table)
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const KEY: &str = "{0EC2A5B4-6A1C-4C5B-9D3E-1F2A3B4C5D6E}";

    /// The smallest sfnt jade reads: a table directory with one OS/2 table.
    fn tiny_font(fs_type: u16) -> Vec<u8> {
        let mut font = Vec::new();
        font.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        font.extend_from_slice(&1u16.to_be_bytes());
        font.extend_from_slice(&[0; 6]);
        font.extend_from_slice(b"OS/2");
        font.extend_from_slice(&0u32.to_be_bytes());
        font.extend_from_slice(&28u32.to_be_bytes());
        font.extend_from_slice(&10u32.to_be_bytes());
        font.extend_from_slice(&[0; 8]);
        font.extend_from_slice(&fs_type.to_be_bytes());
        font
    }

    #[test]
    fn font_key_reads_the_guid_digits_in_reverse_byte_order() {
        let key = font_key_bytes(KEY).unwrap();
        assert_eq!(key[0], 0x6E);
        assert_eq!(key[1], 0x5D);
        assert_eq!(key[15], 0x0E);
        assert_eq!(
            font_key_bytes("0EC2A5B46A1C4C5B9D3E1F2A3B4C5D6E"),
            Some(key)
        );
        assert_eq!(font_key_bytes("{0EC2A5B4}"), None);
        assert_eq!(
            font_key_bytes("{ZZC2A5B4-6A1C-4C5B-9D3E-1F2A3B4C5D6E}"),
            None
        );
    }

    #[test]
    fn deobfuscation_xors_only_the_first_32_bytes_and_round_trips() {
        let font = tiny_font(0);
        let obfuscated = deobfuscate_font(&font, KEY).unwrap();
        let key = font_key_bytes(KEY).unwrap();
        for index in 0..32 {
            assert_eq!(obfuscated[index], font[index] ^ key[index % 16]);
        }
        assert_eq!(obfuscated[32..], font[32..]);
        assert_eq!(deobfuscate_font(&obfuscated, KEY).unwrap(), font);
        assert_eq!(
            deobfuscate_font(&font[..31], KEY),
            Err(EmbeddedFontError::Truncated)
        );
    }

    #[test]
    fn fs_type_decides_whether_the_font_may_be_used() {
        for (fs_type, permission, usable) in [
            (0x0000, EmbeddingPermission::Installable, true),
            (0x0002, EmbeddingPermission::Restricted, false),
            (0x0004, EmbeddingPermission::PreviewAndPrint, true),
            (0x0008, EmbeddingPermission::Editable, true),
            (0x0006, EmbeddingPermission::PreviewAndPrint, true),
            (0x0204, EmbeddingPermission::BitmapOnly, false),
        ] {
            let font = tiny_font(fs_type);
            assert_eq!(
                embedding_permission(&font),
                Some(permission),
                "{fs_type:#06x}"
            );
            let decoded = decode_embedded_font(&deobfuscate_font(&font, KEY).unwrap(), Some(KEY));
            assert_eq!(decoded.is_ok(), usable, "{fs_type:#06x}");
        }
        assert_eq!(embedding_permission(b"not a font at all"), None);
        assert_eq!(
            decode_embedded_font(&tiny_font(0), Some(KEY)),
            Err(EmbeddedFontError::NotAFont),
            "a clear font read as obfuscated is not a font"
        );
        assert!(decode_embedded_font(&tiny_font(0), None).is_ok());
    }

    #[test]
    fn package_embedded_fonts_follow_the_font_table_relationships() {
        let regular = tiny_font(0x0008);
        let restricted = tiny_font(0x0002);
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        files.insert(
            "word/_rels/document.xml.rels".into(),
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable" Target="fontTable.xml"/>
</Relationships>"#
                .to_vec(),
        );
        files.insert(
            "word/fontTable.xml".into(),
            format!(
                r#"<w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:font w:name="Jade Sans"><w:embedRegular r:id="rId1" w:fontKey="{KEY}"/><w:embedBold r:id="rId2" w:fontKey="{KEY}"/><w:embedItalic r:id="rId404" w:fontKey="{KEY}"/></w:font>
  <w:font w:name="Century"/>
</w:fonts>"#
            )
            .into_bytes(),
        );
        files.insert(
            "word/_rels/fontTable.xml.rels".into(),
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/font" Target="fonts/font1.odttf"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/font" Target="fonts/font2.odttf"/>
</Relationships>"#
                .to_vec(),
        );
        files.insert(
            "word/fonts/font1.odttf".into(),
            deobfuscate_font(&regular, KEY).unwrap(),
        );
        files.insert(
            "word/fonts/font2.odttf".into(),
            deobfuscate_font(&restricted, KEY).unwrap(),
        );
        let package = OpenXmlPackage {
            files,
            relationships: vec![crate::common::Relationship {
                id: "rId1".into(),
                r#type: RelationshipTypes::OfficeDocument.as_str().into(),
                target: "word/document.xml".into(),
                target_mode: None,
            }],
            original_size_bytes: None,
        };

        let fonts = package_embedded_fonts(&package);
        assert_eq!(
            fonts,
            vec![EmbeddedFont {
                family: "Jade Sans".into(),
                style: EmbeddedFontStyle::Regular,
                subsetted: false,
                data: regular,
            }],
            "the restricted bold and the missing italic part are skipped"
        );
    }
}
