//! Shared domain operations and persistence for punch adapters.
pub mod basecamp;
pub mod derive;
pub mod error;
pub mod github;
pub mod model;
pub mod ops;
pub mod store;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod process;
