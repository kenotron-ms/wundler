//! Integration tests for `SwcTransformAdapter` source map generation.

use wundler_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use wundler_graph::types::{Chunk, LoadCondition};
use wundler_transform::engine::{TransformDecisions, TransformEngine};
use wundler_transform::swc_adapter::{SwcAdapterConfig, SwcTransformAdapter};

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

/// When `source_maps: true`, a v3 source map is emitted and contains the
/// source file name in the `sources` array.
#[test]
fn source_map_emitted_when_enabled() {
    let adapter = SwcTransformAdapter::with_config(SwcAdapterConfig { source_maps: true });
    let modules = vec![node("foo.ts", "const x = 1;")];
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let out = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        out.source_map.is_some(),
        "source_map should be Some when source_maps is enabled"
    );

    let sm_str = out.source_map.unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&sm_str).expect("source map should be valid JSON");

    assert_eq!(parsed["version"], 3, "source map version should be 3");

    let sources = parsed["sources"]
        .as_array()
        .expect("sources should be a JSON array");
    let has_foo_ts = sources.iter().any(|s| s.as_str() == Some("foo.ts"));
    assert!(
        has_foo_ts,
        "sources array should contain 'foo.ts', got: {:?}",
        sources
    );
}

/// When source maps are disabled (the default), `source_map` is `None`.
#[test]
fn source_map_absent_when_disabled() {
    // `SwcTransformAdapter::new()` uses default config — source_maps: false.
    let adapter = SwcTransformAdapter::new();
    let modules = vec![node("bar.ts", "const y = 2;")];
    let chunk = make_chunk("chunk-0");
    let decisions = TransformDecisions::default();

    let out = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        out.source_map.is_none(),
        "source_map should be None when source_maps is disabled"
    );
}

/// A chunk with multiple modules should have all module paths listed in the
/// `sources` array of the emitted source map.
#[test]
fn source_map_includes_all_modules_in_chunk() {
    let adapter = SwcTransformAdapter::with_config(SwcAdapterConfig { source_maps: true });
    let modules = vec![
        node("alpha.ts", "const a = 1;"),
        node("beta.ts", "const b = 2;"),
        node("gamma.ts", "const c = 3;"),
    ];
    let chunk = make_chunk("chunk-multi");
    let decisions = TransformDecisions::default();

    let out = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert!(
        out.source_map.is_some(),
        "source_map should be Some when source_maps is enabled"
    );

    let sm_str = out.source_map.unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&sm_str).expect("source map should be valid JSON");

    let sources = parsed["sources"]
        .as_array()
        .expect("sources should be a JSON array");
    let source_strs: Vec<&str> = sources.iter().filter_map(|s| s.as_str()).collect();

    for expected in &["alpha.ts", "beta.ts", "gamma.ts"] {
        assert!(
            source_strs.contains(expected),
            "sources should contain '{}', got: {:?}",
            expected,
            source_strs
        );
    }
}
