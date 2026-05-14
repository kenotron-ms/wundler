//! Tests that intra-chunk `import` statements are stripped when assembling a
//! chunk, while external imports (bare specifiers like `'react'`) are kept.

use wundler_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use wundler_graph::types::{Chunk, LoadCondition};
use wundler_transform::engine::{TransformDecisions, TransformEngine};
use wundler_transform::swc_adapter::SwcTransformAdapter;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node(path: &str, src: &str) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::from_source(src),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: vec![],
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: Some("chunk-0".to_string()),
        source: Some(src.to_string()),
    }
}

fn make_chunk(id: &str) -> Chunk {
    Chunk {
        id: id.to_string(),
        modules: vec![],
        hash: ContentHash::from_bytes(b"chunk-hash"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When module A imports module B and actually USES the binding, and both are
/// in the same chunk, the `import B from './B'` line must be stripped from A's
/// output (since B's code is already concatenated into the chunk).
#[test]
fn intra_chunk_import_is_stripped() {
    // App IS used as a value — SWC will keep this import through transpile.
    let entry_src = "import App from './App';\nconst el = App();";
    let entry_node = node("src/index.ts", entry_src);
    let app_src = "export default function App() { return 42; }";
    let app_node = node("src/App.ts", app_src);

    let adapter = SwcTransformAdapter::new();
    let modules = vec![entry_node, app_node];
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let output = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        !output.code.contains("import App from"),
        "intra-chunk import './App' must be stripped, got:\n{}",
        output.code
    );
}

/// External imports (bare specifiers like `'react-dom/client'`) must be kept
/// at the top of the chunk even when other intra-chunk imports are stripped.
#[test]
fn external_import_is_kept() {
    // Both createRoot and App are used so SWC doesn't elide them.
    let entry_src =
        "import { createRoot } from 'react-dom/client';\nimport App from './App';\ncreateRoot(document.body).render(App);";
    let entry_node = node("src/index.ts", entry_src);
    let app_src = "export default function App() { return null; }";
    let app_node = node("src/App.ts", app_src);

    let adapter = SwcTransformAdapter::new();
    let modules = vec![entry_node, app_node];
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let output = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        output.code.contains("react-dom/client"),
        "external import 'react-dom/client' must be kept, got:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("import App from"),
        "intra-chunk import './App' must be stripped, got:\n{}",
        output.code
    );
}

/// A side-effect-only import (`import './analytics'`) for a module that IS
/// in the same chunk must be stripped (the code is already concatenated).
#[test]
fn intra_chunk_side_effect_import_is_stripped() {
    let entry_src = "import './utils/analytics';\nconst x = 1;";
    let entry_node = node("src/index.ts", entry_src);
    let analytics_src = "console.log('analytics loaded');";
    let analytics_node = node("src/utils/analytics.ts", analytics_src);

    let adapter = SwcTransformAdapter::new();
    let modules = vec![entry_node, analytics_node];
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let output = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        !output.code.contains("import './utils/analytics'"),
        "intra-chunk side-effect import must be stripped, got:\n{}",
        output.code
    );
    // The analytics code itself must still appear in the chunk body.
    assert!(
        output.code.contains("analytics loaded"),
        "analytics module body must still appear in chunk, got:\n{}",
        output.code
    );
}

/// An import that resolves to a module in a DIFFERENT chunk (cross-chunk)
/// must be kept — the browser needs to fetch that chunk separately.
#[test]
fn cross_chunk_import_is_kept() {
    // Dashboard IS used as a value so SWC won't elide the import.
    // Dashboard is NOT in this chunk → the import must survive into the output.
    let entry_src =
        "import Dashboard from './pages/Dashboard';\nconst x = Dashboard();";
    let entry_node = node("src/index.ts", entry_src);

    let adapter = SwcTransformAdapter::new();
    let modules = vec![entry_node]; // Dashboard is NOT in this chunk
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let output = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        output.code.contains("Dashboard"),
        "cross-chunk import './pages/Dashboard' must be kept, got:\n{}",
        output.code
    );
}
