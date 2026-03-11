#![forbid(unsafe_code)]

//! Thin top-level runtime wrapper around `runtime-core`.

mod builder;
mod handle;

pub use builder::{IngestMode, NodeRuntimeBuilder};
pub use handle::{NodeRuntime, ShutdownHook};
