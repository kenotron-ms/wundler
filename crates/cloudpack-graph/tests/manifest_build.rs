//! Integration tests for `build_manifest` — ChunkManifest assembly.
//!
//! Acceptance criteria: `cargo test -p cloudpack-graph --test manifest_build`
//! reports `test result: ok. 4 passed; 0 failed`.

use std::collections::HashMap;

use cloudpack_core::types::ContentHash;
use cloudpack_graph::manifest::build_manifest;
use cloudpack_graph::types::{Chunk, LoadCondition};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal [`Chunk`] with the given id and module list.
fn make_chunk(id: &str, modules: Vec<ContentHash>) -> Chunk {
    Chunk {
        id: id.to_string(),
        modules,
        hash: ContentHash(format!("hash_{id}")),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1: build_id is deterministic for same entry points and is 64 chars
// ---------------------------------------------------------------------------

#[test]
fn build_id_deterministic_and_64_chars() {
    let entry_hashes: HashMap<String, ContentHash> = [
        ("/".to_string(), ContentHash("aaa".to_string())),
        ("/about".to_string(), ContentHash("bbb".to_string())),
    ]
    .into_iter()
    .collect();

    let module_index: HashMap<ContentHash, String> = HashMap::new();
    let chunks: Vec<Chunk> = vec![];

    let m1 = build_manifest(chunks.clone(), &entry_hashes, module_index.clone());
    let m2 = build_manifest(chunks.clone(), &entry_hashes, module_index.clone());

    assert_eq!(m1.build_id, m2.build_id, "build_id must be deterministic");
    assert_eq!(m1.build_id.len(), 64, "build_id must be 64 hex chars");
}

// ---------------------------------------------------------------------------
// Test 2: build_id changes when entry set changes ("/" vs "/admin")
// ---------------------------------------------------------------------------

#[test]
fn build_id_changes_when_entry_set_changes() {
    let entry_hashes_slash: HashMap<String, ContentHash> =
        [("/".to_string(), ContentHash("aaa".to_string()))]
            .into_iter()
            .collect();

    let entry_hashes_admin: HashMap<String, ContentHash> =
        [("/admin".to_string(), ContentHash("bbb".to_string()))]
            .into_iter()
            .collect();

    let module_index: HashMap<ContentHash, String> = HashMap::new();
    let chunks: Vec<Chunk> = vec![];

    let m1 = build_manifest(chunks.clone(), &entry_hashes_slash, module_index.clone());
    let m2 = build_manifest(chunks.clone(), &entry_hashes_admin, module_index.clone());

    assert_ne!(
        m1.build_id, m2.build_id,
        "build_id must differ when entry route set changes"
    );
}

// ---------------------------------------------------------------------------
// Test 3: entry_chunks lists commons before the route chunk (commons_pos < c0_pos)
// ---------------------------------------------------------------------------

#[test]
fn entry_chunks_lists_commons_before_route_chunk() {
    // Two modules: h0 lives in commons, h1 lives in the route chunk.
    let h0 = ContentHash("module_h0".to_string());
    let h1 = ContentHash("module_h1".to_string());

    let chunks = vec![
        make_chunk("commons", vec![h0.clone()]),
        make_chunk("initial_root", vec![h1.clone()]),
    ];

    // Entry "/" is backed by module h1 (owned by "initial_root").
    let entry_hashes: HashMap<String, ContentHash> =
        [("/".to_string(), h1.clone())].into_iter().collect();

    let module_index: HashMap<ContentHash, String> = [
        (h0.clone(), "commons".to_string()),
        (h1.clone(), "initial_root".to_string()),
    ]
    .into_iter()
    .collect();

    let manifest = build_manifest(chunks, &entry_hashes, module_index);

    let ordered = manifest
        .entry_chunks
        .get("/")
        .expect("manifest must have an entry for '/'");

    let commons_pos = ordered
        .iter()
        .position(|c| c == "commons")
        .expect("'commons' must appear in entry_chunks for '/'");
    let c0_pos = ordered
        .iter()
        .position(|c| c == "initial_root")
        .expect("'initial_root' must appear in entry_chunks for '/'");

    assert!(
        commons_pos < c0_pos,
        "commons must come before route chunk in ordered list: {ordered:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: module_index is preserved exactly as passed
// ---------------------------------------------------------------------------

#[test]
fn module_index_preserved_as_passed() {
    let h0 = ContentHash("mod_alpha".to_string());
    let h1 = ContentHash("mod_beta".to_string());

    let module_index: HashMap<ContentHash, String> = [
        (h0.clone(), "chunk_a".to_string()),
        (h1.clone(), "chunk_b".to_string()),
    ]
    .into_iter()
    .collect();

    let entry_hashes: HashMap<String, ContentHash> = HashMap::new();
    let chunks: Vec<Chunk> = vec![];

    let manifest = build_manifest(chunks, &entry_hashes, module_index);

    assert_eq!(
        manifest.module_index.get(&h0).map(String::as_str),
        Some("chunk_a"),
        "module_index must preserve h0 → chunk_a"
    );
    assert_eq!(
        manifest.module_index.get(&h1).map(String::as_str),
        Some("chunk_b"),
        "module_index must preserve h1 → chunk_b"
    );
}
