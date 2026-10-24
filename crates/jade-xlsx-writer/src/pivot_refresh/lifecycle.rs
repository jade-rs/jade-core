//! Create/remove a complete pivot relationship graph in a staged package.
mod create;
mod package;
mod remove;
pub use create::{create_pivot, PivotCreate, PivotCreateResult, PivotDestination};
pub use remove::{delete_pivot, PivotDeleteResult};
