//! The paragraph layouts a `SelectionManager` registers, indexed by paragraph.
//!
//! It reads as the slice of registered paragraphs. It can also hold a tail set
//! aside out of sight ([`ParagraphRegistry::park`]) without moving it: a
//! stable edit sets aside every paragraph after the edited one on each
//! keystroke and puts them back after the pass, and moving them out of the
//! vector and back in again costs a copy of the document's tail each time.
//!
//! While a tail is parked, the parked paragraphs stay where they are in
//! `items`, past the registered length. A paragraph registered over one of
//! their slots moves the parked one into `displaced` first, so each parked
//! paragraph is either in `displaced` or still in its slot, and putting the
//! tail back touches only the slots the pass registered over.

use super::ParagraphLayout;
use std::ops::{Deref, DerefMut};

#[derive(Clone, Debug, Default)]
pub(super) struct ParagraphRegistry {
    /// The registered paragraphs, `items[..len]`. While a tail is parked in
    /// place, the slots past them hold the parked paragraphs not yet
    /// displaced, and empty placeholders.
    items: Vec<ParagraphLayout>,
    len: usize,
    parked: Option<Parked>,
    next_park: u64,
    /// An emptied `displaced` kept for the next park, so that each
    /// keystroke's park does not allocate it again.
    spare: Vec<ParagraphLayout>,
}

#[derive(Clone, Debug)]
struct Parked {
    id: u64,
    /// The index of the first parked paragraph.
    from: usize,
    place: Place,
}

#[derive(Clone, Debug)]
enum Place {
    /// The parked paragraphs `from..end`: `displaced[i - from]` for the
    /// first `displaced.len()` of them, `items[i]` for the rest.
    ///
    /// Invariants: `from < end`, `from + displaced.len() <= end`,
    /// `items.len() == max(len, end)`, and every registered index in
    /// `from..end` is below `from + displaced.len()`, so a registered
    /// paragraph never shares a slot with a parked one.
    InPlace {
        end: usize,
        displaced: Vec<ParagraphLayout>,
    },
    /// Moved out, for an operation that shifts the registered paragraphs.
    /// `items.len() == len`.
    Detached(Vec<ParagraphLayout>),
}

impl Deref for ParagraphRegistry {
    type Target = [ParagraphLayout];

    #[inline]
    fn deref(&self) -> &[ParagraphLayout] {
        &self.items[..self.len]
    }
}

impl DerefMut for ParagraphRegistry {
    #[inline]
    fn deref_mut(&mut self) -> &mut [ParagraphLayout] {
        &mut self.items[..self.len]
    }
}

impl ParagraphRegistry {
    /// The end of the parked paragraphs still in `items`, if any are parked
    /// in place.
    fn parked_end(&self) -> Option<usize> {
        match &self.parked {
            Some(Parked {
                place: Place::InPlace { end, .. },
                ..
            }) => Some(*end),
            _ => None,
        }
    }

    /// Nothing is parked in place: `items` holds the registered paragraphs
    /// only, and the vector's own operations apply.
    #[inline]
    fn plain(&self) -> bool {
        self.parked.is_none()
    }

    #[inline]
    pub(super) fn truncate(&mut self, len: usize) {
        if self.plain() {
            self.items.truncate(len);
            self.len = self.items.len();
        } else {
            self.truncate_parked(len);
        }
    }

    #[cold]
    fn truncate_parked(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        match self.parked_end() {
            Some(end) => {
                // Keep the slots the parked paragraphs still occupy; the
                // registered ones among them become placeholders.
                for slot in self.items.iter_mut().take(self.len.min(end)).skip(len) {
                    *slot = ParagraphLayout::default();
                }
                self.items.truncate(len.max(end));
            }
            None => self.items.truncate(len),
        }
        self.len = len;
    }

    #[inline]
    pub(super) fn clear(&mut self) {
        self.truncate(0);
    }

    #[inline]
    pub(super) fn push(&mut self, paragraph: ParagraphLayout) {
        if self.plain() {
            self.items.push(paragraph);
            self.len += 1;
        } else {
            self.push_parked(paragraph);
        }
    }

    fn push_parked(&mut self, paragraph: ParagraphLayout) {
        let index = self.len;
        if index < self.items.len() {
            match &mut self.parked {
                Some(Parked {
                    from,
                    place: Place::InPlace { end, displaced },
                    ..
                }) if index < *end && index == *from + displaced.len() => {
                    displaced.push(std::mem::replace(&mut self.items[index], paragraph));
                }
                // A placeholder.
                _ => self.items[index] = paragraph,
            }
        } else {
            self.items.push(paragraph);
        }
        self.len += 1;
    }

    #[inline]
    pub(super) fn resize_with(&mut self, len: usize, mut fill: impl FnMut() -> ParagraphLayout) {
        if self.plain() {
            self.items.resize_with(len, fill);
            self.len = len;
            return;
        }
        if len <= self.len {
            self.truncate(len);
            return;
        }
        for _ in self.len..len {
            self.push(fill());
        }
    }

    pub(super) fn extend(&mut self, paragraphs: impl IntoIterator<Item = ParagraphLayout>) {
        if self.plain() {
            self.items.extend(paragraphs);
            self.len = self.items.len();
            return;
        }
        for paragraph in paragraphs {
            self.push(paragraph);
        }
    }

    /// The paragraph at `index`, the registry first grown to it with `fill`.
    #[inline]
    pub(super) fn grow_to(
        &mut self,
        index: usize,
        fill: impl FnMut() -> ParagraphLayout,
    ) -> &mut ParagraphLayout {
        if self.len <= index {
            self.resize_with(index + 1, fill);
        }
        &mut self.items[index]
    }

    pub(super) fn insert(&mut self, index: usize, paragraph: ParagraphLayout) {
        self.detach();
        self.items.insert(index, paragraph);
        self.len += 1;
    }

    pub(super) fn remove(&mut self, index: usize) -> ParagraphLayout {
        self.detach();
        self.len -= 1;
        self.items.remove(index)
    }

    pub(super) fn split_off(&mut self, at: usize) -> Vec<ParagraphLayout> {
        self.detach();
        let tail = self.items.split_off(at);
        self.len = self.items.len();
        tail
    }

    /// Moves a tail parked in place out of `items`, leaving it holding the
    /// registered paragraphs only.
    fn detach(&mut self) {
        let Some(Parked { from, place, .. }) = &mut self.parked else {
            return;
        };
        let Place::InPlace { end, displaced } = place else {
            return;
        };
        let mut tail = std::mem::take(displaced);
        let still_in_place = *from + tail.len();
        if still_in_place < *end {
            tail.extend(self.items.drain(still_in_place..*end));
        }
        // What is left past the registered paragraphs is placeholders.
        self.items.truncate(self.len);
        *place = Place::Detached(tail);
    }

    /// Sets the paragraphs from `from` on aside, as moving them out with
    /// `split_off` would, without moving them. `None` when there are none,
    /// as `split_off` would return none, and when a tail is parked already:
    /// the caller takes them then.
    pub(super) fn park(&mut self, from: usize) -> Option<u64> {
        if self.parked.is_some() || from >= self.len {
            return None;
        }
        let id = self.next_park;
        self.next_park = self.next_park.wrapping_add(1);
        self.parked = Some(Parked {
            id,
            from,
            place: Place::InPlace {
                end: self.len,
                displaced: std::mem::take(&mut self.spare),
            },
        });
        self.len = from;
        Some(id)
    }

    /// Puts back the tail parked as `id`. Returns false if it was discarded.
    pub(super) fn restore_parked(&mut self, id: u64, from: usize) -> bool {
        let Some(parked) = self.parked.as_ref().filter(|parked| parked.id == id) else {
            return false;
        };
        let fast = parked.from == from
            && self.len >= from
            && matches!(parked.place, Place::InPlace { .. });
        if !fast {
            let tail = self
                .detach_parked(id)
                .expect("checked parked tail ID above");
            if !tail.is_empty() {
                self.truncate(from);
                self.extend(tail);
            }
            return true;
        }
        let Some(Parked {
            place: Place::InPlace { end, mut displaced },
            ..
        }) = self.parked.take()
        else {
            unreachable!("checked above");
        };
        // Every registered paragraph from `from` on shares a slot below
        // `from + displaced.len()`, or lies past `end`.
        for (slot, paragraph) in self.items[from..].iter_mut().zip(displaced.drain(..)) {
            *slot = paragraph;
        }
        self.spare = displaced;
        self.items.truncate(end);
        self.len = end;
        true
    }

    /// Hands over the tail parked as `id`, leaving registered paragraphs intact.
    pub(super) fn detach_parked(&mut self, id: u64) -> Option<Vec<ParagraphLayout>> {
        self.parked.as_ref().filter(|parked| parked.id == id)?;
        self.detach();
        let Some(Place::Detached(tail)) = self.parked.take().map(|parked| parked.place) else {
            unreachable!("detach moved the parked tail out of the registry");
        };
        Some(tail)
    }

    /// Drops the tail parked as `id`, the registered paragraphs staying as
    /// they are. Nothing when `id` is not the parked one's: that tail is
    /// another suffix's.
    pub(super) fn discard_parked(&mut self, id: u64) {
        if self.parked.as_ref().is_some_and(|parked| parked.id == id) {
            self.drop_parked();
        }
    }

    /// Whether a tail is parked.
    pub(super) fn has_parked(&self) -> bool {
        self.parked.is_some()
    }

    /// Drops whatever tail is parked, if one is.
    pub(super) fn drop_parked(&mut self) {
        if self.parked.take().is_some() {
            self.items.truncate(self.len);
        }
    }
}
