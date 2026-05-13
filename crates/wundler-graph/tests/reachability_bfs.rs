use std::collections::HashSet;

use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::reachability::compute_reachability;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load the multi_entry fixture from disk and return the parsed nodes.
fn load_fixture() -> Vec<BundleGraphNode> {
    let json = include_str!("fixtures/multi_entry.json");
    serde_json::from_str(json).expect("fixture must deserialise as Vec<BundleGraphNode>")
}

/// Convenience: look up a node's ContentHash by its `path` field.
fn hash_of(nodes: &[BundleGraphNode], path: &str) -> ContentHash {
    nodes
        .iter()
        .find(|n| n.path == path)
        .unwrap_or_else(|| panic!("no node with path={path:?} in fixture"))
        .id
        .clone()
}

// ---------------------------------------------------------------------------
// Test 1: BFS visits static + dynamic transitive descendants
// ---------------------------------------------------------------------------

/// Entry `a` has static imports (x1, shared) and a dynamic import (lazy_a1).
/// x1 in turn statically imports x2.  All of {a, x1, x2, lazy_a1, shared}
/// must be alive; unreachable nodes (b, c, y1, y2, z1, z2, lazy_b1,
/// lazy_c1, cir1, cir2) must not appear.
#[test]
fn bfs_follows_static_and_dynamic_transitive_descendants() {
    let nodes = load_fixture();

    let entry = hash_of(&nodes, "a");
    let entry_set: HashSet<ContentHash> = [entry].into_iter().collect();

    let alive = compute_reachability(&nodes, &entry_set);

    // Must be alive
    for path in ["a", "x1", "x2", "lazy_a1", "shared"] {
        let h = hash_of(&nodes, path);
        assert!(
            alive.contains(&h),
            "expected {path:?} to be alive when entry=a"
        );
    }

    // Must NOT be alive
    for path in ["b", "c", "y1", "y2", "z1", "z2", "lazy_b1", "lazy_c1", "cir1", "cir2"] {
        let h = hash_of(&nodes, path);
        assert!(
            !alive.contains(&h),
            "expected {path:?} NOT to be alive when entry=a"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: Multiple entries union their reachability
// ---------------------------------------------------------------------------

/// Entries {a, b} should produce the union of each entry's reachable set.
/// `shared` is reachable from both a and b — it should appear exactly once
/// (set semantics).  Modules only reachable from c (z1, z2, lazy_c1) and
/// the unreachable cycle (cir1, cir2) must not appear.
#[test]
fn multiple_entries_union_their_reachability() {
    let nodes = load_fixture();

    let entry_set: HashSet<ContentHash> = [hash_of(&nodes, "a"), hash_of(&nodes, "b")]
        .into_iter()
        .collect();

    let alive = compute_reachability(&nodes, &entry_set);

    // a's subtree
    for path in ["a", "x1", "x2", "lazy_a1"] {
        let h = hash_of(&nodes, path);
        assert!(alive.contains(&h), "expected {path:?} alive (from entry a)");
    }
    // b's subtree
    for path in ["b", "y1", "y2", "lazy_b1"] {
        let h = hash_of(&nodes, path);
        assert!(alive.contains(&h), "expected {path:?} alive (from entry b)");
    }
    // shared by both
    assert!(
        alive.contains(&hash_of(&nodes, "shared")),
        "expected 'shared' alive (reachable from both a and b)"
    );

    // NOT alive: c's subtree + the cycle
    for path in ["c", "z1", "z2", "lazy_c1", "cir1", "cir2"] {
        let h = hash_of(&nodes, path);
        assert!(
            !alive.contains(&h),
            "expected {path:?} NOT alive when entries={{a,b}}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: Empty entry set yields empty alive set
// ---------------------------------------------------------------------------

#[test]
fn empty_entry_set_yields_empty_alive_set() {
    let nodes = load_fixture();
    let entry_set: HashSet<ContentHash> = HashSet::new();

    let alive = compute_reachability(&nodes, &entry_set);

    assert!(
        alive.is_empty(),
        "expected empty alive set for empty entry set, got {} entries",
        alive.len()
    );
}

// ---------------------------------------------------------------------------
// Test 4: Entry with no imports is alive alone
// ---------------------------------------------------------------------------

/// `x2` has no outgoing imports.  Using it as the sole entry should produce
/// an alive set containing only x2 — no transitive expansion.
#[test]
fn entry_with_no_imports_is_alive_alone() {
    let nodes = load_fixture();

    let entry = hash_of(&nodes, "x2");
    let entry_set: HashSet<ContentHash> = [entry.clone()].into_iter().collect();

    let alive = compute_reachability(&nodes, &entry_set);

    assert!(
        alive.contains(&entry),
        "expected x2 itself to be alive"
    );
    assert_eq!(
        alive.len(),
        1,
        "expected alive set to contain only x2, but got {} entries: {:?}",
        alive.len(),
        alive
    );
}
