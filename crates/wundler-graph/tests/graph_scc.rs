use std::collections::HashMap;

use wundler_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use wundler_graph::graph::tarjan_sccs;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_node(path: &str) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::from_source(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: vec![],
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: None,
        source: None,
    }
}

fn hash(path: &str) -> ContentHash {
    ContentHash::from_source(path)
}

// ---------------------------------------------------------------------------
// Test 1: empty graph has no SCCs
// ---------------------------------------------------------------------------

#[test]
fn scc_empty_graph_returns_empty() {
    let nodes: Vec<BundleGraphNode> = vec![];
    let adj: HashMap<ContentHash, Vec<ContentHash>> = HashMap::new();

    let sccs = tarjan_sccs(&nodes, &adj);

    assert!(
        sccs.is_empty(),
        "expected empty SCCs for empty graph, got {:?}",
        sccs
    );
}

// ---------------------------------------------------------------------------
// Test 2: acyclic graph yields singleton SCCs
// ---------------------------------------------------------------------------

/// A linear chain a → b → c has no cycles; each module is its own SCC.
#[test]
fn scc_acyclic_graph_yields_singleton_sccs() {
    let node_a = make_node("src/a.ts");
    let node_b = make_node("src/b.ts");
    let node_c = make_node("src/c.ts");

    let hash_a = hash("src/a.ts");
    let hash_b = hash("src/b.ts");
    let hash_c = hash("src/c.ts");

    let mut adj: HashMap<ContentHash, Vec<ContentHash>> = HashMap::new();
    adj.insert(hash_a.clone(), vec![hash_b.clone()]);
    adj.insert(hash_b.clone(), vec![hash_c.clone()]);

    let nodes = vec![node_a, node_b, node_c];
    let sccs = tarjan_sccs(&nodes, &adj);

    // Should have exactly 3 SCCs, each of size 1
    assert_eq!(
        sccs.len(),
        3,
        "expected 3 singleton SCCs for acyclic graph, got {:?}",
        sccs
    );

    for scc in &sccs {
        assert_eq!(
            scc.len(),
            1,
            "each SCC should be a singleton in an acyclic graph, got {:?}",
            scc
        );
    }

    // All hashes should appear exactly once
    let all_hashes: Vec<&ContentHash> = sccs.iter().flatten().collect();
    assert_eq!(all_hashes.len(), 3, "expected exactly 3 hashes total");
    assert!(all_hashes.contains(&&hash_a), "expected hash_a in SCCs");
    assert!(all_hashes.contains(&&hash_b), "expected hash_b in SCCs");
    assert!(all_hashes.contains(&&hash_c), "expected hash_c in SCCs");
}

// ---------------------------------------------------------------------------
// Test 3: two-module cycle yields one SCC of size 2
// ---------------------------------------------------------------------------

/// a ↔ b (a imports b, b imports a) forms a cycle; both belong to the same SCC.
#[test]
fn scc_two_module_cycle_yields_one_scc_of_size_2() {
    let node_a = make_node("src/a.ts");
    let node_b = make_node("src/b.ts");

    let hash_a = hash("src/a.ts");
    let hash_b = hash("src/b.ts");

    let mut adj: HashMap<ContentHash, Vec<ContentHash>> = HashMap::new();
    adj.insert(hash_a.clone(), vec![hash_b.clone()]);
    adj.insert(hash_b.clone(), vec![hash_a.clone()]);

    let nodes = vec![node_a, node_b];
    let sccs = tarjan_sccs(&nodes, &adj);

    // Should have exactly 1 SCC of size 2
    assert_eq!(
        sccs.len(),
        1,
        "expected 1 SCC for a two-module cycle, got {:?}",
        sccs
    );
    assert_eq!(
        sccs[0].len(),
        2,
        "expected the SCC to contain both nodes, got {:?}",
        sccs[0]
    );
    assert!(sccs[0].contains(&hash_a), "expected hash_a in the SCC");
    assert!(sccs[0].contains(&hash_b), "expected hash_b in the SCC");
}

// ---------------------------------------------------------------------------
// Test 4: three-module cycle with external dependency
// ---------------------------------------------------------------------------

/// d → a → b → c → a  (a, b, c form a cycle; d is external, pointing into a).
/// Expected: one SCC of size 3 for {a, b, c}, one singleton SCC for {d}.
#[test]
fn scc_three_module_cycle_with_external_dependency() {
    let node_a = make_node("src/a.ts");
    let node_b = make_node("src/b.ts");
    let node_c = make_node("src/c.ts");
    let node_d = make_node("src/d.ts");

    let hash_a = hash("src/a.ts");
    let hash_b = hash("src/b.ts");
    let hash_c = hash("src/c.ts");
    let hash_d = hash("src/d.ts");

    let mut adj: HashMap<ContentHash, Vec<ContentHash>> = HashMap::new();
    // Cycle: a → b → c → a
    adj.insert(hash_a.clone(), vec![hash_b.clone()]);
    adj.insert(hash_b.clone(), vec![hash_c.clone()]);
    adj.insert(hash_c.clone(), vec![hash_a.clone()]);
    // External: d → a
    adj.insert(hash_d.clone(), vec![hash_a.clone()]);

    let nodes = vec![node_a, node_b, node_c, node_d];
    let sccs = tarjan_sccs(&nodes, &adj);

    // Should have 2 SCCs: one of size 3, one of size 1
    assert_eq!(
        sccs.len(),
        2,
        "expected 2 SCCs (one cycle + one singleton), got {:?}",
        sccs
    );

    let scc_sizes: Vec<usize> = sccs.iter().map(|s| s.len()).collect();
    assert!(
        scc_sizes.contains(&3),
        "expected one SCC of size 3 (the a→b→c→a cycle), got sizes {:?}",
        scc_sizes
    );
    assert!(
        scc_sizes.contains(&1),
        "expected one singleton SCC (d), got sizes {:?}",
        scc_sizes
    );

    // Find the cycle SCC and singleton SCC
    let cycle_scc = sccs.iter().find(|s| s.len() == 3).unwrap();
    let singleton_scc = sccs.iter().find(|s| s.len() == 1).unwrap();

    assert!(cycle_scc.contains(&hash_a), "expected hash_a in cycle SCC");
    assert!(cycle_scc.contains(&hash_b), "expected hash_b in cycle SCC");
    assert!(cycle_scc.contains(&hash_c), "expected hash_c in cycle SCC");
    assert!(
        singleton_scc.contains(&hash_d),
        "expected hash_d in singleton SCC"
    );
}
