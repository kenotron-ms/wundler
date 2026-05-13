use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A SHA-256 hex digest of file content, used for cache keying and deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub String);

impl ContentHash {
    /// Hash a UTF-8 string source.
    pub fn from_source(s: &str) -> Self {
        Self::from_bytes(s.as_bytes())
    }

    /// Hash raw bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        ContentHash(hex::encode(hasher.finalize()))
    }

    /// Return the hex string representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Export types
// ---------------------------------------------------------------------------

/// Describes what kind of export a binding represents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExportKind {
    Named,
    Default,
    ReExport,
    StarExport,
}

/// A single export binding in a module.
///
/// `source` is set only for `ReExport` and `StarExport` variants, indicating
/// the module specifier being re-exported from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Export {
    pub name: String,
    pub kind: ExportKind,
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// Import types
// ---------------------------------------------------------------------------

/// Describes the syntactic form of an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImportKind {
    Named,
    Default,
    Namespace,
    SideEffect,
    Dynamic,
}

/// A single import declaration in a module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    pub specifier: String,
    pub kind: ImportKind,
    pub bindings: Vec<String>,
    pub is_dynamic: bool,
}

// ---------------------------------------------------------------------------
// Call edge
// ---------------------------------------------------------------------------

/// A directed edge from one named export to another, representing a call
/// relationship used in tree-shaking analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller: String,
    pub callee: String,
}

// ---------------------------------------------------------------------------
// Side-effect marker
// ---------------------------------------------------------------------------

/// Indicates the side-effect status of a module.
///
/// Uses an internally-tagged serde representation so JSON looks like
/// `{"kind":"NONE"}`, `{"kind":"POSSIBLE","reason":"..."}`, or
/// `{"kind":"DEFINITE"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SideEffectMarker {
    #[serde(rename = "NONE")]
    None,
    #[serde(rename = "POSSIBLE")]
    Possible { reason: String },
    #[serde(rename = "DEFINITE")]
    Definite,
}

// ---------------------------------------------------------------------------
// Module summary
// ---------------------------------------------------------------------------

/// A compact, serialisable summary of a module's public surface and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSummary {
    pub exports: Vec<Export>,
    pub imports: Vec<Import>,
    #[serde(rename = "sideEffects")]
    pub side_effects: SideEffectMarker,
    #[serde(rename = "callEdges")]
    pub call_edges: Vec<CallEdge>,
    #[serde(rename = "ambientRefs")]
    pub ambient_refs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Bundle graph node
// ---------------------------------------------------------------------------

/// A single node in the bundle dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleGraphNode {
    pub id: ContentHash,
    pub path: String,
    pub summary: ModuleSummary,
    pub alive: bool,
    #[serde(rename = "chunkId", skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_from_source_is_64_hex_chars() {
        let hash = ContentHash::from_source("hello world");
        assert_eq!(hash.as_str().len(), 64);
    }

    #[test]
    fn test_content_hash_same_source_same_hash() {
        let a = ContentHash::from_source("same input");
        let b = ContentHash::from_source("same input");
        assert_eq!(a, b);
    }

    #[test]
    fn test_content_hash_different_source_different_hash() {
        let a = ContentHash::from_source("input a");
        let b = ContentHash::from_source("input b");
        assert_ne!(a, b);
    }

    #[test]
    fn test_bundle_graph_node_roundtrips_json() {
        let node = BundleGraphNode {
            id: ContentHash::from_source("some/path.js"),
            path: "some/path.js".to_string(),
            summary: ModuleSummary {
                exports: vec![],
                imports: vec![],
                side_effects: SideEffectMarker::None,
                call_edges: vec![],
                ambient_refs: vec![],
            },
            alive: true,
            chunk_id: Some("chunk-0".to_string()),
        };
        let json = serde_json::to_string(&node).expect("serialization failed");
        let roundtripped: BundleGraphNode =
            serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(roundtripped.id, node.id);
        assert_eq!(roundtripped.path, node.path);
        assert_eq!(roundtripped.alive, node.alive);
        assert_eq!(roundtripped.chunk_id, node.chunk_id);
    }

    #[test]
    fn test_side_effect_marker_possible_serializes_with_reason() {
        let marker = SideEffectMarker::Possible {
            reason: "writes to global".to_string(),
        };
        let json = serde_json::to_string(&marker).expect("serialization failed");
        assert!(
            json.contains("\"kind\":\"POSSIBLE\""),
            "expected kind:POSSIBLE in: {json}"
        );
        assert!(
            json.contains("writes to global"),
            "expected reason text in: {json}"
        );
    }
}
