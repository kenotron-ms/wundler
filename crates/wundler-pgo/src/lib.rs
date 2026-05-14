//! Wundler Profile-Guided Optimization Store.
//!
//! Ingests `TelemetryEvent` JSONL written by the ABS (Plan 4),
//! computes co-request statistics, runs C³-style clustering, and
//! writes advisory hint fields back into `manifest.json`.

pub const CRATE_NAME: &str = "wundler-pgo";
