/// A small-string-optimised container for grapheme clusters.
///
/// Most grapheme clusters fit within 16 bytes (ASCII chars, CJK codepoints,
/// emoji up to 4-byte UTF-8 sequences, etc.).  Storing them inline avoids
/// a heap allocation per cluster, which the profiler identified as a major
/// source of allocator pressure.
#[derive(Clone, Debug, Eq)]
pub enum CompactString {
    /// The grapheme fits in ≤ 16 bytes.  `len` stores the actual byte count.
    Inline { buf: [u8; 16], len: u8 },
    /// Fallback for rare graphemes longer than 16 bytes.
    Heap(String),
}

impl Default for CompactString {
    fn default() -> Self {
        CompactString::Inline {
            buf: [0u8; 16],
            len: 0,
        }
    }
}

impl CompactString {
    const INLINE_CAP: usize = 16;

    #[inline]
    pub fn new(s: &str) -> Self {
        if s.len() <= Self::INLINE_CAP {
            let mut buf = [0u8; 16];
            // Fixed-size copies avoid a memcpy call for typical graphemes.
            match s.len() {
                0 => {}
                1 => buf[0] = s.as_bytes()[0],
                2 => buf[..2].copy_from_slice(s.as_bytes()),
                3 => buf[..3].copy_from_slice(s.as_bytes()),
                4 => buf[..4].copy_from_slice(s.as_bytes()),
                _ => buf[..s.len()].copy_from_slice(s.as_bytes()),
            }
            CompactString::Inline {
                buf,
                len: s.len() as u8,
            }
        } else {
            CompactString::Heap(s.to_owned())
        }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            CompactString::Inline { buf, len } => {
                // SAFETY: The bytes were copied from a valid &str in `new`.
                unsafe { std::str::from_utf8_unchecked(&buf[..*len as usize]) }
            }
            CompactString::Heap(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for CompactString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq for CompactString {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<&str> for CompactString {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for CompactString {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl AsRef<str> for CompactString {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::ops::Deref for CompactString {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for CompactString {
    #[inline]
    fn from(s: &str) -> Self {
        CompactString::new(s)
    }
}

impl From<String> for CompactString {
    #[inline]
    fn from(s: String) -> Self {
        if s.len() <= Self::INLINE_CAP {
            CompactString::new(&s)
        } else {
            CompactString::Heap(s)
        }
    }
}

impl serde::Serialize for CompactString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for CompactString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(CompactString::from(s))
    }
}

#[cfg(test)]
mod tests {
    use super::CompactString;
    use proptest::prelude::*;

    // Keep the pre-optimization constructor as the independent representation
    // oracle, including all zero-filled bytes exposed by the public variant.
    fn original(text: &str) -> CompactString {
        if text.len() <= 16 {
            let mut buf = [0u8; 16];
            buf[..text.len()].copy_from_slice(text.as_bytes());
            CompactString::Inline {
                buf,
                len: text.len() as u8,
            }
        } else {
            CompactString::Heap(text.to_owned())
        }
    }

    fn assert_same_representation(actual: &CompactString, expected: &CompactString) {
        match (actual, expected) {
            (
                CompactString::Inline { buf, len },
                CompactString::Inline {
                    buf: expected_buf,
                    len: expected_len,
                },
            ) => {
                assert_eq!(buf, expected_buf);
                assert_eq!(len, expected_len);
            }
            (CompactString::Heap(text), CompactString::Heap(expected)) => {
                assert_eq!(text, expected);
            }
            _ => panic!("representation differs: {actual:?} versus {expected:?}"),
        }
    }

    fn assert_matches_original(text: &str) {
        let expected = original(text);
        for actual in [
            CompactString::new(text),
            CompactString::from(text),
            CompactString::from(text.to_owned()),
        ] {
            assert_same_representation(&actual, &expected);
            assert_eq!(actual.as_str(), text);
            assert_same_representation(&actual.clone(), &expected);
        }
    }

    #[test]
    fn constructors_preserve_every_unicode_scalar_and_all_inline_bytes() {
        let mut bytes = [0u8; 4];
        for scalar in (0..=0x10ffff).filter_map(char::from_u32) {
            assert_matches_original(scalar.encode_utf8(&mut bytes));
        }
    }

    #[test]
    fn constructors_preserve_boundaries_graphemes_and_unaligned_inputs() {
        assert_same_representation(&CompactString::default(), &original(""));
        let mut texts: Vec<String> = (0..=33).map(|len| "a".repeat(len)).collect();
        for unit in [
            "\0",
            "é",
            "日",
            "𠀀",
            "e\u{301}",
            "\u{3000}",
            "\r\n",
            "👩\u{200d}💻",
            "🇯🇵",
            "aé日𠀀",
            "\u{301}",
            "\"<&\t",
        ] {
            texts.extend((0..=24).map(|len| unit.repeat(len)));
        }
        for text in texts {
            for offset in 0..16 {
                let backing = format!("{}{text}", "_".repeat(offset));
                let slice = &backing[offset..];
                assert_matches_original(slice);
                let encoded = serde_json::to_string(&CompactString::new(slice)).unwrap();
                assert_eq!(encoded, serde_json::to_string(slice).unwrap());
                let decoded: CompactString = serde_json::from_str(&encoded).unwrap();
                assert_same_representation(&decoded, &original(slice));
            }
        }
    }

    #[test]
    fn owned_heap_construction_keeps_the_existing_allocation() {
        for text in ["a".repeat(17), "日".repeat(6), "👩\u{200d}💻".repeat(3)] {
            let pointer = text.as_ptr();
            let expected = text.clone();
            let actual = CompactString::from(text);
            assert!(matches!(actual, CompactString::Heap(_)));
            assert_eq!(actual.as_str(), expected);
            assert_eq!(actual.as_str().as_ptr(), pointer);
        }
    }

    proptest! {
        #[test]
        fn constructors_preserve_arbitrary_strings(text in any::<String>()) {
            assert_matches_original(&text);
        }
    }
}
