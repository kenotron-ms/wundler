//! Integration tests for `assign_chunks` — INITIAL chunk assignment via static-import BFS.

use std::collections::{HashMap, HashSet};

use cloudpack_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary, SideEffectMarker,
};
use cloudpack_graph::chunks::assign_chunks;
use cloudpack_graph::types::LoadCondition;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal `BundleGraphNode` with the given path and import list.
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

/// A static (non-dynamic) import using `ImportKind::Named`.
fn static_import(specifier: &str) -> Import {
    Import {
        specifier: specifier.to_string(),
        kind: ImportKind::Named,
        bindings: vec![],
        is_dynamic: false,
    }
}

/// A dynamic `import()` call.
fn dynamic_import(specifier: &str) -> Import {
    Import {
        specifier: specifier.to_string(),
        kind: ImportKind::Dynamic,
        bindings: vec![],
        is_dynamic: true,
    }
}

// ---------------------------------------------------------------------------
// Test 1: single entry → one INITIAL chunk containing all static descendants
// ---------------------------------------------------------------------------

/// entry.js --static--> util.js --static--> helper.js
///
/// All three modules are alive.  `assign_chunks` must produce exactly one chunk
/// named `"initial_main"` with `LoadCondition::Initial`, containing all three
/// modules.  The module-index must map every module hash to `"initial_main"`.
#[test]
fn single_entry_one_initial_chunk_with_static_descendants() {
    let entry = make_node("entry.js", vec![static_import("util.js")]);
    let util = make_node("util.js", vec![static_import("helper.js")]);
    let helper = make_node("helper.js", vec![]);
    let nodes = vec![entry.clone(), util.clone(), helper.clone()];

    let mut alive = HashSet::new();
    alive.insert(entry.id.clone());
    alive.insert(util.id.clone());
    alive.insert(helper.id.clone());

    let mut entry_hashes = HashMap::new();
    entry_hashes.insert("main".to_string(), entry.id.clone());

    let (chunks, module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 2);

    // Exactly one chunk produced.
    assert_eq!(chunks.len(), 1, "expected exactly 1 chunk");

    let chunk = &chunks[0];
    assert_eq!(chunk.id, "initial_main", "chunk id must be 'initial_main'");
    assert_eq!(
        chunk.load_condition,
        LoadCondition::Initial,
        "chunk must be INITIAL"
    );

    // The chunk must contain all three modules.
    assert_eq!(
        chunk.modules.len(),
        3,
        "expected 3 modules in chunk, got {:?}",
        chunk.modules
    );
    assert!(
        chunk.modules.contains(&entry.id),
        "entry.js must be in chunk"
    );
    assert!(chunk.modules.contains(&util.id), "util.js must be in chunk");
    assert!(
        chunk.modules.contains(&helper.id),
        "helper.js must be in chunk"
    );

    // The module index must map all three to "initial_main".
    assert_eq!(
        module_index.get(&entry.id).map(String::as_str),
        Some("initial_main"),
        "entry.js module index"
    );
    assert_eq!(
        module_index.get(&util.id).map(String::as_str),
        Some("initial_main"),
        "util.js module index"
    );
    assert_eq!(
        module_index.get(&helper.id).map(String::as_str),
        Some("initial_main"),
        "helper.js module index"
    );
}

// ---------------------------------------------------------------------------
// Test 2: entry with no static imports → singleton chunk
// ---------------------------------------------------------------------------

/// An entry module with no imports at all must produce a singleton chunk
/// containing only itself.
#[test]
fn entry_with_no_imports_produces_singleton_chunk() {
    let entry = make_node("entry.js", vec![]);
    let nodes = vec![entry.clone()];

    let mut alive = HashSet::new();
    alive.insert(entry.id.clone());

    let mut entry_hashes = HashMap::new();
    entry_hashes.insert("main".to_string(), entry.id.clone());

    let (chunks, module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 2);

    assert_eq!(chunks.len(), 1, "expected exactly 1 chunk");
    let chunk = &chunks[0];
    assert_eq!(chunk.id, "initial_main");
    assert_eq!(chunk.load_condition, LoadCondition::Initial);
    assert_eq!(
        chunk.modules.len(),
        1,
        "expected singleton chunk, got {:?}",
        chunk.modules
    );
    assert!(chunk.modules.contains(&entry.id));
    assert_eq!(
        module_index.get(&entry.id).map(String::as_str),
        Some("initial_main")
    );
}

// ---------------------------------------------------------------------------
// Test 3: dynamic imports do NOT pull modules into the INITIAL chunk
//
// entry.js --dynamic--> dynamic.js
//
// The dynamic module is alive.  `assign_chunks` must NOT include it in the
// INITIAL chunk — only `entry.js` itself belongs there.  The dynamic module
// gets its own LAZY chunk instead.
// ---------------------------------------------------------------------------

/// entry.js --dynamic--> dynamic.js
///
/// The dynamic module is alive, but `assign_chunks` must NOT include it in the
/// INITIAL chunk — only `entry.js` itself belongs there.  A separate lazy
/// chunk is created for `dynamic.js`.
#[test]
fn dynamic_imports_excluded_from_initial_chunk() {
    let entry = make_node("entry.js", vec![dynamic_import("dynamic.js")]);
    let dyn_mod = make_node("dynamic.js", vec![]);
    let nodes = vec![entry.clone(), dyn_mod.clone()];

    let mut alive = HashSet::new();
    alive.insert(entry.id.clone());
    alive.insert(dyn_mod.id.clone());

    let mut entry_hashes = HashMap::new();
    entry_hashes.insert("main".to_string(), entry.id.clone());

    let (chunks, module_index) = assign_chunks(&nodes, &alive, &entry_hashes, 2);

    // There are now 2 chunks: initial_main and a lazy chunk for dynamic.js.
    assert_eq!(
        chunks.len(),
        2,
        "expected 2 chunks (1 initial + 1 lazy), got {:?}",
        chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
    );

    // Find the initial chunk and verify dynamic.js is NOT in it.
    let initial = chunks
        .iter()
        .find(|c| c.load_condition == LoadCondition::Initial)
        .expect("expected an INITIAL chunk");
    assert_eq!(initial.id, "initial_main");
    assert_eq!(
        initial.modules.len(),
        1,
        "dynamic import must NOT pull dynamic.js into the INITIAL chunk"
    );
    assert!(
        initial.modules.contains(&entry.id),
        "entry.js must be present"
    );
    assert!(
        !initial.modules.contains(&dyn_mod.id),
        "dynamic.js must NOT be in the INITIAL chunk"
    );

    // dynamic.js must be in a LAZY chunk.
    let lazy_chunk = chunks
        .iter()
        .find(|c| c.load_condition == LoadCondition::Lazy)
        .expect("expected a LAZY chunk for dynamic.js");
    assert!(
        lazy_chunk.modules.contains(&dyn_mod.id),
        "dynamic.js must appear in the lazy chunk"
    );

    // dynamic.js must now be in the module index (owned by the lazy chunk).
    assert!(
        module_index.contains_key(&dyn_mod.id),
        "dynamic.js must appear in module_index (owned by lazy chunk)"
    );
}
