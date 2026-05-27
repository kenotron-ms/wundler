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
    pub schema_version: String,
    /// Content-based build identifier (mirrors `BuildOutput.manifest.build_id`).
    pub build_id: String,
    /// `env!("CARGO_PKG_VERSION")` of the `cloudpack-pipeline` crate.
    pub cloudpack_version: String,
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
    /// `true` if a `[budget]` section was present in `cloudpack.toml`.
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

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

impl BuildStatsArtifact {
    /// Construct the full `build-stats.json` artifact from a completed build.
    ///
    /// * `out`      — the output produced by `BuildPipeline::build()`
    /// * `previous` — the previous build's stats artifact, if available
    /// * `timing`   — per-phase wall-clock timings recorded by the pipeline
    pub fn from_build(
        out: &crate::pipeline::BuildOutput,
        previous: Option<&BuildStatsArtifact>,
        timing: BuildTiming,
    ) -> Self {
        let chunks = build_chunk_records(out);

        let total_bundle_bytes: u64 = chunks.iter().map(|c| c.size_bytes).sum();
        let largest_chunk_bytes: u64 = chunks.iter().map(|c| c.size_bytes).max().unwrap_or(0);
        let initial_bundle_bytes: u64 = chunks
            .iter()
            .filter(|c| c.role != ChunkRole::Lazy)
            .map(|c| c.size_bytes)
            .sum();

        let summary = SummaryBlock {
            total_modules: out.stats.total_modules as u32,
            alive_modules: out.stats.alive_modules as u32,
            dead_modules: out.stats.dead_modules as u32,
            chunks_written: out.stats.chunks_written as u32,
            total_bundle_bytes,
            largest_chunk_bytes,
            initial_bundle_bytes,
        };

        let timing_block = TimingBlock {
            build_time_ms: timing.total_ms,
            summarize_ms: timing.summarize_ms,
            analyze_ms: timing.analyze_ms,
            transform_ms: timing.transform_ms,
            emit_ms: timing.emit_ms,
        };

        let entry_points = build_entry_point_records(out, &chunks);

        let previous_build = previous.map(|prev| compute_previous_build_info(prev, &summary, &chunks));

        BuildStatsArtifact {
            schema_version: "1".to_string(),
            build_id: out.manifest.build_id.clone(),
            cloudpack_version: env!("CARGO_PKG_VERSION").to_string(),
            generated_at: chrono::Utc::now().to_rfc3339(),
            summary,
            timing: timing_block,
            chunks,
            entry_points,
            budget: None,
            previous_build,
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Build one [`ChunkRecord`] for every chunk in the manifest.
///
/// File sizes are determined by reading the on-disk file whose stem matches
/// the chunk's content hash (i.e. `chunks/<hash>.js`).
fn build_chunk_records(out: &crate::pipeline::BuildOutput) -> Vec<ChunkRecord> {
    use cloudpack_graph::types::LoadCondition;

    // Build a reverse index: chunk_id → list of entry-point names
    let mut chunk_to_entries: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for (entry, chunk_ids) in &out.manifest.entry_chunks {
        for chunk_id in chunk_ids {
            chunk_to_entries
                .entry(chunk_id.as_str())
                .or_default()
                .push(entry.as_str());
        }
    }

    out.manifest
        .chunks
        .iter()
        .map(|chunk| {
            // Locate the on-disk file whose stem equals the chunk's hash
            let file_path = out.chunk_files.iter().find(|p| {
                p.file_stem().and_then(|s| s.to_str()) == Some(chunk.hash.as_str())
            });

            let size_bytes = file_path
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);

            let module_count = chunk.modules.len() as u32;

            let entry_refs = chunk_to_entries
                .get(chunk.id.as_str())
                .cloned()
                .unwrap_or_default();

            let role = match chunk.load_condition {
                LoadCondition::Lazy | LoadCondition::Prefetch => ChunkRole::Lazy,
                LoadCondition::Initial => {
                    if entry_refs.len() >= 2 {
                        ChunkRole::Commons
                    } else {
                        ChunkRole::Entry
                    }
                }
            };

            let file = format!("chunks/{}.js", chunk.hash.as_str());

            let mut entry_points: Vec<String> =
                entry_refs.iter().map(|s| s.to_string()).collect();
            entry_points.sort();

            ChunkRecord {
                id: chunk.id.clone(),
                hash: chunk.hash.as_str().to_string(),
                file,
                size_bytes,
                module_count,
                role,
                entry_points,
            }
        })
        .collect()
}

/// Build the `entry_points` map for [`BuildStatsArtifact`].
///
/// For each named entry, reports:
/// * `initial_chunks` — the chunk IDs listed in `ChunkManifest::entry_chunks`
/// * `initial_bytes`  — sum of `size_bytes` for those initial chunks
/// * `lazy_chunks`    — all chunks with `ChunkRole::Lazy` (stub: associated
///   with every entry since finer granularity is not tracked in this schema)
fn build_entry_point_records(
    out: &crate::pipeline::BuildOutput,
    chunks: &[ChunkRecord],
) -> std::collections::HashMap<String, EntryPointRecord> {
    let lazy_chunk_ids: Vec<String> = chunks
        .iter()
        .filter(|c| c.role == ChunkRole::Lazy)
        .map(|c| c.id.clone())
        .collect();

    out.manifest
        .entry_chunks
        .iter()
        .map(|(entry, initial_ids)| {
            let initial_bytes: u64 = initial_ids
                .iter()
                .filter_map(|id| chunks.iter().find(|c| &c.id == id))
                .map(|c| c.size_bytes)
                .sum();

            let record = EntryPointRecord {
                initial_chunks: initial_ids.clone(),
                initial_bytes,
                lazy_chunks: lazy_chunk_ids.clone(),
            };

            (entry.clone(), record)
        })
        .collect()
}

fn compute_previous_build_info(
    prev: &BuildStatsArtifact,
    summary: &SummaryBlock,
    chunks: &[ChunkRecord],
) -> PreviousBuildInfo {
    // Compute size deltas
    let total_prev = prev.summary.total_bundle_bytes;
    let total_curr = summary.total_bundle_bytes;
    let total_delta = total_curr as i64 - total_prev as i64;
    let total_pct = if total_prev > 0 {
        (total_delta as f64 / total_prev as f64) * 100.0
    } else {
        0.0
    };

    let init_prev = prev.summary.initial_bundle_bytes;
    let init_curr = summary.initial_bundle_bytes;
    let init_delta = init_curr as i64 - init_prev as i64;
    let init_pct = if init_prev > 0 {
        (init_delta as f64 / init_prev as f64) * 100.0
    } else {
        0.0
    };

    // Compute chunk diffs via set operations on chunk ids
    let prev_ids: std::collections::HashSet<&str> =
        prev.chunks.iter().map(|c| c.id.as_str()).collect();
    let curr_ids: std::collections::HashSet<&str> =
        chunks.iter().map(|c| c.id.as_str()).collect();

    let chunks_added: Vec<String> = curr_ids
        .difference(&prev_ids)
        .map(|s| s.to_string())
        .collect();
    let chunks_removed: Vec<String> = prev_ids
        .difference(&curr_ids)
        .map(|s| s.to_string())
        .collect();

    PreviousBuildInfo {
        present: true,
        build_id: prev.build_id.clone(),
        delta: BuildDelta {
            total_bundle_bytes: SizeDelta {
                prev: total_prev,
                curr: total_curr,
                delta: total_delta,
                pct: (total_pct * 10.0).round() / 10.0,
            },
            initial_bundle_bytes: SizeDelta {
                prev: init_prev,
                curr: init_curr,
                delta: init_delta,
                pct: (init_pct * 10.0).round() / 10.0,
            },
            chunks_added,
            chunks_removed,
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;
    use cloudpack_core::types::ContentHash;
    use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};

    use crate::pipeline::{BuildOutput, BuildStats};

    // ------------------------------------------------------------------
    // Fixture helpers
    // ------------------------------------------------------------------

    /// Build a synthetic 2-chunk output (entry=1000 bytes Initial, lazy=500 bytes Lazy)
    /// with real files written to a temporary directory.
    fn make_fixture() -> (TempDir, BuildOutput) {
        let tmp = TempDir::new().unwrap();
        let out_dir = tmp.path();
        let chunks_dir = out_dir.join("chunks");
        std::fs::create_dir_all(&chunks_dir).unwrap();

        // Entry chunk: 1000-byte file, 2 modules, Initial load
        let entry_hash = "a".repeat(64);
        std::fs::write(chunks_dir.join(format!("{entry_hash}.js")), "x".repeat(1000)).unwrap();

        // Lazy chunk: 500-byte file, 1 module, Lazy load
        let lazy_hash = "b".repeat(64);
        std::fs::write(chunks_dir.join(format!("{lazy_hash}.js")), "y".repeat(500)).unwrap();

        let mod1 = ContentHash("c".repeat(64));
        let mod2 = ContentHash("d".repeat(64));
        let mod3 = ContentHash("e".repeat(64));

        let entry_chunk = Chunk {
            id: "entry".to_string(),
            modules: vec![mod1.clone(), mod2.clone()],
            hash: ContentHash(entry_hash.clone()),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        };

        let lazy_chunk = Chunk {
            id: "lazy".to_string(),
            modules: vec![mod3.clone()],
            hash: ContentHash(lazy_hash.clone()),
            load_condition: LoadCondition::Lazy,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        };

        let mut module_index = HashMap::new();
        module_index.insert(mod1, "entry".to_string());
        module_index.insert(mod2, "entry".to_string());
        module_index.insert(mod3, "lazy".to_string());

        let mut entry_chunks_map = HashMap::new();
        entry_chunks_map.insert("root".to_string(), vec!["entry".to_string()]);

        let manifest = ChunkManifest {
            build_id: "test-build-123".to_string(),
            chunks: vec![entry_chunk, lazy_chunk],
            entry_chunks: entry_chunks_map,
            module_index,
        };

        let stats = BuildStats {
            total_modules: 3,
            alive_modules: 3,
            dead_modules: 0,
            chunks_written: 2,
            build_time_ms: 1234,
            largest_chunk_bytes: 1000,
        };

        let entry_file = chunks_dir.join(format!("{entry_hash}.js"));
        let lazy_file = chunks_dir.join(format!("{lazy_hash}.js"));

        let out = BuildOutput {
            manifest,
            chunk_files: vec![entry_file, lazy_file],
            stats,
        };

        (tmp, out)
    }

    fn zero_timing() -> BuildTiming {
        BuildTiming {
            summarize_ms: 0,
            analyze_ms: 0,
            transform_ms: 0,
            emit_ms: 0,
            total_ms: 0,
        }
    }

    // ------------------------------------------------------------------
    // Test 1: summary + timing fields
    // ------------------------------------------------------------------

    #[test]
    fn from_build_populates_summary_and_timing() {
        let (_tmp, out) = make_fixture();
        let timing = BuildTiming {
            summarize_ms: 10,
            analyze_ms: 20,
            transform_ms: 30,
            emit_ms: 40,
            total_ms: 120,
        };

        let artifact = BuildStatsArtifact::from_build(&out, None, timing);

        assert_eq!(artifact.schema_version, "1");
        assert!(!artifact.build_id.is_empty());
        assert!(!artifact.cloudpack_version.is_empty());
        assert!(!artifact.generated_at.is_empty());

        let s = &artifact.summary;
        assert_eq!(s.total_modules, 3);
        assert_eq!(s.alive_modules, 3);
        assert_eq!(s.dead_modules, 0);
        assert_eq!(s.chunks_written, 2);
        assert_eq!(s.total_bundle_bytes, 1500);
        assert_eq!(s.largest_chunk_bytes, 1000);
        assert_eq!(s.initial_bundle_bytes, 1000);

        let t = &artifact.timing;
        assert_eq!(t.build_time_ms, 120);
        assert_eq!(t.summarize_ms, 10);
        assert_eq!(t.analyze_ms, 20);
        assert_eq!(t.transform_ms, 30);
        assert_eq!(t.emit_ms, 40);
    }

    // ------------------------------------------------------------------
    // Test 2: one ChunkRecord per chunk with correct role
    // ------------------------------------------------------------------

    #[test]
    fn from_build_emits_one_chunk_record_per_file_with_correct_role() {
        let (_tmp, out) = make_fixture();
        let artifact = BuildStatsArtifact::from_build(&out, None, zero_timing());

        assert_eq!(artifact.chunks.len(), 2);

        let entry = artifact.chunks.iter().find(|c| c.id == "entry").unwrap();
        assert_eq!(entry.size_bytes, 1000);
        assert_eq!(entry.module_count, 2);
        assert_eq!(entry.role, ChunkRole::Entry);
        assert_eq!(entry.entry_points, vec!["root".to_string()]);
        assert!(
            entry.file.starts_with("chunks/") && entry.file.ends_with(".js"),
            "unexpected file path: {}",
            entry.file,
        );

        let lazy = artifact.chunks.iter().find(|c| c.id == "lazy").unwrap();
        assert_eq!(lazy.size_bytes, 500);
        assert_eq!(lazy.module_count, 1);
        assert_eq!(lazy.role, ChunkRole::Lazy);
        assert!(lazy.entry_points.is_empty());
    }

    // ------------------------------------------------------------------
    // Test 3: entry point record
    // ------------------------------------------------------------------

    #[test]
    fn from_build_emits_entry_point_record() {
        let (_tmp, out) = make_fixture();
        let artifact = BuildStatsArtifact::from_build(&out, None, zero_timing());

        let root = artifact
            .entry_points
            .get("root")
            .expect("missing 'root' entry point");
        assert_eq!(root.initial_chunks, vec!["entry".to_string()]);
        assert_eq!(root.initial_bytes, 1000);
        assert_eq!(root.lazy_chunks, vec!["lazy".to_string()]);
    }

    // ------------------------------------------------------------------
    // Test 4: optional fields absent when not provided
    // ------------------------------------------------------------------

    #[test]
    fn from_build_omits_optional_fields_when_absent() {
        let (_tmp, out) = make_fixture();
        let artifact = BuildStatsArtifact::from_build(&out, None, zero_timing());

        assert!(artifact.budget.is_none());
        assert!(artifact.previous_build.is_none());

        let json = serde_json::to_string(&artifact).unwrap();
        assert!(
            !json.contains("\"budget\""),
            "JSON should not contain 'budget': {json}",
        );
        assert!(
            !json.contains("\"previous_build\""),
            "JSON should not contain 'previous_build': {json}",
        );
    }
}
