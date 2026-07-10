//! Code intelligence core for Conclave: one language registry, one walker,
//! one index, shared by the map/graph/edit command families.
pub mod cache;
pub mod edit;
pub mod error;
pub mod graph;
pub mod hash;
pub mod index;
pub mod lang;
pub mod map;
pub mod output;
pub mod resolve;
pub mod walk;

pub use error::CoreError;
