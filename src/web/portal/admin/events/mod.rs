//! Admin event handlers.
//!
//! Split into single-event CRUD handlers (`single`) and the
//! per-occurrence (recurring-series exception) handlers (`occurrences`).
//! The router in `crate::web::portal` resolves every handler through the
//! re-exports below, so the module path `crate::web::portal::admin::events`
//! is unchanged.

mod enrollments;
mod occurrences;
mod roster;
mod single;

pub use enrollments::*;
pub use occurrences::*;
pub use roster::*;
pub use single::*;

/// Simple struct for type options in dropdowns
#[derive(Clone)]
pub struct TypeOption {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub color: Option<String>,
}
