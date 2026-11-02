//! SmartArt — DrawingML diagrams — as PowerPoint and Word keep them.
//!
//! A diagram is four parts and a cache: the data (`dgm:dataModel`, the
//! text pane's bullets and how they nest), the layout, colour and style
//! definitions, and the drawing Office caches beside them (`dsp:drawing`),
//! which is what every reader draws. This crate reads and edits the data,
//! lays it out as PowerPoint's built-in layouts lay it out, colours and
//! styles it as the built-in colour schemes and SmartArt styles do, and
//! writes the drawing.
//!
//! The layouts are measured from PowerPoint's own output: given a data part
//! alone, PowerPoint lays it out from its built-in definition of the
//! data's `loTypeId`, and jade lays the same data out to the same boxes.
//! That is also how jade writes a diagram it made or re-laid: the data and
//! the drawing, and no layout, colour or style part of its own, so that
//! PowerPoint lays it out with its own definitions when it is next edited.

pub mod catalog;
pub mod colors;
pub mod data;
pub mod drawing;
pub mod engine;
pub mod layout_def;
pub mod tree;
pub mod xml;

pub use catalog::{Category, LayoutInfo, COLOR_SCHEMES, LAYOUTS, STYLES};
pub use data::{DataModel, Node, NodeKind, ShapeFormat};
pub use drawing::{render, render_model, update_text, Look, Parts};
pub use engine::{lay_out, Drawn, Paint};
pub use layout_def::Algorithm;
