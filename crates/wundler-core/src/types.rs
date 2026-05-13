use serde::{Deserialize, Serialize};

/// A hash of file content for cache keying.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub String);

/// The kind of an import (static or dynamic).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportKind {
    Static,
    Dynamic,
}

/// An import statement in a module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    pub source: String,
    pub kind: ImportKind,
    pub bindings: Vec<String>,
}

/// The kind of an export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportKind {
    Named,
    Default,
    ReExport,
    StarReExport,
}

/// An export statement in a module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Export {
    pub name: String,
    pub kind: ExportKind,
}

/// Marks whether a module has side effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SideEffectMarker {
    Unknown,
    Free,
    HasSideEffects,
}

/// A directed edge representing a function call from one export to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub from: String,
    pub to: String,
}

/// A compact summary of a module's exports, imports, and side effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSummary {
    pub path: String,
    pub content_hash: ContentHash,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    pub side_effects: SideEffectMarker,
    pub call_edges: Vec<CallEdge>,
}

/// A node in the bundle dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleGraphNode {
    pub summary: ModuleSummary,
}
