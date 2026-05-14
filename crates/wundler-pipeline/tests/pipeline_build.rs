//! Integration tests for `BuildPipeline::build()` end-to-end orchestration.
//!
//! Acceptance criteria: `cargo test -p wundler-pipeline --test pipeline_build`
//! reports `test result: ok. 2 passed; 0 failed`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a temp project with:
/// - `src/index.ts` — imports `./util`
/// - `src/util.ts`  — a leaf module (no imports)
/// - `src/orphan.ts` — not imported by anyone (dead)
///
/// Returns (project TempDir, output TempDir, BuildConfig).
fn make_project_and_config(tag: &str) -> (TempDir, TempDir, BuildConfig) {
    let project_dir = TempDir::new().unwrap();
    let out_dir = TempDir::new().unwrap();

    let src = project_dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("index.ts"),
        format!(
            "// {tag}\nimport {{ util }} from './util';\nexport const index = util + 1;\n"
        ),
    )
    .unwrap();
    fs::write(
        src.join("util.ts"),
        format!("// {tag}\nexport const util = 1;\n"),
    )
    .unwrap();
    fs::write(
        src.join("orphan.ts"),
        format!("// {tag}\nexport const orphan = 99;\n"),
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
// Test 1: build() writes chunk files and manifest.json
// ---------------------------------------------------------------------------

#[test]
fn build_writes_chunks_and_manifest() {
    let (_project_dir, out_dir, config) = make_project_and_config("build-writes-test");
    let out_path = out_dir.path().to_path_buf();
    let pipeline = BuildPipeline::new(config);

    let result = pipeline.build().expect("build() failed");

    // At least one chunk must have been written.
    assert!(
        result.stats.chunks_written >= 1,
        "expected at least 1 chunk written, got {}",
        result.stats.chunks_written
    );

    // manifest.json must exist on disk.
    let manifest_path = out_path.join("manifest.json");
    assert!(
        manifest_path.exists(),
        "manifest.json must exist at {manifest_path:?}"
    );

    // The chunks/ directory must contain exactly as many .js files as
    // chunks_written.
    let chunks_dir = out_path.join("chunks");
    assert!(
        chunks_dir.exists(),
        "chunks/ directory must exist at {chunks_dir:?}"
    );

    let js_files: Vec<_> = fs::read_dir(&chunks_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "js")
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        js_files.len(),
        result.stats.chunks_written,
        "number of .js files in chunks/ must equal chunks_written"
    );
}

// ---------------------------------------------------------------------------
// Test 2: build() stats reflect alive/dead module counts
// ---------------------------------------------------------------------------

#[test]
fn build_stats_reflect_alive_dead_counts() {
    let (_project_dir, _out_dir, config) = make_project_and_config("build-stats-test");
    let pipeline = BuildPipeline::new(config);

    let result = pipeline.build().expect("build() failed");

    let stats = &result.stats;

    // We created exactly 3 source files.
    assert_eq!(
        stats.total_modules, 3,
        "expected total_modules = 3, got {}",
        stats.total_modules
    );

    // orphan.ts is not reachable from index.ts → at least 1 dead module.
    assert!(
        stats.dead_modules >= 1,
        "expected at least 1 dead module (orphan.ts), got {}",
        stats.dead_modules
    );

    // alive + dead must equal total.
    assert_eq!(
        stats.alive_modules + stats.dead_modules,
        stats.total_modules,
        "alive ({}) + dead ({}) must equal total ({})",
        stats.alive_modules,
        stats.dead_modules,
        stats.total_modules
    );
}
