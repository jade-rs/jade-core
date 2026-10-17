use thiserror::Error;

/// Everything that can go wrong turning `.xlsx` bytes into a [`crate::Workbook`].
#[derive(Debug, Error)]
pub enum XlsxError {
    #[error("invalid workbook calculation mode `{0}`")]
    BadCalculationMode(String),
    #[error("package: {0}")]
    Package(#[from] jade_opc::OpcError),
    #[error("xml: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("text encoding: {0}")]
    TextEncoding(#[from] quick_xml::encoding::EncodingError),
    #[error("attribute: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("`{value}` is not a cell reference")]
    BadCellReference { value: String },
}

pub type Result<T> = std::result::Result<T, XlsxError>;
