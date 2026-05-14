//! Delta manifest computation for the Asset Bundling Server.
//!
//! Given a [`ChunkManifest`] (the current server build) and a
//! [`ManifestRequest`] (what the client already holds in cache), this module
//! computes the minimal set of chunk URLs the client needs to fetch.

use std::collections::{HashMap, HashSet};

use wundler_core::types::ContentHash;
use wundler_graph::ChunkManifest;

use crate::types::{ManifestRequest, ManifestResponse};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Minimum `co_request_score` for a chunk to be included in `prefetch_urls`.
///
/// Chunks with a score at or above this threshold are suggested as prefetch
/// hints to the client.  The value is tuned by offline validation (Plan 5).
pub const PREFETCH_THRESHOLD: f64 = 0.7;

/// Compute the delta manifest: which chunks the client still needs to fetch.
///
/// # Algorithm
///
/// 1. Look up the entry point in `manifest.entry_chunks`.  If the entry point
///    is unknown, return an empty [`ManifestResponse`] with the manifest's
///    `build_id` and `ttl = 0`.
///
/// 2. Build a by-ID lookup of all chunks in the manifest.
///
/// 3. For each chunk required by the entry point:
///    * A chunk with **all** of its modules present in `cached_hashes` is
///      considered fully cached and is **excluded** from the response.
///    * A chunk with **no modules** is *not* considered fully cached (it is
///      always included).
///    * Any chunk that is not fully cached has its CDN URL added to
///      `fetch_urls` and its ID recorded in `fetched_ids`.
///
/// 4. Build `prefetch_urls` by iterating **all** chunks in the manifest:
///    * Skip chunks already in `fetched_ids`.
///    * Skip chunks with no `co_request_score`.
///    * Skip chunks whose score is below [`PREFETCH_THRESHOLD`].
///    * Skip chunks whose every module is in the client's cached set (same
///      `trust_cache` gate as step 3).
///
/// 5. `ttl` is always 0; the HTTP handler is responsible for setting the
///    actual cache TTL on the response.
pub fn compute_delta(
    manifest: &ChunkManifest,
    request: &ManifestRequest,
    cdn_base_url: &str,
) -> ManifestResponse {
    // 1. Resolve entry's chunk list; unknown entry → empty response.
    let chunk_ids = match manifest.entry_chunks.get(&request.entry_point) {
        Some(ids) => ids,
        None => {
            return ManifestResponse {
                build_id: manifest.build_id.clone(),
                fetch_urls: vec![],
                prefetch_urls: vec![],
                ttl: 0,
            };
        }
    };

    // 2. Build a HashMap of chunks by their ID for O(1) lookup.
    let chunks_by_id: HashMap<&str, _> = manifest
        .chunks
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();

    // 3. Gate cache trust on build_id match.
    //    If the client's build_id is absent or doesn't match the current build,
    //    ignore all cache claims and return the full chunk set.
    let trust_cache = matches!(&request.build_id, Some(bid) if bid == &manifest.build_id);

    // 4. Build the set of hashes the client already holds (empty if cache is untrusted).
    let cached_set: HashSet<&ContentHash> = if trust_cache {
        request.cached_hashes.iter().collect()
    } else {
        HashSet::new()
    };

    // 4. Filter entry chunks: exclude only those whose every module is cached.
    //    Chunks with zero modules are NOT considered fully cached.
    //    Also track fetched_ids so the prefetch pass can skip already-queued chunks.
    let mut fetch_urls: Vec<String> = Vec::new();
    let mut fetched_ids: HashSet<&str> = HashSet::new();
    for chunk_id in chunk_ids {
        if let Some(chunk) = chunks_by_id.get(chunk_id.as_str()) {
            let fully_cached = !chunk.modules.is_empty()
                && chunk.modules.iter().all(|m| cached_set.contains(m));
            if !fully_cached {
                fetch_urls.push(chunk_url(cdn_base_url, &chunk.hash));
                fetched_ids.insert(chunk.id.as_str());
            }
        }
    }

    // 5. Build prefetch hints from co_request_score across ALL chunks.
    let mut prefetch_urls: Vec<String> = Vec::new();
    for chunk in &manifest.chunks {
        // Skip chunks already queued for fetch.
        if fetched_ids.contains(chunk.id.as_str()) {
            continue;
        }
        // Skip chunks without a co_request_score.
        let score = match chunk.co_request_score {
            Some(s) => s,
            None => continue,
        };
        // Skip chunks whose score is below the threshold.
        if score < PREFETCH_THRESHOLD {
            continue;
        }
        // Skip chunks the client already has (same trust_cache gate as fetch).
        let client_has_all = !chunk.modules.is_empty()
            && chunk.modules.iter().all(|m| cached_set.contains(m));
        if client_has_all {
            continue;
        }
        prefetch_urls.push(chunk_url(cdn_base_url, &chunk.hash));
    }

    ManifestResponse {
        build_id: manifest.build_id.clone(),
        fetch_urls,
        prefetch_urls,
        ttl: 0,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the CDN URL for a chunk given its content hash.
///
/// The filename is the **first 8 hex characters** of the chunk hash, matching
/// the naming convention used by the bundler pipeline's output stage.
///
/// # Example
///
/// ```ignore
/// chunk_url("https://cdn.example.com/", &hash)
/// // → "https://cdn.example.com/chunks/aabbccdd.js"
/// ```
fn chunk_url(base: &str, hash: &ContentHash) -> String {
    let base = base.trim_end_matches('/');
    let first8 = &hash.as_str()[..8];
    format!("{base}/chunks/{first8}.js")
}

// ---------------------------------------------------------------------------
// Unit tests for chunk_url helper
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_url_trims_trailing_slash() {
        let hash = ContentHash("aabbccdd1234567890abcdef".to_string());
        let url = chunk_url("https://cdn.example.com/", &hash);
        assert_eq!(url, "https://cdn.example.com/chunks/aabbccdd.js");
    }

    #[test]
    fn chunk_url_no_trailing_slash() {
        let hash = ContentHash("deadbeef1234567890abcdef".to_string());
        let url = chunk_url("https://cdn.example.com", &hash);
        assert_eq!(url, "https://cdn.example.com/chunks/deadbeef.js");
    }
}
