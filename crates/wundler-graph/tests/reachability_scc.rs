//! Integration tests for `compute_reachability_with_sccs`.
//!
//! All tests are built from small programmatic node graphs so they don't
//! depend on fixture JSON files.  Each test targets one of the four
//! spec-required behaviours:
//!
//! 1. A cycle member that is reached pulls its entire SCC alive.
//! 2. An unreachable cycle is entirely dead.
//! 3. A cycle node used as an entry point marks the whole SCC alive.
//! 4. On an acyclic graph SCC-aware BFS produces the same result as plain BFS.

use std::collections::HashSet;

use wundler_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary, SideEffectMarker,
};
use wundler_graph::reachability::{compute_reachability, compute_reachability_with_sccs};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `BundleGraphNode` whose id is deterministically derived from
/// `path` (via `ContentHash::from_source`) and whose imports point to other
/// paths by their specifier string.
fn make_node(path: &str, imports: &[&str]) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::from_source(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: imports
                .iter()
                .map(|s| Import {
                    specifier: s.to_string(),
                    kind: ImportKind::Named,
                    bindings: vec![],
                    is_dynamic: false,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: None,
        source: None,
    }
}

/// Compute the deterministic `ContentHash` for a path string.
fn h(path: &str) -> ContentHash {
    ContentHash::from_source(path)
}

/// Convenience: produce a one-element entry `HashSet` for a single path.
fn entry(path: &str) -> HashSet<ContentHash> {
    [h(path)].into_iter().collect()
}

// ---------------------------------------------------------------------------
// Test 1: Cycle member pulls entire SCC alive
// ---------------------------------------------------------------------------

/// Graph topology:
///   entry → a
///   a → b
///   b → a   (a and b form a 2-cycle)
///
/// Entry: {entry}
///
/// Plain BFS would find `entry` → `a` → `b` → `a` (revisit, stop) — all three
/// are alive anyway.  The SCC-aware variant must also mark *both* `a` and `b`
/// alive the first time it reaches either one, before following any outgoing
/// edges from them.
#[test]
fn cycle_member_pulls_entire_scc_alive() {
    let nodes = vec![
        make_node("entry", &["a"]),
        make_node("a", &["b"]),
        make_node("b", &["a"]),
    ];
    let entry_set = entry("entry");

    let alive = compute_reachability_with_sccs(&nodes, &entry_set);

    assert!(alive.contains(&h("entry")), "'entry' must be alive");
    assert!(
        alive.contains(&h("a")),
        "'a' must be alive (SCC member reached)"
    );
    assert!(
        alive.contains(&h("b")),
        "'b' must be alive (co-SCC member with 'a')"
    );
    assert_eq!(
        alive.len(),
        3,
        "expected exactly 3 alive nodes, got {}",
        alive.len()
    );
}

// ---------------------------------------------------------------------------
// Test 2: Unreachable cycle is entirely dead
// ---------------------------------------------------------------------------

/// Graph topology:
///   entry   (no imports — isolated leaf)
///   cycle_a → cycle_b
///   cycle_b → cycle_a   (unreachable 2-cycle)
///
/// Entry: {entry}
///
/// Neither `cycle_a` nor `cycle_b` is reachable from `entry`, so both must
/// remain dead regardless of the SCC-aware bookkeeping.
#[test]
fn unreachable_cycle_is_entirely_dead() {
    let nodes = vec![
        make_node("entry", &[]),
        make_node("cycle_a", &["cycle_b"]),
        make_node("cycle_b", &["cycle_a"]),
    ];
    let entry_set = entry("entry");

    let alive = compute_reachability_with_sccs(&nodes, &entry_set);

    assert!(alive.contains(&h("entry")), "'entry' must be alive");
    assert!(
        !alive.contains(&h("cycle_a")),
        "'cycle_a' must be dead (unreachable)"
    );
    assert!(
        !alive.contains(&h("cycle_b")),
        "'cycle_b' must be dead (unreachable)"
    );
    assert_eq!(
        alive.len(),
        1,
        "expected exactly 1 alive node, got {}",
        alive.len()
    );
}

// ---------------------------------------------------------------------------
// Test 3: Cycle node in entry set marks whole cycle alive
// ---------------------------------------------------------------------------

/// Graph topology:
///   a → b
///   b → a   (a and b form a 2-cycle)
///   c       (independent leaf, no edges)
///
/// Entry: {a}   — `a` itself is inside the cycle.
///
/// When `a` is seeded as an entry, the entire SCC {a, b} must be activated
/// immediately.  `c` is not in the SCC and is not transitively reachable, so
/// it must remain dead.
#[test]
fn cycle_node_in_entry_marks_whole_cycle_alive() {
    let nodes = vec![
        make_node("a", &["b"]),
        make_node("b", &["a"]),
        make_node("c", &[]),
    ];
    let entry_set = entry("a");

    let alive = compute_reachability_with_sccs(&nodes, &entry_set);

    assert!(
        alive.contains(&h("a")),
        "'a' must be alive (entry + SCC member)"
    );
    assert!(
        alive.contains(&h("b")),
        "'b' must be alive (co-SCC member activated with 'a')"
    );
    assert!(
        !alive.contains(&h("c")),
        "'c' must be dead (not reachable from 'a')"
    );
    assert_eq!(
        alive.len(),
        2,
        "expected exactly 2 alive nodes, got {}",
        alive.len()
    );
}

// ---------------------------------------------------------------------------
// Test 4: SCC-aware matches plain BFS on acyclic graph
// ---------------------------------------------------------------------------

/// Graph topology:
///   x → y → z   (strictly acyclic linear chain)
///
/// Entry: {x}
///
/// On a graph with no cycles every SCC is a singleton, so `compute_reachability_with_sccs`
/// must produce the identical alive set as `compute_reachability`.
#[test]
fn scc_aware_matches_plain_bfs_on_acyclic_graph() {
    let nodes = vec![
        make_node("x", &["y"]),
        make_node("y", &["z"]),
        make_node("z", &[]),
    ];
    let entry_set = entry("x");

    let plain = compute_reachability(&nodes, &entry_set);
    let scc_aware = compute_reachability_with_sccs(&nodes, &entry_set);

    assert_eq!(
        plain, scc_aware,
        "SCC-aware result must equal plain BFS on an acyclic graph"
    );
    assert!(scc_aware.contains(&h("x")), "'x' must be alive");
    assert!(scc_aware.contains(&h("y")), "'y' must be alive");
    assert!(scc_aware.contains(&h("z")), "'z' must be alive");
    assert_eq!(scc_aware.len(), 3, "expected exactly 3 alive nodes");
}
