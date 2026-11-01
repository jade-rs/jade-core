//! Large parts inflated (or written) lately, kept with their compressed
//! bytes so the same entry is not inflated again: an editor writes its
//! package on every edit and opens what it wrote for the next one, which
//! inflated every large part again each time.
//!
//! An entry is taken from here only when its compressed bytes are exactly
//! those kept and decoding metadata matches. Encrypted entries are never
//! cached. A name, checksum, or compressed byte sequence alone is insufficient.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Entries this small are inflated as before: comparing them would cost
/// about what inflating does.
pub(crate) const MIN_COMPRESSED: u64 = 16 * 1024;
/// What the kept parts may add up to, compressed and inflated.
const BUDGET: usize = 64 * 1024 * 1024;

/// Metadata that controls decoding and integrity checks. Equal raw bytes can
/// decode differently under different methods; a hit must not hide a bad CRC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    compression: zip::CompressionMethod,
    flags: u16,
    crc32: u32,
    size: u64,
    pub(crate) compressed_size: u64,
}

impl Identity {
    pub(crate) fn from_entry<R: std::io::Read + ?Sized>(
        entry: &zip::read::ZipFile<'_, R>,
    ) -> Option<Self> {
        if entry.encrypted() {
            return None;
        }
        Some(Self {
            compression: entry.compression(),
            flags: entry.flags().as_u16(),
            crc32: entry.crc32(),
            size: entry.size(),
            compressed_size: entry.compressed_size(),
        })
    }
}

struct Kept {
    name: String,
    identity: Identity,
    compressed: Rc<[u8]>,
    contents: Rc<[u8]>,
}

thread_local! {
    static KEPT: RefCell<VecDeque<Kept>> = const { RefCell::new(VecDeque::new()) };
}

/// The inflated contents of `name` whose compressed bytes are `compressed`,
/// when kept.
pub(crate) fn recall(name: &str, identity: Identity, compressed: &[u8]) -> Option<Rc<[u8]>> {
    KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        let at = kept.iter().position(|part| {
            part.name == name && part.identity == identity && *part.compressed == *compressed
        })?;
        // Most recently used first.
        let part = kept.remove(at)?;
        let contents = part.contents.clone();
        kept.push_front(part);
        Some(contents)
    })
}

/// Keeps `contents` as what `compressed` inflates to, for `name`.
pub(crate) fn keep(name: &str, identity: Identity, compressed: Vec<u8>, contents: &[u8]) {
    // A bounded raw read must not turn a prefix into a complete cache key.
    if compressed.len() as u64 != identity.compressed_size || contents.len() as u64 != identity.size
    {
        return;
    }
    let size = compressed.len() + contents.len();
    if size > BUDGET / 4 {
        return;
    }
    KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        kept.retain(|part| {
            !(part.name == name && part.identity == identity && *part.compressed == *compressed)
        });
        kept.push_front(Kept {
            name: name.to_string(),
            identity,
            compressed: compressed.into(),
            contents: contents.into(),
        });
        let mut total = 0;
        kept.retain(|part| {
            total += part.compressed.len() + part.contents.len();
            total <= BUDGET
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_same_compressed_bytes_bring_the_contents_back() {
        let identity = Identity {
            compression: zip::CompressionMethod::Deflated,
            flags: 0,
            crc32: 0,
            size: 8,
            compressed_size: 3,
        };
        keep("xl/a.xml", identity, vec![1, 2, 3], b"inflated");
        assert_eq!(
            recall("xl/a.xml", identity, &[1, 2, 3]).as_deref(),
            Some(&b"inflated"[..])
        );
        assert!(recall("xl/a.xml", identity, &[1, 2, 4]).is_none());
        assert!(recall("xl/b.xml", identity, &[1, 2, 3]).is_none());
    }

    #[test]
    fn incomplete_raw_or_decoded_reads_are_not_kept() {
        let identity = Identity {
            compression: zip::CompressionMethod::Deflated,
            flags: 0,
            crc32: 0,
            size: 8,
            compressed_size: 3,
        };
        keep("xl/partial-raw.xml", identity, vec![1, 2], b"inflated");
        assert!(recall("xl/partial-raw.xml", identity, &[1, 2]).is_none());
        keep(
            "xl/partial-decoded.xml",
            identity,
            vec![1, 2, 3],
            b"inflate",
        );
        assert!(recall("xl/partial-decoded.xml", identity, &[1, 2, 3]).is_none());
    }
}
