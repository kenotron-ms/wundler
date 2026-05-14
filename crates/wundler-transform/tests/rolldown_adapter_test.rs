//! Tests for the new RolldownAdapter batch-build path (Path B).
//!
//! Unit tests run always; integration tests are gated by the
//! `WUNDLER_INTEGRATION_TEST=1` environment variable.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use wundler_transform::engine::BatchConfig;
use wundler_transform::rolldown_adapter::RolldownAdapter;

// ---------------------------------------------------------------------------
// Unit test 1: find_rolldown_bin walks up from root to find local binary
// ---------------------------------------------------------------------------

#[test]
fn find_rolldown_bin_finds_bin_in_parent_node_modules() {
    let tmp = tempfile::TempDir::new().unwrap();
    // The rolldown binary lives in tmp/node_modules/.bin/
    let bin_dir = tmp.path().join("node_modules").join(".bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let bin_file = bin_dir.join("rolldown");
    fs::write(&bin_file, "#!/usr/bin/env node\n").unwrap();

    // scan root is tmp/src — rolldown is NOT here, but is in tmp/
    let scan_root = tmp.path().join("src");
    fs::create_dir_all(&scan_root).unwrap();

    let found = RolldownAdapter::find_rolldown_bin(&scan_root);
    assert_eq!(
        found, bin_file,
        "expected to walk up from scan_root and find rolldown in parent"
    );
}

// ---------------------------------------------------------------------------
// Unit test 2: find_rolldown_bin falls back to "rolldown" when none found
// ---------------------------------------------------------------------------

#[test]
fn find_rolldown_bin_falls_back_to_path_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    // No node_modules anywhere in the tmpdir tree
    let scan_root = tmp.path().join("src");
    fs::create_dir_all(&scan_root).unwrap();

    let found = RolldownAdapter::find_rolldown_bin(&scan_root);
    assert_eq!(
        found,
        PathBuf::from("rolldown"),
        "should fall back to 'rolldown' (PATH resolution) when not found locally"
    );
}

// ---------------------------------------------------------------------------
// Unit test 3: write_rolldown_config produces a valid .mjs file
// ---------------------------------------------------------------------------

#[test]
fn write_rolldown_config_produces_valid_mjs() {
    let tmp = tempfile::TempDir::new().unwrap();

    let mut entry_points = HashMap::new();
    entry_points.insert("/".to_string(), PathBuf::from("/workspace/src/main.tsx"));

    let config = BatchConfig {
        root: PathBuf::from("/workspace/src"),
        entry_points,
        out_dir: PathBuf::from("/workspace/dist"),
    };

    let cfg_path = RolldownAdapter::write_rolldown_config(&config, tmp.path())
        .expect("write_rolldown_config should succeed");

    assert!(cfg_path.exists(), "config file must be written to disk");
    assert_eq!(
        cfg_path.extension().and_then(|e| e.to_str()),
        Some("mjs"),
        "config file must have .mjs extension"
    );

    let content = fs::read_to_string(&cfg_path).expect("should read config");

    // Must export a default config object
    assert!(
        content.contains("export default"),
        "config must have 'export default'"
    );
    // Entry key "/" → sanitized to "root"
    assert!(content.contains("root"), "sanitized entry key 'root' must appear");
    // Entry path (absolute) must be referenced
    assert!(
        content.contains("/workspace/src/main.tsx"),
        "absolute entry path must appear in config"
    );
    // Output dir (absolute) must be referenced
    assert!(
        content.contains("/workspace/dist"),
        "absolute output dir must appear in config"
    );
    // Must use ESM format
    assert!(content.contains("esm"), "config must specify esm format");
}

// ---------------------------------------------------------------------------
// Unit test 4: sanitize_entry_key helper
// ---------------------------------------------------------------------------

#[test]
fn sanitize_entry_key_converts_slash_to_root() {
    use wundler_transform::engine::sanitize_entry_key;

    assert_eq!(sanitize_entry_key("/"), "root");
    assert_eq!(sanitize_entry_key("/dashboard"), "dashboard");
    assert_eq!(sanitize_entry_key("/admin/users"), "admin-users");
}

// ---------------------------------------------------------------------------
// Integration test (WUNDLER_INTEGRATION_TEST=1 only)
// ---------------------------------------------------------------------------

#[test]
fn rolldown_batch_transform_integration() {
    if std::env::var("WUNDLER_INTEGRATION_TEST").is_err() {
        // Skip unless opt-in
        return;
    }

    use wundler_graph::analyzer::{AnalysisResult, AnalysisStats, GraphAnalyzer};
    use wundler_transform::engine::TransformEngine;

    // Use the test-app directory which has rolldown installed
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let out_dir = workspace_root.join("test-app").join("dist-integration-test");
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir).unwrap();

    let mut entry_points = HashMap::new();
    entry_points.insert(
        "/".to_string(),
        workspace_root.join("test-app").join("src").join("main.tsx"),
    );

    let config = BatchConfig {
        root: workspace_root.join("test-app").join("src"),
        entry_points,
        out_dir: out_dir.clone(),
    };

    // We need a minimal AnalysisResult to pass to batch_transform
    // (the rolldown adapter doesn't use it for actual bundling)
    let scan_root = workspace_root.join("test-app").join("src");
    use wundler_core::cache::local::LocalCache;
    use wundler_core::summarizer::summarize_directory;
    let cache = LocalCache::with_default_root().unwrap();
    let nodes = summarize_directory(&scan_root, &cache).unwrap();

    let mut analyzer = GraphAnalyzer::new({
        let mut m = HashMap::new();
        m.insert(
            "/".to_string(),
            workspace_root.join("test-app").join("src").join("main.tsx"),
        );
        m
    });
    let analysis = analyzer.analyze(nodes).unwrap();

    let adapter = RolldownAdapter::new();
    let outputs = adapter
        .batch_transform(&analysis, &config)
        .expect("batch_transform should succeed");

    assert!(!outputs.is_empty(), "must produce at least one ChunkOutput");
    for out in &outputs {
        assert!(
            out.already_written,
            "rolldown outputs must be already_written"
        );
        assert!(!out.hash.as_str().is_empty(), "hash must be non-empty");
    }

    // Verify output dir has JS files
    let js_files: Vec<_> = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                == Some("js")
        })
        .collect();
    assert!(!js_files.is_empty(), "rolldown must produce at least one JS file");

    let _ = fs::remove_dir_all(&out_dir);
}
