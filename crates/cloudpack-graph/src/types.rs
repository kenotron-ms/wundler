//! Chunk and manifest types for the Cloudpack bundle graph.
//!
//! This module defines the core data structures that describe how modules are
//! grouped into chunks for delivery to the browser.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use cloudpack_core::types::ContentHash;

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// An opaque identifier for a single chunk output file.
pub type ChunkId = String;

/// A key identifying a named entry point in the application (e.g. `"main"`).
pub type EntryPoint = String;

// ---------------------------------------------------------------------------
// LoadCondition
// ---------------------------------------------------------------------------

/// Describes when / how a chunk is loaded by the browser.
///
/// * `Initial`  – part of the entry HTML payload; loaded on first page render.
/// * `Lazy`     – behind a dynamic `import()` boundary; fetched on demand.
/// * `Prefetch` – speculative prefetch; loaded when the browser is idle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadCondition {
    /// Chunk is included in the initial page load.
    Initial,
    /// Chunk is loaded lazily at a dynamic `import()` boundary.
    Lazy,
    /// Chunk is speculatively prefetched by the browser.
    Prefetch,
}

// ---------------------------------------------------------------------------
// Chunk
// ---------------------------------------------------------------------------

/// A single output chunk produced by the bundler.
///
/// A chunk groups one or more modules together for a single network request.
/// The `load_condition` field determines whether the browser loads the chunk
/// eagerly, lazily, or speculatively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique identifier for this chunk (e.g. `"chunk-0"`).
    pub id: ChunkId,

    /// Hashes of all modules included in this chunk.
    pub modules: Vec<ContentHash>,

    /// Content hash of the chunk's output bytes, used for cache busting.
    pub hash: ContentHash,

    /// When/how this chunk is loaded by the browser.
    pub load_condition: LoadCondition,

    /// Probability (0–1) that this chunk is requested alongside another chunk
    /// in the same navigation.  `None` if not yet computed.
    pub co_request_score: Option<f64>,

    /// Median position of this chunk in the observed load waterfall.  `None`
    /// if not yet computed.
    pub median_load_order: Option<f64>,

    /// Suggested target chunk for a merge optimisation pass.  `None` if no
    /// merge candidate has been identified.
    pub suggested_merge: Option<ChunkId>,
}

// ---------------------------------------------------------------------------
// ChunkManifest
// ---------------------------------------------------------------------------

/// The complete manifest produced at the end of a build.
///
/// Combines all chunks with index structures that allow callers to look up
/// which chunk contains a given module, and which chunks belong to a given
/// entry point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkManifest {
    /// Unique identifier for this build (e.g. a short git SHA or timestamp).
    pub build_id: String,

    /// All chunks produced by the build.
    pub chunks: Vec<Chunk>,

    /// Maps each named entry point to the list of chunk IDs it requires.
    pub entry_chunks: HashMap<EntryPoint, Vec<ChunkId>>,

    /// Reverse index: maps each module content hash to the chunk that contains
    /// it.
    pub module_index: HashMap<ContentHash, ChunkId>,
}

impl ChunkManifest {
    /// Serialise the manifest to a pretty-printed JSON string.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Deserialise a manifest from a JSON string.
    ///
    /// Returns an error if the input is not valid JSON or does not match the
    /// expected schema.
    pub fn from_json(s: &str) -> Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}
