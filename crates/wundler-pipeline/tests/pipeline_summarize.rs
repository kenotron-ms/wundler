//! Integration tests for `BuildPipeline::run_summarize()`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

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

#[test]
fn pipeline_summarize_step_walks_root() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("index.ts"), "export const index = 1;").unwrap();
    fs::write(src.join("util.ts"), "export const util = 2;").unwrap();

    let config = make_config(src.clone());
    let pipeline = BuildPipeline::new(config);
    let nodes = pipeline.run_summarize().expect("run_summarize failed");

    assert_eq!(nodes.len(), 2, "expected 2 nodes, got {}", nodes.len());

    let has_index = nodes.iter().any(|n| n.path.ends_with("index.ts"));
    let has_util = nodes.iter().any(|n| n.path.ends_with("util.ts"));
    assert!(has_index, "expected a node with path ending in 'index.ts'");
    assert!(has_util, "expected a node with path ending in 'util.ts'");
}

#[test]
fn pipeline_summarize_empty_root_returns_empty() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    let config = make_config(src.clone());
    let pipeline = BuildPipeline::new(config);
    let nodes = pipeline.run_summarize().expect("run_summarize failed");

    assert_eq!(nodes.len(), 0, "expected 0 nodes for empty dir, got {}", nodes.len());
}
