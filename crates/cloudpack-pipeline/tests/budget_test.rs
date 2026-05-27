//! Tests for `budget::check` against a synthetic `BuildStatsArtifact`.

use std::collections::HashMap;

use cloudpack_pipeline::budget::{check, BudgetConfig};
use cloudpack_pipeline::build_stats::{
    BuildStatsArtifact, BudgetStatus, ChunkRecord, ChunkRole, EntryPointRecord, SummaryBlock,
    TimingBlock,
};

fn artifact(total: u64, initial: u64, lazy_sizes: &[(&str, u64)]) -> BuildStatsArtifact {
    let mut chunks = vec![ChunkRecord {
        id: "entry".to_string(),
        hash: "a".repeat(64),
        file: format!("chunks/{}.js", "a".repeat(64)),
        size_bytes: initial,
        module_count: 1,
        role: ChunkRole::Entry,
        entry_points: vec!["root".to_string()],
    }];
    for (id, size) in lazy_sizes {
        chunks.push(ChunkRecord {
            id: id.to_string(),
            hash: "b".repeat(64),
            file: format!("chunks/{}.js", "b".repeat(64)),
            size_bytes: *size,
            module_count: 1,
            role: ChunkRole::Lazy,
            entry_points: vec![],
        });
    }

    let mut entry_points = HashMap::new();
    entry_points.insert(
        "root".to_string(),
        EntryPointRecord {
            initial_chunks: vec!["entry".to_string()],
            initial_bytes: initial,
            lazy_chunks: lazy_sizes.iter().map(|(id, _)| id.to_string()).collect(),
        },
    );

    BuildStatsArtifact {
        schema_version: "1".to_string(),
        build_id: "test".to_string(),
        cloudpack_version: "0.0.0-test".to_string(),
        generated_at: "2026-05-16T00:00:00+00:00".to_string(),
        summary: SummaryBlock {
            total_modules: 1,
            alive_modules: 1,
            dead_modules: 0,
            chunks_written: chunks.len() as u32,
            total_bundle_bytes: total,
            largest_chunk_bytes: chunks.iter().map(|c| c.size_bytes).max().unwrap_or(0),
            initial_bundle_bytes: initial,
        },
        timing: TimingBlock {
            build_time_ms: 0,
            summarize_ms: 0,
            analyze_ms: 0,
            transform_ms: 0,
            emit_ms: 0,
        },
        chunks,
        entry_points,
        budget: None,
        previous_build: None,
    }
}

#[test]
fn no_limits_returns_ok_with_zero_checks() {
    let a = artifact(1000, 500, &[]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: None,
        lazy_chunk_max_bytes: None,
        total_bundle_max_bytes: None,
    };
    let result = check(&a, &budget);
    assert!(result.is_ok());
}

#[test]
fn under_all_limits_returns_ok() {
    let a = artifact(1000, 500, &[("lazyA", 200)]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: Some(1000),
        lazy_chunk_max_bytes: Some(1000),
        total_bundle_max_bytes: Some(2000),
    };
    assert!(check(&a, &budget).is_ok());
}

#[test]
fn over_total_bundle_returns_violation() {
    let a = artifact(1_200_000, 500_000, &[]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: None,
        lazy_chunk_max_bytes: None,
        total_bundle_max_bytes: Some(1_000_000),
    };
    let err = check(&a, &budget).expect_err("should violate");
    assert_eq!(err.0.len(), 1);
    assert_eq!(err.0[0].name, "total_bundle_max_bytes");
    assert_eq!(err.0[0].limit, 1_000_000);
    assert_eq!(err.0[0].actual, 1_200_000);
    assert_eq!(err.0[0].status, BudgetStatus::Violated);
    assert!(err.0[0].offender.is_none());
    let msg = err.actionable_message();
    assert!(msg.contains("Budget violated"));
    assert!(msg.contains("total_bundle_max_bytes"));
    assert!(msg.contains("1,200,000"));
    assert!(msg.contains("1,000,000"));
}

#[test]
fn over_initial_bundle_returns_violation() {
    let a = artifact(1000, 800, &[]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: Some(500),
        lazy_chunk_max_bytes: None,
        total_bundle_max_bytes: None,
    };
    let err = check(&a, &budget).expect_err("should violate");
    assert_eq!(err.0[0].name, "initial_bundle_max_bytes");
    assert_eq!(err.0[0].actual, 800);
}

#[test]
fn over_lazy_chunk_returns_violation_with_offender() {
    let a = artifact(1000, 200, &[("smallLazy", 100), ("bigLazy", 350_000)]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: None,
        lazy_chunk_max_bytes: Some(250_000),
        total_bundle_max_bytes: None,
    };
    let err = check(&a, &budget).expect_err("should violate");
    assert_eq!(err.0.len(), 1, "only the over-limit lazy chunk reports");
    assert_eq!(err.0[0].name, "lazy_chunk_max_bytes");
    assert_eq!(err.0[0].actual, 350_000);
    assert_eq!(err.0[0].offender.as_deref(), Some("bigLazy"));
}

#[test]
fn multiple_violations_all_reported() {
    let a = artifact(2_000_000, 800_000, &[("oversize", 600_000)]);
    let budget = BudgetConfig {
        initial_bundle_max_bytes: Some(500_000),
        lazy_chunk_max_bytes: Some(500_000),
        total_bundle_max_bytes: Some(1_000_000),
    };
    let err = check(&a, &budget).expect_err("should violate");
    let names: Vec<&str> = err.0.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"initial_bundle_max_bytes"));
    assert!(names.contains(&"lazy_chunk_max_bytes"));
    assert!(names.contains(&"total_bundle_max_bytes"));
}
