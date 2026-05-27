//! Cloudpack Transform Engine — converts a chunk of summarized modules into
//! emittable JavaScript output. Engine-agnostic via the `TransformEngine` trait.

pub mod engine;
pub mod rolldown_adapter;
pub mod swc_adapter;
pub mod swc_util;

pub use engine::{
    sanitize_entry_key, BatchConfig, ChunkOutput, TransformDecisions, TransformEngine,
    TransformError,
};
pub use rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};
pub use swc_adapter::{SwcAdapterConfig, SwcTransformAdapter};

pub fn hello() -> &'static str {
    "cloudpack-transform"
}
