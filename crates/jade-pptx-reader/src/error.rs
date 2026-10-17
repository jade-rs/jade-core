use thiserror::Error;

#[derive(Debug, Error)]
pub enum PptxError {
    #[error("package: {0}")]
    Package(#[from] jade_opc::OpcError),
    #[error("xml: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("attribute: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("text encoding: {0}")]
    TextEncoding(#[from] quick_xml::encoding::EncodingError),
    #[error("malformed xml: {0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, PptxError>;
