//! Integration tests for `ChunkManifest` JSON round-trip.
//!
//! Exercises `to_json` / `from_json` end-to-end through `build_manifest` with
//! realistic data: a commons chunk, an initial chunk with Some PGO fields, and
//! a lazy chunk.
//!
//! Acceptance criteria: `cargo test -p cloudpack-graph --test manifest_json`
//! reports `test result: ok. 3 passed; 0 failed`.

use std::collections::HashMap;

use cloudpack_core::types::ContentHash;
use cloudpack_graph::manifest::build_manifest;
use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_chunk(
    id: &str,
    modules: Vec<ContentHash>,
    load_condition: LoadCondition,
    co_request_score: Option<f64>,
    median_load_order: Option<f64>,
    suggested_merge: Option<String>,
) -> Chunk {
    Chunk {
        id: id.to_string(),
        modules,
        hash: ContentHash(format!("hash_{id}")),
        load_condition,
        co_request_score,
        median_load_order,
        suggested_merge,
    }
}

/// Build a realistic manifest: commons (Initial, no PGO) + initial_root
/// (Initial, Some PGO fields) + lazy_settings (Lazy, no PGO).
fn realistic_manifest() -> ChunkManifest {
    let h_commons = ContentHash("mod_commons".to_string());
    let h_initial = ContentHash("mod_initial".to_string());
    let h_lazy = ContentHash("mod_lazy".to_string());

    let chunks = vec![
        make_chunk(
            "commons",
            vec![h_commons.clone()],
            LoadCondition::Initial,
            None,
            None,
            None,
        ),
        make_chunk(
            "initial_root",
            vec![h_initial.clone()],
            LoadCondition::Initial,
            Some(0.75),
            Some(2.0),
            Some("commons".to_string()),
        ),
        make_chunk(
            "lazy_settings",
            vec![h_lazy.clone()],
            LoadCondition::Lazy,
            None,
            None,
            None,
        ),
    ];

    let entry_hashes: HashMap<String, ContentHash> =
        [("/".to_string(), h_initial.clone())].into_iter().collect();

    let module_index: HashMap<ContentHash, String> = [
        (h_commons.clone(), "commons".to_string()),
        (h_initial.clone(), "initial_root".to_string()),
        (h_lazy.clone(), "lazy_settings".to_string()),
    ]
    .into_iter()
    .collect();

    build_manifest(chunks, &entry_hashes, module_index)
}

// ---------------------------------------------------------------------------
// Test 1: round-trip preserves all fields
// ---------------------------------------------------------------------------

#[test]
fn round_trip_preserves_all_fields() {
    let original = realistic_manifest();
    let json = original.to_json().expect("to_json must succeed");
    let restored = ChunkManifest::from_json(&json).expect("from_json must succeed");

    // build_id preserved
    assert_eq!(
        original.build_id, restored.build_id,
        "build_id must be preserved"
    );

    // entry_chunks preserved
    assert_eq!(
        original.entry_chunks.len(),
        restored.entry_chunks.len(),
        "entry_chunks length must match"
    );
    for (route, chunk_ids) in &original.entry_chunks {
        let restored_ids = restored
            .entry_chunks
            .get(route)
            .unwrap_or_else(|| panic!("entry_chunks must contain route '{route}'"));
        assert_eq!(
            chunk_ids, restored_ids,
            "entry_chunks for '{route}' must match"
        );
    }

    // module_index preserved
    assert_eq!(
        original.module_index.len(),
        restored.module_index.len(),
        "module_index length must match"
    );
    for (hash, chunk_id) in &original.module_index {
        let restored_id = restored
            .module_index
            .get(hash)
            .unwrap_or_else(|| panic!("module_index must contain hash '{}'", hash.0));
        assert_eq!(
            chunk_id, restored_id,
            "module_index entry for '{}' must match",
            hash.0
        );
    }

    // chunk count preserved
    assert_eq!(
        original.chunks.len(),
        restored.chunks.len(),
        "chunk count must match"
    );

    // initial_root chunk: PGO Option fields are Some — verify they survive
    let orig_initial = original
        .chunks
        .iter()
        .find(|c| c.id == "initial_root")
        .expect("original must have initial_root chunk");
    let rest_initial = restored
        .chunks
        .iter()
        .find(|c| c.id == "initial_root")
        .expect("restored must have initial_root chunk");

    assert_eq!(
        rest_initial.load_condition, orig_initial.load_condition,
        "initial_root load_condition must round-trip"
    );
    assert_eq!(
        rest_initial.co_request_score, orig_initial.co_request_score,
        "co_request_score Some(0.75) must be preserved — not serialised as null"
    );
    assert_eq!(
        rest_initial.median_load_order, orig_initial.median_load_order,
        "median_load_order must be preserved"
    );
    assert_eq!(
        rest_initial.suggested_merge, orig_initial.suggested_merge,
        "suggested_merge must be preserved"
    );

    // lazy_settings chunk: LoadCondition::Lazy must round-trip
    let orig_lazy = original
        .chunks
        .iter()
        .find(|c| c.id == "lazy_settings")
        .expect("original must have lazy_settings chunk");
    let rest_lazy = restored
        .chunks
        .iter()
        .find(|c| c.id == "lazy_settings")
        .expect("restored must have lazy_settings chunk");

    assert_eq!(
        rest_lazy.load_condition,
        LoadCondition::Lazy,
        "lazy chunk load_condition must round-trip as Lazy"
    );
    assert_eq!(
        rest_lazy.load_condition, orig_lazy.load_condition,
        "lazy load_condition must match original"
    );
    assert_eq!(
        rest_lazy.co_request_score, None,
        "None co_request_score must remain None after round-trip"
    );
}

// ---------------------------------------------------------------------------
// Test 2: output is valid pretty-printed JSON with expected top-level keys
// ---------------------------------------------------------------------------

#[test]
fn output_is_pretty_printed_json() {
    let manifest = realistic_manifest();
    let json = manifest.to_json().expect("to_json must succeed");

    // Pretty-printed JSON contains newlines (as opposed to compact serialisation)
    assert!(
        json.contains('\n'),
        "pretty-printed JSON must contain newline characters"
    );

    // The output must parse as a JSON object
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("JSON output must be valid JSON");
    assert!(parsed.is_object(), "JSON output must be a JSON object");

    let obj = parsed.as_object().expect("already confirmed object above");
    assert!(
        obj.contains_key("build_id"),
        "JSON must have 'build_id' key"
    );
    assert!(obj.contains_key("chunks"), "JSON must have 'chunks' key");
    assert!(
        obj.contains_key("entry_chunks"),
        "JSON must have 'entry_chunks' key"
    );
    assert!(
        obj.contains_key("module_index"),
        "JSON must have 'module_index' key"
    );
}

// ---------------------------------------------------------------------------
// Test 3: malformed JSON returns an error
// ---------------------------------------------------------------------------

#[test]
fn malformed_json_returns_error() {
    let result = ChunkManifest::from_json("{ not valid json ]");
    assert!(
        result.is_err(),
        "syntactically malformed JSON must return Err"
    );

    let result2 = ChunkManifest::from_json("");
    assert!(result2.is_err(), "empty string must return Err");

    // Structurally valid JSON but wrong schema (build_id must be a string)
    let result3 = ChunkManifest::from_json(
        "{\"build_id\": 42, \"chunks\": [], \"entry_chunks\": {}, \"module_index\": {}}",
    );
    assert!(result3.is_err(), "wrong type for build_id must return Err");
}
