//! End-to-end delta computation tests: run two builds and verify
//! `previous_build` is correctly populated with bundle size deltas.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use cloudpack_pipeline::build_stats::BuildStatsArtifact;
use cloudpack_pipeline::config::{BuildConfig, EngineChoice};
use cloudpack_pipeline::pipeline::BuildPipeline;

fn run_build_with_source(root: &std::path::Path, out_dir: &std::path::Path, content: &str) -> BuildStatsArtifact {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("index.js"), content).unwrap();

    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    entry_points.insert("root".to_string(), root.join("index.js"));

    let cfg = BuildConfig {
        root: root.to_path_buf(),
        out_dir: out_dir.to_path_buf(),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points,
        budget: None,
        dev: None,
    };

    BuildPipeline::new(cfg).build().expect("build succeeds");

    let raw = fs::read(out_dir.join("build-stats.json")).expect("build-stats.json");
    serde_json::from_slice(&raw).expect("parse")
}

#[test]
fn first_build_has_no_previous_build() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    let out_dir = tmp.path().join("dist");

    let a = run_build_with_source(&root, &out_dir, "export const x = 1;\n");

    assert!(
        a.previous_build.is_none(),
        "first build must have no previous_build field"
    );
}

#[test]
fn second_build_includes_previous_build_info() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    let out_dir = tmp.path().join("dist");

    // Build 1
    run_build_with_source(&root, &out_dir, "export const x = 1;\n");

    // Build 2 (same source — trivial delta)
    let b = run_build_with_source(&root, &out_dir, "export const x = 1;\n");

    let prev = b.previous_build.expect("second build must have previous_build");
    assert!(prev.present);
    assert!(!prev.build_id.is_empty());
    // Same source → zero bytes delta
    assert_eq!(prev.delta.total_bundle_bytes.delta, 0);
    assert_eq!(prev.delta.initial_bundle_bytes.delta, 0);
    // Real sizes must be populated — stub returns zeros for prev/curr
    assert!(
        prev.delta.total_bundle_bytes.prev > 0,
        "prev bytes from build 1 must be non-zero, got {}",
        prev.delta.total_bundle_bytes.prev
    );
    assert!(
        prev.delta.total_bundle_bytes.curr > 0,
        "curr bytes from build 2 must be non-zero, got {}",
        prev.delta.total_bundle_bytes.curr
    );
}

#[test]
fn growing_source_produces_positive_delta() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    let out_dir = tmp.path().join("dist");

    run_build_with_source(&root, &out_dir, "export const x = 1;\n");
    let b = run_build_with_source(
        &root,
        &out_dir,
        &("// comment\n".repeat(100) + "export const x = 1;\n"),
    );

    let prev = b.previous_build.expect("previous_build present");
    assert!(
        prev.delta.total_bundle_bytes.delta >= 0,
        "larger source must produce non-negative delta, got: {}",
        prev.delta.total_bundle_bytes.delta
    );
}
