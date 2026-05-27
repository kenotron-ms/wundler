# Observability P1 Full — Extended BuildStats + Budget Enforcement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the basic `BuildStats` written to `build-stats.json` with a versioned `BuildStatsArtifact` that includes per-phase timing, per-chunk role/size records, per-entry-point initial/lazy chunk breakdowns, optional budget enforcement, and an optional delta vs. the previous build.

**Architecture:** A new `build_stats` module owns the on-disk schema (`BuildStatsArtifact`, sub-blocks, and a `from_build()` constructor). A new `budget` module owns opt-in size enforcement (`BudgetConfig`, `check()`, `BudgetViolation` with actionable messages). `BuildPipeline::build()` is instrumented with `Instant`-based per-phase timing, reads any previous `build-stats.json` for delta computation, constructs the artifact, atomically writes it to disk, then runs an opt-in budget check (non-fatal warning in this plan — CLI exit-1 wiring is a deferred follow-up).

**Tech Stack:** Rust 2021, `serde` + `serde_json`, `chrono` (UTC timestamps), `anyhow`, `tempfile` (dev-deps), `cloudpack-graph::types::ChunkManifest` + `LoadCondition`.

---

## File Structure

**Create:**
- `crates/cloudpack-pipeline/src/build_stats.rs` — schema types + constructor
- `crates/cloudpack-pipeline/src/budget.rs` — budget config + checker
- `crates/cloudpack-pipeline/tests/build_stats_test.rs` — integration tests for artifact
- `crates/cloudpack-pipeline/tests/budget_test.rs` — integration tests for budget

**Modify:**
- `crates/cloudpack-pipeline/Cargo.toml` — add `chrono` dependency
- `crates/cloudpack-pipeline/src/lib.rs` — export new modules
- `crates/cloudpack-pipeline/src/config.rs` — add `budget: Option<BudgetConfig>` field
- `crates/cloudpack-pipeline/src/output.rs` — add atomic `write_build_stats` + `read_previous_stats`
- `crates/cloudpack-pipeline/src/pipeline.rs` — replace old `BuildStats`/stats-write logic with `BuildStatsArtifact` + `BuildTiming` instrumentation + budget call

**Delete (logically):** the old `pub struct BuildStats` and its inline write in `pipeline.rs` (replaced by `BuildStatsArtifact`). The struct is removed entirely — there are no external consumers of the old type besides the re-export in `lib.rs`.

---

## Task 1: Add `chrono` and scaffold `build_stats.rs` with all types

**Files:**
- Modify: `crates/cloudpack-pipeline/Cargo.toml`
- Create: `crates/cloudpack-pipeline/src/build_stats.rs`
- Modify: `crates/cloudpack-pipeline/src/lib.rs`

- [ ] **Step 1: Add `chrono` to `Cargo.toml` dependencies**

Open `crates/cloudpack-pipeline/Cargo.toml` and add to the `[dependencies]` section (anywhere among other deps):

```toml
chrono = { version = "0.4", default-features = false, features = ["clock", "serde"] }
```

- [ ] **Step 2: Verify the crate still builds with the new dep**

Run: `~/.cargo/bin/cargo build -p cloudpack-pipeline`
Expected: builds cleanly (chrono compiles, no other changes yet).

- [ ] **Step 3: Create `build_stats.rs` with the full schema (no constructor body yet)**

Create `crates/cloudpack-pipeline/src/build_stats.rs`:

```rust
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
    /// Chunk containing only dead modules. (Reserved for future use; the
    /// current pipeline does not emit dead chunks, but the role is part of the
    /// stable schema.)
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
```

- [ ] **Step 4: Register module + re-exports in `lib.rs`**

The current `crates/cloudpack-pipeline/src/lib.rs` is:

```rust
//! Cloudpack Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod build_id;
pub mod config;
pub mod dev_server;
pub mod output;
pub mod pipeline;

pub use config::{BuildConfig, EngineChoice};
pub use dev_server::DevServer;
pub use pipeline::{BuildOutput, BuildPipeline, BuildStats};

pub fn hello() -> &'static str {
    "cloudpack-pipeline"
}
```

Replace it with:

```rust
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
pub use config::{BuildConfig, EngineChoice};
pub use dev_server::DevServer;
pub use pipeline::{BuildOutput, BuildPipeline};

pub fn hello() -> &'static str {
    "cloudpack-pipeline"
}
```

Note: `BuildStats` is no longer exported (removed in Task 4). `budget` module is created in Task 5 — for now this re-export will not compile. To keep the crate compiling between tasks, temporarily drop the `budget` line:

```rust
// (drop the budget line for now — re-added in Task 5)
// pub use budget::{BudgetConfig, BudgetViolation};
```

And drop `pub mod budget;` until Task 5. Also keep `pub use pipeline::{BuildOutput, BuildPipeline, BuildStats};` until Task 4 actually removes `BuildStats`. So the **transitional** lib.rs for end of Task 1 is:

```rust
//! Cloudpack Build Pipeline — orchestrates summarize → analyze → transform → emit.

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
    "cloudpack-pipeline"
}
```

- [ ] **Step 5: Verify the crate builds**

Run: `~/.cargo/bin/cargo build -p cloudpack-pipeline`
Expected: builds cleanly. No tests yet for the new types — they are pure data shapes.

- [ ] **Step 6: Verify clippy is clean**

Run: `~/.cargo/bin/cargo clippy -p cloudpack-pipeline -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/cloudpack-pipeline/Cargo.toml \
        crates/cloudpack-pipeline/src/build_stats.rs \
        crates/cloudpack-pipeline/src/lib.rs
git commit -m "feat(pipeline): scaffold BuildStatsArtifact schema (schema_version=1)"
```

---

## Task 2: `BuildStatsArtifact::from_build()` constructor + unit tests

**Files:**
- Modify: `crates/cloudpack-pipeline/src/build_stats.rs` (add constructor + unit tests)
- (No external test file yet — these are in-module unit tests against simple inputs.)

- [ ] **Step 1: Write failing unit tests for `from_build` (no previous build)**

Append the following test module at the bottom of `crates/cloudpack-pipeline/src/build_stats.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::path::PathBuf;

    use cloudpack_core::types::{ContentHash, ModuleKind};
    use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};

    use crate::pipeline::{BuildOutput, BuildStats};

    /// Build a synthetic 2-chunk manifest:
    ///   - chunk "entry"  (Initial, modules m1, m2)   → file chunks/<h1>.js
    ///   - chunk "lazy"   (Lazy,    modules m3)       → file chunks/<h2>.js
    fn fixture(tmp: &std::path::Path) -> BuildOutput {
        std::fs::create_dir_all(tmp.join("chunks")).unwrap();

        let h1 = ContentHash::from_hex("a".repeat(64)).unwrap();
        let h2 = ContentHash::from_hex("b".repeat(64)).unwrap();
        let m1 = ContentHash::from_hex("1".repeat(64)).unwrap();
        let m2 = ContentHash::from_hex("2".repeat(64)).unwrap();
        let m3 = ContentHash::from_hex("3".repeat(64)).unwrap();

        // Write deterministic bytes so size_bytes is predictable.
        let f1 = tmp.join("chunks").join(format!("{}.js", h1.as_str()));
        let f2 = tmp.join("chunks").join(format!("{}.js", h2.as_str()));
        std::fs::write(&f1, vec![b'x'; 1000]).unwrap();
        std::fs::write(&f2, vec![b'y'; 500]).unwrap();

        let entry = Chunk {
            id: "entry".to_string(),
            modules: vec![m1.clone(), m2.clone()],
            hash: h1.clone(),
            load_condition: LoadCondition::Initial,
            co_request_score: 0.0,
            median_load_order: 0.0,
            suggested_merge: None,
        };
        let lazy = Chunk {
            id: "lazy".to_string(),
            modules: vec![m3.clone()],
            hash: h2.clone(),
            load_condition: LoadCondition::Lazy,
            co_request_score: 0.0,
            median_load_order: 0.0,
            suggested_merge: None,
        };

        let mut entry_chunks: HashMap<String, Vec<String>> = HashMap::new();
        entry_chunks.insert("root".to_string(), vec!["entry".to_string()]);

        let manifest = ChunkManifest {
            chunks: vec![entry, lazy],
            entry_chunks,
            build_id: "deadbeefdeadbeef".to_string(),
            // The remaining fields in ChunkManifest, if any, take Default.
            ..Default::default()
        };

        BuildOutput {
            manifest,
            chunk_files: vec![f1, f2],
            stats: BuildStats {
                total_modules: 3,
                alive_modules: 3,
                dead_modules: 0,
                chunks_written: 2,
                build_time_ms: 0,
                largest_chunk_bytes: 1000,
            },
        }
    }

    fn timing() -> BuildTiming {
        BuildTiming {
            summarize_ms: 10,
            analyze_ms: 20,
            transform_ms: 30,
            emit_ms: 40,
            total_ms: 110,
        }
    }

    #[test]
    fn from_build_populates_summary_and_timing() {
        let tmp = tempfile::tempdir().unwrap();
        let out = fixture(tmp.path());
        let artifact = BuildStatsArtifact::from_build(&out, None, timing());

        assert_eq!(artifact.schema_version, "1");
        assert_eq!(artifact.build_id, "deadbeefdeadbeef");
        assert!(!artifact.cloudpack_version.is_empty());
        assert!(!artifact.generated_at.is_empty());

        // Summary
        assert_eq!(artifact.summary.total_modules, 3);
        assert_eq!(artifact.summary.alive_modules, 3);
        assert_eq!(artifact.summary.dead_modules, 0);
        assert_eq!(artifact.summary.chunks_written, 2);
        assert_eq!(artifact.summary.total_bundle_bytes, 1500);
        assert_eq!(artifact.summary.largest_chunk_bytes, 1000);
        assert_eq!(artifact.summary.initial_bundle_bytes, 1000);

        // Timing block mirrors BuildTiming with build_time_ms = total_ms.
        assert_eq!(artifact.timing.build_time_ms, 110);
        assert_eq!(artifact.timing.summarize_ms, 10);
        assert_eq!(artifact.timing.analyze_ms, 20);
        assert_eq!(artifact.timing.transform_ms, 30);
        assert_eq!(artifact.timing.emit_ms, 40);
    }

    #[test]
    fn from_build_emits_one_chunk_record_per_file_with_correct_role() {
        let tmp = tempfile::tempdir().unwrap();
        let out = fixture(tmp.path());
        let artifact = BuildStatsArtifact::from_build(&out, None, timing());

        assert_eq!(artifact.chunks.len(), 2);

        let entry = artifact.chunks.iter().find(|c| c.id == "entry").unwrap();
        assert_eq!(entry.size_bytes, 1000);
        assert_eq!(entry.module_count, 2);
        assert_eq!(entry.role, ChunkRole::Entry);
        assert_eq!(entry.entry_points, vec!["root".to_string()]);
        assert!(entry.file.starts_with("chunks/"));
        assert!(entry.file.ends_with(".js"));

        let lazy = artifact.chunks.iter().find(|c| c.id == "lazy").unwrap();
        assert_eq!(lazy.size_bytes, 500);
        assert_eq!(lazy.module_count, 1);
        assert_eq!(lazy.role, ChunkRole::Lazy);
        assert!(lazy.entry_points.is_empty());
    }

    #[test]
    fn from_build_emits_entry_point_record() {
        let tmp = tempfile::tempdir().unwrap();
        let out = fixture(tmp.path());
        let artifact = BuildStatsArtifact::from_build(&out, None, timing());

        let rec = artifact.entry_points.get("root").expect("root present");
        assert_eq!(rec.initial_chunks, vec!["entry".to_string()]);
        assert_eq!(rec.initial_bytes, 1000);
        assert_eq!(rec.lazy_chunks, vec!["lazy".to_string()]);
    }

    #[test]
    fn from_build_omits_optional_fields_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let out = fixture(tmp.path());
        let artifact = BuildStatsArtifact::from_build(&out, None, timing());

        assert!(artifact.budget.is_none());
        assert!(artifact.previous_build.is_none());

        // Confirm JSON serialization drops the fields (serde skip_if).
        let json = serde_json::to_string(&artifact).unwrap();
        assert!(!json.contains("\"budget\""));
        assert!(!json.contains("\"previous_build\""));
    }

    // Suppress unused warnings — `ModuleKind` only kept as a reminder for future
    // tests that touch graph nodes directly.
    fn _module_kind_keepalive(_k: ModuleKind) {}

    // Placeholder used so PathBuf import stays live in case fixture evolves.
    fn _pathbuf_keepalive(_p: PathBuf) {}
}
```

- [ ] **Step 2: Run the test and verify failure (`from_build` not yet defined)**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --lib build_stats::tests`
Expected: FAIL with `error[E0599]: no function or associated item named 'from_build' found`.

- [ ] **Step 3: Implement `from_build` (and a helper for `ChunkRole`)**

Insert this `impl` block in `crates/cloudpack-pipeline/src/build_stats.rs`, immediately after the type definitions (before the `#[cfg(test)]` module):

```rust
// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

use std::fs;
use std::path::Path;

use cloudpack_graph::types::LoadCondition;

use crate::pipeline::BuildOutput;

impl BuildStatsArtifact {
    /// Build an artifact from a completed [`BuildOutput`].
    ///
    /// * `out`      — the just-completed build (manifest + chunk file paths).
    /// * `previous` — the previous build's artifact, if any, used for delta.
    /// * `timing`   — wall-clock timings recorded by the caller.
    pub fn from_build(
        out: &BuildOutput,
        previous: Option<&BuildStatsArtifact>,
        timing: BuildTiming,
    ) -> Self {
        // --- chunk records (one per file in out.chunk_files) ---
        let chunks = build_chunk_records(out);

        // --- summary ---
        let total_bundle_bytes: u64 = chunks.iter().map(|c| c.size_bytes).sum();
        let largest_chunk_bytes: u64 = chunks.iter().map(|c| c.size_bytes).max().unwrap_or(0);
        let initial_bundle_bytes: u64 = chunks
            .iter()
            .filter(|c| matches!(c.role, ChunkRole::Entry | ChunkRole::Commons))
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

        // --- timing ---
        let timing_block = TimingBlock {
            build_time_ms: timing.total_ms,
            summarize_ms: timing.summarize_ms,
            analyze_ms: timing.analyze_ms,
            transform_ms: timing.transform_ms,
            emit_ms: timing.emit_ms,
        };

        // --- entry-point records ---
        let entry_points = build_entry_point_records(out, &chunks);

        // --- delta vs previous build (computed in Task 6) ---
        let previous_build = previous.map(|prev| compute_previous_build_info(prev, &summary, &chunks));

        Self {
            schema_version: "1",
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
// Internal helpers
// ---------------------------------------------------------------------------

/// Build one [`ChunkRecord`] per file in `out.chunk_files`.
///
/// For each file we resolve the matching [`cloudpack_graph::types::Chunk`] from
/// the manifest by hash. Files whose hash does not match any manifest chunk
/// are skipped (defensive — this should not happen in practice).
fn build_chunk_records(out: &BuildOutput) -> Vec<ChunkRecord> {
    // Pre-compute: which entry points reference each chunk id?
    let mut entry_points_by_chunk: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (entry, chunk_ids) in &out.manifest.entry_chunks {
        for cid in chunk_ids {
            entry_points_by_chunk
                .entry(cid.clone())
                .or_default()
                .push(entry.clone());
        }
    }
    for v in entry_points_by_chunk.values_mut() {
        v.sort();
        v.dedup();
    }

    let mut records = Vec::with_capacity(out.chunk_files.len());

    for path in &out.chunk_files {
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        // Match the manifest chunk whose hash hex == file stem.
        let chunk = out
            .manifest
            .chunks
            .iter()
            .find(|c| c.hash.as_str() == stem);

        let (id, module_count, load_condition) = match chunk {
            Some(c) => (c.id.clone(), c.modules.len() as u32, c.load_condition.clone()),
            None => {
                // File present but not in manifest — record it anyway with id = stem.
                (stem.clone(), 0u32, LoadCondition::Initial)
            }
        };

        let eps = entry_points_by_chunk.get(&id).cloned().unwrap_or_default();
        let role = classify_role(&load_condition, &eps);

        let size_bytes = chunk_file_size(path);

        records.push(ChunkRecord {
            id,
            hash: stem.clone(),
            file: format!("chunks/{file_name}"),
            size_bytes,
            module_count,
            role,
            entry_points: eps,
        });
    }

    records
}

fn classify_role(load_condition: &LoadCondition, entry_points: &[String]) -> ChunkRole {
    match load_condition {
        LoadCondition::Lazy => ChunkRole::Lazy,
        LoadCondition::Initial => {
            if entry_points.len() >= 2 {
                ChunkRole::Commons
            } else {
                ChunkRole::Entry
            }
        }
    }
}

fn chunk_file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Build per-entry-point initial/lazy summaries.
///
/// `initial_chunks` comes from `manifest.entry_chunks[entry]`.
/// `lazy_chunks` is every manifest chunk with `LoadCondition::Lazy` (we treat
/// lazy chunks as globally reachable from any entry — the entry-point-specific
/// lazy graph is not yet tracked in the manifest).
fn build_entry_point_records(
    out: &BuildOutput,
    records: &[ChunkRecord],
) -> HashMap<String, EntryPointRecord> {
    let by_id: HashMap<&str, &ChunkRecord> =
        records.iter().map(|r| (r.id.as_str(), r)).collect();

    let lazy_ids: Vec<String> = out
        .manifest
        .chunks
        .iter()
        .filter(|c| matches!(c.load_condition, LoadCondition::Lazy))
        .map(|c| c.id.clone())
        .collect();

    let mut map = HashMap::new();
    for (entry, chunk_ids) in &out.manifest.entry_chunks {
        let initial_bytes: u64 = chunk_ids
            .iter()
            .filter_map(|cid| by_id.get(cid.as_str()))
            .map(|r| r.size_bytes)
            .sum();

        map.insert(
            entry.clone(),
            EntryPointRecord {
                initial_chunks: chunk_ids.clone(),
                initial_bytes,
                lazy_chunks: lazy_ids.clone(),
            },
        );
    }
    map
}

// Filled in by Task 6 — stub for now so `from_build` compiles.
fn compute_previous_build_info(
    _prev: &BuildStatsArtifact,
    _summary: &SummaryBlock,
    _chunks: &[ChunkRecord],
) -> PreviousBuildInfo {
    PreviousBuildInfo {
        present: true,
        build_id: String::new(),
        delta: BuildDelta {
            total_bundle_bytes: SizeDelta { prev: 0, curr: 0, delta: 0, pct: 0.0 },
            initial_bundle_bytes: SizeDelta { prev: 0, curr: 0, delta: 0, pct: 0.0 },
            chunks_added: Vec::new(),
            chunks_removed: Vec::new(),
        },
    }
}
```

- [ ] **Step 4: Verify the `ChunkManifest::default()` requirement**

The fixture uses `..Default::default()` on `ChunkManifest`. Verify the type implements `Default`:

Run: `~/.cargo/bin/cargo build -p cloudpack-pipeline --tests 2>&1 | head -60`

If you see `the trait 'Default' is not implemented for 'ChunkManifest'`, change the fixture to construct the manifest explicitly without the spread (the fields we set — `chunks`, `entry_chunks`, `build_id` — are the only ones used by `from_build`, so if other fields exist you must provide them too). Read `crates/cloudpack-graph/src/types.rs` (`grep -n "pub struct ChunkManifest" crates/cloudpack-graph/src/types.rs`) and adjust the fixture to set every required field with zero/empty values. Expected: build succeeds.

- [ ] **Step 5: Run the new tests and verify they pass**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --lib build_stats::tests`
Expected: 4 tests pass.

- [ ] **Step 6: Verify clippy is clean**

Run: `~/.cargo/bin/cargo clippy -p cloudpack-pipeline --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/cloudpack-pipeline/src/build_stats.rs
git commit -m "feat(pipeline): BuildStatsArtifact::from_build constructor + tests"
```

---

## Task 3: Atomic `write_build_stats`, `read_previous_stats`, and `BuildTiming` plumbing readiness

**Files:**
- Modify: `crates/cloudpack-pipeline/src/output.rs` (add `write_build_stats` + `read_previous_stats`)
- Create: `crates/cloudpack-pipeline/tests/build_stats_io_test.rs`

- [ ] **Step 1: Write a failing test for atomic `write_build_stats` + `read_previous_stats` round-trip**

Create `crates/cloudpack-pipeline/tests/build_stats_io_test.rs`:

```rust
//! Integration tests for `output::write_build_stats` (atomic) and
//! `output::read_previous_stats`.

use std::collections::HashMap;

use cloudpack_pipeline::build_stats::{
    BuildStatsArtifact, ChunkRecord, ChunkRole, EntryPointRecord, SummaryBlock, TimingBlock,
};
use cloudpack_pipeline::output;

fn sample_artifact(build_id: &str) -> BuildStatsArtifact {
    let mut entry_points = HashMap::new();
    entry_points.insert(
        "root".to_string(),
        EntryPointRecord {
            initial_chunks: vec!["entry".to_string()],
            initial_bytes: 1000,
            lazy_chunks: vec![],
        },
    );

    BuildStatsArtifact {
        schema_version: "1",
        build_id: build_id.to_string(),
        cloudpack_version: "0.0.0-test".to_string(),
        generated_at: "2026-05-16T00:00:00+00:00".to_string(),
        summary: SummaryBlock {
            total_modules: 1,
            alive_modules: 1,
            dead_modules: 0,
            chunks_written: 1,
            total_bundle_bytes: 1000,
            largest_chunk_bytes: 1000,
            initial_bundle_bytes: 1000,
        },
        timing: TimingBlock {
            build_time_ms: 100,
            summarize_ms: 10,
            analyze_ms: 20,
            transform_ms: 30,
            emit_ms: 40,
        },
        chunks: vec![ChunkRecord {
            id: "entry".to_string(),
            hash: "a".repeat(64),
            file: format!("chunks/{}.js", "a".repeat(64)),
            size_bytes: 1000,
            module_count: 1,
            role: ChunkRole::Entry,
            entry_points: vec!["root".to_string()],
        }],
        entry_points,
        budget: None,
        previous_build: None,
    }
}

#[test]
fn write_then_read_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let artifact = sample_artifact("buildA");

    output::write_build_stats(tmp.path(), &artifact).expect("write succeeded");

    let read = output::read_previous_stats(tmp.path()).expect("file present");
    assert_eq!(read.schema_version, "1");
    assert_eq!(read.build_id, "buildA");
    assert_eq!(read.summary.total_bundle_bytes, 1000);
    assert_eq!(read.chunks.len(), 1);
    assert_eq!(read.chunks[0].id, "entry");
}

#[test]
fn write_is_atomic_no_tmp_file_left_behind() {
    let tmp = tempfile::tempdir().unwrap();
    output::write_build_stats(tmp.path(), &sample_artifact("idA")).unwrap();

    // After a successful write, only `build-stats.json` should exist —
    // no `.tmp` / `.partial` left over.
    let mut names: Vec<String> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    assert_eq!(names, vec!["build-stats.json".to_string()]);
}

#[test]
fn write_overwrites_existing_file() {
    let tmp = tempfile::tempdir().unwrap();

    output::write_build_stats(tmp.path(), &sample_artifact("idA")).unwrap();
    output::write_build_stats(tmp.path(), &sample_artifact("idB")).unwrap();

    let read = output::read_previous_stats(tmp.path()).unwrap();
    assert_eq!(read.build_id, "idB");
}

#[test]
fn read_previous_stats_returns_none_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(output::read_previous_stats(tmp.path()).is_none());
}

#[test]
fn read_previous_stats_returns_none_for_corrupt_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("build-stats.json"), b"{not json").unwrap();
    assert!(output::read_previous_stats(tmp.path()).is_none());
}
```

- [ ] **Step 2: Run the test and verify failure**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --test build_stats_io_test`
Expected: FAIL with `no function or associated item named 'write_build_stats' / 'read_previous_stats'`.

- [ ] **Step 3: Add `write_build_stats` and `read_previous_stats` to `output.rs`**

At the bottom of `crates/cloudpack-pipeline/src/output.rs`, append:

```rust
// ---------------------------------------------------------------------------
// write_build_stats — atomic
// ---------------------------------------------------------------------------

use crate::build_stats::BuildStatsArtifact;

/// Atomically write the build-stats artifact to `<out_dir>/build-stats.json`.
///
/// Strategy: write JSON to `<out_dir>/build-stats.json.tmp`, then `rename` it
/// into place. The rename is atomic on every platform supported by Cloudpack
/// (POSIX guarantees this; on Windows NTFS the rename is also atomic when the
/// destination is on the same volume).
pub fn write_build_stats(out_dir: &Path, stats: &BuildStatsArtifact) -> Result<()> {
    fs::create_dir_all(out_dir)?;

    let final_path = out_dir.join("build-stats.json");
    let tmp_path = out_dir.join("build-stats.json.tmp");

    let json = serde_json::to_string_pretty(stats)?;
    fs::write(&tmp_path, json)?;
    fs::rename(&tmp_path, &final_path)?;

    Ok(())
}

/// Read `<out_dir>/build-stats.json` if present and parseable.
///
/// Returns `None` on any error (missing file, IO error, JSON parse error) —
/// the caller treats missing/corrupt previous stats as "no previous build".
pub fn read_previous_stats(out_dir: &Path) -> Option<BuildStatsArtifact> {
    let path = out_dir.join("build-stats.json");
    let bytes = fs::read(&path).ok()?;
    serde_json::from_slice::<BuildStatsArtifact>(&bytes).ok()
}
```

- [ ] **Step 4: Run the test and verify it passes**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --test build_stats_io_test`
Expected: all 5 tests pass.

- [ ] **Step 5: Verify clippy is clean**

Run: `~/.cargo/bin/cargo clippy -p cloudpack-pipeline --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/cloudpack-pipeline/src/output.rs \
        crates/cloudpack-pipeline/tests/build_stats_io_test.rs
git commit -m "feat(pipeline): atomic write_build_stats + read_previous_stats"
```

---

## Task 4: Wire `BuildStatsArtifact` into `BuildPipeline::build()`; instrument per-phase timing; remove old `BuildStats`

**Files:**
- Modify: `crates/cloudpack-pipeline/src/pipeline.rs`
- Modify: `crates/cloudpack-pipeline/src/lib.rs` (drop `BuildStats` re-export)
- Modify: `crates/cloudpack-pipeline/src/build_stats.rs` (fixture inside test module no longer relies on the old `BuildStats` field — replace with a small inline struct)
- Create: `crates/cloudpack-pipeline/tests/build_pipeline_stats_test.rs`

This task is the biggest single change in the plan. The full updated `pipeline.rs` is given in Step 3.

- [ ] **Step 1: Write an integration test that runs `BuildPipeline::build()` and asserts the artifact on disk**

Create `crates/cloudpack-pipeline/tests/build_pipeline_stats_test.rs`:

```rust
//! End-to-end test: run BuildPipeline::build() and verify build-stats.json
//! contains the full extended schema.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use cloudpack_pipeline::build_stats::BuildStatsArtifact;
use cloudpack_pipeline::config::{BuildConfig, EngineChoice};
use cloudpack_pipeline::pipeline::BuildPipeline;

/// Build a trivial single-entry project rooted at `tmp/src` and bundle it.
fn run_build(tmp: &std::path::Path) -> BuildStatsArtifact {
    let root = tmp.join("src");
    let out_dir = tmp.join("dist");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("index.js"), "export const x = 1;\n").unwrap();

    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    entry_points.insert("root".to_string(), root.join("index.js"));

    let cfg = BuildConfig {
        root,
        out_dir: out_dir.clone(),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points,
        budget: None,
    };

    let pipeline = BuildPipeline::new(cfg);
    let out = pipeline.build().expect("build succeeds");

    // Verify the artifact was written.
    let raw = fs::read(out_dir.join("build-stats.json")).expect("build-stats.json present");
    let artifact: BuildStatsArtifact = serde_json::from_slice(&raw).expect("parses");

    // Also confirm BuildOutput is consistent (smoke check).
    assert!(!out.chunk_files.is_empty());

    artifact
}

#[test]
fn build_writes_extended_stats_with_schema_version_1() {
    let tmp = tempfile::tempdir().unwrap();
    let a = run_build(tmp.path());

    assert_eq!(a.schema_version, "1");
    assert!(!a.build_id.is_empty());
    assert!(!a.cloudpack_version.is_empty());
    assert!(!a.generated_at.is_empty());
}

#[test]
fn build_stats_summary_matches_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let a = run_build(tmp.path());

    // total_bundle_bytes == sum of every chunk record size_bytes
    let sum: u64 = a.chunks.iter().map(|c| c.size_bytes).sum();
    assert_eq!(a.summary.total_bundle_bytes, sum);

    // largest_chunk_bytes == max of chunk sizes
    let max: u64 = a.chunks.iter().map(|c| c.size_bytes).max().unwrap_or(0);
    assert_eq!(a.summary.largest_chunk_bytes, max);

    // chunks_written == number of chunk records
    assert_eq!(a.summary.chunks_written as usize, a.chunks.len());
}

#[test]
fn build_stats_includes_entry_point_record() {
    let tmp = tempfile::tempdir().unwrap();
    let a = run_build(tmp.path());

    let rec = a.entry_points.get("root").expect("root entry present");
    assert!(!rec.initial_chunks.is_empty());
    assert!(rec.initial_bytes > 0);
}

#[test]
fn build_stats_chunk_size_matches_filesystem() {
    let tmp = tempfile::tempdir().unwrap();
    let a = run_build(tmp.path());

    for c in &a.chunks {
        let on_disk = tmp.path().join("dist").join(&c.file);
        let actual = fs::metadata(&on_disk).unwrap().len();
        assert_eq!(c.size_bytes, actual, "chunk {} size mismatch", c.id);
    }
}

#[test]
fn build_stats_timing_is_populated() {
    let tmp = tempfile::tempdir().unwrap();
    let a = run_build(tmp.path());

    // total_ms is the wall-clock of the whole build — must be >= the sum of
    // per-phase timings (which omit bookkeeping).
    let sum_phases =
        a.timing.summarize_ms + a.timing.analyze_ms + a.timing.transform_ms + a.timing.emit_ms;
    assert!(
        a.timing.build_time_ms >= sum_phases,
        "build_time_ms ({}) < phase sum ({})",
        a.timing.build_time_ms,
        sum_phases
    );
}
```

- [ ] **Step 2: Run the test and verify failure**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --test build_pipeline_stats_test`
Expected: FAIL — `BuildConfig` has no `budget` field, and the test imports types that aren't yet wired in.

- [ ] **Step 3: Update `pipeline.rs` end-to-end**

Replace the **entire** contents of `crates/cloudpack-pipeline/src/pipeline.rs` with:

```rust
//! `BuildPipeline` — orchestrates the full build from summarize → analyze → transform → emit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};

use cloudpack_core::cache::local::LocalCache;
use cloudpack_core::summarizer::summarize_directory;
use cloudpack_core::types::{BundleGraphNode, ContentHash};
use cloudpack_graph::analyzer::{AnalysisResult, GraphAnalyzer};
use cloudpack_graph::types::ChunkManifest;
use cloudpack_transform::engine::{BatchConfig, ChunkOutput, TransformEngine};
use cloudpack_transform::rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};
use cloudpack_transform::swc_adapter::{SwcAdapterConfig, SwcTransformAdapter};

use crate::build_id;
use crate::build_stats::{BuildStatsArtifact, BuildTiming};
use crate::config::{BuildConfig, EngineChoice};
use crate::output;

// ---------------------------------------------------------------------------
// Build output types
// ---------------------------------------------------------------------------

/// Lightweight summary returned alongside [`BuildOutput`]. Identical fields to
/// the legacy `BuildStats` but kept internal — the full versioned stats live
/// in `build-stats.json` via [`BuildStatsArtifact`].
#[derive(Debug, Clone)]
pub struct BuildStats {
    pub total_modules: usize,
    pub alive_modules: usize,
    pub dead_modules: usize,
    pub chunks_written: usize,
    pub build_time_ms: u128,
    pub largest_chunk_bytes: u64,
}

/// The result returned by [`BuildPipeline::build()`].
#[derive(Debug)]
pub struct BuildOutput {
    pub manifest: ChunkManifest,
    pub chunk_files: Vec<PathBuf>,
    pub stats: BuildStats,
}

/// Orchestrates the Cloudpack build pipeline: summarize → analyze → transform → emit.
pub struct BuildPipeline {
    pub config: BuildConfig,
    pub engine: Arc<dyn TransformEngine>,
}

impl BuildPipeline {
    pub fn new(config: BuildConfig) -> Self {
        let engine: Arc<dyn TransformEngine> = match config.engine {
            EngineChoice::Swc => Arc::new(SwcTransformAdapter::with_config(SwcAdapterConfig {
                source_maps: config.source_maps,
            })),
            EngineChoice::Rolldown => {
                Arc::new(RolldownAdapter::with_config(RolldownAdapterConfig::default()))
            }
            EngineChoice::Rspack => Arc::new(SwcTransformAdapter::with_config(SwcAdapterConfig {
                source_maps: config.source_maps,
            })),
        };
        Self { config, engine }
    }

    pub fn run_summarize(&self) -> Result<Vec<BundleGraphNode>> {
        let cache = LocalCache::with_default_root()
            .context("failed to initialise local summary cache")?;
        let nodes = summarize_directory(&self.config.root, &cache)
            .with_context(|| {
                format!(
                    "summarize_directory failed for root: {}",
                    self.config.root.display()
                )
            })?;
        Ok(nodes)
    }

    pub fn run_analyze(&self, nodes: Vec<BundleGraphNode>) -> Result<AnalysisResult> {
        let mut analyzer = GraphAnalyzer::new(self.config.entry_points.clone());
        analyzer.commons_threshold = self.config.commons_threshold;
        let result = analyzer
            .analyze(nodes)
            .with_context(|| "graph analysis failed")?;
        Ok(result)
    }

    pub fn run_transform(&self, analysis: &AnalysisResult) -> Result<Vec<ChunkOutput>> {
        let batch_config = BatchConfig {
            root: self.config.root.clone(),
            entry_points: self.config.entry_points.clone(),
            out_dir: self.config.out_dir.clone(),
        };
        self.engine
            .batch_transform(analysis, &batch_config)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    // -----------------------------------------------------------------------
    // End-to-end build
    // -----------------------------------------------------------------------

    pub fn build(&self) -> Result<BuildOutput> {
        let build_start = Instant::now();

        // ----- Phase 1: Summarize -----
        let phase_start = Instant::now();
        let mut nodes = self.run_summarize()?;
        // Load source for each node (transform engines require node.source).
        for node in &mut nodes {
            let source = std::fs::read_to_string(&node.path)
                .unwrap_or_else(|_| "// empty\n".to_string());
            node.source = Some(source);
        }
        let summarize_ms = phase_start.elapsed().as_millis() as u64;

        // ----- Phase 2: Analyze -----
        let phase_start = Instant::now();
        let mut analysis = self.run_analyze(nodes)?;
        analysis.manifest.build_id = build_id::compute_build_id(&analysis.manifest);
        let analyze_ms = phase_start.elapsed().as_millis() as u64;

        // ----- Phase 3: Transform -----
        let phase_start = Instant::now();
        let outputs = self.run_transform(&analysis)?;
        let transform_ms = phase_start.elapsed().as_millis() as u64;

        // ----- Phase 4: Emit -----
        let phase_start = Instant::now();

        let already_written = outputs.iter().any(|o| o.already_written);

        let id_to_output_hash: HashMap<String, ContentHash> = outputs
            .iter()
            .map(|o| (o.chunk_id.clone(), o.hash.clone()))
            .collect();

        let out_dir = &self.config.out_dir;
        let mut chunk_files: Vec<PathBuf> = Vec::with_capacity(outputs.len());
        let mut largest_chunk_bytes: u64 = 0;

        if already_written {
            for dir_entry in std::fs::read_dir(out_dir)
                .with_context(|| format!("reading out_dir {}", out_dir.display()))?
            {
                let dir_entry = dir_entry?;
                let path = dir_entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("js") {
                    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    largest_chunk_bytes = largest_chunk_bytes.max(size);
                    chunk_files.push(path);
                }
            }
        } else {
            for chunk_output in &outputs {
                let path = output::write_chunk(out_dir, chunk_output)
                    .with_context(|| {
                        format!("write_chunk failed for chunk {}", chunk_output.chunk_id)
                    })?;
                largest_chunk_bytes = largest_chunk_bytes.max(chunk_output.code.len() as u64);
                chunk_files.push(path);
            }
        }

        let chunks_written = chunk_files.len();

        output::write_manifest(out_dir, &analysis.manifest, &id_to_output_hash)
            .context("write_manifest failed")?;

        for entry in self.config.entry_points.keys() {
            if already_written {
                output::write_rolldown_index_html(out_dir, entry).with_context(|| {
                    format!("write_rolldown_index_html failed for entry '{entry}'")
                })?;
            } else {
                output::write_index_html(out_dir, &analysis.manifest, entry, &id_to_output_hash)
                    .with_context(|| format!("write_index_html failed for entry '{entry}'"))?;
            }
        }

        let emit_ms = phase_start.elapsed().as_millis() as u64;

        // ----- Aggregate stats -----
        let total_modules = analysis.nodes.len();
        let alive_modules = analysis.nodes.iter().filter(|n| n.alive).count();
        let dead_modules = total_modules - alive_modules;
        let build_time_ms = build_start.elapsed().as_millis();

        let stats = BuildStats {
            total_modules,
            alive_modules,
            dead_modules,
            chunks_written,
            build_time_ms,
            largest_chunk_bytes,
        };

        let output = BuildOutput {
            manifest: analysis.manifest,
            chunk_files,
            stats,
        };

        // ----- Read previous build (best effort) -----
        let previous = output::read_previous_stats(out_dir);

        // ----- Build the artifact -----
        let timing = BuildTiming {
            summarize_ms,
            analyze_ms,
            transform_ms,
            emit_ms,
            total_ms: build_time_ms as u64,
        };

        let artifact = BuildStatsArtifact::from_build(&output, previous.as_ref(), timing);

        // ----- Write build-stats.json (non-fatal) -----
        if let Err(e) = output::write_build_stats(out_dir, &artifact) {
            eprintln!("warning: failed to write build-stats.json: {e}");
        }

        Ok(output)
    }
}
```

- [ ] **Step 4: Update `lib.rs` re-exports**

Replace `crates/cloudpack-pipeline/src/lib.rs` with:

```rust
//! Cloudpack Build Pipeline — orchestrates summarize → analyze → transform → emit.

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
pub use pipeline::{BuildOutput, BuildPipeline};

pub fn hello() -> &'static str {
    "cloudpack-pipeline"
}
```

(`budget` module and re-export are added in Task 5.)

- [ ] **Step 5: Add the `budget` field to `BuildConfig` (with default `None`)**

Open `crates/cloudpack-pipeline/src/config.rs`. The current `BuildConfig` is:

```rust
#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub root: PathBuf,
    pub out_dir: PathBuf,
    pub source_maps: bool,
    pub commons_threshold: usize,
    pub engine: EngineChoice,
    pub entry_points: HashMap<String, PathBuf>,
}
```

Add a forward-declared opaque placeholder so Task 4 compiles without depending on Task 5's `budget` module. In `config.rs`, change to:

```rust
#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub root: PathBuf,
    pub out_dir: PathBuf,
    pub source_maps: bool,
    pub commons_threshold: usize,
    pub engine: EngineChoice,
    pub entry_points: HashMap<String, PathBuf>,
    /// Optional budget enforcement; `None` means "no budget configured".
    /// Wired up in Task 5 to `[budget]` in `cloudpack.toml`.
    pub budget: Option<crate::budget::BudgetConfig>,
}
```

This forces creating `budget.rs` now as a tiny stub (Task 5 fills it out). At the bottom of `crates/cloudpack-pipeline/src/lib.rs`, add:

```rust
pub mod budget;
```

And create `crates/cloudpack-pipeline/src/budget.rs` as a **stub**:

```rust
//! Budget enforcement — full implementation arrives in Task 5.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BudgetConfig {
    pub initial_bundle_max_bytes: Option<u64>,
    pub lazy_chunk_max_bytes: Option<u64>,
    pub total_bundle_max_bytes: Option<u64>,
}
```

Then update the `BuildConfig::load` constructor in `config.rs` to set `budget: None` (Task 5 wires in the actual TOML field). The current `load` builds the struct here:

```rust
        Ok(Self {
            root: raw.build.root,
            out_dir: raw.build.out_dir,
            source_maps: raw.build.source_maps,
            commons_threshold: raw.build.commons_threshold,
            engine: raw.build.engine,
            entry_points: raw.entry,
        })
```

Change to:

```rust
        Ok(Self {
            root: raw.build.root,
            out_dir: raw.build.out_dir,
            source_maps: raw.build.source_maps,
            commons_threshold: raw.build.commons_threshold,
            engine: raw.build.engine,
            entry_points: raw.entry,
            budget: None,
        })
```

- [ ] **Step 6: Fix the in-module fixture in `build_stats.rs` — old `BuildStats` field now uses `u64`**

`BuildStats.largest_chunk_bytes` changed type from `usize` to `u64`. The fixture in `build_stats.rs` already constructs the struct with literal integers — these coerce fine. No code change needed unless `cargo build` complains. If it does, the fixture's `largest_chunk_bytes: 1000` will need to be `1000u64` — adjust as the compiler reports.

- [ ] **Step 7: Verify the crate compiles**

Run: `~/.cargo/bin/cargo build -p cloudpack-pipeline --all-targets`
Expected: builds. Fix any field-type mismatches the compiler reports (most likely `largest_chunk_bytes` in the in-module fixture).

- [ ] **Step 8: Run all pipeline tests**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline`
Expected: all tests pass (Task 2's in-module tests + Task 3's IO tests + Task 4's pipeline test).

- [ ] **Step 9: Verify clippy is clean**

Run: `~/.cargo/bin/cargo clippy -p cloudpack-pipeline --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/cloudpack-pipeline/src/pipeline.rs \
        crates/cloudpack-pipeline/src/lib.rs \
        crates/cloudpack-pipeline/src/config.rs \
        crates/cloudpack-pipeline/src/budget.rs \
        crates/cloudpack-pipeline/src/build_stats.rs \
        crates/cloudpack-pipeline/tests/build_pipeline_stats_test.rs
git commit -m "feat(pipeline): wire BuildStatsArtifact + per-phase timing into build()"
```

---

## Task 5: Full `budget.rs` — `check()`, `BudgetViolation`, TOML wiring, non-fatal pipeline call

**Files:**
- Modify: `crates/cloudpack-pipeline/src/budget.rs` (replace stub with full impl)
- Modify: `crates/cloudpack-pipeline/src/config.rs` (parse `[budget]` from TOML)
- Modify: `crates/cloudpack-pipeline/src/pipeline.rs` (run budget check, populate `artifact.budget`)
- Create: `crates/cloudpack-pipeline/tests/budget_test.rs`

- [ ] **Step 1: Write failing tests for `budget::check`**

Create `crates/cloudpack-pipeline/tests/budget_test.rs`:

```rust
//! Tests for `budget::check` against a synthetic `BuildStatsArtifact`.

use std::collections::HashMap;

use cloudpack_pipeline::budget::{check, BudgetConfig};
use cloudpack_pipeline::build_stats::{
    BuildStatsArtifact, BudgetStatus, ChunkRecord, ChunkRole, EntryPointRecord, SummaryBlock,
    TimingBlock,
};

fn artifact(total: u64, initial: u64, lazy_sizes: &[(&str, u64)]) -> BuildStatsArtifact {
    let mut chunks = vec![ChunkRecord {
        id: "entry".to_string(),
        hash: "a".repeat(64),
        file: format!("chunks/{}.js", "a".repeat(64)),
        size_bytes: initial,
        module_count: 1,
        role: ChunkRole::Entry,
        entry_points: vec!["root".to_string()],
    }];
    for (id, size) in lazy_sizes {
        chunks.push(ChunkRecord {
            id: id.to_string(),
            hash: "b".repeat(64),
            file: format!("chunks/{}.js", "b".repeat(64)),
            size_bytes: *size,
            module_count: 1,
            role: ChunkRole::Lazy,
            entry_points: vec![],
        });
    }

    let mut entry_points = HashMap::new();
    entry_points.insert(
        "root".to_string(),
        EntryPointRecord {
            initial_chunks: vec!["entry".to_string()],
            initial_bytes: initial,
            lazy_chunks: lazy_sizes.iter().map(|(id, _)| id.to_string()).collect(),
        },
    );

    BuildStatsArtifact {
        schema_version: "1",
        build_id: "test".to_string(),
        cloudpack_version: "0.0.0-test".to_string(),
        generated_at: "2026-05-16T00:00:00+00:00".to_string(),
        summary: SummaryBlock {
            total_modules: 1,
            alive_modules: 1,
            dead_modules: 0,
            chunks_written: chunks.len() as u32,
            total_bundle_bytes: total,
            largest_chunk_bytes: chunks.iter().map(|c| c.size_bytes).max().unwrap_or(0),
            initial_bundle_bytes: initial,
        },
        timing: TimingBlock {
            build_time_ms: 0,
            summarize_ms: 0,
            analyze_ms: 0,
            transform_ms: 0,
            emit_ms: 0,
        },
        chunks,
        entry_points,
        budget: None,
        previous_build: None,
    }
}

#[test]
fn no_limits_returns_ok_with_zero_checks() {
    let a = artifact(1000, 500, &[]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: None,
        lazy_chunk_max_bytes: None,
        total_bundle_max_bytes: None,
    };
    let result = check(&a, &budget);
    assert!(result.is_ok());
}

#[test]
fn under_all_limits_returns_ok() {
    let a = artifact(1000, 500, &[("lazyA", 200)]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: Some(1000),
        lazy_chunk_max_bytes: Some(1000),
        total_bundle_max_bytes: Some(2000),
    };
    assert!(check(&a, &budget).is_ok());
}

#[test]
fn over_total_bundle_returns_violation() {
    let a = artifact(1_200_000, 500_000, &[]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: None,
        lazy_chunk_max_bytes: None,
        total_bundle_max_bytes: Some(1_000_000),
    };
    let err = check(&a, &budget).expect_err("should violate");
    assert_eq!(err.0.len(), 1);
    assert_eq!(err.0[0].name, "total_bundle_max_bytes");
    assert_eq!(err.0[0].limit, 1_000_000);
    assert_eq!(err.0[0].actual, 1_200_000);
    assert_eq!(err.0[0].status, BudgetStatus::Violated);
    assert!(err.0[0].offender.is_none());

    let msg = err.actionable_message();
    assert!(msg.contains("Budget violated"));
    assert!(msg.contains("total_bundle_max_bytes"));
    assert!(msg.contains("1,200,000"));
    assert!(msg.contains("1,000,000"));
}

#[test]
fn over_initial_bundle_returns_violation() {
    let a = artifact(1000, 800, &[]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: Some(500),
        lazy_chunk_max_bytes: None,
        total_bundle_max_bytes: None,
    };
    let err = check(&a, &budget).expect_err("should violate");
    assert_eq!(err.0[0].name, "initial_bundle_max_bytes");
    assert_eq!(err.0[0].actual, 800);
}

#[test]
fn over_lazy_chunk_returns_violation_with_offender() {
    let a = artifact(1000, 200, &[("smallLazy", 100), ("bigLazy", 350_000)]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: None,
        lazy_chunk_max_bytes: Some(250_000),
        total_bundle_max_bytes: None,
    };
    let err = check(&a, &budget).expect_err("should violate");
    assert_eq!(err.0.len(), 1, "only the over-limit lazy chunk reports");
    assert_eq!(err.0[0].name, "lazy_chunk_max_bytes");
    assert_eq!(err.0[0].actual, 350_000);
    assert_eq!(err.0[0].offender.as_deref(), Some("bigLazy"));
}

#[test]
fn multiple_violations_all_reported() {
    let a = artifact(2_000_000, 800_000, &[("oversize", 600_000)]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: Some(500_000),
        lazy_chunk_max_bytes: Some(500_000),
        total_bundle_max_bytes: Some(1_000_000),
    };
    let err = check(&a, &budget).expect_err("should violate");
    let names: Vec<&str> = err.0.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"initial_bundle_max_bytes"));
    assert!(names.contains(&"lazy_chunk_max_bytes"));
    assert!(names.contains(&"total_bundle_max_bytes"));
}
```

- [ ] **Step 2: Run the tests and verify failure**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --test budget_test`
Expected: FAIL — `check` not yet defined.

- [ ] **Step 3: Replace `budget.rs` stub with the full implementation**

Replace the contents of `crates/cloudpack-pipeline/src/budget.rs` with:

```rust
//! Budget enforcement against a [`BuildStatsArtifact`].
//!
//! `[budget]` in `cloudpack.toml` is opt-in: an absent section is `None`, which
//! means **no checks run** and the build behaves exactly as before. Each
//! individual limit field is also `Option<u64>` — `None` means "this rule is
//! disabled".

use serde::Deserialize;

use crate::build_stats::{BudgetCheck, BudgetStatus, BuildStatsArtifact, ChunkRole};

// ---------------------------------------------------------------------------
// Config — populated from `[budget]` in `cloudpack.toml`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BudgetConfig {
    /// Max bytes for the initial bundle (sum of Entry + Commons chunks).
    pub initial_bundle_max_bytes: Option<u64>,
    /// Max bytes for any single Lazy chunk.
    pub lazy_chunk_max_bytes: Option<u64>,
    /// Max bytes for the total bundle (every emitted chunk).
    pub total_bundle_max_bytes: Option<u64>,
}

impl BudgetConfig {
    /// Returns `true` if at least one limit is set.
    pub fn any_limit_set(&self) -> bool {
        self.initial_bundle_max_bytes.is_some()
            || self.lazy_chunk_max_bytes.is_some()
            || self.total_bundle_max_bytes.is_some()
    }
}

// ---------------------------------------------------------------------------
// Violation type
// ---------------------------------------------------------------------------

/// Wrapper around the list of failed [`BudgetCheck`]s.
#[derive(Debug, Clone)]
pub struct BudgetViolation(pub Vec<BudgetCheck>);

impl BudgetViolation {
    /// Produce a multiline, human-readable message suitable for `eprintln!`
    /// and (later) CLI output.
    pub fn actionable_message(&self) -> String {
        let mut out = String::from("Budget violated:\n");
        for c in &self.0 {
            let actual = format_thousands(c.actual);
            let limit = format_thousands(c.limit);
            match &c.offender {
                Some(id) => out.push_str(&format!(
                    "  {}: {} bytes on chunk '{}' (limit {})\n",
                    c.name, actual, id, limit
                )),
                None => out.push_str(&format!(
                    "  {}: {} bytes (limit {})\n",
                    c.name, actual, limit
                )),
            }
        }
        out
    }
}

fn format_thousands(n: u64) -> String {
    // 1234567 → "1,234,567"
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

// ---------------------------------------------------------------------------
// check()
// ---------------------------------------------------------------------------

/// Evaluate every configured limit against `stats`.
///
/// Returns `Ok(())` if every check is `Ok`. Returns
/// `Err(BudgetViolation(failed_checks))` if any check fails. Each individual
/// failed check is included in the error so callers can render the full list.
pub fn check(stats: &BuildStatsArtifact, budget: &BudgetConfig) -> Result<(), BudgetViolation> {
    let mut failures: Vec<BudgetCheck> = Vec::new();

    // 1. Total bundle size
    if let Some(limit) = budget.total_bundle_max_bytes {
        let actual = stats.summary.total_bundle_bytes;
        if actual > limit {
            failures.push(BudgetCheck {
                name: "total_bundle_max_bytes".to_string(),
                limit,
                actual,
                status: BudgetStatus::Violated,
                offender: None,
            });
        }
    }

    // 2. Initial bundle size
    if let Some(limit) = budget.initial_bundle_max_bytes {
        let actual = stats.summary.initial_bundle_bytes;
        if actual > limit {
            failures.push(BudgetCheck {
                name: "initial_bundle_max_bytes".to_string(),
                limit,
                actual,
                status: BudgetStatus::Violated,
                offender: None,
            });
        }
    }

    // 3. Per-lazy-chunk size — emit one violation per offending chunk
    if let Some(limit) = budget.lazy_chunk_max_bytes {
        for c in &stats.chunks {
            if matches!(c.role, ChunkRole::Lazy) && c.size_bytes > limit {
                failures.push(BudgetCheck {
                    name: "lazy_chunk_max_bytes".to_string(),
                    limit,
                    actual: c.size_bytes,
                    status: BudgetStatus::Violated,
                    offender: Some(c.id.clone()),
                });
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(BudgetViolation(failures))
    }
}

/// Build the [`BudgetResult`] block to embed in `BuildStatsArtifact.budget`.
///
/// Always returns a `BudgetResult`: `configured = true`, `result = Ok` when
/// every check passes, `result = Violated` with the list of failures otherwise.
/// Also includes a passing-check record for each enabled limit so the JSON
/// document is self-describing.
pub fn build_budget_result(
    stats: &BuildStatsArtifact,
    budget: &BudgetConfig,
) -> crate::build_stats::BudgetResult {
    use crate::build_stats::BudgetResult;

    let mut checks: Vec<BudgetCheck> = Vec::new();
    let mut any_violation = false;

    if let Some(limit) = budget.total_bundle_max_bytes {
        let actual = stats.summary.total_bundle_bytes;
        let status = if actual > limit {
            any_violation = true;
            BudgetStatus::Violated
        } else {
            BudgetStatus::Ok
        };
        checks.push(BudgetCheck {
            name: "total_bundle_max_bytes".to_string(),
            limit,
            actual,
            status,
            offender: None,
        });
    }

    if let Some(limit) = budget.initial_bundle_max_bytes {
        let actual = stats.summary.initial_bundle_bytes;
        let status = if actual > limit {
            any_violation = true;
            BudgetStatus::Violated
        } else {
            BudgetStatus::Ok
        };
        checks.push(BudgetCheck {
            name: "initial_bundle_max_bytes".to_string(),
            limit,
            actual,
            status,
            offender: None,
        });
    }

    if let Some(limit) = budget.lazy_chunk_max_bytes {
        // Aggregate "worst lazy chunk" check + per-offender records.
        let mut worst: u64 = 0;
        let mut had_offender = false;
        for c in &stats.chunks {
            if matches!(c.role, ChunkRole::Lazy) {
                worst = worst.max(c.size_bytes);
                if c.size_bytes > limit {
                    had_offender = true;
                    any_violation = true;
                    checks.push(BudgetCheck {
                        name: "lazy_chunk_max_bytes".to_string(),
                        limit,
                        actual: c.size_bytes,
                        status: BudgetStatus::Violated,
                        offender: Some(c.id.clone()),
                    });
                }
            }
        }
        if !had_offender {
            checks.push(BudgetCheck {
                name: "lazy_chunk_max_bytes".to_string(),
                limit,
                actual: worst,
                status: BudgetStatus::Ok,
                offender: None,
            });
        }
    }

    BudgetResult {
        configured: true,
        checks,
        result: if any_violation {
            BudgetStatus::Violated
        } else {
            BudgetStatus::Ok
        },
    }
}
```

- [ ] **Step 4: Wire `[budget]` into `BuildConfig::load`**

Open `crates/cloudpack-pipeline/src/config.rs`. The current raw types section is:

```rust
#[derive(Debug, Deserialize)]
struct RawConfig {
    build: RawBuild,
    entry: HashMap<String, PathBuf>,
}
```

Replace with:

```rust
#[derive(Debug, Deserialize)]
struct RawConfig {
    build: RawBuild,
    entry: HashMap<String, PathBuf>,
    #[serde(default)]
    budget: Option<crate::budget::BudgetConfig>,
}
```

And in `BuildConfig::load`, change the constructor from:

```rust
        Ok(Self {
            root: raw.build.root,
            out_dir: raw.build.out_dir,
            source_maps: raw.build.source_maps,
            commons_threshold: raw.build.commons_threshold,
            engine: raw.build.engine,
            entry_points: raw.entry,
            budget: None,
        })
```

to:

```rust
        Ok(Self {
            root: raw.build.root,
            out_dir: raw.build.out_dir,
            source_maps: raw.build.source_maps,
            commons_threshold: raw.build.commons_threshold,
            engine: raw.build.engine,
            entry_points: raw.entry,
            budget: raw.budget,
        })
```

- [ ] **Step 5: Re-export `BudgetConfig` + `BudgetViolation` from `lib.rs`**

In `crates/cloudpack-pipeline/src/lib.rs`, ensure the file contains:

```rust
pub use budget::{BudgetConfig, BudgetViolation};
```

(Add the line if it isn't already present from Task 1's transitional state.)

- [ ] **Step 6: Run the budget tests**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --test budget_test`
Expected: all 6 tests pass.

- [ ] **Step 7: Wire the budget check into `BuildPipeline::build()`**

In `crates/cloudpack-pipeline/src/pipeline.rs`, find the section that constructs the artifact and writes it:

```rust
        let artifact = BuildStatsArtifact::from_build(&output, previous.as_ref(), timing);

        // ----- Write build-stats.json (non-fatal) -----
        if let Err(e) = output::write_build_stats(out_dir, &artifact) {
            eprintln!("warning: failed to write build-stats.json: {e}");
        }

        Ok(output)
```

Replace with:

```rust
        let mut artifact = BuildStatsArtifact::from_build(&output, previous.as_ref(), timing);

        // ----- Budget check (opt-in via [budget] in cloudpack.toml) -----
        // Populate `artifact.budget` so the on-disk JSON reflects the result,
        // then run the strict `check()` for diagnostics. The CLI exit-1 wire-up
        // is a deferred follow-up plan; for now violations are non-fatal here.
        if let Some(budget_cfg) = &self.config.budget {
            if budget_cfg.any_limit_set() {
                artifact.budget = Some(crate::budget::build_budget_result(&artifact, budget_cfg));
                if let Err(violation) = crate::budget::check(&artifact, budget_cfg) {
                    eprintln!("{}", violation.actionable_message());
                }
            }
        }

        // ----- Write build-stats.json (non-fatal) -----
        if let Err(e) = output::write_build_stats(out_dir, &artifact) {
            eprintln!("warning: failed to write build-stats.json: {e}");
        }

        Ok(output)
```

(Note: `artifact` is now `mut`.)

- [ ] **Step 8: Add an integration test asserting `artifact.budget` is populated**

Append to `crates/cloudpack-pipeline/tests/build_pipeline_stats_test.rs`:

```rust
#[test]
fn build_with_budget_populates_budget_result() {
    use cloudpack_pipeline::budget::BudgetConfig;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    let out_dir = tmp.path().join("dist");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("index.js"), "export const x = 1;\n").unwrap();

    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    entry_points.insert("root".to_string(), root.join("index.js"));

    let cfg = BuildConfig {
        root,
        out_dir: out_dir.clone(),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points,
        budget: Some(BudgetConfig {
            initial_bundle_max_bytes: Some(10_000_000), // generous → Ok
            lazy_chunk_max_bytes: None,
            total_bundle_max_bytes: None,
        }),
    };

    let _ = BuildPipeline::new(cfg).build().expect("build succeeds");
    let raw = fs::read(out_dir.join("build-stats.json")).unwrap();
    let a: BuildStatsArtifact = serde_json::from_slice(&raw).unwrap();

    let b = a.budget.expect("budget block present");
    assert!(b.configured);
    assert_eq!(b.result, cloudpack_pipeline::build_stats::BudgetStatus::Ok);
    assert!(b.checks.iter().any(|c| c.name == "initial_bundle_max_bytes"));
}

#[test]
fn build_with_violated_budget_marks_result_violated() {
    use cloudpack_pipeline::build_stats::BudgetStatus;
    use cloudpack_pipeline::budget::BudgetConfig;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    let out_dir = tmp.path().join("dist");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("index.js"), "export const x = 1;\n").unwrap();

    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    entry_points.insert("root".to_string(), root.join("index.js"));

    let cfg = BuildConfig {
        root,
        out_dir: out_dir.clone(),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points,
        budget: Some(BudgetConfig {
            initial_bundle_max_bytes: Some(1), // impossibly small → Violated
            lazy_chunk_max_bytes: None,
            total_bundle_max_bytes: None,
        }),
    };

    let _ = BuildPipeline::new(cfg).build().expect("build still succeeds (non-fatal)");
    let raw = fs::read(out_dir.join("build-stats.json")).unwrap();
    let a: BuildStatsArtifact = serde_json::from_slice(&raw).unwrap();

    let b = a.budget.expect("budget block present");
    assert_eq!(b.result, BudgetStatus::Violated);
}
```

- [ ] **Step 9: Run the full pipeline test suite**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline`
Expected: every test passes.

- [ ] **Step 10: Verify clippy is clean**

Run: `~/.cargo/bin/cargo clippy -p cloudpack-pipeline --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 11: Commit**

```bash
git add crates/cloudpack-pipeline/src/budget.rs \
        crates/cloudpack-pipeline/src/config.rs \
        crates/cloudpack-pipeline/src/lib.rs \
        crates/cloudpack-pipeline/src/pipeline.rs \
        crates/cloudpack-pipeline/tests/budget_test.rs \
        crates/cloudpack-pipeline/tests/build_pipeline_stats_test.rs
git commit -m "feat(pipeline): opt-in budget check + on-disk budget block"
```

---

## Task 6: Implement `previous_build` delta + final sweep

**Files:**
- Modify: `crates/cloudpack-pipeline/src/build_stats.rs` (real `compute_previous_build_info`)
- Create: `crates/cloudpack-pipeline/tests/build_stats_delta_test.rs`

- [ ] **Step 1: Write failing tests for the delta computation**

Create `crates/cloudpack-pipeline/tests/build_stats_delta_test.rs`:

```rust
//! End-to-end delta test: run two builds back-to-back and verify the second
//! build's artifact contains an accurate `previous_build` block.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use cloudpack_pipeline::build_stats::BuildStatsArtifact;
use cloudpack_pipeline::config::{BuildConfig, EngineChoice};
use cloudpack_pipeline::pipeline::BuildPipeline;

fn cfg(root: PathBuf, out_dir: PathBuf) -> BuildConfig {
    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    entry_points.insert("root".to_string(), root.join("index.js"));
    BuildConfig {
        root,
        out_dir,
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points,
        budget: None,
    }
}

fn read_artifact(out_dir: &std::path::Path) -> BuildStatsArtifact {
    let raw = fs::read(out_dir.join("build-stats.json")).unwrap();
    serde_json::from_slice(&raw).unwrap()
}

#[test]
fn first_build_has_no_previous() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    let out_dir = tmp.path().join("dist");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("index.js"), "export const x = 1;\n").unwrap();

    BuildPipeline::new(cfg(root, out_dir.clone()))
        .build()
        .unwrap();

    let a = read_artifact(&out_dir);
    assert!(a.previous_build.is_none());
}

#[test]
fn second_build_records_previous_with_size_delta() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    let out_dir = tmp.path().join("dist");
    fs::create_dir_all(&root).unwrap();

    // Build 1 — small input.
    fs::write(root.join("index.js"), "export const x = 1;\n").unwrap();
    BuildPipeline::new(cfg(root.clone(), out_dir.clone()))
        .build()
        .unwrap();
    let first = read_artifact(&out_dir);
    let first_id = first.build_id.clone();
    let first_total = first.summary.total_bundle_bytes;

    // Build 2 — make the source materially larger.
    let bigger = "export const big = \"".to_string()
        + &"x".repeat(5_000)
        + "\";\nexport const x = 1;\n";
    fs::write(root.join("index.js"), bigger).unwrap();
    BuildPipeline::new(cfg(root, out_dir.clone())).build().unwrap();

    let second = read_artifact(&out_dir);
    let prev = second.previous_build.expect("previous_build present");
    assert!(prev.present);
    assert_eq!(prev.build_id, first_id);

    let td = &prev.delta.total_bundle_bytes;
    assert_eq!(td.prev, first_total);
    assert_eq!(td.curr, second.summary.total_bundle_bytes);
    assert_eq!(td.delta, td.curr as i64 - td.prev as i64);
    assert!(td.curr > td.prev, "second build should be larger");
    assert!(td.pct > 0.0);
}

#[test]
fn previous_build_id_unchanged_when_source_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    let out_dir = tmp.path().join("dist");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("index.js"), "export const x = 1;\n").unwrap();

    BuildPipeline::new(cfg(root.clone(), out_dir.clone()))
        .build()
        .unwrap();
    let first = read_artifact(&out_dir);

    BuildPipeline::new(cfg(root, out_dir.clone())).build().unwrap();
    let second = read_artifact(&out_dir);

    let prev = second.previous_build.expect("previous_build present");
    // Same source → same build_id, zero delta.
    assert_eq!(prev.build_id, first.build_id);
    assert_eq!(prev.delta.total_bundle_bytes.delta, 0);
    assert_eq!(prev.delta.total_bundle_bytes.pct, 0.0);
    assert!(prev.delta.chunks_added.is_empty());
    assert!(prev.delta.chunks_removed.is_empty());
}
```

- [ ] **Step 2: Run the tests and verify failure**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --test build_stats_delta_test`
Expected: FAIL — `compute_previous_build_info` is still the stub from Task 2, so `prev.build_id` will be `""`.

- [ ] **Step 3: Replace the stub `compute_previous_build_info` with the real implementation**

In `crates/cloudpack-pipeline/src/build_stats.rs`, find the stub:

```rust
// Filled in by Task 6 — stub for now so `from_build` compiles.
fn compute_previous_build_info(
    _prev: &BuildStatsArtifact,
    _summary: &SummaryBlock,
    _chunks: &[ChunkRecord],
) -> PreviousBuildInfo {
    PreviousBuildInfo {
        present: true,
        build_id: String::new(),
        delta: BuildDelta {
            total_bundle_bytes: SizeDelta { prev: 0, curr: 0, delta: 0, pct: 0.0 },
            initial_bundle_bytes: SizeDelta { prev: 0, curr: 0, delta: 0, pct: 0.0 },
            chunks_added: Vec::new(),
            chunks_removed: Vec::new(),
        },
    }
}
```

Replace it with:

```rust
fn compute_previous_build_info(
    prev: &BuildStatsArtifact,
    summary: &SummaryBlock,
    chunks: &[ChunkRecord],
) -> PreviousBuildInfo {
    use std::collections::HashSet;

    let total = size_delta(
        prev.summary.total_bundle_bytes,
        summary.total_bundle_bytes,
    );
    let initial = size_delta(
        prev.summary.initial_bundle_bytes,
        summary.initial_bundle_bytes,
    );

    let prev_ids: HashSet<&str> = prev.chunks.iter().map(|c| c.id.as_str()).collect();
    let curr_ids: HashSet<&str> = chunks.iter().map(|c| c.id.as_str()).collect();

    let mut chunks_added: Vec<String> = curr_ids
        .difference(&prev_ids)
        .map(|s| s.to_string())
        .collect();
    let mut chunks_removed: Vec<String> = prev_ids
        .difference(&curr_ids)
        .map(|s| s.to_string())
        .collect();
    chunks_added.sort();
    chunks_removed.sort();

    PreviousBuildInfo {
        present: true,
        build_id: prev.build_id.clone(),
        delta: BuildDelta {
            total_bundle_bytes: total,
            initial_bundle_bytes: initial,
            chunks_added,
            chunks_removed,
        },
    }
}

fn size_delta(prev: u64, curr: u64) -> SizeDelta {
    let delta = curr as i64 - prev as i64;
    let pct = if prev == 0 {
        0.0
    } else {
        (delta as f64 / prev as f64) * 100.0
    };
    SizeDelta {
        prev,
        curr,
        delta,
        pct,
    }
}
```

- [ ] **Step 4: Run the delta tests**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline --test build_stats_delta_test`
Expected: all 3 tests pass.

- [ ] **Step 5: Run the full pipeline test suite**

Run: `~/.cargo/bin/cargo test -p cloudpack-pipeline`
Expected: every test passes (Tasks 2–6).

- [ ] **Step 6: Run a workspace-wide build + clippy to catch downstream regressions**

Run: `~/.cargo/bin/cargo build --workspace`
Expected: clean build. If any crate (e.g. `cloudpack-cli`) referenced the removed `BuildStats` re-export, fix the import — it is now `cloudpack_pipeline::pipeline::BuildStats`. Run `grep -rn "BuildStats" crates/cloudpack-cli/` to check.

Run: `~/.cargo/bin/cargo clippy -p cloudpack-pipeline --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Verify a build-stats.json sample by hand**

Run a one-shot smoke check via a throwaway script — or simply re-read one of the test outputs:

```bash
~/.cargo/bin/cargo test -p cloudpack-pipeline --test build_pipeline_stats_test -- --nocapture 2>&1 | head -5
```

This is just an extra eyeballing pass — automated assertions already cover the schema.

- [ ] **Step 8: Commit**

```bash
git add crates/cloudpack-pipeline/src/build_stats.rs \
        crates/cloudpack-pipeline/tests/build_stats_delta_test.rs
git commit -m "feat(pipeline): previous_build delta in BuildStatsArtifact"
```

---

## Self-Review Summary

**Spec coverage check:**
- `BuildStatsArtifact` + all sub-types → Task 1 ✓
- `from_build` constructor → Task 2 ✓
- `BuildTiming` + per-phase `Instant` instrumentation → Task 4 ✓
- Atomic `write_build_stats` (tmp + rename) → Task 3 ✓
- `read_previous_stats` (None on any error) → Task 3 ✓
- `BudgetConfig` (all 3 limits, all `Option`) → Task 5 ✓
- `BudgetViolation::actionable_message` with thousand-separators → Task 5 ✓
- `budget::check` returning per-violation list → Task 5 ✓
- `BuildConfig.budget` field + TOML wiring → Task 5 ✓
- Budget call AFTER artifact construction, non-fatal in pipeline → Task 5 ✓
- `previous_build` delta computation → Task 6 ✓
- Acceptance criteria — every field tested in Tasks 2/4/5/6 ✓

**Scope boundary respected:** No ABS/Prometheus/web-vitals/CLI-budget-flag changes. Budget violations are non-fatal in `pipeline.rs` (CLI exit-1 deferred). The schema's `ChunkRole::Dead` variant is defined but never emitted — explicitly noted as reserved for future use.

**Type consistency check:**
- `BuildStats.largest_chunk_bytes`: spec says `u64` in old struct context, but the original code uses `usize`. The plan changes it to `u64` in Task 4 to match `BuildStatsArtifact` math; in-module fixture in Task 2 uses `1000` (which coerces) — Task 4 Step 6 calls out the fix if the compiler complains.
- `BuildTiming` fields are `u64` everywhere (constructor in Task 2, instrumentation in Task 4).
- `BudgetConfig::any_limit_set` is added in Task 5 and used by `pipeline.rs` to skip the work when no limits are set — keeps "absent section → zero behavior change" airtight.

Plan complete and saved to `docs/superpowers/plans/2026-05-16-observability-p1-full.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
