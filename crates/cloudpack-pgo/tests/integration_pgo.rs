//! End-to-end integration test for the PGO pipeline.
//!
//! Verifies that inserting synthetic telemetry sessions, running C³ clustering,
//! computing hints, and applying them to a manifest all work together correctly.

use std::collections::HashMap;
use std::io::Write;

use tempfile::NamedTempFile;
use cloudpack_core::types::ContentHash;
use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};
use cloudpack_pgo::clustering::C3Config;
use cloudpack_pgo::hints::compute_hints;
use cloudpack_pgo::store::PgoStore;
use cloudpack_pgo::types::SessionRecord;
use cloudpack_pgo::updater::apply_hints;

fn mk_record(session_id: &str, chunks: &[&str]) -> SessionRecord {
    SessionRecord {
        session_id: session_id.to_string(),
        entry_point: "home".to_string(),
        chunk_sequence: chunks.iter().map(|s| s.to_string()).collect(),
        timestamp_ms: 0,
    }
}

fn chunk(id: &str) -> Chunk {
    Chunk {
        id: id.to_string(),
        modules: vec![],
        hash: ContentHash(format!("hash-{id}")),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    }
}

/// Write a [`ChunkManifest`] to a temp file and return it.
fn write_manifest(m: &ChunkManifest) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "{}", m.to_json().unwrap()).unwrap();
    f.flush().unwrap();
    f
}

#[test]
fn full_pipeline_500_sessions_produces_merge_suggestions() {
    // -----------------------------------------------------------------------
    // Step 1 — open an in-memory store.
    // -----------------------------------------------------------------------
    let store = PgoStore::open_in_memory().unwrap();

    // -----------------------------------------------------------------------
    // Step 2 — insert 500 synthetic sessions.
    //
    //   0–399  : ["commons", "initial_root"]
    //   400–449: ["commons", "initial_root", "lazy_1"]
    //   450–499: ["commons", "initial_root", "lazy_2"]
    // -----------------------------------------------------------------------
    for i in 0..400usize {
        let id = format!("s{i}");
        store
            .insert_session(&mk_record(&id, &["commons", "initial_root"]))
            .unwrap();
    }
    for i in 400..450usize {
        let id = format!("s{i}");
        store
            .insert_session(&mk_record(&id, &["commons", "initial_root", "lazy_1"]))
            .unwrap();
    }
    for i in 450..500usize {
        let id = format!("s{i}");
        store
            .insert_session(&mk_record(&id, &["commons", "initial_root", "lazy_2"]))
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Step 3 — verify co-load counts.
    // -----------------------------------------------------------------------
    assert_eq!(
        store.co_load_count("commons", "initial_root").unwrap(),
        500,
        "commons and initial_root co-loaded in all 500 sessions"
    );
    assert_eq!(
        store.co_load_count("initial_root", "lazy_1").unwrap(),
        50,
        "initial_root and lazy_1 co-loaded in 50 sessions"
    );

    // -----------------------------------------------------------------------
    // Step 4 — compute hints.
    // -----------------------------------------------------------------------
    let build_id = "integ-001";
    let chunk_ids: Vec<String> =
        vec!["commons".into(), "initial_root".into(), "lazy_1".into(), "lazy_2".into()];

    let config = C3Config::default(); // threshold 0.70, min_sessions 100
    let hints = compute_hints(&store, build_id, &chunk_ids, &config).unwrap();

    // -----------------------------------------------------------------------
    // Step 5 — verify merge suggestions.
    //
    //   P(initial_root|commons)    = 500/500 = 1.0  > 0.70 ✓
    //   P(commons|initial_root)    = 500/500 = 1.0  > 0.70 ✓
    //   → bidirectional: both get a suggested_merge pointing to each other.
    //
    //   P(lazy_1|commons)          = 50/500  = 0.10 < 0.70 ✗
    //   P(lazy_1|initial_root)     = 50/500  = 0.10 < 0.70 ✗
    //   P(lazy_2|commons)          = 50/500  = 0.10 < 0.70 ✗
    //   → no merge suggestions for lazy_1 / lazy_2.
    // -----------------------------------------------------------------------
    let commons_hint = hints.chunk_hints.get("commons").expect("commons hint missing");
    let initial_hint = hints
        .chunk_hints
        .get("initial_root")
        .expect("initial_root hint missing");
    let lazy1_hint = hints.chunk_hints.get("lazy_1").expect("lazy_1 hint missing");
    let lazy2_hint = hints.chunk_hints.get("lazy_2").expect("lazy_2 hint missing");

    assert_eq!(
        commons_hint.suggested_merge,
        Some("initial_root".to_string()),
        "commons should suggest merging with initial_root"
    );
    assert_eq!(
        initial_hint.suggested_merge,
        Some("commons".to_string()),
        "initial_root should suggest merging with commons"
    );
    assert_eq!(
        lazy1_hint.suggested_merge, None,
        "lazy_1 should have no merge suggestion (P = 0.10 < 0.70)"
    );
    assert_eq!(
        lazy2_hint.suggested_merge, None,
        "lazy_2 should have no merge suggestion (P = 0.10 < 0.70)"
    );

    // -----------------------------------------------------------------------
    // Step 6 — write a temp manifest, apply hints, read back, verify.
    // -----------------------------------------------------------------------
    let manifest = ChunkManifest {
        build_id: build_id.to_string(),
        chunks: vec![
            chunk("commons"),
            chunk("initial_root"),
            chunk("lazy_1"),
            chunk("lazy_2"),
        ],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    };

    let manifest_file = write_manifest(&manifest);
    let stats = apply_hints(manifest_file.path(), &hints).unwrap();

    assert_eq!(stats.chunks_updated, 4, "all 4 chunks should be updated");
    assert_eq!(stats.merge_suggestions, 2, "commons and initial_root each get a merge suggestion");
    assert_eq!(stats.manifest_build_id, build_id);

    // Read back and verify PGO fields are persisted.
    let updated_json = std::fs::read_to_string(manifest_file.path()).unwrap();
    let updated = ChunkManifest::from_json(&updated_json).unwrap();

    let commons_chunk = updated
        .chunks
        .iter()
        .find(|c| c.id == "commons")
        .unwrap();
    let lazy1_chunk = updated
        .chunks
        .iter()
        .find(|c| c.id == "lazy_1")
        .unwrap();

    assert_eq!(
        commons_chunk.suggested_merge,
        Some("initial_root".to_string()),
        "manifest should reflect commons→initial_root merge suggestion"
    );
    assert!(
        commons_chunk.co_request_score.is_some(),
        "co_request_score should be populated after apply"
    );
    assert_eq!(
        lazy1_chunk.suggested_merge, None,
        "lazy_1 should have no merge suggestion in the persisted manifest"
    );
}
