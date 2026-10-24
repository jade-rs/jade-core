//! OOXML editing-protection verifiers. These are separate from package encryption.
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};
use thiserror::Error;

pub const DEFAULT_SPIN_COUNT: u32 = 100_000;
pub const MAX_SPIN_COUNT: u32 = 10_000_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PasswordError {
    #[error("password exceeds 255 UTF-16 units")]
    TooLong,
    #[error("unsupported protection hash algorithm")]
    UnsupportedAlgorithm,
    #[error("incomplete or malformed protection password metadata")]
    Malformed,
}

/// Raw attributes remain distinguishable from absent password metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PasswordVerifier {
    pub legacy: Option<String>,
    pub algorithm: Option<String>,
    pub hash: Option<String>,
    pub salt: Option<String>,
    pub spin_count: Option<String>,
}
impl PasswordVerifier {
    pub fn is_present(&self) -> bool {
        self.legacy.is_some() || self.modern_present()
    }
    fn modern_present(&self) -> bool {
        self.algorithm.is_some()
            || self.hash.is_some()
            || self.salt.is_some()
            || self.spin_count.is_some()
    }
    /// Check credential metadata without iterating the password hash. Dialogs
    /// use this before asking for a password the verifier cannot accept.
    pub fn validate(&self) -> Result<(), PasswordError> {
        if self.modern_present() {
            let salt = decode(
                self.salt.as_deref().ok_or(PasswordError::Malformed)?,
                65_536,
            )?;
            let expected = decode(self.hash.as_deref().ok_or(PasswordError::Malformed)?, 64)?;
            let spin: u32 = self
                .spin_count
                .as_deref()
                .ok_or(PasswordError::Malformed)?
                .trim_matches([' ', '\t', '\r', '\n'])
                .parse()
                .map_err(|_| PasswordError::Malformed)?;
            if spin > MAX_SPIN_COUNT {
                return Err(PasswordError::Malformed);
            }
            let digest = iso_hash(
                "",
                &salt,
                0,
                self.algorithm.as_deref().ok_or(PasswordError::Malformed)?,
            )?;
            if expected.len() != digest.len() {
                return Err(PasswordError::Malformed);
            }
        } else if let Some(value) = &self.legacy {
            let value = value.trim_matches([' ', '\t', '\r', '\n']);
            if value.len() != 4 || u16::from_str_radix(value, 16).is_err() {
                return Err(PasswordError::Malformed);
            }
        }
        Ok(())
    }
    /// The caller supplies a fresh 16-byte salt from its platform CSPRNG.
    pub fn new(password: &str, salt: &[u8; 16]) -> Result<Self, PasswordError> {
        check_password(password)?;
        Ok(Self {
            algorithm: Some("SHA-512".into()),
            hash: Some(STANDARD.encode(iso_hash(password, salt, DEFAULT_SPIN_COUNT, "SHA-512")?)),
            salt: Some(STANDARD.encode(salt)),
            spin_count: Some(DEFAULT_SPIN_COUNT.to_string()),
            legacy: None,
        })
    }
    pub fn verifies(&self, password: &str) -> Result<bool, PasswordError> {
        check_password(password)?;
        if self.modern_present() {
            let algorithm = self.algorithm.as_deref().ok_or(PasswordError::Malformed)?;
            let salt = decode(
                self.salt.as_deref().ok_or(PasswordError::Malformed)?,
                65_536,
            )?;
            let expected = decode(self.hash.as_deref().ok_or(PasswordError::Malformed)?, 64)?;
            let spin = self
                .spin_count
                .as_deref()
                .ok_or(PasswordError::Malformed)?
                .trim_matches([' ', '\t', '\r', '\n'])
                .parse()
                .map_err(|_| PasswordError::Malformed)?;
            let actual = iso_hash(password, &salt, spin, algorithm)?;
            if expected.len() != actual.len() {
                return Err(PasswordError::Malformed);
            }
            return Ok(equal(&expected, &actual));
        }
        if let Some(text) = &self.legacy {
            let text = text.trim_matches([' ', '\t', '\r', '\n']);
            if text.len() != 4 {
                return Err(PasswordError::Malformed);
            }
            let expected = u16::from_str_radix(text, 16).map_err(|_| PasswordError::Malformed)?;
            // OOXML does not record the originating ANSI code page. Try its
            // listed Windows pages, plus the low/high-byte convention used by
            // existing OOXML consumers. Modern verifiers never fall back here.
            return Ok(legacy_candidates(password).contains(&expected));
        }
        Ok(true)
    }
    /// Range ISO metadata can hash either the Unicode password or one UTF-16
    /// unit containing its legacy verifier (MS-XLSX 2.6.56). Both paths must
    /// match the modern hash; a legacy attribute never substitutes for it.
    pub fn verifies_range(&self, password: &str) -> Result<bool, PasswordError> {
        let direct = self.verifies(password)?;
        if direct || !self.modern_present() {
            return Ok(direct);
        }
        let algorithm = self.algorithm.as_deref().ok_or(PasswordError::Malformed)?;
        let salt = decode(
            self.salt.as_deref().ok_or(PasswordError::Malformed)?,
            65_536,
        )?;
        let expected = decode(self.hash.as_deref().ok_or(PasswordError::Malformed)?, 64)?;
        let spin = self
            .spin_count
            .as_deref()
            .ok_or(PasswordError::Malformed)?
            .trim_matches([' ', '\t', '\r', '\n'])
            .parse()
            .map_err(|_| PasswordError::Malformed)?;
        for verifier in legacy_candidates(password) {
            if equal(
                &expected,
                &iso_hash_units(&[verifier], &salt, spin, algorithm)?,
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
fn legacy_candidates(password: &str) -> Vec<u16> {
    let mut values = Vec::new();
    let pages = [
        encoding_rs::WINDOWS_1252,
        encoding_rs::WINDOWS_1250,
        encoding_rs::WINDOWS_1251,
        encoding_rs::WINDOWS_1253,
        encoding_rs::WINDOWS_1254,
        encoding_rs::WINDOWS_1255,
        encoding_rs::WINDOWS_1256,
        encoding_rs::WINDOWS_1257,
        encoding_rs::WINDOWS_1258,
        encoding_rs::WINDOWS_874,
        encoding_rs::SHIFT_JIS,
        encoding_rs::GBK,
        encoding_rs::EUC_KR,
        encoding_rs::BIG5,
    ];
    for page in pages {
        let mut bytes = Vec::new();
        for c in password.chars() {
            let mut buf = [0; 4];
            let (encoded, _, errors) = page.encode(c.encode_utf8(&mut buf));
            if errors {
                bytes.push(b'?');
            } else {
                bytes.extend_from_slice(&encoded);
            }
        }
        let value = legacy_verifier_bytes(&bytes);
        if !values.contains(&value) {
            values.push(value);
        }
    }
    let bytes: Vec<u8> = password
        .encode_utf16()
        .map(|c| {
            if c as u8 == 0 {
                (c >> 8) as u8
            } else {
                c as u8
            }
        })
        .collect();
    let value = legacy_verifier_bytes(&bytes);
    if !values.contains(&value) {
        values.push(value);
    }
    values
}
fn check_password(password: &str) -> Result<(), PasswordError> {
    if password.encode_utf16().count() > 255 {
        return Err(PasswordError::TooLong);
    }
    Ok(())
}
fn decode(text: &str, maximum: usize) -> Result<Vec<u8>, PasswordError> {
    // Bound the actual base64 payload, preserving XML Schema whitespace folding.
    let mut compact = String::new();
    let limit = maximum.div_ceil(3) * 4;
    for c in text
        .chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
    {
        if compact.len() + c.len_utf8() > limit {
            return Err(PasswordError::Malformed);
        }
        compact.push(c);
    }
    let bytes = STANDARD
        .decode(compact)
        .map_err(|_| PasswordError::Malformed)?;
    if bytes.len() > maximum {
        return Err(PasswordError::Malformed);
    }
    Ok(bytes)
}
fn equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}
fn digest(algorithm: &str, data: &[u8]) -> Result<Vec<u8>, PasswordError> {
    Ok(match algorithm {
        "SHA-1" => sha1::Sha1::digest(data).to_vec(),
        "SHA-224" => Sha224::digest(data).to_vec(),
        "SHA-256" => Sha256::digest(data).to_vec(),
        "SHA-384" => Sha384::digest(data).to_vec(),
        "SHA-512" => Sha512::digest(data).to_vec(),
        _ => return Err(PasswordError::UnsupportedAlgorithm),
    })
}
/// ISO write protection: UTF-16LE; initial salt+password, then previous hash+u32LE counter.
pub fn iso_hash(
    password: &str,
    salt: &[u8],
    spin_count: u32,
    algorithm: &str,
) -> Result<Vec<u8>, PasswordError> {
    check_password(password)?;
    iso_hash_units(
        &password.encode_utf16().collect::<Vec<_>>(),
        salt,
        spin_count,
        algorithm,
    )
}
fn iso_hash_units(
    password: &[u16],
    salt: &[u8],
    spin_count: u32,
    algorithm: &str,
) -> Result<Vec<u8>, PasswordError> {
    if password.len() > 255 {
        return Err(PasswordError::TooLong);
    }
    if spin_count > MAX_SPIN_COUNT || salt.len() > 65_536 {
        return Err(PasswordError::Malformed);
    }
    let mut input = salt.to_vec();
    let units = if password.first() == Some(&0xfeff) {
        &password[1..]
    } else {
        password
    };
    for unit in units {
        input.extend_from_slice(&unit.to_le_bytes());
    }
    let mut hash = digest(algorithm, &input)?;
    for i in 0..spin_count {
        hash.extend_from_slice(&i.to_le_bytes());
        hash = digest(algorithm, &hash)?;
    }
    Ok(hash)
}
/// Legacy SpreadsheetML verifier after ANSI conversion; empty passwords use zero.
pub fn legacy_verifier_bytes(password: &[u8]) -> u16 {
    if password.is_empty() {
        return 0;
    }
    let mut hash = 0u16;
    for byte in password
        .iter()
        .rev()
        .copied()
        .chain(std::iter::once(password.len() as u8))
    {
        hash = ((hash << 1) & 0x7fff) | ((hash >> 14) & 1);
        hash ^= u16::from(byte);
    }
    hash ^ 0xce4b
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iso_vectors_match_independent_hashlib_utf16_and_appended_counter_results() {
        let salt: Vec<u8> = (0..16).collect();
        for (algorithm, expected) in [
            ("SHA-1", "i666Ir3ihywBSiYobXswW9qak9Y="),
            ("SHA-224", "TxkRcSKLEzaGJBkc2IWBz3gLkB+6wVy7jI6mbA=="),
            ("SHA-256", "06re8Tqktrf4w+0PNGtDcBzfZLISFoXXF6Ci2Yxxf30="),
            ("SHA-384", "Hu616QCgfZfIgtLfDQrSs3pDYSkv52KXHHbXhcLrcxekHxzIkrkvOu1XzdKT+Awm"),
            ("SHA-512", "960IHvAzAnjzVPmGE9He4UCC8DPYQ2ETEJFEx4w5xv+Y6ozKCVrPwKLG7EkBlOpgVhPoU86t/Ijf67rbrRrwLw=="),
        ] {
            let hash = iso_hash("密碼🔒", &salt, 3, algorithm).unwrap();
            assert_eq!(STANDARD.encode(&hash), expected, "{algorithm}");
            let verifier = PasswordVerifier {algorithm: Some(algorithm.into()), hash: Some(expected.into()), salt: Some(STANDARD.encode(&salt)), spin_count: Some("3".into()), legacy: None};
            assert!(verifier.verifies("密碼🔒").unwrap());
            assert!(!verifier.verifies("密碼").unwrap());
        }
    }
    #[test]
    fn modern_roundtrip_clears_no_legacy_and_uses_fresh_supplied_salt() {
        let a = PasswordVerifier::new("secret", &[1; 16]).unwrap();
        let b = PasswordVerifier::new("secret", &[2; 16]).unwrap();
        assert_ne!(a.hash, b.hash);
        assert!(a.legacy.is_none());
        assert!(a.verifies("secret").unwrap());
        assert!(!a.verifies("wrong").unwrap());
        assert_eq!(
            iso_hash("\u{feff}secret", &[], 0, "SHA-512").unwrap(),
            iso_hash("secret", &[], 0, "SHA-512").unwrap()
        );
        assert_ne!(
            iso_hash(" secret ", &[], 0, "SHA-512").unwrap(),
            iso_hash("secret", &[], 0, "SHA-512").unwrap()
        );
        let mut wrapped = a.clone();
        wrapped.hash = wrapped
            .hash
            .map(|h| format!("{}{}\r\n", " ".repeat(1_000), h));
        assert!(wrapped.verifies("secret").unwrap());
    }
    #[test]
    fn incomplete_modern_does_not_fall_back_to_a_matching_legacy_verifier() {
        let legacy = Some("83AF".into());
        let partial = PasswordVerifier {
            legacy: legacy.clone(),
            hash: Some("AA==".into()),
            ..Default::default()
        };
        assert_eq!(partial.verifies("password"), Err(PasswordError::Malformed));
        let unsupported = PasswordVerifier {
            legacy,
            algorithm: Some("unrecognized".into()),
            hash: Some("AA==".into()),
            salt: Some("".into()),
            spin_count: Some("0".into()),
        };
        assert_eq!(
            unsupported.verifies("password"),
            Err(PasswordError::UnsupportedAlgorithm)
        );
    }
    #[test]
    fn imported_limits_bad_hex_and_bad_base64_do_not_unlock() {
        for text in ["", "GGGG", "12345"] {
            assert!(PasswordVerifier {
                legacy: Some(text.into()),
                ..Default::default()
            }
            .verifies("x")
            .is_err());
        }
        assert_eq!(
            iso_hash("x", &[], MAX_SPIN_COUNT + 1, "SHA-512"),
            Err(PasswordError::Malformed)
        );
        assert_eq!(
            iso_hash("x", &vec![0; 65_537], 0, "SHA-512"),
            Err(PasswordError::Malformed)
        );
        assert!(iso_hash("x", &vec![0; 65_536], 0, "SHA-512").is_ok());
        assert!(PasswordVerifier::new(&"x".repeat(256), &[0; 16]).is_err());
        let mut v = PasswordVerifier::new("x", &[0; 16]).unwrap();
        v.salt = Some("invalid!".into());
        assert_eq!(v.verifies("x"), Err(PasswordError::Malformed));
    }
    #[test]
    fn legacy_ascii_and_multiple_originating_windows_codepages_match() {
        assert_eq!(legacy_verifier_bytes(b"password"), 0x83af);
        assert_eq!(legacy_verifier_bytes(b"test"), 0xcbeb);
        assert!(PasswordVerifier::default().verifies("").unwrap());
        assert!(PasswordVerifier {
            legacy: Some("0000".into()),
            ..Default::default()
        }
        .verifies("")
        .unwrap());
        for (password, expected) in [
            ("password", "83AF"),
            ("日本", "CF81"),
            ("Привет", "E553"),
            ("€été", "C1DB"),
        ] {
            let v = PasswordVerifier {
                legacy: Some(expected.into()),
                ..Default::default()
            };
            assert!(v.verifies(password).unwrap(), "{password}");
            assert!(!v.verifies("wrong").unwrap());
        }
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;
    #[test]
    fn imported_range_iso_over_legacy_verifier_matches_independent_python_vectors() {
        for (password, hash) in [
            ("password", "jA9zZOAXfLvFkJjipbgp+ZuI0KvdLnOkIcryShL5XGU="),
            ("日本", "hl14X13P1AHattCvyD1kp0w2/czLe/y8xLPp5yE2jbI="),
            (
                "qjarFaYLlNPH",
                "DxHI1k1d04u70YqTu6Oe/j/bGvvGJkXRXmG+gN2DaVU=",
            ),
        ] {
            let verifier = PasswordVerifier {
                legacy: None,
                algorithm: Some("SHA-256".into()),
                hash: Some(hash.into()),
                salt: Some("AAECAwQFBgcICQoLDA0ODw==".into()),
                spin_count: Some("3".into()),
            };
            assert!(!verifier.verifies(password).unwrap());
            assert!(verifier.verifies_range(password).unwrap(), "{password}");
            assert!(!verifier.verifies_range("wrong").unwrap());
        }
    }
    #[test]
    fn range_import_alternatives_still_require_a_complete_supported_modern_hash() {
        let mut verifier = PasswordVerifier::new("secret", &[3; 16]).unwrap();
        verifier.legacy = Some("83AF".into());
        assert!(verifier.verifies_range("secret").unwrap());
        assert!(!verifier.verifies_range("password").unwrap());
        verifier.algorithm = Some("unknown".into());
        assert_eq!(
            verifier.verifies_range("password"),
            Err(PasswordError::UnsupportedAlgorithm)
        );
        let partial = PasswordVerifier {
            legacy: Some("83AF".into()),
            hash: Some("AA==".into()),
            ..Default::default()
        };
        assert_eq!(
            partial.verifies_range("password"),
            Err(PasswordError::Malformed)
        );
    }
}

#[cfg(test)]
mod metadata_validation_tests {
    use super::*;
    #[test]
    fn metadata_query_checks_every_credential_without_running_spin_iterations() {
        assert!(PasswordVerifier::default().validate().is_ok());
        let legacy = PasswordVerifier {
            legacy: Some(" 83AF\n".into()),
            ..Default::default()
        };
        assert!(legacy.validate().is_ok());
        assert!(PasswordVerifier {
            legacy: Some("xyz".into()),
            ..Default::default()
        }
        .validate()
        .is_err());
        for algorithm in ["SHA-1", "SHA-224", "SHA-256", "SHA-384", "SHA-512"] {
            let hash = iso_hash("password", &[3; 16], 0, algorithm).unwrap();
            let good = PasswordVerifier {
                algorithm: Some(algorithm.into()),
                salt: Some(STANDARD.encode([3; 16])),
                spin_count: Some(MAX_SPIN_COUNT.to_string()),
                hash: Some(STANDARD.encode(hash)),
                legacy: Some("invalid but non-authoritative".into()),
            };
            assert!(good.validate().is_ok());
            for invalid in [
                PasswordVerifier {
                    spin_count: Some((MAX_SPIN_COUNT + 1).to_string()),
                    ..good.clone()
                },
                PasswordVerifier {
                    hash: Some("AA==".into()),
                    ..good.clone()
                },
                PasswordVerifier {
                    salt: Some("!".into()),
                    ..good.clone()
                },
                PasswordVerifier {
                    algorithm: Some("MD5".into()),
                    ..good.clone()
                },
                PasswordVerifier {
                    spin_count: None,
                    ..good.clone()
                },
            ] {
                assert!(invalid.validate().is_err());
            }
        }
    }
}
