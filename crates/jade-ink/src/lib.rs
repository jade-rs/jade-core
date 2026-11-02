//! Office ink, shared by the Word and PowerPoint engines.
//!
//! Word (`w14:contentPart`) and PowerPoint (`p:contentPart`) keep ink the
//! same way: a content part relating to an InkML part, usually inside an
//! `mc:AlternateContent` whose fallback is a picture of the strokes. This
//! crate holds what does not depend on which: the InkML itself
//! ([`inkml`]), painting strokes through any [`jade_painter::Painter`]
//! ([`paint`]), and the fallback picture ([`raster`]).

pub mod inkml;
pub mod paint;
pub mod raster;

pub use inkml::{InkBrush, InkContent, InkTool, InkTrace};
