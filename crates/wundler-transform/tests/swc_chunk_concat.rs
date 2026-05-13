use std::collections::{HashMap, HashSet};

use wundler_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use wundler_graph::types::{Chunk, LoadCondition};
use wundler_transform::engine::{TransformDecisions, TransformEngine};
use wundler_transform::swc_adapter::SwcTransformAdapter;

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

/// Each module in the chunk gets a `// module: <path>` comment header.
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
        output.code.contains("// module: a.ts"),
        "expected '// module: a.ts' in output, got:\n{}",
        output.code
    );
    assert!(
        output.code.contains("// module: b.ts"),
        "expected '// module: b.ts' in output, got:\n{}",
        output.code
    );
}

/// Each module must be wrapped in an IIFE for scope isolation.
#[test]
fn scope_isolation_via_iife_wrapping() {
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

    let iife_count = output.code.matches("(function()").count();
    assert_eq!(
        iife_count, 2,
        "expected 2 IIFE wrappers (one per module), found {} in:\n{}",
        iife_count, output.code
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
