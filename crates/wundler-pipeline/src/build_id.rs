//! Deterministic build-ID generation for chunk manifests.
//!
//! Provides [`canonical_bytes`] (stable byte representation of a
//! [`ChunkManifest`], excluding the `build_id` field itself) and
//! [`compute_build_id`] (SHA-256 of those bytes, first 8 bytes hex-encoded
//! as a 16-character lowercase string).

use std::collections::BTreeMap;

use serde::Serialize;
use sha2::{Digest, Sha256};
use wundler_core::types::ContentHash;
use wundler_graph::types::{Chunk, ChunkId, ChunkManifest, LoadCondition};

// ---------------------------------------------------------------------------
// Private canonical types
// ---------------------------------------------------------------------------

/// Canonical representation of a single chunk for hashing.
///
/// Sorts `modules` by content hash ascending to remove any insertion-order
/// dependency from the analysis phase.
#[derive(Serialize)]
struct CanonicalChunk<'a> {
    id: &'a str,
    modules: Vec<&'a ContentHash>,
    hash: &'a ContentHash,
    load_condition: &'a LoadCondition,
    co_request_score: Option<f64>,
    median_load_order: Option<f64>,
    suggested_merge: Option<&'a str>,
}

impl<'a> CanonicalChunk<'a> {
    fn from_chunk(chunk: &'a Chunk) -> Self {
        let mut modules: Vec<&'a ContentHash> = chunk.modules.iter().collect();
        modules.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            id: &chunk.id,
            modules,
            hash: &chunk.hash,
            load_condition: &chunk.load_condition,
            co_request_score: chunk.co_request_score,
            median_load_order: chunk.median_load_order,
            suggested_merge: chunk.suggested_merge.as_deref(),
        }
    }
}

/// Canonical manifest shape — mirrors [`ChunkManifest`] but omits `build_id`
/// to avoid a circular dependency when computing the content hash.
///
/// HashMap fields are replaced with [`BTreeMap`] keyed on `&str` to produce
/// a stable, lexicographically-ordered JSON serialization.
///
/// Note: [`ContentHash`] does not implement [`Ord`], so `module_index` is
/// keyed on `&str` (via [`ContentHash::as_str`]) rather than `&ContentHash`.
#[derive(Serialize)]
struct CanonicalManifest<'a> {
    /// Sorted ascending by chunk `id`.
    chunks: Vec<CanonicalChunk<'a>>,
    /// Sorted by entry-point name.
    entry_chunks: BTreeMap<&'a str, &'a Vec<ChunkId>>,
    /// Sorted by content hash hex string.
    module_index: BTreeMap<&'a str, &'a ChunkId>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Serialize `manifest` to a stable, canonical byte sequence.
///
/// The `build_id` field is excluded to avoid a circular dependency.
/// HashMap fields are converted to sorted structures to eliminate
/// non-deterministic iteration order. Chunk slices are sorted by `id`.
/// Module slices within each chunk are sorted by content hash.
///
/// # Panics
///
/// Panics if `serde_json` fails to serialize — should never happen for a
/// well-formed `ChunkManifest`.
pub fn canonical_bytes(manifest: &ChunkManifest) -> Vec<u8> {
    let mut chunks: Vec<CanonicalChunk<'_>> =
        manifest.chunks.iter().map(CanonicalChunk::from_chunk).collect();
    chunks.sort_by(|a, b| a.id.cmp(b.id));

    let entry_chunks: BTreeMap<&str, &Vec<ChunkId>> = manifest
        .entry_chunks
        .iter()
        .map(|(k, v)| (k.as_str(), v))
        .collect();

    let module_index: BTreeMap<&str, &ChunkId> = manifest
        .module_index
        .iter()
        .map(|(k, v)| (k.0.as_str(), v))
        .collect();

    let canonical = CanonicalManifest {
        chunks,
        entry_chunks,
        module_index,
    };

    serde_json::to_vec(&canonical)
        .expect("CanonicalManifest serialization must never fail")
}

/// Compute a deterministic build ID from a [`ChunkManifest`]'s content.
///
/// Returns a 16-character lowercase hex string derived from the first 8 bytes
/// of the SHA-256 digest of [`canonical_bytes`].
///
/// Two manifests with identical chunks, entry-points, and module-index produce
/// the same ID regardless of HashMap iteration order or `build_id` placeholder.
pub fn compute_build_id(manifest: &ChunkManifest) -> String {
    let bytes = canonical_bytes(manifest);
    let digest = Sha256::digest(&bytes);
    hex::encode(&digest[..8])
}
