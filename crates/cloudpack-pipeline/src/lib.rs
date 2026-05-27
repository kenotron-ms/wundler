//! Cloudpack Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod build_id;
pub mod build_stats;
pub mod budget;
pub mod config;
pub mod dev_server;
pub mod output;
pub mod pipeline;

pub use build_stats::{
    BuildDelta, BuildStatsArtifact, BuildTiming, BudgetCheck, BudgetResult, BudgetStatus,
    ChunkRecord, ChunkRole, EntryPointRecord, PreviousBuildInfo, SizeDelta, SummaryBlock,
    TimingBlock,
};
pub use budget::{BudgetConfig, BudgetViolation};
pub use config::{BuildConfig, DevConfig, EngineChoice};
pub use dev_server::DevServer;
pub use pipeline::{BuildOutput, BuildPipeline};

pub fn hello() -> &'static str {
    "cloudpack-pipeline"
}
