//! Tests that `BuildPipeline::build()` stamps a content-based `build_id`.
//!
//! Acceptance: `cargo test -p wundler-pipeline --test build_id_pipeline_test`
//! reports `test result: ok. 3 passed; 0 failed`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Creates a minimal single-module TypeScript project with a configurable
/// tag string embedded in the source — changing the tag changes the content
/// hash and therefore the build_id.
fn make_project(tag: &str) -> (TempDir, TempDir, BuildConfig) {
    let project_dir = TempDir::new().unwrap();
    let out_dir = TempDir::new().unwrap();

    let src = project_dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("index.ts"),
        format!("// tag:{tag}\nexport const version = \"{tag}\";\n"),
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

/// The build_id in the output manifest must be exactly 16 lowercase hex chars
/// (first 8 bytes of SHA-256), not the old 64-char entry-route hash.
#[test]
fn build_id_is_16_hex_chars() {
    let (_proj, _out, config) = make_project("v1");
    let output = BuildPipeline::new(config).build().expect("build must succeed");

    let id = &output.manifest.build_id;
    assert_eq!(
        id.len(),
        16,
        "build_id must be 16 hex chars, got {id:?} (len={})",
        id.len()
    );
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit()),
        "build_id must be all lowercase hex, got: {id:?}"
    );
}

/// Two builds of identical source must produce the same build_id.
#[test]
fn build_id_is_reproducible() {
    let (_proj1, _out1, config1) = make_project("same-content");
    let (_proj2, _out2, config2) = make_project("same-content");

    let output1 = BuildPipeline::new(config1).build().expect("build 1 must succeed");
    let output2 = BuildPipeline::new(config2).build().expect("build 2 must succeed");

    assert_eq!(
        output1.manifest.build_id, output2.manifest.build_id,
        "identical builds must produce the same build_id"
    );
}

/// A build with different source content must produce a different build_id.
#[test]
fn build_id_changes_with_content() {
    let (_proj1, _out1, config1) = make_project("version-alpha");
    let (_proj2, _out2, config2) = make_project("version-beta");

    let output1 = BuildPipeline::new(config1).build().expect("build 1 must succeed");
    let output2 = BuildPipeline::new(config2).build().expect("build 2 must succeed");

    assert_ne!(
        output1.manifest.build_id, output2.manifest.build_id,
        "builds with different source content must produce different build_ids"
    );
}
