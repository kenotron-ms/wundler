//! Integration tests for `assign_chunks` — LAZY chunk creation at dynamic-import boundaries.

use std::collections::{HashMap, HashSet};

use cloudpack_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary, SideEffectMarker,
};
use cloudpack_graph::chunks::assign_chunks;
use cloudpack_graph::types::LoadCondition;

// ---------------------------------------------------------------------------
// Helpers (copied from chunks_initial.rs for self-contained tests)
// ---------------------------------------------------------------------------

fn make_node(path: &str, imports: Vec<Import>) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::from_source(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports,
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: None,
        source: None,
    }
}

fn static_import(specifier: &str) -> Import {
    Import {
        specifier: specifier.to_string(),
        kind: ImportKind::Named,
        bindings: vec![],
        is_dynamic: false,
    }
}

fn dynamic_import(specifier: &str) -> Import {
    Import {
        specifier: specifier.to_string(),
        kind: ImportKind::Dynamic,
        bindings: vec![],
        is_dynamic: true,
    }
}

// ---------------------------------------------------------------------------
// Test 1: dynamic import creates lazy chunk
//
// entry.js --static--> a.js
// entry.js --dynamic--> lazy_root.js
// lazy_root.js --static--> lazy_dep.js
//
// Expected: initial_main chunk = {entry.js, a.js}
//           lazy_1 chunk = {lazy_root.js, lazy_dep.js}  (LoadCondition::Lazy)
// ---------------------------------------------------------------------------

#[test]
fn dynamic_import_creates_lazy_chunk() {
    let entry = make_node(
        "entry.js",
        vec![static_import("a.js"), dynamic_import("lazy_root.js")],
    );
    let a = make_node("a.js", vec![]);
    let lazy_root = make_node("lazy_root.js", vec![static_import("lazy_dep.js")]);
    let lazy_dep = make_node("lazy_dep.js", vec![]);

    let nodes = vec![
        entry.clone(),
        a.clone(),
        lazy_root.clone(),
        lazy_dep.clone(),
    ];

    let mut alive = HashSet::new();
    alive.insert(entry.id.clone());
    alive.insert(a.id.clone());
    alive.insert(lazy_root.id.clone());
    alive.insert(lazy_dep.id.clone());

    let mut entry_hashes = HashMap::new();
    entry_hashes.insert("main".to_string(), entry.id.clone());

    let (chunks, module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 2);

    // Expect 2 chunks: one initial, one lazy.
    assert_eq!(
        chunks.len(),
        2,
        "expected 2 chunks (1 initial + 1 lazy), got {:?}",
        chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
    );

    // Find the initial chunk.
    let initial = chunks
        .iter()
        .find(|c| c.load_condition == LoadCondition::Initial)
        .expect("expected an INITIAL chunk");
    assert_eq!(initial.id, "initial_main");
    assert_eq!(
        initial.modules.len(),
        2,
        "initial chunk must contain entry.js and a.js"
    );
    assert!(
        initial.modules.contains(&entry.id),
        "entry.js must be in initial"
    );
    assert!(initial.modules.contains(&a.id), "a.js must be in initial");

    // Find the lazy chunk.
    let lazy_chunk = chunks
        .iter()
        .find(|c| c.load_condition == LoadCondition::Lazy)
        .expect("expected a LAZY chunk");
    assert_eq!(lazy_chunk.id, "lazy_1", "first lazy chunk must be 'lazy_1'");
    assert_eq!(
        lazy_chunk.modules.len(),
        2,
        "lazy chunk must contain lazy_root.js and lazy_dep.js, got {:?}",
        lazy_chunk.modules
    );
    assert!(
        lazy_chunk.modules.contains(&lazy_root.id),
        "lazy_root.js must be in lazy chunk"
    );
    assert!(
        lazy_chunk.modules.contains(&lazy_dep.id),
        "lazy_dep.js must be in lazy chunk"
    );

    // All modules must be in the module index.
    assert_eq!(
        module_index.get(&entry.id).map(String::as_str),
        Some("initial_main")
    );
    assert_eq!(
        module_index.get(&a.id).map(String::as_str),
        Some("initial_main")
    );
    assert_eq!(
        module_index.get(&lazy_root.id).map(String::as_str),
        Some("lazy_1")
    );
    assert_eq!(
        module_index.get(&lazy_dep.id).map(String::as_str),
        Some("lazy_1")
    );
}

// ---------------------------------------------------------------------------
// Test 2: multiple dynamic imports create distinct lazy chunks
//
// entry.js --dynamic--> lazy_a.js
// entry.js --dynamic--> lazy_b.js
//
// Expected: initial_main = {entry.js}
//           lazy_1 = {lazy_a.js}  (first in BFS order)
//           lazy_2 = {lazy_b.js}
// ---------------------------------------------------------------------------

#[test]
fn multiple_dynamic_imports_create_distinct_lazy_chunks() {
    let entry = make_node(
        "entry.js",
        vec![dynamic_import("lazy_a.js"), dynamic_import("lazy_b.js")],
    );
    let lazy_a = make_node("lazy_a.js", vec![]);
    let lazy_b = make_node("lazy_b.js", vec![]);

    let nodes = vec![entry.clone(), lazy_a.clone(), lazy_b.clone()];

    let mut alive = HashSet::new();
    alive.insert(entry.id.clone());
    alive.insert(lazy_a.id.clone());
    alive.insert(lazy_b.id.clone());

    let mut entry_hashes = HashMap::new();
    entry_hashes.insert("main".to_string(), entry.id.clone());

    let (chunks, module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 2);

    // Expect 3 chunks: 1 initial + 2 lazy.
    assert_eq!(
        chunks.len(),
        3,
        "expected 3 chunks (1 initial + 2 lazy), got {:?}",
        chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
    );

    let initial = chunks
        .iter()
        .find(|c| c.load_condition == LoadCondition::Initial)
        .expect("expected an INITIAL chunk");
    assert_eq!(initial.id, "initial_main");
    assert_eq!(initial.modules.len(), 1);
    assert!(initial.modules.contains(&entry.id));

    let lazy_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Lazy)
        .collect();
    assert_eq!(lazy_chunks.len(), 2, "expected 2 lazy chunks");

    // The two lazy chunks must have distinct IDs.
    let ids: HashSet<&str> = lazy_chunks.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains("lazy_1"), "expected lazy_1 to exist");
    assert!(ids.contains("lazy_2"), "expected lazy_2 to exist");

    // Each lazy chunk must contain exactly one module.
    for lc in &lazy_chunks {
        assert_eq!(
            lc.modules.len(),
            1,
            "lazy chunk {} must contain exactly 1 module",
            lc.id
        );
    }

    // lazy_a and lazy_b must both be in the module index.
    assert!(
        module_index.contains_key(&lazy_a.id),
        "lazy_a.js must be in module_index"
    );
    assert!(
        module_index.contains_key(&lazy_b.id),
        "lazy_b.js must be in module_index"
    );

    // The two modules must be assigned to different lazy chunks.
    let lazy_a_chunk = module_index.get(&lazy_a.id).unwrap();
    let lazy_b_chunk = module_index.get(&lazy_b.id).unwrap();
    assert_ne!(
        lazy_a_chunk, lazy_b_chunk,
        "lazy_a.js and lazy_b.js must be in different lazy chunks"
    );
}

// ---------------------------------------------------------------------------
// Test 3: nested dynamic imports yield two lazy chunks with different IDs
//
// entry.js --dynamic--> la.js
// la.js --dynamic--> lb.js
//
// Expected: initial_main = {entry.js}
//           lazy_1 = {la.js}        (la.js is the root of the first lazy BFS)
//           lazy_2 = {lb.js}        (lb.js is discovered during BFS of la.js)
// ---------------------------------------------------------------------------

#[test]
fn nested_dynamic_imports_yield_two_distinct_lazy_chunks() {
    let entry = make_node("entry.js", vec![dynamic_import("la.js")]);
    let la = make_node("la.js", vec![dynamic_import("lb.js")]);
    let lb = make_node("lb.js", vec![]);

    let nodes = vec![entry.clone(), la.clone(), lb.clone()];

    let mut alive = HashSet::new();
    alive.insert(entry.id.clone());
    alive.insert(la.id.clone());
    alive.insert(lb.id.clone());

    let mut entry_hashes = HashMap::new();
    entry_hashes.insert("main".to_string(), entry.id.clone());

    let (chunks, module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 2);

    // Expect 3 chunks: 1 initial + 2 lazy.
    assert_eq!(
        chunks.len(),
        3,
        "expected 3 chunks (1 initial + 2 lazy), got {:?}",
        chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
    );

    let initial = chunks
        .iter()
        .find(|c| c.load_condition == LoadCondition::Initial)
        .expect("expected an INITIAL chunk");
    assert_eq!(initial.id, "initial_main");
    assert_eq!(initial.modules.len(), 1);
    assert!(initial.modules.contains(&entry.id));

    let lazy_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Lazy)
        .collect();
    assert_eq!(lazy_chunks.len(), 2, "expected 2 lazy chunks");

    // The two lazy chunks must have different IDs.
    let id_a = lazy_chunks[0].id.as_str();
    let id_b = lazy_chunks[1].id.as_str();
    assert_ne!(id_a, id_b, "lazy chunks must have distinct IDs");

    let ids: HashSet<&str> = lazy_chunks.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains("lazy_1"), "expected lazy_1");
    assert!(ids.contains("lazy_2"), "expected lazy_2");

    // la.js and lb.js must both be in module_index.
    assert!(
        module_index.contains_key(&la.id),
        "la.js must be in module_index"
    );
    assert!(
        module_index.contains_key(&lb.id),
        "lb.js must be in module_index"
    );

    // la.js and lb.js must be in DIFFERENT lazy chunks.
    let la_chunk = module_index.get(&la.id).unwrap();
    let lb_chunk = module_index.get(&lb.id).unwrap();
    assert_ne!(
        la_chunk, lb_chunk,
        "la.js and lb.js must be in different lazy chunks"
    );

    // la.js must be in lazy_1, lb.js must be in lazy_2 (outer discovered first).
    assert_eq!(
        la_chunk.as_str(),
        "lazy_1",
        "la.js (outer dynamic) must be in lazy_1"
    );
    assert_eq!(
        lb_chunk.as_str(),
        "lazy_2",
        "lb.js (inner dynamic) must be in lazy_2"
    );
}
