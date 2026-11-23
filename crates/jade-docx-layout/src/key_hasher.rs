//! A hasher for cache keys that are trusted without comparing their inputs.
//!
//! `FxHasher` is `usize`-wide, so on 32-bit targets (wasm32) its `u64` keys
//! carry only 32 bits: about a 0.3% chance of some collision at 5,000 live
//! keys. [`CacheKeyHasher`] is `FxHasher` itself on 64-bit targets, so native
//! keys and their cost are unchanged, and [`Fx64Hasher`] on 32-bit targets.
use std::hash::Hasher;

#[cfg(target_pointer_width = "64")]
pub type CacheKeyHasher = rustc_hash::FxHasher;
#[cfg(not(target_pointer_width = "64"))]
pub type CacheKeyHasher = Fx64Hasher;

/// FxHash with a `u64` state on every target. wasm32 has native 64-bit
/// integer operations, so this costs about as much as `FxHasher` there.
#[derive(Default, Clone, Copy)]
pub struct Fx64Hasher {
    hash: u64,
}

impl Fx64Hasher {
    const K: u64 = 0xf135_7aea_2e62_a9c5;

    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = self.hash.wrapping_add(word).wrapping_mul(Self::K);
    }
}

impl Hasher for Fx64Hasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Length first, so consecutive byte strings stay prefix-free: "ab"
        // then "c" differs from "a" then "bc", and from "abc".
        self.add(bytes.len() as u64);
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let tail = chunks.remainder();
        if !tail.is_empty() {
            let mut packed = [0_u8; 8];
            packed[..tail.len()].copy_from_slice(tail);
            self.add(u64::from_le_bytes(packed));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // As FxHasher: move the high-entropy top bits down for bucket indexes.
        self.hash.rotate_left(26)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::hash::Hash;

    // Native builds use FxHasher, so test the wasm32 hasher directly.
    fn key(write: &dyn Fn(&mut Fx64Hasher)) -> u64 {
        let mut hasher = Fx64Hasher::default();
        0x9e37_79b9_7f4a_7c15_u64.hash(&mut hasher);
        write(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn keeps_field_positions_after_unset_fields() {
        // The #915 collision shape: the same value after a different number
        // of unset fields.
        assert_ne!(
            key(&|h| (None::<&str>, None::<&str>, Some("游明朝")).hash(h)),
            key(&|h| (Some("游明朝"), None::<&str>).hash(h)),
        );
    }

    #[test]
    fn byte_strings_are_prefix_free() {
        let splits: [&[&[u8]]; 9] = [
            &[b""],
            &[b"", b""],
            &[b"\0"],
            &[b"\0\0"],
            &[b"\0", b"\0"],
            &[b"abc"],
            &[b"ab", b"c"],
            &[b"a", b"bc"],
            &[b"abcdefgh", b"\0"],
        ];
        let mut keys = HashSet::new();
        for parts in splits {
            let digest = key(&|h| parts.iter().for_each(|part| h.write(part)));
            assert!(keys.insert(digest), "{parts:?}");
        }
        assert!(keys.insert(key(&|h| h.write(b"abcdefgh\0\0"))));
    }

    #[test]
    fn varies_in_the_high_32_bits() {
        // FxHasher drops these on wasm32.
        let highs = (0..64_u64)
            .map(|value| key(&|h| value.hash(h)) >> 32)
            .collect::<HashSet<_>>();
        assert!(highs.len() > 60);
    }
}
