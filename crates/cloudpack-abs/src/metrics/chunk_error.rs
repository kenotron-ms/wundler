//! Request and event types for the chunk-error telemetry endpoint.

use serde::{Deserialize, Serialize};

use crate::metrics::ErrorType;

/// Body sent by the Service Worker via `POST /telemetry/chunk-error`.
#[derive(Debug, Deserialize)]
pub struct ChunkErrorReport {
    pub build_id: String,
    pub chunk_id: String,
    pub url: String,
    pub error_type: ErrorType,
    pub timestamp_ms: u64,
    pub session_id: String,
}

/// JSONL event appended to the telemetry log.
#[derive(Debug, Serialize)]
pub struct ChunkErrorEvent {
    pub kind: &'static str, // always "chunk_error"
    pub build_id: String,
    pub chunk_id: String,
    pub url: String,
    pub error_type: ErrorType,
    pub timestamp_ms: u64,
    pub session_id: String,
}

impl From<ChunkErrorReport> for ChunkErrorEvent {
    fn from(r: ChunkErrorReport) -> Self {
        Self {
            kind: "chunk_error",
            build_id: r.build_id,
            chunk_id: r.chunk_id,
            url: r.url,
            error_type: r.error_type,
            timestamp_ms: r.timestamp_ms,
            session_id: r.session_id,
        }
    }
}
