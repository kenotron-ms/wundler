//! `wundler-graph` – dependency-graph layer for Wundler.
//!
//! This crate models the module dependency graph produced by the
//! `wundler-core` analysis pass.  It wraps `petgraph` to provide
//! typed graph nodes, deterministic topological ordering, cycle
//! detection, and JSON serialisation.

pub mod dce;
pub mod graph;
pub mod reachability;
pub mod types;

/// Returns the crate name – used as a lightweight smoke-test sentinel.
pub fn hello() -> &'static str {
    "wundler-graph"
}
