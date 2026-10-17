//! Layout cache module for text measurement and paragraph layout caching.
//!
//! This module provides caching mechanisms for:
//! 1. **Text measurement results** (`measure_text`): By caching the width of
//!    text segments keyed by their font descriptor and content, we can skip
//!    expensive canvas measurement calls when the same text/font combination
//!    is requested again.
//! 2. **Paragraph layout results**: By caching the layout results of paragraphs
//!    and detecting changes via content hashing, we enable incremental layout
//!    where only modified paragraphs and those after them need to be re-laid out.
//!
//! ## Performance
//!
//! This implementation uses hash buckets plus an intrusive doubly linked list
//! so the LRU hot path stays stable:
//! - **get**: O(1) expected lookup + O(1) move-to-back
//! - **insert**: O(1) expected insertion + O(1) eviction
//! - **eviction**: O(1) removal of the oldest entry (front of the list)
//!
//! The previous `IndexMap` implementation looked simple, but `move_index` and
//! `shift_remove_index` still paid index maintenance costs that showed up in
//! the browser CPU profile.

use rustc_hash::{FxHashMap, FxHasher};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub trait TextMeasurer {
    type Error;
    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error>;

    /// Measures text using a caller-supplied *precomputed* font-descriptor hash.
    ///
    /// The wrap engine already carries a stable `font_descriptor_hash: u64` next
    /// to every descriptor (resolved once per applied style). Passing it through
    /// lets the measurement cache resolve the descriptor's interned id from the
    /// hash in O(1) — without re-hashing the (often long) descriptor string on
    /// every grapheme, which is what the P1 interner paid per lookup (`S4`).
    ///
    /// The default forwards to [`Self::measure_text`], so impls that do not use
    /// a [`TextMeasureCache`] (tests, tooling) need no change and stay correct.
    #[inline]
    fn measure_text_with_hash(
        &self,
        text: &str,
        font_descriptor: &str,
        _font_descriptor_hash: u64,
    ) -> Result<f64, Self::Error> {
        self.measure_text(text, font_descriptor)
    }
}

/// Interns font-descriptor strings to small stable `u32` ids so measurement
/// lookups can key on `(font_id, text)` without re-hashing the descriptor bytes.
///
/// Extracted from [`TextMeasureCache`] so a single interner can be shared (via
/// `Rc<RefCell<_>>`) by every cache belonging to one renderer. Sharing keeps ids
/// consistent across the ~3 caches (canvas_renderer, font_measurer, table) so an
/// id resolved once at style-resolution time is valid for all of them and can
/// never collide across caches.
#[derive(Debug, Default)]
pub struct FontDescriptorInterner {
    /// Descriptor string → id.
    ids: FxHashMap<Box<str>, u32>,
    /// id → owned descriptor string (index = id). Lets fast paths content-verify.
    by_id: Vec<Box<str>>,
    /// Precomputed-hash → id index. `intern_with_hash` uses this to recover an id
    /// from the caller's `font_descriptor_hash` without hashing the descriptor
    /// string. Collisions (rare) are disambiguated by a one-time byte compare on
    /// first insertion; thereafter the `(hash, id)` pairing is stable.
    id_by_hash: FxHashMap<u64, u32>,
    /// Pointer fast path for consecutive lookups reusing the same descriptor
    /// `&str` allocation. Confirmed by byte compare (never a hash) to guard
    /// against a reused freed allocation.
    last_descriptor: Option<(usize, usize, u32)>,
}

impl FontDescriptorInterner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the owned descriptor string for an id, if interned.
    #[inline]
    pub fn descriptor_for_id(&self, id: u32) -> Option<&str> {
        self.by_id.get(id as usize).map(|s| s.as_ref())
    }

    /// Interns a descriptor by its bytes (no precomputed hash available).
    ///
    /// Serves the common (repeated descriptor `&str`) case from the pointer fast
    /// path without hashing; a genuinely new-to-this-call descriptor pays one
    /// descriptor-byte hash via the `ids` map.
    #[inline]
    pub fn intern(&mut self, font_descriptor: &str) -> u32 {
        let ptr = font_descriptor.as_ptr() as usize;
        let len = font_descriptor.len();
        if let Some((last_ptr, last_len, id)) = self.last_descriptor {
            if last_ptr == ptr
                && last_len == len
                && self.by_id[id as usize].as_ref() == font_descriptor
            {
                return id;
            }
        }
        let id = if let Some(&id) = self.ids.get(font_descriptor) {
            id
        } else {
            self.insert_new(font_descriptor)
        };
        self.last_descriptor = Some((ptr, len, id));
        id
    }

    /// Interns a descriptor using a caller-supplied precomputed hash.
    ///
    /// After the first sighting of a descriptor this resolves the id from the
    /// u64 hash in O(1) — no descriptor-string hashing, no full-string HashMap
    /// probe. A pointer fast path still short-circuits repeated same-`&str`
    /// lookups. Hash collisions are resolved by a byte compare, only when the
    /// stored-for-that-hash descriptor differs from the incoming one.
    #[inline]
    pub fn intern_with_hash(&mut self, font_descriptor: &str, hash: u64) -> u32 {
        let ptr = font_descriptor.as_ptr() as usize;
        let len = font_descriptor.len();
        if let Some((last_ptr, last_len, id)) = self.last_descriptor {
            if last_ptr == ptr
                && last_len == len
                && self.by_id[id as usize].as_ref() == font_descriptor
            {
                return id;
            }
        }
        let id = if let Some(&id) = self.id_by_hash.get(&hash) {
            // Confirm the hash mapping matches this descriptor's bytes (guards the
            // rare distinct-descriptor hash collision). This compare happens only
            // when the pointer fast path missed; on the steady state the pointer
            // path already returned above, so it is not the per-grapheme cost.
            if self.by_id[id as usize].as_ref() == font_descriptor {
                id
            } else {
                // Genuine collision: fall back to the byte-keyed map / new insert.
                self.ids
                    .get(font_descriptor)
                    .copied()
                    .unwrap_or_else(|| self.insert_new(font_descriptor))
            }
        } else {
            let id = self
                .ids
                .get(font_descriptor)
                .copied()
                .unwrap_or_else(|| self.insert_new(font_descriptor));
            self.id_by_hash.entry(hash).or_insert(id);
            id
        };
        self.last_descriptor = Some((ptr, len, id));
        id
    }

    #[inline]
    fn insert_new(&mut self, font_descriptor: &str) -> u32 {
        let id = self.by_id.len() as u32;
        let owned: Box<str> = font_descriptor.into();
        self.by_id.push(owned.clone());
        self.ids.insert(owned, id);
        id
    }

    /// Resets the pointer fast path. Call when the descriptor allocations a cache
    /// held could be freed (e.g. on `TextMeasureCache::clear`) so a stale pointer
    /// cannot alias a reused allocation. The id tables stay warm.
    #[inline]
    pub fn reset_pointer_fast_path(&mut self) {
        self.last_descriptor = None;
    }

    /// Number of distinct interned descriptors (test-only introspection).
    #[cfg(test)]
    pub fn interned_count(&self) -> usize {
        self.by_id.len()
    }
}

/// Maximum number of entries in the text measurement cache.
/// This prevents unbounded memory growth while keeping frequently
/// accessed measurements available.
/// 8192 entries gives better hit rates for CJK-heavy documents where the
/// number of distinct grapheme × font combinations is much larger than for
/// Latin-only text.
const DEFAULT_CACHE_CAPACITY: usize = 8192;

/// Key for text measurement cache lookups.
/// Combines the font descriptor string with the measured text.
///
/// The bucket hash the cache indexes by is *not* derived from the descriptor
/// bytes (see [`TextMeasureCache`]): the descriptor is interned to a small
/// integer id and mixed with a text-only hash, so cache hits never re-hash the
/// (often long) descriptor string. This key therefore only precomputes the
/// text hash; the descriptor bytes are hashed at most once, when a previously
/// unseen descriptor is interned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextMeasureCacheKey {
    /// Pre-computed hash of `text` only (descriptor is handled via interning).
    text_hash: u64,
    /// The CSS font descriptor (e.g., "12pt 'Times New Roman', serif")
    pub font_descriptor: String,
    /// The text being measured
    pub text: String,
}

impl Hash for TextMeasureCacheKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Use pre-computed text hash for faster hashing. The descriptor
        // contributes through the cache's interned id, not through this hash.
        self.text_hash.hash(state);
    }
}

impl TextMeasureCacheKey {
    /// Creates a new cache key from font descriptor and text.
    pub fn new(font_descriptor: impl Into<String>, text: impl Into<String>) -> Self {
        let font_descriptor = font_descriptor.into();
        let text = text.into();
        let text_hash = hash_text(&text);
        Self {
            text_hash,
            font_descriptor,
            text,
        }
    }

    /// Creates a new cache key without hashing the descriptor bytes.
    /// Used on the insert (miss) path; only the text is hashed.
    #[inline]
    pub fn from_refs(font_descriptor: &str, text: &str) -> Self {
        Self {
            text_hash: hash_text(text),
            font_descriptor: font_descriptor.to_string(),
            text: text.to_string(),
        }
    }

    /// Returns the text-only hash used to seed the bucket hash.
    #[inline]
    pub fn lookup_hash(_font_descriptor: &str, text: &str) -> u64 {
        hash_text(text)
    }
}

/// Hashes text only (no font descriptor).
#[inline]
fn hash_text(text: &str) -> u64 {
    let mut hasher = FxHasher::default();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Mixes an interned font descriptor id into a text hash to form the bucket
/// hash. `font_id` mixing is O(1); the text hash is precomputed once.
#[inline]
fn mix_font_id(font_id: u32, text_hash: u64) -> u64 {
    text_hash.rotate_left(7).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (font_id as u64)
}

/// Cached result of a text measurement operation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMeasureResult {
    /// The measured width in CSS pixels
    pub width: f64,
}

impl TextMeasureResult {
    /// Creates a new measurement result.
    pub fn new(width: f64) -> Self {
        Self { width }
    }
}

/// LRU-based cache for text measurement results.
///
/// This cache stores the results of `measure_text` calls keyed by
/// the font descriptor and text content. When the cache reaches its
/// capacity, the least recently used entries are evicted.
///
/// ## Implementation Details
///
pub struct TextMeasureCache {
    buckets: FxHashMap<u64, Vec<usize>>,
    nodes: Vec<Option<TextMeasureCacheNode>>,
    free_list: Vec<usize>,
    last_lookup_index: Option<usize>,
    second_last_lookup_index: Option<usize>,
    head: Option<usize>,
    tail: Option<usize>,
    len: usize,
    capacity: usize,
    /// Interns font descriptor strings to small stable ids so lookups can key on
    /// `(font_id, text)` without re-hashing the descriptor bytes. Shared across
    /// the caches of one renderer so ids are consistent and never collide across
    /// caches; a `font_id` resolved once at style resolution is valid here.
    interner: Rc<RefCell<FontDescriptorInterner>>,
    /// Statistics for monitoring cache effectiveness
    hits: u64,
    misses: u64,
}

#[derive(Debug)]
struct TextMeasureCacheNode {
    key: TextMeasureCacheKey,
    /// Interned id of `key.font_descriptor`. Bucket hash = `mix_font_id(font_id, key.text_hash)`.
    font_id: u32,
    result: TextMeasureResult,
    prev: Option<usize>,
    next: Option<usize>,
}

impl Default for TextMeasureCache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_CAPACITY)
    }
}

impl TextMeasureCache {
    /// Creates a new cache with the specified capacity and a private interner.
    pub fn new(capacity: usize) -> Self {
        Self::with_interner(
            capacity,
            Rc::new(RefCell::new(FontDescriptorInterner::new())),
        )
    }

    /// Creates a new cache that shares `interner` with other caches of the same
    /// renderer, so interned font ids are consistent across all of them.
    pub fn with_interner(capacity: usize, interner: Rc<RefCell<FontDescriptorInterner>>) -> Self {
        Self {
            buckets: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
            nodes: Vec::with_capacity(capacity),
            free_list: Vec::new(),
            last_lookup_index: None,
            second_last_lookup_index: None,
            head: None,
            tail: None,
            len: 0,
            capacity,
            interner,
            hits: 0,
            misses: 0,
        }
    }

    /// Returns a clone of the shared interner handle so additional caches can be
    /// constructed with [`Self::with_interner`].
    pub fn interner(&self) -> Rc<RefCell<FontDescriptorInterner>> {
        Rc::clone(&self.interner)
    }

    /// Interns a font descriptor to a stable `u32` id (byte-keyed).
    #[inline]
    fn intern_descriptor(&mut self, font_descriptor: &str) -> u32 {
        self.interner.borrow_mut().intern(font_descriptor)
    }

    /// Interns a font descriptor using a caller-supplied precomputed hash.
    #[inline]
    fn intern_descriptor_with_hash(&mut self, font_descriptor: &str, hash: u64) -> u32 {
        self.interner
            .borrow_mut()
            .intern_with_hash(font_descriptor, hash)
    }

    #[inline]
    fn last_lookup_result(&self, font_id: u32, text: &str) -> Option<(usize, TextMeasureResult)> {
        let index = self.last_lookup_index?;
        let node = self.node(index)?;
        // The interned id disambiguates the descriptor, so we only compare text.
        if node.font_id == font_id && node.key.text.len() == text.len() && node.key.text == text {
            Some((index, node.result))
        } else {
            None
        }
    }

    #[inline]
    fn second_last_lookup_result(
        &self,
        font_id: u32,
        text: &str,
    ) -> Option<(usize, TextMeasureResult)> {
        let index = self.second_last_lookup_index?;
        if self.last_lookup_index == Some(index) {
            return None;
        }
        let node = self.node(index)?;
        if node.font_id == font_id && node.key.text.len() == text.len() && node.key.text == text {
            Some((index, node.result))
        } else {
            None
        }
    }

    #[inline]
    fn record_lookup_index(&mut self, index: usize) {
        if self.last_lookup_index == Some(index) {
            return;
        }
        self.second_last_lookup_index = self.last_lookup_index;
        self.last_lookup_index = Some(index);
    }

    /// Attempts to retrieve a cached measurement result.
    /// Returns `Some(result)` if found, moving the entry to the back (MRU position).
    ///
    /// Time complexity: O(1)
    pub fn get(&mut self, key: &TextMeasureCacheKey) -> Option<TextMeasureResult> {
        self.get_fast(&key.font_descriptor, &key.text)
    }

    /// Fast lookup that avoids creating a new TextMeasureCacheKey.
    /// Uses borrowed-key lookup for allocation-free O(1) access.
    ///
    /// Time complexity: O(1)
    #[inline]
    pub fn get_fast(&mut self, font_descriptor: &str, text: &str) -> Option<TextMeasureResult> {
        // Intern the descriptor first: on the common (repeated descriptor) path
        // this recovers the id without hashing the descriptor bytes at all.
        let font_id = self.intern_descriptor(font_descriptor);
        self.get_by_font_id(font_id, text, font_descriptor)
    }

    /// Fast lookup using a caller-supplied *precomputed* descriptor hash.
    ///
    /// The hash resolves the interned id in O(1) (no descriptor-string hashing),
    /// then the lookup keys on `(font_id, text)` and never re-interns or
    /// byte-compares the descriptor on a hit. `font_descriptor` is still passed
    /// only so the miss/insert path can build the owned key and (in debug) verify
    /// the stored descriptor matches.
    #[inline]
    pub fn get_fast_with_hash(
        &mut self,
        font_descriptor: &str,
        text: &str,
        font_descriptor_hash: u64,
    ) -> Option<TextMeasureResult> {
        let font_id = self.intern_descriptor_with_hash(font_descriptor, font_descriptor_hash);
        self.get_by_font_id(font_id, text, font_descriptor)
    }

    /// Shared lookup core keyed by the already-resolved interned `font_id`.
    ///
    /// `descriptor` is only used by the `debug_assertions` correctness guard: on
    /// a hit it asserts the stored node's descriptor equals the descriptor the
    /// caller resolved the id from, so any mis-threaded `font_id` fails loudly in
    /// tests / native dev while costing nothing in release / wasm.
    #[inline]
    fn get_by_font_id(
        &mut self,
        font_id: u32,
        text: &str,
        #[allow(unused_variables)] descriptor: &str,
    ) -> Option<TextMeasureResult> {
        if let Some((index, result)) = self.last_lookup_result(font_id, text) {
            self.debug_assert_descriptor(index, descriptor);
            self.touch(index);
            self.record_lookup_index(index);
            self.hits += 1;
            return Some(result);
        }
        if let Some((index, result)) = self.second_last_lookup_result(font_id, text) {
            self.debug_assert_descriptor(index, descriptor);
            self.touch(index);
            self.record_lookup_index(index);
            self.hits += 1;
            return Some(result);
        }

        // Only the text is hashed here; the descriptor contributes via `font_id`.
        let hash = mix_font_id(font_id, hash_text(text));
        if let Some(index) = self.find_index_in_bucket(hash, font_id, text) {
            self.debug_assert_descriptor(index, descriptor);
            self.touch(index);
            self.record_lookup_index(index);
            self.hits += 1;
            return self.node(index).map(|node| node.result);
        }

        self.misses += 1;
        None
    }

    /// Correctness guard: assert the cached node's descriptor matches the one the
    /// caller resolved its `font_id` from. Compiled out entirely in release.
    #[inline(always)]
    fn debug_assert_descriptor(&self, index: usize, descriptor: &str) {
        #[cfg(debug_assertions)]
        if let Some(node) = self.node(index) {
            debug_assert_eq!(
                node.key.font_descriptor, descriptor,
                "measure cache hit returned a width for the WRONG font: a font_id \
                 was mis-threaded (stored != resolved descriptor)"
            );
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = (index, descriptor);
        }
    }

    /// Measures text width using the provided measurer and caches the result.
    /// If the measurement is cached, returns the cached result immediately.
    ///
    /// The `font_descriptor` is part of the cache key and should already be
    /// applied by the caller to keep measurements consistent.
    pub fn measure_with<M: TextMeasurer>(
        &mut self,
        measurer: &M,
        text: &str,
        font_descriptor: &str,
    ) -> Result<f64, M::Error> {
        if let Some(result) = self.get_fast(font_descriptor, text) {
            return Ok(result.width);
        }

        let width = measurer.measure_text(text, font_descriptor)?;
        let key = TextMeasureCacheKey::from_refs(font_descriptor, text);
        self.insert(key, TextMeasureResult::new(width));
        Ok(width)
    }

    /// Like [`Self::measure_with`], but keyed by a caller-supplied precomputed
    /// descriptor hash so cache hits skip re-interning/re-hashing the descriptor
    /// string (S4). Falls back to `measurer.measure_text` on a miss.
    pub fn measure_with_hash<M: TextMeasurer>(
        &mut self,
        measurer: &M,
        text: &str,
        font_descriptor: &str,
        font_descriptor_hash: u64,
    ) -> Result<f64, M::Error> {
        if let Some(result) = self.get_fast_with_hash(font_descriptor, text, font_descriptor_hash) {
            return Ok(result.width);
        }

        let width = measurer.measure_text(text, font_descriptor)?;
        let key = TextMeasureCacheKey::from_refs(font_descriptor, text);
        self.insert(key, TextMeasureResult::new(width));
        Ok(width)
    }

    /// Like [`Self::measure_with`], but keyed by an *already-resolved* interned
    /// `font_id` (obtained from this cache's shared interner). Cache hits never
    /// intern, hash, or byte-compare the descriptor. `font_descriptor` is only
    /// used on the miss/insert path and by the debug correctness guard.
    pub fn measure_with_id<M: TextMeasurer>(
        &mut self,
        measurer: &M,
        text: &str,
        font_descriptor: &str,
        font_id: u32,
    ) -> Result<f64, M::Error> {
        if let Some(result) = self.get_by_font_id(font_id, text, font_descriptor) {
            return Ok(result.width);
        }

        let width = measurer.measure_text(text, font_descriptor)?;
        let key = TextMeasureCacheKey::from_refs(font_descriptor, text);
        self.insert(key, TextMeasureResult::new(width));
        Ok(width)
    }

    /// Gets or inserts with fast path that minimizes allocations.
    /// Only allocates strings on cache miss.
    #[inline]
    pub fn get_or_insert_fast<F>(
        &mut self,
        font_descriptor: &str,
        text: &str,
        compute: F,
    ) -> TextMeasureResult
    where
        F: FnOnce() -> TextMeasureResult,
    {
        // Try fast lookup first
        if let Some(result) = self.get_fast(font_descriptor, text) {
            return result;
        }

        // Cache miss - compute and store
        let result = compute();
        let key = TextMeasureCacheKey::from_refs(font_descriptor, text);
        self.insert(key, result);
        result
    }

    /// Inserts a measurement result into the cache.
    /// If the cache is at capacity, evicts the least recently used entry (front).
    ///
    /// Time complexity: O(1)
    #[inline]
    pub fn insert(&mut self, key: TextMeasureCacheKey, result: TextMeasureResult) {
        if self.capacity == 0 {
            return;
        }

        let font_id = self.intern_descriptor(&key.font_descriptor);
        let hash = mix_font_id(font_id, key.text_hash);

        if let Some(index) = self.find_index_by_parts(hash, font_id, &key.text) {
            if let Some(node) = self.node_mut(index) {
                node.result = result;
            }
            self.touch(index);
            self.record_lookup_index(index);
            return;
        }

        if self.len >= self.capacity {
            self.evict_lru();
        }

        let index = self.alloc_node(TextMeasureCacheNode {
            key,
            font_id,
            result,
            prev: None,
            next: None,
        });
        self.attach_to_tail(index);
        self.buckets.entry(hash).or_default().push(index);
        self.record_lookup_index(index);
        self.len += 1;
    }

    /// Gets a cached value or computes and caches it using the provided closure.
    ///
    /// Time complexity: O(1) for cache hit, O(1) + closure time for cache miss
    pub fn get_or_insert_with<F>(&mut self, key: TextMeasureCacheKey, f: F) -> TextMeasureResult
    where
        F: FnOnce() -> TextMeasureResult,
    {
        if let Some(result) = self.get(&key) {
            return result;
        }

        let result = f();
        self.insert(key, result);
        result
    }

    /// Returns the current number of cached entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Clears all entries from the cache.
    pub fn clear(&mut self) {
        self.buckets.clear();
        self.nodes.clear();
        self.free_list.clear();
        self.last_lookup_index = None;
        self.second_last_lookup_index = None;
        self.head = None;
        self.tail = None;
        self.len = 0;
        // The descriptor interning table is intentionally kept warm across
        // clears (descriptors are few and reused). Reset the pointer-based fast
        // path, though: a stale pointer could otherwise alias a freed allocation.
        self.interner.borrow_mut().reset_pointer_fast_path();
    }

    /// Returns cache statistics (hits, misses).
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            size: self.len,
            capacity: self.capacity,
        }
    }

    /// Resets hit/miss statistics.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    #[inline]
    /// Looks up a cache index by hash and interned font id, checking the
    /// last/second-last lookup indices first before falling back to the bucket
    /// HashMap. The interned id disambiguates descriptors, so only text is compared.
    fn find_index_by_parts(&self, hash: u64, font_id: u32, text: &str) -> Option<usize> {
        if let Some(index) = self.last_lookup_index {
            if let Some(node) = self.node(index) {
                if node.font_id == font_id && node.key.text == text {
                    return Some(index);
                }
            }
        }
        if let Some(index) = self.second_last_lookup_index {
            if self.last_lookup_index != Some(index) {
                if let Some(node) = self.node(index) {
                    if node.font_id == font_id && node.key.text == text {
                        return Some(index);
                    }
                }
            }
        }
        self.find_index_in_bucket(hash, font_id, text)
    }

    /// Looks up a cache index by hash only in the bucket HashMap, skipping
    /// the last/second-last lookup index checks. Use this when those indices
    /// have already been checked by the caller.
    #[inline]
    fn find_index_in_bucket(&self, hash: u64, font_id: u32, text: &str) -> Option<usize> {
        let text_len = text.len();
        self.buckets.get(&hash).and_then(|indices| {
            indices.iter().copied().find(|&index| {
                self.node(index)
                    .map(|node| {
                        // The interned id fully identifies the descriptor, so a
                        // matching id + text is a match (fast reject by length first).
                        node.font_id == font_id
                            && node.key.text.len() == text_len
                            && node.key.text == text
                    })
                    .unwrap_or(false)
            })
        })
    }

    #[inline]
    fn node(&self, index: usize) -> Option<&TextMeasureCacheNode> {
        self.nodes.get(index).and_then(|node| node.as_ref())
    }

    #[inline]
    fn node_mut(&mut self, index: usize) -> Option<&mut TextMeasureCacheNode> {
        self.nodes.get_mut(index).and_then(|node| node.as_mut())
    }

    #[inline]
    fn alloc_node(&mut self, node: TextMeasureCacheNode) -> usize {
        if let Some(index) = self.free_list.pop() {
            self.nodes[index] = Some(node);
            index
        } else {
            self.nodes.push(Some(node));
            self.nodes.len() - 1
        }
    }

    #[inline]
    fn attach_to_tail(&mut self, index: usize) {
        let prev_tail = self.tail;
        if let Some(node) = self.node_mut(index) {
            node.prev = prev_tail;
            node.next = None;
        }
        if let Some(tail) = prev_tail {
            if let Some(node) = self.node_mut(tail) {
                node.next = Some(index);
            }
        } else {
            self.head = Some(index);
        }
        self.tail = Some(index);
    }

    #[inline]
    fn detach(&mut self, index: usize) {
        let (prev, next) = if let Some(node) = self.node(index) {
            (node.prev, node.next)
        } else {
            return;
        };

        if let Some(prev_index) = prev {
            if let Some(node) = self.node_mut(prev_index) {
                node.next = next;
            }
        } else {
            self.head = next;
        }

        if let Some(next_index) = next {
            if let Some(node) = self.node_mut(next_index) {
                node.prev = prev;
            }
        } else {
            self.tail = prev;
        }

        if let Some(node) = self.node_mut(index) {
            node.prev = None;
            node.next = None;
        }
    }

    #[inline]
    fn touch(&mut self, index: usize) {
        if self.tail == Some(index) {
            return;
        }
        self.detach(index);
        self.attach_to_tail(index);
    }

    fn evict_lru(&mut self) {
        let Some(index) = self.head else {
            return;
        };
        self.remove_index(index);
    }

    fn remove_index(&mut self, index: usize) {
        let hash = if let Some(node) = self.node(index) {
            mix_font_id(node.font_id, node.key.text_hash)
        } else {
            return;
        };

        if self.last_lookup_index == Some(index) {
            self.last_lookup_index = None;
        }
        if self.second_last_lookup_index == Some(index) {
            self.second_last_lookup_index = None;
        }

        self.detach(index);

        if let Some(indices) = self.buckets.get_mut(&hash) {
            if let Some(pos) = indices.iter().position(|&candidate| candidate == index) {
                indices.swap_remove(pos);
            }
            if indices.is_empty() {
                self.buckets.remove(&hash);
            }
        }

        self.nodes[index] = None;
        self.free_list.push(index);
        self.len = self.len.saturating_sub(1);
    }
}

/// Statistics about cache performance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub size: usize,
    pub capacity: usize,
}

impl CacheStats {
    /// Returns the cache hit rate as a value between 0.0 and 1.0.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

// =============================================================================
// Paragraph Layout Cache for Incremental Layout
// =============================================================================

/// A content hash representing the state of a paragraph.
/// Used to detect changes and trigger incremental re-layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct ContentHash(u64);

impl ContentHash {
    /// Creates a new content hash from raw value.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw hash value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

/// Computes a content hash for the given hashable value.
pub fn compute_content_hash<T: Hash>(value: &T) -> ContentHash {
    let mut hasher = FxHasher::default();
    value.hash(&mut hasher);
    ContentHash(hasher.finish())
}

/// Computes a content hash for a paragraph by combining its text content
/// and relevant properties that affect layout.
///
/// This hash is used to detect when a paragraph's content has changed,
/// triggering incremental re-layout from that point onwards.
pub fn compute_paragraph_hash(
    text: &str,
    properties: Option<&ParagraphHashableProps>,
) -> ContentHash {
    let mut hasher = FxHasher::default();
    text.hash(&mut hasher);
    if let Some(props) = properties {
        props.hash(&mut hasher);
    }
    ContentHash(hasher.finish())
}

/// A subset of paragraph properties that affect layout and should be
/// included in the content hash.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ParagraphHashableProps {
    /// Alignment (left, center, right, justify)
    pub alignment: Option<String>,
    /// Spacing before/after in points
    pub spacing_before: Option<i32>,
    pub spacing_after: Option<i32>,
    pub line_spacing: Option<i32>,
    /// Indentation in points (scaled for comparison)
    pub indent_left: Option<i32>,
    pub indent_right: Option<i32>,
    pub indent_first_line: Option<i32>,
    pub indent_hanging: Option<i32>,
    /// Character-based indentation in hundredths of a character width
    pub indent_left_chars: Option<i32>,
    pub indent_right_chars: Option<i32>,
    /// Style name
    pub style: Option<String>,
    /// Numbering reference
    pub numbering_id: Option<String>,
    pub numbering_level: Option<i32>,
}

impl ParagraphHashableProps {
    /// Creates hashable props from raw values.
    /// Floats are converted to integers (scaled by 100) for reliable hashing.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        alignment: Option<&str>,
        spacing_before: Option<f32>,
        spacing_after: Option<f32>,
        line_spacing: Option<f32>,
        indent_left: Option<f32>,
        indent_right: Option<f32>,
        indent_first_line: Option<f32>,
        indent_hanging: Option<f32>,
        indent_left_chars: Option<i32>,
        indent_right_chars: Option<i32>,
        style: Option<&str>,
        numbering_id: Option<&str>,
        numbering_level: Option<u32>,
    ) -> Self {
        Self {
            alignment: alignment.map(|s| s.to_string()),
            spacing_before: spacing_before.map(|v| (v * 100.0) as i32),
            spacing_after: spacing_after.map(|v| (v * 100.0) as i32),
            line_spacing: line_spacing.map(|v| (v * 100.0) as i32),
            indent_left: indent_left.map(|v| (v * 100.0) as i32),
            indent_right: indent_right.map(|v| (v * 100.0) as i32),
            indent_first_line: indent_first_line.map(|v| (v * 100.0) as i32),
            indent_hanging: indent_hanging.map(|v| (v * 100.0) as i32),
            indent_left_chars,
            indent_right_chars,
            style: style.map(|s| s.to_string()),
            numbering_id: numbering_id.map(|s| s.to_string()),
            numbering_level: numbering_level.map(|v| v as i32),
        }
    }
}

/// Cached layout result for a single paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphLayoutResult {
    /// The content hash when this layout was computed.
    pub content_hash: ContentHash,
    /// Absolute Y position where this paragraph starts (includes page offset).
    pub start_y: f64,
    /// Absolute Y position where this paragraph ends (includes page offset).
    pub end_y: f64,
    /// 0-indexed page number where this paragraph starts.
    pub start_page: usize,
    /// 0-indexed page number where this paragraph ends.
    pub end_page: usize,
    /// Absolute page offset at the start of this paragraph's start page.
    pub start_page_offset_y: f64,
    /// Absolute page offset at the end of this paragraph's end page.
    pub end_page_offset_y: f64,
    /// Column index where this paragraph starts.
    pub start_column: usize,
    /// Column index where this paragraph ends.
    pub end_column: usize,
    /// Absolute baseline of the last rendered line before this paragraph starts.
    pub start_last_line_baseline: f64,
    /// Absolute baseline of the last rendered line after this paragraph ends.
    pub end_last_line_baseline: f64,
    /// Spacing-after already included in end_y, consumed by the next paragraph.
    pub trailing_spacing_after: Option<f64>,
    /// Height of this paragraph.
    pub height: f64,
}

impl ParagraphLayoutResult {
    /// Creates a new paragraph layout result.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        content_hash: ContentHash,
        start_y: f64,
        end_y: f64,
        start_page: usize,
        end_page: usize,
        start_page_offset_y: f64,
        end_page_offset_y: f64,
        start_column: usize,
        end_column: usize,
        start_last_line_baseline: f64,
        end_last_line_baseline: f64,
    ) -> Self {
        Self {
            content_hash,
            start_y,
            end_y,
            start_page,
            end_page,
            start_page_offset_y,
            end_page_offset_y,
            start_column,
            end_column,
            start_last_line_baseline,
            end_last_line_baseline,
            trailing_spacing_after: None,
            height: end_y - start_y,
        }
    }

    pub fn apply_y_offset(&mut self, offset: f64) {
        if offset.abs() < f64::EPSILON {
            return;
        }
        self.start_y += offset;
        self.end_y += offset;
        self.start_last_line_baseline += offset;
        self.end_last_line_baseline += offset;
    }

    fn within_page_bounds_after_offset(
        &self,
        offset: f64,
        page_height: f64,
        margin_top: f64,
        margin_bottom: f64,
    ) -> bool {
        const EPS: f64 = 0.001;
        let start_top = self.start_page_offset_y + margin_top;
        let start_bottom = self.start_page_offset_y + page_height - margin_bottom;
        let end_top = self.end_page_offset_y + margin_top;
        let end_bottom = self.end_page_offset_y + page_height - margin_bottom;
        let new_start_y = self.start_y + offset;
        let new_end_y = self.end_y + offset;

        new_start_y >= start_top - EPS
            && new_start_y <= start_bottom + EPS
            && new_end_y >= end_top - EPS
            && new_end_y <= end_bottom + EPS
    }
}

/// Cache for paragraph layout results, enabling incremental layout.
///
/// This cache stores layout results indexed by paragraph index. When a
/// paragraph's content changes (detected via content hash), all paragraphs
/// from that point onwards must be re-laid out since their positions may
/// have shifted.
#[derive(Debug, Default)]
pub struct ParagraphLayoutCache {
    /// Cached layout results, indexed by paragraph index.
    entries: Vec<Option<ParagraphLayoutResult>>,
    /// Statistics for monitoring cache effectiveness.
    hits: u64,
    misses: u64,
    /// Number of paragraphs that were skipped due to cache hits.
    skipped_paragraphs: u64,
}

impl ParagraphLayoutCache {
    /// Creates a new empty paragraph layout cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a cache with pre-allocated capacity for the given number of paragraphs.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            hits: 0,
            misses: 0,
            skipped_paragraphs: 0,
        }
    }

    /// Checks if the cached layout for a paragraph is still valid.
    ///
    /// A cached entry is valid if:
    /// 1. An entry exists for the given paragraph index
    /// 2. The content hash matches the current paragraph content
    /// 3. The start_y matches the expected position (no upstream changes)
    ///
    /// Returns `Some(result)` if valid, `None` otherwise.
    pub fn get_if_valid(
        &mut self,
        paragraph_index: usize,
        content_hash: ContentHash,
        expected_start_y: f64,
    ) -> Option<&ParagraphLayoutResult> {
        if paragraph_index >= self.entries.len() {
            self.misses += 1;
            return None;
        }

        if let Some(ref entry) = self.entries[paragraph_index] {
            // Check both content hash and position
            if entry.content_hash == content_hash
                && (entry.start_y - expected_start_y).abs() < 0.001
            {
                self.hits += 1;
                self.skipped_paragraphs += 1;
                return Some(entry);
            }
        }

        self.misses += 1;
        None
    }

    /// Returns the cached layout entry for a paragraph index, if any.
    ///
    /// This does not validate the entry against current document content; callers should
    /// use it only as a coarse anchor (e.g. best-effort dirty-page estimation).
    pub fn get_entry(&self, paragraph_index: usize) -> Option<&ParagraphLayoutResult> {
        self.entries.get(paragraph_index).and_then(|e| e.as_ref())
    }

    /// Stores a layout result for a paragraph.
    ///
    /// This will also invalidate all cached entries after this paragraph
    /// since their positions may have changed.
    pub fn insert(&mut self, paragraph_index: usize, result: ParagraphLayoutResult) {
        // Ensure vector is large enough
        if paragraph_index >= self.entries.len() {
            self.entries.resize(paragraph_index + 1, None);
        }

        self.entries[paragraph_index] = Some(result);
    }

    /// Invalidates all cached entries from the given paragraph index onwards.
    ///
    /// This should be called when a paragraph's content or formatting changes,
    /// as all subsequent paragraphs may need new positions.
    pub fn invalidate_from(&mut self, paragraph_index: usize) {
        self.entries.truncate(paragraph_index);
    }

    pub fn try_apply_y_offset_from(
        &mut self,
        paragraph_index: usize,
        offset: f64,
        page_height: f64,
        margin_top: f64,
        margin_bottom: f64,
    ) -> bool {
        const EPS: f64 = 0.001;
        if offset.abs() < EPS {
            return true;
        }
        if paragraph_index >= self.entries.len() {
            return true;
        }

        for entry in &self.entries[paragraph_index..] {
            let Some(entry) = entry.as_ref() else {
                return false;
            };
            if entry.start_page != entry.end_page || entry.start_column != entry.end_column {
                return false;
            }
            if !entry.within_page_bounds_after_offset(
                offset,
                page_height,
                margin_top,
                margin_bottom,
            ) {
                return false;
            }
        }

        for entry in &mut self.entries[paragraph_index..] {
            if let Some(entry) = entry.as_mut() {
                entry.apply_y_offset(offset);
            }
        }
        true
    }

    /// Clears all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns the number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(|e| e.is_none())
    }

    /// Returns cache statistics.
    pub fn stats(&self) -> ParagraphLayoutCacheStats {
        ParagraphLayoutCacheStats {
            hits: self.hits,
            misses: self.misses,
            skipped_paragraphs: self.skipped_paragraphs,
            cached_entries: self.len(),
            total_capacity: self.entries.len(),
        }
    }

    /// Resets statistics.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
        self.skipped_paragraphs = 0;
    }

    /// Finds the first paragraph index that needs to be re-laid out.
    ///
    /// Compares the given content hashes against cached entries and returns
    /// the index of the first paragraph that has changed or is missing from cache.
    ///
    /// Returns `0` if no cache exists or the first paragraph changed.
    /// Returns `hashes.len()` if all paragraphs match (nothing to re-layout).
    pub fn find_first_dirty_paragraph(&self, hashes: &[ContentHash]) -> usize {
        for (index, hash) in hashes.iter().enumerate() {
            if index >= self.entries.len() {
                return index;
            }
            match &self.entries[index] {
                Some(entry) if entry.content_hash == *hash => continue,
                _ => return index,
            }
        }
        hashes.len()
    }

    /// Handles layout cache updates for paragraph break operations.
    ///
    /// This method:
    /// 1. Inserts a gap at `insert_at` for the new paragraph
    /// 2. Invalidates the edited paragraph (its content changed)
    /// 3. Attempts to apply a Y-offset to downstream entries if provided
    ///
    /// Returns `true` if the Y-offset was successfully applied to downstream entries,
    /// `false` if the cache was invalidated instead (e.g., page boundary would be crossed).
    #[allow(clippy::too_many_arguments)]
    pub fn handle_paragraph_break(
        &mut self,
        edited_index: usize,
        expected_height_delta: Option<f64>,
        page_height: f64,
        margin_top: f64,
        margin_bottom: f64,
    ) -> bool {
        // Insert a gap for the new paragraph
        let insert_at = edited_index.saturating_add(1);
        if insert_at > self.entries.len() {
            self.entries.resize(insert_at, None);
        }
        self.entries.insert(insert_at, None);

        // Invalidate the edited paragraph (content changed due to split)
        if edited_index < self.entries.len() {
            self.entries[edited_index] = None;
        }

        // If we have an expected height delta, try to apply it to downstream entries
        // Starting from insert_at + 1 (the first downstream paragraph that was shifted)
        let downstream_start = insert_at.saturating_add(1);
        if let Some(delta) = expected_height_delta {
            const EPS: f64 = 0.001;
            if delta.abs() >= EPS {
                return self.try_apply_y_offset_from(
                    downstream_start,
                    delta,
                    page_height,
                    margin_top,
                    margin_bottom,
                );
            }
        } else {
            // Without a known height delta, downstream positions are unsafe to reuse.
            // Invalidate from the newly inserted paragraph to force a fresh layout.
            self.invalidate_from(insert_at);
            return false;
        }

        true
    }

    /// Returns the cached height (end_y - start_y) for a paragraph if available.
    pub fn get_cached_height(&self, paragraph_index: usize) -> Option<f64> {
        self.entries
            .get(paragraph_index)
            .and_then(|e| e.as_ref())
            .map(|e| e.height)
    }

    /// Returns the cached end_y for a paragraph if available.
    pub fn get_cached_end_y(&self, paragraph_index: usize) -> Option<f64> {
        self.entries
            .get(paragraph_index)
            .and_then(|e| e.as_ref())
            .map(|e| e.end_y)
    }

    /// Handles paragraph merge (e.g., backspace at paragraph start merging with previous).
    ///
    /// This method:
    /// 1. Removes the entry for the deleted paragraph
    /// 2. Invalidates the merged paragraph (its content changed)
    /// 3. Attempts to apply a Y-offset to downstream entries if heights are known
    ///
    /// Parameters:
    /// - `merged_index`: The index of the paragraph that received the merged content
    /// - `deleted_index`: The index of the paragraph that was deleted (merged_index + 1)
    /// - `expected_height_delta`: The expected change in total height (usually negative)
    ///
    /// Returns `true` if the operation was successful.
    #[allow(clippy::too_many_arguments)]
    pub fn handle_paragraph_merge(
        &mut self,
        merged_index: usize,
        deleted_index: usize,
        expected_height_delta: Option<f64>,
        page_height: f64,
        margin_top: f64,
        margin_bottom: f64,
    ) -> bool {
        // Remove the deleted paragraph's entry
        if deleted_index < self.entries.len() {
            self.entries.remove(deleted_index);
        }

        // Invalidate the merged paragraph (content changed)
        if merged_index < self.entries.len() {
            self.entries[merged_index] = None;
        }

        // If we have an expected height delta, try to apply it to downstream entries
        // Starting from merged_index + 1 (the first downstream paragraph after the merge)
        let downstream_start = merged_index.saturating_add(1);
        if let Some(delta) = expected_height_delta {
            const EPS: f64 = 0.001;
            if delta.abs() >= EPS {
                return self.try_apply_y_offset_from(
                    downstream_start,
                    delta,
                    page_height,
                    margin_top,
                    margin_bottom,
                );
            }
        }

        true
    }
}

/// Statistics about paragraph layout cache performance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParagraphLayoutCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub skipped_paragraphs: u64,
    pub cached_entries: usize,
    pub total_capacity: usize,
}

impl ParagraphLayoutCacheStats {
    /// Returns the cache hit rate as a value between 0.0 and 1.0.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

// =============================================================================
// Dirty Region Tracking for Partial Re-rendering
// =============================================================================

/// A rectangular region that has been marked as dirty and needs re-rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirtyRect {
    /// Left edge in CSS pixels
    pub x: f64,
    /// Top edge in CSS pixels
    pub y: f64,
    /// Width in CSS pixels
    pub width: f64,
    /// Height in CSS pixels
    pub height: f64,
    /// Page number (0-indexed) this rect belongs to
    pub page: usize,
}

impl DirtyRect {
    /// Creates a new dirty rectangle.
    pub fn new(x: f64, y: f64, width: f64, height: f64, page: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
            page,
        }
    }

    /// Creates a dirty rect covering an entire page.
    ///
    /// NOTE: This sets `y = 0`. If you need an absolute document Y coordinate
    /// (including page gaps), use `full_page_at()` and pass the page's absolute
    /// top offset.
    pub fn full_page(page_width: f64, page_height: f64, page: usize) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: page_width,
            height: page_height,
            page,
        }
    }

    /// Creates a dirty rect covering an entire page, with an explicit absolute `y` offset.
    ///
    /// NOTE: In this project, `DirtyRect.y` is treated as an *absolute document Y*
    /// coordinate (including page gaps). For multi-page documents, callers should
    /// pass the page's absolute top offset.
    pub fn full_page_at(page_width: f64, page_height: f64, page: usize, page_top_y: f64) -> Self {
        Self {
            x: 0.0,
            y: page_top_y,
            width: page_width,
            height: page_height,
            page,
        }
    }

    /// Returns the right edge of this rect.
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    /// Returns the bottom edge of this rect.
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// Checks if this rect intersects with another rect on the same page.
    pub fn intersects(&self, other: &DirtyRect) -> bool {
        if self.page != other.page {
            return false;
        }
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    /// Merges this rect with another, returning the bounding box.
    /// Only merges if on the same page; returns self otherwise.
    pub fn merge(&self, other: &DirtyRect) -> DirtyRect {
        if self.page != other.page {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        DirtyRect {
            x,
            y,
            width: right - x,
            height: bottom - y,
            page: self.page,
        }
    }

    /// Checks if this rect contains a point.
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    /// Expands this rect by the given padding in all directions.
    pub fn expand(&self, padding: f64) -> DirtyRect {
        DirtyRect {
            x: (self.x - padding).max(0.0),
            y: (self.y - padding).max(0.0),
            width: self.width + 2.0 * padding,
            height: self.height + 2.0 * padding,
            page: self.page,
        }
    }
}

/// Tracks dirty regions that need re-rendering.
///
/// This is used to implement partial re-rendering where only changed
/// portions of the document are re-drawn, improving performance during
/// editing operations.
#[derive(Clone, Debug, Default)]
pub struct DirtyRegionTracker {
    /// List of dirty rectangles, grouped by page.
    regions: Vec<DirtyRect>,
    /// Whether the entire document needs re-rendering.
    full_repaint: bool,
    /// Change ID counter, incremented on each modification.
    change_id: u64,
}

fn should_merge_dirty_rects(left: &DirtyRect, right: &DirtyRect) -> bool {
    if left.page != right.page || !left.intersects(right) {
        return false;
    }
    let merged = left.merge(right);
    let source_area = (left.width * left.height).max(0.0) + (right.width * right.height).max(0.0);
    let merged_area = (merged.width * merged.height).max(0.0);
    source_area > 0.0 && merged_area <= source_area * 1.5
}

impl DirtyRegionTracker {
    /// Creates a new empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the entire document as needing full repaint.
    pub fn mark_full_repaint(&mut self) {
        self.full_repaint = true;
        self.regions.clear();
        self.change_id += 1;
    }

    /// Returns whether a full repaint is needed.
    pub fn needs_full_repaint(&self) -> bool {
        self.full_repaint
    }

    /// Marks a rectangular region as dirty.
    pub fn mark_dirty(&mut self, rect: DirtyRect) {
        if self.full_repaint {
            return; // Already doing full repaint
        }

        // Try to merge with existing rects on the same page
        let mut merged = false;
        for existing in &mut self.regions {
            if should_merge_dirty_rects(existing, &rect) {
                *existing = existing.merge(&rect);
                merged = true;
                break;
            }
        }

        if !merged {
            self.regions.push(rect);
        }
        self.change_id += 1;
    }

    /// Marks a paragraph region as dirty.
    /// The rect should cover the paragraph's bounding box.
    pub fn mark_paragraph_dirty(&mut self, paragraph_index: usize, rect: DirtyRect) {
        let _ = paragraph_index; // For future use
        self.mark_dirty(rect);
    }

    /// Returns all dirty regions.
    pub fn dirty_regions(&self) -> &[DirtyRect] {
        &self.regions
    }

    /// Returns dirty regions for a specific page.
    pub fn dirty_regions_for_page(&self, page: usize) -> Vec<&DirtyRect> {
        self.regions.iter().filter(|r| r.page == page).collect()
    }

    /// Clears all dirty regions after rendering.
    pub fn clear(&mut self) {
        self.regions.clear();
        self.full_repaint = false;
    }

    /// Returns the current change ID.
    pub fn change_id(&self) -> u64 {
        self.change_id
    }

    /// Returns true if there are any dirty regions.
    pub fn has_dirty_regions(&self) -> bool {
        self.full_repaint || !self.regions.is_empty()
    }

    /// Removes dirty regions for pages after the given page (0-indexed).
    /// This is used when we detect that no layout shift occurred after editing,
    /// so subsequent pages don't need to be re-rendered.
    pub fn clear_pages_after(&mut self, page: usize) {
        if self.full_repaint {
            // Cannot selectively clear when full repaint is needed
            return;
        }
        self.regions.retain(|r| r.page <= page);
        self.change_id += 1;
    }

    /// Coalesces overlapping dirty regions on the same page.
    pub fn coalesce(&mut self) {
        if self.regions.len() < 2 {
            return;
        }

        // Group by page
        let mut by_page: std::collections::HashMap<usize, Vec<DirtyRect>> =
            std::collections::HashMap::new();
        for rect in self.regions.drain(..) {
            by_page.entry(rect.page).or_default().push(rect);
        }

        // Merge overlapping rects within each page
        for (_page, rects) in by_page.iter_mut() {
            let mut merged: Vec<DirtyRect> = Vec::new();
            for rect in rects.drain(..) {
                let mut added = false;
                for existing in &mut merged {
                    if should_merge_dirty_rects(existing, &rect) {
                        *existing = existing.merge(&rect);
                        added = true;
                        break;
                    }
                }
                if !added {
                    merged.push(rect);
                }
            }
            *rects = merged;
        }

        // Flatten back
        for rects in by_page.into_values() {
            self.regions.extend(rects);
        }
    }
}

/// Represents a change to a paragraph that affects layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParagraphChange {
    /// Index of the changed paragraph.
    pub paragraph_index: usize,
    /// Type of change.
    pub change_type: ParagraphChangeType,
    /// Change ID when this change occurred.
    pub change_id: u64,
}

/// Types of changes that can occur to a paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParagraphChangeType {
    /// Content was modified (text added/removed).
    ContentModified,
    /// Formatting was changed (bold, italic, font size, etc.).
    FormattingChanged,
    /// Paragraph was inserted.
    Inserted,
    /// Paragraph was deleted.
    Deleted,
    /// Paragraph was split (e.g., by pressing Enter).
    Split,
    /// Paragraphs were merged (e.g., by backspace at start).
    Merged,
}

// =============================================================================
// Selection Overlay for Fast Selection Rendering
// =============================================================================

/// A rectangular region representing a selection highlight.
/// Used for rendering selection overlays without full document re-rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionRect {
    /// Left edge in CSS pixels
    pub x: f64,
    /// Top edge in CSS pixels
    pub y: f64,
    /// Width in CSS pixels
    pub width: f64,
    /// Height in CSS pixels
    pub height: f64,
    /// Page number (0-indexed) this rect belongs to
    pub page: usize,
}

impl SelectionRect {
    /// Creates a new selection rectangle.
    pub fn new(x: f64, y: f64, width: f64, height: f64, page: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
            page,
        }
    }

    /// Returns the right edge of this rect.
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    /// Returns the bottom edge of this rect.
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// Converts to a DirtyRect.
    pub fn to_dirty_rect(&self) -> DirtyRect {
        DirtyRect::new(self.x, self.y, self.width, self.height, self.page)
    }
}

/// Collection of selection rectangles for overlay rendering.
/// This allows fast selection updates without re-rendering the entire document.
#[derive(Clone, Debug, Default)]
pub struct SelectionOverlay {
    /// Selection highlight rectangles.
    rects: Vec<SelectionRect>,
    /// Caret position and dimensions (if selection is collapsed).
    caret: Option<CaretRect>,
    /// Whether the selection has changed since last render.
    dirty: bool,
}

/// Represents the caret (text cursor) position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretRect {
    /// X position in CSS pixels
    pub x: f64,
    /// Y position (top) in CSS pixels
    pub y: f64,
    /// Height in CSS pixels
    pub height: f64,
    /// Page number (0-indexed)
    pub page: usize,
}

impl CaretRect {
    /// Creates a new caret rect.
    pub fn new(x: f64, y: f64, height: f64, page: usize) -> Self {
        Self { x, y, height, page }
    }
}

impl SelectionOverlay {
    /// Creates a new empty selection overlay.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether the selection overlay needs re-rendering.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Marks the overlay as dirty (needs re-rendering).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Clears the dirty flag after rendering.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Clears all selection rectangles.
    pub fn clear(&mut self) {
        self.rects.clear();
        self.caret = None;
        self.dirty = true;
    }

    /// Sets the selection rectangles.
    pub fn set_rects(&mut self, rects: Vec<SelectionRect>) {
        self.rects = rects;
        self.caret = None;
        self.dirty = true;
    }

    /// Sets the caret position (for collapsed selection).
    pub fn set_caret(&mut self, caret: CaretRect) {
        self.rects.clear();
        self.caret = Some(caret);
        self.dirty = true;
    }

    /// Adds a selection rectangle.
    pub fn add_rect(&mut self, rect: SelectionRect) {
        self.rects.push(rect);
        self.dirty = true;
    }

    /// Returns all selection rectangles.
    pub fn rects(&self) -> &[SelectionRect] {
        &self.rects
    }

    /// Returns selection rectangles for a specific page.
    pub fn rects_for_page(&self, page: usize) -> Vec<&SelectionRect> {
        self.rects.iter().filter(|r| r.page == page).collect()
    }

    /// Returns the caret position, if selection is collapsed.
    pub fn caret(&self) -> Option<&CaretRect> {
        self.caret.as_ref()
    }

    /// Returns whether there is a selection (non-empty rects or caret).
    pub fn has_selection(&self) -> bool {
        !self.rects.is_empty() || self.caret.is_some()
    }

    /// Returns the number of selection rectangles.
    pub fn rect_count(&self) -> usize {
        self.rects.len()
    }

    /// Computes the bounding box of all selection rectangles on a page.
    pub fn bounding_box_for_page(&self, page: usize) -> Option<SelectionRect> {
        let page_rects: Vec<_> = self.rects.iter().filter(|r| r.page == page).collect();
        if page_rects.is_empty() {
            return self.caret.filter(|c| c.page == page).map(|c| {
                SelectionRect::new(c.x, c.y, 2.0, c.height, c.page) // Caret width = 2px
            });
        }

        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;

        for rect in page_rects {
            min_x = min_x.min(rect.x);
            min_y = min_y.min(rect.y);
            max_x = max_x.max(rect.right());
            max_y = max_y.max(rect.bottom());
        }

        Some(SelectionRect::new(
            min_x,
            min_y,
            max_x - min_x,
            max_y - min_y,
            page,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHasher;
    use std::hint::black_box;
    use std::time::Instant;

    struct LegacyTextMeasureCache {
        entries: indexmap::IndexMap<TextMeasureCacheKey, TextMeasureResult>,
        capacity: usize,
    }

    struct LegacyTextMeasureLookupKey<'a> {
        hash: u64,
        font_descriptor: &'a str,
        text: &'a str,
    }

    impl Hash for LegacyTextMeasureLookupKey<'_> {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.hash.hash(state);
        }
    }

    impl indexmap::Equivalent<TextMeasureCacheKey> for LegacyTextMeasureLookupKey<'_> {
        fn equivalent(&self, key: &TextMeasureCacheKey) -> bool {
            self.hash == key.text_hash
                && self.font_descriptor == key.font_descriptor
                && self.text == key.text
        }
    }

    impl LegacyTextMeasureCache {
        fn new(capacity: usize) -> Self {
            Self {
                entries: indexmap::IndexMap::with_capacity(capacity),
                capacity,
            }
        }

        fn get_fast(&mut self, font_descriptor: &str, text: &str) -> Option<TextMeasureResult> {
            let lookup = LegacyTextMeasureLookupKey {
                hash: TextMeasureCacheKey::lookup_hash(font_descriptor, text),
                font_descriptor,
                text,
            };
            let (lookup_index, _, &value) = self.entries.get_full(&lookup)?;
            let last_idx = self.entries.len() - 1;
            if lookup_index != last_idx {
                self.entries.move_index(lookup_index, last_idx);
            }
            Some(value)
        }

        fn insert(&mut self, key: TextMeasureCacheKey, result: TextMeasureResult) {
            if self.entries.contains_key(&key) {
                self.entries.shift_remove(&key);
            } else if self.entries.len() >= self.capacity {
                self.entries.shift_remove_index(0);
            }
            self.entries.insert(key, result);
        }
    }

    #[test]
    fn test_cache_key_equality() {
        let key1 = TextMeasureCacheKey::new("12pt Arial", "hello");
        let key2 = TextMeasureCacheKey::new("12pt Arial", "hello");
        let key3 = TextMeasureCacheKey::new("14pt Arial", "hello");
        let key4 = TextMeasureCacheKey::new("12pt Arial", "world");

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);
    }

    #[test]
    fn test_cache_key_hash() {
        let key1 = TextMeasureCacheKey::new("12pt Arial", "hello");
        let key2 = TextMeasureCacheKey::new("12pt Arial", "hello");

        let mut hasher1 = FxHasher::default();
        let mut hasher2 = FxHasher::default();
        key1.hash(&mut hasher1);
        key2.hash(&mut hasher2);

        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = TextMeasureCache::new(100);
        let key = TextMeasureCacheKey::new("12pt Arial", "hello");
        let result = TextMeasureResult::new(42.5);

        assert!(cache.get(&key).is_none());

        cache.insert(key.clone(), result);

        let cached = cache.get(&key);
        assert!(cached.is_some());
        assert!((cached.unwrap().width - 42.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_get_fast_hit_and_miss() {
        let mut cache = TextMeasureCache::new(100);
        cache.insert(
            TextMeasureCacheKey::new("12pt Arial", "hello"),
            TextMeasureResult::new(42.5),
        );

        assert_eq!(
            cache.get_fast("12pt Arial", "hello"),
            Some(TextMeasureResult::new(42.5))
        );
        assert_eq!(cache.get_fast("12pt Arial", "world"), None);

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_cache_get_fast_updates_lru_order() {
        let mut cache = TextMeasureCache::new(2);
        cache.insert(
            TextMeasureCacheKey::new("12pt Arial", "a"),
            TextMeasureResult::new(10.0),
        );
        cache.insert(
            TextMeasureCacheKey::new("12pt Arial", "b"),
            TextMeasureResult::new(20.0),
        );

        // Make "a" the most recently used entry.
        assert_eq!(
            cache.get_fast("12pt Arial", "a"),
            Some(TextMeasureResult::new(10.0))
        );

        // Inserting "c" should evict "b" (the least recently used one).
        cache.insert(
            TextMeasureCacheKey::new("12pt Arial", "c"),
            TextMeasureResult::new(30.0),
        );

        assert_eq!(
            cache.get_fast("12pt Arial", "a"),
            Some(TextMeasureResult::new(10.0))
        );
        assert_eq!(cache.get_fast("12pt Arial", "b"), None);
        assert_eq!(
            cache.get_fast("12pt Arial", "c"),
            Some(TextMeasureResult::new(30.0))
        );
    }

    #[test]
    fn test_cache_get_or_insert_with() {
        let mut cache = TextMeasureCache::new(100);
        let key = TextMeasureCacheKey::new("12pt Arial", "hello");

        let mut call_count = 0;
        let result = cache.get_or_insert_with(key.clone(), || {
            call_count += 1;
            TextMeasureResult::new(50.0)
        });
        assert!((result.width - 50.0).abs() < f64::EPSILON);
        assert_eq!(call_count, 1);

        // Second call should use cache
        let result2 = cache.get_or_insert_with(key.clone(), || {
            call_count += 1;
            TextMeasureResult::new(99.0)
        });
        assert!((result2.width - 50.0).abs() < f64::EPSILON);
        assert_eq!(call_count, 1); // Should not have called closure again
    }

    #[test]
    fn test_cache_lru_eviction() {
        let mut cache = TextMeasureCache::new(3);

        let key1 = TextMeasureCacheKey::new("12pt Arial", "a");
        let key2 = TextMeasureCacheKey::new("12pt Arial", "b");
        let key3 = TextMeasureCacheKey::new("12pt Arial", "c");
        let key4 = TextMeasureCacheKey::new("12pt Arial", "d");

        cache.insert(key1.clone(), TextMeasureResult::new(1.0));
        cache.insert(key2.clone(), TextMeasureResult::new(2.0));
        cache.insert(key3.clone(), TextMeasureResult::new(3.0));

        assert_eq!(cache.len(), 3);

        // Access key1 to make it more recently used
        cache.get(&key1);

        // Insert key4, which should evict key2 (least recently used)
        cache.insert(key4.clone(), TextMeasureResult::new(4.0));

        assert_eq!(cache.len(), 3);
        assert!(cache.get(&key1).is_some()); // Still present (was accessed)
        assert!(cache.get(&key2).is_none()); // Evicted (LRU)
        assert!(cache.get(&key3).is_some()); // Still present
        assert!(cache.get(&key4).is_some()); // Just inserted
    }

    #[test]
    fn test_cache_lru_eviction_order() {
        // More detailed test to verify LRU order is maintained correctly
        let mut cache = TextMeasureCache::new(3);

        let key_a = TextMeasureCacheKey::new("12pt Arial", "a");
        let key_b = TextMeasureCacheKey::new("12pt Arial", "b");
        let key_c = TextMeasureCacheKey::new("12pt Arial", "c");
        let key_d = TextMeasureCacheKey::new("12pt Arial", "d");
        let key_e = TextMeasureCacheKey::new("12pt Arial", "e");

        // Insert a, b, c (order front→back: a, b, c)
        cache.insert(key_a.clone(), TextMeasureResult::new(1.0));
        cache.insert(key_b.clone(), TextMeasureResult::new(2.0));
        cache.insert(key_c.clone(), TextMeasureResult::new(3.0));

        // Access b → moves to back (order: a, c, b)
        cache.get(&key_b);

        // Access a → moves to back (order: c, b, a)
        cache.get(&key_a);

        // Insert d → evict c (LRU at front), order becomes: b, a, d
        cache.insert(key_d.clone(), TextMeasureResult::new(4.0));

        assert!(cache.get(&key_c).is_none()); // c was evicted
                                              // Note: These get() calls also update the order!
                                              // After get(a): order is b, d, a
                                              // After get(b): order is d, a, b
                                              // After get(d): order is a, b, d
        assert!(cache.get(&key_a).is_some());
        assert!(cache.get(&key_b).is_some());
        assert!(cache.get(&key_d).is_some());

        // Now order is: a, b, d (a is LRU due to the get() calls above)
        // Insert e → should evict a (LRU at front)
        cache.insert(key_e.clone(), TextMeasureResult::new(5.0));

        assert!(cache.get(&key_a).is_none()); // a was evicted (was LRU)
        assert!(cache.get(&key_b).is_some());
        assert!(cache.get(&key_d).is_some());
        assert!(cache.get(&key_e).is_some());
    }

    #[test]
    fn test_cache_update_existing_key() {
        let mut cache = TextMeasureCache::new(3);

        let key = TextMeasureCacheKey::new("12pt Arial", "hello");

        cache.insert(key.clone(), TextMeasureResult::new(10.0));
        assert!((cache.get(&key).unwrap().width - 10.0).abs() < f64::EPSILON);

        // Update with new value
        cache.insert(key.clone(), TextMeasureResult::new(20.0));
        assert!((cache.get(&key).unwrap().width - 20.0).abs() < f64::EPSILON);

        // Should still be only one entry
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_stats() {
        let mut cache = TextMeasureCache::new(100);
        let key1 = TextMeasureCacheKey::new("12pt Arial", "hello");
        let key2 = TextMeasureCacheKey::new("12pt Arial", "world");

        // Miss
        cache.get(&key1);
        cache.get(&key2);

        // Insert one
        cache.insert(key1.clone(), TextMeasureResult::new(10.0));

        // Hit
        cache.get(&key1);
        cache.get(&key1);

        // Miss again
        cache.get(&key2);

        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 3);
        assert_eq!(stats.size, 1);
        assert_eq!(stats.capacity, 100);
        assert!((stats.hit_rate() - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = TextMeasureCache::new(100);
        let key = TextMeasureCacheKey::new("12pt Arial", "hello");

        cache.insert(key.clone(), TextMeasureResult::new(10.0));
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn test_cache_hit_rate_edge_cases() {
        let cache = TextMeasureCache::new(100);
        let stats = cache.stats();

        // No accesses yet
        assert!((stats.hit_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_zero_capacity_is_noop() {
        let mut cache = TextMeasureCache::new(0);
        let key = TextMeasureCacheKey::new("12pt Arial", "hello");
        cache.insert(key.clone(), TextMeasureResult::new(10.0));
        assert!(cache.is_empty());
        assert_eq!(cache.get(&key), None);
    }

    #[test]
    fn test_different_fonts_different_cache_entries() {
        let mut cache = TextMeasureCache::new(100);

        let key_arial = TextMeasureCacheKey::new("12pt Arial", "hello");
        let key_times = TextMeasureCacheKey::new("12pt 'Times New Roman'", "hello");

        cache.insert(key_arial.clone(), TextMeasureResult::new(30.0));
        cache.insert(key_times.clone(), TextMeasureResult::new(28.0));

        assert_eq!(cache.len(), 2);
        assert!((cache.get(&key_arial).unwrap().width - 30.0).abs() < f64::EPSILON);
        assert!((cache.get(&key_times).unwrap().width - 28.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_same_text_different_sizes() {
        let mut cache = TextMeasureCache::new(100);

        let key_12pt = TextMeasureCacheKey::new("12pt Arial", "hello");
        let key_14pt = TextMeasureCacheKey::new("14pt Arial", "hello");

        cache.insert(key_12pt.clone(), TextMeasureResult::new(30.0));
        cache.insert(key_14pt.clone(), TextMeasureResult::new(35.0));

        assert_eq!(cache.len(), 2);
        assert!((cache.get(&key_12pt).unwrap().width - 30.0).abs() < f64::EPSILON);
        assert!((cache.get(&key_14pt).unwrap().width - 35.0).abs() < f64::EPSILON);
    }

    // ---- P1-1 / P1-3: font-descriptor interning + allocation-free lookups ----

    #[test]
    fn distinct_descriptors_same_text_never_collide() {
        // Two different descriptor strings with identical text must stay distinct
        // entries even though the bucket hash is now derived from an interned id.
        let mut cache = TextMeasureCache::new(100);
        let desc_a = "bold 10.5pt \"MS Mincho\", serif".to_string();
        let desc_b = "10.5pt \"MS Mincho\", serif".to_string();

        assert_eq!(cache.get_fast(&desc_a, "同"), None); // miss
        cache.insert(
            TextMeasureCacheKey::from_refs(&desc_a, "同"),
            TextMeasureResult::new(11.0),
        );
        assert_eq!(cache.get_fast(&desc_b, "同"), None); // miss: different descriptor
        cache.insert(
            TextMeasureCacheKey::from_refs(&desc_b, "同"),
            TextMeasureResult::new(9.0),
        );

        assert_eq!(cache.len(), 2);
        assert_eq!(
            cache.get_fast(&desc_a, "同"),
            Some(TextMeasureResult::new(11.0))
        );
        assert_eq!(
            cache.get_fast(&desc_b, "同"),
            Some(TextMeasureResult::new(9.0))
        );
    }

    #[test]
    fn get_fast_does_not_insert_on_miss() {
        let mut cache = TextMeasureCache::new(100);
        assert_eq!(cache.get_fast("12pt Arial", "hello"), None);
        assert_eq!(cache.len(), 0);
        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn interning_returns_stable_ids() {
        let mut cache = TextMeasureCache::new(100);
        // Owned copies force distinct allocations so the pointer fast path does
        // not mask a wrong-id bug.
        let a1 = "12pt Arial".to_string();
        let b1 = "12pt 'Times New Roman'".to_string();
        let a2 = "12pt Arial".to_string();
        let b2 = "12pt 'Times New Roman'".to_string();

        let id_a1 = cache.intern_descriptor(&a1);
        let id_b1 = cache.intern_descriptor(&b1);
        let id_a2 = cache.intern_descriptor(&a2);
        let id_b2 = cache.intern_descriptor(&b2);

        assert_ne!(id_a1, id_b1);
        assert_eq!(
            id_a1, id_a2,
            "same descriptor content must map to a stable id"
        );
        assert_eq!(id_b1, id_b2);
    }

    #[test]
    fn hit_behavior_matches_across_two_distinct_descriptors_with_same_text() {
        // (a) requirement: hit/miss behavior identical across two distinct
        // descriptors sharing the same text.
        let mut cache = TextMeasureCache::new(100);
        cache.insert(
            TextMeasureCacheKey::new("12pt Arial", "hello"),
            TextMeasureResult::new(30.0),
        );
        cache.insert(
            TextMeasureCacheKey::new("12pt 'Times New Roman'", "hello"),
            TextMeasureResult::new(40.0),
        );

        // Each descriptor resolves to its own value; the other text is a miss.
        assert_eq!(
            cache.get_fast("12pt Arial", "hello"),
            Some(TextMeasureResult::new(30.0))
        );
        assert_eq!(
            cache.get_fast("12pt 'Times New Roman'", "hello"),
            Some(TextMeasureResult::new(40.0))
        );
        assert_eq!(cache.get_fast("12pt Arial", "world"), None);
        assert_eq!(cache.get_fast("14pt Arial", "hello"), None);
    }

    #[test]
    fn repeated_same_descriptor_lookups_use_pointer_fast_path() {
        // Consecutive lookups reusing the same descriptor &str allocation should
        // resolve without repopulating the interning table.
        let mut cache = TextMeasureCache::new(100);
        let desc = "12pt Arial".to_string();
        for text in ["a", "b", "c"] {
            cache.insert(
                TextMeasureCacheKey::from_refs(&desc, text),
                TextMeasureResult::new(1.0),
            );
        }
        // Only one descriptor was ever interned.
        assert_eq!(cache.interner.borrow().interned_count(), 1);
        for text in ["a", "b", "c"] {
            assert_eq!(
                cache.get_fast(&desc, text),
                Some(TextMeasureResult::new(1.0))
            );
        }
        assert_eq!(cache.interner.borrow().interned_count(), 1);
    }

    #[test]
    fn pointer_fast_path_does_not_mis_resolve_after_backing_string_changes() {
        // (c) requirement: intern A, then a DIFFERENT descriptor at a fresh
        // allocation, then A again. The `last_descriptor` pointer cache must not
        // let the intervening descriptor's id leak into A's second resolution,
        // and the byte-comparison guard must reject any (ptr,len) alias.
        let mut cache = TextMeasureCache::new(100);
        cache.insert(
            TextMeasureCacheKey::new("12pt Arial", "x"),
            TextMeasureResult::new(10.0),
        );
        cache.insert(
            TextMeasureCacheKey::new("12pt 'Times New Roman'", "x"),
            TextMeasureResult::new(20.0),
        );

        // Distinct owned allocations for each lookup so the pointer fast path is
        // exercised across genuinely different backing strings.
        let a1 = "12pt Arial".to_string();
        let b = "12pt 'Times New Roman'".to_string();
        let a2 = "12pt Arial".to_string();

        let id_a1 = cache.intern_descriptor(&a1);
        let id_b = cache.intern_descriptor(&b);
        let id_a2 = cache.intern_descriptor(&a2);
        assert_ne!(id_a1, id_b);
        assert_eq!(id_a1, id_a2, "A must resolve to the same id after B");

        // End-to-end: each descriptor still maps to its own cached value.
        assert_eq!(cache.get_fast(&a1, "x"), Some(TextMeasureResult::new(10.0)));
        assert_eq!(cache.get_fast(&b, "x"), Some(TextMeasureResult::new(20.0)));
        assert_eq!(cache.get_fast(&a2, "x"), Some(TextMeasureResult::new(10.0)));
        // No new descriptors were interned by the value lookups above.
        assert_eq!(cache.interner.borrow().interned_count(), 2);
    }

    // ---- S4: interner extraction + hash-keyed / id-keyed lookups ----

    struct NoopMeasurer;
    impl TextMeasurer for NoopMeasurer {
        type Error = ();
        fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
            Ok(text.chars().count() as f64)
        }
    }

    #[test]
    fn font_descriptor_interner_is_stable_and_collision_free() {
        let mut interner = FontDescriptorInterner::new();
        let a = "12pt Arial".to_string();
        let b = "12pt 'Times New Roman'".to_string();
        let a2 = "12pt Arial".to_string();
        let id_a = interner.intern(&a);
        let id_b = interner.intern(&b);
        // Distinct allocation, same content → same id (byte-keyed map).
        assert_eq!(id_a, interner.intern(&a2));
        assert_ne!(id_a, id_b);
        assert_eq!(interner.descriptor_for_id(id_a), Some("12pt Arial"));
        assert_eq!(interner.interned_count(), 2);
    }

    #[test]
    fn intern_with_hash_matches_byte_intern_and_stays_stable() {
        let mut interner = FontDescriptorInterner::new();
        let a = "12pt Arial".to_string();
        let b = "12pt 'Times New Roman'".to_string();
        let ha = hash_text(&a);
        let hb = hash_text(&b);
        // First sighting via byte intern, then hash intern must return the SAME id.
        let id_a = interner.intern(&a);
        assert_eq!(interner.intern_with_hash(&a, ha), id_a);
        // A brand-new descriptor first seen via the hash path also interns cleanly.
        let id_b = interner.intern_with_hash(&b, hb);
        assert_ne!(id_a, id_b);
        assert_eq!(interner.intern(&b), id_b);
        // Reusing an owned copy (fresh allocation) with the same hash is stable.
        let a2 = "12pt Arial".to_string();
        assert_eq!(interner.intern_with_hash(&a2, ha), id_a);
    }

    #[test]
    fn hash_and_id_lookups_agree_with_descriptor_lookups() {
        let mut cache = TextMeasureCache::new(100);
        let desc_a = "bold 12.00pt Arial".to_string();
        let desc_b = "12.00pt 'MS Mincho'".to_string();
        let ha = hash_text(&desc_a);
        let hb = hash_text(&desc_b);

        // Populate via the id-keyed measure path (miss → insert).
        let id_a = cache.interner().borrow_mut().intern(&desc_a);
        assert_eq!(
            cache
                .measure_with_id(&NoopMeasurer, "hi", &desc_a, id_a)
                .unwrap(),
            2.0
        );
        // A hash-keyed lookup for the SAME descriptor+text must hit the same node.
        assert_eq!(
            cache.get_fast_with_hash(&desc_a, "hi", ha),
            Some(TextMeasureResult::new(2.0))
        );
        // The plain descriptor-keyed lookup must agree too.
        assert_eq!(
            cache.get_fast(&desc_a, "hi"),
            Some(TextMeasureResult::new(2.0))
        );
        // A different descriptor with the same text is a distinct miss.
        assert_eq!(cache.get_fast_with_hash(&desc_b, "hi", hb), None);
    }

    #[test]
    fn measure_with_hash_falls_back_to_measurer_on_miss_then_hits() {
        let mut cache = TextMeasureCache::new(100);
        let desc = "12.00pt Arial".to_string();
        let h = hash_text(&desc);
        // Miss → measurer computes width (char count = 5), inserts.
        assert_eq!(
            cache
                .measure_with_hash(&NoopMeasurer, "hello", &desc, h)
                .unwrap(),
            5.0
        );
        assert_eq!(cache.stats().misses, 1);
        // Second call hits without re-measuring.
        assert_eq!(
            cache
                .measure_with_hash(&NoopMeasurer, "hello", &desc, h)
                .unwrap(),
            5.0
        );
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn shared_interner_gives_consistent_ids_across_caches() {
        let interner = Rc::new(RefCell::new(FontDescriptorInterner::new()));
        let cache_a = TextMeasureCache::with_interner(64, Rc::clone(&interner));
        let cache_b = TextMeasureCache::with_interner(64, Rc::clone(&interner));
        let desc = "12.00pt Arial".to_string();
        let id_a = cache_a.interner().borrow_mut().intern(&desc);
        let id_b = cache_b.interner().borrow_mut().intern(&desc);
        assert_eq!(id_a, id_b, "shared interner must produce identical ids");
    }

    #[test]
    fn measure_text_with_hash_trait_default_forwards_to_measure_text() {
        // A measurer that does NOT override measure_text_with_hash must still get
        // a correct width through the default forward.
        let m = NoopMeasurer;
        assert_eq!(
            m.measure_text_with_hash("abcd", "12pt Arial", 0).unwrap(),
            4.0
        );
    }

    #[test]
    #[ignore]
    fn benchmark_text_measure_cache_lru_hot_path() {
        const CAPACITY: usize = 512;
        const UNIQUE_KEYS: usize = 4096;
        const ITERS: usize = 200_000;

        let font = "10.5pt 'Yu Mincho'";
        let keys: Vec<TextMeasureCacheKey> = (0..UNIQUE_KEYS)
            .map(|i| TextMeasureCacheKey::new(font, format!("token-{i:04}")))
            .collect();
        let mut access_pattern = Vec::with_capacity(ITERS);
        for i in 0..ITERS {
            let key_index = (i * 17) % UNIQUE_KEYS;
            access_pattern.push(key_index);
        }

        let mut legacy = LegacyTextMeasureCache::new(CAPACITY);
        let legacy_start = Instant::now();
        for &index in &access_pattern {
            let key = &keys[index];
            if legacy
                .get_fast(key.font_descriptor.as_str(), key.text.as_str())
                .is_none()
            {
                legacy.insert(key.clone(), TextMeasureResult::new((index % 97) as f64));
            }
        }
        let legacy_elapsed = legacy_start.elapsed();

        let mut current = TextMeasureCache::new(CAPACITY);
        let current_start = Instant::now();
        for &index in &access_pattern {
            let key = &keys[index];
            if current
                .get_fast(key.font_descriptor.as_str(), key.text.as_str())
                .is_none()
            {
                current.insert(key.clone(), TextMeasureResult::new((index % 97) as f64));
            }
        }
        let current_elapsed = current_start.elapsed();

        black_box((&legacy, &current));
        println!(
            "text measure cache benchmark: legacy={}us current={}us iterations={} unique_keys={} capacity={}",
            legacy_elapsed.as_micros(),
            current_elapsed.as_micros(),
            ITERS,
            UNIQUE_KEYS,
            CAPACITY,
        );
    }

    #[test]
    #[ignore]
    fn benchmark_text_measure_cache_mru_hot_path() {
        const CAPACITY: usize = 512;
        const ITERS: usize = 500_000;

        let font = "10.5pt 'Yu Mincho'";
        let key = TextMeasureCacheKey::new(font, "a");
        let mut cache = TextMeasureCache::new(CAPACITY);
        cache.insert(key.clone(), TextMeasureResult::new(10.0));

        let start = Instant::now();
        for _ in 0..ITERS {
            black_box(cache.get_fast(key.font_descriptor.as_str(), key.text.as_str()));
        }
        let elapsed = start.elapsed();

        println!(
            "text measure cache mru benchmark: current={}us iterations={}",
            elapsed.as_micros(),
            ITERS,
        );
    }

    #[test]
    #[ignore]
    fn benchmark_text_measure_cache_hash_vs_resolved_id_hot_path() {
        const ITERS: usize = 1_000_000;
        let fonts = [
            "10.5pt 'Yu Mincho'",
            "bold 10.5pt 'Yu Gothic'",
            "10.5pt Arial",
            "italic 10.5pt 'Noto Sans Arabic'",
        ];
        let texts = ["あ", "A", "漢", "。"];

        let mut hash_cache = TextMeasureCache::new(64);
        let mut id_cache = TextMeasureCache::new(64);
        for (font_index, font) in fonts.iter().enumerate() {
            for (text_index, text) in texts.iter().enumerate() {
                let result = TextMeasureResult::new((font_index * texts.len() + text_index) as f64);
                let key = TextMeasureCacheKey::new(*font, *text);
                hash_cache.insert(key.clone(), result);
                id_cache.insert(key, result);
            }
        }
        let hashes = fonts.map(hash_text);
        let ids = fonts.map(|font| id_cache.intern_descriptor(font));

        let hash_start = Instant::now();
        for i in 0..ITERS {
            let index = i & 3;
            black_box(hash_cache.get_fast_with_hash(fonts[index], texts[index], hashes[index]));
        }
        let hash_elapsed = hash_start.elapsed();

        let id_start = Instant::now();
        for i in 0..ITERS {
            let index = i & 3;
            black_box(id_cache.get_by_font_id(ids[index], texts[index], fonts[index]));
        }
        let id_elapsed = id_start.elapsed();

        println!(
            "text measure cache resolved-id benchmark: hash={}us id={}us iterations={}",
            hash_elapsed.as_micros(),
            id_elapsed.as_micros(),
            ITERS,
        );
    }

    #[test]
    #[ignore]
    fn benchmark_text_measure_cache_mru_vs_previous_current_hot_path() {
        fn previous_get_fast(
            cache: &mut TextMeasureCache,
            font_descriptor: &str,
            text: &str,
        ) -> Option<TextMeasureResult> {
            let hash = TextMeasureCacheKey::lookup_hash(font_descriptor, text);
            if let Some(index) = cache.buckets.get(&hash).and_then(|indices| {
                indices.iter().copied().find(|&index| {
                    cache
                        .node(index)
                        .map(|node| {
                            node.key.font_descriptor == font_descriptor && node.key.text == text
                        })
                        .unwrap_or(false)
                })
            }) {
                if cache.tail != Some(index) {
                    cache.detach(index);
                    cache.attach_to_tail(index);
                }
                cache.hits += 1;
                return cache.node(index).map(|node| node.result);
            }

            cache.misses += 1;
            None
        }

        const CAPACITY: usize = 512;
        const ITERS: usize = 500_000;

        let font = "10.5pt 'Yu Mincho'";
        let key = TextMeasureCacheKey::new(font, "a");

        let mut previous = TextMeasureCache::new(CAPACITY);
        previous.insert(key.clone(), TextMeasureResult::new(10.0));
        let previous_start = Instant::now();
        for _ in 0..ITERS {
            black_box(previous_get_fast(
                &mut previous,
                key.font_descriptor.as_str(),
                key.text.as_str(),
            ));
        }
        let previous_elapsed = previous_start.elapsed();

        let mut current = TextMeasureCache::new(CAPACITY);
        current.insert(key.clone(), TextMeasureResult::new(10.0));
        let current_start = Instant::now();
        for _ in 0..ITERS {
            black_box(current.get_fast(key.font_descriptor.as_str(), key.text.as_str()));
        }
        let current_elapsed = current_start.elapsed();

        println!(
            "text measure cache mru compare benchmark: previous_current={}us current={}us iterations={}",
            previous_elapsed.as_micros(),
            current_elapsed.as_micros(),
            ITERS,
        );
    }

    #[test]
    #[ignore]
    fn benchmark_text_measure_cache_recent_pair_hot_path() {
        fn previous_get_fast(
            cache: &mut TextMeasureCache,
            font_descriptor: &str,
            text: &str,
        ) -> Option<TextMeasureResult> {
            let font_id = cache.intern_descriptor(font_descriptor);
            if let Some((index, result)) = cache.last_lookup_result(font_id, text) {
                cache.touch(index);
                cache.last_lookup_index = Some(index);
                cache.hits += 1;
                return Some(result);
            }

            let hash = mix_font_id(font_id, hash_text(text));
            if let Some(index) = cache.find_index_by_parts(hash, font_id, text) {
                cache.touch(index);
                cache.last_lookup_index = Some(index);
                cache.hits += 1;
                return cache.node(index).map(|node| node.result);
            }

            cache.misses += 1;
            None
        }

        const CAPACITY: usize = 512;
        const ITERS: usize = 500_000;

        let font = "10.5pt 'Yu Mincho'";
        let key_a = TextMeasureCacheKey::new(font, "a");
        let key_b = TextMeasureCacheKey::new(font, "ab");

        let mut previous = TextMeasureCache::new(CAPACITY);
        previous.insert(key_a.clone(), TextMeasureResult::new(10.0));
        previous.insert(key_b.clone(), TextMeasureResult::new(15.0));
        let previous_start = Instant::now();
        for i in 0..ITERS {
            let key = if i % 2 == 0 { &key_a } else { &key_b };
            black_box(previous_get_fast(
                &mut previous,
                key.font_descriptor.as_str(),
                key.text.as_str(),
            ));
        }
        let previous_elapsed = previous_start.elapsed();

        let mut current = TextMeasureCache::new(CAPACITY);
        current.insert(key_a.clone(), TextMeasureResult::new(10.0));
        current.insert(key_b.clone(), TextMeasureResult::new(15.0));
        let current_start = Instant::now();
        for i in 0..ITERS {
            let key = if i % 2 == 0 { &key_a } else { &key_b };
            black_box(current.get_fast(key.font_descriptor.as_str(), key.text.as_str()));
        }
        let current_elapsed = current_start.elapsed();

        println!(
            "text measure cache recent pair benchmark: previous_current={}us current={}us iterations={}",
            previous_elapsed.as_micros(),
            current_elapsed.as_micros(),
            ITERS,
        );
    }

    #[test]
    fn test_text_measure_cache_recent_pair_fast_path() {
        let font = "10.5pt 'Yu Mincho'";
        let key_a = TextMeasureCacheKey::new(font, "a");
        let key_b = TextMeasureCacheKey::new(font, "ab");

        let mut cache = TextMeasureCache::new(8);
        cache.insert(key_a.clone(), TextMeasureResult::new(10.0));
        cache.insert(key_b.clone(), TextMeasureResult::new(15.0));

        assert_eq!(
            cache.get_fast(key_a.font_descriptor.as_str(), key_a.text.as_str()),
            Some(TextMeasureResult::new(10.0))
        );
        assert_eq!(
            cache.get_fast(key_b.font_descriptor.as_str(), key_b.text.as_str()),
            Some(TextMeasureResult::new(15.0))
        );
        assert_eq!(
            cache.get_fast(key_a.font_descriptor.as_str(), key_a.text.as_str()),
            Some(TextMeasureResult::new(10.0))
        );
        assert!(cache.last_lookup_index.is_some());
        assert!(cache.second_last_lookup_index.is_some());
    }

    // =========================================================================
    // ParagraphLayoutCache Tests
    // =========================================================================

    #[test]
    fn test_content_hash_equality() {
        let hash1 = compute_content_hash(&"hello world");
        let hash2 = compute_content_hash(&"hello world");
        let hash3 = compute_content_hash(&"hello world!");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_content_hash_different_types() {
        // Same string content should produce same hash
        let hash_str = compute_content_hash(&"test");
        let hash_string = compute_content_hash(&String::from("test"));
        // Note: These may differ because &str and String have different Hash implementations
        // This test verifies the behavior
        assert_eq!(hash_str.value(), hash_string.value());
    }

    #[test]
    fn test_paragraph_layout_result_creation() {
        let hash = compute_content_hash(&"paragraph content");
        let result =
            ParagraphLayoutResult::new(hash, 100.0, 150.0, 0, 0, 0.0, 0.0, 0, 0, 90.0, 140.0);

        assert_eq!(result.content_hash, hash);
        assert!((result.start_y - 100.0).abs() < f64::EPSILON);
        assert!((result.end_y - 150.0).abs() < f64::EPSILON);
        assert!((result.height - 50.0).abs() < f64::EPSILON);
        assert_eq!(result.start_page, 0);
        assert_eq!(result.end_page, 0);
    }

    #[test]
    fn test_paragraph_layout_cache_empty() {
        let cache = ParagraphLayoutCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_paragraph_layout_cache_insert_and_get() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"test paragraph");
        let result = ParagraphLayoutResult::new(hash, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0);

        cache.insert(0, result.clone());

        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());

        // Should return Some when hash and position match
        let cached = cache.get_if_valid(0, hash, 0.0);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().content_hash, hash);
    }

    #[test]
    fn test_paragraph_layout_cache_miss_on_hash_change() {
        let mut cache = ParagraphLayoutCache::new();
        let hash1 = compute_content_hash(&"original content");
        let hash2 = compute_content_hash(&"modified content");
        let result = ParagraphLayoutResult::new(hash1, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0);

        cache.insert(0, result);

        // Should return None when content hash doesn't match
        let cached = cache.get_if_valid(0, hash2, 0.0);
        assert!(cached.is_none());

        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_paragraph_layout_cache_miss_on_position_change() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"test paragraph");
        let result = ParagraphLayoutResult::new(hash, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0);

        cache.insert(0, result);

        // Should return None when expected position doesn't match
        let cached = cache.get_if_valid(0, hash, 10.0);
        assert!(cached.is_none());
    }

    #[test]
    fn test_paragraph_layout_cache_multiple_paragraphs() {
        let mut cache = ParagraphLayoutCache::new();

        let hash0 = compute_content_hash(&"paragraph 0");
        let hash1 = compute_content_hash(&"paragraph 1");
        let hash2 = compute_content_hash(&"paragraph 2");

        cache.insert(
            0,
            ParagraphLayoutResult::new(hash0, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );
        cache.insert(
            1,
            ParagraphLayoutResult::new(hash1, 50.0, 100.0, 0, 0, 0.0, 0.0, 0, 0, 50.0, 100.0),
        );
        cache.insert(
            2,
            ParagraphLayoutResult::new(hash2, 100.0, 150.0, 0, 0, 0.0, 0.0, 0, 0, 100.0, 150.0),
        );

        assert_eq!(cache.len(), 3);

        // All should be valid
        assert!(cache.get_if_valid(0, hash0, 0.0).is_some());
        assert!(cache.get_if_valid(1, hash1, 50.0).is_some());
        assert!(cache.get_if_valid(2, hash2, 100.0).is_some());
    }

    #[test]
    fn test_paragraph_layout_cache_invalidate_from() {
        let mut cache = ParagraphLayoutCache::new();

        let hash0 = compute_content_hash(&"paragraph 0");
        let hash1 = compute_content_hash(&"paragraph 1");
        let hash2 = compute_content_hash(&"paragraph 2");

        cache.insert(
            0,
            ParagraphLayoutResult::new(hash0, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );
        cache.insert(
            1,
            ParagraphLayoutResult::new(hash1, 50.0, 100.0, 0, 0, 0.0, 0.0, 0, 0, 50.0, 100.0),
        );
        cache.insert(
            2,
            ParagraphLayoutResult::new(hash2, 100.0, 150.0, 0, 0, 0.0, 0.0, 0, 0, 100.0, 150.0),
        );

        // Invalidate from paragraph 1 onwards
        cache.invalidate_from(1);

        // Paragraph 0 should still be cached
        assert!(cache.get_if_valid(0, hash0, 0.0).is_some());
        // Paragraphs 1 and 2 should be invalidated
        assert!(cache.get_if_valid(1, hash1, 50.0).is_none());
        assert!(cache.get_if_valid(2, hash2, 100.0).is_none());

        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_paragraph_layout_cache_clear() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"test");
        cache.insert(
            0,
            ParagraphLayoutResult::new(hash, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );
        cache.insert(
            1,
            ParagraphLayoutResult::new(hash, 50.0, 100.0, 0, 0, 0.0, 0.0, 0, 0, 50.0, 100.0),
        );

        assert_eq!(cache.len(), 2);

        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_paragraph_layout_cache_apply_y_offset_success() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"para");
        cache.insert(
            0,
            ParagraphLayoutResult::new(hash, 10.0, 30.0, 0, 0, 0.0, 0.0, 0, 0, 10.0, 30.0),
        );
        cache.insert(
            1,
            ParagraphLayoutResult::new(hash, 30.0, 60.0, 0, 0, 0.0, 0.0, 0, 0, 30.0, 60.0),
        );
        cache.insert(
            2,
            ParagraphLayoutResult::new(hash, 60.0, 90.0, 0, 0, 0.0, 0.0, 0, 0, 60.0, 90.0),
        );

        let applied = cache.try_apply_y_offset_from(1, 15.0, 200.0, 10.0, 10.0);
        assert!(applied);

        let entry0 = cache.get_entry(0).unwrap();
        let entry1 = cache.get_entry(1).unwrap();
        let entry2 = cache.get_entry(2).unwrap();

        assert!((entry0.start_y - 10.0).abs() < f64::EPSILON);
        assert!((entry1.start_y - 45.0).abs() < f64::EPSILON);
        assert!((entry1.end_y - 75.0).abs() < f64::EPSILON);
        assert!((entry2.start_y - 75.0).abs() < f64::EPSILON);
        assert!((entry2.end_y - 105.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_paragraph_layout_cache_apply_y_offset_fails_on_page_boundary() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"para");
        cache.insert(
            0,
            ParagraphLayoutResult::new(hash, 70.0, 85.0, 0, 0, 0.0, 0.0, 0, 0, 70.0, 85.0),
        );

        let applied = cache.try_apply_y_offset_from(0, 10.0, 100.0, 10.0, 10.0);
        assert!(!applied);

        let entry = cache.get_entry(0).unwrap();
        assert!((entry.start_y - 70.0).abs() < f64::EPSILON);
        assert!((entry.end_y - 85.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_paragraph_layout_cache_apply_y_offset_fails_on_page_spanning() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"para");
        cache.insert(
            0,
            ParagraphLayoutResult::new(hash, 80.0, 120.0, 0, 1, 0.0, 200.0, 0, 0, 80.0, 120.0),
        );

        let applied = cache.try_apply_y_offset_from(0, 5.0, 200.0, 10.0, 10.0);
        assert!(!applied);
    }

    #[test]
    fn test_paragraph_layout_cache_apply_y_offset_fails_on_column_spanning() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"para");
        cache.insert(
            0,
            ParagraphLayoutResult::new(hash, 20.0, 40.0, 0, 0, 0.0, 0.0, 0, 1, 20.0, 40.0),
        );

        let applied = cache.try_apply_y_offset_from(0, 5.0, 200.0, 10.0, 10.0);
        assert!(!applied);
    }

    #[test]
    fn test_find_first_dirty_paragraph_all_clean() {
        let mut cache = ParagraphLayoutCache::new();

        let hash0 = compute_content_hash(&"paragraph 0");
        let hash1 = compute_content_hash(&"paragraph 1");
        let hash2 = compute_content_hash(&"paragraph 2");

        cache.insert(
            0,
            ParagraphLayoutResult::new(hash0, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );
        cache.insert(
            1,
            ParagraphLayoutResult::new(hash1, 50.0, 100.0, 0, 0, 0.0, 0.0, 0, 0, 50.0, 100.0),
        );
        cache.insert(
            2,
            ParagraphLayoutResult::new(hash2, 100.0, 150.0, 0, 0, 0.0, 0.0, 0, 0, 100.0, 150.0),
        );

        let hashes = vec![hash0, hash1, hash2];
        let dirty_index = cache.find_first_dirty_paragraph(&hashes);

        // All paragraphs match, so dirty_index should be equal to length
        assert_eq!(dirty_index, 3);
    }

    #[test]
    fn test_find_first_dirty_paragraph_first_changed() {
        let mut cache = ParagraphLayoutCache::new();

        let hash0 = compute_content_hash(&"paragraph 0");
        let hash1 = compute_content_hash(&"paragraph 1");
        let hash0_new = compute_content_hash(&"paragraph 0 modified");

        cache.insert(
            0,
            ParagraphLayoutResult::new(hash0, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );
        cache.insert(
            1,
            ParagraphLayoutResult::new(hash1, 50.0, 100.0, 0, 0, 0.0, 0.0, 0, 0, 50.0, 100.0),
        );

        let hashes = vec![hash0_new, hash1];
        let dirty_index = cache.find_first_dirty_paragraph(&hashes);

        // First paragraph changed
        assert_eq!(dirty_index, 0);
    }

    #[test]
    fn test_find_first_dirty_paragraph_middle_changed() {
        let mut cache = ParagraphLayoutCache::new();

        let hash0 = compute_content_hash(&"paragraph 0");
        let hash1 = compute_content_hash(&"paragraph 1");
        let hash2 = compute_content_hash(&"paragraph 2");
        let hash1_new = compute_content_hash(&"paragraph 1 modified");

        cache.insert(
            0,
            ParagraphLayoutResult::new(hash0, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );
        cache.insert(
            1,
            ParagraphLayoutResult::new(hash1, 50.0, 100.0, 0, 0, 0.0, 0.0, 0, 0, 50.0, 100.0),
        );
        cache.insert(
            2,
            ParagraphLayoutResult::new(hash2, 100.0, 150.0, 0, 0, 0.0, 0.0, 0, 0, 100.0, 150.0),
        );

        let hashes = vec![hash0, hash1_new, hash2];
        let dirty_index = cache.find_first_dirty_paragraph(&hashes);

        // Second paragraph (index 1) changed
        assert_eq!(dirty_index, 1);
    }

    #[test]
    fn test_find_first_dirty_paragraph_new_paragraph_added() {
        let mut cache = ParagraphLayoutCache::new();

        let hash0 = compute_content_hash(&"paragraph 0");
        let hash1 = compute_content_hash(&"paragraph 1");
        let hash2 = compute_content_hash(&"new paragraph");

        cache.insert(
            0,
            ParagraphLayoutResult::new(hash0, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );
        cache.insert(
            1,
            ParagraphLayoutResult::new(hash1, 50.0, 100.0, 0, 0, 0.0, 0.0, 0, 0, 50.0, 100.0),
        );

        // New paragraph added
        let hashes = vec![hash0, hash1, hash2];
        let dirty_index = cache.find_first_dirty_paragraph(&hashes);

        // New paragraph at index 2
        assert_eq!(dirty_index, 2);
    }

    #[test]
    fn test_find_first_dirty_paragraph_empty_cache() {
        let cache = ParagraphLayoutCache::new();

        let hash0 = compute_content_hash(&"paragraph 0");
        let hashes = vec![hash0];
        let dirty_index = cache.find_first_dirty_paragraph(&hashes);

        // Empty cache means start from beginning
        assert_eq!(dirty_index, 0);
    }

    #[test]
    fn test_paragraph_layout_cache_stats() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"test");
        let wrong_hash = compute_content_hash(&"different");

        cache.insert(
            0,
            ParagraphLayoutResult::new(hash, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );

        // Hit
        cache.get_if_valid(0, hash, 0.0);
        // Miss (wrong hash)
        cache.get_if_valid(0, wrong_hash, 0.0);
        // Miss (index out of range)
        cache.get_if_valid(1, hash, 50.0);

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.skipped_paragraphs, 1);
        assert!((stats.hit_rate() - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_paragraph_layout_cache_with_capacity() {
        let cache = ParagraphLayoutCache::with_capacity(100);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_paragraph_layout_cache_sparse_insertion() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"test");

        // Insert at index 5 without inserting 0-4
        cache.insert(
            5,
            ParagraphLayoutResult::new(hash, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );

        // Should have 1 valid entry
        assert_eq!(cache.len(), 1);
        // Should be able to retrieve it
        assert!(cache.get_if_valid(5, hash, 0.0).is_some());
        // Earlier indices should be None
        assert!(cache.get_if_valid(0, hash, 0.0).is_none());
    }

    #[test]
    fn test_paragraph_layout_cache_reset_stats() {
        let mut cache = ParagraphLayoutCache::new();
        let hash = compute_content_hash(&"test");

        cache.insert(
            0,
            ParagraphLayoutResult::new(hash, 0.0, 50.0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 50.0),
        );
        cache.get_if_valid(0, hash, 0.0);
        cache.get_if_valid(1, hash, 0.0);

        let stats_before = cache.stats();
        assert_eq!(stats_before.hits, 1);
        assert_eq!(stats_before.misses, 1);

        cache.reset_stats();

        let stats_after = cache.stats();
        assert_eq!(stats_after.hits, 0);
        assert_eq!(stats_after.misses, 0);
        assert_eq!(stats_after.skipped_paragraphs, 0);
    }

    // =========================================================================
    // ParagraphHashableProps Tests
    // =========================================================================

    #[test]
    fn test_paragraph_hash_same_text() {
        let hash1 = compute_paragraph_hash("Hello world", None);
        let hash2 = compute_paragraph_hash("Hello world", None);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_paragraph_hash_different_text() {
        let hash1 = compute_paragraph_hash("Hello world", None);
        let hash2 = compute_paragraph_hash("Hello World", None);

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_paragraph_hash_with_properties() {
        let props1 = ParagraphHashableProps::new(
            Some("center"),
            Some(12.0),
            Some(12.0),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let props2 = ParagraphHashableProps::new(
            Some("center"),
            Some(12.0),
            Some(12.0),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let hash1 = compute_paragraph_hash("Test", Some(&props1));
        let hash2 = compute_paragraph_hash("Test", Some(&props2));

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_paragraph_hash_different_alignment() {
        let props1 = ParagraphHashableProps::new(
            Some("left"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let props2 = ParagraphHashableProps::new(
            Some("center"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let hash1 = compute_paragraph_hash("Test", Some(&props1));
        let hash2 = compute_paragraph_hash("Test", Some(&props2));

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_paragraph_hash_different_spacing() {
        let props1 = ParagraphHashableProps::new(
            None,
            Some(12.0),
            Some(6.0),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let props2 = ParagraphHashableProps::new(
            None,
            Some(12.0),
            Some(12.0),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let hash1 = compute_paragraph_hash("Test", Some(&props1));
        let hash2 = compute_paragraph_hash("Test", Some(&props2));

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_paragraph_hash_different_indent() {
        let props1 = ParagraphHashableProps::new(
            None,
            None,
            None,
            None,
            Some(36.0), // left indent
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let props2 = ParagraphHashableProps::new(
            None,
            None,
            None,
            None,
            Some(72.0), // different left indent
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        let hash1 = compute_paragraph_hash("Test", Some(&props1));
        let hash2 = compute_paragraph_hash("Test", Some(&props2));

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_paragraph_hash_different_numbering() {
        let props1 = ParagraphHashableProps::new(
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("numId1"),
            Some(0),
        );
        let props2 = ParagraphHashableProps::new(
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("numId2"),
            Some(0),
        );

        let hash1 = compute_paragraph_hash("Test", Some(&props1));
        let hash2 = compute_paragraph_hash("Test", Some(&props2));

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_paragraph_hash_with_none_vs_some_props() {
        let props = ParagraphHashableProps::new(
            None, None, None, None, None, None, None, None, None, None, None, None, None,
        );

        let hash1 = compute_paragraph_hash("Test", None);
        let hash2 = compute_paragraph_hash("Test", Some(&props));

        // These should be different because one has Some(props) and one has None
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_paragraph_hashable_props_float_precision() {
        // Test that small floating point differences (within 0.01) produce the same hash
        // after rounding to int (scaled by 100)
        let props1 = ParagraphHashableProps::new(
            None,
            Some(12.0),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let props2 = ParagraphHashableProps::new(
            None,
            Some(12.00001), // Very small difference
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        // Both should round to 1200 (12.0 * 100)
        assert_eq!(props1.spacing_before, Some(1200));
        assert_eq!(props2.spacing_before, Some(1200));
    }

    #[test]
    fn test_paragraph_hash_different_character_indent() {
        let props1 = ParagraphHashableProps::new(
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(100),
            None,
            None,
            None,
            None,
        );
        let props2 = ParagraphHashableProps::new(
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(200),
            None,
            None,
            None,
            None,
        );

        let hash1 = compute_paragraph_hash("Test", Some(&props1));
        let hash2 = compute_paragraph_hash("Test", Some(&props2));

        assert_ne!(hash1, hash2);
    }

    // =========================================================================
    // DirtyRect Tests
    // =========================================================================

    #[test]
    fn test_dirty_rect_creation() {
        let rect = DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0);
        assert!((rect.x - 10.0).abs() < f64::EPSILON);
        assert!((rect.y - 20.0).abs() < f64::EPSILON);
        assert!((rect.width - 100.0).abs() < f64::EPSILON);
        assert!((rect.height - 50.0).abs() < f64::EPSILON);
        assert_eq!(rect.page, 0);
    }

    #[test]
    fn test_dirty_rect_right_bottom() {
        let rect = DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0);
        assert!((rect.right() - 110.0).abs() < f64::EPSILON);
        assert!((rect.bottom() - 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_dirty_rect_full_page() {
        let rect = DirtyRect::full_page(800.0, 600.0, 1);
        assert!((rect.x - 0.0).abs() < f64::EPSILON);
        assert!((rect.y - 0.0).abs() < f64::EPSILON);
        assert!((rect.width - 800.0).abs() < f64::EPSILON);
        assert!((rect.height - 600.0).abs() < f64::EPSILON);
        assert_eq!(rect.page, 1);
    }

    #[test]
    fn test_dirty_rect_full_page_at_uses_absolute_y() {
        let rect = DirtyRect::full_page_at(800.0, 600.0, 1, 1234.0);
        assert!((rect.x - 0.0).abs() < f64::EPSILON);
        assert!((rect.y - 1234.0).abs() < f64::EPSILON);
        assert!((rect.width - 800.0).abs() < f64::EPSILON);
        assert!((rect.height - 600.0).abs() < f64::EPSILON);
        assert_eq!(rect.page, 1);
    }

    #[test]
    fn test_dirty_rect_intersects_same_page() {
        let rect1 = DirtyRect::new(0.0, 0.0, 100.0, 100.0, 0);
        let rect2 = DirtyRect::new(50.0, 50.0, 100.0, 100.0, 0);
        assert!(rect1.intersects(&rect2));
        assert!(rect2.intersects(&rect1));
    }

    #[test]
    fn test_dirty_rect_no_intersect_different_pages() {
        let rect1 = DirtyRect::new(0.0, 0.0, 100.0, 100.0, 0);
        let rect2 = DirtyRect::new(50.0, 50.0, 100.0, 100.0, 1);
        assert!(!rect1.intersects(&rect2));
    }

    #[test]
    fn test_dirty_rect_no_intersect_disjoint() {
        let rect1 = DirtyRect::new(0.0, 0.0, 50.0, 50.0, 0);
        let rect2 = DirtyRect::new(100.0, 100.0, 50.0, 50.0, 0);
        assert!(!rect1.intersects(&rect2));
    }

    #[test]
    fn test_dirty_rect_merge() {
        let rect1 = DirtyRect::new(0.0, 0.0, 100.0, 100.0, 0);
        let rect2 = DirtyRect::new(50.0, 50.0, 100.0, 100.0, 0);
        let merged = rect1.merge(&rect2);

        assert!((merged.x - 0.0).abs() < f64::EPSILON);
        assert!((merged.y - 0.0).abs() < f64::EPSILON);
        assert!((merged.width - 150.0).abs() < f64::EPSILON);
        assert!((merged.height - 150.0).abs() < f64::EPSILON);
        assert_eq!(merged.page, 0);
    }

    #[test]
    fn test_dirty_rect_merge_different_pages_returns_self() {
        let rect1 = DirtyRect::new(0.0, 0.0, 100.0, 100.0, 0);
        let rect2 = DirtyRect::new(50.0, 50.0, 100.0, 100.0, 1);
        let merged = rect1.merge(&rect2);

        // Should return self when pages differ
        assert_eq!(merged, rect1);
    }

    #[test]
    fn test_dirty_rect_contains_point() {
        let rect = DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0);

        // Inside
        assert!(rect.contains_point(50.0, 40.0));
        // On edge
        assert!(rect.contains_point(10.0, 20.0));
        assert!(rect.contains_point(110.0, 70.0));
        // Outside
        assert!(!rect.contains_point(5.0, 40.0));
        assert!(!rect.contains_point(50.0, 10.0));
    }

    #[test]
    fn test_dirty_rect_expand() {
        let rect = DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0);
        let expanded = rect.expand(5.0);

        assert!((expanded.x - 5.0).abs() < f64::EPSILON);
        assert!((expanded.y - 15.0).abs() < f64::EPSILON);
        assert!((expanded.width - 110.0).abs() < f64::EPSILON);
        assert!((expanded.height - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_dirty_rect_expand_clamps_negative() {
        let rect = DirtyRect::new(2.0, 3.0, 10.0, 10.0, 0);
        let expanded = rect.expand(5.0);

        // x and y should be clamped to 0
        assert!((expanded.x - 0.0).abs() < f64::EPSILON);
        assert!((expanded.y - 0.0).abs() < f64::EPSILON);
    }

    // =========================================================================
    // DirtyRegionTracker Tests
    // =========================================================================

    #[test]
    fn test_dirty_region_tracker_empty() {
        let tracker = DirtyRegionTracker::new();
        assert!(!tracker.has_dirty_regions());
        assert!(!tracker.needs_full_repaint());
        assert!(tracker.dirty_regions().is_empty());
    }

    #[test]
    fn test_dirty_region_tracker_mark_dirty() {
        let mut tracker = DirtyRegionTracker::new();
        let rect = DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0);

        tracker.mark_dirty(rect);

        assert!(tracker.has_dirty_regions());
        assert_eq!(tracker.dirty_regions().len(), 1);
        assert_eq!(tracker.dirty_regions()[0], rect);
    }

    #[test]
    fn test_dirty_region_tracker_mark_full_repaint() {
        let mut tracker = DirtyRegionTracker::new();
        let rect = DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0);
        tracker.mark_dirty(rect);

        tracker.mark_full_repaint();

        assert!(tracker.has_dirty_regions());
        assert!(tracker.needs_full_repaint());
        // Regions should be cleared when full repaint is requested
        assert!(tracker.dirty_regions().is_empty());
    }

    #[test]
    fn test_dirty_region_tracker_mark_dirty_after_full_repaint() {
        let mut tracker = DirtyRegionTracker::new();
        tracker.mark_full_repaint();

        let rect = DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0);
        tracker.mark_dirty(rect);

        // Should ignore new dirty rects when full repaint is needed
        assert!(tracker.dirty_regions().is_empty());
    }

    #[test]
    fn test_dirty_region_tracker_merges_overlapping() {
        let mut tracker = DirtyRegionTracker::new();
        let rect1 = DirtyRect::new(0.0, 0.0, 100.0, 100.0, 0);
        let rect2 = DirtyRect::new(50.0, 50.0, 100.0, 100.0, 0);

        tracker.mark_dirty(rect1);
        tracker.mark_dirty(rect2);

        // Should merge overlapping rects
        assert_eq!(tracker.dirty_regions().len(), 1);
        let merged = &tracker.dirty_regions()[0];
        assert!((merged.width - 150.0).abs() < f64::EPSILON);
        assert!((merged.height - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_dirty_region_tracker_does_not_absorb_sparse_l_shape_into_huge_bbox() {
        let mut tracker = DirtyRegionTracker::new();
        tracker.mark_dirty(DirtyRect::new(0.0, 0.0, 1000.0, 10.0, 0));
        tracker.mark_dirty(DirtyRect::new(0.0, 0.0, 10.0, 1000.0, 0));

        assert_eq!(
            tracker.dirty_regions().len(),
            2,
            "a sparse L-shape must not become a 1000x1000 dirty bbox"
        );
    }

    #[test]
    fn test_dirty_region_tracker_keeps_disjoint_separate() {
        let mut tracker = DirtyRegionTracker::new();
        let rect1 = DirtyRect::new(0.0, 0.0, 50.0, 50.0, 0);
        let rect2 = DirtyRect::new(200.0, 200.0, 50.0, 50.0, 0);

        tracker.mark_dirty(rect1);
        tracker.mark_dirty(rect2);

        // Should keep disjoint rects separate
        assert_eq!(tracker.dirty_regions().len(), 2);
    }

    #[test]
    fn test_dirty_region_tracker_different_pages() {
        let mut tracker = DirtyRegionTracker::new();
        let rect1 = DirtyRect::new(0.0, 0.0, 100.0, 100.0, 0);
        let rect2 = DirtyRect::new(0.0, 0.0, 100.0, 100.0, 1);

        tracker.mark_dirty(rect1);
        tracker.mark_dirty(rect2);

        // Should keep rects on different pages separate
        assert_eq!(tracker.dirty_regions().len(), 2);
    }

    #[test]
    fn test_dirty_region_tracker_clear() {
        let mut tracker = DirtyRegionTracker::new();
        tracker.mark_dirty(DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0));
        tracker.mark_full_repaint();

        tracker.clear();

        assert!(!tracker.has_dirty_regions());
        assert!(!tracker.needs_full_repaint());
        assert!(tracker.dirty_regions().is_empty());
    }

    #[test]
    fn test_dirty_region_tracker_change_id_increments() {
        let mut tracker = DirtyRegionTracker::new();
        let initial_id = tracker.change_id();

        tracker.mark_dirty(DirtyRect::new(10.0, 20.0, 100.0, 50.0, 0));
        assert_eq!(tracker.change_id(), initial_id + 1);

        tracker.mark_full_repaint();
        assert_eq!(tracker.change_id(), initial_id + 2);
    }

    #[test]
    fn test_dirty_region_tracker_regions_for_page() {
        let mut tracker = DirtyRegionTracker::new();
        tracker.mark_dirty(DirtyRect::new(0.0, 0.0, 50.0, 50.0, 0));
        tracker.mark_dirty(DirtyRect::new(100.0, 100.0, 50.0, 50.0, 0));
        tracker.mark_dirty(DirtyRect::new(0.0, 0.0, 50.0, 50.0, 1));

        let page0_regions = tracker.dirty_regions_for_page(0);
        let page1_regions = tracker.dirty_regions_for_page(1);

        assert_eq!(page0_regions.len(), 2);
        assert_eq!(page1_regions.len(), 1);
    }

    #[test]
    fn test_dirty_region_tracker_coalesce() {
        let mut tracker = DirtyRegionTracker::new();
        // Add non-overlapping first
        tracker.mark_dirty(DirtyRect::new(0.0, 0.0, 50.0, 50.0, 0));
        tracker.mark_dirty(DirtyRect::new(200.0, 200.0, 50.0, 50.0, 0));
        tracker.mark_dirty(DirtyRect::new(0.0, 0.0, 50.0, 50.0, 1));

        assert_eq!(tracker.dirty_regions().len(), 3);

        // Add overlapping with first
        tracker
            .regions
            .push(DirtyRect::new(25.0, 25.0, 50.0, 50.0, 0));

        tracker.coalesce();

        // After coalesce, the overlapping rects on page 0 should be merged
        let page0 = tracker.dirty_regions_for_page(0);
        let page1 = tracker.dirty_regions_for_page(1);

        assert_eq!(page0.len(), 2); // Merged + disjoint
        assert_eq!(page1.len(), 1);
    }

    // =========================================================================
    // ParagraphChange Tests
    // =========================================================================

    #[test]
    fn test_paragraph_change_creation() {
        let change = ParagraphChange {
            paragraph_index: 5,
            change_type: ParagraphChangeType::ContentModified,
            change_id: 42,
        };

        assert_eq!(change.paragraph_index, 5);
        assert_eq!(change.change_type, ParagraphChangeType::ContentModified);
        assert_eq!(change.change_id, 42);
    }

    #[test]
    fn test_paragraph_change_types() {
        // Just ensure all variants exist and can be compared
        assert_ne!(
            ParagraphChangeType::ContentModified,
            ParagraphChangeType::FormattingChanged
        );
        assert_ne!(ParagraphChangeType::Inserted, ParagraphChangeType::Deleted);
        assert_ne!(ParagraphChangeType::Split, ParagraphChangeType::Merged);
    }

    // =========================================================================
    // SelectionRect Tests
    // =========================================================================

    #[test]
    fn test_selection_rect_creation() {
        let rect = SelectionRect::new(10.0, 20.0, 100.0, 50.0, 0);
        assert!((rect.x - 10.0).abs() < f64::EPSILON);
        assert!((rect.y - 20.0).abs() < f64::EPSILON);
        assert!((rect.width - 100.0).abs() < f64::EPSILON);
        assert!((rect.height - 50.0).abs() < f64::EPSILON);
        assert_eq!(rect.page, 0);
    }

    #[test]
    fn test_selection_rect_right_bottom() {
        let rect = SelectionRect::new(10.0, 20.0, 100.0, 50.0, 0);
        assert!((rect.right() - 110.0).abs() < f64::EPSILON);
        assert!((rect.bottom() - 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_selection_rect_to_dirty_rect() {
        let selection_rect = SelectionRect::new(10.0, 20.0, 100.0, 50.0, 1);
        let dirty_rect = selection_rect.to_dirty_rect();

        assert!((dirty_rect.x - 10.0).abs() < f64::EPSILON);
        assert!((dirty_rect.y - 20.0).abs() < f64::EPSILON);
        assert!((dirty_rect.width - 100.0).abs() < f64::EPSILON);
        assert!((dirty_rect.height - 50.0).abs() < f64::EPSILON);
        assert_eq!(dirty_rect.page, 1);
    }

    // =========================================================================
    // SelectionOverlay Tests
    // =========================================================================

    #[test]
    fn test_selection_overlay_empty() {
        let overlay = SelectionOverlay::new();
        assert!(!overlay.has_selection());
        assert!(overlay.rects().is_empty());
        assert!(overlay.caret().is_none());
        assert_eq!(overlay.rect_count(), 0);
    }

    #[test]
    fn test_selection_overlay_add_rect() {
        let mut overlay = SelectionOverlay::new();
        overlay.clear_dirty(); // Reset dirty flag

        let rect = SelectionRect::new(10.0, 20.0, 100.0, 50.0, 0);
        overlay.add_rect(rect);

        assert!(overlay.has_selection());
        assert_eq!(overlay.rect_count(), 1);
        assert!(overlay.is_dirty());
    }

    #[test]
    fn test_selection_overlay_set_rects() {
        let mut overlay = SelectionOverlay::new();
        let rects = vec![
            SelectionRect::new(10.0, 20.0, 100.0, 50.0, 0),
            SelectionRect::new(10.0, 80.0, 100.0, 50.0, 0),
        ];

        overlay.set_rects(rects);

        assert!(overlay.has_selection());
        assert_eq!(overlay.rect_count(), 2);
        assert!(overlay.caret().is_none());
    }

    #[test]
    fn test_selection_overlay_set_caret() {
        let mut overlay = SelectionOverlay::new();
        // Add some rects first
        overlay.add_rect(SelectionRect::new(10.0, 20.0, 100.0, 50.0, 0));

        // Set caret should clear rects
        let caret = CaretRect::new(50.0, 100.0, 20.0, 0);
        overlay.set_caret(caret);

        assert!(overlay.has_selection());
        assert!(overlay.rects().is_empty());
        assert!(overlay.caret().is_some());
        assert!((overlay.caret().unwrap().x - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_selection_overlay_clear() {
        let mut overlay = SelectionOverlay::new();
        overlay.add_rect(SelectionRect::new(10.0, 20.0, 100.0, 50.0, 0));
        overlay.clear_dirty();

        overlay.clear();

        assert!(!overlay.has_selection());
        assert!(overlay.rects().is_empty());
        assert!(overlay.caret().is_none());
        assert!(overlay.is_dirty());
    }

    #[test]
    fn test_selection_overlay_dirty_flag() {
        let mut overlay = SelectionOverlay::new();

        // Initially not dirty after creation (no selection yet)
        assert!(!overlay.is_dirty());

        overlay.mark_dirty();
        assert!(overlay.is_dirty());

        overlay.clear_dirty();
        assert!(!overlay.is_dirty());
    }

    #[test]
    fn test_selection_overlay_rects_for_page() {
        let mut overlay = SelectionOverlay::new();
        overlay.add_rect(SelectionRect::new(10.0, 20.0, 100.0, 50.0, 0));
        overlay.add_rect(SelectionRect::new(10.0, 80.0, 100.0, 50.0, 0));
        overlay.add_rect(SelectionRect::new(10.0, 20.0, 100.0, 50.0, 1));

        let page0_rects = overlay.rects_for_page(0);
        let page1_rects = overlay.rects_for_page(1);
        let page2_rects = overlay.rects_for_page(2);

        assert_eq!(page0_rects.len(), 2);
        assert_eq!(page1_rects.len(), 1);
        assert_eq!(page2_rects.len(), 0);
    }

    #[test]
    fn test_selection_overlay_bounding_box_for_page() {
        let mut overlay = SelectionOverlay::new();
        overlay.add_rect(SelectionRect::new(10.0, 20.0, 50.0, 30.0, 0));
        overlay.add_rect(SelectionRect::new(100.0, 80.0, 50.0, 30.0, 0));

        let bbox = overlay.bounding_box_for_page(0).unwrap();

        assert!((bbox.x - 10.0).abs() < f64::EPSILON);
        assert!((bbox.y - 20.0).abs() < f64::EPSILON);
        assert!((bbox.right() - 150.0).abs() < f64::EPSILON);
        assert!((bbox.bottom() - 110.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_selection_overlay_bounding_box_empty_page() {
        let mut overlay = SelectionOverlay::new();
        overlay.add_rect(SelectionRect::new(10.0, 20.0, 50.0, 30.0, 0));

        let bbox = overlay.bounding_box_for_page(1);
        assert!(bbox.is_none());
    }

    #[test]
    fn test_selection_overlay_bounding_box_with_caret_only() {
        let mut overlay = SelectionOverlay::new();
        overlay.set_caret(CaretRect::new(50.0, 100.0, 20.0, 0));

        let bbox = overlay.bounding_box_for_page(0).unwrap();

        assert!((bbox.x - 50.0).abs() < f64::EPSILON);
        assert!((bbox.y - 100.0).abs() < f64::EPSILON);
        assert!((bbox.width - 2.0).abs() < f64::EPSILON); // Caret width
        assert!((bbox.height - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_caret_rect_creation() {
        let caret = CaretRect::new(50.0, 100.0, 20.0, 1);
        assert!((caret.x - 50.0).abs() < f64::EPSILON);
        assert!((caret.y - 100.0).abs() < f64::EPSILON);
        assert!((caret.height - 20.0).abs() < f64::EPSILON);
        assert_eq!(caret.page, 1);
    }
}
