//! Reuse identical DirectWrite faces across document and font-family aliases.
use super::{sfnt_from_collection, with_font_mgr, Typeface};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FACES: usize = 64;

struct Entry {
    hash: u64,
    bytes: Box<[u8]>,
    index: usize,
    face: Typeface,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

impl Cache {
    fn face(&mut self, bytes: &[u8], index: usize) -> Option<Typeface> {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        index.hash(&mut hasher);
        let hash = hasher.finish();
        // Verify the full content too: a hash collision must never select a
        // different font. Family aliases deliberately do not enter this key.
        if let Some(position) = self.entries.iter().position(|entry| {
            entry.hash == hash && entry.index == index && entry.bytes.as_ref() == bytes
        }) {
            let entry = self.entries.remove(position)?;
            let face = entry.face.clone();
            self.entries.push_front(entry);
            return Some(face);
        }
        let face = if index == 0 {
            with_font_mgr(|mgr| mgr.new_from_data(bytes, None))?
        } else {
            let single = sfnt_from_collection(bytes, index)?;
            with_font_mgr(|mgr| mgr.new_from_data(&single, None))?
        };
        // Large fonts remain usable without occupying the entire cache.
        if bytes.len() <= MAX_BYTES {
            while self.entries.len() >= MAX_FACES || self.bytes > MAX_BYTES - bytes.len() {
                let old = self.entries.pop_back()?;
                self.bytes -= old.bytes.len();
            }
            self.bytes += bytes.len();
            self.entries.push_front(Entry {
                hash,
                bytes: bytes.into(),
                index,
                face: face.clone(),
            });
        }
        Some(face)
    }
}

pub(super) fn face(bytes: &[u8], index: usize) -> Option<Typeface> {
    thread_local! {
        static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    }
    CACHE.with(|cache| cache.borrow_mut().face(bytes, index))
}
