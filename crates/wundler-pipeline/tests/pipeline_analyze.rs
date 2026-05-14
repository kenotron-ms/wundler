//! Integration tests for `BuildPipeline::run_analyze()`.
//!
//! Acceptance criteria: `cargo test -p wundler-pipeline --test pipeline_analyze`
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
/// - `src/orphan.ts` — not imported by anyone
///
/// `tag` is embedded in each file as a comment so that parallel test instances
/// produce distinct content hashes and avoid cache write races.
///
/// Returns the temp dir (kept alive) and the root path.
fn make_project(tag: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("index.ts"),
        format!("// {tag}\nimport {{ util }} from './util';\nexport const index = util + 1;\n"),
    )
    .unwrap();
    fs::write(src.join("util.ts"), format!("// {tag}\nexport const util = 1;\n")).unwrap();
    fs::write(
        src.join("orphan.ts"),
        format!("// {tag}\nexport const orphan = 99;\n"),
    )
    .unwrap();

    let root = dir.path().to_path_buf();
    (dir, root)
}

fn make_config(root: PathBuf) -> BuildConfig {
    BuildConfig {
        root,
        out_dir: PathBuf::from("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: {
            let mut m = HashMap::new();
            m.insert("main".to_string(), PathBuf::from("src/index.ts"));
            m
        },
    }
}

// ---------------------------------------------------------------------------
// Test 1: manifest contains at least one chunk; at least one node is dead
// ---------------------------------------------------------------------------

#[test]
fn analyze_produces_manifest_and_marks_dead() {
    let (_dir, root) = make_project("test-manifest-dead");
    let config = make_config(root);
    let pipeline = BuildPipeline::new(config);

    let nodes = pipeline.run_summarize().expect("run_summarize failed");
    assert_eq!(nodes.len(), 3, "expected 3 nodes (index, util, orphan)");

    let result = pipeline.run_analyze(nodes).expect("run_analyze failed");

    // Manifest must contain at least one chunk.
    assert!(
        !result.manifest.chunks.is_empty(),
        "expected at least one chunk in manifest"
    );

    // At least one node should be dead (orphan.ts is not reachable from index.ts).
    let dead_count = result.nodes.iter().filter(|n| !n.alive).count();
    assert!(
        dead_count >= 1,
        "expected at least 1 dead node (orphan.ts), got 0"
    );
}

// ---------------------------------------------------------------------------
// Test 2: every alive node appears in manifest.module_index
// ---------------------------------------------------------------------------

#[test]
fn analyze_manifest_indexes_alive_modules() {
    let (_dir, root) = make_project("test-module-index");
    let config = make_config(root);
    let pipeline = BuildPipeline::new(config);

    let nodes = pipeline.run_summarize().expect("run_summarize failed");
    let result = pipeline.run_analyze(nodes).expect("run_analyze failed");

    for node in &result.nodes {
        if node.alive {
            assert!(
                result.manifest.module_index.contains_key(&node.id),
                "alive node id={:?} path={:?} must appear in manifest.module_index",
                node.id,
                node.path
            );
        }
    }
}
