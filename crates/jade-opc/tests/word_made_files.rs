//! Packages Word itself wrote: a password-protected (Agile Encryption)
//! document and an XML-DSig signed one, read the way jade reads them.
//!
//! No such files ship with the repository yet; these tests look for them in
//! `public/fixtures/word-made/` and pass, saying so, while they are missing.
//! To make them, in Word for Mac (16.x) or Word for Windows:
//!
//! **`agile-encrypted.docx`** — Word's 読み取りパスワード (Agile Encryption,
//! AES-256 / SHA-512, the default since Word 2013):
//! 1. 新規文書 (Blank document); type exactly `Word-made encrypted document`.
//! 2. Mac: 校閲 → 文書の保護 (Protect Document) → このドキュメントを開くには
//!    パスワードが必要です; Windows: ファイル → 情報 → 文書の保護 →
//!    パスワードを使用して暗号化. Enter `jade-word-made` twice.
//! 3. Save as Word 文書 (.docx) named `agile-encrypted.docx`.
//!
//! **`xmldsig-signed.docx`** — a digital signature (Word for Windows; Word for
//! Mac can only show signatures):
//! 1. 新規文書; type exactly `Word-made signed document`; save it as
//!    `xmldsig-signed.docx` first (Word signs a saved file).
//! 2. ファイル → 情報 → 文書の保護 → デジタル署名を追加 (Add a Digital
//!    Signature); pick any certificate with a private key (a self-signed one
//!    from `SelfCert.exe` or `New-SelfSignedCertificate -Type CodeSigningCert`
//!    is fine), purpose `jade test`, 署名 (Sign). Word saves the signed file;
//!    do not edit it afterwards.
//!
//! Copy both files into `public/fixtures/word-made/` and run
//! `cargo test -p jade-opc --test word_made_files`.

use std::io::Read;
use std::path::PathBuf;

const PASSWORD: &str = "jade-word-made";

fn word_made(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures/word-made")
        .join(name);
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!(
                "skipped: {} is not there (see this file's steps to make it in Word)",
                path.display()
            );
            None
        }
    }
}

fn document_xml(package: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(package)).expect("a zip package");
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .expect("word/document.xml")
        .read_to_string(&mut xml)
        .expect("utf-8");
    xml
}

#[test]
fn a_document_word_encrypted_opens_with_its_password_only() {
    let Some(bytes) = word_made("agile-encrypted.docx") else {
        return;
    };
    assert!(jade_opc::encryption::is_encrypted_package(&bytes));
    let package =
        jade_opc::encryption::decrypt_package(&bytes, PASSWORD).expect("Word's own encryption");
    assert!(document_xml(&package).contains("Word-made encrypted document"));
    assert!(jade_opc::encryption::decrypt_package(&bytes, "not the password").is_err());

    // What jade encrypts with the same password opens the same way.
    let again = jade_opc::encryption::encrypt_package(&package, PASSWORD, &mut |buffer| {
        for (index, byte) in buffer.iter_mut().enumerate() {
            *byte = (index * 31 + 7) as u8;
        }
    })
    .expect("encrypt");
    assert_eq!(
        jade_opc::encryption::decrypt_package(&again, PASSWORD).unwrap(),
        package
    );
}

#[test]
fn a_document_word_signed_verifies_until_it_changes() {
    let Some(bytes) = word_made("xmldsig-signed.docx") else {
        return;
    };
    assert!(jade_opc::signature::is_signed(&bytes));
    let signatures = jade_opc::signature::read_signatures(&bytes);
    assert!(!signatures.is_empty());
    for signature in &signatures {
        assert_eq!(
            signature.status,
            jade_opc::signature::SignatureStatus::Valid,
            "{signature:?}"
        );
    }
    assert!(document_xml(&bytes).contains("Word-made signed document"));

    // The signed text changed: the signature no longer holds.
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut changed = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut changed);
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            let name = file.name().unwrap().into_owned();
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            if name == "word/document.xml" {
                data = String::from_utf8(data)
                    .unwrap()
                    .replace("Word-made signed document", "Word-made signed documenT")
                    .into_bytes();
            }
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut writer, &data).unwrap();
        }
        writer.finish().unwrap();
    }
    let changed = changed.into_inner();
    assert!(jade_opc::signature::read_signatures(&changed)
        .iter()
        .all(|signature| signature.status != jade_opc::signature::SignatureStatus::Valid));
}
