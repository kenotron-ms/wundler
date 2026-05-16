//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod build_id;
pub mod build_stats;
pub mod config;
pub mod dev_server;
pub mod output;
pub mod pipeline;

pub use build_stats::{
    BuildDelta, BuildStatsArtifact, BuildTiming, BudgetCheck, BudgetResult, BudgetStatus,
    ChunkRecord, ChunkRole, EntryPointRecord, PreviousBuildInfo, SizeDelta, SummaryBlock,
    TimingBlock,
};
pub use config::{BuildConfig, EngineChoice};
pub use dev_server::DevServer;
pub use pipeline::{BuildOutput, BuildPipeline, BuildStats};

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
