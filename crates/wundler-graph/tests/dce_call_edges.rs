//! Integration tests for `compute_dead_exports` — call-edge transitive DCE.

use std::collections::HashSet;

use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::dce::compute_dead_exports;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load the dead_code fixture from disk and return the parsed nodes.
fn load_fixture() -> Vec<BundleGraphNode> {
    let json = include_str!("fixtures/dead_code.json");
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

/// Build the alive set: entry, util_a, util_a_dep, util_b, and the r1..r8 reachable chain.
/// The d1..d8 modules are intentionally excluded (they are dead/unreachable).
fn alive_set(nodes: &[BundleGraphNode]) -> HashSet<ContentHash> {
    let mut alive = HashSet::new();
    for path in [
        "entry", "util_a", "util_a_dep", "util_b", "r1", "r2", "r3", "r4", "r5", "r6", "r7",
        "r8",
    ] {
        alive.insert(hash_of(nodes, path));
    }
    alive
}

// ---------------------------------------------------------------------------
// Test 1: Dead export within alive module is reported
// ---------------------------------------------------------------------------

/// `util_a` is alive and has two Named exports: `a` (called by entry.main via
/// call edge) and `a_dead` (never called).  The result must contain `a_dead`
/// in `util_a`'s dead set but NOT `a`.
#[test]
fn dead_export_within_alive_module_reported() {
    let nodes = load_fixture();
    let alive = alive_set(&nodes);
    let dead = compute_dead_exports(&nodes, &alive);

    let util_a_hash = hash_of(&nodes, "util_a");
    let dead_exports = dead
        .get(&util_a_hash)
        .expect("util_a should appear as a key in result (it is alive)");

    assert!(
        dead_exports.contains("a_dead"),
        "expected 'a_dead' to be dead in util_a, got: {dead_exports:?}"
    );
    assert!(
        !dead_exports.contains("a"),
        "expected 'a' NOT to be dead in util_a (it is called by entry.main)"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Unreached modules are absent from the result map
// ---------------------------------------------------------------------------

/// Modules d1..d8 are not in the alive set.  They must not appear as keys in
/// the result — `compute_dead_exports` only reports data for alive modules.
#[test]
fn unreached_modules_absent_from_result() {
    let nodes = load_fixture();
    let alive = alive_set(&nodes);
    let dead = compute_dead_exports(&nodes, &alive);

    for path in ["d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8"] {
        let h = hash_of(&nodes, path);
        assert!(
            !dead.contains_key(&h),
            "expected {path:?} NOT to appear as a key in result (module is not alive)"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: Transitive call keeps target export alive
// ---------------------------------------------------------------------------

/// Call chain: entry.main → util_a::a → util_a_dep::live.
/// `util_a_dep` is alive.  Its export `live` must NOT be in the dead set
/// because it is transitively reachable.  Its export `dead` receives no calls
/// and must be in the dead set.
#[test]
fn transitive_call_keeps_target_export_alive() {
    let nodes = load_fixture();
    let alive = alive_set(&nodes);
    let dead = compute_dead_exports(&nodes, &alive);

    let util_a_dep_hash = hash_of(&nodes, "util_a_dep");
    let dead_exports = dead
        .get(&util_a_dep_hash)
        .expect("util_a_dep should appear as a key in result (it is alive)");

    assert!(
        !dead_exports.contains("live"),
        "expected 'live' NOT to be dead in util_a_dep (transitively reachable via entry→util_a→util_a_dep)"
    );
    assert!(
        dead_exports.contains("dead"),
        "expected 'dead' to be dead in util_a_dep (no call edges reach it), got: {dead_exports:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Default exports with no inbound calls are still alive
// ---------------------------------------------------------------------------

/// `entry` has a single Default export named `main`.  Although no other module
/// has a call edge that targets `entry::main`, Default exports are seeded as
/// public roots and are never reported as dead.
#[test]
fn default_exports_with_no_inbound_calls_still_alive() {
    let nodes = load_fixture();
    let alive = alive_set(&nodes);
    let dead = compute_dead_exports(&nodes, &alive);

    let entry_hash = hash_of(&nodes, "entry");
    let dead_exports = dead
        .get(&entry_hash)
        .expect("entry should appear as a key in result (it is alive)");

    assert!(
        !dead_exports.contains("main"),
        "expected 'main' (Default export) NOT to be dead in entry — Default exports are always alive"
    );
}
