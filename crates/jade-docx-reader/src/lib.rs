pub mod color;
pub mod comments;
pub mod common;
pub mod compatibility;
pub mod document;
pub mod docx_xml;
pub mod embedded_font;
pub mod field_datetime;
pub mod field_format;
pub mod font_table;
pub mod numbering;
pub mod package_limits;
pub mod parser;
pub mod settings;
pub mod styles;
pub mod theme;

pub use comments::CommentsPart;
pub use common::{OpenXmlPackage, Relationship, RelationshipTypes};
pub use document::*;
pub use docx_xml::{
    DocxXmlDocument, DocxXmlError, DocxXmlTreeBuilder, XmlAttribute, XmlEditJournal, XmlElement,
    XmlNode, XmlNodeId, XmlNodePath,
};
pub use embedded_font::{docx_embedded_fonts, package_embedded_fonts, EmbeddedFont};
pub use font_table::{EmbeddedFontReference, EmbeddedFontStyle, FontTablePart};
pub use numbering::NumberingPart;
pub use parser::{DocumentParser, RawTreeTeeReader, XmlEventSource};
pub use settings::DocumentSettings;
pub use styles::{
    merge_paragraph_properties_override, merge_run_properties_override, StyleDefinition,
    StylesPart, TableStyleInfo,
};
pub use theme::ThemePart;
