//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod config;
pub mod output;
pub mod pipeline;

pub use config::{BuildConfig, EngineChoice};
pub use pipeline::{BuildOutput, BuildPipeline, BuildStats};

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
