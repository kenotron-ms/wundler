//! `ChunkManifest` assembly for the Cloudpack bundle graph.
//!
//! This module provides [`build_manifest`], which assembles a [`ChunkManifest`]
//! from a completed set of chunks, entry-point hashes, and a module-to-chunk
//! reverse index.

use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};
use cloudpack_core::types::ContentHash;

use crate::types::{Chunk, ChunkId, ChunkManifest, EntryPoint};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute a deterministic build identifier from a set of entry-point routes.
///
/// # Algorithm
///
/// 1. Collect the route path keys (e.g. `"/"`, `"/about"`) as `&str`.
/// 2. Sort the keys lexicographically.
/// 3. Join with `'\n'`.
/// 4. Compute SHA-256 over the UTF-8 bytes.
/// 5. Hex-encode the digest — always 64 lowercase ASCII characters.
pub fn compute_build_id(entry_hashes: &HashMap<String, ContentHash>) -> String {
    let mut keys: Vec<&str> = entry_hashes.keys().map(String::as_str).collect();
    keys.sort();
    let joined = keys.join("\n");
    let digest = Sha256::digest(joined.as_bytes());
    hex::encode(digest)
}

/// Assemble a [`ChunkManifest`] from chunks, entry-point hashes, and a module
/// index.
///
/// # Parameters
///
/// * `chunks` — all chunks produced by the current build.
/// * `entry_hashes` — map from route path (e.g. `"/"`) to the content hash of
///   the entry module for that route.
/// * `module_index` — reverse map from module [`ContentHash`] to the
///   [`ChunkId`] of the chunk that contains it.
///
/// # Returns
///
/// A [`ChunkManifest`] with:
///
/// * `build_id` — derived from the sorted set of entry-point route paths.
/// * `entry_chunks` — for each route, an ordered list of chunk IDs: if a
///   `"commons"` chunk is present it appears first, followed by the owning
///   chunk for that route's entry module (if not already listed).
/// * `chunks` — passed through unchanged.
/// * `module_index` — passed through unchanged.
pub fn build_manifest(
    chunks: Vec<Chunk>,
    entry_hashes: &HashMap<String, ContentHash>,
    module_index: HashMap<ContentHash, ChunkId>,
) -> ChunkManifest {
    // Derive the build identifier from the entry-point route paths.
    let build_id = compute_build_id(entry_hashes);

    // Build the set of known chunk IDs so we can check for "commons" quickly.
    let known_chunk_ids: HashSet<&str> = chunks.iter().map(|c| c.id.as_str()).collect();

    // For each entry route, produce an ordered list of required chunk IDs.
    let mut entry_chunks: HashMap<EntryPoint, Vec<ChunkId>> = HashMap::new();

    for (route, entry_hash) in entry_hashes {
        let mut ordered: Vec<ChunkId> = Vec::new();

        // Step 1: if a "commons" chunk exists, it goes first.
        if known_chunk_ids.contains("commons") {
            ordered.push("commons".to_string());
        }

        // Step 2: find the chunk that owns the entry module for this route.
        if let Some(owning_chunk) = module_index.get(entry_hash) {
            // Only push if it is not the "commons" chunk and not already listed.
            if owning_chunk != "commons" && !ordered.contains(owning_chunk) {
                ordered.push(owning_chunk.clone());
            }
        }

        entry_chunks.insert(route.clone(), ordered);
    }

    ChunkManifest {
        build_id,
        chunks,
        entry_chunks,
        module_index,
    }
}
