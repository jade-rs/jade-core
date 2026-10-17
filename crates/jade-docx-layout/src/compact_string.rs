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
            buf[..s.len()].copy_from_slice(s.as_bytes());
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
