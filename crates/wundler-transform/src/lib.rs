//! Wundler Transform Engine — converts a chunk of summarized modules into
//! emittable JavaScript output. Engine-agnostic via the `TransformEngine` trait.

pub mod engine;
pub mod swc_adapter;
pub mod swc_util;

pub use engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};

pub fn hello() -> &'static str {
    "wundler-transform"
}
