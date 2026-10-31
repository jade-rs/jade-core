//! Formatting-only reuse for control-mark font descriptors. No font metrics or
//! painter state belongs here.
use std::{cell::RefCell, rc::Rc};

const CAPACITY: usize = 4;

struct Entry {
    size_bits: u32,
    prefix_len: usize,
    family_start: usize,
    descriptor: Rc<str>,
}

#[derive(Default)]
struct ControlMarkFontCache {
    entries: Vec<Entry>,
    next_eviction: usize,
}

impl ControlMarkFontCache {
    fn get(&mut self, size_pt: f32, prefix: &str, family: &str) -> Rc<str> {
        let size_bits = size_pt.to_bits();
        if let Some(entry) = self.entries.iter().rev().find(|entry| {
            entry.size_bits == size_bits
                && entry.descriptor.get(..entry.prefix_len) == Some(prefix)
                && entry.descriptor.get(entry.family_start..) == Some(family)
        }) {
            return Rc::clone(&entry.descriptor);
        }
        let descriptor: Rc<str> = if prefix.is_empty() {
            Rc::from(format!("{size_pt:0.2}pt {family}"))
        } else {
            Rc::from(format!("{prefix}{size_pt:0.2}pt {family}"))
        };
        // The descriptor already owns both key strings. Keep their UTF-8 byte
        // boundaries instead of allocating a second copy on cache misses.
        let entry = Entry {
            size_bits,
            prefix_len: prefix.len(),
            family_start: descriptor.len() - family.len(),
            descriptor: Rc::clone(&descriptor),
        };
        if self.entries.len() < CAPACITY {
            self.entries.push(entry);
        } else {
            self.entries[self.next_eviction] = entry;
            self.next_eviction = (self.next_eviction + 1) % CAPACITY;
        }
        descriptor
    }
}

pub(crate) fn descriptor(size_pt: f32, prefix: &str, family: &str) -> Rc<str> {
    // Standalone display-list execution creates a fresh font-state tracker for
    // each mark. Reuse the pure string across those calls without retaining any
    // painter, document, or font-metric state. Drop the borrow before painting.
    thread_local! {
        static CACHE: RefCell<ControlMarkFontCache> = RefCell::default();
    }
    CACHE.with(|cache| cache.borrow_mut().get(size_pt, prefix, family))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn repeated_control_marks_reuse_the_formatted_allocation() {
        let first = descriptor(9.0, "bold ", "'Arial', sans-serif");
        assert_eq!(&*first, "bold 9.00pt 'Arial', sans-serif");
        for _ in 0..256 {
            let next = descriptor(9.0, "bold ", "'Arial', sans-serif");
            assert!(
                Rc::ptr_eq(&first, &next),
                "reformatted an unchanged mark font"
            );
        }
    }

    #[test]
    fn caller_string_mutation_and_float_boundaries_do_not_reuse_stale_text() {
        let mut cache = ControlMarkFontCache::default();
        let mut family = String::from("Arial");
        assert_eq!(&*cache.get(1.0, "", &family), "1.00pt Arial");
        family.replace_range(.., "Times");
        assert_eq!(&*cache.get(1.0, "", &family), "1.00pt Times");
        for (size, prefix, expected) in [
            (0.0, "", "0.00pt Arial"),
            (-0.0, "", "-0.00pt Arial"),
            (f32::INFINITY, "italic ", "italic infpt Arial"),
            (f32::NEG_INFINITY, "", "-infpt Arial"),
            (f32::NAN, "", "NaNpt Arial"),
            (7.875, "bold ", "bold 7.88pt Arial"),
        ] {
            assert_eq!(&*cache.get(size, prefix, "Arial"), expected);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]
        #[test]
        fn mixed_queries_preserve_the_original_formatter(
            queries in prop::collection::vec((any::<u32>(), 0u8..8, 0u8..7), 1..128)
        ) {
            let mut cache = ControlMarkFontCache::default();
            let prefixes = ["", "bold ", "italic ", "bold italic ", "日本語 ", "{} ", "9.00pt ", "é "];
            let families = ["Arial", "'MS Mincho', serif", "'日本語'", "", "sans-serif", "12pt odd family", "A\\B' C"];
            for (bits, prefix, family) in queries {
                let size = f32::from_bits(bits);
                let prefix = prefixes[prefix as usize];
                let family = families[family as usize];
                // The uncached formatter is the existing exact-string contract.
                let expected = format!("{}{:0.2}pt {}", prefix, size, family);
                let first = cache.get(size, prefix, family);
                prop_assert_eq!(&*first, expected);
                prop_assert!(Rc::ptr_eq(&first, &cache.get(size, prefix, family)));
                prop_assert!(cache.entries.len() <= CAPACITY);
            }
        }
    }

    #[test]
    fn mixed_styles_share_a_bounded_cache_and_evicted_values_remain_valid() {
        let mut cache = ControlMarkFontCache::default();
        let first = cache.get(9.0, "bold ", "Arial");
        for family in ["Times", "Mincho", "Gothic"] {
            cache.get(9.0, "", family);
            assert!(Rc::ptr_eq(&first, &cache.get(9.0, "bold ", "Arial")));
        }
        cache.get(9.0, "", "Courier");
        assert_eq!(cache.entries.len(), CAPACITY);
        let renewed = cache.get(9.0, "bold ", "Arial");
        assert!(!Rc::ptr_eq(&first, &renewed));
        assert_eq!(&*first, "bold 9.00pt Arial");
        assert_eq!(first, renewed);
    }
}
