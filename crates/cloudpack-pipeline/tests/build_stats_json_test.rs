//! Tests for `build-stats.json` emission (Observability SCA).
//!
//! Acceptance criteria: `cargo test -p cloudpack-pipeline --test build_stats_json_test`
//! reports `test result: ok. 4 passed; 0 failed`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use cloudpack_pipeline::config::{BuildConfig, EngineChoice};
use cloudpack_pipeline::pipeline::BuildPipeline;

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
        budget: None,
        dev: None,
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

    let contents = fs::read_to_string(out_path.join("build-stats.json"))
        .expect("build-stats.json must be readable");
    let value: serde_json::Value = serde_json::from_str(&contents)
        .expect("build-stats.json must be valid JSON");

    // The extended schema (BuildStatsArtifact) uses top-level envelope fields
    // and nested sub-blocks rather than flat top-level keys.
    let expected_top_keys = [
        "schema_version",
        "build_id",
        "cloudpack_version",
        "generated_at",
        "summary",
        "timing",
        "chunks",
        "entry_points",
    ];
    for key in &expected_top_keys {
        assert!(
            value.get(key).is_some(),
            "build-stats.json is missing required key: {key}"
        );
    }
    // Summary sub-block carries the module/chunk counters.
    let summary = &value["summary"];
    for key in &["total_modules", "alive_modules", "dead_modules", "chunks_written"] {
        assert!(
            summary.get(key).is_some(),
            "build-stats.json summary is missing field: {key}"
        );
    }
    // Timing sub-block carries the wall-clock counters.
    let timing = &value["timing"];
    assert!(
        timing.get("build_time_ms").is_some(),
        "build-stats.json timing is missing field: build_time_ms"
    );
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
}
