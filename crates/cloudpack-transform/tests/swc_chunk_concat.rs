use std::collections::{HashMap, HashSet};

use cloudpack_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use cloudpack_graph::types::{Chunk, LoadCondition};
use cloudpack_transform::engine::{TransformDecisions, TransformEngine};
use cloudpack_transform::swc_adapter::SwcTransformAdapter;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal `BundleGraphNode` with the given path and source text.
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

/// Each module in the chunk gets a `// --- <path> ---` separator comment.
#[test]
fn concatenates_two_modules_in_chunk() {
    let adapter = SwcTransformAdapter::new();
    let modules = vec![
        node("a.ts", "const x = 1;"),
        node("b.ts", "const y = 2;"),
    ];
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let output = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        output.code.contains("// --- a.ts ---"),
        "expected '// --- a.ts ---' separator in output, got:\n{}",
        output.code
    );
    assert!(
        output.code.contains("// --- b.ts ---"),
        "expected '// --- b.ts ---' separator in output, got:\n{}",
        output.code
    );
}

/// Modules must NOT be wrapped in IIFEs; the chunk must use ESM shared scope
/// (flat scope-flattened concatenation) so that `import` declarations remain
/// valid at the top level.
#[test]
fn esm_scope_flattening_no_iife_wrapping() {
    let adapter = SwcTransformAdapter::new();
    let modules = vec![
        node("a.ts", "const x = 1;"),
        node("b.ts", "const y = 2;"),
    ];
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let output = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    // No IIFE wrappers — they are ESM-incompatible (import is top-level only).
    assert!(
        !output.code.contains("(function()"),
        "chunk must NOT use IIFE wrapping — found in:\n{}",
        output.code
    );
    assert!(
        !output.code.contains("})();"),
        "chunk must NOT have IIFE close `}})();` — found in:\n{}",
        output.code
    );

    // Both module bodies must appear in the shared flat scope.
    assert!(
        output.code.contains("const x"),
        "module a.ts body must appear in shared scope, got:\n{}",
        output.code
    );
    assert!(
        output.code.contains("const y"),
        "module b.ts body must appear in shared scope, got:\n{}",
        output.code
    );
}

/// Dead exports are stripped from the module source before it is wrapped and concatenated.
#[test]
fn dead_export_stripped_before_concat() {
    let adapter = SwcTransformAdapter::new();
    let src = "export function dead() {}\nexport function live() {}";
    let module = node("mod.ts", src);
    let chunk = make_chunk("chunk-0");

    let mut dead_set = HashSet::new();
    dead_set.insert("dead".to_string());
    let mut dead_exports: HashMap<ContentHash, HashSet<String>> = HashMap::new();
    dead_exports.insert(module.id.clone(), dead_set);
    let decisions = TransformDecisions { dead_exports };

    let output = adapter
        .transform_chunk(&[module], &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        !output.code.contains("function dead"),
        "dead export should be stripped, got:\n{}",
        output.code
    );
    assert!(
        output.code.contains("live"),
        "live export should be kept, got:\n{}",
        output.code
    );
}

/// An empty module list produces an empty-ish chunk output (no panics, no modules).
#[test]
fn empty_chunk_produces_empty_output() {
    let adapter = SwcTransformAdapter::new();
    let chunk = make_chunk("chunk-empty");
    let decisions = TransformDecisions::default();

    let output = adapter
        .transform_chunk(&[], &chunk, &decisions)
        .expect("transform should succeed on empty input");

    // No module markers should appear.
    assert!(
        !output.code.contains("// module:"),
        "empty chunk should have no module headers, got:\n{}",
        output.code
    );
    assert_eq!(output.chunk_id, "chunk-empty");
}

/// Identical inputs produce identical output hashes (determinism).
#[test]
fn output_hash_is_deterministic() {
    let adapter = SwcTransformAdapter::new();
    let modules = || {
        vec![
            node("a.ts", "const x = 1;"),
            node("b.ts", "const y = 2;"),
        ]
    };
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let out1 = adapter
        .transform_chunk(&modules(), &chunk, &decisions)
        .expect("first transform should succeed");
    let out2 = adapter
        .transform_chunk(&modules(), &chunk, &decisions)
        .expect("second transform should succeed");

    assert_eq!(
        out1.hash, out2.hash,
        "same input must produce same content hash"
    );
    assert_eq!(
        out1.code, out2.code,
        "same input must produce identical code"
    );
}
