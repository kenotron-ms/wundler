//! Wire types for the Asset Bundling Server (ABS) HTTP API.
//!
//! # Chunk caching semantics
//!
//! A chunk is considered "already had" by the client only if **every** module
//! that composes that chunk is listed in the client's `cached_hashes`.  A
//! single missing module means the whole chunk must be re-fetched.
//!
//! # Build-ID matching
//!
//! When `build_id` is absent from a [`ManifestRequest`] **or** the value does
//! not match the server's current build, ABS returns the **full** chunk set
//! for the entry-point — no delta optimisation is attempted.
//!
//! # Session identifiers
//!
//! `session_id` in [`TelemetryEvent`] is a random UUID generated once per
//! browser session.  It is **not** a user identifier and must not be used
//! for user-level analytics or cross-session correlation.

use serde::{Deserialize, Serialize};
use cloudpack_core::types::ContentHash;

/// Request body sent by the browser to ask which chunks it should fetch.
///
/// The client supplies the entry-point it wants to load together with the
/// content-hashes of all modules it already has in its local cache.  The
/// server uses this information to compute the minimal set of chunks the
/// client still needs.
///
/// `build_id`, when present, lets ABS detect stale caches: if the supplied
/// ID does not match the current build the server responds with the full
/// chunk list regardless of `cached_hashes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRequest {
    /// The application entry-point (e.g. `"src/index.ts"`).
    pub entry_point: String,

    /// SHA-256 hashes of every module the client already holds in cache.
    pub cached_hashes: Vec<ContentHash>,

    /// Opaque build identifier issued by the last manifest response.
    ///
    /// Absent on the very first request or after a hard-reload.  If absent
    /// or mismatched the server returns the complete chunk set.
    #[serde(default)]
    pub build_id: Option<String>,
}

/// Response body returned by ABS after evaluating a [`ManifestRequest`].
///
/// `fetch_urls` are chunks the client **must** download immediately.
/// `prefetch_urls` are chunks the client should warm into the cache ahead
/// of time (e.g. chunks reachable via lazy `import()`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestResponse {
    /// Opaque identifier for the current server build.
    ///
    /// Clients should echo this value back in subsequent requests so ABS can
    /// detect stale caches and skip delta computation when the build changes.
    pub build_id: String,

    /// URLs of chunks the client must fetch now.
    pub fetch_urls: Vec<String>,

    /// URLs of chunks worth prefetching for future navigations.
    pub prefetch_urls: Vec<String>,

    /// Cache TTL for this manifest in seconds.
    pub ttl: u64,
}

/// A single telemetry event recorded when a manifest is served.
///
/// `session_id` is a random UUID generated once per browser session.  It is
/// scoped to a single tab/window lifetime and is **not** a user identifier.
///
/// `client_had` lists the content-hashes the client reported; `chunks_served`
/// lists the chunk identifiers that were included in the `fetch_urls` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryEvent {
    /// Random per-session UUID — **not** a user identifier.
    pub session_id: String,

    /// Entry-point that triggered this manifest request.
    pub entry_point: String,

    /// Chunk identifiers returned in the manifest response.
    pub chunks_served: Vec<String>,

    /// Content-hashes the client reported as already cached.
    pub client_had: Vec<ContentHash>,

    /// Unix epoch time in milliseconds when the event was recorded.
    pub timestamp_ms: u64,
}

// ---------------------------------------------------------------------------
// TelemetryEventV2 — tagged enum for typed telemetry log entries.
//
// PGO is tolerant (no deny_unknown_fields) so additive new `kind` values are
// safe. Existing manifest events continue to be logged as the legacy
// `TelemetryEvent` shape (the `kind: "manifest"` arm below is for future
// migration only).
// ---------------------------------------------------------------------------

/// Tagged telemetry event discriminated by `kind`.
///
/// New events (e.g. `chunk_error`) use this type. Old manifest events continue
/// to use the flat `TelemetryEvent` struct until a separate migration PR.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TelemetryEventV2 {
    /// Chunk load failure reported by the Service Worker.
    ChunkError(ChunkErrorEventV2),
}

/// Body of a `kind: "chunk_error"` telemetry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkErrorEventV2 {
    pub build_id: String,
    pub chunk_id: String,
    pub url: String,
    pub error_type: crate::metrics::ErrorType,
    pub timestamp_ms: u64,
    pub session_id: String,
}

#[cfg(test)]
mod v2_tests {
    use super::*;
    use crate::metrics::ErrorType;

    #[test]
    fn chunk_error_event_has_kind_field() {
        let ev = TelemetryEventV2::ChunkError(ChunkErrorEventV2 {
            build_id: "build-1".to_string(),
            chunk_id: "chunk-a".to_string(),
            url: "https://cdn.example.com/chunks/abc.js".to_string(),
            error_type: ErrorType::LoadFailed,
            timestamp_ms: 1716000000000,
            session_id: "sess-1".to_string(),
        });
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "chunk_error");
        assert_eq!(json["build_id"], "build-1");
        assert_eq!(json["error_type"], "load_failed");
    }
}
