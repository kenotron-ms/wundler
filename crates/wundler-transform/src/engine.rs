//! TransformEngine trait — the seam between Wundler's analysis and code generation.

use std::collections::{HashMap, HashSet};

use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::types::{Chunk, ChunkId};

// ---------------------------------------------------------------------------
// ChunkOutput
// ---------------------------------------------------------------------------

/// The result of transforming a single chunk into emittable JavaScript.
#[derive(Debug, Clone)]
pub struct ChunkOutput {
    /// Identifier of the chunk that was transformed.
    pub chunk_id: ChunkId,

    /// Content hash of the emitted `code` bytes.
    pub hash: ContentHash,

    /// The emitted JavaScript source text.
    pub code: String,

    /// Optional source map JSON, if the engine produced one.
    pub source_map: Option<String>,
}

// ---------------------------------------------------------------------------
// TransformDecisions
// ---------------------------------------------------------------------------

/// Decisions made by the analysis pipeline that the engine must respect when
/// emitting code (e.g. which exports have been tree-shaken away).
#[derive(Debug, Clone, Default)]
pub struct TransformDecisions {
    /// Maps each module's content hash to the set of export names that are
    /// dead (unreachable) and should be omitted from the output.
    pub dead_exports: HashMap<ContentHash, HashSet<String>>,
}

// ---------------------------------------------------------------------------
// TransformError
// ---------------------------------------------------------------------------

/// Errors that can occur while transforming a chunk.
#[derive(Debug, thiserror::Error)]
pub enum TransformError {
    /// The engine failed to transform the given chunk.
    #[error("transform failed for chunk {chunk_id}: {reason}")]
    TransformFailed { chunk_id: String, reason: String },

    /// An I/O error occurred (e.g. when writing output files).
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// A serialization error occurred.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// TransformEngine
// ---------------------------------------------------------------------------

/// Engine-agnostic interface for emitting a chunk file from its constituent
/// modules.
///
/// Implementors are responsible for producing `ChunkOutput` from the given
/// slice of `BundleGraphNode`s and `TransformDecisions`.  The trait is
/// `Send + Sync` so that implementations can be used from multiple threads
/// (e.g. via `rayon`).
pub trait TransformEngine: Send + Sync {
    /// Transform all `modules` belonging to `chunk` into a single
    /// `ChunkOutput`, taking `decisions` (e.g. dead exports) into account.
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError>;
}
