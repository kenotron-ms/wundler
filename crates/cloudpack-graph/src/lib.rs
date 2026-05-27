//! `cloudpack-graph` – dependency-graph layer for Cloudpack.
//!
//! This crate models the module dependency graph produced by the
//! `cloudpack-core` analysis pass.  It wraps `petgraph` to provide
//! typed graph nodes, deterministic topological ordering, cycle
//! detection, and JSON serialisation.

pub mod analyzer;
pub mod chunks;
pub mod dce;
pub mod graph;
pub mod manifest;
pub mod reachability;
pub mod types;

// ---------------------------------------------------------------------------
// Convenience re-exports
// ---------------------------------------------------------------------------

pub use analyzer::{AnalysisResult, AnalysisStats, GraphAnalyzer};
pub use types::{Chunk, ChunkId, ChunkManifest, EntryPoint, LoadCondition};

/// Returns the crate name – used as a lightweight smoke-test sentinel.
pub fn hello() -> &'static str {
    "cloudpack-graph"
}
