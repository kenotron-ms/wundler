use std::collections::HashMap;
use cloudpack_core::types::ContentHash;
use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};

/// Test 1: chunk construction populates all fields.
#[test]
fn chunk_construction_populates_all_fields() {
    let hash = ContentHash::from_source("main.js");
    let dep_hash = ContentHash::from_source("dep.js");

    let chunk = Chunk {
        id: "chunk-0".to_string(),
        modules: vec![dep_hash.clone()],
        hash: hash.clone(),
        load_condition: LoadCondition::Initial,
        co_request_score: Some(0.85),
        median_load_order: Some(1.0),
        suggested_merge: Some("chunk-1".to_string()),
    };

    assert_eq!(chunk.id, "chunk-0");
    assert_eq!(chunk.modules.len(), 1);
    assert_eq!(chunk.modules[0], dep_hash);
    assert_eq!(chunk.hash, hash);
    assert_eq!(chunk.load_condition, LoadCondition::Initial);
    assert_eq!(chunk.co_request_score, Some(0.85));
    assert_eq!(chunk.median_load_order, Some(1.0));
    assert_eq!(chunk.suggested_merge, Some("chunk-1".to_string()));
}

/// Test 2: LoadCondition round-trips for all 3 variants.
#[test]
fn load_condition_round_trips_all_variants() {
    for variant in [
        LoadCondition::Initial,
        LoadCondition::Lazy,
        LoadCondition::Prefetch,
    ] {
        let json = serde_json::to_string(&variant).expect("serialize failed");
        let recovered: LoadCondition = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(recovered, variant, "round-trip failed for {variant:?}");
    }
}

/// Test 3: ChunkManifest round-trips with all fields preserved.
#[test]
fn chunk_manifest_round_trips_with_all_fields_preserved() {
    let module_hash = ContentHash::from_source("index.ts");
    let chunk_hash = ContentHash::from_source("chunk-content");

    let chunk = Chunk {
        id: "chunk-main".to_string(),
        modules: vec![module_hash.clone()],
        hash: chunk_hash.clone(),
        load_condition: LoadCondition::Initial,
        co_request_score: Some(1.0),
        median_load_order: Some(0.0),
        suggested_merge: None,
    };

    let mut entry_chunks: HashMap<String, Vec<String>> = HashMap::new();
    entry_chunks.insert("main".to_string(), vec!["chunk-main".to_string()]);

    let mut module_index: HashMap<ContentHash, String> = HashMap::new();
    module_index.insert(module_hash.clone(), "chunk-main".to_string());

    let manifest = ChunkManifest {
        build_id: "build-abc123".to_string(),
        chunks: vec![chunk],
        entry_chunks,
        module_index,
    };

    let json = manifest.to_json().expect("to_json failed");

    // Verify pretty-printing (contains newlines/indentation)
    assert!(json.contains('\n'), "expected pretty-printed JSON");

    let recovered = ChunkManifest::from_json(&json).expect("from_json failed");

    assert_eq!(recovered.build_id, manifest.build_id);
    assert_eq!(recovered.chunks.len(), 1);
    assert_eq!(recovered.chunks[0].id, "chunk-main");
    assert_eq!(recovered.chunks[0].hash, chunk_hash);
    assert_eq!(recovered.chunks[0].load_condition, LoadCondition::Initial);
    assert_eq!(recovered.chunks[0].co_request_score, Some(1.0));
    assert_eq!(recovered.chunks[0].suggested_merge, None);
    assert_eq!(
        recovered.entry_chunks.get("main"),
        Some(&vec!["chunk-main".to_string()])
    );
    assert_eq!(
        recovered.module_index.get(&module_hash),
        Some(&"chunk-main".to_string())
    );
}

/// Test 4: from_json rejects malformed input.
#[test]
fn from_json_rejects_malformed_input() {
    let result = ChunkManifest::from_json("{ not valid json }");
    assert!(result.is_err(), "expected error for malformed JSON");

    let result2 = ChunkManifest::from_json(r#"{"build_id": "x"}"#);
    assert!(
        result2.is_err(),
        "expected error for missing required fields"
    );
}
