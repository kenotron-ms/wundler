//! Log ingestor: reads a JSONL telemetry file and populates a [`PgoStore`].

use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::store::PgoStore;
use crate::types::SessionRecord;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Counters returned after ingesting a telemetry log file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct IngestStats {
    /// Total number of lines read (including blank / malformed).
    pub lines_read: usize,
    /// Sessions where at least one new chunk row was written.
    pub sessions_inserted: usize,
    /// Sessions that were already fully present (zero new rows).
    pub sessions_skipped_duplicate: usize,
    /// Lines that could not be parsed as `TelemetryEvent` JSON.
    pub parse_errors: usize,
}

// ---------------------------------------------------------------------------
// Internal wire type
// ---------------------------------------------------------------------------

/// JSON shape written by `cloudpack-abs` for every served bundle request.
#[derive(serde::Deserialize)]
struct TelemetryEvent {
    session_id: String,
    entry_point: String,
    /// Chunk IDs in the order they were served to the client.
    chunks_served: Vec<String>,
    /// Previously-cached chunks the client already had — ignored for PGO.
    #[allow(dead_code)]
    client_had: Vec<String>,
    timestamp_ms: u64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read `log_path` line-by-line, parse each as a [`TelemetryEvent`], and
/// insert the corresponding [`SessionRecord`] into `store`.
///
/// * Malformed lines are counted in [`IngestStats::parse_errors`] and skipped.
/// * Duplicate sessions (zero new rows inserted) are counted in
///   [`IngestStats::sessions_skipped_duplicate`].
/// * The function is idempotent: re-ingesting the same file is safe.
pub fn ingest_log(store: &PgoStore, log_path: &Path) -> anyhow::Result<IngestStats> {
    let file = std::fs::File::open(log_path)
        .map_err(|e| anyhow::anyhow!("failed to open log file {:?}: {}", log_path, e))?;
    let reader = BufReader::new(file);
    let mut stats = IngestStats::default();

    for line in reader.lines() {
        let line = line.map_err(|e| anyhow::anyhow!("IO error reading log line: {}", e))?;
        stats.lines_read += 1;

        let event: TelemetryEvent = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => {
                stats.parse_errors += 1;
                continue;
            }
        };

        let record = SessionRecord {
            session_id: event.session_id,
            entry_point: event.entry_point,
            chunk_sequence: event.chunks_served,
            timestamp_ms: event.timestamp_ms,
        };

        let rows_inserted = store.insert_session(&record)?;
        if rows_inserted == 0 {
            stats.sessions_skipped_duplicate += 1;
        } else {
            stats.sessions_inserted += 1;
        }
    }

    Ok(stats)
}
