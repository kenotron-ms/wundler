# Observability P2 — Chunk Error Reporting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface client-side chunk load failures as structured server-side telemetry and Prometheus-style counters, so operators can see *which* build / chunk / error-type combinations are breaking in the wild.

**Architecture:** A new `metrics` module owns a small, in-process `Metrics` struct (atomic counters + a `DashMap` keyed by `(build_id, chunk_id, error_type)`). The Service Worker fire-and-forgets `POST /telemetry/chunk-error` reports; the handler increments the counter, appends a `chunk_error` JSONL event to the existing `TelemetryLogger`, and always returns 200 OK. A new `GET /metrics` endpoint renders the counters in Prometheus text-exposition format. On every successful `POST /reload`, stale `build_id` entries are pruned out of the DashMap so the counter set does not grow without bound.

**Tech Stack:** Rust 2021, axum 0.8, tokio, dashmap 5, serde / serde_json, std::sync::atomic. Service Worker code remains hand-rolled vanilla JS (no bundler step).

**Scope Boundary:**
- IN: `POST /telemetry/chunk-error` handler, body cap, Prometheus exposition, `Metrics::prune` on reload, SW `reportChunkError()` + dedup, `TelemetryEventV2` additive tagged enum.
- OUT (deferred, do not touch in this plan):
  - `eval_failed` error reporting (page-side, not SW-side).
  - `POST /telemetry/web-vitals` (P4 — hard gate).
  - Fixing the `session_id`-carries-`build_id` bug at `server.rs:310-313`.
  - Bearer-token auth on `POST /telemetry/chunk-error` — documented as deferred.
  - Per-event `flush()` on the telemetry writer for chunk-error events (rely on existing `BufWriter::flush` in `TelemetryLogger::log`).

---

## File Structure

**Created:**
- `crates/cloudpack-abs/src/metrics/mod.rs` — `Metrics`, `ChunkErrorKey`, `ErrorType`, `Metrics::prune`.
- `crates/cloudpack-abs/src/metrics/chunk_error.rs` — `ChunkErrorReport`, `ChunkErrorEvent`, `ChunkErrorDeps`, `post_chunk_error` handler.
- `crates/cloudpack-abs/src/metrics/prometheus.rs` — `render(&Metrics) -> String` hand-rolled text-exposition renderer.

**Modified:**
- `crates/cloudpack-abs/Cargo.toml` — add `dashmap = "5"`.
- `crates/cloudpack-abs/src/lib.rs` — add `pub mod metrics;`.
- `crates/cloudpack-abs/src/types.rs` — add `TelemetryEventV2` tagged enum + `ManifestEvent` (additive, leaves `TelemetryEvent` untouched).
- `crates/cloudpack-abs/src/server.rs` — thread `Arc<Metrics>` through `RouterState`, register `/telemetry/chunk-error` + `/metrics` routes, call `Metrics::prune` after a successful `POST /reload`, update `test_state()` helper.
- `crates/cloudpack-abs/assets/sw.js` — add `reportChunkError()` + dedup set, wire into chunk-fetch failure paths.

---

## Task 1 — `metrics` module skeleton: types + `prune()`

**Files:**
- Modify: `crates/cloudpack-abs/Cargo.toml`
- Create: `crates/cloudpack-abs/src/metrics/mod.rs`
- Modify: `crates/cloudpack-abs/src/lib.rs`

- [ ] **Step 1.1: Add `dashmap` dependency**

Edit `crates/cloudpack-abs/Cargo.toml`. In the `[dependencies]` block, after `governor = "0.7"`, insert:

```toml
dashmap = "5"
```

Final relevant lines:

```toml
governor = "0.7"
dashmap = "5"
tempfile = "3"
```

- [ ] **Step 1.2: Write the failing unit tests for `Metrics::new` and `Metrics::prune`**

Create `crates/cloudpack-abs/src/metrics/mod.rs` with **only the tests** filled in — the implementation comes next so we can watch the tests fail.

```rust
//! In-process metrics for the Asset Bundling Server.
//!
//! Owns atomic counters for hot-path totals and a `DashMap` of per-`(build_id,
//! chunk_id, error_type)` chunk-error counters.  Cheap to share via `Arc`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

pub mod chunk_error;
pub mod prometheus;

/// Composite key for a single chunk-error counter cell.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct ChunkErrorKey {
    pub build_id: String,
    pub chunk_id: String,
    pub error_type: ErrorType,
}

/// Classification of a chunk failure as reported by the Service Worker.
#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    LoadFailed,
    NetworkTimeout,
    IntegrityMismatch,
}

/// Process-wide metrics.  Cheap to share via `Arc<Metrics>`.
#[derive(Debug, Default)]
pub struct Metrics {
    pub manifest_requests_total: AtomicU64,
    pub manifest_bytes_total: AtomicU64,
    pub chunk_errors: DashMap<ChunkErrorKey, AtomicU64>,
    pub cache_hits_total: AtomicU64,
    pub cache_misses_total: AtomicU64,
}

impl Metrics {
    /// Construct with all counters at zero and an empty chunk-error map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the chunk-error counter for the given key by one,
    /// inserting a fresh `AtomicU64(0)` first if no entry exists yet.
    pub fn incr_chunk_error(&self, key: ChunkErrorKey) {
        self.chunk_errors
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Drop every chunk-error entry whose `build_id` is not in
    /// `active_build_ids`.  Called from `POST /reload` after a successful
    /// swap to stop the map growing without bound across builds.
    pub fn prune(&self, active_build_ids: &HashSet<String>) {
        self.chunk_errors
            .retain(|key, _value| active_build_ids.contains(&key.build_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_empty_and_zeroed() {
        let m = Metrics::new();
        assert_eq!(m.manifest_requests_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.manifest_bytes_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.cache_hits_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.cache_misses_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.chunk_errors.len(), 0);
    }

    #[test]
    fn incr_chunk_error_inserts_and_increments() {
        let m = Metrics::new();
        let key = ChunkErrorKey {
            build_id: "b1".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::LoadFailed,
        };
        m.incr_chunk_error(key.clone());
        m.incr_chunk_error(key.clone());

        let entry = m.chunk_errors.get(&key).expect("entry must exist");
        assert_eq!(entry.value().load(Ordering::Relaxed), 2);
    }

    #[test]
    fn prune_drops_keys_for_inactive_builds() {
        let m = Metrics::new();
        let k_old = ChunkErrorKey {
            build_id: "old".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::LoadFailed,
        };
        let k_new = ChunkErrorKey {
            build_id: "new".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::NetworkTimeout,
        };
        m.incr_chunk_error(k_old.clone());
        m.incr_chunk_error(k_new.clone());
        assert_eq!(m.chunk_errors.len(), 2);

        let mut active = HashSet::new();
        active.insert("new".to_string());
        m.prune(&active);

        assert_eq!(m.chunk_errors.len(), 1);
        assert!(m.chunk_errors.get(&k_old).is_none());
        assert!(m.chunk_errors.get(&k_new).is_some());
    }

    #[test]
    fn prune_with_all_active_keeps_everything() {
        let m = Metrics::new();
        let k = ChunkErrorKey {
            build_id: "b1".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::IntegrityMismatch,
        };
        m.incr_chunk_error(k.clone());

        let mut active = HashSet::new();
        active.insert("b1".to_string());
        m.prune(&active);

        assert_eq!(m.chunk_errors.len(), 1);
    }
}
```

- [ ] **Step 1.3: Register the new module**

Edit `crates/cloudpack-abs/src/lib.rs`. Below the existing `pub mod manifest;` line, add `pub mod metrics;` so the module list reads:

```rust
pub mod archive;
pub mod manifest;
pub mod metrics;
pub mod security;
pub mod server;
pub mod signing;
pub mod state;
pub mod telemetry;
pub mod types;
```

- [ ] **Step 1.4: Stub the two child modules so the crate compiles**

The `mod.rs` references `pub mod chunk_error;` and `pub mod prometheus;`. Create empty stubs so this task can compile and run its tests in isolation. They will be filled out in Tasks 2 and 3.

Create `crates/cloudpack-abs/src/metrics/chunk_error.rs`:

```rust
//! `POST /telemetry/chunk-error` handler. Filled in by Task 2.
```

Create `crates/cloudpack-abs/src/metrics/prometheus.rs`:

```rust
//! Hand-rolled Prometheus text-exposition renderer. Filled in by Task 3.
```

- [ ] **Step 1.5: Run the unit tests — expect them to PASS**

```bash
cd /home/ken/workspace/cloudpack
cargo test -p cloudpack-abs metrics:: -- --nocapture
```

Expected: all 4 tests in `metrics::tests` pass. If `cargo` complains about an unused import in the stubs, that is fine — just ensure tests pass.

- [ ] **Step 1.6: Verify the whole crate still compiles and all pre-existing tests pass**

```bash
cargo test -p cloudpack-abs
```

Expected: full test suite green; nothing in `server.rs`, `archive.rs`, `manifest.rs`, etc. should have regressed.

- [ ] **Step 1.7: Commit**

```bash
git add crates/cloudpack-abs/Cargo.toml \
        crates/cloudpack-abs/src/lib.rs \
        crates/cloudpack-abs/src/metrics/mod.rs \
        crates/cloudpack-abs/src/metrics/chunk_error.rs \
        crates/cloudpack-abs/src/metrics/prometheus.rs
git commit -m "feat(abs): metrics module skeleton — Metrics, ChunkErrorKey, ErrorType, prune"
```

---

## Task 2 — `POST /telemetry/chunk-error` handler

**Files:**
- Modify: `crates/cloudpack-abs/src/metrics/chunk_error.rs`

The handler is built as a *self-contained unit* — it takes its dependencies via a small `ChunkErrorDeps` substate so it can be unit-tested without dragging the entire `RouterState` along. Task 3 wires it into the main router via `axum::extract::FromRef`.

- [ ] **Step 2.1: Write the failing handler tests**

Replace the stub in `crates/cloudpack-abs/src/metrics/chunk_error.rs` with the file below. The tests reference `post_chunk_error`, `ChunkErrorReport`, `ChunkErrorEvent`, and `ChunkErrorDeps`, which do not exist yet — that is intentional.

```rust
//! `POST /telemetry/chunk-error` — fire-and-forget chunk failure reports
//! from the Service Worker.
//!
//! Contract:
//!   * Body MUST be JSON of shape [`ChunkErrorReport`], capped at 8 KiB.
//!   * Successful parse increments the corresponding `Metrics::chunk_errors`
//!     cell and appends a [`ChunkErrorEvent`] JSONL line to the telemetry log.
//!   * The handler NEVER returns 5xx — telemetry log failures degrade to a
//!     `warn!` and a 200 OK so the SW never has a reason to retry.
//!   * Bodies above 8 KiB return 413 (enforced by `DefaultBodyLimit` in the
//!     router wiring).
//!
//! Bearer-token auth on this route is deferred — see plan scope.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::metrics::{ChunkErrorKey, ErrorType, Metrics};
use crate::telemetry::TelemetryLogger;

/// Maximum accepted body size (bytes). Enforced by the route's
/// `DefaultBodyLimit` layer; this constant is the source of truth.
pub const MAX_BODY_BYTES: usize = 8 * 1024;

/// Wire payload sent by the Service Worker.
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
///
/// Carries an explicit `kind` discriminator so consumers grepping the JSONL
/// file can distinguish `chunk_error` lines from the pre-existing manifest
/// events.  Task 4 replaces this with a `TelemetryEventV2` tagged enum.
#[derive(Debug, Serialize)]
pub struct ChunkErrorEvent {
    pub kind: &'static str,
    pub build_id: String,
    pub chunk_id: String,
    pub url: String,
    pub error_type: ErrorType,
    pub timestamp_ms: u64,
    pub session_id: String,
}

/// Substate consumed by [`post_chunk_error`].
///
/// In production this is derived from `RouterState` via `FromRef` (see Task 3).
/// In unit tests it can be constructed directly.
#[derive(Clone)]
pub struct ChunkErrorDeps {
    pub metrics: Arc<Metrics>,
    pub telemetry: TelemetryLogger,
}

/// Handler for `POST /telemetry/chunk-error`.
pub async fn post_chunk_error(
    State(deps): State<ChunkErrorDeps>,
    Json(report): Json<ChunkErrorReport>,
) -> impl IntoResponse {
    let key = ChunkErrorKey {
        build_id: report.build_id.clone(),
        chunk_id: report.chunk_id.clone(),
        error_type: report.error_type,
    };
    deps.metrics.incr_chunk_error(key);

    let event = ChunkErrorEvent {
        kind: "chunk_error",
        build_id: report.build_id,
        chunk_id: report.chunk_id,
        url: report.url,
        error_type: report.error_type,
        timestamp_ms: report.timestamp_ms,
        session_id: report.session_id,
    };

    if let Err(e) = deps.telemetry.log(&event) {
        warn!("chunk-error telemetry log failed: {e}");
    }

    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::Ordering;

    use axum::{
        body::Body,
        extract::DefaultBodyLimit,
        http::{Request, StatusCode},
        routing::post,
        Router,
    };
    use tower::ServiceExt;

    fn deps() -> (ChunkErrorDeps, Arc<Metrics>) {
        let metrics = Arc::new(Metrics::new());
        let tmp = tempfile::NamedTempFile::new().expect("tmp file for telemetry");
        let telemetry =
            TelemetryLogger::new(tmp.path()).expect("telemetry logger");
        // Leak the NamedTempFile so the unlink does not race the open FD.
        // Not strictly necessary on Unix but keeps the assertion simple if a
        // future test reads the file contents back.
        Box::leak(Box::new(tmp));
        let d = ChunkErrorDeps {
            metrics: Arc::clone(&metrics),
            telemetry,
        };
        (d, metrics)
    }

    fn router(deps: ChunkErrorDeps) -> Router {
        Router::new()
            .route("/telemetry/chunk-error", post(post_chunk_error))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(deps)
    }

    fn body(json: &str) -> Body {
        Body::from(json.to_string())
    }

    #[tokio::test]
    async fn valid_report_returns_200_and_increments_counter() {
        let (d, metrics) = deps();
        let router = router(d);

        let payload = r#"{
            "build_id": "b1",
            "chunk_id": "c1",
            "url": "https://cdn.example.com/chunks/abc.js",
            "error_type": "load_failed",
            "timestamp_ms": 1700000000000,
            "session_id": "sess-xyz"
        }"#;
        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/telemetry/chunk-error")
                    .header("content-type", "application/json")
                    .body(body(payload))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let key = ChunkErrorKey {
            build_id: "b1".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::LoadFailed,
        };
        let entry = metrics.chunk_errors.get(&key).expect("counter exists");
        assert_eq!(entry.value().load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn malformed_json_returns_4xx_and_does_not_increment() {
        let (d, metrics) = deps();
        let router = router(d);

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/telemetry/chunk-error")
                    .header("content-type", "application/json")
                    .body(body("{ this is not json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // axum returns 400 for malformed Json by default.
        assert!(resp.status().is_client_error());
        assert_eq!(metrics.chunk_errors.len(), 0);
    }

    #[tokio::test]
    async fn body_over_8kib_returns_413() {
        let (d, _metrics) = deps();
        let router = router(d);

        // Build a JSON payload whose total byte length exceeds 8 KiB by
        // padding `chunk_id`.  The `DefaultBodyLimit` layer rejects it
        // before the handler ever runs.
        let pad = "x".repeat(9000);
        let payload = format!(
            r#"{{"build_id":"b1","chunk_id":"{pad}","url":"u","error_type":"load_failed","timestamp_ms":1,"session_id":"s"}}"#
        );
        assert!(payload.len() > MAX_BODY_BYTES);

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/telemetry/chunk-error")
                    .header("content-type", "application/json")
                    .body(body(&payload))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn multiple_distinct_reports_create_distinct_cells() {
        let (d, metrics) = deps();
        let router = router(d);

        for (chunk, err) in [
            ("c1", "load_failed"),
            ("c1", "network_timeout"),
            ("c2", "load_failed"),
        ] {
            let payload = format!(
                r#"{{"build_id":"b1","chunk_id":"{chunk}","url":"u","error_type":"{err}","timestamp_ms":1,"session_id":"s"}}"#
            );
            let resp = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/telemetry/chunk-error")
                        .header("content-type", "application/json")
                        .body(body(&payload))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
        assert_eq!(metrics.chunk_errors.len(), 3);
    }
}
```

- [ ] **Step 2.2: Run the tests to confirm they fail to compile**

```bash
cargo test -p cloudpack-abs metrics::chunk_error
```

Expected: compile error if you somehow skipped step 2.1's contents — but if you wrote the file as shown above (which already contains the implementation), the tests will compile. In strict TDD you'd write tests first, then add the impl in 2.3. Since the file is one cohesive unit here, instead **temporarily comment out the four `pub fn`/`pub async fn` bodies** (replace each with `todo!()`) and re-run:

```bash
cargo test -p cloudpack-abs metrics::chunk_error
```

Expected: tests compile, but every test fails with `panicked at 'not yet implemented'`. This is the "red" step.

- [ ] **Step 2.3: Restore the implementations from Step 2.1**

Revert the `todo!()` substitutions so the file is identical to Step 2.1. (Or, if you skipped the red-step nudge entirely, just leave the file as-is.)

- [ ] **Step 2.4: Run the tests to confirm they pass**

```bash
cargo test -p cloudpack-abs metrics::chunk_error -- --nocapture
```

Expected output (4 tests, all PASS):
- `valid_report_returns_200_and_increments_counter ... ok`
- `malformed_json_returns_4xx_and_does_not_increment ... ok`
- `body_over_8kib_returns_413 ... ok`
- `multiple_distinct_reports_create_distinct_cells ... ok`

- [ ] **Step 2.5: Sanity-check the whole crate still builds and tests pass**

```bash
cargo test -p cloudpack-abs
```

Expected: full test suite green.

- [ ] **Step 2.6: Commit**

```bash
git add crates/cloudpack-abs/src/metrics/chunk_error.rs
git commit -m "feat(abs): POST /telemetry/chunk-error handler + ChunkErrorDeps substate"
```

---

## Task 3 — Prometheus exposition + thread `Arc<Metrics>` through the router

**Files:**
- Modify: `crates/cloudpack-abs/src/metrics/prometheus.rs`
- Modify: `crates/cloudpack-abs/src/server.rs`

This task does three things at once because they cannot meaningfully be split: (a) write the Prometheus renderer, (b) extend `RouterState` with `Arc<Metrics>` and thread it through `build_router` + `run()`, and (c) register the two new routes (`POST /telemetry/chunk-error`, `GET /metrics`).

- [ ] **Step 3.1: Write the failing tests for `prometheus::render`**

Replace the stub in `crates/cloudpack-abs/src/metrics/prometheus.rs` with the test module only, leaving the function body as `todo!()`:

```rust
//! Hand-rolled Prometheus text-exposition renderer.
//!
//! No SDK dependency. Output conforms to the Prometheus 0.0.4 text
//! exposition format (one HELP line, one TYPE line, then samples).

use std::fmt::Write as _;
use std::sync::atomic::Ordering;

use crate::metrics::{ChunkErrorKey, ErrorType, Metrics};

/// Render `metrics` as a Prometheus text-exposition string.
///
/// Emitted series:
/// * `manifest_requests_total` — counter, scalar
/// * `manifest_bytes_total` — counter, scalar
/// * `cloudpack_chunk_errors_total{build_id=..,chunk_id=..,error_type=..}` — counter, vector
///
/// (Note: only the chunk-error series is prefixed with `cloudpack_`. This
/// matches the design spec — do not "normalise" the other names.)
pub fn render(metrics: &Metrics) -> String {
    todo!("Task 3.3")
}

fn error_type_label(t: ErrorType) -> &'static str {
    match t {
        ErrorType::LoadFailed => "load_failed",
        ErrorType::NetworkTimeout => "network_timeout",
        ErrorType::IntegrityMismatch => "integrity_mismatch",
    }
}

/// Escape a label value for Prometheus exposition.
/// Spec: backslash, double-quote, and newline must be backslash-escaped.
fn esc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str(r#"\""#),
            '\n' => out.push_str(r"\n"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn empty_metrics_render_help_and_type_lines() {
        let m = Metrics::new();
        let out = render(&m);
        // Scalars are always emitted, even at zero.
        assert!(out.contains("# HELP manifest_requests_total"));
        assert!(out.contains("# TYPE manifest_requests_total counter"));
        assert!(out.contains("manifest_requests_total 0"));
        assert!(out.contains("# HELP manifest_bytes_total"));
        assert!(out.contains("# TYPE manifest_bytes_total counter"));
        assert!(out.contains("manifest_bytes_total 0"));
        // Chunk-error series HELP/TYPE always present.
        assert!(out.contains("# HELP cloudpack_chunk_errors_total"));
        assert!(out.contains("# TYPE cloudpack_chunk_errors_total counter"));
    }

    #[test]
    fn scalar_counters_render_their_values() {
        let m = Metrics::new();
        m.manifest_requests_total.store(42, Ordering::Relaxed);
        m.manifest_bytes_total.store(1024, Ordering::Relaxed);

        let out = render(&m);
        assert!(out.contains("manifest_requests_total 42"));
        assert!(out.contains("manifest_bytes_total 1024"));
    }

    #[test]
    fn chunk_error_samples_render_with_labels() {
        let m = Metrics::new();
        let key = ChunkErrorKey {
            build_id: "build-A".into(),
            chunk_id: "chunkX".into(),
            error_type: ErrorType::LoadFailed,
        };
        m.incr_chunk_error(key.clone());
        m.incr_chunk_error(key);

        let out = render(&m);
        let expected = r#"cloudpack_chunk_errors_total{build_id="build-A",chunk_id="chunkX",error_type="load_failed"} 2"#;
        assert!(
            out.contains(expected),
            "expected line not found in output:\n{out}"
        );
    }

    #[test]
    fn label_values_are_escaped() {
        let m = Metrics::new();
        let key = ChunkErrorKey {
            build_id: r#"weird"id"#.into(),
            chunk_id: r"back\slash".into(),
            error_type: ErrorType::NetworkTimeout,
        };
        m.incr_chunk_error(key);

        let out = render(&m);
        assert!(out.contains(r#"build_id="weird\"id""#));
        assert!(out.contains(r#"chunk_id="back\\slash""#));
        assert!(out.contains(r#"error_type="network_timeout""#));
    }
}
```

- [ ] **Step 3.2: Run the renderer tests — expect them to FAIL**

```bash
cargo test -p cloudpack-abs metrics::prometheus
```

Expected: 4 tests fail with `panicked at 'not yet implemented: Task 3.3'`.

- [ ] **Step 3.3: Implement `render` to make the tests pass**

Replace the `todo!()` body in `metrics/prometheus.rs` with:

```rust
pub fn render(metrics: &Metrics) -> String {
    let mut out = String::with_capacity(512);

    // ----- manifest_requests_total -----
    writeln!(
        out,
        "# HELP manifest_requests_total Total number of POST /manifest requests."
    )
    .unwrap();
    writeln!(out, "# TYPE manifest_requests_total counter").unwrap();
    writeln!(
        out,
        "manifest_requests_total {}",
        metrics.manifest_requests_total.load(Ordering::Relaxed)
    )
    .unwrap();

    // ----- manifest_bytes_total -----
    writeln!(
        out,
        "# HELP manifest_bytes_total Total bytes of manifest payloads served."
    )
    .unwrap();
    writeln!(out, "# TYPE manifest_bytes_total counter").unwrap();
    writeln!(
        out,
        "manifest_bytes_total {}",
        metrics.manifest_bytes_total.load(Ordering::Relaxed)
    )
    .unwrap();

    // ----- cloudpack_chunk_errors_total -----
    writeln!(
        out,
        "# HELP cloudpack_chunk_errors_total Chunk load failures reported by service workers, labelled by build/chunk/error_type."
    )
    .unwrap();
    writeln!(out, "# TYPE cloudpack_chunk_errors_total counter").unwrap();

    // Iterate the DashMap.  Order is non-deterministic; that's fine for
    // Prometheus consumers — they parse by name+labels, not by line order.
    for entry in metrics.chunk_errors.iter() {
        let key: &ChunkErrorKey = entry.key();
        let count = entry.value().load(Ordering::Relaxed);
        writeln!(
            out,
            r#"cloudpack_chunk_errors_total{{build_id="{}",chunk_id="{}",error_type="{}"}} {}"#,
            esc(&key.build_id),
            esc(&key.chunk_id),
            error_type_label(key.error_type),
            count,
        )
        .unwrap();
    }

    out
}
```

- [ ] **Step 3.4: Run the renderer tests — expect PASS**

```bash
cargo test -p cloudpack-abs metrics::prometheus -- --nocapture
```

Expected: 4 tests pass.

- [ ] **Step 3.5: Write the failing integration tests in `server.rs`**

Open `crates/cloudpack-abs/src/server.rs`. At the bottom of the existing `#[cfg(test)] mod tests` block (after the `default_config_has_no_rate_limit` test, before the closing `}` on line 771), append:

```rust
    // -----------------------------------------------------------------
    // P2 — /metrics and /telemetry/chunk-error wiring
    // -----------------------------------------------------------------

    /// GET /metrics returns Prometheus text exposition for empty state.
    #[tokio::test]
    async fn metrics_endpoint_renders_text_exposition() {
        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: None,
        });
        let (app, telemetry) = test_state();
        let metrics = Arc::new(crate::metrics::Metrics::new());
        let router = super::build_router(app, telemetry, metrics, security);

        let resp = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("# TYPE manifest_requests_total counter"));
        assert!(body.contains("manifest_requests_total 0"));
        assert!(body.contains("# TYPE cloudpack_chunk_errors_total counter"));
    }

    /// POST /telemetry/chunk-error is reachable through the production router,
    /// increments the metric, and returns 200.
    #[tokio::test]
    async fn chunk_error_endpoint_increments_metric() {
        use std::sync::atomic::Ordering;

        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: None,
        });
        let (app, telemetry) = test_state();
        let metrics = Arc::new(crate::metrics::Metrics::new());
        let router = super::build_router(
            app,
            telemetry,
            Arc::clone(&metrics),
            security,
        );

        let payload = r#"{
            "build_id":"b1",
            "chunk_id":"c1",
            "url":"https://cdn.example.com/chunks/abc.js",
            "error_type":"load_failed",
            "timestamp_ms":1,
            "session_id":"s"
        }"#;
        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/telemetry/chunk-error")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let key = crate::metrics::ChunkErrorKey {
            build_id: "b1".into(),
            chunk_id: "c1".into(),
            error_type: crate::metrics::ErrorType::LoadFailed,
        };
        let entry = metrics.chunk_errors.get(&key).expect("counter exists");
        assert_eq!(entry.value().load(Ordering::Relaxed), 1);
    }
```

- [ ] **Step 3.6: Run these tests — expect compile error**

```bash
cargo test -p cloudpack-abs --no-run
```

Expected: compile errors complaining that `build_router` has the wrong arity (we're passing 4 args) and that there's no `/metrics` or `/telemetry/chunk-error` route. This is the "red" signal.

- [ ] **Step 3.7: Extend `RouterState` and `build_router`**

In `crates/cloudpack-abs/src/server.rs`:

(a) Update the imports near the top of the file. Replace the existing line:

```rust
use crate::telemetry::TelemetryLogger;
```

with:

```rust
use crate::metrics::chunk_error::{post_chunk_error, ChunkErrorDeps};
use crate::metrics::{prometheus as prom, Metrics};
use crate::telemetry::TelemetryLogger;
```

(b) Replace `RouterState` (currently lines 108–112) with:

```rust
/// State threaded through every Axum handler.
///
/// Cheap to `Clone` — all fields are internally reference-counted.
#[derive(Clone)]
struct RouterState {
    app: AppState,
    telemetry: TelemetryLogger,
    metrics: Arc<Metrics>,
}

impl axum::extract::FromRef<RouterState> for ChunkErrorDeps {
    fn from_ref(state: &RouterState) -> Self {
        ChunkErrorDeps {
            metrics: Arc::clone(&state.metrics),
            telemetry: state.telemetry.clone(),
        }
    }
}

impl axum::extract::FromRef<RouterState> for Arc<Metrics> {
    fn from_ref(state: &RouterState) -> Self {
        Arc::clone(&state.metrics)
    }
}
```

(c) Replace `build_router` (currently lines 210–243) with:

```rust
pub fn build_router(
    app: AppState,
    telemetry: TelemetryLogger,
    metrics: Arc<Metrics>,
    security: Arc<ResolvedSecurity>,
) -> Router {
    let state = RouterState {
        app,
        telemetry,
        metrics,
    };
    let cors = build_cors(&security);

    // POST /manifest route, optionally rate-limited per source IP.
    let manifest_route = match security.rate_limiter.clone() {
        None => Router::new().route("/manifest", post(post_manifest)),
        Some(limiter) => Router::new()
            .route("/manifest", post(post_manifest))
            .route_layer(axum::middleware::from_fn_with_state(
                limiter,
                crate::security::ratelimit::rate_limit_mw,
            )),
    };

    // POST /telemetry/chunk-error — 8 KiB body cap, no auth (deferred).
    let chunk_error_route = Router::new()
        .route("/telemetry/chunk-error", post(post_chunk_error))
        .layer(axum::extract::DefaultBodyLimit::max(
            crate::metrics::chunk_error::MAX_BODY_BYTES,
        ));

    Router::new()
        .merge(manifest_route)
        .merge(chunk_error_route)
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
        .route("/reload", post(post_reload))
        .route("/select", post(post_select))
        .route("/versions", get(get_versions))
        .route("/metrics", get(get_metrics))
        // Inner: bearer-token authentication.
        .layer(middleware::from_fn_with_state(security, require_bearer))
        // Outer: CORS — applied last so it wraps the auth layer.
        .layer(cors)
        .with_state(state)
}
```

(d) Add the `get_metrics` handler. After the `get_service_worker` function (after line 358), insert:

```rust
/// `GET /metrics` — Prometheus text exposition.
///
/// Returns `Content-Type: text/plain; version=0.0.4` per the Prometheus
/// exposition format spec.
async fn get_metrics(State(state): State<RouterState>) -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        prom::render(&state.metrics),
    )
}
```

(e) Update `run()` to construct and pass the metrics. Replace the current line (approximately line 271):

```rust
    let router = build_router(app, telemetry, Arc::new(security));
```

with:

```rust
    let metrics = Arc::new(Metrics::new());
    let router = build_router(app, telemetry, metrics, Arc::new(security));
```

(f) Update the existing test helper `router_with_ip` (currently lines 628–642) to take the metrics and forward them:

```rust
    fn router_with_ip(
        app: AppState,
        telemetry: TelemetryLogger,
        metrics: Arc<crate::metrics::Metrics>,
        security: Arc<ResolvedSecurity>,
        ip: IpAddr,
    ) -> axum::Router {
        let inner = super::build_router(app, telemetry, metrics, security);
        inner.layer(middleware::from_fn(
            move |mut req: Request<Body>, next: middleware::Next| async move {
                let ci: ConnectInfo<SocketAddr> = ConnectInfo(SocketAddr::new(ip, 49_152));
                req.extensions_mut().insert(ci);
                next.run(req).await
            },
        ))
    }
```

(g) Update **every existing call site** of `router_with_ip` inside this test module (three places: `manifest_post_is_rate_limited`, `health_is_not_rate_limited`, `default_config_has_no_rate_limit`). For each, immediately above the `let router = router_with_ip(...)` line, add:

```rust
        let metrics = Arc::new(crate::metrics::Metrics::new());
```

…and update the call to pass `Arc::clone(&metrics)` (or just `metrics`) between `telemetry` and `security`. Example (the rate-limit test):

```rust
        let (app, telemetry) = test_state();
        let metrics = Arc::new(crate::metrics::Metrics::new());
        let router = router_with_ip(
            app,
            telemetry,
            metrics,
            security,
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );
```

Apply the same change to the other two helpers.

- [ ] **Step 3.8: Run all crate tests — expect PASS**

```bash
cargo test -p cloudpack-abs
```

Expected: every test passes, including the two new ones from Step 3.5 (`metrics_endpoint_renders_text_exposition`, `chunk_error_endpoint_increments_metric`) and all pre-existing tests.

- [ ] **Step 3.9: `cargo check` the workspace to catch any downstream breakage**

```bash
cargo check --workspace
```

Expected: clean compile. If any other crate calls `cloudpack_abs::server::build_router` directly, update its arity to include `Arc<Metrics>` between `telemetry` and `security` and re-run.

- [ ] **Step 3.10: Commit**

```bash
git add crates/cloudpack-abs/src/metrics/prometheus.rs \
        crates/cloudpack-abs/src/server.rs
git commit -m "feat(abs): GET /metrics + POST /telemetry/chunk-error wired through RouterState"
```

---

## Task 4 — `TelemetryEventV2` tagged enum + `Metrics::prune` on reload

**Files:**
- Modify: `crates/cloudpack-abs/src/types.rs`
- Modify: `crates/cloudpack-abs/src/metrics/chunk_error.rs`
- Modify: `crates/cloudpack-abs/src/server.rs`

- [ ] **Step 4.1: Add the additive `TelemetryEventV2` enum**

Append to `crates/cloudpack-abs/src/types.rs` (after the existing `TelemetryEvent` struct, after line 96):

```rust
// ---------------------------------------------------------------------------
// TelemetryEventV2 — additive tagged-union log format
// ---------------------------------------------------------------------------

/// JSONL log shape used by the new chunk-error pipeline.
///
/// `TelemetryEvent` (above) is preserved verbatim for backward compatibility
/// with existing log files; new event kinds go through this enum, which
/// serializes with an externally-tagged `kind` discriminator so consumers
/// can grep by `"kind":"manifest"` / `"kind":"chunk_error"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TelemetryEventV2 {
    Manifest(ManifestEvent),
    ChunkError(ChunkErrorEventV2),
}

/// Manifest-served event (mirrors the legacy [`TelemetryEvent`] shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEvent {
    pub session_id: String,
    pub entry_point: String,
    pub chunks_served: Vec<String>,
    pub client_had: Vec<ContentHash>,
    pub timestamp_ms: u64,
}

/// Chunk-error event (mirrors the body of `ChunkErrorReport` plus a
/// server-side timestamp).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkErrorEventV2 {
    pub build_id: String,
    pub chunk_id: String,
    pub url: String,
    pub error_type: crate::metrics::ErrorType,
    pub timestamp_ms: u64,
    pub session_id: String,
}
```

- [ ] **Step 4.2: Write a test that the tagged enum round-trips correctly**

Append to the test module at the bottom of `types.rs` (if one exists; otherwise add it). Insert into `crates/cloudpack-abs/src/types.rs`:

```rust
#[cfg(test)]
mod v2_tests {
    use super::*;

    #[test]
    fn chunk_error_v2_serializes_with_kind_tag() {
        let ev = TelemetryEventV2::ChunkError(ChunkErrorEventV2 {
            build_id: "b1".into(),
            chunk_id: "c1".into(),
            url: "u".into(),
            error_type: crate::metrics::ErrorType::LoadFailed,
            timestamp_ms: 42,
            session_id: "s".into(),
        });
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"chunk_error""#));
        assert!(json.contains(r#""error_type":"load_failed""#));

        let parsed: TelemetryEventV2 = serde_json::from_str(&json).unwrap();
        match parsed {
            TelemetryEventV2::ChunkError(e) => {
                assert_eq!(e.build_id, "b1");
                assert_eq!(e.timestamp_ms, 42);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn manifest_v2_round_trips() {
        let ev = TelemetryEventV2::Manifest(ManifestEvent {
            session_id: "s".into(),
            entry_point: "home".into(),
            chunks_served: vec!["c1".into()],
            client_had: vec![],
            timestamp_ms: 7,
        });
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"manifest""#));
        let _round: TelemetryEventV2 = serde_json::from_str(&json).unwrap();
    }
}
```

- [ ] **Step 4.3: Run the new tests — expect PASS**

```bash
cargo test -p cloudpack-abs types::v2_tests
```

Expected: both tests pass. (`TelemetryLogger::log`'s generic `<T: Serialize>` bound means we did not have to touch the logger at all.)

- [ ] **Step 4.4: Switch `post_chunk_error` to log via `TelemetryEventV2`**

In `crates/cloudpack-abs/src/metrics/chunk_error.rs`:

(a) Add the import near the top:

```rust
use crate::types::{ChunkErrorEventV2, TelemetryEventV2};
```

(b) Delete the local `ChunkErrorEvent` struct entirely (the one with `kind: &'static str` from Task 2).

(c) Replace the construction inside `post_chunk_error` so the `event` block reads:

```rust
    let event = TelemetryEventV2::ChunkError(ChunkErrorEventV2 {
        build_id: report.build_id,
        chunk_id: report.chunk_id,
        url: report.url,
        error_type: report.error_type,
        timestamp_ms: report.timestamp_ms,
        session_id: report.session_id,
    });

    if let Err(e) = deps.telemetry.log(&event) {
        warn!("chunk-error telemetry log failed: {e}");
    }
```

(`TelemetryLogger::log` is generic over `T: Serialize`, so this compiles without changing `telemetry.rs`.)

- [ ] **Step 4.5: Add a regression test that the JSONL line carries `"kind":"chunk_error"`**

Add to the `mod tests` inside `chunk_error.rs`:

```rust
    #[tokio::test]
    async fn jsonl_line_has_kind_chunk_error() {
        use std::io::Read;

        // Build deps with a known-on-disk log path so we can read it back.
        let log_path = tempfile::NamedTempFile::new()
            .unwrap()
            .into_temp_path()
            .keep()
            .unwrap();
        let metrics = Arc::new(Metrics::new());
        let telemetry = TelemetryLogger::new(&log_path).unwrap();
        let deps = ChunkErrorDeps {
            metrics: Arc::clone(&metrics),
            telemetry,
        };
        let router = router(deps);

        let payload = r#"{
            "build_id":"b1","chunk_id":"c1","url":"u",
            "error_type":"integrity_mismatch","timestamp_ms":1,"session_id":"s"
        }"#;
        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/telemetry/chunk-error")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Drop the deps clone held by the router to release the BufWriter
        // before we read.  The TelemetryLogger inside `deps` is gone after
        // `with_state`, but the writer is `Arc<Mutex<_>>` so we must release
        // all clones for the flush-on-drop to fire.  `BufWriter::flush` was
        // already called in `log()` so this is belt-and-braces.
        drop(router);

        let mut s = String::new();
        std::fs::File::open(&log_path).unwrap().read_to_string(&mut s).unwrap();
        assert!(
            s.contains(r#""kind":"chunk_error""#),
            "log line missing kind tag: {s}"
        );
        assert!(s.contains(r#""error_type":"integrity_mismatch""#));
    }
```

Run it:

```bash
cargo test -p cloudpack-abs metrics::chunk_error::tests::jsonl_line_has_kind_chunk_error
```

Expected: PASS.

- [ ] **Step 4.6: Wire `Metrics::prune` into `POST /reload`**

In `crates/cloudpack-abs/src/server.rs`, inside `post_reload` (currently lines 401–478), locate the success block at the very bottom:

```rust
    (StatusCode::OK, Json(SwapResponseBody {
        previous: report.previous,
        current: report.current,
    }))
        .into_response()
```

Immediately *before* that final expression (i.e. after `let report = match state.app.swap_to(&body.build_id).await { ... };`), insert:

```rust
    // Prune the chunk-error map: keep counters for the build we just swapped
    // to and for whatever else is currently retained in the archive; drop
    // everything else so the map cannot grow without bound across builds.
    let active: std::collections::HashSet<String> = state
        .app
        .archive
        .list()
        .map(|entries| entries.into_iter().map(|e| e.build_id).collect())
        .unwrap_or_default();
    state.metrics.prune(&active);
```

- [ ] **Step 4.7: Write an integration test that prune fires on reload**

Append to the `tests` module in `server.rs`:

```rust
    /// After a successful POST /reload, chunk-error counters for build_ids
    /// no longer in the archive must be dropped.
    #[tokio::test]
    async fn reload_prunes_stale_chunk_error_counters() {
        use std::io::Write;
        use crate::metrics::{ChunkErrorKey, ErrorType, Metrics};

        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: None,
        });
        let (app, telemetry) = test_state();
        let metrics = Arc::new(Metrics::new());

        // Seed counters for two builds: "test-build" (current) and "ghost".
        metrics.incr_chunk_error(ChunkErrorKey {
            build_id: "test-build".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::LoadFailed,
        });
        metrics.incr_chunk_error(ChunkErrorKey {
            build_id: "ghost".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::LoadFailed,
        });

        let router = super::build_router(
            app,
            telemetry,
            Arc::clone(&metrics),
            security,
        );

        // Write a fresh manifest to disk so /reload can read it.
        let manifest_json = serde_json::json!({
            "build_id": "test-build-2",
            "chunks": [],
            "entry_chunks": {},
            "module_index": {},
        });
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{}", manifest_json).unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/reload")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"manifest_path":"{path}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // "ghost" build is no longer in the archive → counter must be gone.
        let ghost = ChunkErrorKey {
            build_id: "ghost".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::LoadFailed,
        };
        assert!(metrics.chunk_errors.get(&ghost).is_none(), "ghost not pruned");

        // "test-build" was the prior current build — whether it remains in
        // the archive depends on `archive_retention`. With retention=10
        // (the test default), it should still be present.
        let current = ChunkErrorKey {
            build_id: "test-build".into(),
            chunk_id: "c1".into(),
            error_type: ErrorType::LoadFailed,
        };
        assert!(
            metrics.chunk_errors.get(&current).is_some(),
            "retained build counter must survive prune"
        );
    }
```

- [ ] **Step 4.8: Run the full crate test suite**

```bash
cargo test -p cloudpack-abs
```

Expected: all tests pass (legacy + new). If `reload_prunes_stale_chunk_error_counters` fails because the test fixture's `ChunkManifest` deserialization rejects empty `module_index`, simplify the JSON to match `cloudpack_graph::ChunkManifest`'s exact shape — check the struct definition in `crates/cloudpack-graph/src/lib.rs` and adjust.

- [ ] **Step 4.9: Commit**

```bash
git add crates/cloudpack-abs/src/types.rs \
        crates/cloudpack-abs/src/metrics/chunk_error.rs \
        crates/cloudpack-abs/src/server.rs
git commit -m "feat(abs): TelemetryEventV2 tagged enum + prune chunk-error counters on /reload"
```

---

## Task 5 — Service Worker: `reportChunkError()` + dedup + wire into failure paths

**Files:**
- Modify: `crates/cloudpack-abs/assets/sw.js`

The current SW has three places where a chunk fetch can fail without telling the server:

1. **Required-chunk fetch** at lines 127–135 — there is no `try` / `catch` at all; a thrown `fetch()` rejects the surrounding `Promise.all` and abandons all in-flight chunks silently.
2. **Prefetch chunk fetch** at lines 137–148 — has a silent `catch {}`.
3. **`!resp.ok` branches** inside both (1) and (2) — a 404/503 from the CDN is treated identically to a successful fetch.

We replace all three with `reportChunkError()` calls plus a per-SW-lifecycle dedup `Set`. Note that the SW also has unrelated silent catches in `notifyBuildIdChanged` (postMessage failures) and in the post-build-id-change manifest reload — those are **not** chunk failures and stay as-is.

- [ ] **Step 5.1: Read the current SW file to confirm line offsets**

```bash
sed -n '120,150p' crates/cloudpack-abs/assets/sw.js
```

Expected: shows the required-chunk `Promise.all` block (no try/catch) and the prefetch `void (async () => { try { ... } catch {} })()` block. Note exact line offsets in case they have drifted.

- [ ] **Step 5.2: Add `reportChunkError()` helper and the dedup set**

Edit `crates/cloudpack-abs/assets/sw.js`. Immediately above the existing `// src/buildid.ts` marker (around line 61), insert:

```js
  // src/chunkerr.ts
  // Per-SW-lifecycle dedup: each (build_id, chunk_id, error_type) triple
  // is reported at most once to keep the server from being hammered when
  // a single broken chunk fails for every navigation in a tab session.
  const REPORTED_CHUNK_ERRORS = new Set();

  function reportChunkError(absBaseUrl, payload) {
    const dedupKey = `${payload.build_id}|${payload.chunk_id}|${payload.error_type}`;
    if (REPORTED_CHUNK_ERRORS.has(dedupKey)) return;
    REPORTED_CHUNK_ERRORS.add(dedupKey);
    try {
      void fetch(`${absBaseUrl}/telemetry/chunk-error`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
        keepalive: true,
      }).catch(() => {});
    } catch {}
  }

  function makeChunkErrorPayload(buildId, url, errorType) {
    // chunk_id is the last path segment without the `.js` suffix —
    // matches the `<8-char hash>` chunk-id scheme used by the manifest.
    let chunkId = url;
    try {
      const u = new URL(url);
      const seg = u.pathname.split("/").pop() ?? "";
      chunkId = seg.replace(/\.js$/, "") || url;
    } catch {}
    return {
      build_id: buildId ?? "unknown",
      chunk_id: chunkId,
      url,
      error_type: errorType,
      timestamp_ms: Date.now(),
      session_id: globalThis.__CLOUDPACK_SESSION_ID__ ?? "anonymous",
    };
  }
```

- [ ] **Step 5.3: Wire the required-chunk fetch (currently lines 127–135) to report on failure**

Replace this block:

```js
    await Promise.all(
      requiredUrls.map(async (url) => {
        const existing = await cache.match(url);
        if (existing) return;
        const resp = await fetch(url);
        if (resp.ok) {
          await cache.put(url, resp);
        }
      })
    );
```

with:

```js
    await Promise.all(
      requiredUrls.map(async (url) => {
        const existing = await cache.match(url);
        if (existing) return;
        try {
          const resp = await fetch(url);
          if (resp.ok) {
            await cache.put(url, resp);
          } else {
            reportChunkError(
              config.absBaseUrl,
              makeChunkErrorPayload(static_?.build_id, url, "load_failed"),
            );
          }
        } catch (err) {
          const isTimeout =
            err && (err.name === "TimeoutError" || err.name === "AbortError");
          reportChunkError(
            config.absBaseUrl,
            makeChunkErrorPayload(
              static_?.build_id,
              url,
              isTimeout ? "network_timeout" : "load_failed",
            ),
          );
        }
      }),
    );
```

- [ ] **Step 5.4: Wire the prefetch chunk fetch (currently lines 137–148) to report on failure**

Replace this block:

```js
    for (const url of prefetchUrls) {
      void (async () => {
        try {
          const existing = await cache.match(url);
          if (existing) return;
          const resp = await fetch(url);
          if (resp.ok) {
            await cache.put(url, resp);
          }
        } catch {
        }
      })();
    }
```

with:

```js
    for (const url of prefetchUrls) {
      void (async () => {
        try {
          const existing = await cache.match(url);
          if (existing) return;
          const resp = await fetch(url);
          if (resp.ok) {
            await cache.put(url, resp);
          } else {
            reportChunkError(
              config.absBaseUrl,
              makeChunkErrorPayload(static_?.build_id, url, "load_failed"),
            );
          }
        } catch (err) {
          const isTimeout =
            err && (err.name === "TimeoutError" || err.name === "AbortError");
          reportChunkError(
            config.absBaseUrl,
            makeChunkErrorPayload(
              static_?.build_id,
              url,
              isTimeout ? "network_timeout" : "load_failed",
            ),
          );
        }
      })();
    }
```

- [ ] **Step 5.5: Verify SW still parses as valid JavaScript**

There is no JS test harness in this repo, so we lean on Node's parser:

```bash
node --check crates/cloudpack-abs/assets/sw.js
```

Expected: no output (exit 0). Any syntax error would print here.

- [ ] **Step 5.6: Add an inline Rust integration test that proves the SW string contains the new code**

Embed a minimal smoke check so future refactors of `sw.js` can't accidentally drop the chunk-error wiring. Append to the `tests` module in `crates/cloudpack-abs/src/server.rs`:

```rust
    /// Smoke test: the embedded SW carries the chunk-error wiring.
    ///
    /// This is the closest thing to a JS unit test we have in-tree — it
    /// catches accidental regressions that would silently un-instrument
    /// the SW (e.g. a future refactor that re-bundles sw.js from source
    /// and forgets to include the chunkerr module).
    ///
    /// Manual end-to-end test: serve `/sw.js`, register it in a real
    /// browser, force a CDN 404 for a chunk URL, verify a POST hits
    /// `/telemetry/chunk-error` and the `/metrics` endpoint shows the
    /// counter.  (See docs/superpowers/observability.md §P2.)
    #[test]
    fn embedded_sw_contains_chunk_error_reporter() {
        let sw = super::SERVICE_WORKER_JS;
        assert!(sw.contains("reportChunkError"), "SW lacks reportChunkError helper");
        assert!(sw.contains("/telemetry/chunk-error"), "SW lacks endpoint URL");
        assert!(sw.contains("REPORTED_CHUNK_ERRORS"), "SW lacks dedup set");
        assert!(sw.contains("\"load_failed\""), "SW missing load_failed tag");
        assert!(sw.contains("\"network_timeout\""), "SW missing network_timeout tag");
    }
```

- [ ] **Step 5.7: Run the new test**

```bash
cargo test -p cloudpack-abs embedded_sw_contains_chunk_error_reporter
```

Expected: PASS.

- [ ] **Step 5.8: Run the full crate test suite once more**

```bash
cargo test -p cloudpack-abs
```

Expected: every test passes.

- [ ] **Step 5.9: Commit**

```bash
git add crates/cloudpack-abs/assets/sw.js \
        crates/cloudpack-abs/src/server.rs
git commit -m "feat(sw): reportChunkError() with per-lifecycle dedup, wired into required + prefetch fetches"
```

---

## Post-Plan Verification

After all five tasks land:

- [ ] `cargo test --workspace` is fully green.
- [ ] `cargo clippy -p cloudpack-abs --all-targets -- -D warnings` reports no new warnings.
- [ ] `node --check crates/cloudpack-abs/assets/sw.js` exits 0.
- [ ] Boot the server locally (`cargo run -p cloudpack-abs -- ...`), `curl http://localhost:8080/metrics` returns Prometheus text exposition with the three series HELP/TYPE blocks at zero.
- [ ] `curl -X POST -H 'content-type: application/json' -d '{"build_id":"b","chunk_id":"c","url":"u","error_type":"load_failed","timestamp_ms":1,"session_id":"s"}' http://localhost:8080/telemetry/chunk-error` returns `200 OK`; re-curling `/metrics` shows the counter at 1.

---

## Deferred / Documented-Not-Done

- **Bearer-token auth on `/telemetry/chunk-error`.** Intentionally absent. The SW currently has no token to present; adding auth requires either (a) injecting a per-session token via the page or (b) accepting the open endpoint as a trade-off. Tracked separately.
- **Per-event `flush()` on chunk-error.** Relies on `BufWriter`'s internal buffering and the existing `flush()` call inside `TelemetryLogger::log`. Spec explicitly says do not change this.
- **`session_id` carries `build_id` bug at `server.rs:310-313`.** Out of scope; do not fix here.
- **`eval_failed` and `/telemetry/web-vitals`.** P3 / P4. Not in this plan.
