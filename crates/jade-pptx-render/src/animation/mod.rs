//! Plays a slide's animations and transitions for the slide show.
//!
//! [`SlidePlayer`] keeps the show's clock against `<p:timing>`: which click
//! groups have started, which triggers fired. [`SlidePlayer::frame`]
//! evaluates every behaviour at a moment into a [`SlideFrame`] — per
//! shape, paragraph or letter: visible, opacity, offset, scale, rotation,
//! a reveal mask and colour overrides — and [`render_slide_frame`] paints a
//! slide layout in that state. [`compose_transition`] blends two rendered
//! slides the way a `<p:transition>` does, and [`morph`] interpolates two
//! layouts for Morph.

pub mod evaluate;
pub mod formula;
pub mod frame;
pub mod morph;
pub mod path;
pub mod player;
pub mod reveal;
pub mod transition;

#[cfg(test)]
pub(crate) mod test_support;

pub use evaluate::{KeyPart, SlideFrame, TargetKey, TargetState};
pub use frame::{render_slide_frame, target_bounds};
pub use morph::{morph_layouts, MorphOption};
pub use player::{Advance, SlidePlayer, SlideText};
pub use transition::{compose_transition, TransitionFrame};
