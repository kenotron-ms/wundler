//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod config;
pub mod pipeline;

pub use config::{BuildConfig, EngineChoice};
pub use pipeline::BuildPipeline;

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
