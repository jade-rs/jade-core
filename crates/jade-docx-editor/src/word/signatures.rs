//! デジタル署名 and 署名欄 for every shell: the signatures a document
//! carries (who, when, why, still valid or not), what editing does to them —
//! they stop holding, and Word takes them out with the next save — signing
//! a document with a certificate the shell provides, and the signature lines
//! a document asks to be signed on.
//!
//! The checking and signing is `jade-opc`'s; the picture of a signature
//! line is a small page the shell renders with its own fonts from
//! [`signature_line_docx`].

use crate::commands::insert_image::InsertImageCommand;
use crate::commands::insert_signature_line::InsertSignatureLineCommand;
pub use crate::document::SignatureLine;
pub use jade_opc::signature::{
    is_signed, read_signatures, sign_package, strip_signatures, PackageSignature, SignatureStatus,
    Signer,
};
use jade_selection::{SelectionRange, TextPosition};

use super::WordSession;

/// A signing waiting for the save that makes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSign {
    /// `YYYY-MM-DDThh:mm:ssZ`.
    pub signed_at: String,
    /// 署名の目的.
    pub comment: String,
}

/// A document's signatures, as a view keeps them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignatureState {
    pub signatures: Vec<PackageSignature>,
    /// The document was edited since it was opened: its signatures no
    /// longer hold, and the next save takes them out.
    pub edited: bool,
    pub pending: Option<PendingSign>,
}

impl SignatureState {
    /// For a document just opened from `package`.
    pub fn opened(package: &[u8]) -> Self {
        Self {
            signatures: read_signatures(package),
            ..Self::default()
        }
    }

    pub fn is_signed(&self) -> bool {
        !self.signatures.is_empty()
    }

    /// A signature as the 署名 pane shows it: invalid once the document is
    /// edited.
    pub fn status(&self, signature: &PackageSignature) -> SignatureStatus {
        if self.edited {
            SignatureStatus::Invalid("the document was edited after it was signed".into())
        } else {
            signature.status.clone()
        }
    }

    /// The package a save writes: without the signatures once the document
    /// was edited, and signed when a signing waits for this save.
    pub fn finish_save(
        &mut self,
        package: Vec<u8>,
        signer: Option<&dyn Signer>,
    ) -> Result<Vec<u8>, String> {
        let mut package = package;
        if self.edited && is_signed(&package) {
            package = strip_signatures(&package)?;
        }
        if let Some(pending) = self.pending.take() {
            let signer = signer.ok_or("no certificate to sign with")?;
            package = sign_package(&package, signer, &pending.signed_at, &pending.comment)?;
            self.signatures = read_signatures(&package);
            self.edited = false;
        } else if self.edited {
            self.signatures.clear();
        }
        Ok(package)
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The width and height of a signature line's picture, in CSS px.
pub const SIGNATURE_LINE_SIZE: (f64, f64) = (256.0, 128.0);

/// A one-page document drawing a signature line as Word draws its
/// picture: a large ✕ on the line, the signer's name and title below.
/// The shell renders its page and inserts that.
pub fn signature_line_docx(line: &SignatureLine) -> Vec<u8> {
    use std::io::Write;
    let (width, height) = SIGNATURE_LINE_SIZE;
    // Twips: 15 per CSS px.
    let (page_w, page_h) = ((width * 15.0) as u32, (height * 15.0) as u32);
    let text = |value: &Option<String>| escape(value.as_deref().unwrap_or(""));
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:spacing w:before="360" w:after="0" w:line="240" w:lineRule="auto"/><w:ind w:left="120" w:right="120"/><w:pBdr><w:bottom w:val="single" w:sz="12" w:space="1" w:color="000000"/></w:pBdr></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="56"/></w:rPr><w:t>X</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:before="40" w:after="0" w:line="240" w:lineRule="auto"/><w:ind w:left="120"/></w:pPr><w:r><w:rPr><w:sz w:val="18"/></w:rPr><w:t xml:space="preserve">{signer}</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/><w:ind w:left="120"/></w:pPr><w:r><w:rPr><w:sz w:val="18"/></w:rPr><w:t xml:space="preserve">{title}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="{page_w}" w:h="{page_h}"/><w:pgMar w:top="0" w:right="0" w:bottom="0" w:left="0" w:header="0" w:footer="0" w:gutter="0"/></w:sectPr></w:body></w:document>"#,
        signer = text(&line.signer),
        title = text(&line.title),
    );
    let parts = [
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string()),
        ("word/document.xml", document),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in parts {
        if zip
            .start_file(name, zip::write::SimpleFileOptions::default())
            .is_ok()
        {
            zip.write_all(content.as_bytes()).ok();
        }
    }
    zip.finish()
        .map(|cursor| cursor.into_inner())
        .unwrap_or_default()
}

/// RGBA pixels as a PNG.
pub fn rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(rgba)
            .map_err(|error| error.to_string())?;
    }
    Ok(out)
}

/// 署名欄の設定: who is to sign, as the dialog takes it.
pub struct SignatureLineForm {
    /// 署名者, 署名者の役職, 電子メール, 署名者への説明.
    pub fields: [crate::text_edit::TextEdit; 4],
    pub focus: usize,
}

impl Default for SignatureLineForm {
    fn default() -> Self {
        Self {
            fields: std::array::from_fn(|_| crate::text_edit::TextEdit::new(String::new())),
            focus: 0,
        }
    }
}

impl SignatureLineForm {
    /// The fields' labels, as i18n keys.
    pub const LABELS: [&'static str; 4] = [
        "sign.line.signer",
        "sign.line.title_field",
        "sign.line.email",
        "sign.line.instructions",
    ];

    pub fn focused(&self) -> &crate::text_edit::TextEdit {
        &self.fields[self.focus]
    }

    pub fn focused_mut(&mut self) -> &mut crate::text_edit::TextEdit {
        &mut self.fields[self.focus]
    }

    /// Tab: the next field.
    pub fn next_field(&mut self) {
        self.focus = (self.focus + 1) % self.fields.len();
    }

    /// The line the fields describe.
    pub fn line(&self) -> SignatureLine {
        let value = |index: usize| {
            let text = self.fields[index].text.trim().to_string();
            (!text.is_empty()).then_some(text)
        };
        SignatureLine {
            id: None,
            signer: value(0),
            title: value(1),
            email: value(2),
            instructions: value(3),
        }
    }
}

/// 署名欄 for every session.
pub trait WordSignatures: WordSession {
    /// 挿入 › 署名欄: `picture` (the rendered [`signature_line_docx`]) at
    /// the caret, made a signature line asking `line`'s signer to sign.
    fn insert_signature_line(
        &mut self,
        line: SignatureLine,
        picture: Vec<u8>,
    ) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, _) = range.normalized();
        let caret = SelectionRange::collapsed(TextPosition::new(start.paragraph, start.offset));
        self.set_selection_range(Some(caret));
        let (width, height) = SIGNATURE_LINE_SIZE;
        self.execute_command(
            Box::new(InsertSignatureLineCommand::new(
                InsertImageCommand::new(
                    caret,
                    picture,
                    "image/png".to_string(),
                    width,
                    height,
                    None,
                    None,
                ),
                line,
            )),
            false,
        )
    }

    /// The signature lines the document asks to be signed on, in order.
    fn signature_lines(&self) -> Vec<SignatureLine> {
        self.word_document()
            .signature_lines()
            .into_iter()
            .map(|(_, _, line)| line)
            .collect()
    }
}

impl<T: WordSession + ?Sized> WordSignatures for T {}
