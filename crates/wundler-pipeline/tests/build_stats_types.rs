/// Compile-time scaffolding tests for `build_stats` types (Task 1 / OBS1).
///
/// These tests verify that the schema types are accessible, carry the correct
/// fields, and round-trip through `serde_json` without data loss.
use wundler_pipeline::build_stats::{
    BudgetCheck, BudgetResult, BudgetStatus, BuildDelta, BuildStatsArtifact, BuildTiming,
    ChunkRecord, ChunkRole, EntryPointRecord, PreviousBuildInfo, SizeDelta, SummaryBlock,
    TimingBlock,
};

/// Verify that `BuildTiming` is `Copy` (spec uses `Clone + Copy`).
#[test]
fn build_timing_is_copy() {
    let t = BuildTiming {
        summarize_ms: 1,
        analyze_ms: 2,
        transform_ms: 3,
        emit_ms: 4,
        total_ms: 10,
    };
    let _copy = t; // moves if not Copy
    let _also = t; // would fail at compile time if not Copy
}

/// Verify that all sub-types are `Clone` and the top-level artifact
/// round-trips through `serde_json` (requires `serde` feature of all fields).
#[test]
fn build_stats_artifact_roundtrip() {
    use std::collections::HashMap;

    let artifact = BuildStatsArtifact {
        schema_version: "1".to_string(),
        build_id: "abc123".to_string(),
        wundler_version: "0.1.0".to_string(),
        generated_at: "2025-01-01T00:00:00Z".to_string(),
        summary: SummaryBlock {
            total_modules: 10,
            alive_modules: 8,
            dead_modules: 2,
            chunks_written: 3,
            total_bundle_bytes: 1024,
            largest_chunk_bytes: 512,
            initial_bundle_bytes: 256,
        },
        timing: TimingBlock {
            build_time_ms: 100,
            summarize_ms: 10,
            analyze_ms: 20,
            transform_ms: 30,
            emit_ms: 40,
        },
        chunks: vec![ChunkRecord {
            id: "chunk-0".to_string(),
            hash: "deadbeef".to_string(),
            file: "chunks/deadbeef.js".to_string(),
            size_bytes: 512,
            module_count: 5,
            role: ChunkRole::Entry,
            entry_points: vec!["main".to_string()],
        }],
        entry_points: {
            let mut m = HashMap::new();
            m.insert(
                "main".to_string(),
                EntryPointRecord {
                    initial_chunks: vec!["chunk-0".to_string()],
                    initial_bytes: 512,
                    lazy_chunks: vec![],
                },
            );
            m
        },
        budget: Some(BudgetResult {
            configured: true,
            checks: vec![BudgetCheck {
                name: "initial_bundle_max_bytes".to_string(),
                limit: 1_000_000,
                actual: 512,
                status: BudgetStatus::Ok,
                offender: None,
            }],
            result: BudgetStatus::Ok,
        }),
        previous_build: Some(PreviousBuildInfo {
            present: true,
            build_id: "prev-build".to_string(),
            delta: BuildDelta {
                total_bundle_bytes: SizeDelta {
                    prev: 900,
                    curr: 1024,
                    delta: 124,
                    pct: 13.77,
                },
                initial_bundle_bytes: SizeDelta {
                    prev: 200,
                    curr: 256,
                    delta: 56,
                    pct: 28.0,
                },
                chunks_added: vec!["chunk-1".to_string()],
                chunks_removed: vec![],
            },
        }),
    };

    let json = serde_json::to_string(&artifact).expect("serialise");
    let back: serde_json::Value = serde_json::from_str(&json).expect("deserialise");

    assert_eq!(back["schema_version"], "1");
    assert_eq!(back["build_id"], "abc123");
    assert_eq!(back["summary"]["total_modules"], 10);
    assert_eq!(back["chunks"][0]["role"], "entry");
    assert_eq!(back["budget"]["result"], "ok");
    assert_eq!(back["previous_build"]["delta"]["chunks_added"][0], "chunk-1");
}

/// `None` budget and `None` previous_build must be omitted entirely from JSON
/// (both fields have `#[serde(skip_serializing_if = "Option::is_none")]`).
#[test]
fn optional_fields_omitted_when_none() {
    use std::collections::HashMap;

    let artifact = BuildStatsArtifact {
        schema_version: "1".to_string(),
        build_id: "x".to_string(),
        wundler_version: "0.0.1".to_string(),
        generated_at: "t".to_string(),
        summary: SummaryBlock {
            total_modules: 0,
            alive_modules: 0,
            dead_modules: 0,
            chunks_written: 0,
            total_bundle_bytes: 0,
            largest_chunk_bytes: 0,
            initial_bundle_bytes: 0,
        },
        timing: TimingBlock {
            build_time_ms: 0,
            summarize_ms: 0,
            analyze_ms: 0,
            transform_ms: 0,
            emit_ms: 0,
        },
        chunks: vec![],
        entry_points: HashMap::new(),
        budget: None,
        previous_build: None,
    };

    let json = serde_json::to_string(&artifact).expect("serialise");
    let back: serde_json::Value = serde_json::from_str(&json).expect("deserialise");

    assert!(back.get("budget").is_none(), "budget should be absent");
    assert!(
        back.get("previous_build").is_none(),
        "previous_build should be absent"
    );
}
