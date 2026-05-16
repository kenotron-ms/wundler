//! Integration tests for `output::write_build_stats` (atomic) and
//! `output::read_previous_stats`.

use std::collections::HashMap;

use wundler_pipeline::build_stats::{
    BuildStatsArtifact, ChunkRecord, ChunkRole, EntryPointRecord, SummaryBlock, TimingBlock,
};
use wundler_pipeline::output;

fn sample_artifact(build_id: &str) -> BuildStatsArtifact {
    let mut entry_points = HashMap::new();
    entry_points.insert(
        "root".to_string(),
        EntryPointRecord {
            initial_chunks: vec!["entry".to_string()],
            initial_bytes: 1000,
            lazy_chunks: vec![],
        },
    );

    BuildStatsArtifact {
        schema_version: "1".to_string(),
        build_id: build_id.to_string(),
        wundler_version: "0.0.0-test".to_string(),
        generated_at: "2026-05-16T00:00:00+00:00".to_string(),
        summary: SummaryBlock {
            total_modules: 1,
            alive_modules: 1,
            dead_modules: 0,
            chunks_written: 1,
            total_bundle_bytes: 1000,
            largest_chunk_bytes: 1000,
            initial_bundle_bytes: 1000,
        },
        timing: TimingBlock {
            build_time_ms: 100,
            summarize_ms: 10,
            analyze_ms: 20,
            transform_ms: 30,
            emit_ms: 40,
        },
        chunks: vec![ChunkRecord {
            id: "entry".to_string(),
            hash: "a".repeat(64),
            file: format!("chunks/{}.js", "a".repeat(64)),
            size_bytes: 1000,
            module_count: 1,
            role: ChunkRole::Entry,
            entry_points: vec!["root".to_string()],
        }],
        entry_points,
        budget: None,
        previous_build: None,
    }
}

#[test]
fn write_then_read_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let artifact = sample_artifact("buildA");

    output::write_build_stats(tmp.path(), &artifact).expect("write succeeded");

    let read = output::read_previous_stats(tmp.path()).expect("file present");
    assert_eq!(read.schema_version, "1");
    assert_eq!(read.build_id, "buildA");
    assert_eq!(read.summary.total_bundle_bytes, 1000);
    assert_eq!(read.chunks.len(), 1);
    assert_eq!(read.chunks[0].id, "entry");
}

#[test]
fn write_is_atomic_no_tmp_file_left_behind() {
    let tmp = tempfile::tempdir().unwrap();
    output::write_build_stats(tmp.path(), &sample_artifact("idA")).unwrap();

    let mut names: Vec<String> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    assert_eq!(names, vec!["build-stats.json".to_string()]);
}

#[test]
fn write_overwrites_existing_file() {
    let tmp = tempfile::tempdir().unwrap();

    output::write_build_stats(tmp.path(), &sample_artifact("idA")).unwrap();
    output::write_build_stats(tmp.path(), &sample_artifact("idB")).unwrap();

    let read = output::read_previous_stats(tmp.path()).unwrap();
    assert_eq!(read.build_id, "idB");
}

#[test]
fn read_previous_stats_returns_none_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(output::read_previous_stats(tmp.path()).is_none());
}

#[test]
fn read_previous_stats_returns_none_for_corrupt_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("build-stats.json"), b"{not json").unwrap();
    assert!(output::read_previous_stats(tmp.path()).is_none());
}
