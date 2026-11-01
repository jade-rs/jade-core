//! Large parts inflated (or written) lately, kept with their compressed
//! bytes so the same entry is not inflated again: an editor writes its
//! package on every edit and opens what it wrote for the next one, which
//! inflated every large part again each time.
//!
//! An entry is taken from here only when its compressed bytes are exactly
//! those kept — never by name or checksum alone, which a crafted archive
//! could match — so what comes back is what inflating would give.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Entries this small are inflated as before: comparing them would cost
/// about what inflating does.
pub(crate) const MIN_COMPRESSED: u64 = 16 * 1024;
/// What the kept parts may add up to, compressed and inflated.
const BUDGET: usize = 64 * 1024 * 1024;

struct Kept {
    name: String,
    compressed: Rc<[u8]>,
    contents: Rc<[u8]>,
}

thread_local! {
    static KEPT: RefCell<VecDeque<Kept>> = const { RefCell::new(VecDeque::new()) };
}

/// The inflated contents of `name` whose compressed bytes are `compressed`,
/// when kept.
pub(crate) fn recall(name: &str, compressed: &[u8]) -> Option<Rc<[u8]>> {
    KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        let at = kept
            .iter()
            .position(|part| part.name == name && *part.compressed == *compressed)?;
        // Most recently used first.
        let part = kept.remove(at)?;
        let contents = part.contents.clone();
        kept.push_front(part);
        Some(contents)
    })
}

/// Keeps `contents` as what `compressed` inflates to, for `name`.
pub(crate) fn keep(name: &str, compressed: Vec<u8>, contents: &[u8]) {
    let size = compressed.len() + contents.len();
    if size > BUDGET / 4 {
        return;
    }
    KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        kept.retain(|part| !(part.name == name && *part.compressed == *compressed));
        kept.push_front(Kept {
            name: name.to_string(),
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
        keep("xl/a.xml", vec![1, 2, 3], b"inflated");
        assert_eq!(
            recall("xl/a.xml", &[1, 2, 3]).as_deref(),
            Some(&b"inflated"[..])
        );
        assert!(recall("xl/a.xml", &[1, 2, 4]).is_none());
        assert!(recall("xl/b.xml", &[1, 2, 3]).is_none());
    }
}
