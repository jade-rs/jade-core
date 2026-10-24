//! A package with a password to open: ECMA-376 Agile Encryption
//! ([MS-OFFCRYPTO] 2.3.4.10–2.3.4.15), what Word writes when a document is
//! given 読み取りパスワード.
//!
//! The package is not a zip then but an OLE compound file holding the
//! encrypted zip (`EncryptedPackage`), how to decrypt it (`EncryptionInfo`,
//! an XML descriptor) and the data-space streams that name the transform.
//! The key is derived from the password with SHA-512 iterated 100 000 times
//! over a random salt; the package is AES-256-CBC in 4096-byte segments, and
//! an HMAC-SHA512 over the encrypted stream guards it.

use std::io::{Cursor, Read};

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use base64::Engine;
use sha2::{Digest, Sha512};

/// Why a package could not be opened.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecryptError {
    #[error("the password is wrong")]
    WrongPassword,
    #[error("the package is encrypted in a way this cannot open: {0}")]
    Unsupported(String),
    #[error("the encrypted package is damaged: {0}")]
    Malformed(String),
}

/// The compound file signature every encrypted package starts with.
const CFB_SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

const SEGMENT: usize = 4096;
const SPIN_COUNT: u32 = 100_000;
const KEY_BYTES: usize = 32;
const BLOCK: usize = 16;
const SALT: usize = 16;
const HASH: usize = 64;

const VERIFIER_INPUT_BLOCK: [u8; 8] = [0xfe, 0xa7, 0xd2, 0x76, 0x3b, 0x4b, 0x9e, 0x79];
const VERIFIER_VALUE_BLOCK: [u8; 8] = [0xd7, 0xaa, 0x0f, 0x6d, 0x30, 0x61, 0x34, 0x4e];
const KEY_VALUE_BLOCK: [u8; 8] = [0x14, 0x6e, 0x0b, 0xe7, 0xab, 0xac, 0xd0, 0xd6];
const HMAC_KEY_BLOCK: [u8; 8] = [0x5f, 0xb2, 0xad, 0x01, 0x0c, 0xb9, 0xe1, 0xf6];
const HMAC_VALUE_BLOCK: [u8; 8] = [0xa0, 0x67, 0x7f, 0x02, 0xb2, 0x2c, 0x84, 0x33];

/// Whether `bytes` is a compound file holding an encrypted package.
pub fn is_encrypted_package(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&CFB_SIGNATURE) {
        return false;
    }
    cfb::CompoundFile::open(Cursor::new(bytes))
        .is_ok_and(|file| file.is_stream("/EncryptionInfo") && file.is_stream("/EncryptedPackage"))
}

fn sha512(parts: &[&[u8]]) -> [u8; HASH] {
    let mut hasher = Sha512::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn hmac_sha512(key: &[u8], message: &[u8]) -> [u8; HASH] {
    const BLOCK_SIZE: usize = 128;
    let mut padded = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        padded[..HASH].copy_from_slice(&sha512(&[key]));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }
    let inner_pad: Vec<u8> = padded.iter().map(|byte| byte ^ 0x36).collect();
    let outer_pad: Vec<u8> = padded.iter().map(|byte| byte ^ 0x5c).collect();
    let inner = sha512(&[&inner_pad, message]);
    sha512(&[&outer_pad, &inner])
}

/// AES-256-CBC over whole blocks, `data` padded with zeros to them.
fn aes_cbc(key: &[u8], iv: &[u8], data: &[u8], encrypt: bool) -> Result<Vec<u8>, DecryptError> {
    let cipher = aes::Aes256::new_from_slice(key)
        .map_err(|_| DecryptError::Unsupported("key length".into()))?;
    let mut out = data.to_vec();
    out.resize(data.len().div_ceil(BLOCK) * BLOCK, 0);
    let mut chain = [0u8; BLOCK];
    chain.copy_from_slice(&iv[..BLOCK]);
    for block in out.chunks_mut(BLOCK) {
        if encrypt {
            for (byte, previous) in block.iter_mut().zip(chain) {
                *byte ^= previous;
            }
            cipher.encrypt_block(block.into());
            chain.copy_from_slice(block);
        } else {
            let mut next = [0u8; BLOCK];
            next.copy_from_slice(block);
            cipher.decrypt_block(block.into());
            for (byte, previous) in block.iter_mut().zip(chain) {
                *byte ^= previous;
            }
            chain = next;
        }
    }
    Ok(out)
}

/// The password's hash after the spin count ([MS-OFFCRYPTO] 2.3.4.11).
fn password_hash(password: &str, salt: &[u8], spin_count: u32) -> [u8; HASH] {
    let utf16: Vec<u8> = password
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let mut hash = sha512(&[salt, &utf16]);
    for iteration in 0..spin_count {
        hash = sha512(&[&iteration.to_le_bytes(), &hash]);
    }
    hash
}

/// A key for `block` from the password hash, `bytes` long.
fn derived_key(hash: &[u8], block: &[u8], bytes: usize) -> Vec<u8> {
    let mut key = sha512(&[hash, block]).to_vec();
    key.resize(bytes, 0x36);
    key
}

/// The IV for data under `block` (the key data's salt and a block key).
fn derived_iv(salt: &[u8], block: &[u8]) -> Vec<u8> {
    sha512(&[salt, block])[..BLOCK].to_vec()
}

fn segment_iv(salt: &[u8], index: u32) -> Vec<u8> {
    derived_iv(salt, &index.to_le_bytes())
}

/// `EncryptedPackage`'s content: the plaintext's size, then the segments.
fn crypt_package(
    key: &[u8],
    salt: &[u8],
    data: &[u8],
    encrypt: bool,
) -> Result<Vec<u8>, DecryptError> {
    let mut out = Vec::with_capacity(data.len() + BLOCK);
    for (index, segment) in data.chunks(SEGMENT).enumerate() {
        let iv = segment_iv(salt, index as u32);
        out.extend(aes_cbc(key, &iv, segment, encrypt)?);
    }
    Ok(out)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn unb64(text: &str) -> Result<Vec<u8>, DecryptError> {
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|error| DecryptError::Malformed(format!("base64: {error}")))
}

/// UNICODE-LP-P4: a byte length, UTF-16LE, padded to four bytes.
fn unicode_lp_p4(text: &str) -> Vec<u8> {
    let utf16: Vec<u8> = text
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let mut out = (utf16.len() as u32).to_le_bytes().to_vec();
    out.extend(&utf16);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out
}

fn u32le(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// The data-space streams that name the encryption transform.
fn data_spaces() -> Vec<(&'static str, Vec<u8>)> {
    let mut version = unicode_lp_p4("Microsoft.Container.DataSpaces");
    for _ in 0..3 {
        version.extend([1, 0, 0, 0]);
    }

    let mut entry = Vec::new();
    entry.extend(u32le(1)); // one reference component
    entry.extend(u32le(0)); // a stream
    entry.extend(unicode_lp_p4("EncryptedPackage"));
    entry.extend(unicode_lp_p4("StrongEncryptionDataSpace"));
    let mut map = Vec::new();
    map.extend(u32le(8));
    map.extend(u32le(1));
    map.extend(u32le(entry.len() as u32 + 4));
    map.extend(entry);

    let mut definition = Vec::new();
    definition.extend(u32le(8));
    definition.extend(u32le(1));
    definition.extend(unicode_lp_p4("StrongEncryptionTransform"));

    let id = unicode_lp_p4("{FF9A3F03-56EF-4613-BDD5-5A41C1D07246}");
    let mut primary = Vec::new();
    primary.extend(u32le(8 + id.len() as u32));
    primary.extend(u32le(1));
    primary.extend(id);
    primary.extend(unicode_lp_p4("Microsoft.Container.EncryptionTransform"));
    for _ in 0..3 {
        primary.extend([1, 0, 0, 0]);
    }
    primary.extend(u32le(0)); // no encryption name
    primary.extend(u32le(0)); // block size
    primary.extend(u32le(0)); // cipher mode
    primary.extend(u32le(4)); // reserved

    vec![
        ("/\u{6}DataSpaces/Version", version),
        ("/\u{6}DataSpaces/DataSpaceMap", map),
        (
            "/\u{6}DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace",
            definition,
        ),
        (
            "/\u{6}DataSpaces/TransformInfo/StrongEncryptionTransform/\u{6}Primary",
            primary,
        ),
    ]
}

/// Encrypts the package `zip` with `password`, as Word does for
/// 読み取りパスワード. `random` fills buffers with random bytes: the salts,
/// the package key and the integrity key.
pub fn encrypt_package(
    zip: &[u8],
    password: &str,
    random: &mut dyn FnMut(&mut [u8]),
) -> Result<Vec<u8>, DecryptError> {
    let mut fill = |bytes: usize| {
        let mut buffer = vec![0u8; bytes];
        random(&mut buffer);
        buffer
    };
    let key_data_salt = fill(SALT);
    let password_salt = fill(SALT);
    let package_key = fill(KEY_BYTES);
    let verifier_input = fill(SALT);
    let hmac_key = fill(HASH);

    // The package.
    let mut package = (zip.len() as u64).to_le_bytes().to_vec();
    package.extend(crypt_package(&package_key, &key_data_salt, zip, true)?);

    // Its integrity.
    let hmac_value = hmac_sha512(&hmac_key, &package);
    let encrypted_hmac_key = aes_cbc(
        &package_key,
        &derived_iv(&key_data_salt, &HMAC_KEY_BLOCK),
        &hmac_key,
        true,
    )?;
    let encrypted_hmac_value = aes_cbc(
        &package_key,
        &derived_iv(&key_data_salt, &HMAC_VALUE_BLOCK),
        &hmac_value,
        true,
    )?;

    // The password's key and what proves it.
    let hash = password_hash(password, &password_salt, SPIN_COUNT);
    let encrypted_verifier_input = aes_cbc(
        &derived_key(&hash, &VERIFIER_INPUT_BLOCK, KEY_BYTES),
        &password_salt,
        &verifier_input,
        true,
    )?;
    let encrypted_verifier_value = aes_cbc(
        &derived_key(&hash, &VERIFIER_VALUE_BLOCK, KEY_BYTES),
        &password_salt,
        &sha512(&[&verifier_input]),
        true,
    )?;
    let encrypted_key = aes_cbc(
        &derived_key(&hash, &KEY_VALUE_BLOCK, KEY_BYTES),
        &password_salt,
        &package_key,
        true,
    )?;

    let attributes = format!(
        r#"saltSize="{SALT}" blockSize="{BLOCK}" keyBits="{}" hashSize="{HASH}" cipherAlgorithm="AES" cipherChaining="ChainingModeCBC" hashAlgorithm="SHA512""#,
        KEY_BYTES * 8
    );
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<encryption xmlns="http://schemas.microsoft.com/office/2006/encryption" xmlns:p="http://schemas.microsoft.com/office/2006/keyEncryptor/password" xmlns:c="http://schemas.microsoft.com/office/2006/keyEncryptor/certificate"><keyData {attributes} saltValue="{}"/><dataIntegrity encryptedHmacKey="{}" encryptedHmacValue="{}"/><keyEncryptors><keyEncryptor uri="http://schemas.microsoft.com/office/2006/keyEncryptor/password"><p:encryptedKey spinCount="{SPIN_COUNT}" {attributes} saltValue="{}" encryptedVerifierHashInput="{}" encryptedVerifierHashValue="{}" encryptedKeyValue="{}"/></keyEncryptor></keyEncryptors></encryption>"#,
        b64(&key_data_salt),
        b64(&encrypted_hmac_key),
        b64(&encrypted_hmac_value),
        b64(&password_salt),
        b64(&encrypted_verifier_input),
        b64(&encrypted_verifier_value),
        b64(&encrypted_key),
    );
    let mut info = vec![4, 0, 4, 0, 0x40, 0, 0, 0];
    info.extend(xml.as_bytes());

    let data_spaces = data_spaces();
    let mut streams: Vec<(&str, &[u8])> = data_spaces
        .iter()
        .map(|(path, data)| (*path, data.as_slice()))
        .collect();
    streams.push(("/EncryptionInfo", &info));
    streams.push(("/EncryptedPackage", &package));
    crate::compound_file::write(&streams).map_err(DecryptError::Malformed)
}

/// What `EncryptionInfo`'s descriptor says.
struct Descriptor {
    key_data_salt: Vec<u8>,
    key_bits: usize,
    spin_count: u32,
    password_salt: Vec<u8>,
    verifier_input: Vec<u8>,
    verifier_value: Vec<u8>,
    encrypted_key: Vec<u8>,
    hmac_key: Option<Vec<u8>>,
    hmac_value: Option<Vec<u8>>,
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

/// The first start tag named `name` (with any prefix).
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = xml;
    while let Some(at) = rest.find('<') {
        rest = &rest[at + 1..];
        let end = rest.find('>')?;
        let tag = &rest[..end];
        let tag_name = tag.split([' ', '/', '>']).next().unwrap_or_default();
        if tag_name.rsplit(':').next() == Some(name) {
            return Some(tag);
        }
    }
    None
}

fn parse_descriptor(info: &[u8]) -> Result<Descriptor, DecryptError> {
    if info.len() < 8 {
        return Err(DecryptError::Malformed(
            "EncryptionInfo is too short".into(),
        ));
    }
    let (major, minor) = (
        u16::from_le_bytes([info[0], info[1]]),
        u16::from_le_bytes([info[2], info[3]]),
    );
    if (major, minor) != (4, 4) {
        return Err(DecryptError::Unsupported(format!(
            "encryption version {major}.{minor} (only Agile, 4.4)"
        )));
    }
    let xml = String::from_utf8_lossy(&info[8..]);
    let key_data = tag(&xml, "keyData").ok_or(DecryptError::Malformed("no keyData".into()))?;
    let encrypted_key =
        tag(&xml, "encryptedKey").ok_or(DecryptError::Unsupported("no password key".into()))?;
    for (tag, what) in [(key_data, "keyData"), (encrypted_key, "encryptedKey")] {
        let cipher = attribute(tag, "cipherAlgorithm").unwrap_or_default();
        let hash = attribute(tag, "hashAlgorithm").unwrap_or_default();
        let chaining = attribute(tag, "cipherChaining").unwrap_or_default();
        if cipher != "AES" || hash != "SHA512" || chaining != "ChainingModeCBC" {
            return Err(DecryptError::Unsupported(format!(
                "{what}: {cipher} {hash} {chaining}"
            )));
        }
    }
    let needed = |tag: &str, name: &str| -> Result<Vec<u8>, DecryptError> {
        unb64(attribute(tag, name).ok_or_else(|| DecryptError::Malformed(format!("no {name}")))?)
    };
    let integrity = tag(&xml, "dataIntegrity");
    Ok(Descriptor {
        key_data_salt: needed(key_data, "saltValue")?,
        key_bits: attribute(encrypted_key, "keyBits")
            .and_then(|bits| bits.parse().ok())
            .unwrap_or(256),
        spin_count: attribute(encrypted_key, "spinCount")
            .and_then(|count| count.parse().ok())
            .unwrap_or(SPIN_COUNT),
        password_salt: needed(encrypted_key, "saltValue")?,
        verifier_input: needed(encrypted_key, "encryptedVerifierHashInput")?,
        verifier_value: needed(encrypted_key, "encryptedVerifierHashValue")?,
        encrypted_key: needed(encrypted_key, "encryptedKeyValue")?,
        hmac_key: integrity.and_then(|tag| attribute(tag, "encryptedHmacKey").map(unb64)?.ok()),
        hmac_value: integrity.and_then(|tag| attribute(tag, "encryptedHmacValue").map(unb64)?.ok()),
    })
}

fn read_stream(
    file: &mut cfb::CompoundFile<Cursor<&[u8]>>,
    path: &str,
) -> Result<Vec<u8>, DecryptError> {
    let mut stream = file
        .open_stream(path)
        .map_err(|error| DecryptError::Malformed(format!("{path}: {error}")))?;
    let mut data = Vec::new();
    stream
        .read_to_end(&mut data)
        .map_err(|error| DecryptError::Malformed(format!("{path}: {error}")))?;
    Ok(data)
}

/// Decrypts a package Word encrypted with a password to open, giving back
/// the zip. A wrong password is told apart from a damaged file.
pub fn decrypt_package(bytes: &[u8], password: &str) -> Result<Vec<u8>, DecryptError> {
    let mut file = cfb::CompoundFile::open(Cursor::new(bytes))
        .map_err(|error| DecryptError::Malformed(error.to_string()))?;
    let info = read_stream(&mut file, "/EncryptionInfo")?;
    let package = read_stream(&mut file, "/EncryptedPackage")?;
    let descriptor = parse_descriptor(&info)?;
    let key_bytes = descriptor.key_bits / 8;

    let hash = password_hash(password, &descriptor.password_salt, descriptor.spin_count);
    let verifier_input = aes_cbc(
        &derived_key(&hash, &VERIFIER_INPUT_BLOCK, key_bytes),
        &descriptor.password_salt,
        &descriptor.verifier_input,
        false,
    )?;
    let verifier_value = aes_cbc(
        &derived_key(&hash, &VERIFIER_VALUE_BLOCK, key_bytes),
        &descriptor.password_salt,
        &descriptor.verifier_value,
        false,
    )?;
    let expected = sha512(&[&verifier_input[..SALT.min(verifier_input.len())]]);
    if verifier_value.len() < HASH || verifier_value[..HASH] != expected {
        return Err(DecryptError::WrongPassword);
    }
    let package_key = aes_cbc(
        &derived_key(&hash, &KEY_VALUE_BLOCK, key_bytes),
        &descriptor.password_salt,
        &descriptor.encrypted_key,
        false,
    )?;
    let package_key = &package_key[..key_bytes.min(package_key.len())];

    // The integrity check, when the package carries one.
    if let (Some(hmac_key), Some(hmac_value)) = (&descriptor.hmac_key, &descriptor.hmac_value) {
        let key = aes_cbc(
            package_key,
            &derived_iv(&descriptor.key_data_salt, &HMAC_KEY_BLOCK),
            hmac_key,
            false,
        )?;
        let value = aes_cbc(
            package_key,
            &derived_iv(&descriptor.key_data_salt, &HMAC_VALUE_BLOCK),
            hmac_value,
            false,
        )?;
        if key.len() >= HASH
            && value.len() >= HASH
            && hmac_sha512(&key[..HASH], &package) != value[..HASH]
        {
            return Err(DecryptError::Malformed("the integrity check failed".into()));
        }
    }

    if package.len() < 8 {
        return Err(DecryptError::Malformed(
            "EncryptedPackage is too short".into(),
        ));
    }
    let size = u64::from_le_bytes(package[..8].try_into().expect("eight bytes")) as usize;
    let mut zip = crypt_package(package_key, &descriptor.key_data_salt, &package[8..], false)?;
    if size > zip.len() {
        return Err(DecryptError::Malformed(
            "EncryptedPackage is short of its size".into(),
        ));
    }
    zip.truncate(size);
    Ok(zip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn counter() -> impl FnMut(&mut [u8]) {
        let mut next = 7u8;
        move |buffer: &mut [u8]| {
            for byte in buffer {
                next = next.wrapping_mul(31).wrapping_add(17);
                *byte = next;
            }
        }
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // RFC 4231 test case 2.
        let mac = hmac_sha512(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"
        );
    }

    #[test]
    fn an_encrypted_package_opens_with_its_password_only() {
        let zip: Vec<u8> = (0..10_000u32).map(|n| (n % 251) as u8).collect();
        let encrypted = encrypt_package(&zip, "秘密 pass", &mut counter()).unwrap();
        assert!(is_encrypted_package(&encrypted));
        assert!(!is_encrypted_package(&zip));
        assert_eq!(decrypt_package(&encrypted, "秘密 pass").unwrap(), zip);
        assert_eq!(
            decrypt_package(&encrypted, "wrong"),
            Err(DecryptError::WrongPassword)
        );
        // The streams Word looks for are there.
        let mut file = cfb::CompoundFile::open(Cursor::new(&encrypted[..])).unwrap();
        for path in [
            "/EncryptionInfo",
            "/EncryptedPackage",
            "/\u{6}DataSpaces/DataSpaceMap",
            "/\u{6}DataSpaces/TransformInfo/StrongEncryptionTransform/\u{6}Primary",
        ] {
            assert!(file.is_stream(path), "{path}");
        }
        let primary = read_stream(
            &mut file,
            "/\u{6}DataSpaces/TransformInfo/StrongEncryptionTransform/\u{6}Primary",
        )
        .unwrap();
        // TransformLength 0x58 and the 0x4C-byte transform id, as Word writes.
        assert_eq!(&primary[..12], &[0x58, 0, 0, 0, 1, 0, 0, 0, 0x4c, 0, 0, 0]);
    }

    #[test]
    fn a_changed_byte_fails_the_integrity_check() {
        let zip = vec![42u8; 5000];
        let encrypted = encrypt_package(&zip, "pw", &mut counter()).unwrap();
        let mut file = cfb::CompoundFile::open(Cursor::new(encrypted.clone())).unwrap();
        let mut package = Vec::new();
        file.open_stream("/EncryptedPackage")
            .unwrap()
            .read_to_end(&mut package)
            .unwrap();
        let mut tampered = cfb::CompoundFile::open(Cursor::new(encrypted)).unwrap();
        package[100] ^= 1;
        tampered
            .create_stream("/EncryptedPackage")
            .unwrap()
            .write_all(&package)
            .unwrap();
        tampered.flush().unwrap();
        let bytes = tampered.into_inner().into_inner();
        assert!(matches!(
            decrypt_package(&bytes, "pw"),
            Err(DecryptError::Malformed(_))
        ));
    }
}
