//! In-process metrics counters for the Asset Bundling Server.
//!
//! The `Metrics` struct holds atomic counters for manifest requests and a
//! `DashMap<ChunkErrorKey, AtomicU64>` for chunk-level error counts. The
//! DashMap is bounded by `(active_build_ids × chunk_ids × 4_error_types)`;
//! `prune()` removes entries whose `build_id` is no longer active, called
//! on every manifest hot-reload to prevent unbounded growth.

pub mod chunk_error;
pub mod prometheus;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error type enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    LoadFailed,
    NetworkTimeout,
    IntegrityMismatch,
}

// ---------------------------------------------------------------------------
// ChunkErrorKey
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChunkErrorKey {
    pub build_id: String,
    pub chunk_id: String,
    pub error_type: ErrorType,
}

// ---------------------------------------------------------------------------
// Metrics struct
// ---------------------------------------------------------------------------

/// In-process metrics counters.
///
/// All fields are cheap to clone (`Arc`-wrapped or `AtomicU64`).
#[derive(Debug, Clone)]
pub struct Metrics {
    pub manifest_requests_total: Arc<AtomicU64>,
    pub manifest_bytes_total: Arc<AtomicU64>,
    pub chunk_errors: Arc<DashMap<ChunkErrorKey, AtomicU64>>,
    pub cache_hits_total: Arc<AtomicU64>,
    pub cache_misses_total: Arc<AtomicU64>,
}

impl Metrics {
    /// Create a fresh `Metrics` instance with all counters at zero.
    pub fn new() -> Self {
        Self {
            manifest_requests_total: Arc::new(AtomicU64::new(0)),
            manifest_bytes_total: Arc::new(AtomicU64::new(0)),
            chunk_errors: Arc::new(DashMap::new()),
            cache_hits_total: Arc::new(AtomicU64::new(0)),
            cache_misses_total: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Increment the chunk-error counter for `key`. Inserts a zero entry if
    /// this is the first error for that key.
    pub fn increment_chunk_error(&self, key: ChunkErrorKey) {
        self.chunk_errors
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Remove entries whose `build_id` is not in `active_build_ids`.
    ///
    /// Called on every manifest hot-reload so stale per-build counters are
    /// dropped and the DashMap stays bounded.
    pub fn prune(&self, active_build_ids: &HashSet<String>) {
        self.chunk_errors
            .retain(|k, _| active_build_ids.contains(&k.build_id));
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn key(build_id: &str, chunk_id: &str, et: ErrorType) -> ChunkErrorKey {
        ChunkErrorKey {
            build_id: build_id.to_string(),
            chunk_id: chunk_id.to_string(),
            error_type: et,
        }
    }

    #[test]
    fn increment_creates_entry_and_increments() {
        let m = Metrics::new();
        let k = key("build1", "chunk-a", ErrorType::LoadFailed);
        m.increment_chunk_error(k.clone());
        m.increment_chunk_error(k.clone());
        let val = m.chunk_errors.get(&k).map(|v| v.load(Ordering::Relaxed)).unwrap_or(0);
        assert_eq!(val, 2);
    }

    #[test]
    fn different_keys_are_independent() {
        let m = Metrics::new();
        let k1 = key("build1", "chunk-a", ErrorType::LoadFailed);
        let k2 = key("build1", "chunk-a", ErrorType::NetworkTimeout);
        m.increment_chunk_error(k1.clone());
        m.increment_chunk_error(k1.clone());
        m.increment_chunk_error(k2.clone());
        let v1 = m.chunk_errors.get(&k1).map(|v| v.load(Ordering::Relaxed)).unwrap_or(0);
        let v2 = m.chunk_errors.get(&k2).map(|v| v.load(Ordering::Relaxed)).unwrap_or(0);
        assert_eq!(v1, 2);
        assert_eq!(v2, 1);
    }

    #[test]
    fn prune_removes_stale_build_ids() {
        let m = Metrics::new();
        m.increment_chunk_error(key("build1", "chunk-a", ErrorType::LoadFailed));
        m.increment_chunk_error(key("build2", "chunk-b", ErrorType::NetworkTimeout));
        assert_eq!(m.chunk_errors.len(), 2);

        let active: HashSet<String> = ["build1".to_string()].into_iter().collect();
        m.prune(&active);

        assert_eq!(m.chunk_errors.len(), 1);
        assert!(m.chunk_errors.contains_key(&key("build1", "chunk-a", ErrorType::LoadFailed)));
        assert!(!m.chunk_errors.contains_key(&key("build2", "chunk-b", ErrorType::NetworkTimeout)));
    }

    #[test]
    fn prune_with_empty_active_set_clears_all() {
        let m = Metrics::new();
        m.increment_chunk_error(key("build1", "chunk-a", ErrorType::LoadFailed));
        m.prune(&HashSet::new());
        assert_eq!(m.chunk_errors.len(), 0);
    }

    #[test]
    fn prune_with_all_active_keeps_all() {
        let m = Metrics::new();
        m.increment_chunk_error(key("build1", "chunk-a", ErrorType::LoadFailed));
        m.increment_chunk_error(key("build2", "chunk-b", ErrorType::NetworkTimeout));
        let active: HashSet<String> = ["build1".to_string(), "build2".to_string()].into_iter().collect();
        m.prune(&active);
        assert_eq!(m.chunk_errors.len(), 2);
    }
}
