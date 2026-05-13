//! Integration tests for `GraphAnalyzer::analyze()`.
//!
//! Acceptance criteria: `cargo test -p wundler-graph --test analyze_integration`
//! reports `test result: ok. 3 passed; 0 failed`.

use std::collections::HashMap;
use std::path::PathBuf;

use wundler_core::types::BundleGraphNode;
use wundler_graph::analyzer::GraphAnalyzer;
use wundler_graph::types::LoadCondition;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_nodes() -> Vec<BundleGraphNode> {
    let json = include_str!("fixtures/multi_entry.json");
    serde_json::from_str(json).expect("fixture must parse as Vec<BundleGraphNode>")
}

// ---------------------------------------------------------------------------
// Test 1: Three-entry analysis (full pipeline)
//
// Entries: /a → path "a", /b → path "b", /c → path "c"
// Expected:
//   - stats.total = 15, stats.alive = 13, stats.dead = 2 (cir1, cir2 unreachable)
//   - commons chunk exists, contains hash-shared, LoadCondition::Initial
//   - exactly 3 initial route chunks + 3 lazy chunks
//   - every route's entry_chunks first entry is "commons"
//   - every alive node has an entry in manifest.module_index
//   - build_id is 64 lowercase hex chars
//   - stats.chunks == manifest.chunks.len()
// ---------------------------------------------------------------------------

#[test]
fn analyze_three_entries() {
    let nodes = load_nodes();

    let entry_points: HashMap<String, PathBuf> = [
        ("/a".to_string(), PathBuf::from("a")),
        ("/b".to_string(), PathBuf::from("b")),
        ("/c".to_string(), PathBuf::from("c")),
    ]
    .into_iter()
    .collect();

    let analyzer = GraphAnalyzer::new(entry_points);
    let result = analyzer.analyze(nodes).expect("analyze must succeed for valid entries");

    // ── stats ────────────────────────────────────────────────────────────────
    assert_eq!(result.stats.total, 15, "total must be 15 (all nodes in fixture)");
    assert_eq!(result.stats.alive, 13, "alive must be 13 (cir1+cir2 are dead)");
    assert_eq!(result.stats.dead, 2, "dead must be 2 (cir1, cir2)");
    assert_eq!(
        result.stats.chunks,
        result.manifest.chunks.len(),
        "stats.chunks must equal manifest.chunks.len()"
    );

    // ── build_id ─────────────────────────────────────────────────────────────
    assert_eq!(
        result.manifest.build_id.len(),
        64,
        "build_id must be 64 hex chars; got {:?}",
        result.manifest.build_id
    );
    assert!(
        result.manifest.build_id.chars().all(|c| c.is_ascii_hexdigit()),
        "build_id must be lowercase hex; got {:?}",
        result.manifest.build_id
    );

    // ── commons chunk ────────────────────────────────────────────────────────
    let commons = result
        .manifest
        .chunks
        .iter()
        .find(|c| c.id == "commons")
        .expect("commons chunk must exist");

    assert_eq!(
        commons.load_condition,
        LoadCondition::Initial,
        "commons chunk must have LoadCondition::Initial"
    );
    assert!(
        commons.modules.iter().any(|m| m.0 == "hash-shared"),
        "commons must contain hash-shared; commons.modules = {:?}",
        commons.modules
    );

    // ── chunk counts ─────────────────────────────────────────────────────────
    let initial_count = result
        .manifest
        .chunks
        .iter()
        .filter(|c| c.id.starts_with("initial_") && c.load_condition == LoadCondition::Initial)
        .count();
    let lazy_count = result
        .manifest
        .chunks
        .iter()
        .filter(|c| c.id.starts_with("lazy_") && c.load_condition == LoadCondition::Lazy)
        .count();

    assert_eq!(
        initial_count, 3,
        "expected 3 initial route chunks, got {initial_count}; chunks = {:?}",
        result.manifest.chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
    );
    assert_eq!(
        lazy_count, 3,
        "expected 3 lazy chunks, got {lazy_count}; chunks = {:?}",
        result.manifest.chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
    );

    // ── entry_chunks ordering: "commons" first for every route ────────────────
    for (route, chunk_ids) in &result.manifest.entry_chunks {
        assert!(
            !chunk_ids.is_empty(),
            "entry_chunks for route {route:?} must not be empty"
        );
        assert_eq!(
            chunk_ids[0], "commons",
            "first chunk for route {route:?} must be 'commons', got {chunk_ids:?}"
        );
    }

    // ── every alive node appears in module_index ──────────────────────────────
    for node in &result.nodes {
        if node.alive {
            assert!(
                result.manifest.module_index.contains_key(&node.id),
                "alive node id={:?} path={:?} must be in module_index",
                node.id,
                node.path
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test 2: Single-entry analysis (/a only)
//
// With only route /a, only the modules transitively reachable from "a" are
// alive.  Module "b" and "c" (and their private sub-graphs) must be dead.
// ---------------------------------------------------------------------------

#[test]
fn analyze_single_entry_a() {
    let nodes = load_nodes();

    let entry_points: HashMap<String, PathBuf> =
        [("/a".to_string(), PathBuf::from("a"))].into_iter().collect();

    let analyzer = GraphAnalyzer::new(entry_points);
    let result = analyzer
        .analyze(nodes)
        .expect("analyze must succeed for single entry");

    let alive_paths: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.alive)
        .map(|n| n.path.as_str())
        .collect();

    // Modules reachable from "a" must be alive.
    for expected in &["a", "x1", "x2", "shared", "lazy_a1"] {
        assert!(
            alive_paths.contains(expected),
            "expected path {expected:?} to be alive; alive_paths = {alive_paths:?}"
        );
    }

    // Modules NOT reachable from "a" must be dead.
    assert!(
        !alive_paths.contains(&"b"),
        "path 'b' must NOT be alive when only entry /a is given"
    );
    assert!(
        !alive_paths.contains(&"c"),
        "path 'c' must NOT be alive when only entry /a is given"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Unknown path entry returns Err
//
// If the GraphAnalyzer's entry_points maps a route to a path that does not
// exist in the provided node graph, analyze() must return Err.
// ---------------------------------------------------------------------------

#[test]
fn analyze_unknown_path_returns_err() {
    let nodes = load_nodes();

    let entry_points: HashMap<String, PathBuf> = [(
        "/unknown".to_string(),
        PathBuf::from("no_such_module"),
    )]
    .into_iter()
    .collect();

    let analyzer = GraphAnalyzer::new(entry_points);
    let result = analyzer.analyze(nodes);

    assert!(
        result.is_err(),
        "expected Err when entry path is not in graph, got Ok"
    );
}
