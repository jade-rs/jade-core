use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Cursor;
use std::ops::Range;
use std::sync::{Arc, Weak};

/// Exact ZIP bytes, sharing unchanged compressed payloads and metadata.
#[derive(Clone)]
pub struct ArchiveSnapshot {
    chunks: Vec<Arc<Vec<u8>>>,
    len: usize,
}
impl ArchiveSnapshot {
    pub fn restore(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(self.len);
        for chunk in &self.chunks {
            result.extend_from_slice(chunk);
        }
        result
    }
}
impl PartialEq for ArchiveSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self.chunks.len() == other.chunks.len()
            && self
                .chunks
                .iter()
                .zip(&other.chunks)
                .all(|(a, b)| Arc::ptr_eq(a, b) || a == b)
    }
}
impl Eq for ArchiveSnapshot {}

/// A session-local weak interning pool. Dropping snapshots releases their data.
#[derive(Default)]
pub struct SnapshotStore {
    chunks: HashMap<[u8; 32], Vec<Weak<Vec<u8>>>>,
}
impl SnapshotStore {
    pub fn capture(&mut self, bytes: &[u8]) -> ArchiveSnapshot {
        self.chunks.retain(|_, candidates| {
            candidates.retain(|chunk| chunk.strong_count() > 0);
            !candidates.is_empty()
        });
        let ranges =
            payload_ranges(bytes).unwrap_or_else(|| std::iter::once(0..bytes.len()).collect());
        let mut chunks = Vec::with_capacity(ranges.len() * 2 + 1);
        let mut offset = 0;
        for range in ranges {
            if range.start > offset {
                chunks.push(self.intern(&bytes[offset..range.start]));
            }
            if !range.is_empty() {
                chunks.push(self.intern(&bytes[range.clone()]));
            }
            offset = range.end;
        }
        if offset < bytes.len() {
            chunks.push(self.intern(&bytes[offset..]));
        }
        ArchiveSnapshot {
            chunks,
            len: bytes.len(),
        }
    }
    fn intern(&mut self, bytes: &[u8]) -> Arc<Vec<u8>> {
        let key: [u8; 32] = Sha256::digest(bytes).into();
        let candidates = self.chunks.entry(key).or_default();
        for candidate in candidates.iter().filter_map(Weak::upgrade) {
            // Equality checks keep interning correct even under hash collisions.
            if candidate.as_slice() == bytes {
                return candidate;
            }
        }
        // Vec owns a separate buffer: a dead Weak retains no large byte payload.
        let chunk = Arc::new(bytes.to_vec());
        candidates.push(Arc::downgrade(&chunk));
        chunk
    }
    /// Live snapshot byte buffers only; excludes descriptors and hash metadata.
    pub fn retained_bytes(&self) -> usize {
        self.chunks
            .values()
            .flat_map(|v| v.iter())
            .filter_map(Weak::upgrade)
            .map(|c| c.len())
            .sum()
    }
}

fn payload_ranges(bytes: &[u8]) -> Option<Vec<Range<usize>>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut ranges = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i).ok()?;
        let start = usize::try_from(file.data_start()?).ok()?;
        let size = usize::try_from(file.compressed_size()).ok()?;
        let end = start.checked_add(size)?;
        if end > bytes.len() {
            return None;
        }
        if size > 0 {
            ranges.push(start..end);
        }
    }
    ranges.sort_by_key(|range| range.start);
    if ranges.windows(2).any(|pair| pair[0].end > pair[1].start) {
        return None;
    }
    Some(ranges)
}
