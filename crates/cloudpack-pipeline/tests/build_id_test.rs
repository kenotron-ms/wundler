//! Unit tests for `cloudpack_pipeline::build_id`.
//!
//! Acceptance: `cargo test -p cloudpack-pipeline --test build_id_test`
//! reports `test result: ok. 4 passed; 0 failed`.

use std::collections::HashMap;

use cloudpack_core::types::ContentHash;
use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};
use cloudpack_pipeline::build_id::{canonical_bytes, compute_build_id};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_chunk(id: &str, module_hashes: Vec<&str>) -> Chunk {
    Chunk {
        id: id.to_string(),
        modules: module_hashes
            .into_iter()
            .map(|h| ContentHash(h.to_string()))
            .collect(),
        hash: ContentHash("aabbccdd00112233445566778899aabb".to_string()),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    }
}

fn make_manifest(build_id: &str, chunks: Vec<Chunk>) -> ChunkManifest {
    let module_index: HashMap<ContentHash, String> = chunks
        .iter()
        .flat_map(|c| c.modules.iter().map(|m| (m.clone(), c.id.clone())))
        .collect();
    ChunkManifest {
        build_id: build_id.to_string(),
        entry_chunks: HashMap::new(),
        chunks,
        module_index,
    }
}

// ---------------------------------------------------------------------------
// Test 1: determinism — same input produces the same output
// ---------------------------------------------------------------------------

#[test]
fn test_compute_build_id_is_deterministic() {
    let manifest_a = make_manifest("run-1", vec![make_chunk("chunk-0", vec!["mod_abc"])]);
    let manifest_b = make_manifest("run-1", vec![make_chunk("chunk-0", vec!["mod_abc"])]);

    let id_a = compute_build_id(&manifest_a);
    let id_b = compute_build_id(&manifest_b);

    assert_eq!(
        id_a, id_b,
        "identical manifest structure must produce identical build_id"
    );
    assert_eq!(
        id_a.len(),
        16,
        "build_id must be exactly 16 hex characters, got: {id_a:?}"
    );
    assert!(
        id_a.chars().all(|c| c.is_ascii_hexdigit()),
        "build_id must be all lowercase hex, got: {id_a:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: content sensitivity — different content produces different output
// ---------------------------------------------------------------------------

#[test]
fn test_compute_build_id_is_content_sensitive() {
    let manifest_a = make_manifest("same-id", vec![make_chunk("chunk-0", vec!["mod_abc"])]);
    let manifest_b = make_manifest("same-id", vec![make_chunk("chunk-0", vec!["mod_xyz"])]);

    let id_a = compute_build_id(&manifest_a);
    let id_b = compute_build_id(&manifest_b);

    assert_ne!(
        id_a, id_b,
        "manifests with different module content must produce different build_ids"
    );
}

// ---------------------------------------------------------------------------
// Test 3: order independence — Vec order of chunks must not matter
// ---------------------------------------------------------------------------

#[test]
fn test_compute_build_id_is_order_independent() {
    // Same two chunks, reversed Vec order.
    let manifest_alpha = make_manifest(
        "ignored",
        vec![
            make_chunk("chunk-a", vec!["mod_1"]),
            make_chunk("chunk-b", vec!["mod_2"]),
        ],
    );
    let manifest_beta = make_manifest(
        "ignored",
        vec![
            make_chunk("chunk-b", vec!["mod_2"]),
            make_chunk("chunk-a", vec!["mod_1"]),
        ],
    );

    let id_alpha = compute_build_id(&manifest_alpha);
    let id_beta = compute_build_id(&manifest_beta);

    assert_eq!(
        id_alpha, id_beta,
        "chunk Vec order must not affect build_id (chunks must be sorted by id before hashing)"
    );
}

// ---------------------------------------------------------------------------
// Test 4: build_id field exclusion — the field itself must not affect the hash
// ---------------------------------------------------------------------------

#[test]
fn test_canonical_bytes_excludes_build_id_field() {
    let manifest_a =
        make_manifest("placeholder-from-run-1", vec![make_chunk("chunk-0", vec!["mod_x"])]);
    let manifest_b =
        make_manifest("placeholder-from-run-2", vec![make_chunk("chunk-0", vec!["mod_x"])]);

    let bytes_a = canonical_bytes(&manifest_a);
    let bytes_b = canonical_bytes(&manifest_b);

    assert_eq!(
        bytes_a, bytes_b,
        "canonical_bytes must be identical when ONLY build_id differs"
    );
}
