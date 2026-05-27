use std::collections::HashMap;
use std::io::Write;

use tempfile::NamedTempFile;
use cloudpack_core::types::ContentHash;
use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};
use cloudpack_pgo::types::{ChunkHint, PgoHints};
use cloudpack_pgo::updater::apply_hints;

fn make_manifest(build_id: &str, chunk_ids: &[&str]) -> ChunkManifest {
    let chunks = chunk_ids
        .iter()
        .map(|id| Chunk {
            id: id.to_string(),
            modules: vec![],
            hash: ContentHash(format!("hash-{id}")),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        })
        .collect();

    ChunkManifest {
        build_id: build_id.to_string(),
        chunks,
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    }
}

fn write_manifest(manifest: &ChunkManifest) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    let json = manifest.to_json().unwrap();
    write!(f, "{json}").unwrap();
    f.flush().unwrap();
    f
}

fn make_hints(build_id: &str, chunk_id: &str, score: f64, order: f64, merge: Option<&str>) -> PgoHints {
    let mut chunk_hints = HashMap::new();
    chunk_hints.insert(
        chunk_id.to_string(),
        ChunkHint {
            co_request_score: score,
            median_load_order: order,
            suggested_merge: merge.map(|s| s.to_string()),
        },
    );
    PgoHints {
        build_id: build_id.to_string(),
        chunk_hints,
    }
}

// --- happy path ---

#[test]
fn apply_hints_updates_pgo_fields() {
    let manifest = make_manifest("build-01", &["chunk-a", "chunk-b"]);
    let file = write_manifest(&manifest);

    let hints = make_hints("build-01", "chunk-a", 0.85, 1.5, Some("chunk-b"));
    let stats = apply_hints(file.path(), &hints).unwrap();

    // Read back and verify.
    let updated_json = std::fs::read_to_string(file.path()).unwrap();
    let updated: ChunkManifest = ChunkManifest::from_json(&updated_json).unwrap();

    let a = updated.chunks.iter().find(|c| c.id == "chunk-a").unwrap();
    assert_eq!(a.co_request_score, Some(0.85));
    assert_eq!(a.median_load_order, Some(1.5));
    assert_eq!(a.suggested_merge, Some("chunk-b".to_string()));

    // chunk-b not in hints → unchanged
    let b = updated.chunks.iter().find(|c| c.id == "chunk-b").unwrap();
    assert_eq!(b.co_request_score, None);
    assert_eq!(b.suggested_merge, None);

    assert_eq!(stats.chunks_updated, 1);
    assert_eq!(stats.merge_suggestions, 1);
    assert_eq!(stats.manifest_build_id, "build-01");
}

#[test]
fn apply_hints_multiple_chunks() {
    let manifest = make_manifest("b2", &["x", "y"]);
    let file = write_manifest(&manifest);

    let mut chunk_hints = HashMap::new();
    chunk_hints.insert("x".to_string(), ChunkHint { co_request_score: 0.5, median_load_order: 0.0, suggested_merge: Some("y".to_string()) });
    chunk_hints.insert("y".to_string(), ChunkHint { co_request_score: 0.6, median_load_order: 1.0, suggested_merge: Some("x".to_string()) });
    let hints = PgoHints { build_id: "b2".to_string(), chunk_hints };

    let stats = apply_hints(file.path(), &hints).unwrap();
    assert_eq!(stats.chunks_updated, 2);
    assert_eq!(stats.merge_suggestions, 2);
}

// --- error cases ---

#[test]
fn build_id_mismatch_returns_error() {
    let manifest = make_manifest("build-A", &["c1"]);
    let file = write_manifest(&manifest);

    let hints = make_hints("build-B", "c1", 0.5, 1.0, None);
    let err = apply_hints(file.path(), &hints).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("build_id") || msg.contains("mismatch"),
        "error should mention build_id mismatch, got: {msg}"
    );
}

// --- atomicity ---

#[test]
fn no_tmp_file_left_after_successful_apply() {
    let manifest = make_manifest("b3", &["c1"]);
    let file = write_manifest(&manifest);

    let hints = make_hints("b3", "c1", 0.3, 0.0, None);
    apply_hints(file.path(), &hints).unwrap();

    // The temp file {path}.tmp must not exist after a successful apply.
    let tmp_path = format!("{}.tmp", file.path().display());
    assert!(
        !std::path::Path::new(&tmp_path).exists(),
        "temp file should be removed after atomic rename"
    );
}
