//! End-to-end test: run BuildPipeline::build() and verify build-stats.json
//! contains the full extended schema.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use cloudpack_pipeline::build_stats::BuildStatsArtifact;
use cloudpack_pipeline::config::{BuildConfig, EngineChoice};
use cloudpack_pipeline::pipeline::BuildPipeline;

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
        dev: None,
    };

    let pipeline = BuildPipeline::new(cfg);
    let out = pipeline.build().expect("build succeeds");

    let raw = fs::read(out_dir.join("build-stats.json")).expect("build-stats.json present");
    let artifact: BuildStatsArtifact = serde_json::from_slice(&raw).expect("parses");

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
    let sum: u64 = a.chunks.iter().map(|c| c.size_bytes).sum();
    assert_eq!(a.summary.total_bundle_bytes, sum);
    let max: u64 = a.chunks.iter().map(|c| c.size_bytes).max().unwrap_or(0);
    assert_eq!(a.summary.largest_chunk_bytes, max);
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
    let sum_phases =
        a.timing.summarize_ms + a.timing.analyze_ms + a.timing.transform_ms + a.timing.emit_ms;
    assert!(
        a.timing.build_time_ms >= sum_phases,
        "build_time_ms ({}) < phase sum ({})",
        a.timing.build_time_ms,
        sum_phases
    );
}
