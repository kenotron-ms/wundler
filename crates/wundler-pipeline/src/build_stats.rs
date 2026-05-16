//! On-disk schema for `build-stats.json` (schema_version = "1").
//!
//! Produced by [`BuildStatsArtifact::from_build`] after a successful build and
//! written atomically by [`crate::output::write_build_stats`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Per-phase timing — passed into `from_build`
// ---------------------------------------------------------------------------

/// Wall-clock timing for each pipeline phase, in milliseconds.
///
/// Recorded by `BuildPipeline::build` using `std::time::Instant` around each
/// step. `total_ms` is the elapsed time across the entire `build()` call and
/// will generally be `>= summarize_ms + analyze_ms + transform_ms + emit_ms`
/// (the difference is bookkeeping work like writing the manifest / html).
#[derive(Debug, Clone, Copy)]
pub struct BuildTiming {
    pub summarize_ms: u64,
    pub analyze_ms: u64,
    pub transform_ms: u64,
    pub emit_ms: u64,
    pub total_ms: u64,
}

// ---------------------------------------------------------------------------
// Top-level artifact
// ---------------------------------------------------------------------------

/// The full `build-stats.json` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildStatsArtifact {
    /// Always `"1"` for this schema.
    pub schema_version: &'static str,
    /// Content-based build identifier (mirrors `BuildOutput.manifest.build_id`).
    pub build_id: String,
    /// `env!("CARGO_PKG_VERSION")` of the `wundler-pipeline` crate.
    pub wundler_version: String,
    /// ISO-8601 UTC timestamp when the artifact was generated.
    pub generated_at: String,

    pub summary: SummaryBlock,
    pub timing: TimingBlock,
    pub chunks: Vec<ChunkRecord>,
    pub entry_points: HashMap<String, EntryPointRecord>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_build: Option<PreviousBuildInfo>,
}

// ---------------------------------------------------------------------------
// Sub-blocks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryBlock {
    pub total_modules: u32,
    pub alive_modules: u32,
    pub dead_modules: u32,
    pub chunks_written: u32,
    pub total_bundle_bytes: u64,
    pub largest_chunk_bytes: u64,
    /// Sum of file sizes of chunks whose `LoadCondition` is `Initial`.
    pub initial_bundle_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingBlock {
    pub build_time_ms: u64,
    pub summarize_ms: u64,
    pub analyze_ms: u64,
    pub transform_ms: u64,
    pub emit_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub id: String,
    pub hash: String,
    /// Relative path from `out_dir`, e.g. `"chunks/<hash>.js"`.
    pub file: String,
    pub size_bytes: u64,
    pub module_count: u32,
    pub role: ChunkRole,
    /// Entry points whose `entry_chunks` list contains this chunk's id.
    pub entry_points: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkRole {
    /// Initial chunk referenced by at least one entry point.
    Entry,
    /// `LoadCondition::Lazy`.
    Lazy,
    /// Initial chunk shared across multiple entry points (commons-extracted).
    Commons,
    /// Chunk containing only dead modules.
    Dead,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPointRecord {
    pub initial_chunks: Vec<String>,
    pub initial_bytes: u64,
    pub lazy_chunks: Vec<String>,
}

// ---------------------------------------------------------------------------
// Budget result types — populated by `crate::budget::check`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetResult {
    /// `true` if a `[budget]` section was present in `wundler.toml`.
    pub configured: bool,
    pub checks: Vec<BudgetCheck>,
    pub result: BudgetStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStatus {
    Ok,
    Violated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetCheck {
    /// Logical name of the budget, e.g. `"initial_bundle_max_bytes"`.
    pub name: String,
    pub limit: u64,
    pub actual: u64,
    pub status: BudgetStatus,
    /// Chunk id for per-chunk violations; `None` for aggregate checks.
    pub offender: Option<String>,
}

// ---------------------------------------------------------------------------
// Delta vs previous build
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviousBuildInfo {
    /// Always `true` when this struct is emitted (absent → not present).
    pub present: bool,
    pub build_id: String,
    pub delta: BuildDelta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDelta {
    pub total_bundle_bytes: SizeDelta,
    pub initial_bundle_bytes: SizeDelta,
    pub chunks_added: Vec<String>,
    pub chunks_removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeDelta {
    pub prev: u64,
    pub curr: u64,
    pub delta: i64,
    /// Percentage change relative to `prev`. `0.0` when `prev == 0`.
    pub pct: f64,
}
