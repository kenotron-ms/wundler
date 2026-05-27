//! Integration tests for `RolldownAdapter` — subprocess scaffold + SWC fallback.

use std::path::PathBuf;

use cloudpack_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use cloudpack_graph::types::{Chunk, LoadCondition};
use cloudpack_transform::engine::{TransformDecisions, TransformEngine};
use cloudpack_transform::rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};

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
        chunk_id: Some("c0".to_string()),
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

/// When Node is not found and `fallback_on_failure = true`, the adapter must
/// fall back to the SWC engine.  The SWC output always starts with
/// `// chunk: <chunk_id>`, so checking for `// chunk: c0` is sufficient.
#[test]
fn falls_back_to_swc_when_node_missing() {
    let config = RolldownAdapterConfig {
        node_path: PathBuf::from("/nonexistent/node-binary-that-cannot-exist"),
        fallback_on_failure: true,
    };
    let adapter = RolldownAdapter::with_config(config);

    let modules = vec![node("index.js", "export const x = 1;")];
    let chunk = make_chunk("c0");
    let decisions = TransformDecisions::default();

    let output = adapter
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("should fall back to SWC and succeed");

    assert!(
        output.code.contains("// chunk: c0"),
        "SWC fallback output should contain '// chunk: c0', got:\n{}",
        output.code
    );
}

/// When Node is not found and `fallback_on_failure = false`, the adapter must
/// propagate the error rather than silently falling back.
#[test]
fn errors_when_node_missing_and_fallback_disabled() {
    let config = RolldownAdapterConfig {
        node_path: PathBuf::from("/nonexistent/node-binary-that-cannot-exist"),
        fallback_on_failure: false,
    };
    let adapter = RolldownAdapter::with_config(config);

    let modules = vec![node("index.js", "export const x = 1;")];
    let chunk = make_chunk("c0");
    let decisions = TransformDecisions::default();

    let result = adapter.transform_chunk(&modules, &chunk, &decisions);

    assert!(
        result.is_err(),
        "should return an error when node is missing and fallback is disabled"
    );
}

/// The default configuration must use `"node"` as the node binary path and
/// have `fallback_on_failure` set to `true`.
#[test]
fn config_default_path_is_node() {
    let cfg = RolldownAdapterConfig::default();
    assert_eq!(
        cfg.node_path,
        PathBuf::from("node"),
        "default node_path should be 'node'"
    );
    assert!(
        cfg.fallback_on_failure,
        "default fallback_on_failure should be true"
    );
}
