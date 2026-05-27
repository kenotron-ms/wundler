use std::collections::{HashMap, HashSet};
use cloudpack_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use cloudpack_graph::types::{Chunk, LoadCondition};
use cloudpack_transform::engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};

struct StubEngine;

impl TransformEngine for StubEngine {
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        _decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError> {
        Ok(ChunkOutput {
            chunk_id: chunk.id.clone(),
            hash: ContentHash::from_bytes(b"stub"),
            code: format!("// {} modules\n", modules.len()),
            source_map: None,
            already_written: false,
        })
    }
}

fn make_node(hash_bytes: &[u8]) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::from_bytes(hash_bytes),
        path: "some/module.js".to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: vec![],
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: Some("chunk-0".to_string()),
        source: None,
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

#[test]
fn stub_engine_returns_expected_output() {
    let engine = StubEngine;
    let modules = vec![make_node(b"mod-a"), make_node(b"mod-b")];
    let chunk = make_chunk("chunk-42");
    let decisions = TransformDecisions::default();

    let output = engine
        .transform_chunk(&modules, &chunk, &decisions)
        .expect("transform should succeed");

    assert_eq!(output.chunk_id, "chunk-42");
    assert_eq!(output.code, "// 2 modules\n");
    assert!(output.source_map.is_none());
}

#[test]
fn transform_decisions_records_dead_exports() {
    let hash = ContentHash::from_bytes(b"module-x");
    let mut dead: HashMap<ContentHash, HashSet<String>> = HashMap::new();
    dead.insert(
        hash.clone(),
        HashSet::from(["unusedFn".to_string(), "deadHelper".to_string()]),
    );

    let decisions = TransformDecisions { dead_exports: dead };

    let exports_for_module = decisions.dead_exports.get(&hash).expect("key must exist");
    assert!(exports_for_module.contains("unusedFn"));
    assert!(exports_for_module.contains("deadHelper"));
    assert_eq!(exports_for_module.len(), 2);
}

#[test]
fn transform_error_displays_with_chunk_id() {
    let err = TransformError::TransformFailed {
        chunk_id: "c1".to_string(),
        reason: "parse error".to_string(),
    };
    assert!(format!("{}", err).contains("c1"));
}
