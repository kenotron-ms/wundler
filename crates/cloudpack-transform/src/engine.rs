//! TransformEngine trait — the seam between Cloudpack's analysis and code generation.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use cloudpack_core::types::{BundleGraphNode, ContentHash};
use cloudpack_graph::analyzer::AnalysisResult;
use cloudpack_graph::types::{Chunk, ChunkId};

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
    ///
    /// For engines that write their output directly to disk (e.g. `RolldownAdapter`),
    /// this field is empty and `already_written` is `true`.
    pub code: String,

    /// Optional source map JSON, if the engine produced one.
    pub source_map: Option<String>,

    /// When `true`, the file has already been written to `out_dir` by the engine
    /// (e.g. rolldown subprocess) and the pipeline must NOT call `write_chunk` again.
    pub already_written: bool,
}

// ---------------------------------------------------------------------------
// BatchConfig
// ---------------------------------------------------------------------------

/// Minimal configuration for a batch-transform pass.
///
/// Defined here (not in `cloudpack-pipeline`) to avoid a circular crate
/// dependency: `cloudpack-transform` → `cloudpack-pipeline` would be circular.
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Root directory that was scanned for source modules.
    pub root: PathBuf,
    /// Entry-point route → file-path mapping (same as `BuildConfig::entry_points`).
    pub entry_points: HashMap<String, PathBuf>,
    /// Absolute (or workspace-relative) path where bundle output is written.
    pub out_dir: PathBuf,
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

    /// Batch-transform: run all chunks in one pass, returning one
    /// [`ChunkOutput`] per chunk in `analysis.manifest.chunks`.
    ///
    /// The default implementation calls [`transform_chunk`] for each chunk.
    /// Engines like `RolldownAdapter` override this to invoke a subprocess
    /// once for the entire project.
    fn batch_transform(
        &self,
        analysis: &AnalysisResult,
        _config: &BatchConfig,
    ) -> Result<Vec<ChunkOutput>, TransformError> {
        let decisions = TransformDecisions::default();
        analysis
            .manifest
            .chunks
            .iter()
            .map(|chunk| {
                let modules: Vec<BundleGraphNode> = chunk
                    .modules
                    .iter()
                    .filter_map(|hash| analysis.nodes.iter().find(|n| &n.id == hash))
                    .cloned()
                    .collect();
                self.transform_chunk(&modules, chunk, &decisions)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Utility: sanitize_entry_key
// ---------------------------------------------------------------------------

/// Convert a route string into a filesystem-safe identifier used as the
/// rolldown entry name.
///
/// * `"/"` → `"root"`
/// * `"/dashboard"` → `"dashboard"`
/// * `"/admin/users"` → `"admin-users"`
pub fn sanitize_entry_key(route: &str) -> String {
    if route == "/" {
        return "root".to_string();
    }
    route.trim_start_matches('/').replace('/', "-")
}
