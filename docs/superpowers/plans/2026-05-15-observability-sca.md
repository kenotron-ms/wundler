# Observability SCA: `build-stats.json` Implementation Plan

> **Execution:** Use the subagent-driven-development workflow to implement this plan.

**Goal:** Emit a machine-readable `build-stats.json` file next to `manifest.json` on every successful build.
**Architecture:** Add `#[derive(serde::Serialize)]` to the existing `BuildStats` struct, then write the JSON file at the end of `build()` using `serde_json::to_writer_pretty`. Failures are non-fatal — warn with `eprintln!` and continue. No new dependencies needed; `serde` and `serde_json` are already in `Cargo.toml`.
**Tech Stack:** Rust, `serde` (workspace dep with `derive` feature), `serde_json` (workspace dep).

---

## Pre-flight checks

Before starting, verify the workspace builds cleanly:

```
cargo test -p wundler-pipeline
```

Expected: all existing tests pass. If not, stop and fix before proceeding.

---

### Task 1: Write the failing tests

**Files:**
- Create: `crates/wundler-pipeline/tests/build_stats_json_test.rs`

**Step 1: Create the test file**

```rust
//! Tests for `build-stats.json` emission (Observability SCA).
//!
//! Acceptance criteria: `cargo test -p wundler-pipeline --test build_stats_json_test`
//! reports `test result: ok. 4 passed; 0 failed`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

/// Minimal 2-file TypeScript project: `src/index.ts` imports `./util`.
///
/// `tag` is embedded in each file so parallel test instances produce
/// distinct content hashes and avoid racing on the same cache entry.
///
/// Returns `(project_dir, out_dir, config)`.
/// Keep both `TempDir` handles alive for the duration of each test —
/// dropping them deletes the directory.
fn make_project(tag: &str) -> (TempDir, TempDir, BuildConfig) {
    let project_dir = TempDir::new().unwrap();
    let out_dir = TempDir::new().unwrap();

    let src = project_dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("index.ts"),
        format!("// {tag}\nimport {{ util }} from './util';\nexport const index = util + 1;\n"),
    )
    .unwrap();
    fs::write(
        src.join("util.ts"),
        format!("// {tag}\nexport const util = 1;\n"),
    )
    .unwrap();

    let config = BuildConfig {
        root: project_dir.path().to_path_buf(),
        out_dir: out_dir.path().to_path_buf(),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: {
            let mut m = HashMap::new();
            m.insert("main".to_string(), PathBuf::from("src/index.ts"));
            m
        },
    };

    (project_dir, out_dir, config)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_emits_stats_json() {
    let (_project_dir, out_dir, config) = make_project("stats-json-exists");
    let out_path = out_dir.path().to_path_buf();

    BuildPipeline::new(config).build().expect("build() failed");

    let stats_path = out_path.join("build-stats.json");
    assert!(
        stats_path.exists(),
        "build-stats.json must exist at {stats_path:?}"
    );
}

#[test]
fn test_build_stats_json_is_valid_json() {
    let (_project_dir, out_dir, config) = make_project("stats-json-valid");
    let out_path = out_dir.path().to_path_buf();

    BuildPipeline::new(config).build().expect("build() failed");

    let contents = fs::read_to_string(out_path.join("build-stats.json"))
        .expect("build-stats.json must be readable");
    serde_json::from_str::<serde_json::Value>(&contents)
        .expect("build-stats.json must be valid JSON");
}

#[test]
fn test_build_stats_json_contains_expected_fields() {
    let (_project_dir, out_dir, config) = make_project("stats-json-fields");
    let out_path = out_dir.path().to_path_buf();

    BuildPipeline::new(config).build().expect("build() failed");

    let contents = fs::read_to_string(out_path.join("build-stats.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&contents).unwrap();

    let expected_keys = [
        "total_modules",
        "alive_modules",
        "dead_modules",
        "chunks_written",
        "build_time_ms",
        "largest_chunk_bytes",
    ];
    for key in &expected_keys {
        assert!(
            value.get(key).is_some(),
            "build-stats.json is missing required key: {key}"
        );
    }
}

#[test]
fn test_build_stats_json_is_next_to_manifest() {
    let (_project_dir, out_dir, config) = make_project("stats-json-colocation");
    let out_path = out_dir.path().to_path_buf();

    BuildPipeline::new(config).build().expect("build() failed");

    let stats_path = out_path.join("build-stats.json");
    let manifest_path = out_path.join("manifest.json");

    assert!(
        stats_path.exists(),
        "build-stats.json must exist at {stats_path:?}"
    );
    assert!(
        manifest_path.exists(),
        "manifest.json must exist at {manifest_path:?}"
    );
    assert_eq!(
        stats_path.parent(),
        manifest_path.parent(),
        "build-stats.json and manifest.json must be in the same directory"
    );
}
```

**Step 2: Run to verify failure**

```
cargo test -p wundler-pipeline --test build_stats_json_test
```

Expected: all 4 tests **FAIL** — `build-stats.json` does not exist because the write logic hasn't been added yet.

```
test test_build_emits_stats_json ... FAILED
test test_build_stats_json_is_valid_json ... FAILED
test test_build_stats_json_contains_expected_fields ... FAILED
test test_build_stats_json_is_next_to_manifest ... FAILED
```

---

### Task 2: Add `serde::Serialize` to `BuildStats`

**Files:**
- Modify: `crates/wundler-pipeline/src/pipeline.rs:26`

**Step 1: Add `serde::Serialize` to the derive macro on `BuildStats`**

Locate line 26 of `pipeline.rs`. Change:

```rust
#[derive(Debug, Clone)]
pub struct BuildStats {
```

To:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildStats {
```

No other changes to the struct or its fields.

**Step 2: Verify the crate compiles**

```
cargo build -p wundler-pipeline
```

Expected: **compiles without errors**. No test changes yet — the tests still fail at runtime because the file isn't written.

**Step 3: Verify existing tests still pass**

```
cargo test -p wundler-pipeline --test pipeline_build
```

Expected: both existing `pipeline_build` tests **PASS** (no behavioral change from adding a derive).

---

### Task 3: Write `build-stats.json` after build

**Files:**
- Modify: `crates/wundler-pipeline/src/pipeline.rs:226-243`

**Step 1: Refactor the stats assembly and add the write**

Find the `// ----- Compute stats -----` block near the end of `build()` (currently lines 226–243). Replace it with:

```rust
        // ----- Compute stats -----
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

        // ----- Step 4d: Write build-stats.json (non-fatal) -----
        let stats_path = out_dir.join("build-stats.json");
        match std::fs::File::create(&stats_path) {
            Ok(f) => {
                if let Err(e) = serde_json::to_writer_pretty(f, &stats) {
                    eprintln!("warning: failed to write build-stats.json: {e}");
                }
            }
            Err(e) => {
                eprintln!("warning: failed to create build-stats.json: {e}");
            }
        }

        Ok(BuildOutput {
            manifest: analysis.manifest,
            chunk_files,
            stats,
        })
```

> **Note:** `out_dir` is already bound at the top of `build()` as `let out_dir = &self.config.out_dir;`. No new bindings needed.
> The original `BuildOutput { stats: BuildStats { ... } }` inline struct literal becomes `BuildOutput { stats }` using the named variable.

**Step 2: Run the new tests**

```
cargo test -p wundler-pipeline --test build_stats_json_test
```

Expected: all 4 tests **PASS**.

```
test test_build_emits_stats_json ... ok
test test_build_stats_json_is_valid_json ... ok
test test_build_stats_json_contains_expected_fields ... ok
test test_build_stats_json_is_next_to_manifest ... ok

test result: ok. 4 passed; 0 failed
```

---

### Task 4: Verify no regressions across the full crate

**Step 1: Run all `wundler-pipeline` tests**

```
cargo test -p wundler-pipeline
```

Expected: all existing tests plus the 4 new ones pass. Zero failures.

**Step 2: Run workspace-wide check**

```
cargo test --workspace
```

Expected: zero failures across all crates.

---

### Task 5: Commit

**Step 1: Stage the two changed files**

```
git add crates/wundler-pipeline/src/pipeline.rs \
        crates/wundler-pipeline/tests/build_stats_json_test.rs
```

**Step 2: Commit**

```
git commit -m "feat(observability): emit build-stats.json on every successful build

Add serde::Serialize to BuildStats and write build-stats.json to out_dir
at the end of BuildPipeline::build(). Failures are non-fatal (eprintln!
warning, build continues). No new dependencies — serde and serde_json are
already workspace deps with derive enabled.

Closes Observability SCA."
```

---

## Scope boundary (do NOT implement)

These are Observability P1 full — deferred until the SCA artifact pattern is proven:

- `build_id` field in `BuildStats` (requires VRC SCA to land first)
- Per-chunk size breakdown
- Delta tracking against previous builds
- CI budget enforcement
- Extended fields (`total_bundle_bytes`, etc.)

## Expected output format

```json
{
  "total_modules": 2,
  "alive_modules": 2,
  "dead_modules": 0,
  "chunks_written": 1,
  "build_time_ms": 142,
  "largest_chunk_bytes": 312
}
```

Field names are snake_case (serde default). No renaming attributes needed.
