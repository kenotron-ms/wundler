//! Integration tests for `assign_chunks` — commons chunk extraction.
//!
//! These tests verify that modules appearing in ≥ `commons_threshold` candidate
//! chunks are hoisted into a dedicated `"commons"` chunk emitted first.

use std::collections::{HashMap, HashSet};

use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::chunks::assign_chunks;
use wundler_graph::types::LoadCondition;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load the commons fixture from disk and return the parsed nodes.
fn load_fixture() -> Vec<BundleGraphNode> {
    let json = include_str!("fixtures/commons.json");
    serde_json::from_str(json).expect("fixture must deserialise as Vec<BundleGraphNode>")
}

/// Find a node by path, panicking with a helpful message if not found.
fn node_by_path<'a>(nodes: &'a [BundleGraphNode], path: &str) -> &'a BundleGraphNode {
    nodes
        .iter()
        .find(|n| n.path == path)
        .unwrap_or_else(|| panic!("no node with path={path:?} in fixture"))
}

// ---------------------------------------------------------------------------
// Test 1: shared_util extracted to commons when threshold = 2
//
// Fixture layout:
//   entry_a → priv_a1, priv_a2, shared_util  (static)
//   entry_b → priv_b1, priv_b2, shared_util  (static)
//   entry_c → priv_c1, priv_c2, shared_util  (static)
//
// shared_util appears in all 3 candidate chunks → count = 3 ≥ threshold (2)
// → shared_util must be in the "commons" chunk.
// ---------------------------------------------------------------------------

#[test]
fn shared_util_in_commons_when_threshold_2() {
    let nodes = load_fixture();
    let shared_util = node_by_path(&nodes, "shared_util");
    let entry_a = node_by_path(&nodes, "entry_a");
    let entry_b = node_by_path(&nodes, "entry_b");
    let entry_c = node_by_path(&nodes, "entry_c");

    let alive: HashSet<ContentHash> = nodes.iter().map(|n| n.id.clone()).collect();

    let entry_hashes: HashMap<String, ContentHash> = [
        ("a".to_string(), entry_a.id.clone()),
        ("b".to_string(), entry_b.id.clone()),
        ("c".to_string(), entry_c.id.clone()),
    ]
    .into_iter()
    .collect();

    let (chunks, module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 2);

    // commons chunk must be first.
    assert!(
        !chunks.is_empty(),
        "expected at least one chunk, got none"
    );
    let commons = &chunks[0];
    assert_eq!(
        commons.id, "commons",
        "first chunk must be the commons chunk, got {:?}",
        commons.id
    );
    assert_eq!(
        commons.load_condition,
        LoadCondition::Initial,
        "commons chunk must have LoadCondition::Initial"
    );

    // shared_util must be in commons.
    assert!(
        commons.modules.contains(&shared_util.id),
        "shared_util must be in commons chunk; commons.modules = {:?}",
        commons.modules
    );

    // The module_index must map shared_util to "commons".
    assert_eq!(
        module_index.get(&shared_util.id).map(String::as_str),
        Some("commons"),
        "shared_util must be indexed to 'commons' in module_index"
    );

    // commons chunk must contain exactly 1 module (only shared_util qualifies at threshold=2).
    assert_eq!(
        commons.modules.len(),
        1,
        "only shared_util should be in commons at threshold=2, got {:?}",
        commons.modules
    );
}

// ---------------------------------------------------------------------------
// Test 2: private modules are NOT extracted to commons
//
// priv_a1/priv_a2 appear only in the initial_a candidate (count=1 < threshold).
// They must NOT appear in the commons chunk.
// ---------------------------------------------------------------------------

#[test]
fn private_modules_not_in_commons() {
    let nodes = load_fixture();
    let priv_a1 = node_by_path(&nodes, "priv_a1");
    let priv_a2 = node_by_path(&nodes, "priv_a2");
    let priv_b1 = node_by_path(&nodes, "priv_b1");
    let priv_b2 = node_by_path(&nodes, "priv_b2");
    let priv_c1 = node_by_path(&nodes, "priv_c1");
    let priv_c2 = node_by_path(&nodes, "priv_c2");
    let entry_a = node_by_path(&nodes, "entry_a");
    let entry_b = node_by_path(&nodes, "entry_b");
    let entry_c = node_by_path(&nodes, "entry_c");

    let alive: HashSet<ContentHash> = nodes.iter().map(|n| n.id.clone()).collect();

    let entry_hashes: HashMap<String, ContentHash> = [
        ("a".to_string(), entry_a.id.clone()),
        ("b".to_string(), entry_b.id.clone()),
        ("c".to_string(), entry_c.id.clone()),
    ]
    .into_iter()
    .collect();

    let (chunks, _module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 2);

    let commons = chunks
        .iter()
        .find(|c| c.id == "commons")
        .expect("expected a commons chunk");

    let private_hashes = [
        &priv_a1.id, &priv_a2.id,
        &priv_b1.id, &priv_b2.id,
        &priv_c1.id, &priv_c2.id,
    ];

    for hash in private_hashes {
        assert!(
            !commons.modules.contains(hash),
            "private module {:?} must NOT be in commons chunk",
            hash
        );
    }

    // There must be 4 total chunks: commons + 3 initial (a, b, c).
    assert_eq!(
        chunks.len(),
        4,
        "expected commons + 3 route chunks = 4 total, got {:?}",
        chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 3: high threshold (= 4) disables commons extraction
//
// shared_util appears in only 3 candidates; threshold = 4 means no module
// qualifies → no "commons" chunk should be emitted.
// ---------------------------------------------------------------------------

#[test]
fn high_threshold_disables_commons_extraction() {
    let nodes = load_fixture();
    let entry_a = node_by_path(&nodes, "entry_a");
    let entry_b = node_by_path(&nodes, "entry_b");
    let entry_c = node_by_path(&nodes, "entry_c");

    let alive: HashSet<ContentHash> = nodes.iter().map(|n| n.id.clone()).collect();

    let entry_hashes: HashMap<String, ContentHash> = [
        ("a".to_string(), entry_a.id.clone()),
        ("b".to_string(), entry_b.id.clone()),
        ("c".to_string(), entry_c.id.clone()),
    ]
    .into_iter()
    .collect();

    let (chunks, _module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 4);

    // No commons chunk should be present.
    let maybe_commons = chunks.iter().find(|c| c.id == "commons");
    assert!(
        maybe_commons.is_none(),
        "expected NO commons chunk at threshold=4, but found one"
    );

    // There must be exactly 3 initial chunks (a, b, c).
    assert_eq!(
        chunks.len(),
        3,
        "expected 3 route chunks (no commons), got {:?}",
        chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
    );
}
