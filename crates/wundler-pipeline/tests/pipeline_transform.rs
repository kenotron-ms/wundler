//! Integration tests for `BuildPipeline::run_transform()`.
//!
//! Acceptance criteria: `cargo test -p wundler-pipeline --test pipeline_transform`
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
        budget: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1: one ChunkOutput per chunk; every output has non-empty code
// ---------------------------------------------------------------------------

#[test]
fn run_transform_returns_chunk_outputs() {
    let (_dir, root) = make_project("test-transform-outputs");
    let config = make_config(root);
    let pipeline = BuildPipeline::new(config);

    // Summarize, then populate source on every node before analysis.
    let mut nodes = pipeline.run_summarize().expect("run_summarize failed");
    for node in &mut nodes {
        let source = fs::read_to_string(&node.path)
            .unwrap_or_else(|_| "// empty\n".to_string());
        node.source = Some(source);
    }

    let analysis = pipeline.run_analyze(nodes).expect("run_analyze failed");
    let outputs = pipeline.run_transform(&analysis).expect("run_transform failed");

    assert_eq!(
        outputs.len(),
        analysis.manifest.chunks.len(),
        "expected one ChunkOutput per manifest chunk"
    );

    for output in &outputs {
        assert!(
            !output.code.is_empty(),
            "chunk {} must produce non-empty code",
            output.chunk_id
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: two runs produce identical outputs (deterministic / parallel-safe)
// ---------------------------------------------------------------------------

#[test]
fn run_transform_is_parallel_safe() {
    let (_dir, root) = make_project("test-transform-parallel");
    let config = make_config(root);
    let pipeline = BuildPipeline::new(config);

    // Summarize + populate source.
    let mut nodes = pipeline.run_summarize().expect("run_summarize failed");
    for node in &mut nodes {
        let source = fs::read_to_string(&node.path)
            .unwrap_or_else(|_| "// empty\n".to_string());
        node.source = Some(source);
    }

    let analysis = pipeline.run_analyze(nodes).expect("run_analyze failed");

    // Run transform twice with the same analysis.
    let mut outputs1 = pipeline.run_transform(&analysis).expect("first run_transform failed");
    let mut outputs2 = pipeline
        .run_transform(&analysis)
        .expect("second run_transform failed");

    // Sort both by chunk_id for a stable comparison.
    outputs1.sort_by(|a, b| a.chunk_id.cmp(&b.chunk_id));
    outputs2.sort_by(|a, b| a.chunk_id.cmp(&b.chunk_id));

    assert_eq!(
        outputs1.len(),
        outputs2.len(),
        "both runs should produce the same number of chunk outputs"
    );

    for (o1, o2) in outputs1.iter().zip(outputs2.iter()) {
        assert_eq!(o1.chunk_id, o2.chunk_id, "chunk_ids should match");
        assert_eq!(
            o1.code, o2.code,
            "code must be identical across runs for chunk {}",
            o1.chunk_id
        );
        assert_eq!(
            o1.hash, o2.hash,
            "hashes must be identical across runs for chunk {}",
            o1.chunk_id
        );
    }
}
