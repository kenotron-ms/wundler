use cloudpack_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary, SideEffectMarker,
};
use cloudpack_graph::graph::build_adjacency;

// ---------------------------------------------------------------------------
// Helpers
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

fn make_import(specifier: &str, kind: ImportKind) -> Import {
    Import {
        specifier: specifier.to_string(),
        kind: kind.clone(),
        bindings: vec![],
        is_dynamic: matches!(kind, ImportKind::Dynamic),
    }
}

// ---------------------------------------------------------------------------
// Test 1: adjacency resolves imports by path with mixed static + dynamic
// ---------------------------------------------------------------------------

/// A node can have both static (Named/Default) and dynamic imports to different
/// targets — both should appear in its adjacency list.
#[test]
fn adjacency_resolves_imports_by_path_with_mixed_static_and_dynamic() {
    let node_b = make_node("src/b.ts", vec![]);
    let node_c = make_node("src/c.ts", vec![]);
    let node_a = make_node(
        "src/a.ts",
        vec![
            make_import("src/b.ts", ImportKind::Named),   // static
            make_import("src/c.ts", ImportKind::Dynamic), // dynamic
        ],
    );

    let hash_a = ContentHash::from_source("src/a.ts");
    let hash_b = ContentHash::from_source("src/b.ts");
    let hash_c = ContentHash::from_source("src/c.ts");

    let nodes = vec![node_a, node_b, node_c];
    let adj = build_adjacency(&nodes);

    let targets = adj.get(&hash_a).expect("expected adjacency entry for a");
    assert_eq!(targets.len(), 2, "expected 2 unique targets for a");
    assert!(targets.contains(&hash_b), "expected edge a → b");
    assert!(targets.contains(&hash_c), "expected edge a → c");
}

// ---------------------------------------------------------------------------
// Test 2: unresolved imports are silently dropped
// ---------------------------------------------------------------------------

/// Imports whose specifier does not match any node path (e.g. npm packages)
/// must not appear in the adjacency map at all — they should be silently ignored.
#[test]
fn adjacency_silently_drops_unresolved_imports() {
    let node_a = make_node(
        "src/a.ts",
        vec![make_import("react", ImportKind::Default)], // npm package — not in graph
    );

    let hash_a = ContentHash::from_source("src/a.ts");

    let adj = build_adjacency(&[node_a]);

    // a should not be in the adjacency map (no resolved targets)
    assert!(
        !adj.contains_key(&hash_a),
        "expected no adjacency entry for a when all imports are unresolved"
    );
}

// ---------------------------------------------------------------------------
// Test 3: empty input returns empty map
// ---------------------------------------------------------------------------

/// Calling build_adjacency with an empty slice must return an empty HashMap.
#[test]
fn adjacency_handles_empty_node_list() {
    let adj = build_adjacency(&[]);
    assert!(
        adj.is_empty(),
        "expected empty adjacency map for empty input"
    );
}

// ---------------------------------------------------------------------------
// Test 4: self-import is preserved
// ---------------------------------------------------------------------------

/// A node that imports its own path is pathological but legal — the resulting
/// adjacency entry should include the node's own hash.
#[test]
fn adjacency_handles_self_import() {
    let node_a = make_node(
        "src/a.ts",
        vec![make_import("src/a.ts", ImportKind::SideEffect)],
    );

    let hash_a = ContentHash::from_source("src/a.ts");

    let adj = build_adjacency(&[node_a]);

    let targets = adj
        .get(&hash_a)
        .expect("expected adjacency entry for self-importing node");
    assert_eq!(targets.len(), 1, "expected exactly one self-edge");
    assert_eq!(targets[0], hash_a, "expected self-edge a → a");
}

// ---------------------------------------------------------------------------
// Test 5: static + dynamic to same target deduplicates to one entry
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test 6: realistic paths — relative specifier must resolve to file path
// ---------------------------------------------------------------------------

/// Regression test: build_adjacency resolves relative import specifiers to
/// their actual node paths.
///
/// In real TypeScript source, `import App from './App'` appears in a file at
/// `src/main.tsx`. The adjacent node for `App.tsx` has path `src/App.tsx`.
/// The specifier `"./App"` is NOT identical to the node path `"src/App.tsx"`,
/// so the old exact-match lookup silently dropped this edge.
///
/// After the fix, `build_adjacency` must resolve the specifier to the correct
/// node path by joining it with the importer's directory and trying TypeScript
/// extensions in order.
#[test]
fn adjacency_resolves_relative_specifier_to_node_path() {
    let node_app = make_node("src/App.tsx", vec![]);
    let node_main = make_node(
        "src/main.tsx",
        vec![make_import("./App", ImportKind::Named)],
    );

    let hash_main = ContentHash::from_source("src/main.tsx");
    let hash_app = ContentHash::from_source("src/App.tsx");

    let nodes = vec![node_main, node_app];
    let adj = build_adjacency(&nodes);

    let targets = adj
        .get(&hash_main)
        .expect("expected adjacency entry for src/main.tsx");
    assert_eq!(
        targets.len(),
        1,
        "expected exactly 1 edge from main.tsx, got {:?}",
        targets
    );
    assert!(
        targets.contains(&hash_app),
        "expected edge main.tsx -> App.tsx"
    );
}

/// When a file uses `./` prefix in the WalkDir-style path (e.g. the binary
/// scans from `.` and WalkDir produces `"./src/App.tsx"`), the resolver must
/// still find the correct target.
#[test]
fn adjacency_resolves_relative_specifier_with_dot_slash_prefix() {
    // Paths as stored by WalkDir when scan root is "."
    let node_app = make_node("./src/App.tsx", vec![]);
    let node_main = make_node(
        "./src/main.tsx",
        vec![make_import("./App", ImportKind::Named)],
    );

    let hash_main = ContentHash::from_source("./src/main.tsx");
    let hash_app = ContentHash::from_source("./src/App.tsx");

    let nodes = vec![node_main, node_app];
    let adj = build_adjacency(&nodes);

    let targets = adj
        .get(&hash_main)
        .expect("expected adjacency entry for ./src/main.tsx");
    assert_eq!(
        targets.len(),
        1,
        "expected exactly 1 edge from ./src/main.tsx, got {:?}",
        targets
    );
    assert!(
        targets.contains(&hash_app),
        "expected edge ./src/main.tsx -> ./src/App.tsx"
    );
}

/// If a node imports the same target module both statically and dynamically,
/// the adjacency list for that source must contain the target exactly once.
#[test]
fn adjacency_deduplicates_static_and_dynamic_to_same_target() {
    let node_b = make_node("src/b.ts", vec![]);
    let node_a = make_node(
        "src/a.ts",
        vec![
            make_import("src/b.ts", ImportKind::Named), // static ref to b
            make_import("src/b.ts", ImportKind::Dynamic), // dynamic ref to same b
        ],
    );

    let hash_a = ContentHash::from_source("src/a.ts");
    let hash_b = ContentHash::from_source("src/b.ts");

    let nodes = vec![node_a, node_b];
    let adj = build_adjacency(&nodes);

    let targets = adj.get(&hash_a).expect("expected adjacency entry for a");
    assert_eq!(
        targets.len(),
        1,
        "expected exactly 1 deduplicated entry for a → b, got {:?}",
        targets
    );
    assert_eq!(targets[0], hash_b, "expected deduplicated edge a → b");
}
