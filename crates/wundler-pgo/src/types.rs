//! Data types shared across the PGO Store.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One ingested telemetry record, normalized to a chunk sequence.
///
/// `chunk_sequence` preserves the order in which the ABS told the
/// client to fetch its chunks for this entry. The PGO Store treats
/// position 0 as "first chunk loaded".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub session_id: String,
    pub entry_point: String,
    pub chunk_sequence: Vec<String>,
    pub timestamp_ms: u64,
}

/// All hints computed for a single manifest by one `wundler pgo apply` run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PgoHints {
    pub build_id: String,
    pub chunk_hints: HashMap<String, ChunkHint>,
}

/// Per-chunk advisory information written back into `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkHint {
    pub co_request_score: f64,
    pub median_load_order: f64,
    pub suggested_merge: Option<String>,
}
