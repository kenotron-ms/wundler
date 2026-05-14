//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod config;
pub mod dev_server;
pub mod output;
pub mod pipeline;

pub use config::{BuildConfig, EngineChoice};
pub use dev_server::DevServer;
pub use pipeline::{BuildOutput, BuildPipeline, BuildStats};

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
